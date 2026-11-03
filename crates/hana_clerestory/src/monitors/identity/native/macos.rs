use std::ffi::c_void;

use hana_rigging::prelude::PlatformDeviceHandle;
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::ReportedSerial;
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_foundation::NSNumber;
use objc2_foundation::NSString;
use winit::monitor::MonitorHandle;
use winit::platform::macos::MonitorHandleExtMacOS;

use super::MonitorEvidenceObservation;
use super::attachment_path_when_unreported;
use crate::Platform;
use crate::constants::CORE_GRAPHICS_SUCCESS;
use crate::constants::MACOS_DISPLAY_UUID_BYTES;
use crate::monitors::DisplayPhysicalSize;
use crate::monitors::DisplayProductName;
#[cfg(any(test, feature = "test"))]
use crate::monitors::identity;
use crate::monitors::identity::DisplayFingerprint;
use crate::monitors::identity::DisplayIdentityEvidence;
use crate::monitors::identity::MonitorIdentificationError;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(in crate::monitors) enum QualifiedEvidence {
    MacOsDisplayUuid(MacOsDisplayUuid),
    #[cfg(any(test, feature = "test"))]
    Synthetic(Vec<u8>),
}

impl QualifiedEvidence {
    #[expect(
        clippy::unused_self,
        reason = "same signature as the Linux and Windows arms, which read the EDID size"
    )]
    pub(in crate::monitors) const fn physical_size(&self) -> DisplayPhysicalSize {
        DisplayPhysicalSize::NotReported
    }
    /// Bytes that identify the physical display and nothing about this process or this boot.
    #[allow(
        clippy::missing_const_for_fn,
        reason = "const only compiles where the macOS arm is the sole arm; the `Synthetic` arm is not"
    )]
    pub(in crate::monitors::identity) fn stable_bytes(&self) -> &[u8] {
        match self {
            Self::MacOsDisplayUuid(uuid) => uuid.as_bytes(),
            #[cfg(any(test, feature = "test"))]
            Self::Synthetic(bytes) => bytes,
        }
    }

    /// Reporter evidence for this observation, given the `fingerprint` of its stable bytes.
    pub(in crate::monitors::identity) const fn identity_evidence(
        &self,
        fingerprint: DisplayFingerprint,
    ) -> DisplayIdentityEvidence {
        match self {
            Self::MacOsDisplayUuid(_) => DisplayIdentityEvidence::Synthesized {
                display_fingerprint: fingerprint,
                serial:              ReportedSerial::PlatformCannotReport,
            },
            #[cfg(any(test, feature = "test"))]
            Self::Synthetic(_) => identity::synthesized_without_serial(fingerprint),
        }
    }
}

/// The display's `ColorSync` UUID: 16 bytes macOS derives from the display's own vendor, model and
/// serial number, falling back to the physical port for displays that report no serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::monitors) struct MacOsDisplayUuid([u8; MACOS_DISPLAY_UUID_BYTES]);

impl MacOsDisplayUuid {
    const fn as_bytes(&self) -> &[u8] { &self.0 }
}

/// `CFUUIDBytes` in declaration order, laid out as CoreFoundation defines it.
#[repr(C)]
#[derive(Clone, Copy)]
struct CoreFoundationUuidBytes {
    bytes: [u8; MACOS_DISPLAY_UUID_BYTES],
}

#[link(name = "ColorSync", kind = "framework")]
unsafe extern "C" {
    /// Returns a retained `CFUUIDRef` for the display behind a `CGDirectDisplayID`, or null.
    fn CGDisplayCreateUUIDFromDisplayID(display_id: u32) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFUUIDGetUUIDBytes(uuid: *const c_void) -> CoreFoundationUuidBytes;
    fn CFRelease(reference: *const c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
}

/// Describe an empty winit display list with the active-display count `CoreGraphics` reports.
#[allow(
    clippy::unnecessary_wraps,
    reason = "the Linux and Windows backends share this signature and return `None`"
)]
pub(crate) fn active_display_detail(observed_count: usize) -> Option<String> {
    let mut active_display_count = 0;
    // SAFETY: a zero-capacity query writes only `display_count`; the active-display pointer is null
    // because no display identifiers are requested by this count query.
    let error_code =
        unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &raw mut active_display_count) };
    if error_code != CORE_GRAPHICS_SUCCESS {
        return Some(format!(
            "CGGetActiveDisplayList failed with CGError code {error_code} while \
             {observed_count} observed displays remained"
        ));
    }

    Some(format!(
        "winit returned no current display handles while CGGetActiveDisplayList reported \
         {active_display_count} active displays and {observed_count} observed displays remained"
    ))
}

/// Read the `ColorSync` UUID for a monitor, or report that no stable identity is available.
fn macos_display_evidence(
    handle: &MonitorHandle,
) -> Result<MacOsDisplayUuid, MonitorIdentificationError> {
    let display_id = handle.native_id();
    // SAFETY: `display_id` comes from winit's `CGDirectDisplayID`. The call returns either null or
    // a retained CFUUIDRef; both are handled, and the non-null reference is released exactly
    // once.
    let uuid_reference = unsafe { CGDisplayCreateUUIDFromDisplayID(display_id) };
    if uuid_reference.is_null() {
        return Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable);
    }
    // SAFETY: `uuid_reference` is a non-null CFUUIDRef owned by this function until `CFRelease`.
    let uuid_bytes = unsafe { CFUUIDGetUUIDBytes(uuid_reference) };
    // SAFETY: this is the final use of the retained reference.
    unsafe { CFRelease(uuid_reference) };
    Ok(MacOsDisplayUuid(uuid_bytes.bytes))
}

pub(in crate::monitors) fn monitor_evidence(
    handle: &MonitorHandle,
    platform: Platform,
) -> MonitorEvidenceObservation {
    match platform {
        Platform::MacOs => MonitorEvidenceObservation {
            identity:   macos_display_evidence(handle).map(QualifiedEvidence::MacOsDisplayUuid),
            attachment: attachment_path_when_unreported(platform),
        },
        Platform::Windows | Platform::X11 | Platform::Wayland => MonitorEvidenceObservation {
            identity:   Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
            attachment: attachment_path_when_unreported(platform),
        },
    }
}

/// Retain the exact window-server handle attached to this topology entry.
pub(in crate::monitors) fn platform_device_handle(
    monitor_handle: Option<&MonitorHandle>,
) -> PlatformDeviceHandle {
    let Some(monitor_handle) = monitor_handle else {
        return PlatformDeviceHandle::PlatformReportedNothing;
    };
    ReportedId::new(monitor_handle.native_id().to_string()).map_or(
        PlatformDeviceHandle::PlatformReportedNothing,
        PlatformDeviceHandle::Reported,
    )
}

/// Resolve one winit monitor's product name from the matching `AppKit` screen.
pub(in crate::monitors) fn product_name(
    monitor_handle: Option<&MonitorHandle>,
) -> DisplayProductName {
    let Some(monitor_handle) = monitor_handle else {
        return DisplayProductName::NotReported;
    };
    let Some(main_thread) = MainThreadMarker::new() else {
        return DisplayProductName::NotReported;
    };
    let screen_number_key = NSString::from_str("NSScreenNumber");
    let display_id = monitor_handle.native_id();
    NSScreen::screens(main_thread)
        .iter()
        .find(|screen| {
            screen
                .deviceDescription()
                .objectForKey(&screen_number_key)
                .is_some_and(|screen_number| {
                    screen_number
                        .downcast_ref::<NSNumber>()
                        .is_some_and(|screen_number| screen_number.unsignedIntValue() == display_id)
                })
        })
        .map_or(DisplayProductName::NotReported, |screen| {
            DisplayProductName::Reported(screen.localizedName().to_string())
        })
}
