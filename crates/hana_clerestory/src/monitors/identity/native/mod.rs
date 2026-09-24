#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(target_os = "windows", path = "windows.rs")]
mod selected;

use hana_rigging::prelude::AttachmentPath;
pub(in crate::monitors) use selected::QualifiedEvidence;
pub(crate) use selected::active_display_detail;
pub(in crate::monitors) use selected::monitor_evidence;
pub(in crate::monitors) use selected::platform_device_handle;
pub(in crate::monitors) use selected::product_name;

use super::MonitorIdentificationError;
use crate::Platform;

/// One monitor read with display identity kept separate from its attachment location.
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
