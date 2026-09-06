//! Public-surface coverage, including a production client that does not enable `test-support`.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::error::Error;
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::app::Update;
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::query::Changed;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::platform::time::Instant;
use bevy::prelude::Component;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::reflect::TypePath;
use bevy::time::Real;
use bevy::time::Time;
use bevy::time::TimeUpdateStrategy;
use hana_rigging::prelude::Applied;
use hana_rigging::prelude::ApplyContext;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::AttemptCompletion;
use hana_rigging::prelude::AttemptInvalidation;
use hana_rigging::prelude::AttemptRef;
use hana_rigging::prelude::AuthoritativeReporterCoverage;
use hana_rigging::prelude::BindingAuthoring;
use hana_rigging::prelude::BindingPolicy;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::ConnectedCause;
use hana_rigging::prelude::ContinuousFlowExpectation;
use hana_rigging::prelude::ContinuousFlowExpiryCause;
use hana_rigging::prelude::ContinuousFlowView;
use hana_rigging::prelude::CoveredDeviceIdentitySpace;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceTransport;
use hana_rigging::prelude::DiscoveryCadence;
use hana_rigging::prelude::DriverCleanupRoleEntity;
use hana_rigging::prelude::DriverCompletion;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointDriverRegistration;
use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::EstablishedContext;
use hana_rigging::prelude::EstablishedFlowView;
use hana_rigging::prelude::FirstDatumTimeout;
use hana_rigging::prelude::FlowExpectation;
use hana_rigging::prelude::HardwareWait;
use hana_rigging::prelude::KernelRolePresentation;
use hana_rigging::prelude::MaximumDatumGap;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::ReporterActivation;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::RetirementOutcome;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingPlugin;
use hana_rigging::prelude::RiggingRuntimeClock;
use hana_rigging::prelude::RiggingSystems;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RolePresentationView;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;
use hana_rigging::prelude::RoleUnavailableCause;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::SessionDatumArrivalEvidence;
use hana_rigging::prelude::SessionLease;
use hana_rigging::prelude::SessionRef;
use hana_rigging::prelude::SessionReleaseCause;
use hana_rigging::prelude::TargetResolution;
use hana_rigging::prelude::TargetResolutionContext;
use hana_rigging::prelude::TransportObservation;
use hana_rigging::prelude::WaitingStatusView;
use hana_rigging::prelude::register_binding;
use hana_rigging::prelude::replace_binding;
use hana_rigging_scripted::ScriptedDevice;
use hana_rigging_scripted::ScriptedDriver;
use hana_rigging_scripted::ScriptedReporter;
use hana_rigging_scripted::ScriptedScan;
use hana_rigging_scripted::advance_reporter;
use hana_rigging_scripted::reported_key;
use hana_rigging_scripted::scan;

const FIRST_DATUM_TIMEOUT: Duration = Duration::from_secs(2);
const MAXIMUM_DATUM_GAP: Duration = Duration::from_secs(3);
const MAXIMUM_DURATION_MINUS_ONE_NANOSECOND: Duration = Duration::new(u64::MAX, 999_999_998);
const SUBSECOND_FIRST_DATUM_TIMEOUT: Duration = Duration::from_millis(125);
const SUBSECOND_MAXIMUM_DATUM_GAP: Duration = Duration::from_millis(875);
const ABSENT_ROLE: &str = "external-presentation-absent";
const REFLECTED_PRESENTATION_ROLE: &str = "external-presentation-reflected";
const SCANNING_ROLE: &str = "external-presentation-scanning";
const TEST_DEVICE_SCHEME: &str = "external-flow-device";
const TEST_DEVICE_VALUE: &str = "flow-source";
const TEST_ROLE: &str = "external-flow-role";
const UNREACHABLE_ROLE: &str = "external-presentation-unreachable";
const UNREACHABLE_SINCE: Duration = Duration::from_secs(2);
const STRICTLY_PAST_BOUND: Duration = Duration::from_nanos(1);
/// Frames a concurrency probe runs while a worker thread finishes its attempt.
const CONCURRENT_FINISH_FRAME_CEILING: usize = 64;

#[derive(Component, Reflect)]
#[reflect(Component)]
struct FlowConfiguration;

struct PendingFlowAttempt {
    role:       RoleKey,
    attempt:    AttemptRef,
    completion: AttemptCompletion<FlowConfiguration>,
}

#[derive(Default)]
struct FlowDriverState {
    attempts:                   VecDeque<PendingFlowAttempt>,
    sessions:                   Vec<(RoleKey, SessionLease<FlowConfiguration>)>,
    released_sessions:          Vec<SessionLease<FlowConfiguration>>,
    releases:                   Vec<(SessionRef, SessionReleaseCause)>,
    pre_establishment_arrivals: HashMap<RoleKey, SessionDatumArrivalEvidence>,
}

struct FlowDriver {
    state: Arc<Mutex<FlowDriverState>>,
}

#[derive(Clone, Resource)]
struct FlowDriverControl {
    state: Arc<Mutex<FlowDriverState>>,
}

impl FlowDriver {
    fn new() -> (Self, FlowDriverControl) {
        let state = Arc::new(Mutex::new(FlowDriverState::default()));
        (
            Self {
                state: Arc::clone(&state),
            },
            FlowDriverControl { state },
        )
    }
}

impl FlowDriverControl {
    fn pending_attempt(&self, role: &RoleKey) -> Option<AttemptRef> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .attempts
            .iter()
            .find_map(|pending| (&pending.role == role).then_some(pending.attempt))
    }

    fn finish_attempt(&self, attempt: AttemptRef) -> bool {
        let completion = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(index) = state
                .attempts
                .iter()
                .position(|pending| pending.attempt == attempt)
            else {
                drop(state);
                return false;
            };
            let Some(pending) = state.attempts.remove(index) else {
                drop(state);
                return false;
            };
            let completion = pending.completion;
            drop(state);
            completion
        };
        completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
        true
    }

    /// Take one pending attempt's completion authority so a caller can finish it off-thread.
    fn take_completion(&self, attempt: AttemptRef) -> Option<AttemptCompletion<FlowConfiguration>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = state
            .attempts
            .iter()
            .position(|pending| pending.attempt == attempt)?;
        let pending = state.attempts.remove(index);
        drop(state);
        pending.map(|pending| pending.completion)
    }

    fn session_ref(&self, role: &RoleKey) -> Option<SessionRef> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sessions
            .iter()
            .find_map(|(candidate, lease)| (candidate == role).then(|| lease.session_ref()))
    }

    fn releases(&self) -> Vec<(SessionRef, SessionReleaseCause)> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .releases
            .clone()
    }

    fn retain_pre_establishment_arrival(&self, role: RoleKey, observed_at: Instant) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pre_establishment_arrivals
            .insert(
                role,
                FlowTransport::pre_establishment_evidence(&DeliveredDatum, observed_at),
            );
    }

    fn record_active_arrival(&self, role: &RoleKey) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(lease) = state
            .sessions
            .iter_mut()
            .find_map(|(candidate, lease)| (candidate == role).then_some(lease))
        else {
            drop(state);
            return false;
        };
        lease.observe::<FlowTransport>(&DeliveredDatum);
        drop(state);
        true
    }

    fn record_released_arrival(&self, session: SessionRef) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(lease) = state
            .released_sessions
            .iter_mut()
            .find_map(|lease| (lease.session_ref() == session).then_some(lease))
        else {
            drop(state);
            return false;
        };
        lease.observe::<FlowTransport>(&DeliveredDatum);
        drop(state);
        true
    }
}

/// One datum this test driver delivered to the consumer reading its stream.
struct DeliveredDatum;

/// The test driver's transport contract.
///
/// It observes exactly one thing, so its classification is the whole of it — which is the point:
/// the device-specific judgment lives here, and the kernel owns every transition that follows.
struct FlowTransport;

impl DeviceTransport for FlowTransport {
    type Observation = DeliveredDatum;

    fn classify(_delivered: &Self::Observation) -> TransportObservation {
        TransportObservation::DatumDelivered
    }
}

impl EndpointDriver for FlowDriver {
    type Configuration = FlowConfiguration;
    type Target = ();

    fn resolve_target(
        &mut self,
        _: &mut World,
        _: &TargetResolutionContext<'_>,
        _: &Self::Configuration,
    ) -> TargetResolution<Self::Target> {
        TargetResolution::Reached(())
    }

    fn start_apply(
        &mut self,
        _: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        _: &Self::Configuration,
        (): Self::Target,
    ) {
        let role = context.target().role().clone();
        let attempt = context.attempt();
        let completion = context.into_completion();
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .attempts
            .push_back(PendingFlowAttempt {
                role,
                attempt,
                completion,
            });
    }

    fn established(&mut self, _: &mut World, context: EstablishedContext<'_, Self::Configuration>) {
        let role = context.role().clone();
        let retained_arrival = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pre_establishment_arrivals
            .remove(&role);
        let lease = context
            .into_lease(retained_arrival.unwrap_or(SessionDatumArrivalEvidence::NoDatumObserved));
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sessions
            .push((role, lease));
    }

    fn cancel_apply(
        &mut self,
        _: &mut World,
        _: &RoleKey,
        _: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        _: AttemptInvalidation,
    ) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .attempts
            .retain(|pending| pending.attempt != attempt);
    }

    fn release_session(
        &mut self,
        _: &mut World,
        role: &RoleKey,
        _: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.releases.push((session, cause));
        let session_index = state
            .sessions
            .iter()
            .position(|(candidate, lease)| candidate == role && lease.session_ref() == session);
        if let Some(session_index) = session_index {
            let (_, released) = state.sessions.remove(session_index);
            state.released_sessions.push(released);
        }
    }
}

#[derive(Default, Resource)]
struct DatumTestimonyPlan {
    active_roles:            Vec<RoleKey>,
    released_sessions:       Vec<SessionRef>,
    pre_establishment_roles: Vec<RoleKey>,
}

#[derive(Default, Resource)]
struct DatumTestimonyFailures(Vec<String>);

#[derive(Default, Resource)]
struct RoleStatusEdgeCount(usize);

#[derive(Default, Resource)]
struct BindingsEdgeCount(usize);

fn deliver_planned_datum_testimony(
    control: Res<'_, FlowDriverControl>,
    time: Res<'_, Time<Real>>,
    mut plan: ResMut<'_, DatumTestimonyPlan>,
    mut failures: ResMut<'_, DatumTestimonyFailures>,
) {
    let observed_at = time.last_update().unwrap_or_else(|| time.startup());
    for role in std::mem::take(&mut plan.pre_establishment_roles) {
        control.retain_pre_establishment_arrival(role, observed_at);
    }
    for role in std::mem::take(&mut plan.active_roles) {
        if !control.record_active_arrival(&role) {
            failures.0.push(format!(
                "no active session accepted datum testimony for `{role}`"
            ));
        }
    }
    for session in std::mem::take(&mut plan.released_sessions) {
        if !control.record_released_arrival(session) {
            failures.0.push(format!(
                "no released session accepted datum testimony for `{session:?}`"
            ));
        }
    }
}

fn count_role_status_edges(
    statuses: Query<'_, '_, (), Changed<RoleStatus>>,
    mut count: ResMut<'_, RoleStatusEdgeCount>,
) {
    count.0 += statuses.iter().count();
}

fn count_bindings_edges(bindings: Res<'_, Bindings>, mut count: ResMut<'_, BindingsEdgeCount>) {
    count.0 += usize::from(bindings.is_changed());
}

struct FlowHarness {
    app:       App,
    control:   FlowDriverControl,
    driver:    EndpointDriverRegistration<FlowConfiguration>,
    reporter:  ReporterId,
    roles:     Vec<RoleKey>,
    endpoints: Vec<DeviceEndpoint>,
    policy:    BindingPolicy,
}

impl FlowHarness {
    fn new(role_names: &[&str], policy: BindingPolicy) -> Result<Self, Box<dyn Error>> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(RiggingPlugin)
            .register_device_scheme(SchemeName::new(TEST_DEVICE_SCHEME)?);
        let frame_started_at = app.world().resource::<Time<Real>>().startup();
        app.insert_resource(TimeUpdateStrategy::ManualInstant(frame_started_at))
            .init_resource::<DatumTestimonyPlan>()
            .init_resource::<DatumTestimonyFailures>()
            .init_resource::<RoleStatusEdgeCount>()
            .init_resource::<BindingsEdgeCount>()
            .add_systems(
                Update,
                deliver_planned_datum_testimony
                    .after(RiggingSystems::Collect)
                    .before(RiggingSystems::SessionLoss),
            )
            .add_systems(
                Update,
                (count_role_status_edges, count_bindings_edges).after(RiggingSystems::Apply),
            );

        let roles = role_names
            .iter()
            .map(|name| RoleKey::new(*name))
            .collect::<Result<Vec<_>, _>>()?;
        let devices = role_names
            .iter()
            .map(|name| reported_key(DeviceKind::ControlSurface, TEST_DEVICE_SCHEME, name))
            .collect::<Result<Vec<_>, _>>()?;
        let endpoints = devices
            .iter()
            .cloned()
            .map(|device| DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            })
            .collect::<Vec<_>>();
        let reporter = app.add_device_reporter(
            ScriptedReporter::new([
                ScriptedScan::Complete(
                    devices
                        .iter()
                        .cloned()
                        .map(ScriptedDevice::present)
                        .collect(),
                ),
                ScriptedScan::Complete(
                    devices
                        .iter()
                        .cloned()
                        .map(ScriptedDevice::absent)
                        .collect(),
                ),
            ]),
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                ReporterCoverage::MatchingEvidenceOnly,
                Duration::from_secs(1),
            ),
        );
        let (driver, control) = FlowDriver::new();
        app.insert_resource(control.clone());
        let driver = app.add_endpoint_driver(driver);
        for (role, endpoint) in roles.iter().zip(&endpoints) {
            register_binding(
                app.world_mut(),
                BindingAuthoring::new(
                    role.clone(),
                    endpoint.clone(),
                    driver,
                    FlowConfiguration,
                    policy,
                ),
            )?;
        }

        advance_reporter(&mut app, reporter)?;
        for _ in 0..8 {
            if roles
                .iter()
                .all(|role| control.pending_attempt(role).is_some())
            {
                return Ok(Self {
                    app,
                    control,
                    driver,
                    reporter,
                    roles,
                    endpoints,
                    policy,
                });
            }
            app.update();
        }
        Err("the flow-test driver did not receive every apply attempt".into())
    }

    fn establish_all(
        &mut self,
        pre_establishment_roles: &[RoleKey],
    ) -> Result<Instant, Box<dyn Error>> {
        self.app
            .world_mut()
            .resource_mut::<DatumTestimonyPlan>()
            .pre_establishment_roles
            .extend_from_slice(pre_establishment_roles);
        let attempts = self
            .roles
            .iter()
            .map(|role| {
                self.control.pending_attempt(role).ok_or_else(|| {
                    io::Error::other(format!("role `{role}` has no pending attempt"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for attempt in attempts {
            if !self.control.finish_attempt(attempt) {
                return Err(format!("attempt `{attempt:?}` could not be finished").into());
            }
        }
        for _ in 0..8 {
            self.app.update();
            if self
                .roles
                .iter()
                .all(|role| self.control.session_ref(role).is_some())
            {
                self.assert_no_testimony_failures()?;
                return Ok(self.now());
            }
        }
        Err("the flow-test driver did not establish every session".into())
    }

    fn now(&self) -> Instant {
        let time = self.app.world().resource::<Time<Real>>();
        time.last_update().unwrap_or_else(|| time.startup())
    }

    fn runtime_clock(&self) -> RiggingRuntimeClock {
        *self.app.world().resource::<RiggingRuntimeClock>()
    }

    /// Retain a pre-establishment observation on its own earlier frame, then establish later.
    ///
    /// Separate frames because an observation stamped with the establishing frame's instant and
    /// one stamped with its own cannot be told apart when both land on the same frame.
    fn establish_all_after_arrival_at(
        &mut self,
        pre_establishment_roles: &[RoleKey],
        observed_at: Instant,
        established_at: Instant,
    ) -> Result<(), Box<dyn Error>> {
        self.app
            .world_mut()
            .resource_mut::<DatumTestimonyPlan>()
            .pre_establishment_roles
            .extend_from_slice(pre_establishment_roles);
        self.update_at(observed_at)?;
        self.app
            .insert_resource(TimeUpdateStrategy::ManualInstant(established_at));
        self.establish_all(&[])?;
        Ok(())
    }

    /// Replay the reporter's departure scan so every bound device stops resolving.
    fn depart_bound_devices(&mut self) -> Result<(), Box<dyn Error>> {
        advance_reporter(&mut self.app, self.reporter)?;
        self.assert_no_testimony_failures()
    }

    fn published_flow(&self, role: &RoleKey) -> Result<EstablishedFlowView, Box<dyn Error>> {
        match role_status(&self.app, role)?.view() {
            RoleStatusView::Established { flow, .. } => Ok(*flow),
            other => Err(format!("role `{role}` is not established: {other:?}").into()),
        }
    }

    fn is_established_and_stalled(&self, role: &RoleKey) -> bool {
        matches!(
            self.published_flow(role),
            Ok(EstablishedFlowView::Continuous(
                ContinuousFlowView::Stalled { .. }
            ))
        )
    }

    fn flow_stalled_releases(&self) -> usize {
        self.control
            .releases()
            .iter()
            .filter(|(_, cause)| cause == &SessionReleaseCause::FlowStalled)
            .count()
    }

    fn update_at(&mut self, instant: Instant) -> Result<(), Box<dyn Error>> {
        self.app
            .insert_resource(TimeUpdateStrategy::ManualInstant(instant));
        self.app.update();
        self.assert_no_testimony_failures()
    }

    fn testify_for_active_at(
        &mut self,
        roles: &[RoleKey],
        instant: Instant,
    ) -> Result<(), Box<dyn Error>> {
        self.app
            .world_mut()
            .resource_mut::<DatumTestimonyPlan>()
            .active_roles
            .extend_from_slice(roles);
        self.update_at(instant)
    }

    fn testify_for_released_at(
        &mut self,
        session: SessionRef,
        instant: Instant,
    ) -> Result<(), Box<dyn Error>> {
        self.app
            .world_mut()
            .resource_mut::<DatumTestimonyPlan>()
            .released_sessions
            .push(session);
        self.update_at(instant)
    }

    fn assert_no_testimony_failures(&self) -> Result<(), Box<dyn Error>> {
        let failures = &self.app.world().resource::<DatumTestimonyFailures>().0;
        if failures.is_empty() {
            return Ok(());
        }
        Err(format!("datum testimony failures: {failures:?}").into())
    }

    fn clear_status_edge_count(&mut self) {
        self.app.world_mut().resource_mut::<RoleStatusEdgeCount>().0 = 0;
    }

    fn status_edge_count(&self) -> usize { self.app.world().resource::<RoleStatusEdgeCount>().0 }

    fn clear_bindings_edge_count(&mut self) {
        self.app.world_mut().resource_mut::<BindingsEdgeCount>().0 = 0;
    }

    fn bindings_edge_count(&self) -> usize { self.app.world().resource::<BindingsEdgeCount>().0 }

    fn replace_first_role(&mut self) -> Result<(), Box<dyn Error>> {
        replace_binding(
            self.app.world_mut(),
            BindingAuthoring::new(
                self.roles[0].clone(),
                self.endpoints[0].clone(),
                self.driver,
                FlowConfiguration,
                self.policy,
            ),
        )?;
        Ok(())
    }

    fn finish_pending_attempt_for_first_role(&mut self) -> Result<SessionRef, Box<dyn Error>> {
        for _ in 0..8 {
            if let Some(attempt) = self.control.pending_attempt(&self.roles[0]) {
                if !self.control.finish_attempt(attempt) {
                    return Err("the replacement attempt could not be finished".into());
                }
                break;
            }
            self.app.update();
        }
        for _ in 0..8 {
            self.app.update();
            if let Some(session) = self.control.session_ref(&self.roles[0]) {
                return Ok(session);
            }
        }
        Err("the replacement session did not establish".into())
    }
}

/// Coverage whose complete report establishes absence for every key these fixtures author.
///
/// `ReporterCoverage::MatchingEvidenceOnly` covers no key at all, so a reporter carrying it can
/// neither leave a key awaiting its first report nor prove a bound device has departed. Every
/// fixture that needs either conclusion registers this instead.
fn authoritative_coverage() -> Result<ReporterCoverage, Box<dyn Error>> {
    Ok(ReporterCoverage::EstablishesAbsence(
        AuthoritativeReporterCoverage::one(CoveredDeviceIdentitySpace::ReportedScheme {
            kind:   DeviceKind::ControlSurface,
            scheme: SchemeName::new(TEST_DEVICE_SCHEME)?,
        }),
    ))
}

fn continuous_policy(retry: RetryOn) -> Result<BindingPolicy, Box<dyn Error>> {
    Ok(BindingPolicy::new(
        RecoveryPolicy::default(),
        retry,
        OnAbort::default(),
        OnSessionLoss::default(),
        ApplyDeadline::ProcessDefault,
    )
    .with_continuous_flow(ContinuousFlowExpectation::new(
        FirstDatumTimeout::new(FIRST_DATUM_TIMEOUT)?,
        MaximumDatumGap::new(MAXIMUM_DATUM_GAP)?,
    )))
}

#[test]
fn first_datum_silence_expires_into_stalled() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["first-datum-expiry"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;

    harness.update_at(established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND)?;

    assert!(matches!(
        role_status(&harness.app, &harness.roles[0])?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Stalled { .. }),
            ..
        }
    ));
    assert!(harness.control.releases().is_empty());
    Ok(())
}

#[test]
fn maximum_datum_gap_expires_from_the_latest_arrival() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["datum-gap-expiry"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let datum_at = established_at + STRICTLY_PAST_BOUND;
    let role = harness.roles[0].clone();
    harness.testify_for_active_at(std::slice::from_ref(&role), datum_at)?;

    harness.update_at(datum_at + MAXIMUM_DATUM_GAP + STRICTLY_PAST_BOUND)?;

    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Stalled { .. }),
            ..
        }
    ));
    Ok(())
}

#[test]
fn datum_on_each_exact_expiry_frame_remains_current() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["datum-on-boundary"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let first_datum_at = established_at + FIRST_DATUM_TIMEOUT;
    harness.testify_for_active_at(std::slice::from_ref(&role), first_datum_at)?;
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. }),
            ..
        }
    ));

    harness.testify_for_active_at(
        std::slice::from_ref(&role),
        first_datum_at + MAXIMUM_DATUM_GAP,
    )?;

    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. }),
            ..
        }
    ));
    assert!(harness.control.releases().is_empty());
    Ok(())
}

#[test]
fn both_flow_intervals_expire_only_strictly_past_their_bounds() -> Result<(), Box<dyn Error>> {
    let mut first = FlowHarness::new(
        &["first-boundary"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let first_established_at = first.establish_all(&[])?;
    first.update_at(first_established_at + FIRST_DATUM_TIMEOUT)?;
    assert!(matches!(
        role_status(&first.app, &first.roles[0])?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::AwaitingFirstDatum { .. }),
            ..
        }
    ));
    first.update_at(first_established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND)?;
    assert!(matches!(
        role_status(&first.app, &first.roles[0])?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Stalled { .. }),
            ..
        }
    ));

    let mut gap = FlowHarness::new(&["gap-boundary"], continuous_policy(RetryOn::NewRevision)?)?;
    let gap_established_at = gap.establish_all(&[])?;
    let gap_role = gap.roles[0].clone();
    gap.testify_for_active_at(std::slice::from_ref(&gap_role), gap_established_at)?;
    gap.update_at(gap_established_at + MAXIMUM_DATUM_GAP)?;
    assert!(matches!(
        role_status(&gap.app, &gap_role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. }),
            ..
        }
    ));
    gap.update_at(gap_established_at + MAXIMUM_DATUM_GAP + STRICTLY_PAST_BOUND)?;
    assert!(matches!(
        role_status(&gap.app, &gap_role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Stalled { .. }),
            ..
        }
    ));
    Ok(())
}

#[test]
fn first_datum_moves_the_published_session_to_flowing() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["first-datum-flowing"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();

    harness.testify_for_active_at(
        std::slice::from_ref(&role),
        established_at + Duration::from_secs(1),
    )?;

    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { flowing_since: _ }),
            ..
        }
    ));
    Ok(())
}

#[test]
fn repeated_arrivals_extend_expiry_without_a_status_edge() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["repeated-arrivals"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let first_datum_at = established_at + Duration::from_secs(1);
    harness.clear_status_edge_count();
    harness.testify_for_active_at(std::slice::from_ref(&role), first_datum_at)?;
    assert_eq!(harness.status_edge_count(), 1);
    let first_flow_view = match role_status(&harness.app, &role)?.view() {
        RoleStatusView::Established { flow, .. } => *flow,
        other => return Err(format!("first datum published unexpected status: {other:?}").into()),
    };

    let latest_datum_at = first_datum_at + Duration::from_secs(2);
    harness.clear_status_edge_count();
    harness.testify_for_active_at(std::slice::from_ref(&role), latest_datum_at)?;

    assert_eq!(harness.status_edge_count(), 0);
    let repeated_flow_view = match role_status(&harness.app, &role)?.view() {
        RoleStatusView::Established { flow, .. } => flow,
        other => {
            return Err(format!("repeated datum published unexpected status: {other:?}").into());
        },
    };
    assert_eq!(repeated_flow_view, &first_flow_view);
    harness.update_at(latest_datum_at + MAXIMUM_DATUM_GAP)?;
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. }),
            ..
        }
    ));
    harness.update_at(latest_datum_at + MAXIMUM_DATUM_GAP + STRICTLY_PAST_BOUND)?;
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Stalled { .. }),
            ..
        }
    ));
    Ok(())
}

#[test]
fn healthy_continuous_flow_leaves_bindings_unchanged_when_no_work_is_due()
-> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["continuous-fast-path"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();

    harness.clear_bindings_edge_count();
    harness.update_at(established_at + Duration::from_secs(1))?;
    assert_eq!(harness.bindings_edge_count(), 0);

    harness.testify_for_active_at(
        std::slice::from_ref(&role),
        established_at + Duration::from_secs(1),
    )?;
    harness.clear_bindings_edge_count();
    harness.update_at(established_at + Duration::from_secs(2))?;

    assert_eq!(harness.bindings_edge_count(), 0);
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. }),
            ..
        }
    ));
    Ok(())
}

#[test]
fn a_pre_establishment_arrival_is_credited_at_its_own_earlier_instant() -> Result<(), Box<dyn Error>>
{
    let mut harness = FlowHarness::new(
        &["retained-arrival"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let role = harness.roles[0].clone();
    let observed_at = harness.now() + Duration::from_secs(1);
    let established_at = observed_at + Duration::from_millis(1_500);

    harness.establish_all_after_arrival_at(
        std::slice::from_ref(&role),
        observed_at,
        established_at,
    )?;

    // Flowing from the driver's own observation, not from the frame that issued the lease.
    let observed_runtime_time = harness.runtime_clock().time_at(observed_at);
    assert_eq!(
        harness.published_flow(&role)?,
        EstablishedFlowView::Continuous(ContinuousFlowView::Flowing {
            flowing_since: observed_runtime_time,
        })
    );

    // The maximum gap therefore runs from the observation. Stamping the establishing frame
    // instead would hold the session flowing for another 1.5 seconds; discarding the arrival
    // altogether would leave it awaiting its first datum until 3.5 seconds past the observation.
    harness.update_at(observed_at + MAXIMUM_DATUM_GAP)?;
    assert_eq!(
        harness.published_flow(&role)?,
        EstablishedFlowView::Continuous(ContinuousFlowView::Flowing {
            flowing_since: observed_runtime_time,
        })
    );
    harness.update_at(observed_at + MAXIMUM_DATUM_GAP + STRICTLY_PAST_BOUND)?;
    assert!(harness.is_established_and_stalled(&role));
    Ok(())
}

#[test]
fn flow_judgment_writes_the_binding_register_only_when_it_publishes() -> Result<(), Box<dyn Error>>
{
    let mut harness = FlowHarness::new(
        &["crediting-only-frames"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let session = harness
        .control
        .session_ref(&role)
        .ok_or("the established role has no session")?;
    let first_datum_at = established_at + Duration::from_secs(1);
    harness.testify_for_active_at(std::slice::from_ref(&role), first_datum_at)?;

    // A camera testifying every frame only credits a session that is already flowing: no
    // variant transition and no release, so nothing the binding register holds has changed.
    // Marking it changed here wakes every consumer gated on it at source frame rate.
    harness.clear_bindings_edge_count();
    let mut latest_datum_at = first_datum_at;
    for frame in 1..=8 {
        latest_datum_at = first_datum_at + Duration::from_millis(frame * 100);
        harness.testify_for_active_at(std::slice::from_ref(&role), latest_datum_at)?;
    }
    assert_eq!(harness.bindings_edge_count(), 0);
    assert_eq!(
        harness.published_flow(&role)?,
        EstablishedFlowView::Continuous(ContinuousFlowView::Flowing {
            flowing_since: harness.runtime_clock().time_at(first_datum_at),
        })
    );

    // The frame that publishes the stall does change what the register holds.
    let stalled_at = latest_datum_at + MAXIMUM_DATUM_GAP + STRICTLY_PAST_BOUND;
    harness.clear_bindings_edge_count();
    harness.update_at(stalled_at)?;
    assert!(harness.is_established_and_stalled(&role));
    assert_eq!(harness.bindings_edge_count(), 1);

    // So does the frame that releases the stalled session and hands the role back to retry.
    harness.clear_bindings_edge_count();
    harness.update_at(stalled_at)?;
    assert_eq!(
        harness.control.releases(),
        vec![(session, SessionReleaseCause::FlowStalled)]
    );
    assert_eq!(harness.bindings_edge_count(), 1);
    Ok(())
}

#[test]
fn a_stalled_session_whose_device_departs_is_released_once_and_leaves_established()
-> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["stalled-device-departure"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let crossed_at = established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND;
    harness.update_at(crossed_at)?;
    assert!(harness.is_established_and_stalled(&role));

    harness.depart_bound_devices()?;
    for _ in 0..32 {
        harness.update_at(crossed_at)?;
    }

    // A crossing owes exactly one release. Skipping the loss that follows it leaves the role
    // published as stalled and re-releases the same session on every later update.
    assert!(
        harness.flow_stalled_releases() <= 1,
        "the stalled session was released more than once: {:?}",
        harness.control.releases()
    );
    assert_no_session_released_twice(&harness.control.releases());
    assert!(
        !harness.is_established_and_stalled(&role),
        "the role is still published as an established stalled session"
    );
    Ok(())
}

#[test]
fn a_flowing_session_whose_device_departs_never_stalls_forever() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["flowing-device-departure"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let datum_at = established_at + Duration::from_secs(1);
    harness.testify_for_active_at(std::slice::from_ref(&role), datum_at)?;

    harness.depart_bound_devices()?;
    let past_the_gap = datum_at + MAXIMUM_DATUM_GAP + STRICTLY_PAST_BOUND;
    for _ in 0..32 {
        harness.update_at(past_the_gap)?;
    }

    assert_no_session_released_twice(&harness.control.releases());
    assert!(
        !harness.is_established_and_stalled(&role),
        "the role is still published as an established stalled session"
    );
    Ok(())
}

#[test]
fn an_attempt_finished_on_a_worker_thread_establishes_its_session() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["off-thread-completion"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let role = harness.roles[0].clone();
    let attempt = harness
        .control
        .pending_attempt(&role)
        .ok_or("the role has no pending attempt")?;
    let completion = harness
        .control
        .take_completion(attempt)
        .ok_or("the pending attempt has no completion authority")?;

    // The kernel advances frames while a driver thread stamps and queues its result, which is
    // how a real driver finishes. Both sides take the frame instant and the report mailbox.
    let worker = thread::spawn(move || {
        completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
    });
    for _ in 0..CONCURRENT_FINISH_FRAME_CEILING {
        harness.app.update();
    }
    worker
        .join()
        .map_err(|_| "the worker thread finishing the attempt panicked")?;

    for _ in 0..8 {
        harness.app.update();
        if harness.control.session_ref(&role).is_some() {
            assert_eq!(
                harness.published_flow(&role)?,
                EstablishedFlowView::Continuous(ContinuousFlowView::AwaitingFirstDatum {
                    deadline: harness
                        .runtime_clock()
                        .time_at(harness.now() + FIRST_DATUM_TIMEOUT),
                })
            );
            return Ok(());
        }
    }
    Err("the off-thread completion never established a session".into())
}

fn assert_no_session_released_twice(releases: &[(SessionRef, SessionReleaseCause)]) {
    for (index, (session, _)) in releases.iter().enumerate() {
        assert!(
            !releases[..index]
                .iter()
                .any(|(earlier, _)| earlier == session),
            "session {session:?} was released more than once: {releases:?}"
        );
    }
}

#[test]
fn one_arrival_credits_several_exact_sessions() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["shared-stream-owner", "shared-stream-subscriber"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let roles = harness.roles.clone();
    let sessions = roles
        .iter()
        .map(|role| {
            harness
                .control
                .session_ref(role)
                .ok_or_else(|| io::Error::other(format!("role `{role}` has no exact session")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    harness.testify_for_active_at(&roles, established_at + Duration::from_secs(1))?;

    for (role, session) in roles.iter().zip(sessions) {
        assert!(matches!(
            role_status(&harness.app, role)?.view(),
            RoleStatusView::Established {
                session: published_session,
                flow: EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. }),
                ..
            } if *published_session == session
        ));
    }
    Ok(())
}

#[test]
fn stalled_is_published_for_one_update_before_waiting_for_retry() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["stalled-before-waiting"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let session = harness
        .control
        .session_ref(&role)
        .ok_or("the established role has no session")?;
    let crossed_at = established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND;

    harness.update_at(crossed_at)?;
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            session: published_session,
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Stalled { .. }),
            ..
        } if *published_session == session
    ));
    assert!(harness.control.releases().is_empty());

    harness.update_at(crossed_at)?;
    assert_eq!(
        harness.control.releases(),
        vec![(session, SessionReleaseCause::FlowStalled)]
    );
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Waiting(WaitingStatusView::KernelRetry { .. })
    ));
    Ok(())
}

#[test]
fn late_testimony_does_not_revive_a_stalled_session() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["late-testimony"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let session = harness
        .control
        .session_ref(&role)
        .ok_or("the established role has no session")?;
    let crossed_at = established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND;
    harness.update_at(crossed_at)?;

    harness.testify_for_active_at(std::slice::from_ref(&role), crossed_at)?;

    assert_eq!(
        harness.control.releases(),
        vec![(session, SessionReleaseCause::FlowStalled)]
    );
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Waiting(WaitingStatusView::KernelRetry { .. })
    ));
    Ok(())
}

#[test]
fn released_session_testimony_does_not_renew_its_replacement() -> Result<(), Box<dyn Error>> {
    let retry_interval = Duration::from_secs(1);
    let mut harness = FlowHarness::new(
        &["stale-testimony"],
        continuous_policy(RetryOn::Interval(retry_interval))?,
    )?;
    let first_established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let first_session = harness
        .control
        .session_ref(&role)
        .ok_or("the first session was not retained")?;
    let crossed_at = first_established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND;
    harness.update_at(crossed_at)?;
    harness.update_at(crossed_at)?;
    harness.update_at(crossed_at + retry_interval + STRICTLY_PAST_BOUND)?;
    let replacement_session = harness.finish_pending_attempt_for_first_role()?;
    assert_ne!(replacement_session, first_session);
    let replacement_established_at = harness.now();

    harness.testify_for_released_at(
        first_session,
        replacement_established_at + Duration::from_secs(1),
    )?;
    harness.update_at(replacement_established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND)?;

    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            session,
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Stalled { .. }),
            ..
        } if *session == replacement_session
    ));
    assert_eq!(
        harness.control.releases(),
        vec![(first_session, SessionReleaseCause::FlowStalled)]
    );
    Ok(())
}

#[test]
fn replacement_session_resets_flow_to_awaiting_first_datum() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["replacement-reset"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();
    let first_session = harness
        .control
        .session_ref(&role)
        .ok_or("the first session was not retained")?;
    harness.testify_for_active_at(
        std::slice::from_ref(&role),
        established_at + Duration::from_secs(1),
    )?;
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. }),
            ..
        }
    ));

    harness.replace_first_role()?;
    let replacement_session = harness.finish_pending_attempt_for_first_role()?;

    assert_ne!(replacement_session, first_session);
    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            session,
            flow: EstablishedFlowView::Continuous(
                ContinuousFlowView::AwaitingFirstDatum { .. }
            ),
            ..
        } if *session == replacement_session
    ));
    Ok(())
}

#[test]
fn recovery_releases_exactly_once_for_each_crossing() -> Result<(), Box<dyn Error>> {
    let retry_interval = Duration::from_secs(1);
    let mut harness = FlowHarness::new(
        &["one-release-per-crossing"],
        continuous_policy(RetryOn::Interval(retry_interval))?,
    )?;
    let first_established_at = harness.establish_all(&[])?;
    let first_crossing = first_established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND;
    harness.update_at(first_crossing)?;
    harness.update_at(first_crossing)?;
    assert_eq!(harness.control.releases().len(), 1);
    for _ in 0..3 {
        harness.update_at(first_crossing)?;
    }
    assert_eq!(harness.control.releases().len(), 1);

    harness.update_at(first_crossing + retry_interval + STRICTLY_PAST_BOUND)?;
    harness.finish_pending_attempt_for_first_role()?;
    let second_established_at = harness.now();
    let second_crossing = second_established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND;
    harness.update_at(second_crossing)?;
    assert_eq!(harness.control.releases().len(), 1);
    harness.update_at(second_crossing)?;
    assert_eq!(harness.control.releases().len(), 2);
    for _ in 0..3 {
        harness.update_at(second_crossing)?;
    }
    assert_eq!(harness.control.releases().len(), 2);
    assert!(
        harness
            .control
            .releases()
            .iter()
            .all(|(_, cause)| cause == &SessionReleaseCause::FlowStalled)
    );
    Ok(())
}

#[test]
fn not_monitored_never_stalls_or_releases_without_data() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(&["unmonitored-flow"], default_binding_policy())?;
    let established_at = harness.establish_all(&[])?;
    let role = harness.roles[0].clone();

    for cycle in 1..=32 {
        harness.update_at(established_at + Duration::from_secs(cycle * 10))?;
    }

    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::NotMonitored,
            ..
        }
    ));
    assert!(harness.control.releases().is_empty());
    Ok(())
}

#[test]
fn external_client_matches_boxed_retirement_outcome() -> Result<(), Box<dyn Error>> {
    let (mut app, role) = established_role(default_binding_policy())?;

    let outcome = app.world_mut().resource_mut::<Bindings>().retire(&role)?;
    let RetirementOutcome::Retired(retired) = outcome else {
        return Err("the registered role was already unbound".into());
    };

    assert_eq!(retired.role, role);
    Ok(())
}

#[test]
fn flow_intervals_reject_zero() {
    assert!(FirstDatumTimeout::new(Duration::ZERO).is_err());
    assert!(MaximumDatumGap::new(Duration::ZERO).is_err());
}

#[test]
fn continuous_flow_expectation_returns_each_interval_unchanged() -> Result<(), Box<dyn Error>> {
    for (first_datum_duration, maximum_gap_duration) in [
        (SUBSECOND_FIRST_DATUM_TIMEOUT, SUBSECOND_MAXIMUM_DATUM_GAP),
        (FIRST_DATUM_TIMEOUT, MAXIMUM_DATUM_GAP),
        (Duration::MAX, MAXIMUM_DURATION_MINUS_ONE_NANOSECOND),
    ] {
        let continuous_flow_expectation = ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(first_datum_duration)?,
            MaximumDatumGap::new(maximum_gap_duration)?,
        );

        assert_eq!(
            continuous_flow_expectation.first_datum_timeout().duration(),
            first_datum_duration
        );
        assert_eq!(
            continuous_flow_expectation.maximum_datum_gap().duration(),
            maximum_gap_duration
        );
    }

    Ok(())
}

#[test]
fn default_binding_projects_not_monitored_flow() -> Result<(), Box<dyn Error>> {
    let (app, role) = established_role(default_binding_policy())?;

    assert_eq!(
        app.world()
            .resource::<Bindings>()
            .binding(&role)?
            .policy()
            .flow_expectation(),
        FlowExpectation::NotMonitored
    );
    assert!(matches!(
        role_status(&app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::NotMonitored,
            ..
        }
    ));
    Ok(())
}

#[test]
fn continuous_binding_projects_awaiting_first_datum() -> Result<(), Box<dyn Error>> {
    let continuous_flow_expectation = ContinuousFlowExpectation::new(
        FirstDatumTimeout::new(FIRST_DATUM_TIMEOUT)?,
        MaximumDatumGap::new(MAXIMUM_DATUM_GAP)?,
    );
    let binding_policy = default_binding_policy().with_continuous_flow(continuous_flow_expectation);
    assert_eq!(
        binding_policy.flow_expectation(),
        FlowExpectation::Continuous(continuous_flow_expectation)
    );
    let (app, role) = established_role(binding_policy)?;

    assert_eq!(
        app.world()
            .resource::<Bindings>()
            .binding(&role)?
            .policy()
            .flow_expectation(),
        FlowExpectation::Continuous(continuous_flow_expectation)
    );
    assert!(matches!(
        role_status(&app, &role)?.view(),
        RoleStatusView::Established {
            flow: EstablishedFlowView::Continuous(ContinuousFlowView::AwaitingFirstDatum { .. }),
            ..
        }
    ));
    Ok(())
}

#[test]
fn an_applying_role_presents_connecting() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["presentation-applying"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let role = harness.roles[0].clone();
    let now = harness.now();

    harness.update_at(now)?;

    assert!(matches!(
        role_status(&harness.app, &role)?.view(),
        RoleStatusView::Applying { .. }
    ));
    assert_presents(&harness.app, &role, RolePresentationView::Connecting)?;
    Ok(())
}

#[test]
fn a_role_awaiting_its_first_report_presents_scanning() -> Result<(), Box<dyn Error>> {
    let (app, role) = presentation_fixture(SCANNING_ROLE, FixtureEvidence::NoCompletedScan)?;

    let status = role_status(&app, &role)?.view();
    assert!(
        matches!(
            status,
            RoleStatusView::Waiting(WaitingStatusView::Reporter(
                HardwareWait::AwaitingFirstReport { .. }
            ))
        ),
        "the fixture role is not awaiting a first report: {status:?}"
    );
    assert_presents(&app, &role, RolePresentationView::Scanning)?;
    Ok(())
}

#[test]
fn an_unreachable_device_presents_unavailable() -> Result<(), Box<dyn Error>> {
    let (app, role) = presentation_fixture(UNREACHABLE_ROLE, FixtureEvidence::UnreachableDevice)?;

    assert!(matches!(
        role_status(&app, &role)?.view(),
        RoleStatusView::Waiting(WaitingStatusView::Reporter(
            HardwareWait::Unreachable { .. }
        ))
    ));
    assert_presents(
        &app,
        &role,
        RolePresentationView::Unavailable(RoleUnavailableCause::DeviceUnreachable),
    )?;
    Ok(())
}

#[test]
fn an_established_monitored_session_presents_connected_awaiting_its_first_datum()
-> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["presentation-awaiting-first-datum"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let role = harness.roles[0].clone();

    harness.establish_all(&[])?;

    assert!(matches!(
        harness.published_flow(&role)?,
        EstablishedFlowView::Continuous(ContinuousFlowView::AwaitingFirstDatum { .. })
    ));
    assert_presents(
        &harness.app,
        &role,
        RolePresentationView::Connected(ConnectedCause::AwaitingFirstDatum),
    )?;
    Ok(())
}

#[test]
fn an_unmonitored_established_session_presents_connected_with_no_flow_claim()
-> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(&["presentation-unmonitored"], default_binding_policy())?;
    let role = harness.roles[0].clone();

    harness.establish_all(&[])?;

    assert_eq!(
        harness.published_flow(&role)?,
        EstablishedFlowView::NotMonitored
    );
    assert_presents(
        &harness.app,
        &role,
        RolePresentationView::Connected(ConnectedCause::FlowNotMonitored),
    )?;
    Ok(())
}

#[test]
fn a_flowing_session_presents_presenting() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["presentation-flowing"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let role = harness.roles[0].clone();
    let established_at = harness.establish_all(&[])?;

    harness.testify_for_active_at(
        std::slice::from_ref(&role),
        established_at + STRICTLY_PAST_BOUND,
    )?;

    assert!(matches!(
        harness.published_flow(&role)?,
        EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. })
    ));
    assert_presents(&harness.app, &role, RolePresentationView::Presenting)?;
    Ok(())
}

#[test]
fn a_session_that_never_delivers_presents_a_first_datum_stall() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["presentation-first-datum-stall"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let role = harness.roles[0].clone();
    let established_at = harness.establish_all(&[])?;

    harness.update_at(established_at + FIRST_DATUM_TIMEOUT + STRICTLY_PAST_BOUND)?;

    assert!(harness.is_established_and_stalled(&role));
    assert_presents(
        &harness.app,
        &role,
        RolePresentationView::Connected(ConnectedCause::Stalled(
            ContinuousFlowExpiryCause::FirstDatumOverdue,
        )),
    )?;
    Ok(())
}

#[test]
fn a_session_that_falls_silent_presents_a_maximum_gap_stall() -> Result<(), Box<dyn Error>> {
    let mut harness = FlowHarness::new(
        &["presentation-maximum-gap-stall"],
        continuous_policy(RetryOn::NewRevision)?,
    )?;
    let role = harness.roles[0].clone();
    let established_at = harness.establish_all(&[])?;
    let datum_at = established_at + STRICTLY_PAST_BOUND;
    harness.testify_for_active_at(std::slice::from_ref(&role), datum_at)?;

    harness.update_at(datum_at + MAXIMUM_DATUM_GAP + STRICTLY_PAST_BOUND)?;

    assert!(harness.is_established_and_stalled(&role));
    assert_presents(
        &harness.app,
        &role,
        RolePresentationView::Connected(ConnectedCause::Stalled(
            ContinuousFlowExpiryCause::MaximumDatumGapExceeded,
        )),
    )?;
    Ok(())
}

#[test]
fn an_absent_device_presents_disconnected() -> Result<(), Box<dyn Error>> {
    let (app, role) = presentation_fixture(ABSENT_ROLE, FixtureEvidence::AbsentDevice)?;

    let status = role_status(&app, &role)?.view();
    assert!(
        matches!(status, RoleStatusView::Waiting(_)),
        "the absent role is not waiting: {status:?}"
    );
    assert_presents(&app, &role, RolePresentationView::Disconnected)?;
    Ok(())
}

/// The typed `World::get::<KernelRolePresentation>` path every other presentation test uses
/// resolves whether or not the type is reflect-registered, so it cannot notice a lost
/// registration. Reflected readers -- BRP, scene serialization -- can see the published
/// component only through `AppTypeRegistry`, so assert the registration itself is there and
/// that it actually reflects the live component off the role entity.
#[test]
fn the_published_presentation_is_reachable_through_the_type_registry() -> Result<(), Box<dyn Error>>
{
    let (app, role) = presentation_fixture(
        REFLECTED_PRESENTATION_ROLE,
        FixtureEvidence::NoCompletedScan,
    )?;
    let role_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    let type_path = <KernelRolePresentation as TypePath>::type_path();

    let reflect_component = app
        .world()
        .resource::<AppTypeRegistry>()
        .read()
        .get_with_type_path(type_path)
        .ok_or_else(|| format!("`{type_path}` is not registered in AppTypeRegistry"))
        .and_then(|registration| {
            registration
                .data::<ReflectComponent>()
                .cloned()
                .ok_or_else(|| {
                    format!("the `{type_path}` registration carries no ReflectComponent data")
                })
        })?;

    let reflects_the_published_component = reflect_component
        .reflect(app.world().entity(role_entity))
        .is_some();

    assert!(
        reflects_the_published_component,
        "role `{role}` exposes no reflected `{type_path}` on its role entity"
    );
    Ok(())
}

fn default_binding_policy() -> BindingPolicy {
    BindingPolicy::new(
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        OnAbort::default(),
        OnSessionLoss::default(),
        ApplyDeadline::ProcessDefault,
    )
}

fn established_role(binding_policy: BindingPolicy) -> Result<(App, RoleKey), Box<dyn Error>> {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(RiggingPlugin)
        .register_device_scheme(SchemeName::new(TEST_DEVICE_SCHEME)?);

    let device = reported_key(
        DeviceKind::ControlSurface,
        TEST_DEVICE_SCHEME,
        TEST_DEVICE_VALUE,
    )?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([scan![ScriptedDevice::present(device.clone())]]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            ReporterCoverage::MatchingEvidenceOnly,
            Duration::from_secs(1),
        ),
    );
    let (driver, control) = ScriptedDriver::<FlowConfiguration>::new();
    let driver = app.add_endpoint_driver(driver);
    let role = RoleKey::new(TEST_ROLE)?;
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            driver,
            FlowConfiguration,
            binding_policy,
        ),
    )?;

    advance_reporter(&mut app, reporter)?;
    let attempt = (0..8)
        .find_map(|_| {
            app.update();
            control.pending_attempts().first().copied()
        })
        .ok_or("the external-client driver received no apply attempt")?;
    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    for _ in 0..8 {
        app.update();
        if role_status(&app, &role)
            .is_ok_and(|status| matches!(status.view(), RoleStatusView::Established { .. }))
        {
            return Ok((app, role));
        }
    }

    Err("the external-client role did not establish a session".into())
}

fn role_presentation<'app>(
    app: &'app App,
    role: &RoleKey,
) -> Result<&'app KernelRolePresentation, Box<dyn Error>> {
    let role_entity = app.world().resource::<Bindings>().role_entity(role)?;
    app.world()
        .get::<KernelRolePresentation>(role_entity)
        .ok_or_else(|| format!("role `{role}` has no published presentation").into())
}

/// Assert the kernel published exactly this operator presentation for `role`.
fn assert_presents(
    app: &App,
    role: &RoleKey,
    expected: RolePresentationView<RoleUnavailableCause>,
) -> Result<(), Box<dyn Error>> {
    let published = role_presentation(app, role)?.view();
    if published == &expected {
        return Ok(());
    }
    Err(format!("role `{role}` presents {published:?}, expected {expected:?}").into())
}

fn role_status<'app>(app: &'app App, role: &RoleKey) -> Result<&'app RoleStatus, Box<dyn Error>> {
    let role_entity = app.world().resource::<Bindings>().role_entity(role)?;
    app.world()
        .get::<RoleStatus>(role_entity)
        .ok_or_else(|| format!("role `{role}` has no published status").into())
}

/// Device evidence a presentation fixture publishes before its role is read.
///
/// Named cases rather than an absent scan because the empty case is a state the kernel reports
/// on — every covering reporter is still awaited — not a missing argument.
#[derive(Clone, Copy)]
enum FixtureEvidence {
    /// A covering reporter is registered and enabled but has completed no scan, so the key is
    /// still awaiting its first report.
    NoCompletedScan,
    /// A completed scan reports the bound device as unreachable.
    UnreachableDevice,
    /// A covering reporter's completed scan establishes that the bound device is gone.
    AbsentDevice,
}

/// Register one role against a device whose evidence never reaches a usable conclusion.
fn presentation_fixture(
    role_name: &str,
    evidence: FixtureEvidence,
) -> Result<(App, RoleKey), Box<dyn Error>> {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(RiggingPlugin)
        .register_device_scheme(SchemeName::new(TEST_DEVICE_SCHEME)?);

    let device = reported_key(DeviceKind::ControlSurface, TEST_DEVICE_SCHEME, role_name)?;
    let (scans, coverage) = match evidence {
        FixtureEvidence::NoCompletedScan => (Vec::new(), authoritative_coverage()?),
        FixtureEvidence::UnreachableDevice => (
            vec![ScriptedScan::Complete(vec![ScriptedDevice::unreachable(
                device.clone(),
                UNREACHABLE_SINCE,
            )])],
            ReporterCoverage::MatchingEvidenceOnly,
        ),
        FixtureEvidence::AbsentDevice => (
            vec![ScriptedScan::Complete(vec![ScriptedDevice::absent(
                device.clone(),
            )])],
            authoritative_coverage()?,
        ),
    };
    let reporter = app.add_device_reporter(
        ScriptedReporter::new(scans),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            coverage,
            Duration::from_secs(1),
        ),
    );
    let (driver, _control) = ScriptedDriver::<FlowConfiguration>::new();
    let driver = app.add_endpoint_driver(driver);
    let role = RoleKey::new(role_name)?;
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            driver,
            FlowConfiguration,
            default_binding_policy(),
        ),
    )?;

    match evidence {
        FixtureEvidence::NoCompletedScan => {},
        FixtureEvidence::UnreachableDevice | FixtureEvidence::AbsentDevice => {
            advance_reporter(&mut app, reporter)?;
        },
    }
    app.update();

    Ok((app, role))
}

#[test]
fn production_prelude_compiles_without_test_support() -> Result<(), Box<dyn Error>> {
    let hana_rigging_path = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture_manifest = hana_rigging_path.join("tests/external_client/Cargo.toml");
    let manifest_file = File::open(&fixture_manifest).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "external client fixture manifest {} could not be read: {error}",
                fixture_manifest.display()
            ),
        )
    })?;
    drop(manifest_file);

    let workspace_root = hana_rigging_path.join("../..");
    let workspace_target_directory = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_root.join("target"), PathBuf::from);
    let target_directory = workspace_target_directory.join("external-client");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));

    let output = Command::new(cargo)
        .arg("check")
        .arg("--manifest-path")
        .arg(&fixture_manifest)
        .arg("--target-dir")
        .arg(target_directory)
        .arg("--offline")
        .output()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "could not run Cargo for external client fixture {}: {error}",
                    fixture_manifest.display()
                ),
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(io::Error::other(format!(
        "external client without test-support did not compile ({status}):\n\
         --- stdout ---\n{stdout}\n\
         --- stderr ---\n{stderr}",
        status = output.status,
        stdout = String::from_utf8_lossy(&output.stdout),
        stderr = String::from_utf8_lossy(&output.stderr),
    ))
    .into())
}
