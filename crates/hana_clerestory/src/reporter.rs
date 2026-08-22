//! Reports every connected display panel to the Hana Rigging kernel as one device.
//!
//! # What identity each platform can reach
//!
//! The kernel ranks identity by what the unit itself published. Clerestory's evidence readers
//! decide which rank a panel can reach, and the answer differs per platform:
//!
//! - **macOS** reads only the `ColorSync` display UUID (`monitors::identity::native`'s
//!   `CGDisplayCreateUUIDFromDisplayID` call). It does not open the `IORegistry`, does not read
//!   `IODisplayEDID`, and never asks `IODisplayCreateInfoDictionary` for `kDisplaySerialNumber`, so
//!   no macOS display supplies a panel-published serial today. A non-null UUID reaches
//!   `DeviceIdSource::Synthesized`, which restores a saved window correctly and never authorizes
//!   output. A null result supplies no durable panel name and remains match evidence only.
//! - **Windows and X11/DRM** validate the complete EDID. A usable numeric or text EDID serial is
//!   reported under the shared `edid-serial` scheme. A validated EDID with no usable serial keeps
//!   its descriptor fingerprint and reports synthesized identity. A failed query or an unavailable,
//!   malformed, or incomplete EDID supplies no panel evidence, so it remains match evidence only
//!   rather than claiming that the unit permanently exposes no serial. On Linux, a checked internal
//!   connector may still name a non-swappable built-in panel; that synthesized identity does not
//!   claim that the panel's serial was observed to be absent.
//! - **Wayland** withholds panel evidence entirely, so its displays report match evidence with no
//!   durable name at all.
//!
//! `MonitorPlugin` registers `edid-serial` before the kernel ingests any display scan. A scheme
//! names the panel-published identity space rather than the operating system that read it.
//!
//! # Attachment evidence
//!
//! X11/DRM reports the checked connector name, such as `HDMI-A-1` or `eDP-1`, as an attachment
//! path. The same external connector never becomes panel identity. Windows' current `DisplayConfig`
//! reader resolves a path to its panel but retains no durable named port, so it reports that the
//! platform supplied no attachment. macOS does not fabricate one from `CGDirectDisplayID`, and
//! Wayland exposes no stable attachment concept through the current reader. Missing Clerestory
//! bookkeeping does not change those capabilities: missing indices, handles, or injected evidence
//! remain `PlatformReportedNothing` on Windows/X11 and `PlatformHasNoConcept` on macOS/Wayland.

#[cfg(any(test, feature = "test"))]
use bevy::prelude::Entity;
#[cfg(any(test, feature = "test"))]
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::winit::WINIT_WINDOWS;
use bevy::winit::WinitMonitors;
use hana_rigging::prelude::Capabilities;
use hana_rigging::prelude::Claim;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::DeviceDescriptor;
use hana_rigging::prelude::DeviceIdSource;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceRecord;
use hana_rigging::prelude::DeviceReporter;
use hana_rigging::prelude::DeviceScan;
use hana_rigging::prelude::Digest;
use hana_rigging::prelude::DiscoveryWork;
use hana_rigging::prelude::MainThreadDiscoveryJob;
use hana_rigging::prelude::Presence;
use hana_rigging::prelude::ReportedAs;
use hana_rigging::prelude::ReportedParent;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::SchemeNameError;

use crate::constants::EDID_SERIAL_SCHEME;
use crate::monitors::DisplayDeviceEvidence;
use crate::monitors::DisplayTopologyObservation;
use crate::monitors::EnumeratedDisplayEvidence;
use crate::monitors::PanelIdentityEvidence;

/// Enumerates connected display panels for the Hana Rigging kernel.
///
/// The reporter holds no state. Each job freshly enumerates winit's displays and joins them to
/// Clerestory's cached evidence through the exact current entity and native handle.
pub(crate) struct MonitorReporter;

impl DeviceReporter for MonitorReporter {
    /// Hand the kernel one main-thread job, because winit monitor state is not `Send`.
    ///
    /// Identity and attachment probing remains in `MonitorPlugin`; this job performs only winit's
    /// current display enumeration and exact joins to that cached evidence.
    fn discover(&mut self) -> DiscoveryWork {
        DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(|world: &mut World| {
            display_scan(world)
        }))
    }
}

#[cfg(any(test, feature = "test"))]
#[derive(Default, Resource)]
pub(crate) struct InjectedFreshWinitDisplays {
    pub(crate) entities: Vec<Entity>,
}

/// Build one complete scan from an observed topology, or retain the previous scan while startup
/// topology is still pending.
fn display_scan(world: &World) -> DeviceScan {
    let edid_serial_scheme = match edid_serial_scheme() {
        Ok(edid_serial_scheme) => edid_serial_scheme,
        Err(error) => {
            return DeviceScan::Failed(discovery_transport_error(&format!(
                "invalid built-in EDID serial scheme: {error}"
            )));
        },
    };
    match world.resource::<DisplayTopologyObservation>() {
        DisplayTopologyObservation::AwaitingInitialTopology => {
            DeviceScan::Failed(DeviceAccessError::Transport {
                detail: String::from("monitor topology has not completed its initial observation"),
            })
        },
        DisplayTopologyObservation::Observed(observed) => {
            match fresh_display_evidence(world, observed) {
                Ok(display_device_evidence) => DeviceScan::Complete(
                    display_device_evidence
                        .iter()
                        .map(|evidence| display_record(evidence, &edid_serial_scheme))
                        .collect(),
                ),
                Err(error) => DeviceScan::Failed(error),
            }
        },
    }
}

fn fresh_display_evidence(
    world: &World,
    observed: &[EnumeratedDisplayEvidence],
) -> Result<Vec<DisplayDeviceEvidence>, DeviceAccessError> {
    #[cfg(any(test, feature = "test"))]
    if let Some(injected) = world.get_resource::<InjectedFreshWinitDisplays>() {
        return injected
            .entities
            .iter()
            .map(|entity| evidence_for_entity(*entity, observed))
            .collect();
    }

    let winit_monitors = world
        .get_resource::<WinitMonitors>()
        .ok_or_else(|| discovery_transport_error("WinitMonitors is not available"))?;
    let current_handles = WINIT_WINDOWS.with_borrow(|winit_windows| {
        let window = winit_windows
            .windows
            .values()
            .next()
            .ok_or_else(|| discovery_transport_error("no current winit window is available"))?;
        Ok::<Vec<_>, DeviceAccessError>(window.available_monitors().collect())
    })?;

    current_handles
        .iter()
        .map(|current_handle| {
            let mut matches = observed.iter().filter(|entry| {
                winit_monitors
                    .find_entity(entry.entity)
                    .as_ref()
                    .is_some_and(|cached_handle| cached_handle == current_handle)
            });
            let entry = matches.next().ok_or_else(|| {
                discovery_transport_error(
                    "current winit display has no exact Clerestory evidence association",
                )
            })?;
            if matches.next().is_some() {
                return Err(discovery_transport_error(
                    "current winit display has multiple Clerestory evidence associations",
                ));
            }
            Ok(entry.device_evidence.clone())
        })
        .collect()
}

#[cfg(any(test, feature = "test"))]
fn evidence_for_entity(
    entity: Entity,
    observed: &[EnumeratedDisplayEvidence],
) -> Result<DisplayDeviceEvidence, DeviceAccessError> {
    let mut matches = observed.iter().filter(|entry| entry.entity == entity);
    let entry = matches.next().ok_or_else(|| {
        discovery_transport_error("fresh test display has no exact Clerestory evidence association")
    })?;
    if matches.next().is_some() {
        return Err(discovery_transport_error(
            "fresh test display has multiple Clerestory evidence associations",
        ));
    }
    Ok(entry.device_evidence.clone())
}

fn discovery_transport_error(detail: &str) -> DeviceAccessError {
    DeviceAccessError::Transport {
        detail: detail.to_owned(),
    }
}

/// Classification that either produces the reporter's exact device key or records its absence.
///
/// The live monitor bridge uses this same result while installing topology, so it cannot rebuild
/// a slightly different key after platform evidence or the reporter's identity rules change.
pub(crate) enum DisplayKeyClassification {
    /// The panel supplied durable evidence that can name one kernel device.
    Keyed(DeviceKey),
    /// The platform supplied match evidence only, which cannot address a monitor later.
    MatchEvidenceOnly,
}

/// Apply the reporter's durable display-key rules to one panel's evidence.
#[cfg(all(target_os = "macos", not(test)))]
pub(crate) const fn classify_display_key(
    evidence: &DisplayDeviceEvidence,
    _edid_serial_scheme: &SchemeName,
) -> DisplayKeyClassification {
    match &evidence.panel_identity {
        PanelIdentityEvidence::Synthesized {
            fingerprint: panel_fingerprint,
            ..
        } => DisplayKeyClassification::Keyed(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(panel_fingerprint.get()),
            },
        }),
        PanelIdentityEvidence::Unavailable { .. } => DisplayKeyClassification::MatchEvidenceOnly,
    }
}

/// Apply the reporter's durable display-key rules to one panel's evidence.
#[cfg(any(test, not(target_os = "macos")))]
pub(crate) fn classify_display_key(
    evidence: &DisplayDeviceEvidence,
    edid_serial_scheme: &SchemeName,
) -> DisplayKeyClassification {
    match &evidence.panel_identity {
        #[cfg(any(target_os = "windows", all(unix, not(target_os = "macos")), test))]
        PanelIdentityEvidence::ReportedSerial(value) => {
            DisplayKeyClassification::Keyed(DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Reported {
                    scheme: edid_serial_scheme.clone(),
                    value:  value.clone(),
                },
            })
        },
        PanelIdentityEvidence::Synthesized {
            fingerprint: panel_fingerprint,
            ..
        } => DisplayKeyClassification::Keyed(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(panel_fingerprint.get()),
            },
        }),
        PanelIdentityEvidence::Unavailable { .. } => DisplayKeyClassification::MatchEvidenceOnly,
    }
}

/// Report one panel, naming it durably only when its own evidence can name it.
fn display_record(
    evidence: &DisplayDeviceEvidence,
    edid_serial_scheme: &SchemeName,
) -> DeviceRecord {
    let reported_as = match classify_display_key(evidence, edid_serial_scheme) {
        DisplayKeyClassification::Keyed(device_key) => ReportedAs::Keyed(device_key),
        DisplayKeyClassification::MatchEvidenceOnly => ReportedAs::MatchEvidenceOnly,
    };
    DeviceRecord {
        reported_as,
        parent: ReportedParent::Root,
        presence: Presence::Present,
        claim: Claim::NotApplicable,
        capabilities: Capabilities::new(),
        serial: evidence.panel_identity.reported_serial(),
        platform_device_handle: evidence.platform_device_handle.clone(),
        attachment: evidence.attachment.clone(),
        descriptor: DeviceDescriptor::PlatformReportedNothing,
    }
}

pub(crate) fn edid_serial_scheme() -> Result<SchemeName, SchemeNameError> {
    SchemeName::new(EDID_SERIAL_SCHEME)
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use hana_rigging::prelude::AttachmentPath;
    use hana_rigging::prelude::Devices;
    use hana_rigging::prelude::DiscoveryCadence;
    use hana_rigging::prelude::DiscoveryControl;
    use hana_rigging::prelude::DiscoveryProgressChanged;
    use hana_rigging::prelude::IdentityChanged;
    use hana_rigging::prelude::IdentityVerdict;
    use hana_rigging::prelude::PlatformDeviceHandle;
    use hana_rigging::prelude::ReportedId;
    use hana_rigging::prelude::ReportedSerial;
    use hana_rigging::prelude::ReporterCoverage;
    use hana_rigging::prelude::ReporterRegistration;
    use hana_rigging::prelude::RiggingAppExt;
    use hana_rigging::prelude::RiggingPlugin;
    use hana_rigging::prelude::UnverifiedReason;

    use super::*;
    use crate::monitors::DisplayDeviceEvidence;
    use crate::monitors::EnumeratedDisplayEvidence;
    use crate::monitors::PanelFingerprint;

    #[derive(Default, Resource)]
    struct ProgressEventCount(usize);

    #[derive(Default, Resource)]
    struct IdentityVerdictHistory(Vec<IdentityVerdict>);

    fn count_progress_events(
        _: On<DiscoveryProgressChanged>,
        mut count: ResMut<ProgressEventCount>,
    ) {
        count.0 += 1;
    }

    fn record_identity_changes(
        event: On<IdentityChanged>,
        mut history: ResMut<IdentityVerdictHistory>,
    ) {
        history.0.push(event.verdict.clone());
    }

    fn reported_id(value: &str) -> ReportedId {
        let reported_id = ReportedId::new(value);
        assert!(reported_id.is_ok());
        match reported_id {
            Ok(reported_id) => reported_id,
            Err(error) => panic!("test reported id is invalid: {error}"),
        }
    }

    fn test_edid_serial_scheme() -> SchemeName {
        match edid_serial_scheme() {
            Ok(edid_serial_scheme) => edid_serial_scheme,
            Err(error) => panic!("test EDID serial scheme is invalid: {error}"),
        }
    }

    fn display_record_for_test(evidence: &DisplayDeviceEvidence) -> DeviceRecord {
        display_record(evidence, &test_edid_serial_scheme())
    }

    fn unavailable_evidence() -> DisplayDeviceEvidence {
        DisplayDeviceEvidence {
            panel_identity:         PanelIdentityEvidence::Unavailable {
                serial: ReportedSerial::PlatformCannotReport,
            },
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
        }
    }

    fn synthesized_evidence(bytes: &[u8]) -> DisplayDeviceEvidence {
        DisplayDeviceEvidence {
            panel_identity:         PanelIdentityEvidence::Synthesized {
                fingerprint: PanelFingerprint::from_evidence_bytes(bytes),
                serial:      ReportedSerial::NotExposedByUnit,
            },
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformReportedNothing,
        }
    }

    fn enumerated(
        entity: Entity,
        device_evidence: DisplayDeviceEvidence,
    ) -> EnumeratedDisplayEvidence {
        EnumeratedDisplayEvidence {
            entity,
            device_evidence,
        }
    }

    fn world_with_fresh_displays(
        observed: Vec<EnumeratedDisplayEvidence>,
        entities: Vec<Entity>,
    ) -> World {
        let mut world = World::new();
        world.insert_resource(DisplayTopologyObservation::Observed(observed));
        world.insert_resource(InjectedFreshWinitDisplays { entities });
        world
    }

    #[test]
    fn pending_initial_topology_is_a_failed_scan_not_authoritative_absence() {
        let mut world = World::new();
        world.init_resource::<DisplayTopologyObservation>();

        assert!(matches!(display_scan(&world), DeviceScan::Failed(_)));
    }

    #[test]
    fn observed_empty_topology_is_an_authoritative_empty_scan() {
        let world = world_with_fresh_displays(Vec::new(), Vec::new());

        let device_scan = display_scan(&world);
        assert!(matches!(&device_scan, DeviceScan::Complete(_)));
        let DeviceScan::Complete(records) = device_scan else {
            return;
        };
        assert!(records.is_empty());
    }

    #[test]
    fn reporter_uses_the_handle_retained_with_each_topology_entry() {
        let entity = Entity::from_bits(1);
        let expected_handle =
            PlatformDeviceHandle::Reported(reported_id("panel-a-window-server-handle"));
        let world = world_with_fresh_displays(
            vec![enumerated(
                entity,
                DisplayDeviceEvidence {
                    panel_identity:         PanelIdentityEvidence::Unavailable {
                        serial: ReportedSerial::PlatformCannotReport,
                    },
                    platform_device_handle: expected_handle.clone(),
                    attachment:             AttachmentPath::PlatformHasNoConcept,
                },
            )],
            vec![entity],
        );

        let device_scan = display_scan(&world);
        assert!(matches!(&device_scan, DeviceScan::Complete(_)));
        let DeviceScan::Complete(records) = device_scan else {
            return;
        };
        assert_eq!(
            records.first().map(|record| &record.platform_device_handle),
            Some(&expected_handle)
        );
    }

    #[test]
    fn reported_edid_serial_mints_reported_identity_and_serial_evidence() {
        let serial = reported_id("SN-42-A");
        let record = display_record_for_test(&DisplayDeviceEvidence {
            panel_identity:         PanelIdentityEvidence::ReportedSerial(serial.clone()),
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformReportedNothing,
        });

        assert_eq!(record.serial, ReportedSerial::Provided(serial.clone()));
        assert!(matches!(
            record.reported_as,
            ReportedAs::Keyed(DeviceKey {
                kind: DeviceKind::Display,
                id: DeviceIdSource::Reported { scheme, value },
            }) if scheme == test_edid_serial_scheme() && value == serial
        ));
    }

    #[test]
    fn serial_less_descriptor_mints_synthesized_identity() {
        let fingerprint = PanelFingerprint::from_evidence_bytes(b"validated-edid-descriptor");
        let record = display_record_for_test(&DisplayDeviceEvidence {
            panel_identity:         PanelIdentityEvidence::Synthesized {
                fingerprint,
                serial: ReportedSerial::NotExposedByUnit,
            },
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformReportedNothing,
        });

        assert_eq!(record.serial, ReportedSerial::NotExposedByUnit);
        assert!(matches!(
            record.reported_as,
            ReportedAs::Keyed(DeviceKey {
                kind: DeviceKind::Display,
                id: DeviceIdSource::Synthesized { digest },
            }) if digest == Digest::new(fingerprint.get())
        ));
    }

    #[test]
    fn unavailable_panel_evidence_remains_match_evidence_only() {
        let record = display_record_for_test(&unavailable_evidence());

        assert_eq!(record.reported_as, ReportedAs::MatchEvidenceOnly);
        assert_eq!(record.serial, ReportedSerial::PlatformCannotReport);
    }

    fn duplicate_descriptor_records() -> Vec<DeviceRecord> {
        let evidence = DisplayDeviceEvidence {
            panel_identity:         PanelIdentityEvidence::Synthesized {
                fingerprint: PanelFingerprint::from_evidence_bytes(b"same-serial-less-edid"),
                serial:      ReportedSerial::NotExposedByUnit,
            },
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformReportedNothing,
        };
        vec![
            display_record_for_test(&evidence),
            display_record_for_test(&evidence),
        ]
    }

    struct DuplicateDescriptorReporter;

    impl DeviceReporter for DuplicateDescriptorReporter {
        fn discover(&mut self) -> DiscoveryWork {
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(|_| {
                DeviceScan::Complete(duplicate_descriptor_records())
            }))
        }
    }

    #[test]
    fn identical_serial_less_panels_reach_kernel_duplicate_key_verdict() {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        app.add_device_reporter(
            DuplicateDescriptorReporter,
            ReporterRegistration::required(
                DiscoveryCadence::Periodic {
                    interval: Duration::ZERO,
                },
                ReporterCoverage::MatchingEvidenceOnly,
            ),
        );
        for _ in 0..3 {
            app.update();
        }

        let mut query = app.world_mut().query::<&IdentityVerdict>();
        assert!(query.iter(app.world()).any(|verdict| {
            *verdict == IdentityVerdict::Unverified(UnverifiedReason::NotUniqueInScan)
        }));
    }

    #[test]
    fn backstop_reads_fresh_winit_set_without_a_dirty_notification() {
        let initial_panel = Entity::from_bits(11);
        let replacement_panel = Entity::from_bits(12);
        let initial_evidence = synthesized_evidence(b"panel-a");
        let replacement_evidence = synthesized_evidence(b"panel-b");
        let initial_key = match display_record_for_test(&initial_evidence).reported_as {
            ReportedAs::Keyed(key) => key,
            ReportedAs::MatchEvidenceOnly => panic!("panel A should have a synthesized key"),
        };
        let replacement_key = match display_record_for_test(&replacement_evidence).reported_as {
            ReportedAs::Keyed(key) => key,
            ReportedAs::MatchEvidenceOnly => panic!("panel B should have a synthesized key"),
        };

        let mut app = App::new();
        app.insert_resource(DisplayTopologyObservation::Observed(vec![
            enumerated(initial_panel, initial_evidence),
            enumerated(replacement_panel, replacement_evidence),
        ]))
        .insert_resource(InjectedFreshWinitDisplays {
            entities: vec![initial_panel],
        })
        .init_resource::<ProgressEventCount>()
        .add_observer(count_progress_events)
        .add_plugins(RiggingPlugin);
        app.add_device_reporter(
            MonitorReporter,
            ReporterRegistration::required(
                DiscoveryCadence::EventDriven {
                    backstop: crate::constants::MONITOR_DISCOVERY_BACKSTOP,
                },
                ReporterCoverage::MatchingEvidenceOnly,
            ),
        );
        for _ in 0..3 {
            app.update();
        }
        assert!(
            app.world()
                .resource::<Devices>()
                .states()
                .any(|state| state.key == initial_key)
        );

        app.world_mut()
            .resource_mut::<InjectedFreshWinitDisplays>()
            .entities = vec![replacement_panel];
        std::thread::sleep(
            crate::constants::MONITOR_DISCOVERY_BACKSTOP + Duration::from_millis(50),
        );
        for _ in 0..3 {
            app.update();
        }

        let devices = app.world().resource::<Devices>();
        assert!(devices.states().any(|state| state.key == replacement_key));
        assert!(!devices.states().any(|state| state.key == initial_key));
        assert_eq!(app.world().resource::<ProgressEventCount>().0, 0);
    }

    fn panel_on_connector(panel: &[u8]) -> Vec<DeviceRecord> {
        let attachment = AttachmentPath::Reported(reported_id("HDMI-A-1"));
        let evidence = DisplayDeviceEvidence {
            attachment,
            ..synthesized_evidence(panel)
        };
        vec![display_record_for_test(&evidence)]
    }

    fn panel_a_on_connector() -> Vec<DeviceRecord> { panel_on_connector(b"panel-a") }

    fn panel_b_on_connector() -> Vec<DeviceRecord> { panel_on_connector(b"panel-b") }

    struct ChangingConnectorReporter(Arc<Mutex<fn() -> Vec<DeviceRecord>>>);

    impl DeviceReporter for ChangingConnectorReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let build = match self.0.lock() {
                Ok(build) => *build,
                Err(poisoned) => *poisoned.into_inner(),
            };
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| {
                DeviceScan::Complete(build())
            }))
        }
    }

    #[test]
    fn changed_panel_on_reported_connector_reaches_displaced_verdict() {
        let scans = Arc::new(Mutex::new(
            panel_a_on_connector as fn() -> Vec<DeviceRecord>,
        ));
        let panel_a_key = match panel_a_on_connector().remove(0).reported_as {
            ReportedAs::Keyed(key) => key,
            ReportedAs::MatchEvidenceOnly => panic!("panel A should have a synthesized key"),
        };
        let mut app = App::new();
        app.init_resource::<IdentityVerdictHistory>()
            .add_observer(record_identity_changes)
            .add_plugins(RiggingPlugin);
        let reporter = app.add_device_reporter(
            ChangingConnectorReporter(Arc::clone(&scans)),
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
            ),
        );
        for _ in 0..3 {
            app.update();
        }

        match scans.lock() {
            Ok(mut build) => *build = panel_b_on_connector,
            Err(poisoned) => *poisoned.into_inner() = panel_b_on_connector,
        }
        let request = app
            .world_mut()
            .resource_mut::<DiscoveryControl>()
            .request(reporter);
        assert!(request.is_ok());
        for _ in 0..3 {
            app.update();
        }

        assert!(
            app.world()
                .resource::<IdentityVerdictHistory>()
                .0
                .iter()
                .any(|verdict| {
                    *verdict
                        == IdentityVerdict::Displaced {
                            saved: panel_a_key.clone(),
                        }
                })
        );
    }
}
