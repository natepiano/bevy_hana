use std::mem::size_of;

use windows::Win32::Devices::DeviceAndDriverInstallation::DICS_FLAG_GLOBAL;
use windows::Win32::Devices::DeviceAndDriverInstallation::DIGCF_DEVICEINTERFACE;
use windows::Win32::Devices::DeviceAndDriverInstallation::DIGCF_PRESENT;
use windows::Win32::Devices::DeviceAndDriverInstallation::DIREG_DEV;
use windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO;
use windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DATA;
use windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DETAIL_DATA_W;
use windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVINFO_DATA;
use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiGetClassDevsW;
use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiGetDeviceInterfaceDetailW;
use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiOpenDevRegKey;
use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiOpenDeviceInterfaceW;
use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
use windows::Win32::Devices::Display::DISPLAYCONFIG_MODE_INFO;
use windows::Win32::Devices::Display::DISPLAYCONFIG_PATH_INFO;
use windows::Win32::Devices::Display::DISPLAYCONFIG_SOURCE_DEVICE_NAME;
use windows::Win32::Devices::Display::DISPLAYCONFIG_TARGET_DEVICE_NAME;
use windows::Win32::Devices::Display::DisplayConfigGetDeviceInfo;
use windows::Win32::Devices::Display::GUID_DEVINTERFACE_MONITOR;
use windows::Win32::Devices::Display::GetDisplayConfigBufferSizes;
use windows::Win32::Devices::Display::QDC_ONLY_ACTIVE_PATHS;
use windows::Win32::Devices::Display::QueryDisplayConfig;
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::Graphics::Gdi::GetMonitorInfoW;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::Graphics::Gdi::MONITORINFO;
use windows::Win32::Graphics::Gdi::MONITORINFOEXW;
use windows::Win32::System::Registry::HKEY;
use windows::Win32::System::Registry::KEY_READ;
use windows::Win32::System::Registry::RRF_RT_REG_BINARY;
use windows::Win32::System::Registry::RegGetValueW;
use windows::core::HRESULT;
use windows::core::Owned;
use windows::core::PCWSTR;
use windows::core::w;
use winit::monitor::MonitorHandle;
use winit::platform::windows::MonitorHandleExtWindows;

use super::MonitorEvidenceObservation;
use super::attachment_path_when_unreported;
use crate::Platform;
use crate::constants::DISPLAY_CONFIG_ACQUISITION_ATTEMPTS;
#[cfg(test)]
use crate::constants::DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG;
#[cfg(test)]
use crate::constants::DRM_INTERNAL_CONNECTOR_PREFIXES;
use crate::monitors::identity::MonitorIdentificationError;
use crate::monitors::identity::OperatingSystemQueryError;
use crate::monitors::identity::edid::EdidEvidence;
use crate::monitors::identity::edid::EdidIdentityEvidence;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(in crate::monitors) enum QualifiedEvidence {
    WindowsEdid(EdidIdentityEvidence),
    #[cfg(test)]
    InternalConnector(InternalConnector),
    #[cfg(test)]
    X11Edid(EdidIdentityEvidence),
    #[cfg(any(test, feature = "test"))]
    Synthetic(Vec<u8>),
}

impl QualifiedEvidence {
    /// Bytes that identify the physical display and nothing about this process or this boot.
    pub(in crate::monitors::identity) fn stable_bytes(&self) -> &[u8] {
        match self {
            Self::WindowsEdid(edid) => edid.stable_bytes(),
            #[cfg(test)]
            Self::InternalConnector(connector) => connector.as_bytes(),
            #[cfg(test)]
            Self::X11Edid(edid) => edid.stable_bytes(),
            #[cfg(any(test, feature = "test"))]
            Self::Synthetic(bytes) => bytes,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::monitors) struct InternalConnector(Vec<u8>);

#[cfg(test)]
impl InternalConnector {
    fn as_bytes(&self) -> &[u8] { &self.0 }

    fn identify(connector_name: &str) -> Option<Self> {
        DRM_INTERNAL_CONNECTOR_PREFIXES
            .iter()
            .any(|prefix| connector_name.starts_with(prefix))
            .then(|| {
                let mut evidence = DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG.to_vec();
                evidence.extend_from_slice(connector_name.as_bytes());
                Self(evidence)
            })
    }
}

pub(in crate::monitors) fn monitor_evidence(
    handle: &MonitorHandle,
    platform: Platform,
) -> MonitorEvidenceObservation {
    match platform {
        Platform::Windows => MonitorEvidenceObservation {
            identity:   windows_display_evidence(handle).map(QualifiedEvidence::WindowsEdid),
            attachment: attachment_path_when_unreported(platform),
        },
        Platform::MacOs | Platform::X11 | Platform::Wayland => MonitorEvidenceObservation {
            identity:   Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
            attachment: attachment_path_when_unreported(platform),
        },
    }
}

fn windows_display_evidence(
    handle: &MonitorHandle,
) -> Result<EdidIdentityEvidence, MonitorIdentificationError> {
    let native_id = windows_native_id(handle)?;
    let mut matched_path = None;
    for path in windows_display_config_paths()? {
        if windows_source_name(&path)? != native_id {
            continue;
        }
        if matched_path.replace(path).is_some() {
            return Err(MonitorIdentificationError::InvalidStableIdentity);
        }
    }
    let path = matched_path.ok_or(MonitorIdentificationError::InvalidStableIdentity)?;
    let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    target.header.size = u32::try_from(size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>())
        .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?;
    target.header.adapterId = path.targetInfo.adapterId;
    target.header.id = path.targetInfo.id;
    check_display_config(unsafe { DisplayConfigGetDeviceInfo(&raw mut target.header) })?;

    let device_path = terminated_utf16(&target.monitorDevicePath)
        .ok_or(MonitorIdentificationError::InvalidStableIdentity)?;
    qualify_edid(windows_edid(&device_path))
}

fn windows_native_id(handle: &MonitorHandle) -> Result<String, MonitorIdentificationError> {
    let native_handle = handle.hmonitor();
    let hmonitor = HMONITOR(std::ptr::with_exposed_provenance_mut(
        native_handle.cast_unsigned(),
    ));
    let mut native_monitor_info = MONITORINFOEXW::default();
    native_monitor_info.monitorInfo.cbSize = u32::try_from(size_of::<MONITORINFOEXW>())
        .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?;
    if !unsafe {
        GetMonitorInfoW(
            hmonitor,
            (&raw mut native_monitor_info).cast::<MONITORINFO>(),
        )
    }
    .as_bool()
    {
        return Err(OperatingSystemQueryError::DisplayConfiguration.into());
    }
    let native_id = utf16_string(&native_monitor_info.szDevice);
    if native_id.is_empty() {
        Err(MonitorIdentificationError::InvalidStableIdentity)
    } else {
        Ok(native_id)
    }
}

fn windows_display_config_paths() -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, MonitorIdentificationError>
{
    for _ in 0..DISPLAY_CONFIG_ACQUISITION_ATTEMPTS {
        let mut path_count = 0;
        let mut mode_count = 0;
        check_win32_result(unsafe {
            GetDisplayConfigBufferSizes(
                QDC_ONLY_ACTIVE_PATHS,
                &raw mut path_count,
                &raw mut mode_count,
            )
        })?;

        let path_capacity = usize::try_from(path_count)
            .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?;
        let mode_capacity = usize::try_from(mode_count)
            .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?;
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_capacity];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_capacity];
        let result = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &raw mut path_count,
                paths.as_mut_ptr(),
                &raw mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };
        if result == ERROR_SUCCESS {
            paths.truncate(
                usize::try_from(path_count)
                    .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?,
            );
            return Ok(paths);
        }
        if result != ERROR_INSUFFICIENT_BUFFER {
            return Err(OperatingSystemQueryError::DisplayConfiguration.into());
        }
    }
    Err(OperatingSystemQueryError::DisplayConfiguration.into())
}

fn windows_source_name(
    path: &DISPLAYCONFIG_PATH_INFO,
) -> Result<String, MonitorIdentificationError> {
    let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    source.header.size = u32::try_from(size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>())
        .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?;
    source.header.adapterId = path.sourceInfo.adapterId;
    source.header.id = path.sourceInfo.id;
    check_display_config(unsafe { DisplayConfigGetDeviceInfo(&raw mut source.header) })?;
    let source_name = utf16_string(&source.viewGdiDeviceName);
    if source_name.is_empty() {
        Err(MonitorIdentificationError::InvalidStableIdentity)
    } else {
        Ok(source_name)
    }
}

fn check_display_config(result: i32) -> Result<(), MonitorIdentificationError> {
    if result.cast_unsigned() == ERROR_SUCCESS.0 {
        Ok(())
    } else {
        Err(OperatingSystemQueryError::DisplayConfiguration.into())
    }
}

fn check_win32_result(result: WIN32_ERROR) -> Result<(), MonitorIdentificationError> {
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(OperatingSystemQueryError::DisplayConfiguration.into())
    }
}

fn windows_edid(device_path: &[u16]) -> Result<Vec<u8>, MonitorIdentificationError> {
    let device_info_set = windows_monitor_device_info_set()?;
    let interface_data = windows_monitor_device_interface(*device_info_set, device_path)?;
    let device_info = windows_monitor_device_info(*device_info_set, &interface_data)?;
    let registry_key = windows_monitor_registry_key(*device_info_set, &device_info)?;
    windows_registry_edid(*registry_key)
}

fn windows_monitor_device_info_set() -> Result<Owned<HDEVINFO>, MonitorIdentificationError> {
    let device_info_set = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_MONITOR),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }
    .map_err(|_| OperatingSystemQueryError::MonitorDeviceInterface)?;
    Ok(unsafe { Owned::new(device_info_set) })
}

fn windows_monitor_device_interface(
    device_info_set: HDEVINFO,
    device_path: &[u16],
) -> Result<SP_DEVICE_INTERFACE_DATA, MonitorIdentificationError> {
    let mut interface_data = SP_DEVICE_INTERFACE_DATA {
        cbSize: u32::try_from(size_of::<SP_DEVICE_INTERFACE_DATA>())
            .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?,
        ..Default::default()
    };
    unsafe {
        SetupDiOpenDeviceInterfaceW(
            device_info_set,
            PCWSTR(device_path.as_ptr()),
            0,
            Some(&raw mut interface_data),
        )
    }
    .map_err(|_| OperatingSystemQueryError::MonitorDeviceInterface)?;
    Ok(interface_data)
}

fn windows_monitor_device_info(
    device_info_set: HDEVINFO,
    interface_data: &SP_DEVICE_INTERFACE_DATA,
) -> Result<SP_DEVINFO_DATA, MonitorIdentificationError> {
    let mut required_size = 0;
    let detail_result = unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            device_info_set,
            std::ptr::from_ref(interface_data),
            None,
            0,
            Some(&raw mut required_size),
            None,
        )
    };
    let Err(size_error) = detail_result else {
        return Err(OperatingSystemQueryError::MonitorDeviceInterface.into());
    };
    if size_error.code() != HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0) {
        return Err(OperatingSystemQueryError::MonitorDeviceInterface.into());
    }

    let required_bytes = usize::try_from(required_size)
        .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?;
    let word_count = required_bytes.div_ceil(size_of::<usize>());
    let mut detail_storage = vec![0_usize; word_count];
    let detail_data = detail_storage
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    unsafe {
        (*detail_data).cbSize = u32::try_from(size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>())
            .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?;
    }
    let mut device_info = SP_DEVINFO_DATA {
        cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>())
            .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?,
        ..Default::default()
    };
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            device_info_set,
            std::ptr::from_ref(interface_data),
            Some(detail_data),
            required_size,
            None,
            Some(&raw mut device_info),
        )
    }
    .map_err(|_| OperatingSystemQueryError::MonitorDeviceInterface)?;
    Ok(device_info)
}

fn windows_monitor_registry_key(
    device_info_set: HDEVINFO,
    device_info: &SP_DEVINFO_DATA,
) -> Result<Owned<HKEY>, MonitorIdentificationError> {
    let registry_key = unsafe {
        SetupDiOpenDevRegKey(
            device_info_set,
            std::ptr::from_ref(device_info),
            DICS_FLAG_GLOBAL.0,
            0,
            DIREG_DEV,
            KEY_READ.0,
        )
    }
    .map_err(|_| OperatingSystemQueryError::StableIdentityProperty)?;
    Ok(unsafe { Owned::new(registry_key) })
}

fn windows_registry_edid(registry_key: HKEY) -> Result<Vec<u8>, MonitorIdentificationError> {
    let mut edid_size = 0;
    let size_result = unsafe {
        RegGetValueW(
            registry_key,
            PCWSTR::null(),
            w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            None,
            Some(&raw mut edid_size),
        )
    };
    if size_result == ERROR_FILE_NOT_FOUND {
        return Err(MonitorIdentificationError::InvalidStableIdentity);
    }
    if size_result != ERROR_SUCCESS {
        return Err(OperatingSystemQueryError::StableIdentityProperty.into());
    }
    if edid_size == 0 {
        return Err(MonitorIdentificationError::InvalidStableIdentity);
    }
    let mut edid = vec![
        0;
        usize::try_from(edid_size)
            .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?
    ];
    if unsafe {
        RegGetValueW(
            registry_key,
            PCWSTR::null(),
            w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            Some(edid.as_mut_ptr().cast()),
            Some(&raw mut edid_size),
        )
    } != ERROR_SUCCESS
    {
        return Err(OperatingSystemQueryError::StableIdentityProperty.into());
    }
    edid.truncate(
        usize::try_from(edid_size)
            .map_err(|_| MonitorIdentificationError::InvalidStableIdentity)?,
    );
    Ok(edid)
}

fn utf16_string(code_units: &[u16]) -> String {
    let length = code_units
        .iter()
        .position(|code_unit| *code_unit == 0)
        .unwrap_or(code_units.len());
    String::from_utf16_lossy(&code_units[..length])
}

fn terminated_utf16(code_units: &[u16]) -> Option<Vec<u16>> {
    let terminator = code_units.iter().position(|code_unit| *code_unit == 0)?;
    (terminator > 0).then(|| code_units[..=terminator].to_vec())
}

fn qualify_edid(
    query: Result<Vec<u8>, MonitorIdentificationError>,
) -> Result<EdidIdentityEvidence, MonitorIdentificationError> {
    EdidEvidence::qualify(query?)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn query_failures_and_rejected_identity_data_remain_distinct() {
        let query_error =
            qualify_edid(Err(OperatingSystemQueryError::StableIdentityProperty.into()));
        let rejected_data = qualify_edid(Ok(Vec::new()));

        assert_eq!(
            query_error,
            Err(MonitorIdentificationError::OperatingSystemQuery(
                OperatingSystemQueryError::StableIdentityProperty
            ))
        );
        assert_eq!(
            rejected_data,
            Err(MonitorIdentificationError::InvalidStableIdentity)
        );
    }

    #[test]
    fn qualified_edid_variants_retain_the_extracted_serial_tier() {
        let identity = qualify_edid(Ok(serial_edid())).expect("fixture EDID should qualify");
        let windows = QualifiedEvidence::WindowsEdid(identity.clone());
        let x11 = QualifiedEvidence::X11Edid(identity);

        assert_eq!(windows.stable_bytes(), b"42");
        assert_eq!(x11.stable_bytes(), b"42");
    }

    #[test]
    fn internal_connector_test_evidence_uses_the_linux_tag() {
        let connector = InternalConnector::identify("eDP-1").expect("eDP-1 is internal");
        let qualified = QualifiedEvidence::InternalConnector(connector);

        assert!(
            qualified
                .stable_bytes()
                .starts_with(DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG)
        );
    }

    fn serial_edid() -> Vec<u8> {
        let mut edid = vec![0_u8; EdidEvidence::BLOCK_SIZE];
        edid[..EdidEvidence::HEADER.len()].copy_from_slice(&EdidEvidence::HEADER);
        edid[EdidEvidence::SERIAL_START..EdidEvidence::SERIAL_END]
            .copy_from_slice(&42_u32.to_le_bytes());
        let checksum_index = EdidEvidence::BLOCK_SIZE - 1;
        let checksum = edid[..checksum_index]
            .iter()
            .fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
        edid[checksum_index] = 0_u8.wrapping_sub(checksum);
        edid
    }
}
