//! Reports every connected display to the Hana Rigging kernel as one device.
//!
//! # What identity each platform can reach
//!
//! The kernel ranks identity by what the unit itself published. How far Clerestory's evidence
//! readers can read sets the rank a display reaches, and that differs per platform:
//!
//! - **macOS** reads only the `ColorSync` display UUID (`monitors::identity::native`'s
//!   `CGDisplayCreateUUIDFromDisplayID` call). It does not open the `IORegistry`, does not read
//!   `IODisplayEDID`, and never asks `IODisplayCreateInfoDictionary` for `kDisplaySerialNumber`, so
//!   no macOS display supplies a serial of its own today. A non-null UUID reaches
//!   `DeviceIdSource::Synthesized`, which restores a saved window correctly and never authorizes
//!   output. A null result supplies no durable display name and remains match evidence only.
//! - **Windows and X11/DRM** validate the complete EDID. A usable numeric or text EDID serial is
//!   reported under the shared `edid-serial` scheme. A validated EDID with no usable serial keeps
//!   its descriptor fingerprint and reports synthesized identity. A failed query or an unavailable,
//!   malformed, or incomplete EDID supplies no display evidence, so it remains match evidence only
//!   rather than reporting that the unit permanently exposes no serial. On Linux, a checked
//!   internal connector may still name a non-swappable built-in display; that synthesized identity
//!   does not mean the display's serial was read and found absent.
//! - **Wayland** withholds display evidence entirely, so its displays report match evidence with no
//!   durable name at all.
//!
//! `MonitorPlugin` registers `edid-serial` before the kernel ingests any display scan. A scheme
//! names the identity space the display itself publishes, rather than the operating system that
//! read it.
//!
//! # Attachment evidence
//!
//! X11/DRM reports the checked connector name, such as `HDMI-A-1` or `eDP-1`, as an attachment
//! path. The same external connector never becomes display identity. Windows' current
//! `DisplayConfig` reader resolves a path to its display but retains no durable named port, so it
//! reports that the platform supplied no attachment. macOS does not fabricate one from
//! `CGDirectDisplayID`, and Wayland exposes no stable attachment concept through the current
//! reader. Missing Clerestory bookkeeping does not change those capabilities: missing indices,
//! handles, or injected evidence remain `PlatformReportedNothing` on Windows/X11 and
//! `PlatformHasNoConcept` on macOS/Wayland.

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
#[cfg(test)]
use hana_rigging::prelude::DeviceIdSource;
use hana_rigging::prelude::DeviceKey;
#[cfg(test)]
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceRecord;
use hana_rigging::prelude::DeviceReporter;
use hana_rigging::prelude::DeviceScan;
#[cfg(test)]
use hana_rigging::prelude::Digest;
use hana_rigging::prelude::DiscoveryWork;
use hana_rigging::prelude::MainThreadDiscoveryJob;
use hana_rigging::prelude::Presence;
use hana_rigging::prelude::ReportedAs;
use hana_rigging::prelude::ReportedParent;
use hana_rigging::prelude::ReporterDeferral;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::SchemeNameError;

#[cfg(any(test, feature = "test"))]
use crate::constants::DISPLAY_TEST_ENUMERATION_FAILURE;
use crate::constants::EDID_SERIAL_SCHEME;
#[cfg(any(test, feature = "test"))]
use crate::display_test_adapter::DisplayTestReporterDelivery;
#[cfg(test)]
use crate::monitors::DisplayIdentityEvidence;
use crate::monitors::DisplayTopologyObservation;
use crate::monitors::EnumeratedDisplayEvidence;
use crate::monitors::LiveDisplayEndpoint;
use crate::monitors::LiveDisplayMonitor;
use crate::platform;

/// Enumerates connected displays for the Hana Rigging kernel.
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
    #[cfg(any(test, feature = "test"))]
    if matches!(
        crate::display_test_adapter::display_test_reporter_delivery(world),
        DisplayTestReporterDelivery::Fail
    ) {
        return DeviceScan::Failed(discovery_transport_error(DISPLAY_TEST_ENUMERATION_FAILURE));
    }
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
            DeviceScan::Deferred(ReporterDeferral::WaitingForTopology)
        },
        DisplayTopologyObservation::Observed(observed) => {
            match fresh_display_evidence(world, observed) {
                Ok(enumerated_displays) => DeviceScan::Complete(
                    enumerated_displays
                        .iter()
                        .map(|evidence| display_record(world, evidence, &edid_serial_scheme))
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
) -> Result<Vec<EnumeratedDisplayEvidence>, DeviceAccessError> {
    #[cfg(any(test, feature = "test"))]
    if let Some(injected) = world.get_resource::<InjectedFreshWinitDisplays>() {
        if injected.entities.is_empty() && !observed.is_empty() {
            return Err(discovery_transport_error(
                "the injected current display list is empty while observed display evidence is \
                 non-empty",
            ));
        }
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
    if current_handles.is_empty() && !observed.is_empty() {
        return Err(platform::empty_current_display_list_error(observed.len()));
    }

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
            Ok(entry.clone())
        })
        .collect()
}

#[cfg(any(test, feature = "test"))]
fn evidence_for_entity(
    entity: Entity,
    observed: &[EnumeratedDisplayEvidence],
) -> Result<EnumeratedDisplayEvidence, DeviceAccessError> {
    let mut matches = observed.iter().filter(|entry| entry.entity == entity);
    let entry = matches.next().ok_or_else(|| {
        discovery_transport_error("fresh test display has no exact Clerestory evidence association")
    })?;
    if matches.next().is_some() {
        return Err(discovery_transport_error(
            "fresh test display has multiple Clerestory evidence associations",
        ));
    }
    Ok(entry.clone())
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
    /// The display supplied durable evidence that can name one kernel device.
    Keyed(DeviceKey),
    /// The platform supplied match evidence only, which cannot address a monitor later.
    MatchEvidenceOnly,
}

/// Report one display, naming it durably only when its own evidence can name it.
fn display_record(
    world: &World,
    evidence: &EnumeratedDisplayEvidence,
    edid_serial_scheme: &SchemeName,
) -> DeviceRecord {
    let reported_as =
        match platform::classify_display_key(&evidence.device_evidence, edid_serial_scheme) {
            DisplayKeyClassification::Keyed(device_key) => ReportedAs::Keyed(device_key),
            DisplayKeyClassification::MatchEvidenceOnly => ReportedAs::MatchEvidenceOnly,
        };
    let capabilities = Capabilities::new().with(LiveDisplayEndpoint {
        monitor:         evidence.entity,
        descriptor:      evidence.descriptor,
        legacy_identity: evidence.legacy_identity,
    });
    let capabilities = if world.get_entity(evidence.entity).is_ok() {
        capabilities.with(LiveDisplayMonitor::new(evidence.entity))
    } else {
        capabilities
    };
    DeviceRecord {
        reported_as,
        parent: ReportedParent::Root,
        presence: Presence::Present,
        claim: Claim::NotApplicable,
        capabilities,
        serial: evidence.device_evidence.identity_evidence.reported_serial(),
        platform_device_handle: evidence.device_evidence.platform_device_handle.clone(),
        attachment: evidence.device_evidence.attachment.clone(),
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
    use bevy::prelude::IVec2;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Time;
    use bevy::prelude::UVec2;
    use bevy::time::Real;
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
    use crate::monitors::DisplayFingerprint;
    use crate::monitors::DisplayIdentity;
    use crate::monitors::EnumeratedDisplayEvidence;
    use crate::monitors::LiveDisplayDevices;
    use crate::monitors::MonitorDescriptor;

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
        display_record(
            &World::new(),
            &enumerated(Entity::PLACEHOLDER, evidence.clone()),
            &test_edid_serial_scheme(),
        )
    }

    fn unavailable_evidence() -> DisplayDeviceEvidence {
        DisplayDeviceEvidence {
            identity_evidence:      DisplayIdentityEvidence::Unavailable {
                serial: ReportedSerial::PlatformCannotReport,
            },
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
        }
    }

    fn synthesized_evidence(bytes: &[u8]) -> DisplayDeviceEvidence {
        DisplayDeviceEvidence {
            identity_evidence:      DisplayIdentityEvidence::Synthesized {
                display_fingerprint: DisplayFingerprint::from_evidence_bytes(bytes),
                serial:              ReportedSerial::NotExposedByUnit,
            },
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
        }
    }

    fn enumerated(
        entity: Entity,
        device_evidence: DisplayDeviceEvidence,
    ) -> EnumeratedDisplayEvidence {
        let legacy_identity = match device_evidence.identity_evidence {
            DisplayIdentityEvidence::Synthesized {
                display_fingerprint,
                ..
            } => DisplayIdentity::Fingerprinted(display_fingerprint),
            DisplayIdentityEvidence::Unavailable { .. }
            | DisplayIdentityEvidence::ReportedSerial(_) => DisplayIdentity::Anonymous,
        };
        EnumeratedDisplayEvidence {
            entity,
            descriptor: MonitorDescriptor::for_current_enumeration(0, 1.0, IVec2::ZERO, UVec2::ONE),
            device_evidence,
            legacy_identity,
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

    struct LiveMonitorRecordReporter {
        monitor: Entity,
    }

    impl DeviceReporter for LiveMonitorRecordReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let monitor = self.monitor;
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |world| {
                DeviceScan::Complete(vec![display_record(
                    world,
                    &enumerated(monitor, synthesized_evidence(b"live-monitor-record")),
                    &test_edid_serial_scheme(),
                )])
            }))
        }
    }

    #[test]
    fn pending_initial_topology_is_deferred_not_authoritative_absence() {
        let mut world = World::new();
        world.init_resource::<DisplayTopologyObservation>();

        assert!(matches!(
            display_scan(&world),
            DeviceScan::Deferred(ReporterDeferral::WaitingForTopology)
        ));
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
    fn empty_current_handles_with_observed_displays_is_a_transport_failure() {
        let world = world_with_fresh_displays(
            vec![enumerated(Entity::from_bits(1), unavailable_evidence())],
            Vec::new(),
        );

        assert!(matches!(
            display_scan(&world),
            DeviceScan::Failed(DeviceAccessError::Transport { .. })
        ));
    }

    #[test]
    fn reporter_uses_the_handle_retained_with_each_topology_entry() {
        let entity = Entity::from_bits(1);
        let expected_handle =
            PlatformDeviceHandle::Reported(reported_id("display-a-window-server-handle"));
        let world = world_with_fresh_displays(
            vec![enumerated(
                entity,
                DisplayDeviceEvidence {
                    identity_evidence:      DisplayIdentityEvidence::Unavailable {
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
    fn live_monitor_record_projects_the_monitor_relationship() -> Result<(), String> {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let monitor = app.world_mut().spawn_empty().id();
        app.add_device_reporter(
            LiveMonitorRecordReporter { monitor },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                Duration::from_secs(10),
            ),
        );
        for _ in 0..3 {
            app.update();
        }

        let live_display_devices =
            app.world()
                .get::<LiveDisplayDevices>(monitor)
                .ok_or_else(|| {
                    String::from("the live monitor has no projected display relationship")
                })?;
        let device = live_display_devices
            .device()
            .map_err(|error| format!("the live monitor does not resolve to one device: {error}"))?;
        assert!(app.world().get::<LiveDisplayMonitor>(device).is_some());
        Ok(())
    }

    #[test]
    fn reported_edid_serial_creates_reported_identity_and_serial_evidence() {
        let serial = reported_id("SN-42-A");
        let record = display_record_for_test(&DisplayDeviceEvidence {
            identity_evidence:      DisplayIdentityEvidence::ReportedSerial(serial.clone()),
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
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
    fn serial_less_descriptor_creates_synthesized_identity() {
        let fingerprint = DisplayFingerprint::from_evidence_bytes(b"validated-edid-descriptor");
        let record = display_record_for_test(&DisplayDeviceEvidence {
            identity_evidence:      DisplayIdentityEvidence::Synthesized {
                display_fingerprint: fingerprint,
                serial:              ReportedSerial::NotExposedByUnit,
            },
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
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
    fn unavailable_display_evidence_remains_match_evidence_only() {
        let record = display_record_for_test(&unavailable_evidence());

        assert_eq!(record.reported_as, ReportedAs::MatchEvidenceOnly);
        assert_eq!(record.serial, ReportedSerial::PlatformCannotReport);
    }

    fn duplicate_descriptor_records() -> Vec<DeviceRecord> {
        let evidence = DisplayDeviceEvidence {
            identity_evidence:      DisplayIdentityEvidence::Synthesized {
                display_fingerprint: DisplayFingerprint::from_evidence_bytes(
                    b"same-serial-less-edid",
                ),
                serial:              ReportedSerial::NotExposedByUnit,
            },
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment:             AttachmentPath::PlatformHasNoConcept,
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
    fn identical_serial_less_displays_reach_kernel_duplicate_key_verdict() {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        app.add_device_reporter(
            DuplicateDescriptorReporter,
            ReporterRegistration::required(
                DiscoveryCadence::Periodic {
                    interval: Duration::ZERO,
                },
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
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
        let initial_display = Entity::from_bits(11);
        let replacement_display = Entity::from_bits(12);
        let initial_evidence = synthesized_evidence(b"display-a");
        let replacement_evidence = synthesized_evidence(b"display-b");
        let initial_key = match display_record_for_test(&initial_evidence).reported_as {
            ReportedAs::Keyed(key) => key,
            ReportedAs::MatchEvidenceOnly => panic!("display A should have a synthesized key"),
        };
        let replacement_key = match display_record_for_test(&replacement_evidence).reported_as {
            ReportedAs::Keyed(key) => key,
            ReportedAs::MatchEvidenceOnly => panic!("display B should have a synthesized key"),
        };

        let mut app = App::new();
        app.insert_resource(DisplayTopologyObservation::Observed(vec![
            enumerated(initial_display, initial_evidence),
            enumerated(replacement_display, replacement_evidence),
        ]))
        .insert_resource(InjectedFreshWinitDisplays {
            entities: vec![initial_display],
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
                std::time::Duration::from_secs(10),
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
            .entities = vec![replacement_display];
        let startup = app.world().resource::<Time<Real>>().startup();
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .update_with_instant(
                startup + crate::constants::MONITOR_DISCOVERY_BACKSTOP + Duration::from_millis(50),
            );
        for _ in 0..3 {
            app.update();
        }

        let devices = app.world().resource::<Devices>();
        assert!(devices.states().any(|state| state.key == replacement_key));
        assert!(devices.states().any(|state| state.key == initial_key));
        assert_eq!(app.world().resource::<ProgressEventCount>().0, 0);
    }

    fn display_on_connector(display: &[u8]) -> Vec<DeviceRecord> {
        let attachment = AttachmentPath::Reported(reported_id("HDMI-A-1"));
        let evidence = DisplayDeviceEvidence {
            attachment,
            ..synthesized_evidence(display)
        };
        vec![display_record_for_test(&evidence)]
    }

    fn display_a_on_connector() -> Vec<DeviceRecord> { display_on_connector(b"display-a") }

    fn display_b_on_connector() -> Vec<DeviceRecord> { display_on_connector(b"display-b") }

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
    fn changed_display_on_reported_connector_reaches_displaced_verdict() {
        let scans = Arc::new(Mutex::new(
            display_a_on_connector as fn() -> Vec<DeviceRecord>,
        ));
        let display_a_key = match display_a_on_connector().remove(0).reported_as {
            ReportedAs::Keyed(key) => key,
            ReportedAs::MatchEvidenceOnly => panic!("display A should have a synthesized key"),
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
                std::time::Duration::from_secs(10),
            ),
        );
        for _ in 0..3 {
            app.update();
        }

        match scans.lock() {
            Ok(mut build) => *build = display_b_on_connector,
            Err(poisoned) => *poisoned.into_inner() = display_b_on_connector,
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
                            saved: display_a_key.clone(),
                        }
                })
        );
    }
}
