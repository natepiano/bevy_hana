use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::str;

use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::ReportedId;
use winit::monitor::MonitorHandle;
use winit::platform::x11::MonitorHandleExtX11;
use x11rb::NONE;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
use x11rb::protocol::randr::Output;
use x11rb::protocol::xproto::AtomEnum;
use x11rb::protocol::xproto::ConnectionExt as XprotoConnectionExt;
use x11rb::xcb_ffi::XCBConnection;

use super::MonitorEvidenceObservation;
use super::attachment_path_when_unreported;
use crate::Platform;
use crate::constants::DRM_CLASS_DIRECTORY;
use crate::constants::DRM_CONNECTOR_EDID_FILE;
use crate::constants::DRM_CONNECTOR_NAME_SEPARATOR;
use crate::constants::DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG;
use crate::constants::DRM_INTERNAL_CONNECTOR_PREFIXES;
use crate::constants::X11_EDID_PROPERTY_NAME;
use crate::monitors::identity::MonitorIdentificationError;
use crate::monitors::identity::OperatingSystemQueryError;
use crate::monitors::identity::edid::EdidEvidence;
use crate::monitors::identity::edid::EdidIdentityEvidence;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(in crate::monitors) enum QualifiedEvidence {
    InternalConnector(InternalConnector),
    LinuxEdid(EdidIdentityEvidence),
    #[cfg(test)]
    WindowsEdid(EdidIdentityEvidence),
    #[cfg(any(test, feature = "test"))]
    Synthetic(Vec<u8>),
}

impl QualifiedEvidence {
    /// Bytes that identify the physical display and nothing about this process or this boot.
    pub(in crate::monitors::identity) fn stable_bytes(&self) -> &[u8] {
        match self {
            Self::InternalConnector(connector) => connector.as_bytes(),
            Self::LinuxEdid(edid) => edid.stable_bytes(),
            #[cfg(test)]
            Self::WindowsEdid(edid) => edid.stable_bytes(),
            #[cfg(any(test, feature = "test"))]
            Self::Synthetic(bytes) => bytes,
        }
    }
}

/// The DRM connector name of a display built into the machine, tagged with
/// [`DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::monitors) struct InternalConnector(Vec<u8>);

impl InternalConnector {
    /// The evidence for `connector_name`, or `None` when that connector accepts a replaceable
    /// display.
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

pub(in crate::monitors) fn monitor_evidence(
    handle: &MonitorHandle,
    platform: Platform,
) -> MonitorEvidenceObservation {
    match platform {
        Platform::X11 => x11_monitor_evidence(handle),
        Platform::Wayland => MonitorEvidenceObservation {
            identity:   wayland_display_evidence(
                handle.name().as_deref(),
                Path::new(DRM_CLASS_DIRECTORY),
            ),
            attachment: attachment_path_when_unreported(platform),
        },
        Platform::MacOs | Platform::Windows => MonitorEvidenceObservation {
            identity:   Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
            attachment: attachment_path_when_unreported(platform),
        },
    }
}

/// Return identity evidence for a Wayland output from the DRM connector that shares its name.
///
/// No Wayland protocol carries the EDID to a client. Plasma, GNOME, and wlroots compositors name
/// each `wl_output` after its DRM connector (`DP-3`, `HDMI-A-2`), so `output_name` only locates the
/// connector's kernel `edid` file under `drm_class_directory`; the display's identity is read from
/// that EDID. An output with no name, or a name that matches no single connector, supplies none.
fn wayland_display_evidence(
    output_name: Option<&str>,
    drm_class_directory: &Path,
) -> Result<QualifiedEvidence, MonitorIdentificationError> {
    let unresolved_error = MonitorIdentificationError::StablePhysicalIdentityUnavailable;
    output_name.map_or(Err(unresolved_error), |output_name| {
        drm_display_evidence(
            output_name.as_bytes(),
            drm_class_directory,
            unresolved_error,
        )
    })
}

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
    let display_evidence = qualify_edid(x11_output_edid(&connection, output))
        .map(QualifiedEvidence::LinuxEdid)
        .or_else(|property_error| {
            drm_display_evidence(
                &connector_name,
                Path::new(DRM_CLASS_DIRECTORY),
                property_error,
            )
        });
    MonitorEvidenceObservation {
        identity:   display_evidence,
        attachment: connector_attachment(&connector_name),
    }
}

fn connector_attachment(connector_name: &[u8]) -> AttachmentPath {
    str::from_utf8(connector_name)
        .ok()
        .and_then(|connector_name| ReportedId::new(connector_name.to_owned()).ok())
        .map_or(
            attachment_path_when_unreported(Platform::X11),
            AttachmentPath::Reported,
        )
}

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

/// Return identity evidence from the kernel connector file, or `unresolved_error` when that file
/// supplies none.
///
/// X11 reaches this when its output's EDID property is unavailable; Wayland has no other source.
fn drm_display_evidence(
    connector_name: &[u8],
    drm_class_directory: &Path,
    unresolved_error: MonitorIdentificationError,
) -> Result<QualifiedEvidence, MonitorIdentificationError> {
    let Ok(connector_name) = str::from_utf8(connector_name) else {
        return Err(unresolved_error);
    };
    let Some(connector_directory) = drm_connector_directory(connector_name, drm_class_directory)
    else {
        return Err(unresolved_error);
    };
    let Ok(edid) = fs::read(connector_directory.join(DRM_CONNECTOR_EDID_FILE)) else {
        return Err(unresolved_error);
    };
    if !edid.is_empty() {
        return EdidEvidence::qualify(edid).map(QualifiedEvidence::LinuxEdid);
    }
    InternalConnector::identify(connector_name).map_or(Err(unresolved_error), |connector| {
        Ok(QualifiedEvidence::InternalConnector(connector))
    })
}

/// The `drm_class_directory` entry whose connector segment equals `connector_name`, or `None` when
/// no entry or more than one entry matches.
fn drm_connector_directory(connector_name: &str, drm_class_directory: &Path) -> Option<PathBuf> {
    let mut matched_directory = None;
    for entry in fs::read_dir(drm_class_directory).ok()? {
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
    use tempfile::tempdir;

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
    fn every_internal_connector_prefix_names_its_display() {
        for prefix in DRM_INTERNAL_CONNECTOR_PREFIXES {
            let connector_name = format!("{prefix}1");

            assert!(InternalConnector::identify(&connector_name).is_some());
        }
    }

    #[test]
    fn connectors_that_accept_any_display_name_no_display() {
        for connector_name in ["DP-1", "DVI-D-1", "HDMI-A-1", "VGA-1", "Virtual-1"] {
            assert!(InternalConnector::identify(connector_name).is_none());
        }
    }

    #[test]
    fn internal_connector_evidence_is_tagged_and_distinct_per_connector() {
        let first = InternalConnector::identify("eDP-1").expect("eDP-1 is internal");
        let second = InternalConnector::identify("eDP-2").expect("eDP-2 is internal");
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
        let identity = qualify_edid(Ok(serial_edid())).expect("fixture EDID should qualify");
        let windows = QualifiedEvidence::WindowsEdid(identity.clone());
        let linux = QualifiedEvidence::LinuxEdid(identity);

        assert_eq!(windows.stable_bytes(), b"42");
        assert_eq!(linux.stable_bytes(), b"42");
    }

    #[test]
    fn wayland_output_name_reads_the_edid_of_its_drm_connector() {
        let drm_class_directory = tempdir().expect("temporary DRM class directory");
        write_connector_edid(drm_class_directory.path(), "card1-DP-3", &serial_edid());
        write_connector_edid(drm_class_directory.path(), "card1-HDMI-A-2", &[]);

        let identity = qualify_edid(Ok(serial_edid())).expect("fixture EDID should qualify");

        assert_eq!(
            wayland_display_evidence(Some("DP-3"), drm_class_directory.path()),
            Ok(QualifiedEvidence::LinuxEdid(identity))
        );
    }

    #[test]
    fn wayland_built_in_panel_without_an_edid_is_named_by_its_connector() {
        let drm_class_directory = tempdir().expect("temporary DRM class directory");
        write_connector_edid(drm_class_directory.path(), "card0-eDP-1", &[]);

        let connector = InternalConnector::identify("eDP-1").expect("eDP-1 is internal");

        assert_eq!(
            wayland_display_evidence(Some("eDP-1"), drm_class_directory.path()),
            Ok(QualifiedEvidence::InternalConnector(connector))
        );
    }

    #[test]
    fn wayland_output_without_exactly_one_matching_connector_supplies_no_identity() {
        let drm_class_directory = tempdir().expect("temporary DRM class directory");
        write_connector_edid(drm_class_directory.path(), "card0-DP-1", &serial_edid());
        write_connector_edid(drm_class_directory.path(), "card1-DP-1", &serial_edid());
        write_connector_edid(drm_class_directory.path(), "card1-HDMI-A-1", &[]);

        for output_name in [None, Some("DP-2"), Some("DP-1"), Some("HDMI-A-1")] {
            assert_eq!(
                wayland_display_evidence(output_name, drm_class_directory.path()),
                Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
                "output {output_name:?}"
            );
        }
    }

    fn write_connector_edid(drm_class_directory: &Path, connector_directory: &str, edid: &[u8]) {
        let connector_directory = drm_class_directory.join(connector_directory);
        fs::create_dir(&connector_directory).expect("connector directory");
        fs::write(connector_directory.join(DRM_CONNECTOR_EDID_FILE), edid)
            .expect("connector edid file");
    }

    #[test]
    fn external_connector_is_attachment_evidence_not_identity() {
        let connector_name = b"HDMI-A-1";

        assert_eq!(
            connector_attachment(connector_name),
            AttachmentPath::Reported(ReportedId::new("HDMI-A-1").expect("valid connector"))
        );
        assert!(InternalConnector::identify("HDMI-A-1").is_none());
    }

    #[test]
    fn internal_connector_supplies_attachment_and_display_evidence() {
        let connector_name = b"eDP-1";

        assert_eq!(
            connector_attachment(connector_name),
            AttachmentPath::Reported(ReportedId::new("eDP-1").expect("valid connector"))
        );
        assert!(InternalConnector::identify("eDP-1").is_some());
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
