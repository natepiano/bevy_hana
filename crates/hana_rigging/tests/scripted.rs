//! The hardware-free rule suite.
//!
//! Every case here drives the kernel through `hana_rigging_scripted::ScriptedReporter`, which
//! replays a written list of whole-set scans. No test touches hardware, and no test mocks anything
//! but the scan list and the driver: a rule that cannot be reached this way has put input or output
//! inside the kernel, which is the defect this suite exists to catch.

use std::any::TypeId;
use std::collections::HashMap;
use std::error::Error;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::time::Duration;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::app::Update;
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::change_detection::Tick;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::platform::time::Instant;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::On;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::reflect::TypePath;
use bevy::reflect::TypeRegistration;
use bevy::reflect::TypeRegistry;
use bevy::reflect::serde::ReflectSerializer;
use bevy::reflect::structs::Struct;
use bevy::time::Real;
use bevy::time::Time;
use bevy::time::TimeUpdateStrategy;
use hana_rigging::prelude::AdoptionOutcome;
use hana_rigging::prelude::Applied;
use hana_rigging::prelude::ApplyContext;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::AttemptCompletion;
use hana_rigging::prelude::AttemptEndingView;
use hana_rigging::prelude::AttemptInvalidation;
use hana_rigging::prelude::AttemptInvalidationView;
use hana_rigging::prelude::AttemptOutcomeView;
use hana_rigging::prelude::AttemptRef;
use hana_rigging::prelude::AuthoritativeReporterCoverage;
use hana_rigging::prelude::BindingAuthoring;
use hana_rigging::prelude::BindingPolicy;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::Capabilities;
use hana_rigging::prelude::CapabilityProjectionFailure;
use hana_rigging::prelude::CapabilityProjectionStatus;
use hana_rigging::prelude::Claim;
use hana_rigging::prelude::ClaimHolder;
use hana_rigging::prelude::CompletedDiscoveryOutcome;
use hana_rigging::prelude::ConfiguredDevice;
use hana_rigging::prelude::ConfiguredDeviceConnection;
use hana_rigging::prelude::ConfiguredDeviceMode;
use hana_rigging::prelude::ConfiguredDeviceName;
use hana_rigging::prelude::CoveredDeviceIdentitySpace;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::DeviceAccessErrorView;
use hana_rigging::prelude::DeviceArrived;
use hana_rigging::prelude::DeviceChange;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DeviceIdSource;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceResolution;
use hana_rigging::prelude::DeviceRevisionLookup;
use hana_rigging::prelude::DeviceStateLookup;
use hana_rigging::prelude::DeviceStatus;
use hana_rigging::prelude::Devices;
use hana_rigging::prelude::Digest;
use hana_rigging::prelude::DiscoveryBatchId;
use hana_rigging::prelude::DiscoveryCadence;
use hana_rigging::prelude::DiscoveryControl;
use hana_rigging::prelude::DiscoveryFinished;
use hana_rigging::prelude::DiscoveryLimits;
use hana_rigging::prelude::DiscoveryProgress;
use hana_rigging::prelude::DiscoveryProgressChanged;
use hana_rigging::prelude::DriverCleanupRoleEntity;
use hana_rigging::prelude::DriverCompletion;
use hana_rigging::prelude::DriverStopReasonView;
use hana_rigging::prelude::EndedRegistrationLifetime;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointDriverRegistration;
use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::EstablishedContext;
use hana_rigging::prelude::FailureRunStatus;
use hana_rigging::prelude::FirstCompleteSetStatus;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::HardwareWait;
use hana_rigging::prelude::IdentityAdoptionPreparation;
use hana_rigging::prelude::IdentityAnswer;
use hana_rigging::prelude::IdentityChanged;
use hana_rigging::prelude::IdentityDecisionOwed;
use hana_rigging::prelude::IdentityDecisions;
use hana_rigging::prelude::IdentityQuestion;
use hana_rigging::prelude::IdentityQuestionLookup;
use hana_rigging::prelude::IdentityQuestionRaised;
use hana_rigging::prelude::IdentityQuestionState;
use hana_rigging::prelude::IdentityVerdict;
use hana_rigging::prelude::KeyAvailability;
use hana_rigging::prelude::LastKnownGoodConfigurationAccessError;
use hana_rigging::prelude::LiveRoleChange;
use hana_rigging::prelude::LiveRoleChanged;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::PartName;
use hana_rigging::prelude::PlatformDeviceHandle;
use hana_rigging::prelude::Presence;
use hana_rigging::prelude::PreviousSuccess;
use hana_rigging::prelude::ReapplyConfiguration;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::RegistrationAttemptEnded;
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::ReporterActivation;
use hana_rigging::prelude::ReporterActivityView;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterDeferral;
use hana_rigging::prelude::ReporterHealth;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterOutcomeHealth;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::ReporterResume;
use hana_rigging::prelude::ResolvedToDevice;
use hana_rigging::prelude::ResumeCondition;
use hana_rigging::prelude::RetireRole;
use hana_rigging::prelude::RetiredRoleChange;
use hana_rigging::prelude::RetiredRoleChanged;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingLimits;
use hana_rigging::prelude::RiggingPlugin;
use hana_rigging::prelude::RiggingRevision;
use hana_rigging::prelude::RiggingSystems;
use hana_rigging::prelude::RoleEndpoint;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::SessionDatumArrivalEvidence;
use hana_rigging::prelude::SessionLease;
use hana_rigging::prelude::SessionRef;
use hana_rigging::prelude::SessionReleaseCause;
use hana_rigging::prelude::StartupDiscoveryChanged;
use hana_rigging::prelude::StartupDiscoveryState;
use hana_rigging::prelude::StoppedStatusView;
use hana_rigging::prelude::TargetResolution;
use hana_rigging::prelude::TargetResolutionContext;
use hana_rigging::prelude::UnconfirmedBasis;
use hana_rigging::prelude::WaitTiming;
use hana_rigging::prelude::WaitingStatusView;
use hana_rigging::prelude::WaitingWork;
use hana_rigging::prelude::register_binding;
use hana_rigging::prelude::replace_binding;
use hana_rigging_scripted::ScriptedDevice;
use hana_rigging_scripted::ScriptedDriver;
use hana_rigging_scripted::ScriptedDriverControl;
use hana_rigging_scripted::ScriptedDriverControlError;
use hana_rigging_scripted::ScriptedReporter;
use hana_rigging_scripted::ScriptedScan;
use hana_rigging_scripted::advance_reporter;
use hana_rigging_scripted::advance_until_accepted;
use hana_rigging_scripted::advance_until_running;
use hana_rigging_scripted::install_scripted_io_task_pool;
use hana_rigging_scripted::reported_key;
use hana_rigging_scripted::scan;

/// Identity space every scripted panel in this suite is named in.
const PANEL_SCHEME: &str = "usb-serial";

/// Slot every scripted role asks its driver to move the endpoint to.
const REQUESTED_SLOT: u32 = 1;

/// Transport text the scripted driver reports when a case asks it to fail an apply.
const DRIVER_FAILURE_DETAIL: &str = "the scripted driver was asked to refuse this apply";

/// Unsupported-operation text returned by the scripted driver.
const UNSUPPORTED_DRIVER_DETAIL: &str =
    "the scripted driver has no implementation for this operation";

/// Transport text a scripted reporter reports when a case asks it to fail an enumeration.
const DISCOVERY_FAILURE_DETAIL: &str = "the scripted reporter was asked to fail this scan";

/// Platform contract text used by scripted unsupported discovery outcomes.
const UNSUPPORTED_DISCOVERY_DETAIL: &str =
    "the scripted platform has no physical-device identity contract";

/// Placement the scripted role asks its driver for.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct PanelPlacement {
    slot: u32,
}

#[derive(Component, Debug, PartialEq, Reflect)]
#[reflect(Component, PartialEq)]
struct ScanRateCapability(u32);

#[derive(Component, Debug, PartialEq, Reflect)]
#[reflect(Component, PartialEq)]
struct KeyedDisplayCapability;

#[derive(Component, Debug, PartialEq, Reflect)]
#[reflect(Component, PartialEq)]
struct EvidenceScreenCapability;

#[derive(Component, Debug, PartialEq, Reflect)]
struct AlphaRegistrationMissing;

#[derive(Component, Debug, PartialEq, Reflect)]
struct ZetaRegistrationMissing;

#[derive(Component, Debug, PartialEq, Reflect)]
#[type_path = "scripted::capabilities"]
struct CustomPathRegistrationMissing;

#[derive(Component)]
#[relationship(relationship_target = RecoveryRoleClients)]
struct RecoveryClientRole(Entity);

#[derive(Component)]
#[relationship_target(relationship = RecoveryClientRole)]
struct RecoveryRoleClients(Vec<Entity>);

/// What the scripted driver was asked to do, in dispatch order.
///
/// Written through the `World` the driver is handed rather than kept in the driver value, because
/// the driver lives inside the kernel's registry and the test never sees it again after
/// registration.
#[derive(Default, Resource)]
struct DriverCalls {
    /// One entry per `start_apply`, holding the identifier the kernel allocated for it.
    started:                        Vec<AttemptRef>,
    /// How many more polls answer with a failure before the driver starts converging.
    ///
    /// Zero for every case that is not about retry, so a test opts into failure rather than
    /// spelling out a second driver type whose only difference is its first answer.
    failures_remaining:             usize,
    /// How many polls report that this platform cannot perform the operation.
    unsupported_failures_remaining: usize,
    sessions:                       HashMap<RoleKey, SessionLease<PanelPlacement>>,
    releases:                       Vec<SessionReleaseObservation>,
}

/// What the driver could read about its role at the moment the kernel released the session.
///
/// A bare `bool` cannot state the difference this suite exists to check: "the entity was there and
/// the role was not established" and "there was no entity left to ask" are different facts, and
/// the second is the whole reason the callback takes a [`DriverCleanupRoleEntity`] rather than an
/// [`Entity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoleStatusAtRelease {
    /// The role entity was live and its status read `Established`.
    Established,
    /// The role entity was live and its status read anything else.
    NotEstablished,
    /// The role entity was already despawned, so no status could be read.
    RoleEntityRemoved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionReleaseObservation {
    role:        RoleKey,
    role_entity: DriverCleanupRoleEntity,
    session:     SessionRef,
    cause:       SessionReleaseCause,
    role_status: RoleStatusAtRelease,
}

fn record_session_release(
    world: &mut World,
    role: &RoleKey,
    role_entity: DriverCleanupRoleEntity,
    session: SessionRef,
    cause: SessionReleaseCause,
) {
    let role_status = match role_entity {
        DriverCleanupRoleEntity::Live(entity) => {
            if world
                .get::<RoleStatus>(entity)
                .is_some_and(|status| matches!(status.view(), RoleStatusView::Established { .. }))
            {
                RoleStatusAtRelease::Established
            } else {
                RoleStatusAtRelease::NotEstablished
            }
        },
        DriverCleanupRoleEntity::Removed => RoleStatusAtRelease::RoleEntityRemoved,
    };
    let mut calls = world.resource_mut::<DriverCalls>();
    if calls
        .sessions
        .get(role)
        .is_some_and(|lease| lease.session_ref() == session)
    {
        calls.sessions.remove(role);
        calls.releases.push(SessionReleaseObservation {
            role: role.clone(),
            role_entity,
            session,
            cause,
            role_status,
        });
    }
}

/// Driver that records every dispatch and finishes its one-use completion synchronously.
struct RecordingDriver;

/// One `cancel_apply` dispatch, with everything the kernel told the driver about it.
///
/// A tuple carried the role entity alone until the cleanup callbacks began reporting a despawned
/// role: the invalidation cause is what separates a retired role from a displaced one, and the
/// despawned-entity cases turn on exactly that distinction.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AttemptCancellationObservation {
    role:        RoleKey,
    role_entity: DriverCleanupRoleEntity,
    attempt:     AttemptRef,
    cause:       AttemptInvalidation,
}

#[derive(Default, Resource)]
struct PendingDriverCalls {
    completions:   HashMap<AttemptRef, AttemptCompletion<PanelPlacement>>,
    cancellations: Vec<AttemptCancellationObservation>,
}

struct PendingDriver;

impl EndpointDriver for PendingDriver {
    type Configuration = PanelPlacement;
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
        world: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        _: &Self::Configuration,
        (): Self::Target,
    ) {
        let attempt = context.attempt();
        world
            .resource_mut::<PendingDriverCalls>()
            .completions
            .insert(attempt, context.into_completion());
    }

    fn established(&mut self, _: &mut World, _: EstablishedContext<'_, Self::Configuration>) {}

    fn cancel_apply(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        cause: AttemptInvalidation,
    ) {
        let mut calls = world.resource_mut::<PendingDriverCalls>();
        calls.completions.remove(&attempt);
        calls.cancellations.push(AttemptCancellationObservation {
            role: role.clone(),
            role_entity,
            attempt,
            cause,
        });
    }

    fn release_session(
        &mut self,
        _: &mut World,
        _: &RoleKey,
        _: DriverCleanupRoleEntity,
        _: SessionRef,
        _: SessionReleaseCause,
    ) {
    }
}

impl EndpointDriver for RecordingDriver {
    type Configuration = PanelPlacement;
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
        world: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        _: &Self::Configuration,
        (): Self::Target,
    ) {
        let attempt = context.attempt();
        let completion = context.into_completion();
        let outcome = {
            let mut driver_calls = world.resource_mut::<DriverCalls>();
            driver_calls.started.push(attempt);
            if driver_calls.unsupported_failures_remaining > 0 {
                driver_calls.unsupported_failures_remaining -= 1;
                DriverCompletion::Failed(DeviceAccessError::Unsupported {
                    detail: UNSUPPORTED_DRIVER_DETAIL.to_owned(),
                })
            } else if driver_calls.failures_remaining == 0 {
                DriverCompletion::Succeeded(Applied::AsDispatched)
            } else {
                driver_calls.failures_remaining -= 1;
                DriverCompletion::Failed(DeviceAccessError::Transport {
                    detail: DRIVER_FAILURE_DETAIL.to_owned(),
                })
            }
        };
        completion.finish(outcome);
    }

    fn established(
        &mut self,
        world: &mut World,
        context: EstablishedContext<'_, Self::Configuration>,
    ) {
        let role = context.role().clone();
        let lease = context.into_lease(SessionDatumArrivalEvidence::NoDatumObserved);
        world
            .resource_mut::<DriverCalls>()
            .sessions
            .insert(role, lease);
    }

    fn cancel_apply(
        &mut self,
        _: &mut World,
        _: &RoleKey,
        _: DriverCleanupRoleEntity,
        _: AttemptRef,
        _: AttemptInvalidation,
    ) {
    }

    fn release_session(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    ) {
        record_session_release(world, role, role_entity, session, cause);
    }
}

/// Driver that reaches its target only while one reporter's capability projection supports it.
struct CapabilityDriver<Capability> {
    reporter: ReporterId,
    marker:   PhantomData<fn(Capability)>,
}

#[derive(Resource)]
struct ScheduledCompletion {
    control:    ScriptedDriverControl<PanelPlacement>,
    attempt:    AttemptRef,
    completion: Option<DriverCompletion<PanelPlacement>>,
}

fn finish_scheduled_completion(mut scheduled: ResMut<ScheduledCompletion>) {
    let Some(completion) = scheduled.completion.take() else {
        return;
    };
    let result = scheduled
        .control
        .finish_attempt(scheduled.attempt, completion);
    assert!(
        result.is_ok(),
        "the scheduled completion must still belong to its attempt: {result:?}"
    );
}

impl<Capability> CapabilityDriver<Capability> {
    const fn new(reporter: ReporterId) -> Self {
        Self {
            reporter,
            marker: PhantomData,
        }
    }
}

impl<Capability> EndpointDriver for CapabilityDriver<Capability>
where
    Capability: Component + Reflect + TypePath,
{
    type Configuration = PanelPlacement;
    type Target = ();

    fn resolve_target(
        &mut self,
        world: &mut World,
        context: &TargetResolutionContext<'_>,
        _: &Self::Configuration,
    ) -> TargetResolution<Self::Target> {
        match context.required_capability::<Capability>(world, self.reporter) {
            Ok(_) => TargetResolution::Reached(()),
            Err(unavailable) => TargetResolution::Deferred(unavailable.target_wait(world)),
        }
    }

    fn start_apply(
        &mut self,
        _: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        _: &Self::Configuration,
        (): Self::Target,
    ) {
        context
            .into_completion()
            .finish(DriverCompletion::Succeeded(Applied::AsDispatched));
    }

    fn established(
        &mut self,
        world: &mut World,
        context: EstablishedContext<'_, Self::Configuration>,
    ) {
        let role = context.role().clone();
        let lease = context.into_lease(SessionDatumArrivalEvidence::NoDatumObserved);
        world
            .resource_mut::<DriverCalls>()
            .sessions
            .insert(role, lease);
    }

    fn cancel_apply(
        &mut self,
        _: &mut World,
        _: &RoleKey,
        _: DriverCleanupRoleEntity,
        _: AttemptRef,
        _: AttemptInvalidation,
    ) {
    }

    fn release_session(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    ) {
        record_session_release(world, role, role_entity, session, cause);
    }
}

/// Every event this suite observes, in arrival order per axis.
#[derive(Default, Resource)]
struct ObservedEvents {
    role_availability:            Vec<ObservedRoleAvailability>,
    arrivals:                     Vec<DeviceKey>,
    device_changes:               Vec<ObservedDeviceAvailabilityChange>,
    role_state:                   Vec<ObservedRoleLifecycle>,
    role_status:                  Vec<ObservedRoleStatusChange>,
    retired_roles:                Vec<ObservedRetiredRole>,
    attempt_endings:              Vec<AttemptEndingView>,
    registration_attempt_endings: Vec<ObservedRegistrationAttemptEnding>,
    publication_order:            Vec<ObservedRolePublication>,
    startup:                      Vec<StartupDiscoveryState>,
    discovery_progress:           Vec<ObservedDiscoveryProgress>,
    discovery_finished:           Vec<(DiscoveryBatchId, ReporterId, CompletedDiscoveryOutcome)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedDeviceAvailabilityChange {
    key:  DeviceKey,
    from: KeyAvailability,
    to:   KeyAvailability,
}

/// Complete readable status edge carried by one `LiveRoleChanged` event.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedRoleStatusChange {
    role: RoleKey,
    from: RoleStatusView,
    to:   RoleStatusView,
}

/// Durable identity carried by one `RetiredRoleChanged::Retired` event.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedRetiredRole {
    role:     RoleKey,
    endpoint: DeviceEndpoint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedRegistrationAttemptEnding {
    role:                  RoleKey,
    endpoint:              DeviceEndpoint,
    attempt:               AttemptRef,
    ending:                AttemptEndingView,
    lifetime:              EndedRegistrationLifetime,
    registration_was_live: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ObservedRolePublication {
    Status(RoleKey),
    LiveAttemptEnded {
        role:     RoleKey,
        binding:  Entity,
        endpoint: DeviceEndpoint,
        attempt:  AttemptRef,
        lifetime: ObservedRegistrationLifetime,
    },
    Retired(RoleKey),
    RegistrationAttemptEnded {
        role:     RoleKey,
        attempt:  AttemptRef,
        lifetime: ObservedRegistrationLifetime,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObservedRegistrationLifetime {
    Live,
    Displaced,
    Retired,
    RetirementBlocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObservedRoleLifecycle {
    Waiting,
    Applying(AttemptRef),
    Established,
    Stopped,
}

impl From<EndedRegistrationLifetime> for ObservedRegistrationLifetime {
    fn from(lifetime: EndedRegistrationLifetime) -> Self {
        match lifetime {
            EndedRegistrationLifetime::Displaced => Self::Displaced,
            EndedRegistrationLifetime::Retired => Self::Retired,
            EndedRegistrationLifetime::RetirementBlocked => Self::RetirementBlocked,
        }
    }
}

/// Which side of a role's availability edge one observed event reported.
///
/// One ordered list rather than two, because the rule under test is the alternation: a role that
/// went available, back to awaiting, and available again is indistinguishable from one that never
/// left if each side is counted on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ObservedRoleAvailability {
    /// A status edge entered a reporter-backed hardware wait.
    Awaiting(RoleKey, WaitingWork),
    /// A status edge left a reporter-backed hardware wait.
    Available(RoleKey),
}

/// Every identity question raised, in arrival order.
#[derive(Default, Resource)]
struct ObservedQuestions {
    raised: Vec<(RoleKey, DeviceKey)>,
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

fn observe_arrival(event: On<DeviceArrived>, mut observed: ResMut<ObservedEvents>) {
    observed.arrivals.push(event.key.clone());
}

fn observe_departure(event: On<DeviceChange>, mut observed: ResMut<ObservedEvents>) {
    let DeviceChange::Availability { key, from, to } = event.event();
    observed
        .device_changes
        .push(ObservedDeviceAvailabilityChange {
            key:  key.clone(),
            from: from.clone(),
            to:   to.clone(),
        });
}

const fn observed_role_lifecycle(status: &RoleStatusView) -> ObservedRoleLifecycle {
    match status {
        RoleStatusView::Waiting(_) => ObservedRoleLifecycle::Waiting,
        RoleStatusView::Applying { attempt, .. } => ObservedRoleLifecycle::Applying(*attempt),
        RoleStatusView::Established { .. } => ObservedRoleLifecycle::Established,
        RoleStatusView::Stopped(_) => ObservedRoleLifecycle::Stopped,
    }
}

fn observe_live_role_changed(
    event: On<LiveRoleChanged>,
    bindings: Res<Bindings>,
    mut observed: ResMut<ObservedEvents>,
) {
    match &event.change {
        LiveRoleChange::Status { from, to } => {
            observed
                .publication_order
                .push(ObservedRolePublication::Status(event.role.clone()));
            let state_from = observed_role_lifecycle(from.view());
            let state_to = observed_role_lifecycle(to.view());
            if state_from != state_to {
                observed.role_state.push(state_to);
            }
            let was_waiting = matches!(
                from.view(),
                RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
            );
            let is_waiting = matches!(
                to.view(),
                RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
            );
            match (was_waiting, is_waiting) {
                (false, true) => {
                    observed
                        .role_availability
                        .push(ObservedRoleAvailability::Awaiting(
                            event.role.clone(),
                            bindings.waiting_work(&event.role),
                        ));
                },
                (true, false) => observed
                    .role_availability
                    .push(ObservedRoleAvailability::Available(event.role.clone())),
                (false, false) | (true, true) => {},
            }
            observed.role_status.push(ObservedRoleStatusChange {
                role: event.role.clone(),
                from: from.view().clone(),
                to:   to.view().clone(),
            });
        },
        LiveRoleChange::AttemptEnded { attempt, ending } => {
            let Ok(binding) = bindings.binding(&event.role) else {
                return;
            };
            observed
                .publication_order
                .push(ObservedRolePublication::LiveAttemptEnded {
                    role:     event.role.clone(),
                    binding:  event.binding,
                    endpoint: binding.endpoint.clone(),
                    attempt:  *attempt,
                    lifetime: ObservedRegistrationLifetime::Live,
                });
            observed.attempt_endings.push(ending.clone());
        },
    }
}

fn observe_retired_role_changed(
    event: On<RetiredRoleChanged>,
    mut observed: ResMut<ObservedEvents>,
) {
    let RetiredRoleChange::Retired = &event.change;
    observed
        .publication_order
        .push(ObservedRolePublication::Retired(event.role.clone()));
    observed.retired_roles.push(ObservedRetiredRole {
        role:     event.role.clone(),
        endpoint: event.endpoint.clone(),
    });
}

fn observe_registration_attempt_ended(
    event: On<RegistrationAttemptEnded>,
    bindings: Res<Bindings>,
    mut observed: ResMut<ObservedEvents>,
) {
    observed
        .publication_order
        .push(ObservedRolePublication::RegistrationAttemptEnded {
            role:     event.role.clone(),
            attempt:  event.attempt,
            lifetime: event.lifetime.into(),
        });
    observed
        .registration_attempt_endings
        .push(ObservedRegistrationAttemptEnding {
            role:                  event.role.clone(),
            endpoint:              event.endpoint.clone(),
            attempt:               event.attempt,
            ending:                event.ending.clone(),
            lifetime:              event.lifetime,
            registration_was_live: bindings.role_entity(&event.role).is_ok(),
        });
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

/// Build an app with the kernel, the observers, and one scripted reporter that establishes absence.
fn scripted_app(scans: Vec<ScriptedScan>) -> Result<(App, ReporterId), Box<dyn Error>> {
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new(scans),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            std::time::Duration::from_secs(10),
        ),
    );

    Ok((app, reporter))
}

fn add_matching_scripted_reporter(app: &mut App, scans: Vec<ScriptedScan>) -> ReporterId {
    app.add_device_reporter(
        ScriptedReporter::new(scans),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            ReporterCoverage::MatchingEvidenceOnly,
            Duration::from_secs(10),
        ),
    )
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
        ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            panel_coverage()?,
            std::time::Duration::from_secs(10),
        ),
    );

    Ok((app, reporter))
}

/// The kernel, the scripted scheme, and every observer this suite reads events through.
fn observing_app() -> Result<App, Box<dyn Error>> {
    install_scripted_io_task_pool();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(RiggingPlugin)
        .init_resource::<DriverCalls>()
        .init_resource::<ObservedEvents>()
        .init_resource::<ObservedQuestions>()
        .register_device_scheme(SchemeName::new(PANEL_SCHEME)?)
        .add_observer(observe_arrival)
        .add_observer(observe_departure)
        .add_observer(observe_live_role_changed)
        .add_observer(observe_retired_role_changed)
        .add_observer(observe_registration_attempt_ended)
        .add_observer(observe_startup)
        .add_observer(observe_discovery_progress)
        .add_observer(observe_discovery_finished)
        .add_observer(observe_question_raised);

    Ok(app)
}

fn reporter_health(app: &App) -> Result<&ReporterHealth, Box<dyn Error>> {
    app.world()
        .iter_entities()
        .find_map(|entity| entity.get::<ReporterHealth>())
        .ok_or_else(|| "the registered reporter has no ReporterHealth component".into())
}

fn reporter_health_for(app: &App, reporter: ReporterId) -> Result<&ReporterHealth, Box<dyn Error>> {
    app.world()
        .iter_entities()
        .filter_map(|entity| entity.get::<ReporterHealth>())
        .find(|health| health.belongs_to(reporter))
        .ok_or_else(|| "the requested reporter has no ReporterHealth component".into())
}

fn reporter_health_change_tick(app: &App) -> Result<Tick, Box<dyn Error>> {
    app.world()
        .iter_entities()
        .find_map(|entity| entity.get_ref::<ReporterHealth>())
        .map(|health| health.last_changed())
        .ok_or_else(|| "the registered reporter has no ReporterHealth component".into())
}

fn device_entity(app: &App, key: &DeviceKey) -> Result<Entity, Box<dyn Error>> {
    app.world()
        .iter_entities()
        .find(|entity| entity.get::<DeviceKey>() == Some(key))
        .map(|entity| entity.id())
        .ok_or_else(|| format!("device `{key:?}` has no projected entity").into())
}

fn component_change_tick<ComponentType: Component>(
    app: &App,
    entity: Entity,
) -> Result<Tick, Box<dyn Error>> {
    app.world()
        .get_entity(entity)?
        .get_ref::<ComponentType>()
        .map(|component| component.last_changed())
        .ok_or_else(|| format!("entity `{entity}` has no requested component").into())
}

fn retained_device_presence(app: &App, key: &DeviceKey) -> Result<Presence, Box<dyn Error>> {
    let devices = app.world().resource::<Devices>();
    let DeviceResolution::Resolved(device_id) = devices.resolve(key) else {
        return Err("the reported device does not resolve".into());
    };
    let DeviceStateLookup::Retained(state) = devices.state(device_id) else {
        return Err("the reported device has no retained state".into());
    };

    Ok(state.presence)
}

fn reporter_health_json(app: &App) -> Result<String, Box<dyn Error>> {
    let type_registry = app.world().resource::<AppTypeRegistry>().read();
    let type_path = <ReporterHealth as TypePath>::type_path();
    let registration = type_registry.get_with_type_path(type_path).ok_or_else(|| {
        format!("ReporterHealth type path `{type_path}` is not registered in AppTypeRegistry")
    })?;
    let reflect_component = registration.data::<ReflectComponent>().ok_or_else(|| {
        format!("ReporterHealth registration `{type_path}` has no ReflectComponent data")
    })?;
    let health: &dyn Reflect = app
        .world()
        .iter_entities()
        .find_map(|entity| reflect_component.reflect(entity))
        .ok_or_else(|| {
            format!("no entity contains the reflected ReporterHealth component `{type_path}`")
        })?;
    let serializer = ReflectSerializer::new(health.as_partial_reflect(), &type_registry);
    let json = serde_json::to_string(&serializer)?;
    drop(type_registry);

    Ok(json)
}

fn reflected_component_json<ComponentType>(
    app: &App,
    entity: Entity,
) -> Result<String, Box<dyn Error>>
where
    ComponentType: Component + Reflect + TypePath,
{
    let type_registry = app.world().resource::<AppTypeRegistry>().read();
    let type_path = <ComponentType as TypePath>::type_path();
    let registration = type_registry
        .get_with_type_path(type_path)
        .ok_or_else(|| format!("component type path `{type_path}` is not registered"))?;
    let reflect_component = registration
        .data::<ReflectComponent>()
        .ok_or_else(|| format!("component registration `{type_path}` has no reflection data"))?;
    let entity_ref = app
        .world()
        .get_entity(entity)
        .map_err(|_| format!("entity {entity} is not live"))?;
    let reflected = reflect_component
        .reflect(entity_ref)
        .ok_or_else(|| format!("entity {entity} has no `{type_path}` component"))?;
    let serializer = ReflectSerializer::new(reflected.as_partial_reflect(), &type_registry);
    let json = serde_json::to_string(&serializer)?;
    drop(type_registry);

    Ok(json)
}

fn role_component<'app>(
    app: &'app App,
    role: &RoleKey,
) -> Result<&'app RoleStatus, Box<dyn Error>> {
    let binding = app.world().resource::<Bindings>().role_entity(role)?;
    app.world()
        .get::<RoleStatus>(binding)
        .ok_or_else(|| format!("role `{role}` has no readable status").into())
}

fn role_lifecycle(app: &App, role: &RoleKey) -> Result<ObservedRoleLifecycle, Box<dyn Error>> {
    Ok(observed_role_lifecycle(role_component(app, role)?.view()))
}

fn role_hardware_wait(app: &App, role: &RoleKey) -> Result<HardwareWait, Box<dyn Error>> {
    let RoleStatusView::Waiting(WaitingStatusView::Reporter(wait)) =
        role_component(app, role)?.view()
    else {
        return Err(format!("role `{role}` is not waiting on reporter evidence").into());
    };
    Ok(wait.clone())
}

/// Coverage that makes a scripted reporter's omission of a panel key mean the unit is gone.
///
/// Authoritative rather than `ReporterCoverage::MatchingEvidenceOnly`: an evidence-only reporter's
/// omission of a key proves nothing, so it can produce no departure, no `DeviceChange`, and no
/// absent-then-present reacquisition — the three things most of this suite is about.
fn panel_coverage() -> Result<ReporterCoverage, Box<dyn Error>> {
    Ok(ReporterCoverage::EstablishesAbsence(
        AuthoritativeReporterCoverage::one(CoveredDeviceIdentitySpace::ReportedScheme {
            kind:   DeviceKind::ControlSurface,
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
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                recovery,
                retry,
                OnAbort::default(),
                OnSessionLoss::default(),
                apply_deadline,
            ),
        ),
    )?;

    Ok(())
}

fn register_capability_role<Capability>(
    app: &mut App,
    reporter: ReporterId,
    role: &RoleKey,
    device: DeviceKey,
) -> Result<(), Box<dyn Error>>
where
    Capability: Component + Reflect + TypePath,
{
    let driver = app.add_endpoint_driver(CapabilityDriver::<Capability>::new(reporter));
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::Interval(Duration::ZERO),
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;
    Ok(())
}

const SCRIPTED_APPLY_DEADLINE: Duration = Duration::from_millis(10);
const SCRIPTED_APPLY_OVERRUN: Duration = Duration::from_millis(5);

struct CompletionFixture {
    app:      App,
    control:  ScriptedDriverControl<PanelPlacement>,
    driver:   EndpointDriverRegistration<PanelPlacement>,
    role:     RoleKey,
    attempt:  AttemptRef,
    hard_end: Instant,
}

fn completion_fixture() -> Result<CompletionFixture, Box<dyn Error>> {
    let key = panel_key("completion-boundary")?;
    let mut app = observing_app()?;
    let started_at = app.world().resource::<Time<Real>>().startup() + Duration::from_secs(1);
    app.insert_resource(TimeUpdateStrategy::ManualInstant(started_at));
    app.world_mut()
        .resource_mut::<RiggingLimits>()
        .apply_overrun = SCRIPTED_APPLY_OVERRUN;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([scan![ScriptedDevice::present(key.clone())]]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let (driver, control) = ScriptedDriver::<PanelPlacement>::new();
    let driver = app.add_endpoint_driver(driver);
    let role = RoleKey::new("completion-boundary")?;
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device: key,
                id:     EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::Interval(Duration::ZERO),
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::Authored(SCRIPTED_APPLY_DEADLINE),
            ),
        ),
    )?;
    advance_reporter(&mut app, reporter)?;
    for _ in 0..8 {
        if let Some(attempt) = control.pending_attempts().first().copied() {
            return Ok(CompletionFixture {
                app,
                control,
                driver,
                role,
                attempt,
                hard_end: started_at + SCRIPTED_APPLY_DEADLINE + SCRIPTED_APPLY_OVERRUN,
            });
        }
        app.update();
    }
    Err("the scripted completion driver received no attempt".into())
}

fn clear_role_publications(app: &mut App) {
    let mut observed = app.world_mut().resource_mut::<ObservedEvents>();
    observed.attempt_endings.clear();
    observed.registration_attempt_endings.clear();
    observed.publication_order.clear();
    observed.retired_roles.clear();
}

fn replacement_authoring(
    role: RoleKey,
    endpoint: DeviceEndpoint,
    driver: EndpointDriverRegistration<PanelPlacement>,
) -> BindingAuthoring<PanelPlacement> {
    BindingAuthoring::new(
        role,
        endpoint,
        driver,
        PanelPlacement {
            slot: REQUESTED_SLOT,
        },
        BindingPolicy::new(
            RecoveryPolicy::default(),
            RetryOn::Interval(Duration::ZERO),
            OnAbort::default(),
            OnSessionLoss::default(),
            ApplyDeadline::Authored(SCRIPTED_APPLY_DEADLINE),
        ),
    )
}

enum RegistrationChangeState {
    Replace {
        endpoint: DeviceEndpoint,
        driver:   EndpointDriverRegistration<PanelPlacement>,
    },
    Retire,
    Applied,
}

#[derive(Resource)]
struct RegistrationChangeBeforeAttemptEnding {
    role:  RoleKey,
    state: RegistrationChangeState,
}

fn change_registration_before_attempt_ending(
    event: On<LiveRoleChanged>,
    mut commands: Commands,
    mut bindings: ResMut<Bindings>,
    mut scheduled: ResMut<RegistrationChangeBeforeAttemptEnding>,
) {
    if !matches!(&event.change, LiveRoleChange::Status { .. }) || event.role != scheduled.role {
        return;
    }
    let state = std::mem::replace(&mut scheduled.state, RegistrationChangeState::Applied);
    let result = match state {
        RegistrationChangeState::Replace { endpoint, driver } => bindings
            .replace_authoring(replacement_authoring(event.role.clone(), endpoint, driver))
            .map(|_| ()),
        RegistrationChangeState::Retire => bindings.retire(&event.role).map(|_| ()),
        RegistrationChangeState::Applied => return,
    };
    if result.is_ok() {
        commands.entity(event.binding).despawn();
    }
}

#[derive(Resource)]
enum RoleEntityRemovalBeforeAttemptCompletion {
    Pending(Entity),
    Completed,
}

fn remove_role_entity_before_attempt_completion(world: &mut World) {
    let removal = {
        let mut removal = world.resource_mut::<RoleEntityRemovalBeforeAttemptCompletion>();
        std::mem::replace(
            &mut *removal,
            RoleEntityRemovalBeforeAttemptCompletion::Completed,
        )
    };
    if let RoleEntityRemovalBeforeAttemptCompletion::Pending(entity) = removal {
        world.entity_mut(entity).despawn();
    }
}

fn schedule_role_entity_removal_before_attempt_completion(app: &mut App, entity: Entity) {
    app.insert_resource(RoleEntityRemovalBeforeAttemptCompletion::Pending(entity))
        .add_systems(
            Update,
            remove_role_entity_before_attempt_completion
                .after(RiggingSystems::Reconcile)
                .before(RiggingSystems::Apply),
        );
}

#[derive(Resource)]
struct RoleEntityRemovalAfterRecovery {
    role: RoleKey,
}

fn remove_role_entity_after_recovery(world: &mut World) {
    let role = world
        .resource::<RoleEntityRemovalAfterRecovery>()
        .role
        .clone();
    let role_entity = world.resource::<Bindings>().role_entity(&role);
    if let Ok(role_entity) = role_entity
        && world.get_entity(role_entity).is_ok()
    {
        world.entity_mut(role_entity).despawn();
    }
}

fn schedule_role_entity_removal_after_recovery(app: &mut App, role: RoleKey) {
    app.insert_resource(RoleEntityRemovalAfterRecovery { role })
        .add_systems(
            Update,
            remove_role_entity_after_recovery
                .after(RiggingSystems::RoleEntityRecovery)
                .before(RiggingSystems::Reconcile),
        );
}

#[test]
fn a_live_attempt_ending_follows_its_status_edge() -> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    clear_role_publications(&mut app);

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();

    let observed = app.world().resource::<ObservedEvents>();
    let status_index = observed
        .publication_order
        .iter()
        .position(|publication| {
            matches!(publication, ObservedRolePublication::Status(published) if published == &role)
        })
        .ok_or("the successful attempt published no status edge")?;
    let ending_index = observed
        .publication_order
        .iter()
        .position(|publication| {
            matches!(
                publication,
                ObservedRolePublication::LiveAttemptEnded {
                    role: published_role,
                    attempt: published_attempt,
                    ..
                }
                    if published_role == &role && *published_attempt == attempt
            )
        })
        .ok_or("the successful attempt published no ending")?;
    assert!(status_index < ending_index);
    assert!(matches!(
        observed.attempt_endings.as_slice(),
        [AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(
            _
        ))]
    ));
    assert!(observed.registration_attempt_endings.is_empty());
    Ok(())
}

#[test]
fn a_matching_registration_receives_its_queued_ending_after_entity_recovery()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let removed_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    clear_role_publications(&mut app);
    schedule_role_entity_removal_before_attempt_completion(&mut app, removed_binding);

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();
    assert!(app.world().get_entity(removed_binding).is_err());
    assert!(
        app.world()
            .resource::<ObservedEvents>()
            .attempt_endings
            .is_empty()
    );
    assert!(
        app.world()
            .resource::<ObservedEvents>()
            .registration_attempt_endings
            .is_empty()
    );

    app.update();

    let recovered_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    assert_ne!(recovered_binding, removed_binding);
    let observed = app.world().resource::<ObservedEvents>();
    assert!(matches!(
        observed.attempt_endings.as_slice(),
        [AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(
            _
        ))]
    ));
    assert!(observed.registration_attempt_endings.is_empty());
    assert!(observed.publication_order.iter().any(|publication| {
        matches!(
            publication,
            ObservedRolePublication::LiveAttemptEnded {
                role: published_role,
                binding: published_binding,
                endpoint: published_endpoint,
                attempt: published_attempt,
                lifetime: ObservedRegistrationLifetime::Live,
            } if published_role == &role
                && *published_binding == recovered_binding
                && published_endpoint == &endpoint
                && *published_attempt == attempt
        )
    }));
    Ok(())
}

#[test]
fn an_ending_retires_when_role_entity_recovery_cannot_keep_an_entity_alive()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    clear_role_publications(&mut app);
    schedule_role_entity_removal_after_recovery(&mut app, role.clone());

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();
    assert!(
        app.world()
            .resource::<ObservedEvents>()
            .attempt_endings
            .is_empty()
    );
    assert!(
        app.world()
            .resource::<ObservedEvents>()
            .registration_attempt_endings
            .is_empty()
    );

    app.update();
    assert!(
        app.world()
            .resource::<Bindings>()
            .role_entity(&role)
            .is_err()
    );
    assert!(
        app.world()
            .resource::<ObservedEvents>()
            .registration_attempt_endings
            .is_empty()
    );

    app.update();
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert_eq!(
        observed.retired_roles,
        vec![ObservedRetiredRole {
            role:     role.clone(),
            endpoint: endpoint.clone(),
        }]
    );
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint: published_endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(_)),
            lifetime: EndedRegistrationLifetime::Retired,
            registration_was_live: false,
        }] if published_role == &role
            && published_endpoint == &endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

#[test]
fn a_different_endpoint_displaces_a_queued_ending_from_a_missing_entity()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let displaced_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let displaced_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    let successor_endpoint = DeviceEndpoint {
        device: panel_key("missing-ending-successor")?,
        id:     EndpointId::Whole,
    };
    clear_role_publications(&mut app);
    schedule_role_entity_removal_before_attempt_completion(&mut app, displaced_binding);

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();
    replace_binding(
        app.world_mut(),
        replacement_authoring(role.clone(), successor_endpoint.clone(), driver),
    )?;
    app.update();

    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.endpoint,
        successor_endpoint
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(_)),
            lifetime: EndedRegistrationLifetime::Displaced,
            registration_was_live: true,
        }] if published_role == &role
            && endpoint == &displaced_endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

#[test]
fn a_same_endpoint_successor_displaces_a_queued_ending_from_a_missing_entity()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let displaced_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let shared_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    clear_role_publications(&mut app);
    schedule_role_entity_removal_before_attempt_completion(&mut app, displaced_binding);

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();
    replace_binding(
        app.world_mut(),
        replacement_authoring(role.clone(), shared_endpoint.clone(), driver),
    )?;
    app.update();

    let successor_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    assert_ne!(successor_binding, displaced_binding);
    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.endpoint,
        shared_endpoint
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint: published_endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(_)),
            lifetime: EndedRegistrationLifetime::Displaced,
            registration_was_live: true,
        }] if published_role == &role
            && published_endpoint == &shared_endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

#[test]
fn retirement_before_entity_recovery_retires_the_queued_ending() -> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let retired_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let retired_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    clear_role_publications(&mut app);
    schedule_role_entity_removal_before_attempt_completion(&mut app, retired_binding);

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();
    app.world_mut().trigger(RetireRole { role: role.clone() });
    app.update();

    assert!(
        app.world()
            .resource::<Bindings>()
            .role_entity(&role)
            .is_err()
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(_)),
            lifetime: EndedRegistrationLifetime::Retired,
            registration_was_live: false,
        }] if published_role == &role
            && endpoint == &retired_endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

#[test]
fn a_live_attempt_ending_survives_displacement_before_publication() -> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let displaced_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let displaced_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    let successor_endpoint = DeviceEndpoint {
        device: panel_key("ending-publication-successor")?,
        id:     EndpointId::Whole,
    };
    clear_role_publications(&mut app);
    app.insert_resource(RegistrationChangeBeforeAttemptEnding {
        role:  role.clone(),
        state: RegistrationChangeState::Replace {
            endpoint: successor_endpoint.clone(),
            driver,
        },
    })
    .add_observer(change_registration_before_attempt_ending);

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();

    assert!(app.world().get_entity(displaced_binding).is_err());
    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.endpoint,
        successor_endpoint
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(_)),
            lifetime: EndedRegistrationLifetime::Displaced,
            registration_was_live: true,
        }] if published_role == &role
            && endpoint == &displaced_endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

#[test]
fn a_live_attempt_ending_survives_retirement_before_publication() -> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let retired_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let retired_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    clear_role_publications(&mut app);
    app.insert_resource(RegistrationChangeBeforeAttemptEnding {
        role:  role.clone(),
        state: RegistrationChangeState::Retire,
    })
    .add_observer(change_registration_before_attempt_ending);

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();

    assert!(app.world().get_entity(retired_binding).is_err());
    assert!(
        app.world()
            .resource::<Bindings>()
            .role_entity(&role)
            .is_err()
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(_)),
            lifetime: EndedRegistrationLifetime::Retired,
            registration_was_live: false,
        }] if published_role == &role
            && endpoint == &retired_endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

#[test]
fn a_replaced_attempt_ending_names_the_displaced_registration() -> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let displaced_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    let successor_endpoint = DeviceEndpoint {
        device: panel_key("replacement-successor")?,
        id:     EndpointId::Whole,
    };
    clear_role_publications(&mut app);

    replace_binding(
        app.world_mut(),
        replacement_authoring(role.clone(), successor_endpoint.clone(), driver),
    )?;
    app.update();

    assert_eq!(
        control.cancellations(),
        vec![(attempt, AttemptInvalidation::BindingReplaced)]
    );
    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.endpoint,
        successor_endpoint
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Invalidated(AttemptInvalidationView::BindingReplaced),
            lifetime: EndedRegistrationLifetime::Displaced,
            registration_was_live: true,
        }] if published_role == &role
            && endpoint == &displaced_endpoint
            && *published_attempt == attempt
    ));
    let status_index = observed
        .publication_order
        .iter()
        .rposition(|publication| {
            matches!(publication, ObservedRolePublication::Status(published) if published == &role)
        })
        .ok_or("the successor published no status edge")?;
    let ending_index = observed
        .publication_order
        .iter()
        .position(|publication| {
            matches!(
                publication,
                ObservedRolePublication::RegistrationAttemptEnded {
                    role: published_role,
                    attempt: published_attempt,
                    lifetime: ObservedRegistrationLifetime::Displaced,
                } if published_role == &role && *published_attempt == attempt
            )
        })
        .ok_or("the displaced registration published no ending")?;
    assert!(status_index < ending_index);
    Ok(())
}

#[test]
fn a_displaced_attempt_with_no_cleanup_entity_still_publishes_its_ending()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let displaced_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let displaced_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    let successor_endpoint = DeviceEndpoint {
        device: panel_key("removed-cleanup-successor")?,
        id:     EndpointId::Whole,
    };
    clear_role_publications(&mut app);

    replace_binding(
        app.world_mut(),
        replacement_authoring(role.clone(), successor_endpoint, driver),
    )?;
    app.world_mut().trigger(RetireRole { role: role.clone() });
    app.world_mut().entity_mut(displaced_binding).despawn();
    app.update();

    assert_eq!(
        control.cancellations().len(),
        1,
        "the driver is told to cancel even though the role entity is gone: the attempt has \
         hardware started and only the driver can undo it, and warning about the missing entity \
         instead left that work running with nothing left to observe it"
    );
    assert_eq!(
        control.cancellations()[0].0,
        attempt,
        "the cancellation names the attempt the kernel handed out, which is all a driver has to \
         key its own record by once the entity is gone"
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Invalidated(AttemptInvalidationView::BindingReplaced),
            lifetime: EndedRegistrationLifetime::Displaced,
            registration_was_live: false,
        }] if published_role == &role
            && endpoint == &displaced_endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

#[test]
fn a_retired_attempt_ending_follows_despawn_and_retirement() -> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    clear_role_publications(&mut app);

    app.world_mut().trigger(RetireRole { role: role.clone() });
    app.update();

    assert_eq!(
        control.cancellations(),
        vec![(attempt, AttemptInvalidation::RoleRetired)]
    );
    assert!(app.world().get_entity(binding).is_err());
    assert!(
        app.world()
            .resource::<Bindings>()
            .role_entity(&role)
            .is_err()
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint: published_endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Invalidated(AttemptInvalidationView::RoleRetired),
            lifetime: EndedRegistrationLifetime::Retired,
            registration_was_live: false,
        }] if published_role == &role
            && published_endpoint == &endpoint
            && *published_attempt == attempt
    ));
    let retired_index = observed
        .publication_order
        .iter()
        .position(|publication| {
            matches!(publication, ObservedRolePublication::Retired(published) if published == &role)
        })
        .ok_or("the retired role published no retirement")?;
    let ending_index = observed
        .publication_order
        .iter()
        .position(|publication| {
            matches!(
                publication,
                ObservedRolePublication::RegistrationAttemptEnded {
                    role: published_role,
                    attempt: published_attempt,
                    lifetime: ObservedRegistrationLifetime::Retired,
                } if published_role == &role && *published_attempt == attempt
            )
        })
        .ok_or("the retired registration published no ending")?;
    assert!(retired_index < ending_index);
    Ok(())
}

#[test]
fn a_retired_attempt_with_no_cleanup_entity_still_publishes_its_ending()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let retired_binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let retired_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    clear_role_publications(&mut app);

    app.world_mut().trigger(RetireRole { role: role.clone() });
    app.world_mut().entity_mut(retired_binding).despawn();
    app.update();

    assert_eq!(
        control.cancellations().len(),
        1,
        "the driver is told to cancel even though the role entity is gone: the attempt has \
         hardware started and only the driver can undo it, and warning about the missing entity \
         instead left that work running with nothing left to observe it"
    );
    assert_eq!(
        control.cancellations()[0].0,
        attempt,
        "the cancellation names the attempt the kernel handed out, which is all a driver has to \
         key its own record by once the entity is gone"
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Invalidated(AttemptInvalidationView::RoleRetired),
            lifetime: EndedRegistrationLifetime::Retired,
            registration_was_live: false,
        }] if published_role == &role
            && endpoint == &retired_endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

#[test]
fn a_retired_attempt_ending_never_reaches_a_same_key_successor() -> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let retired_endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();
    let successor_endpoint = DeviceEndpoint {
        device: panel_key("retired-role-successor")?,
        id:     EndpointId::Whole,
    };
    clear_role_publications(&mut app);

    app.world_mut().trigger(RetireRole { role: role.clone() });
    register_binding(
        app.world_mut(),
        replacement_authoring(role.clone(), successor_endpoint.clone(), driver),
    )?;
    app.update();

    assert_eq!(
        control.cancellations(),
        vec![(attempt, AttemptInvalidation::RoleRetired)]
    );
    assert_eq!(
        app.world().resource::<Bindings>().binding(&role)?.endpoint,
        successor_endpoint
    );
    let observed = app.world().resource::<ObservedEvents>();
    assert!(observed.attempt_endings.is_empty());
    assert!(matches!(
        observed.registration_attempt_endings.as_slice(),
        [ObservedRegistrationAttemptEnding {
            role: published_role,
            endpoint,
            attempt: published_attempt,
            ending: AttemptEndingView::Invalidated(AttemptInvalidationView::RoleRetired),
            lifetime: EndedRegistrationLifetime::Retired,
            registration_was_live: true,
        }] if published_role == &role
            && endpoint == &retired_endpoint
            && *published_attempt == attempt
    ));
    Ok(())
}

fn advance_frame_to(app: &mut App, instant: Instant) {
    app.insert_resource(TimeUpdateStrategy::ManualInstant(instant));
    app.update();
}

fn advance_past_departure_grace(app: &mut App) {
    let departure_grace = app.world().resource::<RiggingLimits>().departure_grace;
    let time = app.world().resource::<Time<Real>>();
    let now = time.last_update().unwrap_or_else(|| time.startup());
    advance_frame_to(app, now + departure_grace + Duration::from_nanos(1));
}

fn wait_for_established_session(
    app: &mut App,
    role: &RoleKey,
    settle_frames: usize,
) -> Result<SessionRef, Box<dyn Error>> {
    for _ in 0..8 {
        if app
            .world()
            .resource::<DriverCalls>()
            .sessions
            .contains_key(role)
        {
            break;
        }
        app.update();
    }
    for _ in 0..settle_frames {
        app.update();
    }
    app.world()
        .resource::<DriverCalls>()
        .sessions
        .get(role)
        .map(SessionLease::session_ref)
        .ok_or_else(|| format!("role `{role}` did not establish a driver session").into())
}

fn panel_key(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
    Ok(reported_key(
        DeviceKind::ControlSurface,
        PANEL_SCHEME,
        value,
    )?)
}

/// A panel key the reporter synthesized from location evidence, rather than one the unit itself
/// reported.
const fn synthesized_panel_key() -> DeviceKey {
    DeviceKey {
        kind: DeviceKind::ControlSurface,
        id:   DeviceIdSource::Synthesized {
            digest: Digest::new(0x5AFE_D001),
        },
    }
}

/// A record naming an unregistered scheme remains readable through the device register.
#[test]
fn an_unregistered_scheme_remains_readable() -> Result<(), Box<dyn Error>> {
    let mistyped = reported_key(DeviceKind::ControlSurface, "usb-seral", "CL15")?;
    let (mut app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(mistyped)]])?;

    advance_reporter(&mut app, reporter)?;

    let scheme = SchemeName::new("usb-seral")?;
    assert!(
        app.world()
            .resource::<Devices>()
            .unregistered_schemes()
            .contains(&scheme)
    );
    Ok(())
}

/// A device nobody else can use must not be seized, however long the role has waited for it.
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
    for _ in 0..8 {
        if !app.world().resource::<DriverCalls>().releases.is_empty() {
            break;
        }
        app.update();
    }

    assert_eq!(role_lifecycle(&app, &role)?, ObservedRoleLifecycle::Waiting);
    assert!(app.world().resource::<DriverCalls>().started.is_empty());

    Ok(())
}

/// A unit that returns during departure grace keeps its entity and must be applied again.
#[test]
fn a_return_during_departure_grace_cancels_retirement_and_reacquires() -> Result<(), Box<dyn Error>>
{
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
    app.world_mut()
        .resource_mut::<ObservedEvents>()
        .device_changes
        .clear();

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
    assert_eq!(observed.arrivals, vec![key.clone()]);
    let [returned] = observed.device_changes.as_slice() else {
        return Err(
            "the return did not publish exactly one post-baseline availability edge".into(),
        );
    };
    assert_eq!(returned.key, key);
    assert!(matches!(
        returned.from,
        KeyAvailability::DepartureGrace { .. }
    ));
    assert!(matches!(returned.to, KeyAvailability::Present(_)));
    assert_eq!(app.world().resource::<DriverCalls>().started.len(), 2);

    Ok(())
}

#[test]
fn departure_releases_the_established_session_before_reacquisition() -> Result<(), Box<dyn Error>> {
    let key = panel_key("session-departure")?;
    let (scripted_reporter, gate) = ScriptedReporter::gated(
        vec![
            scan![ScriptedDevice::present(key.clone())],
            scan![],
            scan![ScriptedDevice::present(key.clone())],
        ],
        DiscoveryProgress::Indeterminate,
    );
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        scripted_reporter,
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("session-departure")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::ReapplyOnReturn,
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_until_running(&mut app, reporter)?;
    gate.release();
    advance_until_accepted(&mut app, reporter)?;
    let first_session = wait_for_established_session(&mut app, &role, 4)?;
    let releases_before_departure = app.world().resource::<DriverCalls>().releases.len();
    app.world_mut()
        .resource_mut::<ObservedEvents>()
        .device_changes
        .clear();
    advance_until_running(&mut app, reporter)?;
    gate.release();
    advance_until_accepted(&mut app, reporter)?;
    for _ in 0..8 {
        if app.world().resource::<DriverCalls>().releases.len() > releases_before_departure {
            break;
        }
        app.update();
    }

    assert_eq!(role_lifecycle(&app, &role)?, ObservedRoleLifecycle::Waiting);
    assert_eq!(
        app.world()
            .resource::<ObservedEvents>()
            .device_changes
            .len(),
        1
    );
    let calls = app.world().resource::<DriverCalls>();
    assert_eq!(calls.releases.len(), releases_before_departure + 1);
    assert_eq!(
        calls.releases[releases_before_departure].session,
        first_session
    );
    assert!(matches!(
        calls.releases[releases_before_departure].cause,
        SessionReleaseCause::DeviceUnavailable { .. }
    ));
    assert_eq!(
        calls.releases[releases_before_departure].role_status,
        RoleStatusAtRelease::Established
    );
    assert!(!calls.sessions.contains_key(&role));

    advance_until_running(&mut app, reporter)?;
    gate.release();
    advance_until_accepted(&mut app, reporter)?;
    let second_session = wait_for_established_session(&mut app, &role, 0)?;
    let calls = app.world().resource::<DriverCalls>();
    assert_ne!(second_session, first_session);
    assert_eq!(calls.sessions.len(), 1);
    assert_eq!(calls.releases.len(), releases_before_departure + 1);

    Ok(())
}

#[test]
fn departure_cancels_an_apply_into_its_hardware_wait() -> Result<(), Box<dyn Error>> {
    let key = panel_key("departure-during-apply")?;
    let mut app = observing_app()?;
    app.init_resource::<PendingDriverCalls>();
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([
            ScriptedScan::Complete(vec![ScriptedDevice::present(key.clone())]),
            ScriptedScan::Complete(Vec::new()),
        ]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let driver = app.add_endpoint_driver(PendingDriver);
    let role = RoleKey::new("departure-during-apply")?;
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device: key.clone(),
                id:     EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;

    advance_reporter(&mut app, reporter)?;
    for _ in 0..8 {
        if !app
            .world()
            .resource::<PendingDriverCalls>()
            .completions
            .is_empty()
        {
            break;
        }
        app.update();
    }
    assert!(matches!(
        role_lifecycle(&app, &role)?,
        ObservedRoleLifecycle::Applying(_)
    ));

    advance_reporter(&mut app, reporter)?;
    for _ in 0..8 {
        if !app
            .world()
            .resource::<PendingDriverCalls>()
            .cancellations
            .is_empty()
        {
            break;
        }
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<PendingDriverCalls>()
            .cancellations
            .len(),
        1
    );
    assert!(matches!(
        role_hardware_wait(&app, &role)?,
        HardwareWait::DepartureGrace {
            key: waiting_key,
            ..
        } if waiting_key == key
    ));
    Ok(())
}

#[test]
fn a_revision_advance_restarts_an_invalidated_attempt_at_that_revision()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("revision-advance-during-apply")?;
    let (mut app, reporter) = scripted_app(vec![
        scan![ScriptedDevice::present(key.clone()).with_claim(Claim::Free)],
        scan![ScriptedDevice::present(key.clone()).with_claim(Claim::Held)],
    ])?;
    let (driver, control) = ScriptedDriver::<PanelPlacement>::new();
    let driver = app.add_endpoint_driver(driver);
    let role = RoleKey::new("revision-advance-during-apply")?;
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device: key.clone(),
                id:     EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;

    advance_reporter(&mut app, reporter)?;
    for _ in 0..8 {
        if !control.pending_attempts().is_empty() {
            break;
        }
        app.update();
    }
    let first_attempts = control.pending_attempts();
    let [first_attempt] = first_attempts.as_slice() else {
        return Err("the first device revision did not issue exactly one attempt".into());
    };
    let first_attempt = *first_attempt;
    let first_revision = {
        let devices = app.world().resource::<Devices>();
        let DeviceResolution::Resolved(device_id) = devices.resolve(&key) else {
            return Err("the scripted device did not resolve after its first report".into());
        };
        devices.revision(device_id)
    };

    advance_reporter(&mut app, reporter)?;
    for _ in 0..8 {
        if !control.cancellations().is_empty() && control.pending_attempts() != first_attempts {
            break;
        }
        app.update();
    }

    assert_eq!(
        control.cancellations(),
        vec![(first_attempt, AttemptInvalidation::RevisionAdvanced)]
    );
    let successor_attempts = control.pending_attempts();
    let [successor_attempt] = successor_attempts.as_slice() else {
        return Err("the advanced device revision did not issue exactly one successor".into());
    };
    assert_ne!(*successor_attempt, first_attempt);
    let advanced_revision = {
        let devices = app.world().resource::<Devices>();
        let DeviceResolution::Resolved(device_id) = devices.resolve(&key) else {
            return Err("the scripted device stopped resolving after its revision advanced".into());
        };
        devices.revision(device_id)
    };
    assert_ne!(advanced_revision, first_revision);
    assert!(matches!(
        role_lifecycle(&app, &role)?,
        ObservedRoleLifecycle::Applying(_)
    ));

    Ok(())
}

/// Run one role through arrival, departure, and return, and report what the kernel did.
///
/// The three cases below differ only in the policy they register, so the cycle is written once:
/// a shared body is what makes the assertions the whole content of each test, and what stops the
/// three from drifting into running different numbers of frames and comparing the results.
fn depart_and_return_under(
    recovery: RecoveryPolicy,
) -> Result<(App, RoleKey, usize), Box<dyn Error>> {
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
        key,
        recovery,
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, reporter)?;
    app.update();
    let applies_after_arrival = app.world().resource::<DriverCalls>().started.len();

    advance_reporter(&mut app, reporter)?;
    app.update();

    advance_reporter(&mut app, reporter)?;
    // The same frame count `an_absent_then_present_cycle_reacquires` needs, so a dispatch a policy
    // forbids gets as many chances to appear as the one that permits it.
    for _ in 0..16 {
        app.update();
    }

    Ok((app, role, applies_after_arrival))
}

/// Trigger an application's request that one role's saved configuration go back to its device.
///
/// The request only dispatches the apply; settling it takes the same frames a returning unit's
/// apply takes, so the caller sees the outcome rather than the attempt still in flight. A refusal
/// gets those frames too, which is what makes "nothing happened" mean nothing was ever going to.
fn request_reapply(app: &mut App, role: &RoleKey) -> Result<(), Box<dyn Error>> {
    let binding = app.world().resource::<Bindings>().role_entity(role)?;
    app.world_mut().trigger(ReapplyConfiguration { binding });
    for _ in 0..16 {
        app.update();
    }
    Ok(())
}

/// A policy that holds the kernel on departure must still hold it when the unit returns.
///
/// `RecoveryPolicy::ReapplyOnRequest` keeps the saved configuration and sends it only when asked.
/// Holding to that across a return is what `WaitingWork::ReapplyRequestOwed` is for: without
/// it the returning unit reaches `WaitingRole::Hardware` and has its authored request dispatched,
/// which is the automatic reapply this policy exists to decline. This is the counterpart of
/// `an_absent_then_present_cycle_reacquires`, and the pair is what makes the choice between the two
/// policies observable rather than a matter of reading the variant's documentation.
#[test]
fn a_request_policy_role_does_not_reacquire_when_its_unit_returns() -> Result<(), Box<dyn Error>> {
    let (app, role, applies_after_arrival) =
        depart_and_return_under(RecoveryPolicy::ReapplyOnRequest)?;

    assert_eq!(applies_after_arrival, 1);
    assert_eq!(
        app.world().resource::<Bindings>().waiting_work(&role),
        WaitingWork::ReapplyRequestOwed
    );
    assert_eq!(role_lifecycle(&app, &role)?, ObservedRoleLifecycle::Waiting);
    assert_eq!(app.world().resource::<DriverCalls>().started.len(), 1);

    Ok(())
}

#[test]
fn application_held_roles_publish_application_waits_after_return() -> Result<(), Box<dyn Error>> {
    let (request_app, request_role, _) = depart_and_return_under(RecoveryPolicy::ReapplyOnRequest)?;
    assert!(matches!(
        role_component(&request_app, &request_role)?.view(),
        RoleStatusView::Waiting(WaitingStatusView::ApplicationReapply { .. })
    ));

    let (forget_app, forget_role, _) = depart_and_return_under(RecoveryPolicy::Forget)?;
    assert!(matches!(
        role_component(&forget_app, &forget_role)?.view(),
        RoleStatusView::Waiting(WaitingStatusView::NewRegistration { .. })
    ));

    Ok(())
}

/// Every hold the kernel records has to be one the application can actually clear.
///
/// `a_request_policy_role_does_not_reacquire_when_its_unit_returns` proves the kernel stops; this
/// proves it starts again on the one move that is supposed to restart it. A hold with no reachable
/// payment is indistinguishable from a hold with one until a test sends the payment, so without
/// this case choosing the policy would silently leave the role unrecoverable.
#[test]
fn a_reapply_request_restarts_a_held_role() -> Result<(), Box<dyn Error>> {
    let (mut app, role, _) = depart_and_return_under(RecoveryPolicy::ReapplyOnRequest)?;

    request_reapply(&mut app, &role)?;

    assert_eq!(
        app.world().resource::<Bindings>().waiting_work(&role),
        WaitingWork::Nothing
    );
    assert_eq!(
        role_lifecycle(&app, &role)?,
        ObservedRoleLifecycle::Established
    );
    assert_eq!(app.world().resource::<DriverCalls>().started.len(), 2);

    Ok(())
}

#[test]
fn a_status_change_queued_before_entity_projection_publishes_its_complete_edge()
-> Result<(), Box<dyn Error>> {
    let (mut app, role, _) = depart_and_return_under(RecoveryPolicy::ReapplyOnRequest)?;
    let binding = app.world().resource::<Bindings>().role_entity(&role)?;
    let status_before = app
        .world()
        .get::<RoleStatus>(binding)
        .ok_or("the held role has no readable status before the request")?
        .view()
        .clone();
    app.world_mut()
        .resource_mut::<ObservedEvents>()
        .role_status
        .clear();

    app.world_mut().trigger(ReapplyConfiguration { binding });
    assert!(
        app.world()
            .resource::<ObservedEvents>()
            .role_status
            .is_empty()
    );
    app.update();

    let status_after = app
        .world()
        .get::<RoleStatus>(binding)
        .ok_or("the held role has no readable status after the request")?
        .view()
        .clone();
    let published = &app.world().resource::<ObservedEvents>().role_status;
    let first = published
        .first()
        .ok_or("the queued role status change produced no LiveRoleChanged event")?;
    let last = published
        .last()
        .ok_or("the queued role status change produced no final LiveRoleChanged event")?;
    assert!(published.iter().all(|change| change.role == role));
    assert_eq!(first.from, status_before);
    assert!(published.windows(2).all(|pair| pair[0].to == pair[1].from));
    assert_eq!(last.to, status_after);
    Ok(())
}

#[test]
fn retiring_a_role_publishes_its_durable_endpoint() -> Result<(), Box<dyn Error>> {
    let mut app = observing_app()?;
    let role = RoleKey::new("retired-panel")?;
    let key = panel_key("RETIRE")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;
    app.update();
    let endpoint = app
        .world()
        .resource::<Bindings>()
        .binding(&role)?
        .endpoint
        .clone();

    app.world_mut().trigger(RetireRole { role: role.clone() });
    assert!(
        app.world()
            .resource::<ObservedEvents>()
            .retired_roles
            .is_empty()
    );
    app.update();

    assert_eq!(
        app.world().resource::<ObservedEvents>().retired_roles,
        vec![ObservedRetiredRole { role, endpoint }]
    );
    Ok(())
}

/// A role whose saved value was discarded owes a registration, and says so.
///
/// `RecoveryPolicy::Forget` drops the configuration at the departure, so `ReapplyConfiguration` has
/// nothing to send and leaves the role exactly where it was. That refusal works only because the
/// hold is `WaitingWork::RegistrationOwed` rather than sharing one value with the requestable hold:
/// an application told only that a request is owed would send the one message the kernel silently
/// ignores, and wait forever on the reply.
#[test]
fn a_forgetful_role_owes_a_registration_no_request_can_pay() -> Result<(), Box<dyn Error>> {
    let (mut app, role, applies_after_arrival) = depart_and_return_under(RecoveryPolicy::Forget)?;

    assert_eq!(applies_after_arrival, 1);
    assert_eq!(
        app.world().resource::<Bindings>().waiting_work(&role),
        WaitingWork::RegistrationOwed
    );

    request_reapply(&mut app, &role)?;

    assert_eq!(
        app.world().resource::<Bindings>().waiting_work(&role),
        WaitingWork::RegistrationOwed
    );
    assert_eq!(role_lifecycle(&app, &role)?, ObservedRoleLifecycle::Waiting);
    assert_eq!(app.world().resource::<DriverCalls>().started.len(), 1);

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

#[test]
fn completions_before_and_at_the_hard_end_win_when_drained_later() -> Result<(), Box<dyn Error>> {
    for finish_before_end in [true, false] {
        let CompletionFixture {
            mut app,
            control,
            driver: _,
            role,
            attempt,
            hard_end,
        } = completion_fixture()?;
        let finished_at = if finish_before_end {
            hard_end
                .checked_sub(Duration::from_nanos(1))
                .ok_or("the hard end cannot precede the test origin")?
        } else {
            hard_end
        };
        advance_frame_to(&mut app, finished_at);
        control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
        advance_frame_to(&mut app, hard_end + Duration::from_nanos(1));

        let session = control
            .session_ref(&role)
            .ok_or("the accepted completion received no session lease")?;
        assert!(matches!(
            role_component(&app, &role)?.view(),
            RoleStatusView::Established {
                session: published,
                ..
            } if *published == session
        ));
        assert!(control.cancellations().is_empty());
        let established = app
            .world()
            .resource::<Bindings>()
            .binding(&role)?
            .last_known_good()?
            .downcast_ref::<PanelPlacement>()
            .ok_or("the requested placement was not retained as the last known good")?;
        assert_eq!(established.slot, REQUESTED_SLOT);
    }
    Ok(())
}

#[test]
fn an_accepted_success_mirrors_the_driver_configuration_on_the_role_entity()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;

    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();

    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Established { .. }
    ));
    app.update();

    let role_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    let mirrored_configuration = app
        .world()
        .get::<PanelPlacement>(role_entity)
        .ok_or("the established role entity has no mirrored driver configuration")?;
    assert_eq!(mirrored_configuration.slot, REQUESTED_SLOT);

    Ok(())
}

#[test]
fn a_completion_after_the_hard_end_is_refused_and_the_attempt_is_cancelled_once()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end,
    } = completion_fixture()?;
    app.insert_resource(ScheduledCompletion {
        control: control.clone(),
        attempt,
        completion: Some(DriverCompletion::Succeeded(Applied::AsDispatched)),
    });
    app.add_systems(
        Update,
        finish_scheduled_completion
            .after(RiggingSystems::Collect)
            .before(RiggingSystems::Apply),
    );

    advance_frame_to(&mut app, hard_end + Duration::from_nanos(1));

    assert_eq!(
        control.cancellations(),
        vec![(attempt, AttemptInvalidation::OverrunExhausted)]
    );
    assert_eq!(control.session_ref(&role), None);
    assert!(matches!(
        app.world()
            .resource::<ObservedEvents>()
            .attempt_endings
            .as_slice(),
        [AttemptEndingView::Invalidated(
            AttemptInvalidationView::OverrunExhausted
        )]
    ));
    Ok(())
}

#[test]
fn an_unfinished_attempt_expires_only_after_the_hard_end_and_cancels_once()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role: _,
        attempt,
        hard_end,
    } = completion_fixture()?;

    advance_frame_to(&mut app, hard_end);
    assert!(control.cancellations().is_empty());
    assert_eq!(control.pending_attempts(), vec![attempt]);

    advance_frame_to(&mut app, hard_end + Duration::from_nanos(1));
    assert_eq!(
        control.cancellations(),
        vec![(attempt, AttemptInvalidation::OverrunExhausted)]
    );
    assert!(!control.pending_attempts().contains(&attempt));
    Ok(())
}

#[test]
fn session_changes_update_last_known_good_and_restore_keeps_that_value()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    assert!(matches!(
        app.world()
            .resource::<Bindings>()
            .binding(&role)?
            .last_known_good(),
        Err(LastKnownGoodConfigurationAccessError::NotEstablished)
    ));

    control.finish_attempt(
        attempt,
        DriverCompletion::Succeeded(Applied::DiffersFromDispatched(PanelPlacement { slot: 2 })),
    )?;
    app.update();
    assert_eq!(
        app.world()
            .resource::<Bindings>()
            .binding(&role)?
            .last_known_good()?
            .downcast_ref::<PanelPlacement>()
            .ok_or("the differing placement had the wrong concrete type")?
            .slot,
        2
    );

    control.configuration_changed(&role, PanelPlacement { slot: 3 })?;
    app.update();
    assert_eq!(
        app.world()
            .resource::<Bindings>()
            .binding(&role)?
            .last_known_good()?
            .downcast_ref::<PanelPlacement>()
            .ok_or("the changed placement had the wrong concrete type")?
            .slot,
        3
    );

    control.report_loss(
        &role,
        DeviceAccessError::Absent {
            detail: "the scripted session ended".to_owned(),
        },
    )?;
    for _ in 0..4 {
        app.update();
        if !control.pending_attempts().is_empty() {
            break;
        }
    }
    let restoration = control
        .pending_attempts()
        .first()
        .copied()
        .ok_or("session loss did not start a restoration attempt")?;
    control.finish_attempt(
        restoration,
        DriverCompletion::Succeeded(Applied::AsDispatched),
    )?;
    app.update();
    assert_eq!(
        app.world()
            .resource::<Bindings>()
            .binding(&role)?
            .last_known_good()?
            .downcast_ref::<PanelPlacement>()
            .ok_or("the restored placement had the wrong concrete type")?
            .slot,
        3
    );
    Ok(())
}

fn spawn_recovery_clients(app: &mut App, role_entity: Entity) -> [Entity; 2] {
    [
        app.world_mut().spawn(RecoveryClientRole(role_entity)).id(),
        app.world_mut().spawn(RecoveryClientRole(role_entity)).id(),
    ]
}

fn assert_recovery_clients_retargeted(
    app: &App,
    clients: [Entity; 2],
    recovered_role_entity: Entity,
) -> Result<(), Box<dyn Error>> {
    for client in clients {
        assert_eq!(
            app.world()
                .get::<RecoveryClientRole>(client)
                .ok_or("a recovery client relationship was lost")?
                .0,
            recovered_role_entity
        );
    }
    Ok(())
}

fn assert_one_attempt_ending(app: &App) {
    assert_eq!(
        app.world()
            .resource::<ObservedEvents>()
            .attempt_endings
            .len(),
        1
    );
}

#[test]
fn role_entity_recovery_preserves_applying_queued_and_established_authorities()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    let applying_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    app.register_rigging_role_relationship::<RecoveryClientRole>();
    let applying_clients = spawn_recovery_clients(&mut app, applying_entity);
    app.world_mut().entity_mut(applying_entity).despawn();
    app.update();
    let repaired_applying_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    assert_ne!(repaired_applying_entity, applying_entity);
    assert_recovery_clients_retargeted(&app, applying_clients, repaired_applying_entity)?;
    assert_eq!(control.pending_attempts(), vec![attempt]);
    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();
    app.update();
    assert!(control.pending_attempts().is_empty());
    assert_one_attempt_ending(&app);
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Established { .. }
    ));

    let CompletionFixture {
        mut app,
        control,
        driver: _,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    app.register_rigging_role_relationship::<RecoveryClientRole>();
    let queued_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    let queued_clients = spawn_recovery_clients(&mut app, queued_entity);
    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.world_mut().entity_mut(queued_entity).despawn();
    app.update();
    let repaired_queued_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    assert_ne!(repaired_queued_entity, queued_entity);
    assert_recovery_clients_retargeted(&app, queued_clients, repaired_queued_entity)?;
    assert!(control.pending_attempts().is_empty());
    assert_one_attempt_ending(&app);
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Established { .. }
    ));

    let session = control
        .session_ref(&role)
        .ok_or("the queued completion received no session")?;
    let established_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    app.world_mut().entity_mut(established_entity).despawn();
    app.update();
    let repaired_established_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    assert_ne!(repaired_established_entity, established_entity);
    assert_recovery_clients_retargeted(&app, queued_clients, repaired_established_entity)?;
    assert_eq!(control.session_ref(&role), Some(session));
    assert_one_attempt_ending(&app);
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Established {
            session: published,
            ..
        } if *published == session
    ));
    let reported_loss = || DeviceAccessError::Absent {
        detail: "the recovered scripted session ended".to_owned(),
    };
    control.report_loss(&role, reported_loss())?;
    assert_eq!(control.session_ref(&role), None);
    assert!(matches!(
        control.report_loss(&role, reported_loss()),
        Err(ScriptedDriverControlError::UnknownSession { role: missing_role })
            if missing_role == role
    ));
    app.update();
    assert_eq!(
        control.releases(),
        vec![(session, SessionReleaseCause::ReportedLoss)]
    );
    app.update();
    assert_eq!(
        control.releases(),
        vec![(session, SessionReleaseCause::ReportedLoss)]
    );
    assert_one_attempt_ending(&app);
    Ok(())
}

fn assert_applying_cleanup_uses_the_recovered_role_entity() -> Result<(), Box<dyn Error>> {
    let applying_key = panel_key("cleanup-applying")?;
    let (mut applying_app, reporter) =
        scripted_app(vec![scan![ScriptedDevice::present(applying_key.clone())]])?;
    applying_app.init_resource::<PendingDriverCalls>();
    let driver = applying_app.add_endpoint_driver(PendingDriver);
    let role = RoleKey::new("cleanup-applying")?;
    register_binding(
        applying_app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device: applying_key,
                id:     EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;
    advance_reporter(&mut applying_app, reporter)?;
    for _ in 0..8 {
        if !applying_app
            .world()
            .resource::<PendingDriverCalls>()
            .completions
            .is_empty()
        {
            break;
        }
        applying_app.update();
    }
    let old_applying_entity = applying_app
        .world()
        .resource::<Bindings>()
        .role_entity(&role)?;
    applying_app
        .world_mut()
        .entity_mut(old_applying_entity)
        .despawn();
    replace_binding(
        applying_app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device: panel_key("cleanup-applying-replacement")?,
                id:     EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;
    applying_app.update();
    let recovered_applying_entity = applying_app
        .world()
        .resource::<Bindings>()
        .role_entity(&role)?;
    assert_ne!(recovered_applying_entity, old_applying_entity);
    let cancellations = &applying_app
        .world()
        .resource::<PendingDriverCalls>()
        .cancellations;
    assert_eq!(cancellations.len(), 1);
    assert_eq!(
        cancellations[0].role_entity,
        DriverCleanupRoleEntity::Live(recovered_applying_entity)
    );
    Ok(())
}

fn assert_established_cleanup_uses_the_recovered_role_entity() -> Result<(), Box<dyn Error>> {
    let established_key = panel_key("cleanup-established")?;
    let (mut established_app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(
        established_key.clone()
    )]])?;
    let established_role = RoleKey::new("cleanup-established")?;
    let established_driver = established_app.add_endpoint_driver(RecordingDriver);
    register_binding(
        established_app.world_mut(),
        BindingAuthoring::new(
            established_role.clone(),
            DeviceEndpoint {
                device: established_key,
                id:     EndpointId::Whole,
            },
            established_driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;
    advance_reporter(&mut established_app, reporter)?;
    for _ in 0..8 {
        if established_app
            .world()
            .resource::<DriverCalls>()
            .sessions
            .contains_key(&established_role)
        {
            break;
        }
        established_app.update();
    }
    let old_established_entity = established_app
        .world()
        .resource::<Bindings>()
        .role_entity(&established_role)?;
    established_app
        .world_mut()
        .entity_mut(old_established_entity)
        .despawn();
    replace_binding(
        established_app.world_mut(),
        BindingAuthoring::new(
            established_role.clone(),
            DeviceEndpoint {
                device: panel_key("cleanup-established-replacement")?,
                id:     EndpointId::Whole,
            },
            established_driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;
    established_app.update();
    let recovered_established_entity = established_app
        .world()
        .resource::<Bindings>()
        .role_entity(&established_role)?;
    assert_ne!(recovered_established_entity, old_established_entity);
    let releases = &established_app.world().resource::<DriverCalls>().releases;
    assert_eq!(releases.len(), 1);
    assert_eq!(
        releases[0].role_entity,
        DriverCleanupRoleEntity::Live(recovered_established_entity)
    );
    assert_eq!(releases[0].cause, SessionReleaseCause::BindingReplaced);

    Ok(())
}

#[test]
fn queued_driver_cleanup_uses_the_recovered_role_entity() -> Result<(), Box<dyn Error>> {
    assert_applying_cleanup_uses_the_recovered_role_entity()?;
    assert_established_cleanup_uses_the_recovered_role_entity()?;
    Ok(())
}

/// Author one role on a fresh scripted app, returning the app, its reporter, and the role.
///
/// The despawned-entity cases below need a role whose entity nothing else is competing for, so
/// each builds its own app rather than reusing a shared fixture. Both driver record resources are
/// installed before the reporter advances, because the first frame of discovery already dispatches
/// `start_apply` and a driver that reached for a missing resource there would panic in the
/// fixture rather than fail in the case.
fn cleanup_role_app<Driver>(
    name: &str,
    driver: Driver,
) -> Result<(App, ReporterId, RoleKey), Box<dyn Error>>
where
    Driver: EndpointDriver<Configuration = PanelPlacement>,
{
    let key = panel_key(name)?;
    let (mut app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(key.clone())]])?;
    app.init_resource::<PendingDriverCalls>();
    let driver = app.add_endpoint_driver(driver);
    let role = RoleKey::new(name)?;
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device: key,
                id:     EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;
    advance_reporter(&mut app, reporter)?;

    Ok((app, reporter, role))
}

/// Run frames until `ready` answers, then a few more, and report whether it ever did.
fn advance_until(app: &mut App, frames: usize, ready: impl Fn(&App) -> bool) -> bool {
    for _ in 0..frames {
        if ready(app) {
            return true;
        }
        app.update();
    }
    ready(app)
}

#[test]
fn a_despawned_established_role_entity_still_reaches_the_driver_to_release_its_session()
-> Result<(), Box<dyn Error>> {
    let (mut app, _reporter, role) =
        cleanup_role_app("cleanup-despawned-established", RecordingDriver)?;
    assert!(
        advance_until(&mut app, 8, |app| app
            .world()
            .resource::<DriverCalls>()
            .sessions
            .contains_key(&role)),
        "the fixture must reach an established session, or the despawn below proves nothing"
    );
    assert!(
        app.world().resource::<DriverCalls>().releases.is_empty(),
        "nothing has been released yet, so the single release asserted below is the despawn's"
    );

    // Retiring the role and despawning its entity in the same frame is how an application takes a
    // role entity away for good: the retirement records the entity it had, and nothing re-creates
    // it. That is the situation the callback's `Removed` variant exists for.
    let established_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    app.world_mut().trigger(RetireRole { role: role.clone() });
    app.world_mut().entity_mut(established_entity).despawn();
    app.update();

    assert!(
        app.world()
            .resource::<Bindings>()
            .role_entity(&role)
            .is_err(),
        "the retired role keeps no entity, or the driver was handed a live one and this test is \
         asserting the wrong variant"
    );
    let calls = app.world().resource::<DriverCalls>();
    assert_eq!(
        calls.releases.len(),
        1,
        "the session is released exactly once; a driver told twice would close a handle it no \
         longer owns"
    );
    assert_eq!(
        calls.releases[0].role_entity,
        DriverCleanupRoleEntity::Removed,
        "the driver is told the entity is gone rather than handed a dangling id it cannot tell \
         apart from a live one"
    );
    assert_eq!(
        calls.releases[0].cause,
        SessionReleaseCause::RoleRetired,
        "a role whose entity cannot be kept alive is retired, and `RoleRetired` is what tells the \
         driver this session is over for good rather than about to be replaced"
    );
    assert_eq!(
        calls.releases[0].role_status,
        RoleStatusAtRelease::RoleEntityRemoved,
        "there is no entity left to read a status from, which is precisely why the release is \
         dispatched on the kernel's own record rather than on anything read off the role"
    );
    assert_eq!(
        calls.releases[0].role, role,
        "the release names the role whose session it ends, which is all the driver has left to \
         key its own record by once the entity is gone"
    );
    assert!(
        !calls.sessions.contains_key(&role),
        "the driver actually dropped its session record, which is the point of the call: the \
         assertions above would all hold on a driver that was told and did nothing"
    );
    Ok(())
}

#[test]
fn a_despawned_applying_role_entity_still_reaches_the_driver_to_cancel_its_attempt()
-> Result<(), Box<dyn Error>> {
    let (mut app, _reporter, role) = cleanup_role_app("cleanup-despawned-applying", PendingDriver)?;
    assert!(
        advance_until(&mut app, 8, |app| !app
            .world()
            .resource::<PendingDriverCalls>()
            .completions
            .is_empty()),
        "the fixture must reach an attempt the driver is still holding, or the despawn below \
         proves nothing"
    );
    let attempt = *app
        .world()
        .resource::<PendingDriverCalls>()
        .completions
        .keys()
        .next()
        .ok_or("the pending driver retained no attempt completion")?;

    let applying_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    app.world_mut().trigger(RetireRole { role: role.clone() });
    app.world_mut().entity_mut(applying_entity).despawn();
    app.update();

    assert!(
        app.world()
            .resource::<Bindings>()
            .role_entity(&role)
            .is_err(),
        "the retired role keeps no entity, or the driver was handed a live one and this test is \
         asserting the wrong variant"
    );
    let calls = app.world().resource::<PendingDriverCalls>();
    assert_eq!(
        calls.cancellations,
        vec![AttemptCancellationObservation {
            role,
            role_entity: DriverCleanupRoleEntity::Removed,
            attempt,
            cause: AttemptInvalidation::RoleRetired,
        }],
        "the attempt is cancelled exactly once, naming the role, the attempt the kernel handed \
         out, and `RoleRetired` as the reason — a driver that branches on the cause has to be \
         able to tell a retirement from a replacement even when the entity is gone"
    );
    assert!(
        !calls.completions.contains_key(&attempt),
        "the driver actually dropped the completion it was holding; a cancel it was told about \
         and ignored would leave the one-use authority alive for an attempt the kernel abandoned"
    );
    Ok(())
}

#[test]
fn a_role_entity_the_kernel_records_but_the_world_lost_reaches_the_driver_as_removed()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("stranded-role-entity")?;
    // A gated reporter, so each scan is admitted only when this test releases it: the departure
    // below has to land after the despawn, and an ungated scripted reporter would let the empty
    // scan through while the role was still establishing.
    let (scripted_reporter, gate) = ScriptedReporter::gated(
        vec![scan![ScriptedDevice::present(key.clone())], scan![]],
        DiscoveryProgress::Indeterminate,
    );
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        scripted_reporter,
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("stranded-role-entity")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::ReapplyOnReturn,
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_until_running(&mut app, reporter)?;
    gate.release();
    advance_until_accepted(&mut app, reporter)?;
    let session = wait_for_established_session(&mut app, &role, 4)?;
    let established_entity = app.world().resource::<Bindings>().role_entity(&role)?;

    // Taking `RoleKey` off the entity before despawning it is what strands the recorded id, and it
    // is the only way to reach this state: the despawn hook reads the key off the entity to record
    // the loss, so an entity without one goes with nothing noticing. Role-entity recovery never
    // re-spawns it and `Bindings` keeps the dead id for the life of the binding, which is why
    // every later release has to ask the world rather than trust the record.
    app.world_mut()
        .entity_mut(established_entity)
        .remove::<RoleKey>();
    app.world_mut().entity_mut(established_entity).despawn();
    app.update();

    assert_eq!(
        app.world().resource::<Bindings>().role_entity(&role).ok(),
        Some(established_entity),
        "the kernel must still be recording the despawned id, or recovery re-spawned the entity \
         and the release below would be handed a live one — the ordinary case, not this one"
    );
    assert!(
        app.world().get_entity(established_entity).is_err(),
        "the recorded id must actually be dead, or this case proves nothing about a dangling one"
    );

    // The device departs. Releasing its session is one of the paths that reads the recorded role
    // entity rather than the world, so it is where the driver would be handed the dead id.
    let releases_before_departure = app.world().resource::<DriverCalls>().releases.len();
    advance_until_running(&mut app, reporter)?;
    gate.release();
    advance_until_accepted(&mut app, reporter)?;
    assert!(
        advance_until(&mut app, 8, |app| app
            .world()
            .resource::<DriverCalls>()
            .releases
            .len()
            > releases_before_departure),
        "the departure must actually release the session, or the assertions below read nothing"
    );

    let release = app
        .world()
        .resource::<DriverCalls>()
        .releases
        .last()
        .cloned()
        .ok_or("the departure released no session")?;
    assert_eq!(
        release.session, session,
        "the release names the session the driver was holding, so the observation read below is \
         the one this fixture set up"
    );
    assert!(
        matches!(release.cause, SessionReleaseCause::DeviceUnavailable { .. }),
        "the departure path is the one under test; another cause means the fixture ended the \
         session for a different reason: {:?}",
        release.cause
    );
    assert_eq!(
        release.role_entity,
        DriverCleanupRoleEntity::Removed,
        "a role entity the kernel still records but the world no longer holds reaches the driver \
         as `Removed`. `Live` promises a spawned entity the driver may read or write, so handing \
         over a dangling id would panic any driver that took the contract at its word"
    );
    assert_eq!(
        release.role_status,
        RoleStatusAtRelease::RoleEntityRemoved,
        "the driver could read no status off the role, which is the observable consequence of \
         being told the truth: an id it trusted would have been read instead"
    );
    Ok(())
}

#[test]
fn replace_then_retire_batch_resolves_displaced_driver_authorities_once()
-> Result<(), Box<dyn Error>> {
    let CompletionFixture {
        mut app,
        control,
        driver,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    replace_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device: panel_key("replace-retire-applying")?,
                id:     EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::Interval(Duration::ZERO),
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::Authored(SCRIPTED_APPLY_DEADLINE),
            ),
        ),
    )?;
    app.world_mut().trigger(RetireRole { role });
    app.update();

    assert_eq!(
        control.cancellations(),
        vec![(attempt, AttemptInvalidation::BindingReplaced)]
    );
    assert!(control.pending_attempts().is_empty());
    assert!(control.releases().is_empty());

    let CompletionFixture {
        mut app,
        control,
        driver,
        role,
        attempt,
        hard_end: _,
    } = completion_fixture()?;
    control.finish_attempt(attempt, DriverCompletion::Succeeded(Applied::AsDispatched))?;
    app.update();
    app.update();
    let session = control
        .session_ref(&role)
        .ok_or("the successful attempt received no session lease")?;
    replace_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device: panel_key("replace-retire-established")?,
                id:     EndpointId::Whole,
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::default(),
                RetryOn::Interval(Duration::ZERO),
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::Authored(SCRIPTED_APPLY_DEADLINE),
            ),
        ),
    )?;
    app.world_mut().trigger(RetireRole { role: role.clone() });
    app.update();

    assert!(control.cancellations().is_empty());
    assert_eq!(
        control.releases(),
        vec![(session, SessionReleaseCause::BindingReplaced)]
    );
    assert_eq!(control.session_ref(&role), None);
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
/// panel still reported every other unit it named correctly, and a pass that dropped them all would
/// turn one ambiguous webcam into a total discovery outage.
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
        role_lifecycle(&app, &duplicated_role)?,
        ObservedRoleLifecycle::Waiting
    );
    // One dispatch, and the suppressed role is the one still waiting, so it was the unsuppressed
    // key that reached a driver.
    assert_eq!(app.world().resource::<DriverCalls>().started.len(), 1);

    let DeviceResolution::Resolved(single_id) = devices.resolve(&single) else {
        return Err("the unambiguous panel has no device handle".into());
    };
    let DeviceRevisionLookup::Retained(single_revision) = devices.revision(single_id) else {
        return Err("the unambiguous panel has no revision".into());
    };
    let status = app
        .world()
        .iter_entities()
        .filter_map(|entity| entity.get::<DeviceStatus>())
        .find(|status| status.key() == &single)
        .ok_or("the unambiguous panel has no DeviceStatus component")?;
    assert_eq!(status.id().get(), single_id.get());
    assert_eq!(status.revision().get(), single_revision.get());
    assert!(matches!(
        status.availability(),
        KeyAvailability::Present(evidence) if !evidence.contributors().as_slice().is_empty()
    ));

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
            std::time::Duration::from_secs(10),
        ),
    );

    advance_until_running(&mut app, reporter)?;
    // The kernel marks a reporter running on the main thread as it spawns the job, so the run
    // arrives here with the `Indeterminate` placeholder still retained and the scripted progress
    // no further along than the I/O pool has taken it. Waiting for the job to reach its gate is
    // what makes the progress asserted below the progress the reporter actually sent.
    gate.wait_until_held()?;

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
/// consumer would ever see that the enumeration is broken.
#[test]
fn repeated_failures_publish_serializable_reporter_health() -> Result<(), Box<dyn Error>> {
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Failed(DeviceAccessError::Transport {
            detail: DISCOVERY_FAILURE_DETAIL.to_owned(),
        })]),
        ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            panel_coverage()?,
            Duration::ZERO,
        ),
    );

    for _ in 0..3 {
        advance_reporter(&mut app, reporter)?;
    }

    let health = reporter_health(&app)?;
    assert!(!health.identity().name.is_empty());
    assert_eq!(*health.retained_records(), 0);
    assert!(matches!(
        health.first_complete_set(),
        FirstCompleteSetStatus::Waiting(WaitTiming::Overdue { .. })
    ));
    let ReporterOutcomeHealth::Failing {
        batch,
        duration: _,
        run,
    } = health.outcome()
    else {
        return Err("the repeated discovery failure was not published as Failing".into());
    };
    assert!(batch.get() > 0);
    assert_eq!(run.consecutive.get(), 3);

    let json = reporter_health_json(&app)?;
    assert!(!json.is_empty());
    assert!(json.contains("ScriptedReporter"));
    assert!(json.contains("Failing"));

    Ok(())
}

/// An initial failure followed by sixty one-second cadence failures spans a full minute, reaches
/// only the one-minute milestone, and remains in that window on the following failure.
#[test]
fn one_minute_reporter_failure_run_follows_the_periodic_cadence() -> Result<(), Box<dyn Error>> {
    const FAILURES_AFTER_RUN_START: u32 = 60;
    const REPORTER_CADENCE: Duration = Duration::from_secs(1);

    let mut app = observing_app()?;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Failed(DeviceAccessError::Transport {
            detail: DISCOVERY_FAILURE_DETAIL.to_owned(),
        })]),
        ReporterRegistration::required(
            DiscoveryCadence::Periodic {
                interval: REPORTER_CADENCE,
            },
            panel_coverage()?,
            Duration::from_secs(30),
        ),
    );

    advance_until_accepted(&mut app, reporter)?;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(REPORTER_CADENCE));
    for _ in 0..FAILURES_AFTER_RUN_START {
        advance_until_accepted(&mut app, reporter)?;
    }

    let health = reporter_health(&app)?;
    assert!(matches!(
        health.first_complete_set(),
        FirstCompleteSetStatus::Waiting(WaitTiming::Overdue { .. })
    ));
    let ReporterOutcomeHealth::Failing { run, .. } = health.outcome() else {
        return Err("the one-minute reporter run was not published as Failing".into());
    };
    assert_eq!(run.consecutive.get(), FAILURES_AFTER_RUN_START + 1);
    assert_eq!(
        run.last_failure_at
            .elapsed()
            .saturating_sub(run.first_failure_at.elapsed()),
        Duration::from_secs(60)
    );

    advance_until_accepted(&mut app, reporter)?;
    let ReporterOutcomeHealth::Failing { run, .. } = reporter_health(&app)?.outcome() else {
        return Err("the repeated reporter failure left its typed failure run".into());
    };
    assert_eq!(run.consecutive.get(), FAILURES_AFTER_RUN_START + 2);

    Ok(())
}

#[test]
fn minute_one_role_status_names_the_overdue_reporter_and_saved_endpoint()
-> Result<(), Box<dyn Error>> {
    const FAILURES_AFTER_RUN_START: usize = 60;
    let key = panel_key("minute-one-panel")?;
    let role = RoleKey::new("minute-one-role")?;
    let mut app = observing_app()?;
    let scans = (0..=FAILURES_AFTER_RUN_START).map(|_| {
        ScriptedScan::Failed(DeviceAccessError::Transport {
            detail: DISCOVERY_FAILURE_DETAIL.to_owned(),
        })
    });
    let reporter = app.add_device_reporter(
        ScriptedReporter::new(scans),
        ReporterRegistration::required(
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(1),
            },
            panel_coverage()?,
            Duration::from_secs(30),
        ),
    );
    register_role(
        &mut app,
        &role,
        key.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_until_accepted(&mut app, reporter)?;
    let role_state_edges_before = app.world().resource::<ObservedEvents>().role_state.len();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(1)));
    for _ in 0..FAILURES_AFTER_RUN_START {
        advance_until_accepted(&mut app, reporter)?;
    }
    assert_eq!(
        app.world().resource::<ObservedEvents>().role_state.len(),
        role_state_edges_before
    );

    let binding_entity = app.world().resource::<Bindings>().role_entity(&role)?;
    let endpoint = app
        .world()
        .get::<RoleEndpoint>(binding_entity)
        .ok_or("the role entity has no readable endpoint")?;
    assert_eq!(endpoint.endpoint().device, key);
    let status = app
        .world()
        .get::<RoleStatus>(binding_entity)
        .ok_or("the role entity has no readable status")?;
    let reporter_ref = reporter_health(&app)?.identity().reporter_ref;
    assert!(matches!(
        status.view(),
        RoleStatusView::Waiting(WaitingStatusView::Reporter(
            HardwareWait::AwaitingFirstReport {
                key: waiting_key,
                reporters,
                timing: WaitTiming::Overdue { .. },
            }
        )) if waiting_key == &key && reporters.as_slice() == [reporter_ref]
    ));
    assert!(
        app.world()
            .get::<ResolvedToDevice>(binding_entity)
            .is_none()
    );

    let endpoint_json = reflected_component_json::<RoleEndpoint>(&app, binding_entity)?;
    let status_json = reflected_component_json::<RoleStatus>(&app, binding_entity)?;
    assert!(!endpoint_json.is_empty());
    assert!(!status_json.is_empty());

    Ok(())
}

#[test]
fn an_unchanged_complete_set_refreshes_an_expired_freshness_lease() -> Result<(), Box<dyn Error>> {
    const REPORTER_CADENCE: Duration = Duration::from_secs(1);

    let key = panel_key("freshness-panel")?;
    let mut app = observing_app()?;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut().resource_mut::<RiggingLimits>().report_grace = Duration::ZERO;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([scan![ScriptedDevice::present(key.clone())]]),
        ReporterRegistration::required(
            DiscoveryCadence::Periodic {
                interval: REPORTER_CADENCE,
            },
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );

    advance_until_accepted(&mut app, reporter)?;
    assert_eq!(retained_device_presence(&app, &key)?, Presence::Present);
    let revision_after_first_set = *app.world().resource::<RiggingRevision>();

    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(2)));
    app.update();
    assert!(matches!(
        retained_device_presence(&app, &key)?,
        Presence::Unreachable { .. }
    ));
    assert_eq!(
        *app.world().resource::<RiggingRevision>(),
        revision_after_first_set
    );

    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    assert_eq!(retained_device_presence(&app, &key)?, Presence::Present);
    assert_eq!(
        *app.world().resource::<RiggingRevision>(),
        revision_after_first_set
    );

    Ok(())
}

#[test]
fn expired_uncovered_absence_becomes_unreachable_once() -> Result<(), Box<dyn Error>> {
    const REPORTER_CADENCE: Duration = Duration::from_secs(1);

    let key = panel_key("expired-uncovered-absence")?;
    let mut app = observing_app()?;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut().resource_mut::<RiggingLimits>().report_grace = Duration::ZERO;
    let (scripted_reporter, gate) = ScriptedReporter::gated(
        [
            ScriptedScan::Complete(vec![ScriptedDevice::absent(key.clone())]),
            ScriptedScan::Complete(vec![ScriptedDevice::absent(key.clone())]),
        ],
        DiscoveryProgress::Indeterminate,
    );
    let reporter = app.add_device_reporter(
        scripted_reporter,
        ReporterRegistration::optional(
            DiscoveryCadence::Periodic {
                interval: REPORTER_CADENCE,
            },
            ReporterActivation::Enabled,
            ReporterCoverage::MatchingEvidenceOnly,
            Duration::from_secs(10),
        ),
    );

    advance_until_running(&mut app, reporter)?;
    gate.release();
    advance_until_accepted(&mut app, reporter)?;
    assert!(matches!(
        app.world()
            .get::<DeviceStatus>(device_entity(&app, &key)?)
            .ok_or("the uncovered device has no status")?
            .availability(),
        KeyAvailability::Unconfirmed {
            basis: UnconfirmedBasis::UncoveredAbsence { .. },
            ..
        }
    ));

    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(2)));
    app.update();
    assert!(matches!(
        app.world()
            .get::<DeviceStatus>(device_entity(&app, &key)?)
            .ok_or("the expired device has no status")?
            .availability(),
        KeyAvailability::Unreachable { .. }
    ));

    let settled_counts = observed_counts(&app);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    assert_eq!(observed_counts(&app), settled_counts);
    Ok(())
}

#[test]
fn one_expired_absence_reveals_another_reporters_fresh_presence() -> Result<(), Box<dyn Error>> {
    const ABSENCE_REPORTER_CADENCE: Duration = Duration::from_secs(10);

    let key = panel_key("expired-mixed-absence")?;
    let mut app = observing_app()?;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    {
        let mut limits = app.world_mut().resource_mut::<RiggingLimits>();
        limits.departure_grace = Duration::from_secs(1);
        limits.report_grace = Duration::ZERO;
    }
    let present_reporter = add_matching_scripted_reporter(
        &mut app,
        vec![ScriptedScan::Complete(vec![ScriptedDevice::present(
            key.clone(),
        )])],
    );
    let (absence_script, absence_gate) = ScriptedReporter::gated(
        [
            ScriptedScan::Complete(vec![ScriptedDevice::absent(key.clone())]),
            ScriptedScan::Complete(vec![ScriptedDevice::absent(key.clone())]),
        ],
        DiscoveryProgress::Indeterminate,
    );
    let absence_reporter = app.add_device_reporter(
        absence_script,
        ReporterRegistration::optional(
            DiscoveryCadence::Periodic {
                interval: ABSENCE_REPORTER_CADENCE,
            },
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );

    advance_reporter(&mut app, present_reporter)?;
    advance_until_running(&mut app, absence_reporter)?;
    absence_gate.release();
    advance_until_accepted(&mut app, absence_reporter)?;
    assert!(matches!(
        app.world()
            .get::<DeviceStatus>(device_entity(&app, &key)?)
            .ok_or("the mixed device has no grace status")?
            .availability(),
        KeyAvailability::DepartureGrace { .. }
    ));

    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(2)));
    app.update();
    assert_eq!(
        app.world().resource::<Devices>().resolve(&key),
        DeviceResolution::NotResolved
    );

    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(9)));
    app.update();
    let status = app
        .world()
        .get::<DeviceStatus>(device_entity(&app, &key)?)
        .ok_or("the revealed device has no status")?;
    let KeyAvailability::Present(evidence) = status.availability() else {
        return Err("fresh presence did not replace expired absence".into());
    };
    assert_eq!(evidence.contributors().as_slice().len(), 1);

    let settled_counts = observed_counts(&app);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    assert_eq!(observed_counts(&app), settled_counts);
    Ok(())
}

#[test]
fn disabled_optional_reporter_starts_its_bound_when_enabled() -> Result<(), Box<dyn Error>> {
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Complete(Vec::new())]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Disabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );

    let health = reporter_health(&app)?;
    assert_eq!(health.activity(), &ReporterActivityView::Disabled);
    assert!(matches!(
        health.first_complete_set(),
        FirstCompleteSetStatus::Waiting(WaitTiming::Unbounded { .. })
    ));

    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .enable(reporter)?;
    app.update();

    assert!(matches!(
        reporter_health(&app)?.first_complete_set(),
        FirstCompleteSetStatus::Waiting(WaitTiming::Bounded { .. })
    ));
    reporter_health_json(&app)?;

    Ok(())
}

#[test]
fn disabling_optional_reporter_restarts_its_first_complete_set_bound() -> Result<(), Box<dyn Error>>
{
    let first_complete_set_bound = Duration::from_millis(50);
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Complete(Vec::new())]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            first_complete_set_bound,
        ),
    );
    let FirstCompleteSetStatus::Waiting(WaitTiming::Bounded {
        deadline: original_deadline,
        ..
    }) = reporter_health(&app)?.first_complete_set()
    else {
        return Err("the enabled reporter did not start with a bounded wait".into());
    };
    let original_deadline = *original_deadline;

    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .disable(reporter)?;
    app.update();

    let disabled_health = reporter_health(&app)?;
    let disabled_since = match (
        disabled_health.activity(),
        disabled_health.first_complete_set(),
    ) {
        (
            ReporterActivityView::Disabled,
            FirstCompleteSetStatus::Waiting(WaitTiming::Unbounded { since }),
        ) => *since,
        _ => return Err("disabling the reporter did not suspend its first-set bound".into()),
    };

    std::thread::sleep(first_complete_set_bound + Duration::from_millis(25));
    app.update();
    assert!(matches!(
        (
            reporter_health(&app)?.activity(),
            reporter_health(&app)?.first_complete_set(),
        ),
        (
            ReporterActivityView::Disabled,
            FirstCompleteSetStatus::Waiting(WaitTiming::Unbounded { since }),
        ) if *since == disabled_since
    ));

    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .enable(reporter)?;
    app.update();

    let FirstCompleteSetStatus::Waiting(WaitTiming::Bounded {
        since: enabled_since,
        ..
    }) = reporter_health(&app)?.first_complete_set()
    else {
        return Err("re-enabling the reporter did not start a fresh bounded wait".into());
    };
    let enabled_since = *enabled_since;
    assert!(enabled_since.elapsed() > original_deadline.elapsed());
    assert!(enabled_since.elapsed() > disabled_since.elapsed());
    reporter_health_json(&app)?;

    Ok(())
}

#[test]
fn reenabled_optional_reporter_crosses_its_restarted_first_complete_set_bound()
-> Result<(), Box<dyn Error>> {
    let first_complete_set_bound = Duration::from_secs(10);
    let frame_past_bound = first_complete_set_bound + Duration::from_secs(1);
    let mut app = observing_app()?;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Deferred(ReporterDeferral::WaitingForTopology)]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            first_complete_set_bound,
        ),
    );

    advance_until_accepted(&mut app, reporter)?;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(frame_past_bound));
    app.update();
    let FirstCompleteSetStatus::Waiting(WaitTiming::Overdue {
        deadline: first_deadline,
        ..
    }) = reporter_health(&app)?.first_complete_set()
    else {
        return Err("the optional reporter did not cross its first bound".into());
    };
    let first_deadline = *first_deadline;

    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .disable(reporter)?;
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.update();
    assert!(matches!(
        reporter_health(&app)?.first_complete_set(),
        FirstCompleteSetStatus::Waiting(WaitTiming::Unbounded { .. })
    ));

    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .enable(reporter)?;
    app.update();
    let FirstCompleteSetStatus::Waiting(WaitTiming::Bounded {
        deadline: restarted_deadline,
        ..
    }) = reporter_health(&app)?.first_complete_set()
    else {
        return Err("re-enabling the optional reporter did not arm a new bound".into());
    };
    let restarted_deadline = *restarted_deadline;
    assert!(restarted_deadline.elapsed() > first_deadline.elapsed());

    app.insert_resource(TimeUpdateStrategy::ManualDuration(frame_past_bound));
    app.update();
    assert!(matches!(
        reporter_health(&app)?.first_complete_set(),
        FirstCompleteSetStatus::Waiting(WaitTiming::Overdue { deadline, .. })
            if *deadline == restarted_deadline
    ));

    Ok(())
}

#[test]
fn unsupported_reporter_health_stays_stopped_until_requested() -> Result<(), Box<dyn Error>> {
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Unsupported {
            detail: UNSUPPORTED_DISCOVERY_DETAIL.to_owned(),
        }]),
        ReporterRegistration::optional(
            DiscoveryCadence::Periodic {
                interval: Duration::ZERO,
            },
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );

    advance_until_accepted(&mut app, reporter)?;
    let health = reporter_health(&app)?;
    assert!(matches!(
        health.activity(),
        ReporterActivityView::Stopped {
            resumes_when: ReporterResume::ExplicitRequestOrEnable,
            ..
        }
    ));
    assert!(matches!(
        health.outcome(),
        ReporterOutcomeHealth::Unsupported {
            error: DeviceAccessErrorView::Unsupported { .. },
            ..
        }
    ));

    for _ in 0..3 {
        app.update();
        assert!(matches!(
            reporter_health(&app)?.activity(),
            ReporterActivityView::Stopped { .. }
        ));
    }

    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .request(reporter)?;
    app.update();
    assert!(matches!(
        reporter_health(&app)?.activity(),
        ReporterActivityView::Queued { .. }
    ));
    advance_until_accepted(&mut app, reporter)?;
    assert!(matches!(
        reporter_health(&app)?.activity(),
        ReporterActivityView::Stopped { .. }
    ));
    reporter_health_json(&app)?;

    Ok(())
}

#[test]
fn deferred_reporter_health_retains_records_and_deferral_start() -> Result<(), Box<dyn Error>> {
    let key = panel_key("health-panel")?;
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([
            scan![ScriptedDevice::present(key.clone())],
            ScriptedScan::Deferred(ReporterDeferral::WaitingForTopology),
            ScriptedScan::Deferred(ReporterDeferral::WaitingForTopology),
            ScriptedScan::Deferred(ReporterDeferral::WaitingForTopology),
            scan![ScriptedDevice::present(key)],
        ]),
        ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            panel_coverage()?,
            Duration::ZERO,
        ),
    );

    app.update();
    assert!(matches!(
        reporter_health(&app)?.first_complete_set(),
        FirstCompleteSetStatus::Waiting(WaitTiming::Overdue { .. })
    ));
    advance_until_accepted(&mut app, reporter)?;
    assert_eq!(*reporter_health(&app)?.retained_records(), 1);

    let mut first_since = None;
    for _ in 0..3 {
        advance_reporter(&mut app, reporter)?;
        let health = reporter_health(&app)?;
        assert_eq!(*health.retained_records(), 1);
        let ReporterOutcomeHealth::Deferred { since, deferral } = health.outcome() else {
            return Err("the deferred discovery outcome was not published".into());
        };
        let since = *since;
        assert_eq!(*deferral, ReporterDeferral::WaitingForTopology);
        match first_since {
            None => first_since = Some(since),
            Some(expected) => assert_eq!(since, expected),
        }
    }

    advance_reporter(&mut app, reporter)?;
    assert!(matches!(
        reporter_health(&app)?.outcome(),
        ReporterOutcomeHealth::Succeeded { records: 1, .. }
    ));
    reporter_health_json(&app)?;

    Ok(())
}

#[test]
fn a_redeclared_identical_capability_leaves_the_component_change_tick_untouched()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("capability-tick")?;
    let device = ScriptedDevice::present(key.clone())
        .with_capabilities(|| Capabilities::new().with(ScanRateCapability(60)));
    let (mut app, reporter) = scripted_app(vec![
        ScriptedScan::Complete(vec![device.clone()]),
        ScriptedScan::Complete(vec![device]),
    ])?;

    advance_reporter(&mut app, reporter)?;
    let entity = device_entity(&app, &key)?;
    assert_eq!(
        app.world().get::<ScanRateCapability>(entity),
        Some(&ScanRateCapability(60))
    );
    let first_change = component_change_tick::<ScanRateCapability>(&app, entity)?;

    advance_reporter(&mut app, reporter)?;

    assert_eq!(
        component_change_tick::<ScanRateCapability>(&app, entity)?,
        first_change
    );

    Ok(())
}

#[test]
fn an_omitted_capability_is_removed_from_the_device_entity() -> Result<(), Box<dyn Error>> {
    let key = panel_key("omitted-capability")?;
    let (mut app, reporter) = scripted_app(vec![
        scan![
            ScriptedDevice::present(key.clone())
                .with_capabilities(|| Capabilities::new().with(ScanRateCapability(60)))
        ],
        scan![ScriptedDevice::present(key.clone())],
    ])?;

    advance_reporter(&mut app, reporter)?;
    let entity = device_entity(&app, &key)?;
    assert_eq!(
        app.world().get::<ScanRateCapability>(entity),
        Some(&ScanRateCapability(60))
    );

    advance_reporter(&mut app, reporter)?;

    assert_eq!(app.world().get::<ScanRateCapability>(entity), None);

    Ok(())
}

#[test]
fn a_retraction_with_missing_registration_reaches_reporter_health() -> Result<(), Box<dyn Error>> {
    let key = panel_key("unregistered-capability-retraction")?;
    let (mut app, reporter) = scripted_app(vec![
        scan![
            ScriptedDevice::present(key.clone())
                .with_capabilities(|| Capabilities::new().with(ScanRateCapability(60)))
        ],
        scan![ScriptedDevice::present(key.clone())],
    ])?;

    advance_reporter(&mut app, reporter)?;
    let entity = device_entity(&app, &key)?;
    assert_eq!(
        app.world().get::<ScanRateCapability>(entity),
        Some(&ScanRateCapability(60))
    );

    let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
    let retained_registrations = {
        let type_registry = app_type_registry.read();
        type_registry
            .iter()
            .filter(|registration| registration.type_id() != TypeId::of::<ScanRateCapability>())
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut type_registry_without_capability = TypeRegistry::empty();
    for registration in retained_registrations {
        type_registry_without_capability.add_registration(registration);
    }
    *app_type_registry.write() = type_registry_without_capability;

    advance_reporter(&mut app, reporter)?;

    assert_eq!(
        app.world().get::<ScanRateCapability>(entity),
        Some(&ScanRateCapability(60)),
        "a failed all-lookups pass must leave the projected component untouched"
    );
    let ReporterOutcomeHealth::Succeeded {
        capability_projection: CapabilityProjectionStatus::Failed(failures),
        ..
    } = reporter_health(&app)?.outcome()
    else {
        return Err("the retraction failure did not reach reporter health".into());
    };
    assert_eq!(
        failures.entries(),
        [CapabilityProjectionFailure::ReflectComponentNotRegistered {
            type_path: ScanRateCapability::type_path().to_owned(),
        }]
    );

    Ok(())
}

#[test]
fn a_uniquely_resolved_capability_is_removed_when_its_handle_becomes_ambiguous()
-> Result<(), Box<dyn Error>> {
    let first = panel_key("first-newly-ambiguous-capability")?;
    let second = panel_key("second-newly-ambiguous-capability")?;
    let handle = ReportedId::new("newly-ambiguous-platform-handle")?;
    let (mut app, reporter) = scripted_app(vec![
        scan![
            ScriptedDevice::present(first.clone())
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone())),
            ScriptedDevice::match_evidence_only(Presence::Present)
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone()))
                .with_capabilities(|| Capabilities::new().with(EvidenceScreenCapability)),
        ],
        scan![
            ScriptedDevice::present(first.clone())
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone())),
            ScriptedDevice::present(second)
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone())),
            ScriptedDevice::match_evidence_only(Presence::Present)
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle))
                .with_capabilities(|| Capabilities::new().with(EvidenceScreenCapability)),
        ],
    ])?;

    advance_reporter(&mut app, reporter)?;
    let entity = device_entity(&app, &first)?;
    assert_eq!(
        app.world().get::<EvidenceScreenCapability>(entity),
        Some(&EvidenceScreenCapability)
    );

    advance_reporter(&mut app, reporter)?;

    assert_eq!(app.world().get::<EvidenceScreenCapability>(entity), None);

    Ok(())
}

#[test]
fn keyed_and_evidence_only_capabilities_attach_to_the_same_device() -> Result<(), Box<dyn Error>> {
    let key = panel_key("joined-capabilities")?;
    let handle = ReportedId::new("joined-platform-handle")?;
    let mut app = observing_app()?;
    let keyed_reporter = add_matching_scripted_reporter(
        &mut app,
        vec![scan![
            ScriptedDevice::present(key.clone())
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone()))
                .with_capabilities(|| Capabilities::new().with(KeyedDisplayCapability))
        ]],
    );
    let evidence_reporter = add_matching_scripted_reporter(
        &mut app,
        vec![scan![
            ScriptedDevice::match_evidence_only(Presence::Present)
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle))
                .with_capabilities(|| Capabilities::new().with(EvidenceScreenCapability))
        ]],
    );

    advance_reporter(&mut app, keyed_reporter)?;
    advance_reporter(&mut app, evidence_reporter)?;

    let entity = device_entity(&app, &key)?;
    assert_eq!(
        app.world().get::<KeyedDisplayCapability>(entity),
        Some(&KeyedDisplayCapability)
    );
    assert_eq!(
        app.world().get::<EvidenceScreenCapability>(entity),
        Some(&EvidenceScreenCapability)
    );

    Ok(())
}

#[test]
fn an_evidence_only_capability_with_an_ambiguous_handle_attaches_nothing()
-> Result<(), Box<dyn Error>> {
    let first = panel_key("first-ambiguous-capability")?;
    let second = panel_key("second-ambiguous-capability")?;
    let handle = ReportedId::new("ambiguous-platform-handle")?;
    let (mut app, reporter) = scripted_app(vec![scan![
        ScriptedDevice::present(first.clone())
            .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone())),
        ScriptedDevice::present(second.clone())
            .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone())),
        ScriptedDevice::match_evidence_only(Presence::Present)
            .with_platform_device_handle(PlatformDeviceHandle::Reported(handle))
            .with_capabilities(|| Capabilities::new().with(EvidenceScreenCapability)),
    ]])?;

    advance_reporter(&mut app, reporter)?;

    assert_eq!(
        app.world()
            .get::<EvidenceScreenCapability>(device_entity(&app, &first)?),
        None
    );
    assert_eq!(
        app.world()
            .get::<EvidenceScreenCapability>(device_entity(&app, &second)?),
        None
    );

    Ok(())
}

#[test]
fn projection_failures_are_named_sorted_deduplicated_and_stable() -> Result<(), Box<dyn Error>> {
    let first = ScriptedDevice::present(panel_key("missing-registration-first")?)
        .with_capabilities(|| Capabilities::new().with(ZetaRegistrationMissing));
    let second = ScriptedDevice::present(panel_key("missing-registration-second")?)
        .with_capabilities(|| {
            Capabilities::new()
                .with(ZetaRegistrationMissing)
                .with(AlphaRegistrationMissing)
        });
    let scan = vec![first, second];
    let (mut app, reporter) = scripted_app(vec![
        ScriptedScan::Complete(scan.clone()),
        ScriptedScan::Complete(scan),
    ])?;

    advance_reporter(&mut app, reporter)?;
    let expected = vec![
        CapabilityProjectionFailure::ReflectComponentNotRegistered {
            type_path: AlphaRegistrationMissing::type_path().to_owned(),
        },
        CapabilityProjectionFailure::ReflectComponentNotRegistered {
            type_path: ZetaRegistrationMissing::type_path().to_owned(),
        },
    ];
    let ReporterOutcomeHealth::Succeeded {
        capability_projection: CapabilityProjectionStatus::Failed(failures),
        ..
    } = reporter_health(&app)?.outcome()
    else {
        return Err("the successful scan did not report its capability projection failures".into());
    };
    assert_eq!(failures.entries(), expected);

    advance_reporter(&mut app, reporter)?;
    let ReporterOutcomeHealth::Succeeded {
        capability_projection: CapabilityProjectionStatus::Failed(failures),
        ..
    } = reporter_health(&app)?.outcome()
    else {
        return Err("the repeated scan did not retain its capability projection failures".into());
    };
    assert_eq!(failures.entries(), expected);
    let settled_change = reporter_health_change_tick(&app)?;
    app.update();
    assert_eq!(reporter_health_change_tick(&app)?, settled_change);

    Ok(())
}

#[test]
fn missing_capability_component_registration_reaches_the_kernel_status()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("missing-capability-role")?;
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([scan![
            ScriptedDevice::present(key.clone())
                .with_capabilities(|| Capabilities::new().with(AlphaRegistrationMissing))
        ]]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("missing-capability-role")?;
    register_capability_role::<AlphaRegistrationMissing>(&mut app, reporter, &role, key)?;

    advance_reporter(&mut app, reporter)?;
    app.update();

    let expected = CapabilityProjectionFailure::ReflectComponentNotRegistered {
        type_path: AlphaRegistrationMissing::type_path().to_owned(),
    };
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Waiting(
            WaitingStatusView::ApplicationCapabilityRegistrationRequired { failure, .. }
        ) if failure == &expected
    ));
    Ok(())
}

#[test]
fn custom_capability_type_path_still_correlates_its_registration_failure()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("custom-capability-path")?;
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([scan![
            ScriptedDevice::present(key.clone())
                .with_capabilities(|| Capabilities::new().with(CustomPathRegistrationMissing))
        ]]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("custom-capability-path")?;
    register_capability_role::<CustomPathRegistrationMissing>(&mut app, reporter, &role, key)?;

    advance_reporter(&mut app, reporter)?;
    app.update();

    let expected = CapabilityProjectionFailure::ReflectComponentNotRegistered {
        type_path: CustomPathRegistrationMissing::type_path().to_owned(),
    };
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Waiting(
            WaitingStatusView::ApplicationCapabilityRegistrationRequired { failure, .. }
        ) if failure == &expected
    ));
    Ok(())
}

#[test]
fn ordinary_capability_absence_waits_for_the_reporter() -> Result<(), Box<dyn Error>> {
    let key = panel_key("ordinary-capability-absence")?;
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([scan![ScriptedDevice::present(key.clone())]]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("ordinary-capability-absence")?;
    register_capability_role::<ScanRateCapability>(&mut app, reporter, &role, key)?;

    advance_reporter(&mut app, reporter)?;
    app.update();

    assert!(matches!(
        reporter_health(&app)?.outcome(),
        ReporterOutcomeHealth::Succeeded {
            capability_projection: CapabilityProjectionStatus::AllProjected,
            ..
        }
    ));
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Waiting(WaitingStatusView::Reporter(_))
    ));
    Ok(())
}

#[test]
fn ordinary_reporter_failure_retains_a_successful_capability_for_reapply()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("retained-capability-after-reporter-failure")?;
    let declared = || Capabilities::new().with(ScanRateCapability(60));
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([
            scan![ScriptedDevice::present(key.clone()).with_capabilities(declared)],
            ScriptedScan::Failed(DeviceAccessError::Transport {
                detail: DISCOVERY_FAILURE_DETAIL.to_owned(),
            }),
        ]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("retained-capability-after-reporter-failure")?;
    register_capability_role::<ScanRateCapability>(&mut app, reporter, &role, key)?;

    advance_reporter(&mut app, reporter)?;
    app.update();
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Established { .. }
    ));

    advance_reporter(&mut app, reporter)?;
    assert!(matches!(
        reporter_health(&app)?.outcome(),
        ReporterOutcomeHealth::Failing {
            run: FailureRunStatus {
                previous_success: PreviousSuccess::At {
                    capability_projection: CapabilityProjectionStatus::AllProjected,
                    ..
                },
                ..
            },
            ..
        }
    ));
    app.world_mut()
        .resource_mut::<DriverCalls>()
        .sessions
        .remove(&role)
        .ok_or("the established capability driver retained no lease")?
        .report_loss(DeviceAccessError::Absent {
            detail: "the scripted capability session ended".to_owned(),
        });
    for _ in 0..4 {
        app.update();
    }

    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Established { .. }
    ));
    Ok(())
}

#[test]
fn a_projection_failure_refuses_a_stale_capability_component() -> Result<(), Box<dyn Error>> {
    let key = panel_key("stale-capability-role")?;
    let declared = || Capabilities::new().with(ScanRateCapability(60));
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([
            scan![ScriptedDevice::present(key.clone()).with_capabilities(declared)],
            scan![ScriptedDevice::present(key.clone()).with_capabilities(declared)],
        ]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("stale-capability-role")?;
    register_capability_role::<ScanRateCapability>(&mut app, reporter, &role, key.clone())?;

    advance_reporter(&mut app, reporter)?;
    app.update();
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Established { .. }
    ));
    let entity = device_entity(&app, &key)?;
    assert_eq!(
        app.world().get::<ScanRateCapability>(entity),
        Some(&ScanRateCapability(60))
    );

    app.world()
        .resource::<AppTypeRegistry>()
        .write()
        .overwrite_registration(TypeRegistration::of::<ScanRateCapability>());
    advance_reporter(&mut app, reporter)?;
    assert_eq!(
        app.world().get::<ScanRateCapability>(entity),
        Some(&ScanRateCapability(60))
    );
    app.world_mut()
        .resource_mut::<DriverCalls>()
        .sessions
        .remove(&role)
        .ok_or("the established capability driver retained no lease")?
        .report_loss(DeviceAccessError::Absent {
            detail: "the scripted capability session ended".to_owned(),
        });
    app.update();
    app.update();

    let expected = CapabilityProjectionFailure::ReflectComponentNotRegistered {
        type_path: ScanRateCapability::type_path().to_owned(),
    };
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Waiting(
            WaitingStatusView::ApplicationCapabilityRegistrationRequired { failure, .. }
        ) if failure == &expected
    ));
    Ok(())
}

#[test]
fn a_missing_application_type_registry_is_named_by_reporter_and_role() -> Result<(), Box<dyn Error>>
{
    let key = panel_key("missing-application-type-registry")?;
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([scan![
            ScriptedDevice::present(key.clone())
                .with_capabilities(|| Capabilities::new().with(ScanRateCapability(60)))
        ]]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("missing-application-type-registry")?;
    register_capability_role::<ScanRateCapability>(&mut app, reporter, &role, key)?;
    app.world_mut().remove_resource::<AppTypeRegistry>();

    advance_reporter(&mut app, reporter)?;
    app.update();

    let expected = CapabilityProjectionFailure::ApplicationTypeRegistryUnavailable {
        affected_type_path: ScanRateCapability::type_path().to_owned(),
    };
    let ReporterOutcomeHealth::Succeeded {
        capability_projection: CapabilityProjectionStatus::Failed(failures),
        ..
    } = reporter_health(&app)?.outcome()
    else {
        return Err("the missing application type registry did not reach reporter health".into());
    };
    assert_eq!(failures.entries(), std::slice::from_ref(&expected));
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Waiting(
            WaitingStatusView::ApplicationCapabilityRegistrationRequired { failure, .. }
        ) if failure == &expected
    ));
    Ok(())
}

#[test]
fn failure_after_deferral_starts_a_new_consecutive_run() -> Result<(), Box<dyn Error>> {
    let repeated_failure = ScriptedScan::Failed(DeviceAccessError::Transport {
        detail: DISCOVERY_FAILURE_DETAIL.to_owned(),
    });
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([
            repeated_failure.clone(),
            ScriptedScan::Deferred(ReporterDeferral::WaitingForTopology),
            repeated_failure,
        ]),
        ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );

    advance_until_accepted(&mut app, reporter)?;
    assert!(matches!(
        reporter_health(&app)?.outcome(),
        ReporterOutcomeHealth::Failing { run, .. } if run.consecutive == NonZeroU32::MIN
    ));

    advance_reporter(&mut app, reporter)?;
    assert!(matches!(
        reporter_health(&app)?.outcome(),
        ReporterOutcomeHealth::Deferred {
            deferral: ReporterDeferral::WaitingForTopology,
            ..
        }
    ));

    advance_reporter(&mut app, reporter)?;
    assert!(matches!(
        reporter_health(&app)?.outcome(),
        ReporterOutcomeHealth::Failing { run, .. } if run.consecutive == NonZeroU32::MIN
    ));

    Ok(())
}

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

/// An unsupported reporter must remain stopped through cadence deadlines and run only after an
/// explicit request.
#[test]
fn unsupported_discovery_stops_automatic_cadence_until_requested() -> Result<(), Box<dyn Error>> {
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Unsupported {
            detail: UNSUPPORTED_DISCOVERY_DETAIL.to_owned(),
        }]),
        ReporterRegistration::optional(
            DiscoveryCadence::Periodic {
                interval: Duration::ZERO,
            },
            ReporterActivation::Enabled,
            panel_coverage()?,
            std::time::Duration::from_secs(10),
        ),
    );

    advance_until_accepted(&mut app, reporter)?;
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(reporter_health_for(&app, reporter)?.completed_runs(), 1);

    advance_reporter(&mut app, reporter)?;
    let reporter_status = reporter_health_for(&app, reporter)?;
    assert_eq!(reporter_status.completed_runs(), 2);
    assert!(matches!(
        reporter_status.outcome(),
        ReporterOutcomeHealth::Unsupported {
            error: DeviceAccessErrorView::Unsupported { .. },
            ..
        }
    ));

    Ok(())
}

/// Deferral keeps the preceding whole set and its first `since`, records deferred completions, and
/// closes required startup until a complete set arrives.
#[test]
fn deferred_discovery_retains_state_and_since_until_complete() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let (mut app, reporter) = required_scripted_app(vec![
        scan![ScriptedDevice::present(key.clone())],
        ScriptedScan::Deferred(ReporterDeferral::WaitingForTopology),
        ScriptedScan::Deferred(ReporterDeferral::WaitingForTopology),
        ScriptedScan::Deferred(ReporterDeferral::WaitingForTopology),
        scan![ScriptedDevice::present(key.clone())],
    ])?;

    advance_reporter(&mut app, reporter)?;
    app.world_mut()
        .resource_mut::<ObservedEvents>()
        .discovery_finished
        .clear();

    advance_reporter(&mut app, reporter)?;
    let ReporterOutcomeHealth::Deferred {
        since: first_since, ..
    } = reporter_health_for(&app, reporter)?.outcome()
    else {
        return Err("the first deferred scan did not retain a deferred outcome".into());
    };
    let first_since = *first_since;

    for _ in 0..2 {
        advance_reporter(&mut app, reporter)?;
        assert!(matches!(
            reporter_health_for(&app, reporter)?.outcome(),
            ReporterOutcomeHealth::Deferred { since, .. } if *since == first_since
        ));
    }

    let health = reporter_health_for(&app, reporter)?;
    assert_eq!(health.completed_runs(), 4);
    assert!(matches!(
        health.first_complete_set(),
        FirstCompleteSetStatus::Completed { .. }
    ));
    let discovery_finished = &app.world().resource::<ObservedEvents>().discovery_finished;
    assert_eq!(discovery_finished.len(), 3);
    assert!(discovery_finished.iter().all(|(_, finished_by, outcome)| {
        *finished_by == reporter
            && matches!(
                outcome,
                CompletedDiscoveryOutcome::Deferred {
                    deferral: ReporterDeferral::WaitingForTopology,
                    ..
                }
            )
    }));
    assert!(matches!(
        app.world().resource::<Devices>().resolve(&key),
        DeviceResolution::Resolved(_)
    ));

    advance_reporter(&mut app, reporter)?;
    assert!(matches!(
        reporter_health_for(&app, reporter)?.first_complete_set(),
        FirstCompleteSetStatus::Completed { .. }
    ));

    Ok(())
}

/// One stopped reporter must not prevent a different reporter from continuing its own cadence.
#[test]
fn supported_reporters_continue_while_another_is_unsupported() -> Result<(), Box<dyn Error>> {
    let mut app = observing_app()?;
    let unsupported = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Unsupported {
            detail: UNSUPPORTED_DISCOVERY_DETAIL.to_owned(),
        }]),
        ReporterRegistration::optional(
            DiscoveryCadence::Periodic {
                interval: Duration::ZERO,
            },
            ReporterActivation::Enabled,
            panel_coverage()?,
            std::time::Duration::from_secs(10),
        ),
    );
    let supported = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Complete(Vec::new())]),
        ReporterRegistration::optional(
            DiscoveryCadence::Periodic {
                interval: Duration::ZERO,
            },
            ReporterActivation::Enabled,
            panel_coverage()?,
            std::time::Duration::from_secs(10),
        ),
    );

    for _ in 0..8 {
        app.update();
    }

    assert_eq!(reporter_health_for(&app, unsupported)?.completed_runs(), 1);
    assert!(reporter_health_for(&app, supported)?.completed_runs() > 1);

    Ok(())
}

/// An authored device the application marked offline must still report its connectivity, and no
/// driver may be touched for it.
///
/// The two halves are the whole point of `ConfiguredDeviceMode::Offline`: a user interface has to
/// be able to show a configured-but-disabled fixture as plugged in, and the kernel has to dispatch
/// no driver operation for it while it is offline.
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
            name: ConfiguredDeviceName::NeverDerived,
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
    assert!(observed.attempt_endings.is_empty());
    assert!(app.world().resource::<DriverCalls>().started.is_empty());

    Ok(())
}

fn first_authoritative_empty_wait(scan: ScriptedScan) -> Result<HardwareWait, Box<dyn Error>> {
    let key = panel_key("first-empty")?;
    let (mut app, reporter) = scripted_app(vec![scan])?;
    let role = RoleKey::new("first-empty")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, reporter)?;
    app.update();

    role_hardware_wait(&app, &role)
}

#[test]
fn a_first_authoritative_empty_complete_establishes_absence() -> Result<(), Box<dyn Error>> {
    assert!(matches!(
        first_authoritative_empty_wait(ScriptedScan::Complete(Vec::new()))?,
        HardwareWait::Absent { .. }
    ));
    Ok(())
}

#[test]
fn a_first_authoritative_empty_complete_with_projection_establishes_absence()
-> Result<(), Box<dyn Error>> {
    assert!(matches!(
        first_authoritative_empty_wait(ScriptedScan::CompleteWithProjection(Vec::new()))?,
        HardwareWait::Absent { .. }
    ));
    Ok(())
}

#[test]
fn a_failure_before_first_success_keeps_the_role_awaiting_its_first_report()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("failure-before-success")?;
    let (mut app, reporter) =
        scripted_app(vec![ScriptedScan::Failed(DeviceAccessError::Transport {
            detail: String::from("enumeration unavailable"),
        })])?;
    let role = RoleKey::new("failure-before-success")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, reporter)?;
    app.update();

    assert!(matches!(
        role_hardware_wait(&app, &role)?,
        HardwareWait::AwaitingFirstReport { .. }
    ));
    assert!(app.world().resource::<DriverCalls>().started.is_empty());
    Ok(())
}

#[test]
fn zero_fresh_evidence_and_uncovered_absence_publish_different_bases() -> Result<(), Box<dyn Error>>
{
    let key = panel_key("unconfirmed-basis")?;
    let mut app = observing_app()?;
    let reporter = add_matching_scripted_reporter(
        &mut app,
        vec![
            ScriptedScan::Complete(vec![ScriptedDevice::absent(key.clone())]),
            ScriptedScan::Complete(Vec::new()),
        ],
    );
    let role = RoleKey::new("unconfirmed-basis")?;
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
    assert!(matches!(
        role_hardware_wait(&app, &role)?,
        HardwareWait::Unconfirmed {
            basis: UnconfirmedBasis::UncoveredAbsence { .. },
            ..
        }
    ));

    advance_reporter(&mut app, reporter)?;
    app.update();
    let entity = device_entity(&app, &key)?;
    assert!(matches!(
        app.world()
            .get::<DeviceStatus>(entity)
            .ok_or("the unconfirmed device has no status")?
            .availability(),
        KeyAvailability::Unconfirmed {
            basis: UnconfirmedBasis::NoFreshEvidence,
            ..
        }
    ));
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Waiting(
            WaitingStatusView::ApplicationReporterRegistrationRequired {
                key: uncovered,
                ..
            }
        ) if uncovered == &key
    ));
    Ok(())
}

#[test]
fn no_fresh_evidence_wait_names_only_reporters_covering_the_key() -> Result<(), Box<dyn Error>> {
    let key = panel_key("covered-unconfirmed-wait")?;
    let mut app = observing_app()?;
    let noncovering = add_matching_scripted_reporter(
        &mut app,
        vec![
            ScriptedScan::Complete(vec![ScriptedDevice::absent(key.clone())]),
            ScriptedScan::Complete(Vec::new()),
        ],
    );
    let role = RoleKey::new("covered-unconfirmed-wait")?;
    register_role(
        &mut app,
        &role,
        key.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, noncovering)?;
    app.update();
    advance_reporter(&mut app, noncovering)?;
    app.update();
    assert!(matches!(
        app.world()
            .get::<DeviceStatus>(device_entity(&app, &key)?)
            .ok_or("the retained unconfirmed device has no status")?
            .availability(),
        KeyAvailability::Unconfirmed {
            basis: UnconfirmedBasis::NoFreshEvidence,
            ..
        }
    ));
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Waiting(
            WaitingStatusView::ApplicationReporterRegistrationRequired {
                key: uncovered,
                ..
            }
        ) if uncovered == &key
    ));

    let reporter_refs_before = app
        .world()
        .iter_entities()
        .filter_map(|entity| entity.get::<ReporterHealth>())
        .map(|health| health.identity().reporter_ref)
        .collect::<Vec<_>>();
    app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Complete(Vec::new())]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Disabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let covering_reporter = app
        .world()
        .iter_entities()
        .filter_map(|entity| entity.get::<ReporterHealth>())
        .map(|health| health.identity().reporter_ref)
        .find(|reporter| !reporter_refs_before.contains(reporter))
        .ok_or("the covering reporter has no diagnostic reference")?;

    app.update();
    assert!(matches!(
        role_hardware_wait(&app, &role)?,
        HardwareWait::Unconfirmed {
            basis: UnconfirmedBasis::NoFreshEvidence,
            reporters,
            ..
        } if reporters.as_slice() == [covering_reporter]
    ));
    Ok(())
}

#[test]
fn an_uncovering_empty_complete_does_not_hide_a_covered_present_key() -> Result<(), Box<dyn Error>>
{
    let key = panel_key("uncovering-empty")?;
    let mut app = observing_app()?;
    let covered = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Complete(vec![ScriptedDevice::present(
            key.clone(),
        )])]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let uncovering =
        add_matching_scripted_reporter(&mut app, vec![ScriptedScan::Complete(Vec::new())]);
    let role = RoleKey::new("uncovering-empty")?;
    register_role(
        &mut app,
        &role,
        key.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, covered)?;
    app.update();
    app.world_mut()
        .resource_mut::<ObservedEvents>()
        .device_changes
        .clear();
    advance_reporter(&mut app, uncovering)?;
    app.update();

    let entity = device_entity(&app, &key)?;
    assert!(matches!(
        app.world()
            .get::<DeviceStatus>(entity)
            .ok_or("the covered device has no status")?
            .availability(),
        KeyAvailability::Present(_)
    ));
    assert!(
        app.world()
            .resource::<ObservedEvents>()
            .device_changes
            .is_empty()
    );
    Ok(())
}

#[test]
fn a_keyed_unreachable_record_never_authorizes_service() -> Result<(), Box<dyn Error>> {
    let key = panel_key("keyed-unreachable")?;
    let (mut app, reporter) = scripted_app(vec![ScriptedScan::Complete(vec![
        ScriptedDevice::unreachable(key.clone(), Duration::from_secs(2)),
    ])])?;
    let role = RoleKey::new("keyed-unreachable")?;
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

    let entity = device_entity(&app, &key)?;
    assert!(matches!(
        app.world()
            .get::<DeviceStatus>(entity)
            .ok_or("the unreachable device has no status")?
            .availability(),
        KeyAvailability::Unreachable { .. }
    ));
    assert!(matches!(
        role_hardware_wait(&app, &role)?,
        HardwareWait::Unreachable { .. }
    ));
    assert!(app.world().resource::<DriverCalls>().started.is_empty());
    Ok(())
}

#[test]
fn a_mixed_contributor_set_uses_its_combined_presence_and_names_its_reporter()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("mixed-presence")?;
    let mut app = observing_app()?;
    let present_reporter = add_matching_scripted_reporter(
        &mut app,
        vec![ScriptedScan::Complete(vec![ScriptedDevice::present(
            key.clone(),
        )])],
    );
    let reporter_refs_before = app
        .world()
        .iter_entities()
        .filter_map(|entity| entity.get::<ReporterHealth>())
        .map(|health| health.identity().reporter_ref)
        .collect::<Vec<_>>();
    let absence_reporter = app.add_device_reporter(
        ScriptedReporter::new([ScriptedScan::Complete(vec![ScriptedDevice::absent(
            key.clone(),
        )])]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let absence_reporter_ref = app
        .world()
        .iter_entities()
        .filter_map(|entity| entity.get::<ReporterHealth>())
        .map(|health| health.identity().reporter_ref)
        .find(|reporter| !reporter_refs_before.contains(reporter))
        .ok_or("the absence reporter has no published identity")?;
    let role = RoleKey::new("mixed-presence")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, absence_reporter)?;
    let (absence_batch, finished_reporter, _) = app
        .world()
        .resource::<ObservedEvents>()
        .discovery_finished
        .last()
        .ok_or("the absence reporter announced no completed batch")?;
    assert_eq!(*finished_reporter, absence_reporter);
    let absence_batch = *absence_batch;

    advance_reporter(&mut app, present_reporter)?;

    let HardwareWait::Absent { established_by, .. } = role_hardware_wait(&app, &role)? else {
        return Err("mixed presence did not publish established absence".into());
    };
    assert_eq!(established_by.reporter, absence_reporter_ref);
    assert_eq!(established_by.batch.get(), absence_batch.get());
    assert!(app.world().resource::<DriverCalls>().started.is_empty());
    Ok(())
}

#[test]
fn repeated_identical_reports_advance_present_and_unconfirmed_batches() -> Result<(), Box<dyn Error>>
{
    let present_key = panel_key("repeated-present-batch")?;
    let unconfirmed_key = panel_key("repeated-unconfirmed-batch")?;
    let devices = vec![
        ScriptedDevice::present(present_key.clone()),
        ScriptedDevice::absent(unconfirmed_key.clone()),
    ];
    let mut app = observing_app()?;
    let reporter = add_matching_scripted_reporter(
        &mut app,
        vec![
            ScriptedScan::Complete(devices.clone()),
            ScriptedScan::Complete(devices),
        ],
    );

    advance_reporter(&mut app, reporter)?;
    let first_present = app
        .world()
        .get::<DeviceStatus>(device_entity(&app, &present_key)?)
        .ok_or("the first present report has no status")?;
    let KeyAvailability::Present(first_present_evidence) = first_present.availability() else {
        return Err("the first present report did not establish presence".into());
    };
    let first_present_batch = first_present_evidence.contributors().as_slice()[0].batch;
    let first_unconfirmed = app
        .world()
        .get::<DeviceStatus>(device_entity(&app, &unconfirmed_key)?)
        .ok_or("the first unconfirmed report has no status")?;
    let KeyAvailability::Unconfirmed {
        basis:
            UnconfirmedBasis::UncoveredAbsence {
                batch: first_unconfirmed_batch,
                ..
            },
        ..
    } = first_unconfirmed.availability()
    else {
        return Err("the first absent record did not establish uncovered absence".into());
    };
    let first_unconfirmed_batch = *first_unconfirmed_batch;

    advance_reporter(&mut app, reporter)?;
    let second_present = app
        .world()
        .get::<DeviceStatus>(device_entity(&app, &present_key)?)
        .ok_or("the second present report has no status")?;
    let KeyAvailability::Present(second_present_evidence) = second_present.availability() else {
        return Err("the second present report did not establish presence".into());
    };
    let second_present_batch = second_present_evidence.contributors().as_slice()[0].batch;
    let second_unconfirmed = app
        .world()
        .get::<DeviceStatus>(device_entity(&app, &unconfirmed_key)?)
        .ok_or("the second unconfirmed report has no status")?;
    let KeyAvailability::Unconfirmed {
        basis:
            UnconfirmedBasis::UncoveredAbsence {
                batch: second_unconfirmed_batch,
                ..
            },
        ..
    } = second_unconfirmed.availability()
    else {
        return Err("the second absent record did not establish uncovered absence".into());
    };

    assert!(second_present_batch.get() > first_present_batch.get());
    assert!(second_unconfirmed_batch.get() > first_unconfirmed_batch.get());
    Ok(())
}

#[test]
fn an_earlier_omission_becomes_decisive_when_the_last_sighting_drops() -> Result<(), Box<dyn Error>>
{
    let key = panel_key("last-sighting")?;
    let mut app = observing_app()?;
    let (omission_script, omission_gate) = ScriptedReporter::gated(
        [ScriptedScan::Complete(Vec::new())],
        DiscoveryProgress::Indeterminate,
    );
    let earlier_omission = app.add_device_reporter(
        omission_script,
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let (sighting_script, sighting_gate) = ScriptedReporter::gated(
        [
            ScriptedScan::Complete(vec![ScriptedDevice::present(key.clone())]),
            ScriptedScan::Complete(Vec::new()),
        ],
        DiscoveryProgress::Indeterminate,
    );
    let last_sighting = app.add_device_reporter(
        sighting_script,
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("last-sighting")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_until_running(&mut app, earlier_omission)?;
    omission_gate.release();
    advance_until_accepted(&mut app, earlier_omission)?;
    let HardwareWait::Absent {
        established_by: earlier_evidence,
        ..
    } = role_hardware_wait(&app, &role)?
    else {
        return Err("the first authoritative omission did not establish absence".into());
    };

    advance_until_running(&mut app, last_sighting)?;
    sighting_gate.release();
    advance_until_accepted(&mut app, last_sighting)?;
    assert!(matches!(
        app.world()
            .get::<DeviceStatus>(device_entity(&app, &panel_key("last-sighting")?)?)
            .ok_or("the sighted device has no status")?
            .availability(),
        KeyAvailability::Present(_)
    ));
    app.world_mut()
        .resource_mut::<ObservedEvents>()
        .device_changes
        .clear();

    advance_until_running(&mut app, last_sighting)?;
    sighting_gate.release();
    advance_until_accepted(&mut app, last_sighting)?;
    let transition = app
        .world()
        .resource::<ObservedEvents>()
        .device_changes
        .last()
        .ok_or("dropping the last sighting announced no availability edge")?;
    let KeyAvailability::DepartureGrace { evidence, .. } = &transition.to else {
        return Err("dropping the last sighting did not enter departure grace".into());
    };
    assert_eq!(*evidence, earlier_evidence);
    Ok(())
}

#[test]
fn an_exact_evidence_only_sighting_counts_as_a_present_contributor() -> Result<(), Box<dyn Error>> {
    let key = panel_key("evidence-only-sighting")?;
    let handle = ReportedId::new("shared-evidence-handle")?;
    let mut app = observing_app()?;
    let keyed = add_matching_scripted_reporter(
        &mut app,
        vec![ScriptedScan::Complete(vec![
            ScriptedDevice::present(key.clone())
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone())),
        ])],
    );
    let evidence_only = add_matching_scripted_reporter(
        &mut app,
        vec![ScriptedScan::Complete(vec![
            ScriptedDevice::match_evidence_only(Presence::Present)
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle)),
        ])],
    );
    let role = RoleKey::new("evidence-only-sighting")?;
    register_role(
        &mut app,
        &role,
        key.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut app, keyed)?;
    advance_reporter(&mut app, evidence_only)?;
    app.update();
    let status = app
        .world()
        .get::<DeviceStatus>(device_entity(&app, &key)?)
        .ok_or("the exactly joined device has no status")?;
    let KeyAvailability::Present(evidence) = status.availability() else {
        return Err("the exact evidence-only sighting did not establish presence".into());
    };
    assert_eq!(evidence.contributors().as_slice().len(), 2);

    Ok(())
}

#[test]
fn evidence_only_presence_loses_ownership_when_the_keyed_record_expires()
-> Result<(), Box<dyn Error>> {
    let evidence_only_key = panel_key("only-evidence-sighting")?;
    let evidence_only_handle = ReportedId::new("only-evidence-handle")?;
    let mut evidence_only_app = observing_app()?;
    evidence_only_app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    evidence_only_app
        .world_mut()
        .resource_mut::<RiggingLimits>()
        .report_grace = Duration::ZERO;
    let (keyed_script, keyed_gate) = ScriptedReporter::gated(
        [
            ScriptedScan::Complete(vec![
                ScriptedDevice::present(evidence_only_key.clone()).with_platform_device_handle(
                    PlatformDeviceHandle::Reported(evidence_only_handle.clone()),
                ),
            ]),
            ScriptedScan::Complete(Vec::new()),
        ],
        DiscoveryProgress::Indeterminate,
    );
    let keyed_reporter = evidence_only_app.add_device_reporter(
        keyed_script,
        ReporterRegistration::optional(
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(1),
            },
            ReporterActivation::Enabled,
            ReporterCoverage::MatchingEvidenceOnly,
            Duration::from_secs(10),
        ),
    );
    let evidence_reporter = add_matching_scripted_reporter(
        &mut evidence_only_app,
        vec![ScriptedScan::Complete(vec![
            ScriptedDevice::match_evidence_only(Presence::Present)
                .with_platform_device_handle(PlatformDeviceHandle::Reported(evidence_only_handle)),
        ])],
    );
    let covering_reporter = evidence_only_app.add_device_reporter(
        ScriptedReporter::new([
            ScriptedScan::Complete(Vec::new()),
            ScriptedScan::Complete(Vec::new()),
        ]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );

    advance_until_running(&mut evidence_only_app, keyed_reporter)?;
    keyed_gate.release();
    advance_until_accepted(&mut evidence_only_app, keyed_reporter)?;
    advance_reporter(&mut evidence_only_app, evidence_reporter)?;
    advance_reporter(&mut evidence_only_app, covering_reporter)?;
    evidence_only_app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(2)));
    evidence_only_app.update();
    advance_reporter(&mut evidence_only_app, covering_reporter)?;

    let only_evidence_status = evidence_only_app
        .world()
        .get::<DeviceStatus>(device_entity(&evidence_only_app, &evidence_only_key)?)
        .ok_or("the evidence-only device has no status")?;
    let KeyAvailability::DepartureGrace { .. } = only_evidence_status.availability() else {
        return Err("the covering omission did not win after handle ownership expired".into());
    };

    Ok(())
}

#[test]
fn an_ambiguous_evidence_only_sighting_does_not_count() -> Result<(), Box<dyn Error>> {
    let handle = ReportedId::new("shared-evidence-handle")?;
    let first = panel_key("first-ambiguous-sighting")?;
    let second = panel_key("second-ambiguous-sighting")?;
    let mut ambiguous_app = observing_app()?;
    let ambiguous_keys = add_matching_scripted_reporter(
        &mut ambiguous_app,
        vec![ScriptedScan::Complete(vec![
            ScriptedDevice::present(first.clone())
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone())),
            ScriptedDevice::present(second)
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle.clone())),
        ])],
    );
    let ambiguous_evidence = add_matching_scripted_reporter(
        &mut ambiguous_app,
        vec![ScriptedScan::Complete(vec![
            ScriptedDevice::match_evidence_only(Presence::Absent)
                .with_platform_device_handle(PlatformDeviceHandle::Reported(handle)),
        ])],
    );
    let ambiguous_role = RoleKey::new("ambiguous-evidence-only-sighting")?;
    register_role(
        &mut ambiguous_app,
        &ambiguous_role,
        first.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_reporter(&mut ambiguous_app, ambiguous_keys)?;
    advance_reporter(&mut ambiguous_app, ambiguous_evidence)?;
    ambiguous_app.update();
    let ambiguous_status = ambiguous_app
        .world()
        .get::<DeviceStatus>(device_entity(&ambiguous_app, &first)?)
        .ok_or("the ambiguously joined device has no status")?;
    let KeyAvailability::Present(ambiguous_present) = ambiguous_status.availability() else {
        return Err("ambiguous evidence-only absence changed keyed presence".into());
    };
    assert_eq!(ambiguous_present.contributors().as_slice().len(), 1);
    Ok(())
}

#[test]
fn one_departure_evidence_is_identical_across_device_role_and_event() -> Result<(), Box<dyn Error>>
{
    let key = panel_key("shared-retirement-evidence")?;
    let (scripted_reporter, gate) = ScriptedReporter::gated(
        [
            ScriptedScan::Complete(vec![ScriptedDevice::present(key.clone())]),
            ScriptedScan::Complete(Vec::new()),
        ],
        DiscoveryProgress::Indeterminate,
    );
    let mut app = observing_app()?;
    let reporter = app.add_device_reporter(
        scripted_reporter,
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            panel_coverage()?,
            Duration::from_secs(10),
        ),
    );
    let role = RoleKey::new("shared-retirement-evidence")?;
    register_role(
        &mut app,
        &role,
        key.clone(),
        RecoveryPolicy::default(),
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    advance_until_running(&mut app, reporter)?;
    gate.release();
    advance_until_accepted(&mut app, reporter)?;
    app.world_mut()
        .resource_mut::<ObservedEvents>()
        .device_changes
        .clear();
    advance_until_running(&mut app, reporter)?;
    gate.release();
    advance_until_accepted(&mut app, reporter)?;

    let event_evidence = match &app
        .world()
        .resource::<ObservedEvents>()
        .device_changes
        .last()
        .ok_or("departure grace announced no availability edge")?
        .to
    {
        KeyAvailability::DepartureGrace { evidence, .. } => *evidence,
        _ => return Err("the availability edge did not enter departure grace".into()),
    };
    let device_evidence = match app
        .world()
        .get::<DeviceStatus>(device_entity(&app, &key)?)
        .ok_or("the device has no status during departure grace")?
        .availability()
    {
        KeyAvailability::DepartureGrace { evidence, .. } => *evidence,
        _ => return Err("DeviceStatus did not publish departure grace".into()),
    };
    let HardwareWait::DepartureGrace {
        evidence: role_evidence,
        ..
    } = role_hardware_wait(&app, &role)?
    else {
        return Err("the role did not publish departure grace".into());
    };

    assert_eq!(event_evidence, device_evidence);
    assert_eq!(device_evidence, role_evidence);
    Ok(())
}

/// An omitted covered key enters grace before the deadline retires its entity.
#[test]
fn a_covered_omission_retires_only_after_departure_grace() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let cycle = availability_cycle_after(&key, scan![])?;

    assert_eq!(cycle.entered_grace.key, key);
    assert!(matches!(
        cycle.entered_grace.from,
        KeyAvailability::Present(_)
    ));
    let KeyAvailability::DepartureGrace { evidence, .. } = cycle.entered_grace.to else {
        return Err("covered omission did not enter departure grace".into());
    };
    let KeyAvailability::Absent { established_by, .. } = cycle.retired.to else {
        return Err("grace deadline did not establish absence".into());
    };
    assert_eq!(evidence, established_by);
    assert_eq!(cycle.retained_during_grace, 1);
    assert_eq!(cycle.retained_after_retirement, 0);

    Ok(())
}

/// A keyed covered absence follows the same transition table as a covered omission.
#[test]
fn a_keyed_covered_absence_retires_after_departure_grace() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL15")?;
    let cycle = availability_cycle_after(&key, scan![ScriptedDevice::absent(key.clone())])?;

    assert_eq!(cycle.entered_grace.key, key);
    assert!(matches!(
        cycle.entered_grace.to,
        KeyAvailability::DepartureGrace { .. }
    ));
    assert!(matches!(cycle.retired.to, KeyAvailability::Absent { .. }));
    assert_eq!(cycle.retained_during_grace, 1);
    assert_eq!(cycle.retained_after_retirement, 0);

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
    let scans = vec![
        scan![ScriptedDevice::present(key.clone())],
        // Two more present scans, so the arrival's apply settles and the safe-capture pass then
        // gets a frame with the role `Ready` and its unit present — the only conditions under
        // which a value is established. Frames cannot be spent here to the same end: a
        // bare `app.update()` advances the script, so idling would consume the departure
        // rather than wait for it.
        scan![ScriptedDevice::present(key.clone())],
        scan![ScriptedDevice::present(key.clone())],
        scan![ScriptedDevice::absent(key.clone())],
        scan![ScriptedDevice::present(key.clone())],
    ];
    let scripted_scans = scans.len();
    let (mut app, reporter) = scripted_app(scans)?;
    let role = RoleKey::new("panel")?;
    register_role(
        &mut app,
        &role,
        key,
        RecoveryPolicy::ReapplyOnReturn,
        RetryOn::NewRevision,
        ApplyDeadline::ProcessDefault,
    )?;

    // One call per scripted scan: `advance_reporter` returns as soon as its own batch lands, so the
    // script is what paces this cycle, and taking the count from the script is what keeps the two
    // from drifting when a scan is added.
    for _ in 0..scripted_scans {
        advance_reporter(&mut app, reporter)?;
    }

    assert_eq!(
        app.world().resource::<ObservedEvents>().role_availability,
        vec![
            // The opening edge is a role that has never had hardware: nothing is owed, and the
            // kernel is the party still waiting. The closing one follows a departure under
            // `ReapplyOnReturn` with an established value, which is the kernel's own work too.
            ObservedRoleAvailability::Awaiting(role.clone(), WaitingWork::Nothing),
            ObservedRoleAvailability::Available(role.clone()),
            ObservedRoleAvailability::Awaiting(role.clone(), WaitingWork::RestorationOwed),
            ObservedRoleAvailability::Available(role),
        ]
    );

    Ok(())
}

/// Availability edges and retained-device counts around one grace interval.
struct ObservedAvailabilityCycle {
    entered_grace:             ObservedDeviceAvailabilityChange,
    retired:                   ObservedDeviceAvailabilityChange,
    retained_during_grace:     usize,
    retained_after_retirement: usize,
}

/// Introduce `key`, replace the scan, and cross the retirement deadline.
fn availability_cycle_after(
    key: &DeviceKey,
    second: ScriptedScan,
) -> Result<ObservedAvailabilityCycle, Box<dyn Error>> {
    let (mut app, reporter) =
        scripted_app(vec![scan![ScriptedDevice::present(key.clone())], second])?;

    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;

    let entered_grace = app
        .world()
        .resource::<ObservedEvents>()
        .device_changes
        .first()
        .cloned()
        .ok_or("the scripted scan change announced no availability edge")?;
    let retained_during_grace = app.world().resource::<Devices>().count();

    advance_past_departure_grace(&mut app);

    let retired = app
        .world()
        .resource::<ObservedEvents>()
        .device_changes
        .get(1)
        .cloned()
        .ok_or("the grace deadline announced no retirement edge")?;

    Ok(ObservedAvailabilityCycle {
        entered_grace,
        retired,
        retained_during_grace,
        retained_after_retirement: app.world().resource::<Devices>().count(),
    })
}

/// A role that fails and restarts inside one frame must announce both moves.
///
/// The state is announced from the apply stage rather than from a mirrored component precisely so
/// this is visible: a coarse lifecycle mirror would compare applying against applying, conclude
/// nothing moved, and hide the failure that ran between them along
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
        [
            ObservedRoleLifecycle::Waiting,
            ObservedRoleLifecycle::Applying(_)
        ]
    ));

    Ok(())
}

/// An unsupported apply stops without consuming the role's fault run or accepting reacquisition.
#[test]
fn unsupported_apply_requires_explicit_restart_and_leaves_the_fault_run_clear()
-> Result<(), Box<dyn Error>> {
    let key = panel_key("CL16")?;
    let (mut app, reporter) = scripted_app(vec![
        scan![ScriptedDevice::present(key.clone())],
        scan![ScriptedDevice::present(key.clone())],
        scan![ScriptedDevice::absent(key.clone())],
        scan![ScriptedDevice::absent(key.clone())],
        scan![ScriptedDevice::present(key.clone())],
        scan![ScriptedDevice::present(key.clone())],
    ])?;
    let role = RoleKey::new("unsupported-panel")?;
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
        .unsupported_failures_remaining = 1;

    advance_reporter(&mut app, reporter)?;
    for _ in 0..3 {
        app.update();
        if role_lifecycle(&app, &role)? == ObservedRoleLifecycle::Stopped {
            break;
        }
    }
    assert_eq!(app.world().resource::<DriverCalls>().started.len(), 1);
    let attempt_endings = &app.world().resource::<ObservedEvents>().attempt_endings;
    assert!(
        matches!(
            attempt_endings.as_slice(),
            [AttemptEndingView::Reported(AttemptOutcomeView::Failed(
                DeviceAccessErrorView::Unsupported { .. }
            ))]
        ),
        "unexpected attempt endings: {attempt_endings:?}"
    );
    assert_eq!(role_lifecycle(&app, &role)?, ObservedRoleLifecycle::Stopped);
    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Stopped(StoppedStatusView::Unsupported {
            reason: DriverStopReasonView::Unsupported { detail },
            resumes_when: ResumeCondition::ExplicitRestart,
        }) if detail == UNSUPPORTED_DRIVER_DETAIL
    ));
    let binding = app.world().resource::<Bindings>().role_entity(&role)?;
    assert!(!reflected_component_json::<RoleStatus>(&app, binding)?.is_empty());
    let starts_before_reacquisition = app.world().resource::<DriverCalls>().started.len();

    for _ in 0..2 {
        app.update();
    }
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;
    assert_eq!(role_lifecycle(&app, &role)?, ObservedRoleLifecycle::Stopped);
    assert_eq!(
        app.world().resource::<DriverCalls>().started.len(),
        starts_before_reacquisition
    );

    app.world_mut()
        .resource_mut::<DriverCalls>()
        .failures_remaining = 2;
    app.world_mut()
        .resource_mut::<Bindings>()
        .restart_role(&role)?;
    for _ in 0..3 {
        app.update();
    }

    assert!(matches!(
        role_lifecycle(&app, &role)?,
        ObservedRoleLifecycle::Applying(_)
    ));
    assert_eq!(
        app.world().resource::<DriverCalls>().started.len(),
        starts_before_reacquisition + 3
    );

    Ok(())
}

/// Three transport failures publish the count, last ending, and reacquisition restart rule.
#[test]
fn repeated_apply_failures_publish_their_typed_stopped_status() -> Result<(), Box<dyn Error>> {
    let key = panel_key("CL17")?;
    let (mut app, reporter) = scripted_app(vec![scan![ScriptedDevice::present(key.clone())]])?;
    let role = RoleKey::new("repeated-failure-panel")?;
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
        .failures_remaining = 3;

    advance_reporter(&mut app, reporter)?;
    for _ in 0..16 {
        if matches!(
            role_component(&app, &role)?.view(),
            RoleStatusView::Stopped(StoppedStatusView::RepeatedFailures { .. })
        ) {
            break;
        }
        app.update();
    }

    assert!(matches!(
        role_component(&app, &role)?.view(),
        RoleStatusView::Stopped(StoppedStatusView::RepeatedFailures {
            failures,
            last_ending: AttemptEndingView::Reported(AttemptOutcomeView::Failed(
                DeviceAccessErrorView::Transport { detail }
            )),
            resumes_when: ResumeCondition::ExplicitRestartOrReacquisition,
        }) if failures.get() == 3 && detail == DRIVER_FAILURE_DETAIL
    ));
    let binding = app.world().resource::<Bindings>().role_entity(&role)?;
    assert!(!reflected_component_json::<RoleStatus>(&app, binding)?.is_empty());

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

    let RoleStatusView::Applying {
        since, deadline, ..
    } = role_component(&app, &role)?.view()
    else {
        return Err("the scripted attempt was not applying when its deadline was read".into());
    };

    Ok(deadline.elapsed().saturating_sub(since.elapsed()))
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
        std::any::TypeId::of::<IdentityChanged>(),
        std::any::TypeId::of::<LiveRoleChanged>(),
        std::any::TypeId::of::<RetiredRoleChanged>(),
        std::any::TypeId::of::<RegistrationAttemptEnded>(),
        std::any::TypeId::of::<ReapplyConfiguration>(),
        std::any::TypeId::of::<DeviceChange>(),
        std::any::TypeId::of::<RetireRole>(),
        std::any::TypeId::of::<DiscoveryProgressChanged>(),
        std::any::TypeId::of::<DiscoveryFinished>(),
        std::any::TypeId::of::<StartupDiscoveryChanged>(),
    ] {
        assert!(type_registry.get(type_id).is_some());
    }
    drop(type_registry);
}

/// Read how many of each observed event have arrived so far.
fn observed_counts(app: &App) -> (usize, usize, usize, usize, usize, usize, usize) {
    let observed = app.world().resource::<ObservedEvents>();
    (
        observed.arrivals.len(),
        observed.device_changes.len(),
        observed.role_state.len(),
        observed.role_status.len(),
        observed.retired_roles.len(),
        observed.attempt_endings.len(),
        observed.registration_attempt_endings.len(),
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
    register_binding(
        app.world_mut(),
        BindingAuthoring::new(
            role.clone(),
            DeviceEndpoint {
                device,
                id: EndpointId::Part(PartName::new(part)?),
            },
            driver,
            PanelPlacement {
                slot: REQUESTED_SLOT,
            },
            BindingPolicy::new(
                RecoveryPolicy::ReapplyOnReturn,
                RetryOn::NewRevision,
                OnAbort::default(),
                OnSessionLoss::default(),
                ApplyDeadline::ProcessDefault,
            ),
        ),
    )?;

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
            name: ConfiguredDeviceName::NeverDerived,
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

    // `Bindings::readdress` puts the role back in a waiting status, so the binding entity's
    // resolved-device link catches up on the following frame rather than inside the adoption.
    app.update();

    assert!(matches!(
        resolved_device(&app, &role),
        RoleDeviceResolution::Resolved(resolved) if resolved == candidate
    ));

    Ok(())
}

/// The record that several roles address one physical unit lives in the device integration, so one
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
    // unplugged one — so only a presence read shows the question has nothing left to answer.
    assert!(matches!(
        app.world().resource::<Devices>().resolve(&candidate),
        DeviceResolution::Resolved(_)
    ));
    assert!(questions(&app).is_empty());
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

    // The opening reporter-backed status edge requested the scan that introduced `third`; an
    // explicit rescan repeats it and must not raise the same question again.
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
        ScriptedScan::Complete(vec![at_shared_slot(candidate)?]),
        ScriptedScan::Complete(Vec::new()),
    ])?;
    let role = RoleKey::new("panel")?;
    register_panel_role(&mut app, &role, saved)?;
    advance_reporter(&mut app, reporter)?;
    advance_reporter(&mut app, reporter)?;
    assert_eq!(questions(&app).len(), 1);

    advance_reporter(&mut app, reporter)?;

    assert!(questions(&app).is_empty());
    Ok(())
}

/// A retired role has no endpoint to rebind, so its question has nothing left to answer.
#[test]
fn a_question_expires_when_its_role_is_retired() -> Result<(), Box<dyn Error>> {
    let (mut app, _, role, ..) = displaced_app()?;

    app.world_mut().resource_mut::<Bindings>().retire(&role)?;
    app.update();

    assert!(questions(&app).is_empty());
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

    // Both saved keys remain retained without authorization during departure grace, so what
    // follows is about their identity evidence rather than about one half vacating sooner.
    assert!(matches!(
        reported_app
            .world()
            .resource::<Devices>()
            .resolve(&reported_saved),
        DeviceResolution::Resolved(_)
    ));
    assert!(matches!(
        synthesized_app
            .world()
            .resource::<Devices>()
            .resolve(&synthesized_saved),
        DeviceResolution::Resolved(_)
    ));

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
/// application has no remaining reference to.
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
