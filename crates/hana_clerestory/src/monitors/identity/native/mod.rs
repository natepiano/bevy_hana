#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use hana_rigging::prelude::AttachmentPath;
#[cfg(target_os = "linux")]
use linux as selected;
#[cfg(target_os = "macos")]
use macos as selected;
pub(in crate::monitors) use selected::QualifiedEvidence;
pub(in crate::monitors) use selected::monitor_evidence;
#[cfg(target_os = "windows")]
use windows as selected;

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
