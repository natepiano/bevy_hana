#[cfg(target_os = "macos")]
use std::ffi::c_void;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "windows")]
use std::mem::size_of;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::str;

use hana_rigging::prelude::AttachmentPath;
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
use hana_rigging::prelude::ReportedId;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::DICS_FLAG_GLOBAL;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::DIGCF_DEVICEINTERFACE;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::DIGCF_PRESENT;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::DIREG_DEV;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DATA;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVICE_INTERFACE_DETAIL_DATA_W;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVINFO_DATA;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiGetClassDevsW;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiGetDeviceInterfaceDetailW;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiOpenDevRegKey;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiOpenDeviceInterfaceW;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::DISPLAYCONFIG_MODE_INFO;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::DISPLAYCONFIG_PATH_INFO;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::DISPLAYCONFIG_SOURCE_DEVICE_NAME;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::DISPLAYCONFIG_TARGET_DEVICE_NAME;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::DisplayConfigGetDeviceInfo;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::GUID_DEVINTERFACE_MONITOR;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::GetDisplayConfigBufferSizes;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::QDC_ONLY_ACTIVE_PATHS;
#[cfg(target_os = "windows")]
use windows::Win32::Devices::Display::QueryDisplayConfig;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::ERROR_SUCCESS;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::WIN32_ERROR;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::GetMonitorInfoW;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::HMONITOR;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::MONITORINFO;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::MONITORINFOEXW;
#[cfg(target_os = "windows")]
use windows::Win32::System::Registry::HKEY;
#[cfg(target_os = "windows")]
use windows::Win32::System::Registry::KEY_READ;
#[cfg(target_os = "windows")]
use windows::Win32::System::Registry::RRF_RT_REG_BINARY;
#[cfg(target_os = "windows")]
use windows::Win32::System::Registry::RegGetValueW;
#[cfg(target_os = "windows")]
use windows::core::HRESULT;
#[cfg(target_os = "windows")]
use windows::core::Owned;
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;
#[cfg(target_os = "windows")]
use windows::core::w;
use winit::monitor::MonitorHandle;
#[cfg(target_os = "macos")]
use winit::platform::macos::MonitorHandleExtMacOS;
#[cfg(target_os = "windows")]
use winit::platform::windows::MonitorHandleExtWindows;
#[cfg(all(unix, not(target_os = "macos")))]
use winit::platform::x11::MonitorHandleExtX11;
#[cfg(all(unix, not(target_os = "macos")))]
use x11rb::NONE;
#[cfg(all(unix, not(target_os = "macos")))]
use x11rb::connection::Connection;
#[cfg(all(unix, not(target_os = "macos")))]
use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
#[cfg(all(unix, not(target_os = "macos")))]
use x11rb::protocol::randr::Output;
#[cfg(all(unix, not(target_os = "macos")))]
use x11rb::protocol::xproto::AtomEnum;
#[cfg(all(unix, not(target_os = "macos")))]
use x11rb::protocol::xproto::ConnectionExt as XprotoConnectionExt;
#[cfg(all(unix, not(target_os = "macos")))]
use x11rb::xcb_ffi::XCBConnection;

use super::MonitorIdentificationError;
#[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
use super::OperatingSystemQueryError;
#[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
use super::edid::EdidEvidence;
#[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
use super::edid::EdidIdentityEvidence;
use crate::Platform;
#[cfg(target_os = "windows")]
use crate::constants::DISPLAY_CONFIG_ACQUISITION_ATTEMPTS;
#[cfg(target_os = "linux")]
use crate::constants::DRM_CLASS_DIRECTORY;
#[cfg(target_os = "linux")]
use crate::constants::DRM_CONNECTOR_EDID_FILE;
#[cfg(target_os = "linux")]
use crate::constants::DRM_CONNECTOR_NAME_SEPARATOR;
#[cfg(any(test, target_os = "linux"))]
use crate::constants::DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG;
#[cfg(any(test, target_os = "linux"))]
use crate::constants::DRM_INTERNAL_CONNECTOR_PREFIXES;
#[cfg(target_os = "macos")]
use crate::constants::MACOS_DISPLAY_UUID_BYTES;
#[cfg(all(unix, not(target_os = "macos")))]
use crate::constants::X11_EDID_PROPERTY_NAME;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(in crate::monitors) enum QualifiedEvidence {
    #[cfg(any(test, target_os = "linux"))]
    InternalConnector(InternalConnector),
    #[cfg(target_os = "macos")]
    MacOsDisplayUuid(MacOsDisplayUuid),
    #[cfg(any(test, target_os = "windows"))]
    WindowsEdid(EdidIdentityEvidence),
    #[cfg(any(test, all(unix, not(target_os = "macos"))))]
    X11Edid(EdidIdentityEvidence),
    #[cfg(test)]
    Synthetic(Vec<u8>),
}

/// The DRM connector name of a panel built into the machine, tagged with
/// [`DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG`].
///
/// Only a connector whose name starts with one of [`DRM_INTERNAL_CONNECTOR_PREFIXES`] may be named
/// this way. Those connectors carry a panel that is soldered or wired to the board and can never
/// be exchanged, so naming the connector names the panel. Naming any other connector would claim
/// that whatever is plugged into a socket today is the same display that was plugged into it when
/// the position was saved, which is the conflation `PanelIdentity::Anonymous` exists to prevent.
///
/// macOS already resolves the same case the same way: `CGDisplayCreateUUIDFromDisplayID` falls
/// back to the physical port for panels that report no serial, which is why a built-in laptop
/// panel produces stable reported evidence there and, before this
/// evidence existed, was not on Linux.
#[cfg(any(test, target_os = "linux"))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::monitors) struct InternalConnector(Vec<u8>);

#[cfg(any(test, target_os = "linux"))]
impl InternalConnector {
    /// The evidence for `connector_name`, or `None` when that connector accepts a display the user
    /// can swap.
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

    fn as_bytes(&self) -> &[u8] { &self.0 }
}

/// The panel's `ColorSync` UUID: 16 bytes macOS derives from the display's own vendor, model and
/// serial number, falling back to the physical port for panels that report no serial.
///
/// This replaced a per-process counter keyed by winit `MonitorHandle`. That counter was correct
/// for telling two live monitors apart within one run, but it was a different number on the next
/// launch and did not even survive a replug inside one run, so it could never have been written
/// to a file.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::monitors) struct MacOsDisplayUuid([u8; MACOS_DISPLAY_UUID_BYTES]);

#[cfg(target_os = "macos")]
impl MacOsDisplayUuid {
    pub(super) const fn as_bytes(&self) -> &[u8] { &self.0 }
}

/// `CFUUIDBytes` — sixteen bytes in declaration order, laid out as CoreFoundation defines it.
#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct CoreFoundationUuidBytes {
    bytes: [u8; MACOS_DISPLAY_UUID_BYTES],
}

#[cfg(target_os = "macos")]
#[link(name = "ColorSync", kind = "framework")]
unsafe extern "C" {
    /// Returns a retained `CFUUIDRef` for the panel behind a `CGDirectDisplayID`, or null.
    fn CGDisplayCreateUUIDFromDisplayID(display_id: u32) -> *const c_void;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFUUIDGetUUIDBytes(uuid: *const c_void) -> CoreFoundationUuidBytes;
    fn CFRelease(reference: *const c_void);
}

/// Read the `ColorSync` UUID for a monitor, or report that no stable identity is available.
///
/// `CGDisplayCreateUUIDFromDisplayID` is a Create function, so the reference is released before
/// returning. A null result means the display ID no longer names a live display — which happens
/// routinely while monitors are being connected — and is reported as unavailable rather than
/// treated as an error worth retrying.
#[cfg(target_os = "macos")]
fn macos_panel_evidence(
    handle: &MonitorHandle,
) -> Result<MacOsDisplayUuid, MonitorIdentificationError> {
    let display_id = handle.native_id();
    // SAFETY: `display_id` comes from winit's own `CGDirectDisplayID`. The call returns either
    // null or a retained CFUUIDRef; both are handled, and the reference is released exactly once
    // on the non-null path before it goes out of scope.
    let uuid_reference = unsafe { CGDisplayCreateUUIDFromDisplayID(display_id) };
    if uuid_reference.is_null() {
        return Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable);
    }
    // SAFETY: `uuid_reference` is a non-null CFUUIDRef that this thread owns until `CFRelease`.
    let uuid_bytes = unsafe { CFUUIDGetUUIDBytes(uuid_reference) };
    // SAFETY: released once, immediately after the last read of the reference.
    unsafe { CFRelease(uuid_reference) };
    Ok(MacOsDisplayUuid(uuid_bytes.bytes))
}

impl QualifiedEvidence {
    /// Bytes that identify the physical panel and nothing about this process or this boot.
    ///
    /// Everything hashed here comes from the display itself, so the result is the same on every
    /// launch. Adding anything process-local (a handle address, a counter, an index) would make
    /// the fingerprint useless for persistence while still looking correct within one run.
    #[allow(
        clippy::missing_const_for_fn,
        reason = "const only compiles where the macOS arm is the sole arm; the Vec arms are not"
    )]
    pub(super) fn stable_bytes(&self) -> &[u8] {
        match self {
            #[cfg(any(test, target_os = "linux"))]
            Self::InternalConnector(connector) => connector.as_bytes(),
            #[cfg(target_os = "macos")]
            Self::MacOsDisplayUuid(uuid) => uuid.as_bytes(),
            #[cfg(any(test, target_os = "windows"))]
            Self::WindowsEdid(edid) => edid.stable_bytes(),
            #[cfg(any(test, all(unix, not(target_os = "macos"))))]
            Self::X11Edid(edid) => edid.stable_bytes(),
            #[cfg(test)]
            Self::Synthetic(bytes) => bytes,
        }
    }
}

/// One monitor read with panel identity kept separate from its attachment location.
pub(in crate::monitors) struct MonitorEvidenceObservation {
    pub identity:   Result<QualifiedEvidence, MonitorIdentificationError>,
    pub attachment: AttachmentPath,
}

/// Attachment evidence for a platform observation that supplied no checked connector.
pub(in crate::monitors) const fn attachment_path_when_unreported(
    platform: Platform,
) -> AttachmentPath {
    match platform {
        Platform::MacOs | Platform::Wayland => AttachmentPath::PlatformHasNoConcept,
        Platform::Windows | Platform::X11 => AttachmentPath::PlatformReportedNothing,
    }
}

pub(in crate::monitors) fn monitor_evidence(
    handle: &MonitorHandle,
    platform: Platform,
) -> MonitorEvidenceObservation {
    match platform {
        Platform::MacOs => {
            #[cfg(target_os = "macos")]
            {
                MonitorEvidenceObservation {
                    identity:   macos_panel_evidence(handle)
                        .map(QualifiedEvidence::MacOsDisplayUuid),
                    attachment: attachment_path_when_unreported(platform),
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                MonitorEvidenceObservation {
                    identity:   Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
                    attachment: attachment_path_when_unreported(platform),
                }
            }
        },
        Platform::Windows => {
            #[cfg(target_os = "windows")]
            {
                MonitorEvidenceObservation {
                    identity:   windows_panel_evidence(handle).map(QualifiedEvidence::WindowsEdid),
                    attachment: attachment_path_when_unreported(platform),
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                MonitorEvidenceObservation {
                    identity:   Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
                    attachment: attachment_path_when_unreported(platform),
                }
            }
        },
        Platform::X11 => {
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                x11_monitor_evidence(handle)
            }
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            {
                MonitorEvidenceObservation {
                    identity:   Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
                    attachment: attachment_path_when_unreported(platform),
                }
            }
        },
        Platform::Wayland => MonitorEvidenceObservation {
            identity:   Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
            attachment: attachment_path_when_unreported(platform),
        },
    }
}

#[cfg(target_os = "windows")]
fn windows_panel_evidence(
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

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
fn check_display_config(result: i32) -> Result<(), MonitorIdentificationError> {
    if result.cast_unsigned() == ERROR_SUCCESS.0 {
        Ok(())
    } else {
        Err(OperatingSystemQueryError::DisplayConfiguration.into())
    }
}

#[cfg(target_os = "windows")]
fn check_win32_result(result: WIN32_ERROR) -> Result<(), MonitorIdentificationError> {
    if result == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(OperatingSystemQueryError::DisplayConfiguration.into())
    }
}

#[cfg(target_os = "windows")]
fn windows_edid(device_path: &[u16]) -> Result<Vec<u8>, MonitorIdentificationError> {
    let device_info_set = windows_monitor_device_info_set()?;
    let interface_data = windows_monitor_device_interface(*device_info_set, device_path)?;
    let device_info = windows_monitor_device_info(*device_info_set, &interface_data)?;
    let registry_key = windows_monitor_registry_key(*device_info_set, &device_info)?;
    windows_registry_edid(*registry_key)
}

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
fn utf16_string(code_units: &[u16]) -> String {
    let length = code_units
        .iter()
        .position(|code_unit| *code_unit == 0)
        .unwrap_or(code_units.len());
    String::from_utf16_lossy(&code_units[..length])
}

#[cfg(target_os = "windows")]
fn terminated_utf16(code_units: &[u16]) -> Option<Vec<u16>> {
    let terminator = code_units.iter().position(|code_unit| *code_unit == 0)?;
    (terminator > 0).then(|| code_units[..=terminator].to_vec())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn x11_monitor_evidence(handle: &MonitorHandle) -> MonitorEvidenceObservation {
    let located_output: Result<_, MonitorIdentificationError> = (|| {
        let (connection, screen_number) = XCBConnection::connect(None)
            .map_err(|_| OperatingSystemQueryError::DisplayConfiguration)?;
        let root = connection
            .setup()
            .roots
            .get(screen_number)
            .ok_or(MonitorIdentificationError::InvalidStableIdentity)?
            .root;
        let resources = connection
            .randr_get_screen_resources_current(root)
            .map_err(|_| OperatingSystemQueryError::DisplayConfiguration)?
            .reply()
            .map_err(|_| OperatingSystemQueryError::DisplayConfiguration)?;
        let crtc = handle.native_id();
        let mut matched_output = None;
        for output in resources.outputs {
            let output_info = connection
                .randr_get_output_info(output, resources.config_timestamp)
                .map_err(|_| OperatingSystemQueryError::DisplayConfiguration)?
                .reply()
                .map_err(|_| OperatingSystemQueryError::DisplayConfiguration)?;
            if output_info.crtc != crtc {
                continue;
            }
            if matched_output.replace((output, output_info.name)).is_some() {
                return Err(MonitorIdentificationError::InvalidStableIdentity);
            }
        }
        let (output, connector_name) =
            matched_output.ok_or(MonitorIdentificationError::InvalidStableIdentity)?;
        Ok((connection, output, connector_name))
    })();
    let (connection, output, connector_name) = match located_output {
        Ok(located_output) => located_output,
        Err(error) => {
            return MonitorEvidenceObservation {
                identity:   Err(error),
                attachment: attachment_path_when_unreported(Platform::X11),
            };
        },
    };
    let panel_evidence =
        qualify_edid(x11_output_edid(&connection, output)).map(QualifiedEvidence::X11Edid);
    // XWayland answers `randr_get_output_property` for `EDID` with nothing, so under a Wayland
    // compositor every panel would lack durable evidence and
    // `accept_eligible_registrations` would decline every window, disabling reconnect recovery on
    // the whole desktop. The kernel still describes the panel the X server said nothing about.
    #[cfg(target_os = "linux")]
    let panel_evidence = panel_evidence
        .or_else(|property_error| drm_panel_evidence(&connector_name, property_error));
    MonitorEvidenceObservation {
        identity:   panel_evidence,
        attachment: connector_attachment(&connector_name),
    }
}

#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn connector_attachment(connector_name: &[u8]) -> AttachmentPath {
    std::str::from_utf8(connector_name)
        .ok()
        .and_then(|connector_name| ReportedId::new(connector_name.to_owned()).ok())
        .map_or(
            attachment_path_when_unreported(Platform::X11),
            AttachmentPath::Reported,
        )
}

#[cfg(all(unix, not(target_os = "macos")))]
fn x11_output_edid(
    connection: &XCBConnection,
    output: Output,
) -> Result<Vec<u8>, MonitorIdentificationError> {
    let edid_atom = connection
        .intern_atom(true, X11_EDID_PROPERTY_NAME)
        .map_err(|_| OperatingSystemQueryError::StableIdentityProperty)?
        .reply()
        .map_err(|_| OperatingSystemQueryError::StableIdentityProperty)?
        .atom;
    if edid_atom == NONE {
        return Err(MonitorIdentificationError::InvalidStableIdentity);
    }
    let edid = connection
        .randr_get_output_property(output, edid_atom, AtomEnum::ANY, 0, u32::MAX, false, false)
        .map_err(|_| OperatingSystemQueryError::StableIdentityProperty)?
        .reply()
        .map_err(|_| OperatingSystemQueryError::StableIdentityProperty)?;
    if edid.format != EdidEvidence::PROPERTY_FORMAT || edid.bytes_after != 0 {
        return Err(MonitorIdentificationError::InvalidStableIdentity);
    }
    Ok(edid.data)
}

/// What the kernel says about the panel on `connector_name`, or `property_error` when it describes
/// the panel no better than the `RandR` property did.
///
/// `connector_name` is the `RandR` output name, which the modesetting driver keeps identical to the
/// kernel's connector name (`eDP-1`, `HDMI-A-1`), so it selects the connector directory directly.
///
/// An `edid` file that reads back empty is the kernel stating that this panel publishes no EDID at
/// all — Apple's internal panels describe themselves through firmware instead. That is a durable
/// property of the hardware rather than a transient read failure, so a connector that
/// [`InternalConnector::identify`] accepts may be named directly at that point. A read that fails
/// makes no such statement and leaves the monitor unverified.
#[cfg(target_os = "linux")]
fn drm_panel_evidence(
    connector_name: &[u8],
    property_error: MonitorIdentificationError,
) -> Result<QualifiedEvidence, MonitorIdentificationError> {
    let Ok(connector_name) = str::from_utf8(connector_name) else {
        return Err(property_error);
    };
    let Some(connector_directory) = drm_connector_directory(connector_name) else {
        return Err(property_error);
    };
    let Ok(edid) = fs::read(connector_directory.join(DRM_CONNECTOR_EDID_FILE)) else {
        return Err(property_error);
    };
    if !edid.is_empty() {
        return EdidEvidence::qualify(edid).map(QualifiedEvidence::X11Edid);
    }
    InternalConnector::identify(connector_name).map_or(Err(property_error), |connector| {
        Ok(QualifiedEvidence::InternalConnector(connector))
    })
}

/// The [`DRM_CLASS_DIRECTORY`] entry for `connector_name`, matched on the part of the entry name
/// that follows the card it belongs to.
///
/// Returns `None` when no entry matches, and when two DRM cards expose the same connector name,
/// since nothing then says which panel the entry describes.
#[cfg(target_os = "linux")]
fn drm_connector_directory(connector_name: &str) -> Option<PathBuf> {
    let mut matched_directory = None;
    for entry in fs::read_dir(DRM_CLASS_DIRECTORY).ok()? {
        let Ok(entry) = entry else {
            continue;
        };
        let entry_name = entry.file_name();
        let Some((_, entry_connector)) = entry_name
            .to_str()
            .and_then(|entry_name| entry_name.split_once(DRM_CONNECTOR_NAME_SEPARATOR))
        else {
            continue;
        };
        if entry_connector != connector_name {
            continue;
        }
        if matched_directory.replace(entry.path()).is_some() {
            return None;
        }
    }
    matched_directory
}

#[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
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
    fn every_internal_connector_prefix_names_its_panel() {
        for prefix in DRM_INTERNAL_CONNECTOR_PREFIXES {
            let connector_name = format!("{prefix}1");

            assert!(
                InternalConnector::identify(&connector_name).is_some(),
                "{connector_name} carries a panel that cannot be exchanged"
            );
        }
    }

    #[test]
    fn connectors_that_accept_any_display_name_no_panel() {
        // `DP-1` shares its first two characters with the `DPI-` prefix, so a prefix test that
        // dropped `DRM_CONNECTOR_NAME_SEPARATOR` would treat a DisplayPort socket as a wired panel.
        for connector_name in ["DP-1", "DVI-D-1", "HDMI-A-1", "VGA-1", "Virtual-1"] {
            assert!(
                InternalConnector::identify(connector_name).is_none(),
                "{connector_name} accepts a display the user can swap"
            );
        }
    }

    #[test]
    fn internal_connector_evidence_is_tagged_and_distinct_per_connector() {
        let first = InternalConnector::identify("eDP-1").expect("eDP-1 is an internal connector");
        let second = InternalConnector::identify("eDP-2").expect("eDP-2 is an internal connector");
        let qualified = QualifiedEvidence::InternalConnector(first.clone());

        assert!(
            qualified
                .stable_bytes()
                .starts_with(DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG)
        );
        assert_ne!(first.as_bytes(), second.as_bytes());
    }

    #[test]
    fn qualified_edid_variants_retain_the_extracted_serial_tier() {
        let mut edid = vec![0_u8; EdidEvidence::BLOCK_SIZE];
        edid[..EdidEvidence::HEADER.len()].copy_from_slice(&EdidEvidence::HEADER);
        edid[EdidEvidence::SERIAL_START..EdidEvidence::SERIAL_END]
            .copy_from_slice(&42_u32.to_le_bytes());
        let checksum_index = EdidEvidence::BLOCK_SIZE - 1;
        let checksum = edid[..checksum_index]
            .iter()
            .fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
        edid[checksum_index] = 0_u8.wrapping_sub(checksum);
        let identity = qualify_edid(Ok(edid)).expect("fixture EDID should qualify");
        let windows = QualifiedEvidence::WindowsEdid(identity.clone());
        let x11 = QualifiedEvidence::X11Edid(identity);

        assert_eq!(windows.stable_bytes(), b"42");
        assert_eq!(x11.stable_bytes(), b"42");
    }

    #[test]
    fn external_connector_is_attachment_evidence_not_panel_identity() {
        let connector_name = b"HDMI-A-1";

        assert_eq!(
            connector_attachment(connector_name),
            AttachmentPath::Reported(ReportedId::new("HDMI-A-1").expect("test connector is valid"))
        );
        assert!(InternalConnector::identify("HDMI-A-1").is_none());
    }

    #[test]
    fn internal_connector_supplies_attachment_and_panel_evidence() {
        let connector_name = b"eDP-1";

        assert_eq!(
            connector_attachment(connector_name),
            AttachmentPath::Reported(ReportedId::new("eDP-1").expect("test connector is valid"))
        );
        assert!(InternalConnector::identify("eDP-1").is_some());
    }
}
