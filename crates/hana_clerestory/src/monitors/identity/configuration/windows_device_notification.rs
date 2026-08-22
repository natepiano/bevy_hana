use std::mem::size_of;

use windows::Win32::Devices::Display::GUID_DEVINTERFACE_MONITOR;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::UI::WindowsAndMessaging::DBT_DEVICEARRIVAL;
use windows::Win32::UI::WindowsAndMessaging::DBT_DEVICEREMOVECOMPLETE;
use windows::Win32::UI::WindowsAndMessaging::DBT_DEVTYP_DEVICEINTERFACE;
use windows::Win32::UI::WindowsAndMessaging::DEV_BROADCAST_DEVICEINTERFACE_W;
use windows::Win32::UI::WindowsAndMessaging::DEV_BROADCAST_HDR;
use windows::Win32::UI::WindowsAndMessaging::WM_DEVICECHANGE;
use windows::Win32::UI::WindowsAndMessaging::WM_DISPLAYCHANGE;

use super::MonitorConfigurationGenerationTracker;
use crate::monitors::identity::OperatingSystemQueryError;

pub(super) fn monitor_device_notification_filter()
-> Result<DEV_BROADCAST_DEVICEINTERFACE_W, OperatingSystemQueryError> {
    let dbcc_size = u32::try_from(size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>())
        .map_err(|_| OperatingSystemQueryError::ConfigurationNotificationRegistration)?;
    Ok(DEV_BROADCAST_DEVICEINTERFACE_W {
        dbcc_size,
        dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
        dbcc_reserved: 0,
        dbcc_classguid: GUID_DEVINTERFACE_MONITOR,
        dbcc_name: [0],
    })
}

pub(super) unsafe fn process_configuration_message(
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    tracker: &MonitorConfigurationGenerationTracker,
) {
    if unsafe { classify_configuration_message(message, wparam, lparam) }
        == ConfigurationNotification::Advance
    {
        tracker.advance();
    }
}

unsafe fn classify_configuration_message(
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> ConfigurationNotification {
    match message {
        WM_DISPLAYCHANGE => ConfigurationNotification::Advance,
        WM_DEVICECHANGE => unsafe { classify_device_change(wparam, lparam) },
        _ => ConfigurationNotification::Ignore,
    }
}

unsafe fn classify_device_change(wparam: WPARAM, lparam: LPARAM) -> ConfigurationNotification {
    let Ok(event) = u32::try_from(wparam.0) else {
        return ConfigurationNotification::Ignore;
    };
    if !matches!(event, DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE) {
        return ConfigurationNotification::Ignore;
    }
    let Some(class_guid) = (unsafe { device_interface_class_guid(lparam) }) else {
        return ConfigurationNotification::Ignore;
    };
    if class_guid == GUID_DEVINTERFACE_MONITOR {
        ConfigurationNotification::Advance
    } else {
        ConfigurationNotification::Ignore
    }
}

unsafe fn device_interface_class_guid(lparam: LPARAM) -> Option<windows::core::GUID> {
    if lparam.0 == 0 {
        return None;
    }
    let header_size = u32::try_from(size_of::<DEV_BROADCAST_HDR>()).ok()?;
    let interface_size = u32::try_from(size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>()).ok()?;
    let address = lparam.0.cast_unsigned();
    let header =
        unsafe { std::ptr::with_exposed_provenance::<DEV_BROADCAST_HDR>(address).read_unaligned() };
    if header.dbch_size < header_size
        || header.dbch_devicetype != DBT_DEVTYP_DEVICEINTERFACE
        || header.dbch_size < interface_size
    {
        return None;
    }
    let interface = unsafe {
        std::ptr::with_exposed_provenance::<DEV_BROADCAST_DEVICEINTERFACE_W>(address)
            .read_unaligned()
    };
    Some(interface.dbcc_classguid)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigurationNotification {
    Advance,
    Ignore,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE;
    use windows::core::GUID;

    use super::*;
    use crate::monitors::identity::configuration::MonitorConfigurationState;

    const UNRELATED_DEVICE_INTERFACE: GUID =
        GUID::from_u128(0x53f5_6307_b6bf_11d0_94f2_00a0_c91e_fb8b);

    #[test]
    fn monitor_filter_selects_monitor_device_interfaces() {
        let filter = monitor_device_notification_filter()
            .expect("monitor notification filter should be representable");

        assert_eq!(filter.dbcc_classguid, GUID_DEVINTERFACE_MONITOR);
        assert_eq!(filter.dbcc_devicetype, DBT_DEVTYP_DEVICEINTERFACE.0);
    }

    #[test]
    fn monitor_arrival_advances_configuration_generation() {
        assert_device_notification_advances(DBT_DEVICEARRIVAL);
    }

    #[test]
    fn monitor_removal_advances_configuration_generation() {
        assert_device_notification_advances(DBT_DEVICEREMOVECOMPLETE);
    }

    #[test]
    fn display_change_advances_configuration_generation() {
        let tracker = MonitorConfigurationGenerationTracker::default();

        unsafe {
            process_configuration_message(WM_DISPLAYCHANGE, WPARAM(0), LPARAM(0), &tracker);
        }

        assert_eq!(
            tracker.state(),
            MonitorConfigurationState::Ready(1_u64.into())
        );
    }

    #[test]
    fn unrelated_and_malformed_messages_do_not_advance_generation() {
        let tracker = MonitorConfigurationGenerationTracker::default();
        let unrelated = device_interface(UNRELATED_DEVICE_INTERFACE);
        let wrong_type = DEV_BROADCAST_HDR {
            dbch_size: u32::try_from(size_of::<DEV_BROADCAST_HDR>())
                .expect("broadcast header size should fit in u32"),
            ..Default::default()
        };
        let malformed = DEV_BROADCAST_HDR {
            dbch_size: 0,
            dbch_devicetype: DBT_DEVTYP_DEVICEINTERFACE,
            ..Default::default()
        };

        for (message, wparam, lparam) in [
            (
                WM_DEVICECHANGE,
                WPARAM(usize::try_from(DBT_DEVICEARRIVAL).expect("event fits")),
                LPARAM(0),
            ),
            (
                WM_DEVICECHANGE,
                WPARAM(usize::try_from(DBT_DEVICEARRIVAL).expect("event fits")),
                lparam(&unrelated),
            ),
            (
                WM_DEVICECHANGE,
                WPARAM(usize::try_from(DBT_DEVICEARRIVAL).expect("event fits")),
                lparam(&wrong_type),
            ),
            (
                WM_DEVICECHANGE,
                WPARAM(usize::try_from(DBT_DEVICEARRIVAL).expect("event fits")),
                lparam(&malformed),
            ),
            (
                WM_DEVICECHANGE,
                WPARAM(0),
                lparam(&device_interface(GUID_DEVINTERFACE_MONITOR)),
            ),
            (WM_NCCREATE, WPARAM(0), LPARAM(0)),
        ] {
            unsafe { process_configuration_message(message, wparam, lparam, &tracker) };
        }

        assert_eq!(
            tracker.state(),
            MonitorConfigurationState::Ready(0_u64.into())
        );
    }

    fn assert_device_notification_advances(event: u32) {
        let tracker = MonitorConfigurationGenerationTracker::default();
        let interface = device_interface(GUID_DEVINTERFACE_MONITOR);

        unsafe {
            process_configuration_message(
                WM_DEVICECHANGE,
                WPARAM(usize::try_from(event).expect("event should fit in usize")),
                lparam(&interface),
                &tracker,
            );
        }

        assert_eq!(
            tracker.state(),
            MonitorConfigurationState::Ready(1_u64.into())
        );
    }

    fn device_interface(class_guid: GUID) -> DEV_BROADCAST_DEVICEINTERFACE_W {
        DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size:       u32::try_from(size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>())
                .expect("device interface size should fit in u32"),
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_reserved:   0,
            dbcc_classguid:  class_guid,
            dbcc_name:       [0],
        }
    }

    fn lparam<T>(value: &T) -> LPARAM {
        LPARAM(std::ptr::from_ref(value).expose_provenance().cast_signed())
    }
}
