use std::ffi::c_void;

use winit::monitor::MonitorHandle;
use winit::platform::macos::MonitorHandleExtMacOS;

use super::MonitorEvidenceObservation;
use super::attachment_path_when_unreported;
use crate::Platform;
#[cfg(test)]
use crate::constants::DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG;
#[cfg(test)]
use crate::constants::DRM_INTERNAL_CONNECTOR_PREFIXES;
use crate::constants::MACOS_DISPLAY_UUID_BYTES;
use crate::monitors::identity::MonitorIdentificationError;
#[cfg(test)]
use crate::monitors::identity::OperatingSystemQueryError;
#[cfg(test)]
use crate::monitors::identity::edid::EdidEvidence;
#[cfg(test)]
use crate::monitors::identity::edid::EdidIdentityEvidence;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(in crate::monitors) enum QualifiedEvidence {
    MacOsDisplayUuid(MacOsDisplayUuid),
    #[cfg(test)]
    InternalConnector(InternalConnector),
    #[cfg(test)]
    WindowsEdid(EdidIdentityEvidence),
    #[cfg(test)]
    X11Edid(EdidIdentityEvidence),
    #[cfg(any(test, feature = "test"))]
    Synthetic(Vec<u8>),
}

impl QualifiedEvidence {
    /// Bytes that identify the physical display and nothing about this process or this boot.
    #[allow(
        clippy::missing_const_for_fn,
        reason = "const only compiles where the macOS arm is the sole arm; the Vec arms are not"
    )]
    pub(in crate::monitors::identity) fn stable_bytes(&self) -> &[u8] {
        match self {
            Self::MacOsDisplayUuid(uuid) => uuid.as_bytes(),
            #[cfg(test)]
            Self::InternalConnector(connector) => connector.as_bytes(),
            #[cfg(test)]
            Self::WindowsEdid(edid) | Self::X11Edid(edid) => edid.stable_bytes(),
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn test_only_edid_evidence_keeps_serial_tiers_and_query_errors_distinct() {
        let query_error =
            qualify_edid(Err(OperatingSystemQueryError::StableIdentityProperty.into()));
        assert_eq!(
            query_error,
            Err(MonitorIdentificationError::OperatingSystemQuery(
                OperatingSystemQueryError::StableIdentityProperty
            ))
        );

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

    fn qualify_edid(
        query: Result<Vec<u8>, MonitorIdentificationError>,
    ) -> Result<EdidIdentityEvidence, MonitorIdentificationError> {
        EdidEvidence::qualify(query?)
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
