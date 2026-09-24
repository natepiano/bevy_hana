use crate::constants::DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG;
use crate::constants::DRM_INTERNAL_CONNECTOR_PREFIXES;

/// The DRM connector name of a display built into the machine, tagged with
/// [`DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::monitors) struct InternalConnector(Vec<u8>);

impl InternalConnector {
    /// The evidence for `connector_name`, or `None` when that connector accepts a replaceable
    /// display.
    pub(super) fn identify(connector_name: &str) -> Option<Self> {
        DRM_INTERNAL_CONNECTOR_PREFIXES
            .iter()
            .any(|prefix| connector_name.starts_with(prefix))
            .then(|| {
                let mut evidence = DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG.to_vec();
                evidence.extend_from_slice(connector_name.as_bytes());
                Self(evidence)
            })
    }

    pub(super) fn as_bytes(&self) -> &[u8] { &self.0 }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

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

        assert!(
            first
                .as_bytes()
                .starts_with(DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG)
        );
        assert_ne!(first.as_bytes(), second.as_bytes());
    }
}
