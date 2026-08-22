//! The hardware-free rule suite.
//!
//! Every case here drives the kernel through `hana_rigging_scripted::ScriptedReporter`, which
//! replays a written list of whole-set scans. No test touches hardware, and no test mocks anything
//! but the scan list and the driver: a rule that cannot be reached this way has put input or output
//! inside the kernel, which is the defect this suite exists to catch.

use std::error::Error;
use std::num::NonZeroU32;
use std::time::Duration;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::On;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::reflect::structs::Struct;
use bevy::time::Real;
use bevy::time::Time;
use hana_rigging::prelude::AdoptionOutcome;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::ApplyPermit;
use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::AttemptFinished;
use hana_rigging::prelude::AttemptId;
use hana_rigging::prelude::AttemptLookup;
use hana_rigging::prelude::AttemptOutcome;
use hana_rigging::prelude::AttemptProgress;
use hana_rigging::prelude::Attempts;
use hana_rigging::prelude::AuthoritativeReporterCoverage;
use hana_rigging::prelude::Binding;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::CapabilitiesDisputed;
use hana_rigging::prelude::CaptureOutcome;
use hana_rigging::prelude::Claim;
use hana_rigging::prelude::ClaimChanged;
use hana_rigging::prelude::ClaimHolder;
use hana_rigging::prelude::CompletedDiscoveryOutcome;
use hana_rigging::prelude::ConfiguredDevice;
use hana_rigging::prelude::ConfiguredDeviceConnection;
use hana_rigging::prelude::ConfiguredDeviceConnectionChanged;
use hana_rigging::prelude::ConfiguredDeviceMode;
use hana_rigging::prelude::CoveredDeviceIdentitySpace;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::DeviceArrived;
use hana_rigging::prelude::DeviceDeparted;
use hana_rigging::prelude::DeviceDeparture;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DeviceIdSource;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceResolution;
use hana_rigging::prelude::DeviceStateLookup;
use hana_rigging::prelude::Devices;
use hana_rigging::prelude::Digest;
use hana_rigging::prelude::DiscoveryBatchId;
use hana_rigging::prelude::DiscoveryCadence;
use hana_rigging::prelude::DiscoveryFinished;
use hana_rigging::prelude::DiscoveryLimits;
use hana_rigging::prelude::DiscoveryProgress;
use hana_rigging::prelude::DiscoveryProgressChanged;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::IdentityAdoptionPreparation;
use hana_rigging::prelude::IdentityAnswer;
use hana_rigging::prelude::IdentityChanged;
use hana_rigging::prelude::IdentityDecisionOwed;
use hana_rigging::prelude::IdentityDecisions;
use hana_rigging::prelude::IdentityQuestion;
use hana_rigging::prelude::IdentityQuestionExpired;
use hana_rigging::prelude::IdentityQuestionLookup;
use hana_rigging::prelude::IdentityQuestionRaised;
use hana_rigging::prelude::IdentityQuestionState;
use hana_rigging::prelude::IdentityVerdict;
use hana_rigging::prelude::LastKnownGoodConfiguration;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::PartName;
use hana_rigging::prelude::Presence;
use hana_rigging::prelude::PresenceChanged;
use hana_rigging::prelude::ReapplyConfiguration;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::RecoveryPolicyChanged;
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::ReporterActivation;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::RequestedConfiguration;
use hana_rigging::prelude::RetireRole;
use hana_rigging::prelude::RetiredRoleAttemptEnded;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingLimits;
use hana_rigging::prelude::RiggingPlugin;
use hana_rigging::prelude::RoleAvailable;
use hana_rigging::prelude::RoleAwaiting;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleState;
use hana_rigging::prelude::RoleStateChanged;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::StartupDiscoveryChanged;
use hana_rigging::prelude::StartupDiscoveryState;
use hana_rigging::prelude::UnregisteredSchemeReported;
use hana_rigging_scripted::ScriptedDevice;
use hana_rigging_scripted::ScriptedReporter;
use hana_rigging_scripted::ScriptedScan;
use hana_rigging_scripted::advance_reporter;
use hana_rigging_scripted::advance_until_accepted;
use hana_rigging_scripted::advance_until_running;
use hana_rigging_scripted::reported_key;
use hana_rigging_scripted::scan;

/// Identity space every scripted panel in this suite is named in.
const PANEL_SCHEME: &str = "usb-serial";

/// Slot the scripted driver reports the endpoint is already sitting in.
const CAPTURED_SLOT: u32 = 7;

/// Slot every scripted role asks its driver to move the endpoint to.
const REQUESTED_SLOT: u32 = 1;

/// Transport text the scripted driver reports when a case asks it to fail an apply.
const DRIVER_FAILURE_DETAIL: &str = "the scripted driver was asked to refuse this apply";

/// Transport text a scripted reporter reports when a case asks it to fail an enumeration.
const DISCOVERY_FAILURE_DETAIL: &str = "the scripted reporter was asked to fail this scan";

/// Placement the scripted role asks its driver for.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct PanelPlacement {
    slot: u32,
}

/// What the scripted driver was asked to do, in dispatch order.
///
/// Written through the `World` the driver is handed rather than kept in the driver value, because
/// the driver lives inside the kernel's registry and the test never sees it again after
/// registration.
#[derive(Default, Resource)]
struct DriverCalls {
    /// One entry per `start_apply`, holding the identifier the kernel minted for it.
    started:            Vec<AttemptId>,
    /// How many more polls answer with a failure before the driver starts converging.
    ///
    /// Zero for every case that is not about retry, so a test opts into failure rather than
    /// spelling out a second driver type whose only difference is its first answer.
    failures_remaining: usize,
}

/// Driver that records every dispatch and reports the apply converged on the first poll.
///
/// Its readback answers `CaptureOutcome::Read` rather than `CaptureOutcome::NotReadable` because a
/// role whose endpoint exposes nothing never establishes a `LastKnownGoodConfiguration::Known`, and
/// `RecoveryPolicy::ReapplyOnReturn` has no saved value to owe back on a departure — so an
/// unreadable scripted endpoint would silently disable every recovery case in this suite.
struct RecordingDriver;

impl EndpointDriver for RecordingDriver {
    type Configuration = PanelPlacement;

    fn capture(
        &mut self,
        _: &mut World,
        _: &DeviceEndpoint,
    ) -> CaptureOutcome<Self::Configuration> {
        CaptureOutcome::Read(PanelPlacement {
            slot: CAPTURED_SLOT,
        })
    }

    fn start_apply(
        &mut self,
        world: &mut World,
        _: &DeviceEndpoint,
        _: &Self::Configuration,
        attempt: AttemptId,
        _: ApplyPermit,
    ) {
        world.resource_mut::<DriverCalls>().started.push(attempt);
    }

    fn poll(&mut self, world: &mut World, _: AttemptId) -> AttemptProgress {
        let mut driver_calls = world.resource_mut::<DriverCalls>();
        if driver_calls.failures_remaining == 0 {
            return AttemptProgress::Finished(AttemptOutcome::Succeeded);
        }
        driver_calls.failures_remaining -= 1;

        AttemptProgress::Finished(AttemptOutcome::Failed(DeviceAccessError::Transport {
            detail: DRIVER_FAILURE_DETAIL.to_owned(),
        }))
    }
}

/// Every event this suite observes, in arrival order per axis.
#[derive(Default, Resource)]
struct ObservedEvents {
    presences:          Vec<Presence>,
    claims:             Vec<Claim>,
    role_availability:  Vec<ObservedRoleAvailability>,
    arrivals:           Vec<DeviceKey>,
    departures:         Vec<(DeviceKey, DeviceDeparture)>,
    role_state:         Vec<RoleState>,
    schemes:            Vec<SchemeName>,
    connections:        Vec<(DeviceKey, ConfiguredDeviceConnection)>,
    attempt_outcomes:   Vec<AttemptOutcome>,
    startup:            Vec<StartupDiscoveryState>,
    discovery_progress: Vec<ObservedDiscoveryProgress>,
    discovery_finished: Vec<(DiscoveryBatchId, ReporterId, CompletedDiscoveryOutcome)>,
}

/// Which side of a role's availability edge one observed event reported.
///
/// One ordered list rather than two, because the rule under test is the alternation: a role that
/// went available, back to awaiting, and available again is indistinguishable from one that never
/// left if each side is counted on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ObservedRoleAvailability {
    /// `RoleAwaiting`: nothing live sits behind the role's endpoint.
    Awaiting(RoleKey),
    /// `RoleAvailable`: a present device sits behind the role's endpoint.
    Available(RoleKey),
}

/// Every identity question raised or expired, in arrival order.
///
/// Kept apart from `ObservedEvents` because the identity-question events are the two stated
/// exceptions to the derived-from-mirrored-components rule, and a suite that reads them out of the
/// same record as the derived events would blur that line.
#[derive(Default, Resource)]
struct ObservedQuestions {
    raised:  Vec<(RoleKey, DeviceKey)>,
    expired: Vec<(RoleKey, DeviceKey)>,
}

/// What one `DiscoveryProgressChanged` carried.
///
/// Kept as a named record rather than a tuple because the four counts are all `usize` and a test
/// comparing them positionally would pass with any two of them swapped.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedDiscoveryProgress {
    batch:     DiscoveryBatchId,
    reporter:  ReporterId,
    progress:  DiscoveryProgress,
    completed: usize,
    total:     usize,
    running:   usize,
    queued:    usize,
}

fn observe_presence(event: On<PresenceChanged>, mut observed: ResMut<ObservedEvents>) {
    observed.presences.push(event.presence);
}

fn observe_claim(event: On<ClaimChanged>, mut observed: ResMut<ObservedEvents>) {
    observed.claims.push(event.claim.clone());
}

fn observe_arrival(event: On<DeviceArrived>, mut observed: ResMut<ObservedEvents>) {
    observed.arrivals.push(event.key.clone());
}

fn observe_departure(event: On<DeviceDeparted>, mut observed: ResMut<ObservedEvents>) {
    observed
        .departures
        .push((event.key.clone(), event.departure));
}

fn observe_role_awaiting(event: On<RoleAwaiting>, mut observed: ResMut<ObservedEvents>) {
    observed
        .role_availability
        .push(ObservedRoleAvailability::Awaiting(event.role.clone()));
}

fn observe_role_available(event: On<RoleAvailable>, mut observed: ResMut<ObservedEvents>) {
    observed
        .role_availability
        .push(ObservedRoleAvailability::Available(event.role.clone()));
}

fn observe_role_state(event: On<RoleStateChanged>, mut observed: ResMut<ObservedEvents>) {
    observed.role_state.push(event.state);
}

fn observe_scheme(event: On<UnregisteredSchemeReported>, mut observed: ResMut<ObservedEvents>) {
    observed.schemes.push(event.scheme.clone());
}

fn observe_connection(
    event: On<ConfiguredDeviceConnectionChanged>,
    mut observed: ResMut<ObservedEvents>,
) {
    observed
        .connections
        .push((event.key.clone(), event.connection));
}

fn observe_attempt_finished(event: On<AttemptFinished>, mut observed: ResMut<ObservedEvents>) {
    observed.attempt_outcomes.push(event.outcome.clone());
}

fn observe_startup(event: On<StartupDiscoveryChanged>, mut observed: ResMut<ObservedEvents>) {
    observed.startup.push(event.state.clone());
}

fn observe_discovery_progress(
    event: On<DiscoveryProgressChanged>,
    mut observed: ResMut<ObservedEvents>,
) {
    observed.discovery_progress.push(ObservedDiscoveryProgress {
        batch:     event.batch,
        reporter:  event.reporter,
        progress:  event.progress.clone(),
        completed: event.completed,
        total:     event.total,
        running:   event.running,
        queued:    event.queued,
    });
}

fn observe_discovery_finished(event: On<DiscoveryFinished>, mut observed: ResMut<ObservedEvents>) {
    observed
        .discovery_finished
        .push((event.batch, event.reporter, event.outcome.clone()));
}

fn observe_question_raised(
    event: On<IdentityQuestionRaised>,
    mut observed: ResMut<ObservedQuestions>,
) {
    observed
        .raised
        .push((event.role.clone(), event.candidate.clone()));
}

fn observe_question_expired(
    event: On<IdentityQuestionExpired>,
    mut observed: ResMut<ObservedQuestions>,
) {
    observed
        .expired
        .push((event.role.clone(), event.candidate.clone()));
}

/// Build an app with the kernel, the observers, and one scripted reporter that establishes absence.
fn scripted_app(scans: Vec<ScriptedScan>) -> Result<(App, ReporterId), Box<dyn Error>> {
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new(scans),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
        ),
    );

    Ok((app, reporter))
}

/// Build the same app around a reporter startup is not allowed to proceed without.
///
/// A separate builder rather than a flag on `scripted_app`: `ReporterRegistration::required`
/// carries no activation argument, because a reporter startup waits for cannot also be one the
/// application leaves disabled.
fn required_scripted_app(scans: Vec<ScriptedScan>) -> Result<(App, ReporterId), Box<dyn Error>> {
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new(scans),
        ReporterRegistration::required(DiscoveryCadence::OnDemand, panel_coverage()?),
    );

    Ok((app, reporter))
}

/// The kernel, the scripted scheme, and every observer this suite reads events through.
fn observing_app() -> Result<App, Box<dyn Error>> {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(RiggingPlugin)
        .init_resource::<DriverCalls>()
        .init_resource::<ObservedEvents>()
        .init_resource::<ObservedQuestions>()
        .register_device_scheme(SchemeName::new(PANEL_SCHEME)?)
        .add_observer(observe_presence)
        .add_observer(observe_claim)
        .add_observer(observe_arrival)
        .add_observer(observe_departure)
        .add_observer(observe_role_awaiting)
        .add_observer(observe_role_available)
        .add_observer(observe_role_state)
        .add_observer(observe_scheme)
        .add_observer(observe_connection)
        .add_observer(observe_attempt_finished)
        .add_observer(observe_startup)
        .add_observer(observe_discovery_progress)
        .add_observer(observe_discovery_finished)
        .add_observer(observe_question_raised)
        .add_observer(observe_question_expired);

    Ok(app)
}

/// Coverage that makes a scripted reporter's omission of a panel key mean the unit is gone.
///
/// Authoritative rather than `ReporterCoverage::MatchingEvidenceOnly`: an evidence-only reporter's
/// omission of a key proves nothing, so it can produce no departure, no `DeviceDeparted`, and no
/// absent-then-present reacquisition — the three things most of this suite is about.
fn panel_coverage() -> Result<ReporterCoverage, Box<dyn Error>> {
    Ok(ReporterCoverage::EstablishesAbsence(
        AuthoritativeReporterCoverage::one(CoveredDeviceIdentitySpace::ReportedScheme {
            kind:   DeviceKind::HidPanel,
            scheme: SchemeName::new(PANEL_SCHEME)?,
        }),
    ))
}

/// Register one role against a scripted panel key, with an authored or defaulted apply deadline.
fn register_role(
    app: &mut App,
    role: &RoleKey,
    device: DeviceKey,
    recovery: RecoveryPolicy,
    retry: RetryOn,
    apply_deadline: ApplyDeadline,
) -> Result<(), Box<dyn Error>> {
    let driver = app.add_endpoint_driver(RecordingDriver);
    app.world_mut()
        .resource_mut::<Bindings>()
        .register(Binding {
            role: role.clone(),
            endpoint: DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            driver,
            recovery,
            retry,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::default(),
            requested: RequestedConfiguration::new(PanelPlacement {
                slot: REQUESTED_SLOT,
            }),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline,
        })?;

    Ok(())
}

fn panel_key(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
    Ok(reported_key(DeviceKind::HidPanel, PANEL_SCHEME, value)?)
}

/// A panel key the reporter synthesized from location evidence, rather than one the unit itself
/// reported.
const fn synthesized_panel_key() -> DeviceKey {
    DeviceKey {
        kind: DeviceKind::HidPanel,
        id:   DeviceIdSource::Synthesized {
            digest: Digest::new(0x5AFE_D001),
        },
    }
}

/// A record naming a scheme nobody registered must reach a consumer rather than vanishing.
///
/// Ingest rejects the record, so it produces no device, no mirrored component, and therefore no
/// event under the derived-from-mirrored-components rule. Without this event a reporter author
/// debugging a typo'd scheme has nothing at all to look at.
#[test]
fn an_unregistered_scheme_reaches_a_consumer() -> Result<(), Box<dyn Error>> {
    let mistyped = reported_key(DeviceKind::HidPanel, "usb-seral", "CL15")?;
    let (mut app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(mistyped)]])?;

    advance_reporter(&mut app, reporter)?;

    let scheme = SchemeName::new("usb-seral")?;
    assert!(
        app.world()
            .resource::<Devices>()
            .unregistered_schemes()
            .contains(&scheme)
    );
    assert_eq!(
        app.world().resource::<ObservedEvents>().schemes,
        vec![scheme]
    );

    Ok(())
}

/// A device nobody else can use must not be seized, however much the role wants it.
///
/// `RecoveryPolicy::ReapplyOnReturn` is the variant that acts on its own, so it is the one that
/// would take a contended unit if the claim were not consulted.
#[test]
fn a_contended_device_does_not_reacquire() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let (mut app, reporter) = scripted_app(vec![scan![
        ScriptedDevice::present(key.clone()).with_claim(Claim::Contended {
            holder: ClaimHolder::Unidentified,
        })
    ]])?;
    let role = RoleKey::new("panel")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::ReapplyOnReturn,
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, reporter)?;
    app.update();

    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.state,
        RoleState::Waiting
    );
    assert!(app.world().resource::<Attempts>().is_empty());
    assert!(app.world().resource::<DriverCalls>().started.is_empty());

    Ok(())
}

/// A unit that leaves and returns must be applied again, and the departure must name it.
#[test]
fn an_absent_then_present_cycle_reacquires() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let (mut app, reporter) = scripted_app(vec![
        scan![ScriptedDevice::present(key.clone())],
        scan![],
        scan![ScriptedDevice::present(key.clone())],
    ])?;
    let role = RoleKey::new("panel")?;
    register_role(
        &mut app,
        &role,
        key.clone(),
        RecoveryPolicy::ReapplyOnReturn,
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, reporter)?;
    app.update();
    let applies_after_arrival = app.world().resource::<DriverCalls>().started.len();

    advance_reporter(&mut app, reporter)?;
    app.update();

    advance_reporter(&mut app, reporter)?;
    for _ in 0..16 {
        if app.world().resource::<DriverCalls>().started.len() > applies_after_arrival {
            break;
        }
        app.update();
    }

    let observed = app.world().resource::<ObservedEvents>();
    assert_eq!(applies_after_arrival, 1);
    // Twice: a `DeviceDeparture::KeyLeftTheSet` despawns the device entity, so the unit's return
    // spawns a new one and arrives again. The durable key is what ties the two together.
    assert_eq!(observed.arrivals, vec![key.clone(), key.clone()]);
    assert_eq!(
        observed
            .departures
            .iter()
            .map(|(departed_key, _)| departed_key.clone())
            .collect::<Vec<_>>(),
        vec![key]
    );
    assert_eq!(app.world().resource::<DriverCalls>().started.len(), 2);

    Ok(())
}

/// A frame in which nothing moved must emit nothing at all.
///
/// The mirrors are written only when a value differs, so a settled frame leaves every mirror
/// untouched and the event stage has nothing to derive. A mirror that rewrote an equal value would
/// make every once-per-change consumer fire forever, which is what this case guards.
#[test]
fn a_settled_frame_emits_nothing() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let (mut app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(key)]])?;

    advance_reporter(&mut app, reporter)?;
    app.update();
    let settled_from = observed_counts(&app);
    app.update();
    app.update();

    assert_eq!(observed_counts(&app), settled_from);

    Ok(())
}

/// Every mirrored axis must fire exactly once per change and never on a repeat of the same value.
#[test]
fn a_mirrored_axis_fires_once_per_change() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let (mut app, reporter) = scripted_app(vec![
        scan![ScriptedDevice::present(key.clone()).with_claim(Claim::Free)],
        scan![ScriptedDevice::present(key.clone()).with_claim(Claim::Free)],
        scan![ScriptedDevice::present(key).with_claim(Claim::Held)],
    ])?;

    for _ in 0..3 {
        advance_reporter(&mut app, reporter)?;
        app.update();
    }

    let observed = app.world().resource::<ObservedEvents>();
    assert_eq!(observed.presences, vec![Presence::Present]);
    assert_eq!(observed.claims, vec![Claim::Free, Claim::Held]);

    Ok(())
}

/// An authored per-binding deadline must reach the attempt.
///
/// One process drives endpoints with genuinely different costs, so a single process-wide bound
/// either abandons the slow one or lets the fast one hang.
#[test]
fn an_authored_apply_deadline_is_stamped_on_the_attempt() -> Result<(), Box<dyn Error>> {
    let authored = Duration::from_secs(5);
    assert_eq!(
        rounded_deadline_gap(ApplyDeadline::Authored(authored))?,
        authored
    );

    Ok(())
}

/// A binding that authors no deadline of its own must be stamped with the process-wide one.
///
/// `ApplyDeadline::ProcessDefault` is the variant almost every binding uses, so a fallback that
/// silently stamped nothing — or stamped a hard-coded constant of its own — would leave the
/// kernel's one configurable bound unreachable for every ordinary role.
#[test]
fn an_unauthored_apply_deadline_falls_back_to_the_process_default() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        rounded_deadline_gap(ApplyDeadline::ProcessDefault)?,
        RiggingLimits::default().apply_deadline
    );

    Ok(())
}

/// A key duplicated inside one scan must be suppressed without suppressing the rest of the scan.
///
/// The whole point of the rule is that it is per key: a reporter that repeats one weakly-identified
/// panel is still telling the truth about every other unit it named, and a pass that dropped them
/// all would turn one ambiguous webcam into a total discovery outage.
#[test]
fn duplicate_key_suppression_is_per_key() -> Result<(), Box<dyn Error>> {
    let duplicated = panel_key("CL15")?;
    let single = panel_key("CL16")?;
    let (mut app, reporter) = scripted_app(vec![scan![
        ScriptedDevice::present(duplicated.clone()),
        ScriptedDevice::present(duplicated.clone()),
        ScriptedDevice::present(single.clone()),
    ]])?;
    let duplicated_role = RoleKey::new("duplicated-panel")?;
    let single_role = RoleKey::new("single-panel")?;
    register_role(
        &mut app,
        &duplicated_role,
        duplicated.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;
    register_role(
        &mut app,
        &single_role,
        single.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, reporter)?;

    let devices = app.world().resource::<Devices>();
    assert!(devices.duplicate_keys().contains(&duplicated));
    assert!(!devices.duplicate_keys().contains(&single));
    assert_eq!(
        app.world()
            .resource::<Bindings>()
            .binding(&duplicated_role)?
            .state,
        RoleState::Waiting
    );
    // One dispatch, and the suppressed role is the one still waiting, so it was the unsuppressed
    // key that reached a driver.
    assert_eq!(app.world().resource::<DriverCalls>().started.len(), 1);

    Ok(())
}

/// Progress must stay silent until a run outlasts `DiscoveryLimits::progress_after`, and the
/// per-reporter and aggregate views of the same run must never disagree.
///
/// The delay is what keeps an interface from flashing a progress indicator for every scan that
/// finishes in a millisecond. The agreement matters because both events are derived from one
/// recorded transition: a second derivation would let the reporter row and the summary bar report
/// different batches.
#[test]
fn discovery_progress_waits_for_its_delay_then_agrees_across_both_views()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let scripted_progress = DiscoveryProgress::Measured {
        completed: 1,
        total:     NonZeroU32::new(4).ok_or("four is not zero")?,
    };
    // Held on the I/O pool rather than run inline: an immediate scan finishes inside the admission
    // call that started it, so the kernel never retains a running activity to progress at all.
    let (scripted_reporter, gate) = ScriptedReporter::gated(
        vec![scan![ScriptedDevice::present(key)]],
        scripted_progress.clone(),
    );
    let mut app = observing_app()?;
    // Longer than the case can run, so the silent half is decided by the delay and not by timing.
    app.world_mut()
        .resource_mut::<DiscoveryLimits>()
        .set_progress_after(Duration::from_hours(1));
    let reporter = app.add_device_reporter(
        scripted_reporter,
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
        ),
    );

    advance_until_running(&mut app, reporter)?;

    {
        let observed = app.world().resource::<ObservedEvents>();
        assert!(observed.discovery_progress.is_empty());
    }

    app.world_mut()
        .resource_mut::<DiscoveryLimits>()
        .set_progress_after(Duration::ZERO);
    app.update();

    {
        let observed = app.world().resource::<ObservedEvents>();
        assert!(!observed.discovery_progress.is_empty());
        for observed_progress in &observed.discovery_progress {
            assert_eq!(observed_progress.reporter, reporter);
            assert_eq!(observed_progress.progress, scripted_progress);
            assert_eq!(observed_progress.total, 1);
            assert_eq!(
                observed_progress.completed + observed_progress.running + observed_progress.queued,
                1
            );
        }
    }

    gate.release();
    advance_until_accepted(&mut app, reporter)?;

    Ok(())
}

/// A required reporter's failure must close startup, and its later success must open it.
///
/// Startup readiness is the one discovery fact the rest of an application gates on, so a failure
/// that left it in `StartupDiscoveryState::Discovering` would read as a slow probe forever and no
/// consumer would ever learn the enumeration is broken.
#[test]
fn a_required_discovery_failure_blocks_startup_until_a_later_success() -> Result<(), Box<dyn Error>>
{
    let key = panel_key("CL15")?;
    let (mut app, reporter) = required_scripted_app(vec![
        ScriptedScan::Failed(DeviceAccessError::Transport {
            detail: DISCOVERY_FAILURE_DETAIL.to_owned(),
        }),
        scan![ScriptedDevice::present(key)],
    ])?;

    advance_reporter(&mut app, reporter)?;

    let blocked = app
        .world()
        .resource::<ObservedEvents>()
        .startup
        .last()
        .cloned()
        .ok_or("a required reporter's failure announced no startup state")?;
    assert!(matches!(
        blocked,
        StartupDiscoveryState::BlockedByFailure {
            reporter: blocked_by,
            ..
        } if blocked_by == reporter
    ));

    assert!(matches!(
        app.world()
            .resource::<ObservedEvents>()
            .discovery_finished
            .last(),
        Some((_, finished_by, CompletedDiscoveryOutcome::Failed { .. }))
            if *finished_by == reporter
    ));

    advance_reporter(&mut app, reporter)?;

    let observed = app.world().resource::<ObservedEvents>();
    assert_eq!(observed.startup.last(), Some(&StartupDiscoveryState::Ready));
    assert!(matches!(
        observed.discovery_finished.last(),
        Some((_, _, CompletedDiscoveryOutcome::Succeeded { .. }))
    ));

    Ok(())
}

/// An authored device the application marked offline must still report its connectivity, and no
/// driver may be touched for it.
///
/// The two halves are the whole point of `ConfiguredDeviceMode::Offline`: a user interface has to
/// be able to show a configured-but-disabled fixture as plugged in, and the kernel has to keep its
/// promise that nothing drives it while it is offline.
#[test]
fn an_offline_authored_device_reports_connection_without_touching_its_driver()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let (mut app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(key.clone())]])?;
    app.world_mut()
        .resource_mut::<HardwareInventory>()
        .configure(ConfiguredDevice {
            key:  key.clone(),
            mode: ConfiguredDeviceMode::Offline,
        });
    let role = RoleKey::new("panel")?;
    register_role(
        &mut app,
        &role,
        key.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, reporter)?;
    app.update();

    assert_eq!(
        app.world()
            .resource::<HardwareInventory>()
            .connection(&key)?,
        ConfiguredDeviceConnection::Present
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(
        observed
            .connections
            .contains(&(key, ConfiguredDeviceConnection::Present))
    );
    assert!(observed.attempt_outcomes.is_empty());
    assert!(app.world().resource::<DriverCalls>().started.is_empty());

    Ok(())
}

/// A key no reporter names any more must depart as `DeviceDeparture::KeyLeftTheSet`, and the event
/// must still name it after its entity is gone.
#[test]
fn a_key_that_left_the_set_departs_after_its_entity_is_despawned() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let departure = departure_after(&key, scan![])?;

    assert_eq!(departure.key, key);
    assert_eq!(departure.departure, DeviceDeparture::KeyLeftTheSet);
    assert_eq!(departure.retained_devices, 0);

    Ok(())
}

/// A key a reporter still names while reporting the unit gone must depart as
/// `DeviceDeparture::RetainedButNotPresent` with its device entity alive, and the two departure
/// kinds must be told apart from the event payload alone.
///
/// A consumer of the despawning kind has no entity left to inspect, so if the payload did not carry
/// the distinction it could not be recovered from anywhere at all.
#[test]
fn a_retained_but_absent_key_departs_with_its_entity_alive() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let retained = departure_after(&key, scan![ScriptedDevice::absent(key.clone())])?;
    let left_the_set = departure_after(&key, scan![])?;

    assert_eq!(retained.key, key);
    assert_eq!(retained.departure, DeviceDeparture::RetainedButNotPresent);
    assert_eq!(retained.retained_devices, 1);
    assert_ne!(retained.departure, left_the_set.departure);

    Ok(())
}

/// A role whose key is still reported while its unit is gone must be announced as awaiting again,
/// and as available again only when the unit comes back.
///
/// The retained-but-absent scan is the case the two events are worth having for: the key still
/// resolves to a retained device, so an availability edge derived from resolution alone reports the
/// role as available for the whole outage and never re-announces it on the return. What decides the
/// edge is whether the resolved device is `Presence::Present`.
#[test]
fn role_availability_reopens_while_a_retained_key_has_no_present_unit() -> Result<(), Box<dyn Error>>
{
    let key = panel_key("CL15")?;
    let (mut app, reporter) = scripted_app(vec![
        scan![ScriptedDevice::present(key.clone())],
        scan![ScriptedDevice::absent(key.clone())],
        scan![ScriptedDevice::present(key.clone())],
    ])?;
    let role = RoleKey::new("panel")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::ReapplyOnReturn,
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;

    assert_eq!(
        app.world().resource::<ObservedEvents>().role_availability,
        vec![
            ObservedRoleAvailability::Awaiting(role.clone()),
            ObservedRoleAvailability::Available(role.clone()),
            ObservedRoleAvailability::Awaiting(role.clone()),
            ObservedRoleAvailability::Available(role),
        ]
    );

    Ok(())
}

/// What one scripted departure reported, and how much device state outlived it.
struct ObservedDeparture {
    key:              DeviceKey,
    departure:        DeviceDeparture,
    retained_devices: usize,
}

/// Introduce `key`, then replace the scan with `second` and report the one departure that follows.
fn departure_after(
    key: &DeviceKey,
    second: ScriptedScan,
) -> Result<ObservedDeparture, Box<dyn Error>> {
    let (mut app, reporter) =
        scripted_app(vec![scan![ScriptedDevice::present(key.clone())], second])?;

    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;

    let (departed_key, departure) = app
        .world()
        .resource::<ObservedEvents>()
        .departures
        .first()
        .cloned()
        .ok_or("the scripted scan change announced no departure")?;

    Ok(ObservedDeparture {
        key: departed_key,
        departure,
        retained_devices: app.world().resource::<Devices>().count(),
    })
}

/// A role that fails and restarts inside one frame must announce both moves.
///
/// The state is announced from the apply stage rather than from a mirrored component precisely so
/// this is visible: a once-per-frame mirror would compare `RoleState::Applying` against
/// `RoleState::Applying`, conclude nothing moved, and hide the failure that ran between them along
/// with the retry it caused.
#[test]
fn a_role_that_fails_and_restarts_in_one_frame_announces_both_moves() -> Result<(), Box<dyn Error>>
{
    let key = panel_key("CL15")?;
    let (mut app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(key.clone())]])?;
    let role = RoleKey::new("panel")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::default(),
        RetryOn::Interval(Duration::ZERO),
        ApplyDeadline::ProcessDefault,
    )?;
    app.world_mut()
        .resource_mut::<DriverCalls>()
        .failures_remaining = 1;

    advance_reporter(&mut app, reporter)?;

    let announced_before = app.world().resource::<ObservedEvents>().role_state.len();
    app.update();

    assert!(matches!(
        &app.world().resource::<ObservedEvents>().role_state[announced_before..],
        [RoleState::Waiting, RoleState::Applying(_)]
    ));

    Ok(())
}

/// Start one apply under `apply_deadline` and report how far past the start its deadline sits.
///
/// Rounded to whole seconds because the stamp is `start + deadline` and the start is however many
/// milliseconds into the run the dispatch happened; the fact under test is which of the two
/// durations was used, not the frame it landed on.
fn rounded_deadline_gap(apply_deadline: ApplyDeadline) -> Result<Duration, Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let (mut app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(key.clone())]])?;
    let role = RoleKey::new("panel")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        apply_deadline,
    )?;

    advance_reporter(&mut app, reporter)?;

    let started = app
        .world()
        .resource::<DriverCalls>()
        .started
        .first()
        .copied()
        .ok_or("the scripted driver was never dispatched")?;
    let startup = app.world().resource::<Time<Real>>().startup();
    let AttemptLookup::InFlight(attempt) = app.world().resource::<Attempts>().in_flight(started)
    else {
        return Err("the attempt ended before its deadline could be read".into());
    };

    Ok(Duration::from_secs(
        attempt.deadline.duration_since(startup).as_secs(),
    ))
}

/// Every event type must resolve through the registry, so a consumer can watch the whole lifecycle
/// over the Bevy Remote Protocol without a line of application code.
///
/// The workspace `bevy` enables `reflect_auto_register`, so this builds a bare app and reads the
/// registry rather than calling `App::register_type`.
#[test]
fn every_event_type_resolves_through_the_type_registry() {
    let app = App::new();
    let type_registry = app.world().resource::<AppTypeRegistry>().read();

    for type_id in [
        std::any::TypeId::of::<DeviceArrived>(),
        std::any::TypeId::of::<PresenceChanged>(),
        std::any::TypeId::of::<ClaimChanged>(),
        std::any::TypeId::of::<IdentityChanged>(),
        std::any::TypeId::of::<RoleStateChanged>(),
        std::any::TypeId::of::<RecoveryPolicyChanged>(),
        std::any::TypeId::of::<ReapplyConfiguration>(),
        std::any::TypeId::of::<DeviceDeparted>(),
        std::any::TypeId::of::<RetireRole>(),
        std::any::TypeId::of::<RoleAwaiting>(),
        std::any::TypeId::of::<RoleAvailable>(),
        std::any::TypeId::of::<ConfiguredDeviceConnectionChanged>(),
        std::any::TypeId::of::<UnregisteredSchemeReported>(),
        std::any::TypeId::of::<AttemptFinished>(),
        std::any::TypeId::of::<RetiredRoleAttemptEnded>(),
        std::any::TypeId::of::<CapabilitiesDisputed>(),
        std::any::TypeId::of::<DiscoveryProgressChanged>(),
        std::any::TypeId::of::<DiscoveryFinished>(),
        std::any::TypeId::of::<StartupDiscoveryChanged>(),
    ] {
        assert!(type_registry.get(type_id).is_some());
    }
    drop(type_registry);
}

/// Read how many of each observed event have arrived so far.
fn observed_counts(app: &App) -> (usize, usize, usize, usize, usize) {
    let observed = app.world().resource::<ObservedEvents>();
    (
        observed.presences.len(),
        observed.claims.len(),
        observed.arrivals.len(),
        observed.departures.len(),
        observed.role_state.len(),
    )
}

/// Transport address the saved unit and every candidate are reported at, which is what makes an
/// arriving unit look like it took the departed one's place.
const SHARED_SLOT: &str = "usb-bus-1-port-4";

/// Independent transport address used by grouped identity-answer tests.
const SECONDARY_SLOT: &str = "usb-bus-1-port-5";

/// One unit reported at the address every identity question in this suite turns on.
fn at_shared_slot(device_key: DeviceKey) -> Result<ScriptedDevice, Box<dyn Error>> {
    at_slot(device_key, SHARED_SLOT)
}

/// Place one scripted unit at a reported transport address.
fn at_slot(device_key: DeviceKey, slot: &str) -> Result<ScriptedDevice, Box<dyn Error>> {
    Ok(ScriptedDevice::present(device_key)
        .with_attachment(AttachmentPath::Reported(ReportedId::new(slot)?)))
}

/// Register a role against a scripted panel with the settings the identity cases share.
fn register_panel_role(
    app: &mut App,
    role: &RoleKey,
    device: DeviceKey,
) -> Result<(), Box<dyn Error>> {
    register_role(
        app,
        role,
        device,
        RecoveryPolicy::ReapplyOnReturn,
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )
}

/// Register one independently addressed part of a scripted panel.
fn register_panel_part_role(
    app: &mut App,
    role: &RoleKey,
    device: DeviceKey,
    part: &str,
) -> Result<(), Box<dyn Error>> {
    let driver = app.add_endpoint_driver(RecordingDriver);
    app.world_mut()
        .resource_mut::<Bindings>()
        .register(Binding {
            role: role.clone(),
            endpoint: DeviceEndpoint {
                device,
                id: EndpointId::Part(PartName::new(part)?),
            },
            driver,
            recovery: RecoveryPolicy::ReapplyOnReturn,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::default(),
            requested: RequestedConfiguration::new(PanelPlacement {
                slot: REQUESTED_SLOT,
            }),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline: ApplyDeadline::ProcessDefault,
        })?;

    Ok(())
}

/// Scripted state with two roles displaced as one device group and one unrelated displacement.
struct GroupedIdentityFixture {
    app:                 App,
    first:               RoleKey,
    second:              RoleKey,
    unrelated:           RoleKey,
    saved:               DeviceKey,
    candidate:           DeviceKey,
    unrelated_saved:     DeviceKey,
    unrelated_candidate: DeviceKey,
}

#[derive(Debug, PartialEq, Eq)]
struct IdentityQuestionReading {
    role:      RoleKey,
    saved:     DeviceKey,
    candidate: DeviceKey,
    state:     IdentityQuestionState,
}

/// Raise two same-pair questions plus one question at another saved/candidate pair.
fn grouped_identity_fixture() -> Result<GroupedIdentityFixture, Box<dyn Error>> {
    let saved = panel_key("GROUP-SAVED")?;
    let candidate = panel_key("GROUP-CANDIDATE")?;
    let unrelated_saved = panel_key("OTHER-SAVED")?;
    let unrelated_candidate = panel_key("OTHER-CANDIDATE")?;
    let (mut app, reporter) = scripted_app(vec![
        ScriptedScan::Complete(vec![
            at_slot(saved.clone(), SHARED_SLOT)?,
            at_slot(unrelated_saved.clone(), SECONDARY_SLOT)?,
        ]),
        ScriptedScan::Complete(vec![
            at_slot(candidate.clone(), SHARED_SLOT)?,
            at_slot(unrelated_candidate.clone(), SECONDARY_SLOT)?,
        ]),
    ])?;
    let first = RoleKey::new("group/first")?;
    let second = RoleKey::new("group/second")?;
    let unrelated = RoleKey::new("unrelated")?;
    register_panel_part_role(&mut app, &first, saved.clone(), "first")?;
    register_panel_part_role(&mut app, &second, saved.clone(), "second")?;
    register_panel_role(&mut app, &unrelated, unrelated_saved.clone())?;
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;

    Ok(GroupedIdentityFixture {
        app,
        first,
        second,
        unrelated,
        saved,
        candidate,
        unrelated_saved,
        unrelated_candidate,
    })
}

fn identity_question_reading(app: &App) -> Vec<IdentityQuestionReading> {
    questions(app)
        .iter()
        .map(|question| IdentityQuestionReading {
            role:      question.role.clone(),
            saved:     question.saved.clone(),
            candidate: question.candidate.clone(),
            state:     question.state,
        })
        .collect()
}

/// Drive the saved unit in, then a candidate into the address it left, with a role bound to the
/// saved key throughout — the port geometry that puts a saved key up for replacement.
///
/// The pass is left unsettled: callers that need the replacement concluded call `update`
/// themselves.
fn replaced_at_shared_slot(
    saved: DeviceKey,
) -> Result<(App, ReporterId, RoleKey, DeviceKey, DeviceKey), Box<dyn Error>> {
    let candidate = panel_key("CANDIDATE-UNIT")?;
    let (mut app, reporter) = scripted_app(vec![
        ScriptedScan::Complete(vec![at_shared_slot(saved.clone())?]),
        ScriptedScan::Complete(vec![at_shared_slot(candidate.clone())?]),
    ])?;
    let role = RoleKey::new("panel")?;
    register_panel_role(&mut app, &role, saved.clone())?;
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;

    Ok((app, reporter, role, saved, candidate))
}

/// The replacement with a saved key the unit itself reported — the one situation that raises an
/// identity question.
fn displaced_app() -> Result<(App, ReporterId, RoleKey, DeviceKey, DeviceKey), Box<dyn Error>> {
    replaced_at_shared_slot(panel_key("SAVED-UNIT")?)
}

fn questions(app: &App) -> &[IdentityQuestion] {
    app.world().resource::<IdentityDecisions>().questions()
}

fn observed_questions(app: &App) -> &ObservedQuestions {
    app.world().resource::<ObservedQuestions>()
}

fn bound_device(app: &App, role: &RoleKey) -> Result<DeviceKey, Box<dyn Error>> {
    Ok(app
        .world()
        .resource::<Bindings>()
        .binding(role)?
        .endpoint
        .device
        .clone())
}

/// A saved key that stops matching while a same-kind unit holds its attachment is the whole
/// premise of the register, and re-raising it every pass would make it unanswerable.
#[test]
fn a_displaced_saved_key_raises_one_question_and_never_raises_it_again()
-> Result<(), Box<dyn Error>> {
    let (mut app, reporter, role, saved, candidate) = displaced_app()?;

    assert_eq!(questions(&app).len(), 1);
    let question = &questions(&app)[0];
    assert_eq!(question.role, role);
    assert_eq!(question.saved, saved);
    assert_eq!(question.candidate, candidate);
    assert_eq!(question.state, IdentityQuestionState::Unseen);
    assert_eq!(observed_questions(&app).raised.len(), 1);

    advance_reporter(&mut app, reporter)?;
    app.update();

    assert_eq!(questions(&app).len(), 1);
    assert_eq!(observed_questions(&app).raised.len(), 1);
    assert!(observed_questions(&app).expired.is_empty());

    Ok(())
}

/// An adoption that moved the binding but left the authored entry behind would leave the operator
/// configuring one unit and driving another.
#[test]
fn adopting_rewrites_the_binding_endpoint_and_the_authored_entry_together()
-> Result<(), Box<dyn Error>> {
    let (mut app, _, role, saved, candidate) = displaced_app()?;
    app.world_mut()
        .resource_mut::<HardwareInventory>()
        .configure(ConfiguredDevice {
            key:  saved.clone(),
            mode: ConfiguredDeviceMode::Managed,
        });

    let outcome = app.world_mut().resource_mut::<IdentityDecisions>().answer(
        &role,
        &candidate,
        IdentityAnswer::Adopt,
    );
    assert_eq!(outcome, AdoptionOutcome::Adopted);

    app.update();

    assert_eq!(bound_device(&app, &role)?, candidate);
    let inventory = app.world().resource::<HardwareInventory>();
    assert!(inventory.configured_device(&candidate).is_ok());
    assert!(inventory.configured_device(&saved).is_err());
    assert!(questions(&app).is_empty());

    // `Bindings::readdress` puts the role back in `RoleState::Waiting`, so the binding entity's
    // resolved-device link catches up on the following frame rather than inside the adoption.
    app.update();

    assert!(matches!(
        resolved_device(&app, &role),
        RoleDeviceResolution::Resolved(resolved) if resolved == candidate
    ));

    Ok(())
}

/// A device integration owns the knowledge that several roles address one physical unit, so one
/// prepared adoption must answer every exact-match question in that supplied role set while
/// retaining questions about another displacement.
#[test]
fn a_prepared_device_adoption_answers_its_complete_exact_match_set() -> Result<(), Box<dyn Error>> {
    let GroupedIdentityFixture {
        mut app,
        first,
        second,
        unrelated,
        saved,
        candidate,
        unrelated_saved,
        unrelated_candidate,
    } = grouped_identity_fixture()?;
    assert_eq!(questions(&app).len(), 3);
    assert!(questions(&app).iter().any(|question| {
        question.role == first && question.saved == saved && question.candidate == candidate
    }));
    assert!(questions(&app).iter().any(|question| {
        question.role == second && question.saved == saved && question.candidate == candidate
    }));

    let preparation = app
        .world()
        .resource::<IdentityDecisions>()
        .prepare_device_adoption(&first, &candidate, [&first, &second]);
    let IdentityAdoptionPreparation::Prepared(prepared) = preparation else {
        return Err("the complete exact-match set should prepare".into());
    };
    let outcome = app
        .world_mut()
        .resource_mut::<IdentityDecisions>()
        .answer_prepared_adoption(prepared);

    assert_eq!(outcome, AdoptionOutcome::Adopted);
    assert_eq!(questions(&app).len(), 1);
    let standing = &questions(&app)[0];
    assert_eq!(standing.role, unrelated);
    assert_eq!(standing.saved, unrelated_saved);
    assert_eq!(standing.candidate, unrelated_candidate);
    assert_eq!(bound_device(&app, &first)?, saved);
    assert_eq!(bound_device(&app, &second)?, saved);
    app.update();
    assert_eq!(bound_device(&app, &first)?, candidate);
    assert_eq!(bound_device(&app, &second)?, candidate);
    assert_eq!(bound_device(&app, &unrelated)?, unrelated_saved);

    Ok(())
}

/// A conflict on any sibling question must refuse the prepared set before the register removes
/// the selected question or any earlier sibling.
#[test]
fn a_device_adoption_preflight_refusal_changes_no_matching_question() -> Result<(), Box<dyn Error>>
{
    let GroupedIdentityFixture {
        mut app,
        first,
        second,
        unrelated: _,
        saved,
        candidate,
        unrelated_saved: _,
        unrelated_candidate: _,
    } = grouped_identity_fixture()?;
    let owner = RoleKey::new("candidate/second-owner")?;
    register_panel_part_role(&mut app, &owner, candidate.clone(), "second")?;
    app.update();
    let questions_before = identity_question_reading(&app);

    let preparation = app
        .world()
        .resource::<IdentityDecisions>()
        .prepare_device_adoption(&first, &candidate, [&first, &second]);

    assert!(matches!(
        preparation,
        IdentityAdoptionPreparation::Refused(AdoptionOutcome::CandidateEndpointOwned {
            by
        }) if by == owner
    ));
    assert_eq!(identity_question_reading(&app), questions_before);
    assert_eq!(bound_device(&app, &first)?, saved);
    assert_eq!(bound_device(&app, &second)?, saved);

    Ok(())
}

/// A prepared answer names the complete checked question set, so changing one sibling before the
/// answer reaches the register must not let the remaining sibling adopt alone.
#[test]
fn a_prepared_device_adoption_refuses_a_changed_matching_set() -> Result<(), Box<dyn Error>> {
    let GroupedIdentityFixture {
        mut app,
        first,
        second,
        unrelated: _,
        saved,
        candidate,
        unrelated_saved: _,
        unrelated_candidate: _,
    } = grouped_identity_fixture()?;
    let preparation = app
        .world()
        .resource::<IdentityDecisions>()
        .prepare_device_adoption(&first, &candidate, [&first, &second]);
    let IdentityAdoptionPreparation::Prepared(prepared) = preparation else {
        return Err("the initial complete exact-match set should prepare".into());
    };
    assert_eq!(
        app.world_mut().resource_mut::<IdentityDecisions>().answer(
            &second,
            &candidate,
            IdentityAnswer::Reject
        ),
        AdoptionOutcome::Refused
    );
    let questions_before_batch = identity_question_reading(&app);

    let outcome = app
        .world_mut()
        .resource_mut::<IdentityDecisions>()
        .answer_prepared_adoption(prepared);

    assert_eq!(outcome, AdoptionOutcome::NoSuchQuestion);
    assert_eq!(identity_question_reading(&app), questions_before_batch);
    assert_eq!(bound_device(&app, &first)?, saved);
    assert_eq!(bound_device(&app, &second)?, saved);

    Ok(())
}

/// How the role a question was answered for resolves once the adoption has been applied.
///
/// A named result rather than an optional key: a role whose binding never moved and a role whose
/// endpoint moved onto a unit the kernel does not retain are different failures, and a test reading
/// "no key" as one of them would pass for the other.
#[derive(Debug, PartialEq, Eq)]
enum RoleDeviceResolution {
    /// The role's endpoint names a key the kernel does not currently retain.
    NotResolved,
    /// The role's endpoint names this retained key.
    Resolved(DeviceKey),
}

/// Read which retained device one role's binding currently addresses.
fn resolved_device(app: &App, role: &RoleKey) -> RoleDeviceResolution {
    let Ok(binding) = app.world().resource::<Bindings>().binding(role) else {
        return RoleDeviceResolution::NotResolved;
    };
    let device = binding.endpoint.device.clone();
    match app.world().resource::<Devices>().resolve(&device) {
        DeviceResolution::NotResolved => RoleDeviceResolution::NotResolved,
        DeviceResolution::Resolved(_) => RoleDeviceResolution::Resolved(device),
    }
}

/// A unit that displaces a key nobody bound a role to is the case the register cannot ask about, so
/// pinning it would leave hardware unusable for the life of the process with no question to answer.
#[test]
fn a_displaced_unit_no_role_addresses_is_authorizable_without_an_answer()
-> Result<(), Box<dyn Error>> {
    let saved = panel_key("SAVED-UNIT")?;
    let candidate = panel_key("CANDIDATE-UNIT")?;
    let (mut app, reporter) = scripted_app(vec![
        ScriptedScan::Complete(vec![at_shared_slot(saved)?]),
        ScriptedScan::Complete(vec![at_shared_slot(candidate.clone())?]),
    ])?;
    advance_reporter(&mut app, reporter)?;

    // The scan that carries the arriving unit is also the frame that discharges its debt, and
    // `Devices::discharge_identity_decision` concludes the verdict from the candidate's own key at
    // the same time. No third scan follows, because a further pass through the merge is what would
    // conclude the verdict anyway and hide a discharge that left it `IdentityVerdict::Displaced`.
    advance_reporter(&mut app, reporter)?;

    assert!(questions(&app).is_empty());
    assert!(observed_questions(&app).raised.is_empty());
    let devices = app.world().resource::<Devices>();
    let DeviceResolution::Resolved(device_id) = devices.resolve(&candidate) else {
        return Err("the arriving unit must resolve".into());
    };
    devices.authorize_service(device_id)?;

    Ok(())
}

/// A reporter that stops reporting a unit present keeps its key in the identity map, so a question
/// about it would otherwise stand forever and an adoption onto absent hardware would stay possible.
#[test]
fn a_question_expires_when_its_candidate_stops_being_present() -> Result<(), Box<dyn Error>> {
    let saved = panel_key("SAVED-UNIT")?;
    let candidate = panel_key("CANDIDATE-UNIT")?;
    let (mut app, reporter) = scripted_app(vec![
        ScriptedScan::Complete(vec![at_shared_slot(saved.clone())?]),
        ScriptedScan::Complete(vec![at_shared_slot(candidate.clone())?]),
        ScriptedScan::Complete(vec![ScriptedDevice::absent(candidate.clone())]),
    ])?;
    let role = RoleKey::new("panel")?;
    register_panel_role(&mut app, &role, saved)?;
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;
    assert_eq!(questions(&app).len(), 1);

    advance_reporter(&mut app, reporter)?;

    // The key is still in the identity map — this is the retained-but-not-present departure, not an
    // unplugged one — so only a presence read can tell the question it has nothing left to answer.
    assert!(matches!(
        app.world().resource::<Devices>().resolve(&candidate),
        DeviceResolution::Resolved(_)
    ));
    assert!(questions(&app).is_empty());
    assert_eq!(observed_questions(&app).expired, vec![(role, candidate)]);

    Ok(())
}

/// One entry per frame in which a register a settled frame must not touch was written.
#[derive(Default, Debug, PartialEq, Eq, Resource)]
struct SettledRegisterWrites {
    devices:            usize,
    identity_decisions: usize,
}

fn count_settled_register_writes(
    devices: Res<Devices>,
    identity_decisions: Res<IdentityDecisions>,
    mut settled_register_writes: ResMut<SettledRegisterWrites>,
) {
    settled_register_writes.devices += usize::from(devices.is_changed());
    settled_register_writes.identity_decisions += usize::from(identity_decisions.is_changed());
}

/// An answer is the one thing that leaves a permanent record behind, so it is where a register that
/// re-offers its own history would start marking itself changed on every later frame.
#[test]
fn frames_after_an_answer_write_neither_register() -> Result<(), Box<dyn Error>> {
    let (mut app, _, role, _, candidate) = displaced_app()?;
    app.init_resource::<SettledRegisterWrites>()
        .add_systems(bevy::app::PostUpdate, count_settled_register_writes);

    app.world_mut().resource_mut::<IdentityDecisions>().answer(
        &role,
        &candidate,
        IdentityAnswer::Reject,
    );
    for _ in 0..3 {
        app.update();
    }
    *app.world_mut().resource_mut::<SettledRegisterWrites>() = SettledRegisterWrites::default();
    for _ in 0..3 {
        app.update();
    }

    assert_eq!(
        *app.world().resource::<SettledRegisterWrites>(),
        SettledRegisterWrites::default()
    );

    Ok(())
}

/// An adopted unit must be usable at once: a reporter that only rescans when the operating system
/// reports a change can leave the operator staring at hardware they just accepted and cannot drive.
#[test]
fn an_adopted_unit_is_authorized_on_the_frame_its_answer_is_applied() -> Result<(), Box<dyn Error>>
{
    let (mut app, _, role, _, candidate) = displaced_app()?;
    let outcome = app.world_mut().resource_mut::<IdentityDecisions>().answer(
        &role,
        &candidate,
        IdentityAnswer::Adopt,
    );
    assert_eq!(outcome, AdoptionOutcome::Adopted);

    // The one frame that applies the answer, and no further scan: `advance_reporter` is what would
    // hide the defect, because the merge concludes the verdict again on any pass that ingests one.
    app.update();

    let devices = app.world().resource::<Devices>();
    let DeviceResolution::Resolved(device_id) = devices.resolve(&candidate) else {
        return Err("the adopted unit must resolve".into());
    };
    assert!(matches!(
        devices.state(device_id),
        DeviceStateLookup::Retained(state) if state.verdict == IdentityVerdict::Proven
    ));
    devices.authorize_service(device_id)?;

    Ok(())
}

/// A frame that neither expires a standing question nor changes one must leave the register alone,
/// or every consumer watching `IdentityDecisions` through change detection wakes up at frame rate
/// for as long as one question stands.
#[test]
fn frames_on_which_a_question_merely_stands_write_neither_register() -> Result<(), Box<dyn Error>> {
    let (mut app, _, _, _, _) = displaced_app()?;
    app.init_resource::<SettledRegisterWrites>()
        .add_systems(bevy::app::PostUpdate, count_settled_register_writes);

    app.update();
    assert_eq!(questions(&app).len(), 1);
    *app.world_mut().resource_mut::<SettledRegisterWrites>() = SettledRegisterWrites::default();
    for _ in 0..3 {
        app.update();
    }

    assert_eq!(questions(&app).len(), 1);
    assert_eq!(
        *app.world().resource::<SettledRegisterWrites>(),
        SettledRegisterWrites::default()
    );

    Ok(())
}

/// Two roles on one endpoint is the invariant `Bindings` exists to hold, so an adoption cannot be
/// the one move allowed to break it.
#[test]
fn adopting_an_endpoint_another_role_owns_changes_nothing_and_names_the_owner()
-> Result<(), Box<dyn Error>> {
    let (mut app, reporter, role, saved, candidate) = displaced_app()?;
    let owner = RoleKey::new("owning-panel")?;
    register_panel_role(&mut app, &owner, candidate.clone())?;
    advance_reporter(&mut app, reporter)?;

    let outcome = app.world_mut().resource_mut::<IdentityDecisions>().answer(
        &role,
        &candidate,
        IdentityAnswer::Adopt,
    );

    assert_eq!(
        outcome,
        AdoptionOutcome::CandidateEndpointOwned { by: owner }
    );
    assert_eq!(questions(&app).len(), 1);
    assert_eq!(bound_device(&app, &role)?, saved);

    Ok(())
}

/// A refusal has to be permanent for the unit it names and silent about every other unit, or the
/// operator answers the same question forever.
#[test]
fn rejecting_removes_the_entry_and_a_third_unit_raises_a_new_question() -> Result<(), Box<dyn Error>>
{
    let saved = panel_key("SAVED-UNIT")?;
    let candidate = panel_key("CANDIDATE-UNIT")?;
    let third = panel_key("THIRD-UNIT")?;
    let (mut app, reporter) = scripted_app(vec![
        ScriptedScan::Complete(vec![at_shared_slot(saved.clone())?]),
        ScriptedScan::Complete(vec![at_shared_slot(candidate.clone())?]),
        ScriptedScan::Complete(vec![at_shared_slot(third.clone())?]),
    ])?;
    let role = RoleKey::new("panel")?;
    register_panel_role(&mut app, &role, saved)?;
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;

    let outcome = app.world_mut().resource_mut::<IdentityDecisions>().answer(
        &role,
        &candidate,
        IdentityAnswer::Reject,
    );
    assert_eq!(outcome, AdoptionOutcome::Refused);
    app.update();
    assert_eq!(questions(&app).len(), 1);
    assert_eq!(questions(&app)[0].candidate, third);

    // The opening `RoleAwaiting` edge requested the scan that introduced `third`; an explicit
    // rescan repeats it and must not raise the same question again.
    advance_reporter(&mut app, reporter)?;

    assert_eq!(questions(&app).len(), 1);
    assert_eq!(questions(&app)[0].candidate, third);
    assert_eq!(observed_questions(&app).raised.len(), 2);

    Ok(())
}

/// Deferring is the answer "not now", which has to stop the prompt without discarding the question.
#[test]
fn a_deferred_question_stays_in_the_register_and_stops_being_re_raised()
-> Result<(), Box<dyn Error>> {
    let (mut app, reporter, role, _, candidate) = displaced_app()?;

    let deferred = matches!(
        app.world_mut()
            .resource_mut::<IdentityDecisions>()
            .defer(&role, &candidate),
        IdentityQuestionLookup::Pending(_)
    );
    assert!(deferred);

    advance_reporter(&mut app, reporter)?;

    assert_eq!(questions(&app).len(), 1);
    assert_eq!(questions(&app)[0].state, IdentityQuestionState::Deferred);
    assert_eq!(observed_questions(&app).raised.len(), 1);

    Ok(())
}

/// A question about a unit that left names a candidate nobody can adopt.
#[test]
fn a_question_expires_when_the_candidate_unit_departs() -> Result<(), Box<dyn Error>> {
    let saved = panel_key("SAVED-UNIT")?;
    let candidate = panel_key("CANDIDATE-UNIT")?;
    let (mut app, reporter) = scripted_app(vec![
        ScriptedScan::Complete(vec![at_shared_slot(saved.clone())?]),
        ScriptedScan::Complete(vec![at_shared_slot(candidate.clone())?]),
        ScriptedScan::Complete(Vec::new()),
    ])?;
    let role = RoleKey::new("panel")?;
    register_panel_role(&mut app, &role, saved)?;
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;
    assert_eq!(questions(&app).len(), 1);

    advance_reporter(&mut app, reporter)?;

    assert!(questions(&app).is_empty());
    assert_eq!(observed_questions(&app).expired, vec![(role, candidate)]);

    Ok(())
}

/// A retired role has no endpoint to rebind, so its question has nothing left to answer.
#[test]
fn a_question_expires_when_its_role_is_retired() -> Result<(), Box<dyn Error>> {
    let (mut app, _, role, _, candidate) = displaced_app()?;

    app.world_mut().resource_mut::<Bindings>().retire(&role)?;
    app.update();

    assert!(questions(&app).is_empty());
    assert_eq!(observed_questions(&app).expired, vec![(role, candidate)]);

    Ok(())
}

/// Expiry means "nobody can answer this", which an answered question is not.
#[test]
fn an_answered_question_expires_nothing() -> Result<(), Box<dyn Error>> {
    let (mut app, _, role, _, candidate) = displaced_app()?;

    app.world_mut().resource_mut::<IdentityDecisions>().answer(
        &role,
        &candidate,
        IdentityAnswer::Adopt,
    );
    app.update();

    assert!(questions(&app).is_empty());
    assert!(observed_questions(&app).expired.is_empty());

    Ok(())
}

/// A synthesized key was never a claim about the unit's own identity, so its failure to match says
/// nothing an operator could rule on — and the device must not be pinned waiting for one.
#[test]
fn a_synthesized_saved_key_raises_no_question() -> Result<(), Box<dyn Error>> {
    let saved = synthesized_panel_key();
    let (mut app, ..) = replaced_at_shared_slot(saved)?;
    app.update();

    assert!(questions(&app).is_empty());
    assert!(observed_questions(&app).raised.is_empty());

    Ok(())
}

/// Whether a replaced unit is adoptable turns on the *kind* of evidence behind the saved key.
///
/// Both halves drive the identical port geometry through one builder — a unit leaves the shared
/// reported address, a same-kind unit arrives into it, a role bound to the departed key throughout
/// — and differ only in whether the saved key is a serial the unit itself reported or one the
/// reporter synthesized from location evidence.
///
/// The departed-slot join fires the same way for both: it owes a human decision on any
/// non-authored saved key that left the slot. `answerable` separates them one system later, and
/// the discharge that follows restores the scanned verdict — so the synthesized half ends
/// observably indistinguishable from a unit that displaced nothing, which is the intent: it must
/// not sit pinned on a question nobody can answer. The reported half is what holds the join: if
/// the join stopped firing, the standing question and the outstanding `Displaced` both vanish
/// there rather than reading as a correct refusal here.
#[test]
fn only_a_saved_key_the_unit_itself_reported_makes_a_replacement_adoptable()
-> Result<(), Box<dyn Error>> {
    let reported_saved = panel_key("SAVED-UNIT")?;
    let synthesized_saved = synthesized_panel_key();
    let (mut reported_app, .., reported_candidate) =
        replaced_at_shared_slot(reported_saved.clone())?;
    reported_app.update();
    let (mut synthesized_app, .., synthesized_candidate) =
        replaced_at_shared_slot(synthesized_saved.clone())?;
    synthesized_app.update();

    // The same departure happened on both sides, so what follows is about the saved key's evidence
    // rather than about one half never vacating the slot.
    assert_eq!(
        reported_app
            .world()
            .resource::<Devices>()
            .resolve(&reported_saved),
        DeviceResolution::NotResolved
    );
    assert_eq!(
        synthesized_app
            .world()
            .resource::<Devices>()
            .resolve(&synthesized_saved),
        DeviceResolution::NotResolved
    );

    assert_eq!(
        arrived_state(&reported_app, &reported_candidate)?,
        (
            IdentityVerdict::Displaced {
                saved: reported_saved.clone(),
            },
            IdentityDecisionOwed::HumanDecision(IdentityVerdict::Displaced {
                saved: reported_saved.clone(),
            })
        )
    );
    let reported_questions = questions(&reported_app);
    assert_eq!(reported_questions.len(), 1);
    assert_eq!(reported_questions[0].candidate, reported_candidate);
    assert_eq!(reported_questions[0].saved, reported_saved);

    assert_eq!(
        arrived_state(&synthesized_app, &synthesized_candidate)?,
        (IdentityVerdict::Proven, IdentityDecisionOwed::Nothing)
    );
    assert!(questions(&synthesized_app).is_empty());

    Ok(())
}

/// Read what the pass concluded about the arriving unit and what it still owes a human.
fn arrived_state(
    app: &App,
    candidate: &DeviceKey,
) -> Result<(IdentityVerdict, IdentityDecisionOwed), Box<dyn Error>> {
    let devices = app.world().resource::<Devices>();
    let DeviceResolution::Resolved(device_id) = devices.resolve(candidate) else {
        return Err("the arriving unit must resolve".into());
    };
    let DeviceStateLookup::Retained(state) = devices.state(device_id) else {
        return Err("the arriving unit must be retained".into());
    };

    Ok((state.verdict.clone(), state.decision_owed.clone()))
}

/// An answer written against a question that already expired must not move a binding the
/// application no longer has any reason to think it is talking about.
#[test]
fn an_answer_written_after_the_question_expired_changes_nothing() -> Result<(), Box<dyn Error>> {
    let (mut app, _, role, saved, candidate) = displaced_app()?;
    app.world_mut().resource_mut::<Bindings>().retire(&role)?;
    app.update();
    register_panel_role(&mut app, &role, saved.clone())?;

    let outcome = app.world_mut().resource_mut::<IdentityDecisions>().answer(
        &role,
        &candidate,
        IdentityAnswer::Adopt,
    );
    app.update();

    assert_eq!(outcome, AdoptionOutcome::NoSuchQuestion);
    assert_eq!(bound_device(&app, &role)?, saved);

    Ok(())
}

/// A role with nothing outstanding reads as a named state rather than an absent one.
#[test]
fn a_role_with_nothing_outstanding_reads_as_having_no_question() -> Result<(), Box<dyn Error>> {
    let (app, _) = scripted_app(Vec::new())?;
    let role = RoleKey::new("panel")?;

    assert!(matches!(
        app.world().resource::<IdentityDecisions>().question(&role),
        IdentityQuestionLookup::NoQuestion
    ));

    Ok(())
}

/// Every reflectable register type has to reach an inspector the same way the rest of the kernel
/// does. `IdentityQuestionLookup` is absent because it borrows the register and cannot derive
/// `Reflect`.
#[test]
fn every_register_type_resolves_through_the_type_registry() -> Result<(), Box<dyn Error>> {
    let (app, _) = scripted_app(Vec::new())?;

    let missing: Vec<&str> = {
        let app_type_registry = app.world().resource::<AppTypeRegistry>().read();
        [
            "hana_rigging::identity_decisions::AdoptionOutcome",
            "hana_rigging::identity_decisions::IdentityAnswer",
            "hana_rigging::identity_decisions::IdentityDecisions",
            "hana_rigging::identity_decisions::IdentityQuestion",
            "hana_rigging::identity_decisions::IdentityQuestionState",
            "hana_rigging::events::IdentityQuestionExpired",
            "hana_rigging::events::IdentityQuestionRaised",
        ]
        .into_iter()
        .filter(|type_path| app_type_registry.get_with_type_path(type_path).is_none())
        .collect()
    };

    assert!(
        missing.is_empty(),
        "missing from the type registry: {missing:?}"
    );

    Ok(())
}

/// Nothing mirrors the register onto an entity, so the resource is the only path an inspector or
/// the Bevy Remote Protocol has to the standing questions.
#[test]
fn the_standing_questions_are_readable_through_reflection() -> Result<(), Box<dyn Error>> {
    let (app, _, role, _, candidate) = displaced_app()?;

    let identity_decisions: &dyn Struct = app.world().resource::<IdentityDecisions>();
    let Some(questions) = identity_decisions
        .field("questions")
        .and_then(|field| field.reflect_ref().as_list().ok())
    else {
        return Err("the register must reflect its standing questions".into());
    };
    assert_eq!(questions.len(), 1);
    let Some(question) = questions
        .get(0)
        .and_then(|question| question.reflect_ref().as_struct().ok())
    else {
        return Err("a standing question must reflect as a struct".into());
    };
    assert_eq!(
        question
            .field("role")
            .and_then(|field| field.try_downcast_ref::<RoleKey>()),
        Some(&role)
    );
    assert_eq!(
        question
            .field("candidate")
            .and_then(|field| field.try_downcast_ref::<DeviceKey>()),
        Some(&candidate)
    );

    Ok(())
}
