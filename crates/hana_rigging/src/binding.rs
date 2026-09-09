//! # Role-owned field matrix
//!
//! `RegisteredRole` keeps authored identity beside the private lifecycle state. Every fact that is
//! valid for only part of the lifecycle is stored in that `RoleState` variant.
//!
//! | Previous `Bindings` storage | Valid lifecycle owner |
//! |---|---|
//! | `by_role` | `RegisteredRole`; authored role, endpoint, policies, driver, and configuration |
//! | `owner_by_endpoint` | `Bindings`; reverse index over registered authored endpoints |
//! | `roles_by_device` | `Bindings`; reverse index over registered authored device keys |
//! | `waiting_work` | `WaitingState`, or `ActiveApply::continuation` while work is active |
//! | `applying_source` | `ActiveApply::source` |
//! | `establishing_attempts` | `EstablishedSession::establishing_attempt` |
//! | `attempt_failures` | `WaitingState::failures`, `ActiveApply::failures_before_attempt`, or `StoppedState::RepeatedFailures::failures` |
//! | `retry_gates` | `WaitingState::retry_pacing` |
//! | `stopped_role_endpoints` | `StoppedState::RepeatedFailures::reacquisition` |
//! | `generation_by_role` | `RegisteredRole::generation`; valid for the complete registration lifetime |
//! | generation, session, attempt, and transition counters | `Bindings` or `Attempts`; process-lifetime issuers |
//! | pending transition queue | `Bindings`; bounded cross-schedule handoff, independent of role state |

use std::any::TypeId;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::num::NonZeroUsize;
use std::ops::Deref;
use std::ops::DerefMut;
use std::time::Duration;

use bevy::ecs::entity::Entity;
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::reflect::ReflectResource;
use bevy::ecs::relationship::Relationship;
use bevy::ecs::relationship::RelationshipTarget;
use bevy::ecs::system::SystemParam;
use bevy::ecs::world::DeferredWorld;
use bevy::platform::time::Instant;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::reflect::ReflectDeserialize;
use bevy::reflect::ReflectSerialize;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::AppliedKind;
use crate::ApplyFailureDisposition;
use crate::ApplySourceView;
use crate::AttemptEnding;
use crate::AttemptEndingView;
use crate::AttemptInvalidation;
use crate::AttemptRef;
use crate::CapabilityProjectionFailure;
use crate::ClaimHolder;
use crate::ClaimHolderView;
use crate::DeviceEndpoint;
use crate::DeviceId;
use crate::DeviceKey;
use crate::DeviceKind;
use crate::DeviceNameDisambiguationText;
use crate::DeviceRevisionGateView;
use crate::DeviceRevisionView;
use crate::DiscoveryCadence;
use crate::DriverContractFailureReport;
use crate::DriverContractFailureView;
use crate::DriverId;
use crate::DriverOutcomeStatus;
use crate::DriverStopReason;
use crate::DriverStopReasonView;
use crate::EndpointDriverRegistration;
use crate::EstablishedFlow;
use crate::FlowExpectation;
use crate::HardwareWait;
use crate::LastKnownGoodConfiguration;
use crate::NonEmptyReporterRefs;
use crate::OnAbort;
use crate::OnSessionLoss;
use crate::OperatorAssignedDeviceName;
use crate::PermissionGateView;
use crate::RecoveryPolicy;
use crate::RegistrationApplicationRunView;
use crate::ReportedDeviceName;
use crate::ReporterId;
use crate::ResumeCondition;
use crate::RetiredRoleChange;
use crate::RetiredRoleChanged;
use crate::RetirementEvidence;
use crate::RetryGateView;
use crate::RetryOn;
use crate::RetryScheduleView;
use crate::RiggingLimits;
use crate::RiggingRuntimeClock;
use crate::RiggingRuntimeTime;
use crate::RoleApplyFailureRunView;
use crate::RoleEndpoint;
use crate::RoleKey;
use crate::RoleStatusView;
use crate::SessionDatumArrivalEvidence;
use crate::SessionRef;
use crate::StoppedStatusView;
use crate::WaitTiming;
use crate::WaitingStatusView;
use crate::attempt::RetryGate;
use crate::contract::ErasedApplied;
use crate::contract::SessionDatumArrivalReceiver;
use crate::devices::DeviceRevision;
use crate::devices::DeviceRevisionLookup;
use crate::flow::EstablishedFlowExpiry;
use crate::reconcile::FrameClockReading;
use crate::registration::RegisteredReporter;

const CONSECUTIVE_FAILURE_LIMIT: u32 = 3;
const DEFAULT_PENDING_TRANSITION_CAPACITY: usize = 4_096;

/// Recovery, retry, cancellation, session-loss, apply-timing, and flow-monitoring rules authored
/// for one role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub struct BindingPolicy {
    recovery:                    RecoveryPolicy,
    retry:                       RetryOn,
    on_abort:                    OnAbort,
    on_loss:                     OnSessionLoss,
    apply_deadline:              ApplyDeadline,
    pub(super) flow_expectation: FlowExpectation,
}

impl BindingPolicy {
    /// Collect the complete policy applied to one registered role.
    #[must_use]
    pub const fn new(
        recovery: RecoveryPolicy,
        retry: RetryOn,
        on_abort: OnAbort,
        on_loss: OnSessionLoss,
        apply_deadline: ApplyDeadline,
    ) -> Self {
        Self {
            recovery,
            retry,
            on_abort,
            on_loss,
            apply_deadline,
            flow_expectation: FlowExpectation::NotMonitored,
        }
    }

    pub(super) const fn with_flow_expectation(mut self, flow_expectation: FlowExpectation) -> Self {
        self.flow_expectation = flow_expectation;
        self
    }

    /// Return the retention rule applied when the bound device departs.
    #[must_use]
    pub const fn recovery(self) -> RecoveryPolicy { self.recovery }

    /// Return the condition that permits another attempt after a recoverable failure.
    #[must_use]
    pub const fn retry(self) -> RetryOn { self.retry }

    /// Return the response to an attempt abandoned after the device revision changes.
    #[must_use]
    pub const fn on_abort(self) -> OnAbort { self.on_abort }

    /// Return the response to loss of a still-present device session.
    #[must_use]
    pub const fn on_session_loss(self) -> OnSessionLoss { self.on_loss }

    /// Return the authored or process-default apply deadline.
    #[must_use]
    pub const fn apply_deadline(self) -> ApplyDeadline { self.apply_deadline }
}

/// Typed authored values accepted by the role-registration boundary.
///
/// The driver registration and requested configuration share `Configuration`, so application code
/// cannot pair a configuration with a driver instance that accepts another type. Fields stay
/// private so all construction passes through [`Self::new`] with an already validated role and
/// endpoint.
pub struct BindingAuthoring<Configuration> {
    role:      RoleKey,
    endpoint:  DeviceEndpoint,
    driver:    EndpointDriverRegistration<Configuration>,
    requested: Configuration,
    policy:    BindingPolicy,
}

impl<Configuration> BindingAuthoring<Configuration> {
    /// Build one typed registration request from validated authored values.
    #[must_use]
    /// The registered driver and the requested configuration are the same
    /// `Configuration`, so a request cannot ask one driver to apply another
    /// driver's settings:
    ///
    /// ```
    /// use bevy::prelude::Reflect;
    /// use hana_rigging::BindingAuthoring;
    /// use hana_rigging::BindingPolicy;
    /// use hana_rigging::DeviceEndpoint;
    /// use hana_rigging::EndpointDriverRegistration;
    /// use hana_rigging::RoleKey;
    ///
    /// #[derive(Reflect)]
    /// struct WindowPlacement;
    ///
    /// fn author(
    ///     role: RoleKey,
    ///     endpoint: DeviceEndpoint,
    ///     driver: EndpointDriverRegistration<WindowPlacement>,
    ///     policy: BindingPolicy,
    /// ) -> BindingAuthoring<WindowPlacement> {
    ///     BindingAuthoring::new(role, endpoint, driver, WindowPlacement, policy)
    /// }
    /// ```
    ///
    /// The request above is what keeps this case meaningful — a rename would
    /// break it loudly rather than leaving this one failing for an unrelated
    /// reason:
    ///
    /// ```compile_fail,E0308
    /// use bevy::prelude::Reflect;
    /// use hana_rigging::{
    ///     BindingAuthoring, BindingPolicy, DeviceEndpoint, EndpointDriverRegistration, RoleKey,
    /// };
    ///
    /// #[derive(Reflect)]
    /// struct WindowPlacement;
    ///
    /// #[derive(Reflect)]
    /// struct CameraSettings;
    ///
    /// fn author_camera_settings_with_window_driver(
    ///     role: RoleKey,
    ///     endpoint: DeviceEndpoint,
    ///     driver: EndpointDriverRegistration<WindowPlacement>,
    ///     policy: BindingPolicy,
    /// ) {
    ///     let _ = BindingAuthoring::new(role, endpoint, driver, CameraSettings, policy);
    /// }
    /// ```
    pub const fn new(
        role: RoleKey,
        endpoint: DeviceEndpoint,
        driver: EndpointDriverRegistration<Configuration>,
        requested: Configuration,
        policy: BindingPolicy,
    ) -> Self {
        Self {
            role,
            endpoint,
            driver,
            requested,
            policy,
        }
    }

    /// Borrow the validated role key carried by this request.
    #[must_use]
    pub const fn role(&self) -> &RoleKey { &self.role }

    /// Borrow the durable endpoint carried by this request.
    #[must_use]
    pub const fn endpoint(&self) -> &DeviceEndpoint { &self.endpoint }

    fn erase(self) -> Binding
    where
        Configuration: Reflect,
    {
        Binding {
            role:             self.role,
            endpoint:         self.endpoint,
            driver:           self.driver.driver_id(),
            recovery:         self.policy.recovery,
            retry:            self.policy.retry,
            on_abort:         self.policy.on_abort,
            on_loss:          self.policy.on_loss,
            requested:        RequestedConfiguration::new(self.requested),
            last_known_good:  LastKnownGoodConfiguration::NotEstablished,
            apply_deadline:   self.policy.apply_deadline,
            flow_expectation: self.policy.flow_expectation,
        }
    }
}

/// Registration access that reserves a role entity beside the accepted kernel record.
#[derive(SystemParam)]
pub struct BindingRegistration<'w, 's> {
    commands: Commands<'w, 's>,
    bindings: ResMut<'w, Bindings>,
}

impl BindingRegistration<'_, '_> {
    /// Borrow one retained binding while authoring a client relationship.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError::RoleNotBound`] when the role is not registered.
    pub fn binding(&self, role: &RoleKey) -> Result<&Binding, BindingError> {
        self.bindings.binding(role)
    }

    /// Return the entity retained by one registered role.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError::RoleNotBound`] when the role is not registered.
    pub fn role_entity(&self, role: &RoleKey) -> Result<Entity, BindingError> {
        self.bindings.role_entity(role)
    }

    /// Report whether a retained role routes through this typed driver registration.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError::RoleNotBound`] when the role is not registered.
    pub fn is_routed_by<Configuration>(
        &self,
        role: &RoleKey,
        driver: EndpointDriverRegistration<Configuration>,
    ) -> Result<bool, BindingError> {
        self.bindings
            .binding(role)
            .map(|binding| binding.driver == driver.driver_id())
    }

    /// Iterate the retained roles routed through one typed driver registration.
    pub fn roles_routed_by<Configuration>(
        &self,
        driver: EndpointDriverRegistration<Configuration>,
    ) -> impl Iterator<Item = &RoleKey> {
        self.bindings.roles_routed_by(driver)
    }

    /// Return the readable configuration currently associated with one role.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError::RoleNotBound`] when the role is not registered.
    pub fn configuration_for(
        &self,
        role: &RoleKey,
    ) -> Result<AvailableConfiguration<'_>, BindingError> {
        self.bindings.configuration_for(role)
    }

    /// Register one typed role and return its reserved entity in this system.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError`] before reserving an entity when the role or endpoint conflicts, or
    /// when the bounded lifecycle handoff cannot retain the registration.
    pub fn register<Configuration>(
        &mut self,
        authoring: BindingAuthoring<Configuration>,
    ) -> Result<Entity, BindingError>
    where
        Configuration: Reflect,
    {
        let binding = authoring.erase();
        let reserved_transition = self.bindings.reserve_registration(&binding)?;
        let entity = self.commands.spawn(role_entity_bundle(&binding)).id();
        self.bindings
            .register_reserved(binding, entity, reserved_transition);
        Ok(entity)
    }

    /// Insert a client relationship after any role entity reserved earlier by this parameter.
    pub fn insert_client_relationship<Relation>(&mut self, client: Entity, relationship: Relation)
    where
        Relation: Relationship,
    {
        self.commands.entity(client).insert(relationship);
    }

    /// Replace one role through the typed authoring boundary and retain its entity.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError`] when the role is not registered, the endpoint conflicts, or the
    /// lifecycle handoff cannot accept the replacement.
    pub fn replace<Configuration>(
        &mut self,
        authoring: BindingAuthoring<Configuration>,
    ) -> Result<Entity, BindingError>
    where
        Configuration: Reflect,
    {
        let role = authoring.role().clone();
        self.bindings.replace_authoring(authoring)?;
        self.bindings.role_entity(&role)
    }

    /// Retire one role authored by this client.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError`] when the lifecycle handoff cannot accept the retirement.
    pub fn retire(&mut self, role: &RoleKey) -> Result<RetirementOutcome, BindingError> {
        self.bindings.retire(role)
    }
}

/// Register one typed role from an exclusive system and return its entity immediately.
///
/// This is the `&mut World` counterpart to [`BindingRegistration::register`]. It exists for
/// integration boundaries that must coordinate registration with other resources atomically in an
/// exclusive system.
///
/// # Errors
///
/// Returns [`BindingError`] before spawning an entity under the same conditions as
/// [`BindingRegistration::register`].
pub fn register_binding<Configuration>(
    world: &mut World,
    authoring: BindingAuthoring<Configuration>,
) -> Result<Entity, BindingError>
where
    Configuration: Reflect,
{
    let binding = authoring.erase();
    let reserved_transition = world
        .resource::<Bindings>()
        .reserve_registration(&binding)?;
    let entity = world.spawn(role_entity_bundle(&binding)).id();
    world
        .resource_mut::<Bindings>()
        .register_reserved(binding, entity, reserved_transition);
    Ok(entity)
}

/// Replace one typed role from an exclusive system and retain its entity.
///
/// # Errors
///
/// Returns [`BindingError`] when the role is not registered or the replacement conflicts with
/// another retained role.
pub fn replace_binding<Configuration>(
    world: &mut World,
    authoring: BindingAuthoring<Configuration>,
) -> Result<Entity, BindingError>
where
    Configuration: Reflect,
{
    let role = authoring.role().clone();
    world
        .resource_mut::<Bindings>()
        .replace_authoring(authoring)?;
    world.resource::<Bindings>().role_entity(&role)
}

fn role_entity_bundle(
    binding: &Binding,
) -> (RegisteredRoleEntity, RoleKey, RecoveryPolicy, RoleEndpoint) {
    (
        RegisteredRoleEntity,
        binding.role.clone(),
        binding.recovery,
        RoleEndpoint::new(binding.endpoint.clone()),
    )
}

/// Marks the entity whose lifetime is owned by one live `RegisteredRole`.
#[derive(Component)]
#[component(on_despawn = preserve_registered_role_relationships)]
struct RegisteredRoleEntity;

#[derive(Clone, Copy)]
struct RoleRelationshipRepair {
    relationship: TypeId,
    sources:      fn(&DeferredWorld<'_>, Entity) -> Vec<Entity>,
    retarget:     fn(&mut World, Entity, Entity),
}

/// Relationship implementations whose sources must follow a recovered role entity.
#[derive(Default, Resource)]
pub(crate) struct RiggingRoleRelationshipRepairs(Vec<RoleRelationshipRepair>);

impl RiggingRoleRelationshipRepairs {
    pub(crate) fn register<Source>(&mut self)
    where
        Source: Relationship,
    {
        let relationship = TypeId::of::<Source>();
        if self
            .0
            .iter()
            .any(|repair| repair.relationship == relationship)
        {
            return;
        }
        self.0.push(RoleRelationshipRepair {
            relationship,
            sources: related_sources::<Source>,
            retarget: retarget_source::<Source>,
        });
    }
}

#[derive(Clone, Copy)]
struct LostRoleRelationship {
    source:   Entity,
    retarget: fn(&mut World, Entity, Entity),
}

/// Source relationships captured while an externally removed role entity is still readable.
#[derive(Default, Resource)]
pub(crate) struct LostRoleRelationships(HashMap<RoleKey, Vec<LostRoleRelationship>>);

/// Registered roles whose owned entity was observed being removed outside the kernel.
#[derive(Default, Resource)]
pub(crate) struct LostRegisteredRoleEntities(HashSet<RoleKey>);

fn related_sources<Source>(world: &DeferredWorld<'_>, entity: Entity) -> Vec<Entity>
where
    Source: Relationship,
{
    world
        .get::<Source::RelationshipTarget>(entity)
        .map(|target| target.iter().collect())
        .unwrap_or_default()
}

fn retarget_source<Source>(world: &mut World, source: Entity, replacement: Entity)
where
    Source: Relationship,
{
    let Ok(mut source_entity) = world.get_entity_mut(source) else {
        return;
    };
    source_entity.insert(Source::from(replacement));
}

fn preserve_registered_role_relationships(
    mut world: DeferredWorld<'_>,
    HookContext { entity, .. }: HookContext,
) {
    let Some(role) = world.get::<RoleKey>(entity).cloned() else {
        return;
    };
    let still_registered = world
        .get_resource::<Bindings>()
        .and_then(|bindings| bindings.role_entity(&role).ok())
        .is_some_and(|registered_entity| registered_entity == entity);
    if !still_registered {
        return;
    }
    world
        .resource_mut::<LostRegisteredRoleEntities>()
        .0
        .insert(role.clone());
    let repairs = world
        .get_resource::<RiggingRoleRelationshipRepairs>()
        .map(|repairs| repairs.0.clone())
        .unwrap_or_default();
    let lost = repairs
        .into_iter()
        .flat_map(|repair| {
            (repair.sources)(&world, entity)
                .into_iter()
                .map(move |source| LostRoleRelationship {
                    source,
                    retarget: repair.retarget,
                })
        })
        .collect::<Vec<_>>();
    if !lost.is_empty() {
        world
            .resource_mut::<LostRoleRelationships>()
            .0
            .entry(role)
            .or_default()
            .extend(lost);
    }
}

/// One authored role binding, including its durable endpoint and driver-specific configuration.
///
/// A `Binding` keeps the application role separate from the device entity that may currently
/// represent `endpoint.device`. This lets a window, camera slot, or panel key retain its authored
/// configuration while the physical unit is absent, without treating a process-local `DeviceId`
/// as durable identity.
#[derive(Reflect)]
pub struct Binding {
    /// Application role that remains stable while devices leave and return.
    pub role:             RoleKey,
    /// Durable device key and provider-defined part that this role exclusively owns.
    pub endpoint:         DeviceEndpoint,
    /// Registered endpoint driver that receives this role's erased configuration.
    pub driver:           DriverId,
    /// Retention rule applied when the device supplying this endpoint departs.
    pub recovery:         RecoveryPolicy,
    /// Retry rule applied after an endpoint driver reports a recoverable failure.
    pub retry:            RetryOn,
    /// Response selected when an in-flight operation is abandoned by a new device report.
    pub on_abort:         OnAbort,
    /// Response selected when a still-present endpoint loses its local session.
    pub on_loss:          OnSessionLoss,
    /// Authored driver target for this role, sent to the driver by a requested apply.
    pub requested:        RequestedConfiguration,
    /// Driver value a safe readback most recently proved was on this endpoint.
    pub last_known_good:  LastKnownGoodConfiguration,
    /// How long an attempt for this role may run before the kernel abandons it.
    ///
    /// Authored per binding because one process drives endpoints with genuinely different costs: a
    /// window move lands in milliseconds while opening a screen-capture stream can take seconds,
    /// and a single process-wide bound either abandons the capture or lets the window hang.
    /// The default keeps the process-wide value, so a binding that has no reason to differ
    /// says nothing.
    pub apply_deadline:   ApplyDeadline,
    /// Data-arrival rule evaluated after this role establishes a session.
    pub flow_expectation: FlowExpectation,
}

/// How long one role's attempts may run, and whether the binding authored that itself.
///
/// A named enum rather than an optional `std::time::Duration` because the two cases lead to
/// different behaviour when `crate::RiggingLimits::apply_deadline` is later retuned: a
/// `Self::ProcessDefault` binding follows the new value and a `Self::Authored` one deliberately
/// does not.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Reflect)]
pub enum ApplyDeadline {
    /// Use `crate::RiggingLimits::apply_deadline`, the bound every role shares.
    ///
    /// The default, so that adding this field asked nothing of a binding whose endpoint has no
    /// reason to be timed differently from the rest of the process.
    #[default]
    ProcessDefault,
    /// Use this role's own bound instead of the process-wide one.
    Authored(Duration),
}

impl ApplyDeadline {
    /// Resolve the authored choice against the process-wide bound.
    ///
    /// The result names which of the two supplied the value rather than returning a bare duration,
    /// so a caller reading a stamped attempt back can tell a role that authored five seconds from
    /// one that inherited five seconds from the process.
    #[must_use]
    pub(crate) const fn resolve(self, rigging_limits: &RiggingLimits) -> ApplyDeadlineLookup {
        match self {
            Self::ProcessDefault => {
                ApplyDeadlineLookup::ProcessDefault(rigging_limits.apply_deadline)
            },
            Self::Authored(apply_deadline) => ApplyDeadlineLookup::Authored(apply_deadline),
        }
    }
}

/// Which bound an attempt was stamped with, and where it came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
pub(crate) enum ApplyDeadlineLookup {
    /// The binding authored nothing, so the attempt carries the process-wide bound.
    ProcessDefault(Duration),
    /// The binding authored its own bound, which a later change to
    /// `crate::RiggingLimits::apply_deadline` will not move.
    Authored(Duration),
}

impl ApplyDeadlineLookup {
    /// Return the bound, discarding which of the two supplied it.
    #[must_use]
    pub(crate) const fn duration(self) -> Duration {
        match self {
            Self::ProcessDefault(apply_deadline) | Self::Authored(apply_deadline) => apply_deadline,
        }
    }
}

/// Authored driver configuration held without exposing the concrete configuration type to the
/// kernel.
///
/// The concrete configuration remains owned by the endpoint driver. `RequestedConfiguration`
/// exists because a display placement and a camera format can share a `DeviceKey` while requiring
/// unrelated driver types and routing rules.
#[derive(Reflect)]
pub struct RequestedConfiguration(
    #[reflect(ignore, default = "default_erased_configuration")] Box<dyn Reflect>,
);

impl RequestedConfiguration {
    /// Erase one driver-specific value while retaining it as authored role intent.
    #[must_use]
    pub fn new(configuration: impl Reflect) -> Self { Self(Box::new(configuration)) }

    pub(super) fn configuration(&self) -> &dyn Reflect { self.0.as_ref() }
}

impl Binding {
    /// Return the authored lifecycle policy for this binding.
    #[must_use]
    pub const fn policy(&self) -> BindingPolicy {
        BindingPolicy::new(
            self.recovery,
            self.retry,
            self.on_abort,
            self.on_loss,
            self.apply_deadline,
        )
        .with_flow_expectation(self.flow_expectation)
    }

    /// Borrow the last configuration an accepted driver success established.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LastKnownGoodConfigurationAccessError::NotEstablished`] until the first
    /// accepted success.
    pub fn last_known_good(
        &self,
    ) -> Result<&dyn Reflect, crate::LastKnownGoodConfigurationAccessError> {
        self.last_known_good
            .as_reflect(self.requested.configuration())
    }
}

fn default_erased_configuration() -> Box<dyn Reflect> { Box::new(()) }

/// Configuration currently available to an offline UI or authoring workflow.
///
/// A proven value takes precedence because it describes the endpoint state a safe readback
/// observed. When no readback has succeeded, the authored request remains useful for presenting
/// the role's intended value without fabricating endpoint evidence.
pub enum AvailableConfiguration<'a> {
    /// A safe readback established this value on the endpoint.
    LastKnownGood(&'a dyn Reflect),
    /// No readback established a value, so this is the authored target instead.
    Requested(&'a dyn Reflect),
}

/// Identity of one installed binding, issued by `Bindings` each time a role's binding is
/// registered, replaced, or readdressed.
///
/// A `crate::RoleKey` is the role's durable name and survives replacement; this value does not.
/// Each attempt is stamped with the generation that dispatched it, so an attempt ending can be
/// told apart from the binding currently installed under the same role name: an ending whose
/// generation is not the current one belongs to a superseded binding and may not touch the
/// current binding's retry gates or failure counts.
///
/// Reflection sees the counter value opaquely for the same reason as `crate::AttemptRef`: a
/// dynamic tuple struct must not be able to fabricate a generation the kernel never issued.
/// Numbering starts at 1 by the same convention, so a defaulted value names no installed binding.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Reflect)]
#[reflect(opaque)]
pub(crate) struct BindingGeneration(u64);

/// Consecutive failed applies retained for one role.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum RoleApplyFailureRun {
    /// No failed apply precedes the next dispatch.
    #[default]
    Clear,
    /// This many failed applies precede the next dispatch.
    Consecutive {
        failures:    NonZeroU32,
        last_ending: AttemptEnding,
    },
}

impl RoleApplyFailureRun {
    fn incremented(self, last_ending: AttemptEnding) -> Self {
        match self {
            Self::Clear => Self::Consecutive {
                failures: NonZeroU32::MIN,
                last_ending,
            },
            Self::Consecutive { failures, .. } => {
                let incremented = failures.get().saturating_add(1);
                Self::Consecutive {
                    failures: NonZeroU32::new(incremented).unwrap_or(NonZeroU32::MAX),
                    last_ending,
                }
            },
        }
    }

    const fn count(&self) -> u32 {
        match self {
            Self::Clear => 0,
            Self::Consecutive { failures, .. } => failures.get(),
        }
    }
}

/// Attempt history retained for the lifetime of one binding generation.
#[derive(Clone, Debug, Default)]
enum RegistrationApplicationRun {
    /// The registration has not issued an apply.
    #[default]
    NotStarted,
    /// The registration's first apply is in flight.
    InitialApplication,
    /// A later apply is in flight after the retained ending.
    Reapplying {
        applications: NonZeroU32,
        last_ending:  AttemptEnding,
    },
    /// The most recent apply ended and no successor has started yet.
    Ended {
        applications: NonZeroU32,
        last_ending:  AttemptEnding,
    },
}

impl RegistrationApplicationRun {
    fn start(&mut self) {
        let previous = std::mem::take(self);
        *self = match previous {
            Self::NotStarted => Self::InitialApplication,
            Self::Ended {
                applications,
                last_ending,
            } => Self::Reapplying {
                applications: NonZeroU32::new(applications.get().saturating_add(1))
                    .unwrap_or(NonZeroU32::MAX),
                last_ending,
            },
            Self::InitialApplication | Self::Reapplying { .. } => previous,
        };
    }

    fn end(&mut self, last_ending: AttemptEnding) {
        let previous = std::mem::take(self);
        *self = match previous {
            Self::InitialApplication => Self::Ended {
                applications: NonZeroU32::MIN,
                last_ending,
            },
            Self::Reapplying { applications, .. } => Self::Ended {
                applications,
                last_ending,
            },
            Self::NotStarted | Self::Ended { .. } => previous,
        };
    }

    fn view(&self) -> RegistrationApplicationRunView {
        match self {
            Self::NotStarted | Self::InitialApplication => RegistrationApplicationRunView::Initial,
            Self::Reapplying {
                applications,
                last_ending,
            } => RegistrationApplicationRunView::Reapplying {
                applications: *applications,
                last_ending:  project_attempt_ending(last_ending),
            },
            Self::Ended {
                applications,
                last_ending,
            } => {
                if *applications == NonZeroU32::MIN {
                    RegistrationApplicationRunView::Initial
                } else {
                    RegistrationApplicationRunView::Reapplying {
                        applications: *applications,
                        last_ending:  project_attempt_ending(last_ending),
                    }
                }
            },
        }
    }
}

/// One or more reporter identifiers required by a reporter-ended wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NonEmptyReporterIds {
    first: ReporterId,
    rest:  Vec<ReporterId>,
}

impl NonEmptyReporterIds {
    fn from_registered_reporters<'a>(
        first: &RegisteredReporter<'a>,
        rest: &[RegisteredReporter<'a>],
    ) -> Self {
        Self {
            first: first.reporter,
            rest:  rest.iter().map(|reporter| reporter.reporter).collect(),
        }
    }

    pub(crate) fn from_reporter_ids(first: ReporterId, rest: &[ReporterId]) -> Self {
        Self {
            first,
            rest: rest.to_vec(),
        }
    }

    fn reporter_refs(&self) -> NonEmptyReporterRefs {
        NonEmptyReporterRefs::from_first_and_rest(self.first, &self.rest)
    }
}

/// Timing input owned by one wait condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitBoundInput {
    /// The wait crosses its bound at this schedule instant.
    Until(Instant),
    /// The wait has no time bound.
    Unbounded,
}

/// Whether an active wait has crossed its typed time bound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum WaitBoundState {
    /// The current schedule instant has not crossed the bound.
    #[default]
    Within,
    /// The bound was crossed and has already produced its transition.
    Crossed { crossed_at: Instant },
}

/// Reporter-owned hardware condition ending a role wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimedReporterWait {
    key:       DeviceKey,
    reporters: NonEmptyReporterIds,
    bound:     WaitBoundInput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReporterAbsenceWait {
    key:      DeviceKey,
    evidence: RetirementEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnconfirmedReporterWait {
    wait:  TimedReporterWait,
    basis: crate::UnconfirmedBasis,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReporterWait {
    AwaitingFirstReport(TimedReporterWait),
    Unconfirmed(UnconfirmedReporterWait),
    Unreachable(TimedReporterWait),
    DepartureGrace {
        key:      DeviceKey,
        deadline: Instant,
        evidence: RetirementEvidence,
    },
    Absent(ReporterAbsenceWait),
}

impl ReporterWait {
    pub(crate) fn awaiting_first_report(
        key: DeviceKey,
        now: Instant,
        first: &RegisteredReporter<'_>,
        rest: &[RegisteredReporter<'_>],
    ) -> Self {
        let first_complete_set_bound = rest
            .iter()
            .fold(first.first_complete_set_bound, |longest, reporter| {
                longest.max(reporter.first_complete_set_bound)
            });
        Self::AwaitingFirstReport(TimedReporterWait {
            key,
            reporters: NonEmptyReporterIds::from_registered_reporters(first, rest),
            bound: WaitBoundInput::Until(now + first_complete_set_bound),
        })
    }

    pub(crate) fn unconfirmed(
        key: DeviceKey,
        now: Instant,
        first: &RegisteredReporter<'_>,
        rest: &[RegisteredReporter<'_>],
        rigging_limits: &RiggingLimits,
        basis: crate::UnconfirmedBasis,
    ) -> Self {
        Self::Unconfirmed(UnconfirmedReporterWait {
            wait: Self::timed_from_cadences(key, now, first, rest, rigging_limits),
            basis,
        })
    }

    pub(crate) fn unreachable(
        key: DeviceKey,
        now: Instant,
        first: &RegisteredReporter<'_>,
        rest: &[RegisteredReporter<'_>],
        rigging_limits: &RiggingLimits,
    ) -> Self {
        Self::Unreachable(Self::timed_from_cadences(
            key,
            now,
            first,
            rest,
            rigging_limits,
        ))
    }

    pub(crate) const fn absent(key: DeviceKey, established_by: RetirementEvidence) -> Self {
        Self::Absent(ReporterAbsenceWait {
            key,
            evidence: established_by,
        })
    }

    pub(crate) const fn departure_grace(
        key: DeviceKey,
        deadline: Instant,
        evidence: RetirementEvidence,
    ) -> Self {
        Self::DepartureGrace {
            key,
            deadline,
            evidence,
        }
    }

    fn timed_from_cadences(
        key: DeviceKey,
        now: Instant,
        first: &RegisteredReporter<'_>,
        rest: &[RegisteredReporter<'_>],
        rigging_limits: &RiggingLimits,
    ) -> TimedReporterWait {
        let reporters = NonEmptyReporterIds::from_registered_reporters(first, rest);
        let bound = std::iter::once(first)
            .chain(rest.iter())
            .try_fold(Duration::ZERO, |longest, reporter| {
                reporter_cadence_bound(reporter.cadence, rigging_limits)
                    .map(|bound| longest.max(bound))
            })
            .map_or(WaitBoundInput::Unbounded, |bound| {
                WaitBoundInput::Until(now + bound)
            });
        TimedReporterWait {
            key,
            reporters,
            bound,
        }
    }

    const fn bound(&self) -> WaitBoundInput {
        match self {
            Self::AwaitingFirstReport(wait) | Self::Unreachable(wait) => wait.bound,
            Self::Unconfirmed(wait) => wait.wait.bound,
            Self::DepartureGrace { deadline, .. } => WaitBoundInput::Until(*deadline),
            Self::Absent(_) => WaitBoundInput::Unbounded,
        }
    }

    fn same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::AwaitingFirstReport(left), Self::AwaitingFirstReport(right))
            | (Self::Unreachable(left), Self::Unreachable(right)) => {
                left.key == right.key && left.reporters == right.reporters
            },
            (Self::Unconfirmed(left), Self::Unconfirmed(right)) => {
                left.wait.key == right.wait.key
                    && left.wait.reporters == right.wait.reporters
                    && left.basis == right.basis
            },
            (Self::Absent(left), Self::Absent(right)) => {
                left.key == right.key && left.evidence == right.evidence
            },
            (
                Self::DepartureGrace { key: left_key, .. },
                Self::DepartureGrace { key: right_key, .. },
            ) => left_key == right_key,
            _ => false,
        }
    }
}

fn reporter_cadence_bound(
    discovery_cadence: &DiscoveryCadence,
    rigging_limits: &RiggingLimits,
) -> Option<Duration> {
    match discovery_cadence {
        DiscoveryCadence::OnDemand => None,
        DiscoveryCadence::EventDriven { backstop } => Some(*backstop + rigging_limits.report_grace),
        DiscoveryCadence::Periodic { interval } => Some(*interval + rigging_limits.report_grace),
    }
}

/// Condition whose actor or state change ends a role wait.
#[expect(
    dead_code,
    reason = "later phases produce the actor-specific waits whose vocabulary is installed here"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WaitingCondition {
    Reporter(ReporterWait),
    KernelRetry(RetryGate),
    ApplicationReapply,
    ApplicationReporterEnable {
        reporters: NonEmptyReporterIds,
    },
    NewRegistration,
    OperatorDecision {
        role:      RoleKey,
        candidate: DeviceKey,
    },
    ClaimRelease {
        holder: ClaimHolder,
    },
    DriverRepair {
        error: DriverContractFailureReport,
    },
    ApplicationCapabilityRegistrationRequired {
        failure: CapabilityProjectionFailure,
    },
    ApplicationReporterRegistrationRequired {
        key: DeviceKey,
    },
    ApplicationDeviceEnableRequired {
        key: DeviceKey,
    },
    ApplicationPermissionRequired {
        gate: crate::PermissionGate,
    },
    ApplicationBindingRepairRequired,
    ApplicationTargetAttachmentRequired,
    KernelStateRepairRequired,
}

impl WaitingCondition {
    const fn bound(&self) -> WaitBoundInput {
        match self {
            Self::Reporter(reporter_wait) => reporter_wait.bound(),
            Self::KernelRetry(RetryGate::AwaitingInstant(retry_at)) => {
                WaitBoundInput::Until(*retry_at)
            },
            Self::KernelRetry(RetryGate::AwaitingRevision(_))
            | Self::ApplicationReapply
            | Self::ApplicationReporterEnable { .. }
            | Self::NewRegistration
            | Self::OperatorDecision { .. }
            | Self::ClaimRelease { .. }
            | Self::DriverRepair { .. }
            | Self::ApplicationCapabilityRegistrationRequired { .. }
            | Self::ApplicationReporterRegistrationRequired { .. }
            | Self::ApplicationDeviceEnableRequired { .. }
            | Self::ApplicationPermissionRequired { .. }
            | Self::ApplicationBindingRepairRequired
            | Self::ApplicationTargetAttachmentRequired
            | Self::KernelStateRepairRequired => WaitBoundInput::Unbounded,
        }
    }

    fn same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Reporter(left), Self::Reporter(right)) => left.same_identity(right),
            (Self::KernelRetry(left), Self::KernelRetry(right)) => match (left, right) {
                (RetryGate::AwaitingRevision(left), RetryGate::AwaitingRevision(right)) => {
                    left == right
                },
                (RetryGate::AwaitingInstant(_), RetryGate::AwaitingInstant(_)) => true,
                _ => false,
            },
            (Self::ApplicationReapply, Self::ApplicationReapply)
            | (Self::NewRegistration, Self::NewRegistration)
            | (Self::ApplicationBindingRepairRequired, Self::ApplicationBindingRepairRequired)
            | (
                Self::ApplicationTargetAttachmentRequired,
                Self::ApplicationTargetAttachmentRequired,
            )
            | (Self::KernelStateRepairRequired, Self::KernelStateRepairRequired) => true,
            (
                Self::ApplicationReporterEnable { reporters: left },
                Self::ApplicationReporterEnable { reporters: right },
            ) => left == right,
            (
                Self::OperatorDecision {
                    role: left_role,
                    candidate: left_candidate,
                },
                Self::OperatorDecision {
                    role: right_role,
                    candidate: right_candidate,
                },
            ) => left_role == right_role && left_candidate == right_candidate,
            (Self::ClaimRelease { holder: left }, Self::ClaimRelease { holder: right }) => {
                left == right
            },
            (Self::DriverRepair { error: left }, Self::DriverRepair { error: right }) => {
                left == right
            },
            (
                Self::ApplicationCapabilityRegistrationRequired { failure: left },
                Self::ApplicationCapabilityRegistrationRequired { failure: right },
            ) => left == right,
            (
                Self::ApplicationReporterRegistrationRequired { key: left },
                Self::ApplicationReporterRegistrationRequired { key: right },
            )
            | (
                Self::ApplicationDeviceEnableRequired { key: left },
                Self::ApplicationDeviceEnableRequired { key: right },
            ) => left == right,
            (
                Self::ApplicationPermissionRequired { gate: left },
                Self::ApplicationPermissionRequired { gate: right },
            ) => left == right,
            _ => false,
        }
    }
}

/// Failure run and pacing retained while a role waits.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WaitingState {
    failures:             RoleApplyFailureRun,
    retry_pacing:         RetryPacing,
    revision_retry_bound: WaitBoundInput,
    condition:            WaitingCondition,
    wait_start:           WaitStart,
    bound_state:          WaitBoundState,
    waiting_work:         WaitingWork,
}

impl WaitingState {
    const fn registered() -> Self {
        Self {
            failures:             RoleApplyFailureRun::Clear,
            retry_pacing:         RetryPacing::Ready,
            revision_retry_bound: WaitBoundInput::Unbounded,
            condition:            WaitingCondition::NewRegistration,
            wait_start:           WaitStart::BeforeFirstSchedule,
            bound_state:          WaitBoundState::Within,
            waiting_work:         WaitingWork::Nothing,
        }
    }

    const fn published_bound(&self) -> WaitBoundInput {
        match (&self.condition, self.retry_pacing) {
            (
                WaitingCondition::KernelRetry(_)
                | WaitingCondition::DriverRepair { .. }
                | WaitingCondition::ApplicationBindingRepairRequired,
                RetryPacing::Blocked(retry_gate),
            ) => retry_gate_bound(retry_gate, self.revision_retry_bound),
            _ => self.condition.bound(),
        }
    }
}

/// Schedule-clock state for the beginning of one retained wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitStart {
    /// The role was registered before the schedule supplied its first frame instant.
    BeforeFirstSchedule,
    /// The wait began at this schedule instant.
    At(Instant),
}

/// Full authorization retained inside an applying role.
#[derive(Clone, Debug)]
pub(crate) struct AuthorizedApplyAttempt {
    reference:       AttemptRef,
    generation:      BindingGeneration,
    endpoint:        DeviceEndpoint,
    device_id:       DeviceId,
    device_revision: DeviceRevision,
    started_at:      Instant,
    deadline:        Instant,
}

impl AuthorizedApplyAttempt {
    pub(crate) const fn new(
        reference: AttemptRef,
        generation: BindingGeneration,
        endpoint: DeviceEndpoint,
        device_id: DeviceId,
        device_revision: DeviceRevision,
        started_at: Instant,
        deadline: Instant,
    ) -> Self {
        Self {
            reference,
            generation,
            endpoint,
            device_id,
            device_revision,
            started_at,
            deadline,
        }
    }

    pub(crate) const fn reference(&self) -> AttemptRef { self.reference }

    pub(crate) const fn generation(&self) -> BindingGeneration { self.generation }

    pub(crate) const fn endpoint(&self) -> &DeviceEndpoint { &self.endpoint }

    pub(crate) const fn device_id(&self) -> DeviceId { self.device_id }

    pub(crate) const fn device_revision(&self) -> DeviceRevision { self.device_revision }

    pub(crate) const fn deadline(&self) -> Instant { self.deadline }
}

/// State reached after the active apply ends.
#[derive(Clone, Debug)]
enum ApplyContinuation {
    Establish,
    Wait(WaitingState),
    DeviceUnavailable(WaitingState),
}

/// Configuration source and authorization retained during driver work.
#[derive(Clone, Debug)]
struct ActiveApply {
    attempt:                 AuthorizedApplyAttempt,
    source:                  ApplyConfigurationSource,
    failures_before_attempt: RoleApplyFailureRun,
    continuation:            ApplyContinuation,
}

#[derive(Clone, Copy)]
enum SuccessfulApplySessionAuthority {
    EstablishingAttempt,
    NoEstablishingAttempt,
}

#[derive(Debug)]
enum EstablishedSessionDatumArrivals {
    Lease(SessionDatumArrivalReceiver),
    NoLeaseIssued,
}

impl EstablishedSessionDatumArrivals {
    fn has_pending(&self) -> bool {
        match self {
            Self::Lease(receiver) => receiver.has_pending(),
            Self::NoLeaseIssued => false,
        }
    }

    fn take_latest(&self) -> SessionDatumArrivalEvidence {
        match self {
            Self::Lease(receiver) => receiver.take_latest(),
            Self::NoLeaseIssued => SessionDatumArrivalEvidence::NoDatumObserved,
        }
    }
}

/// Facts valid only while a role has an established driver session.
#[expect(
    dead_code,
    reason = "the retained configuration source is reserved for later session diagnostics"
)]
#[derive(Debug)]
struct EstablishedSession {
    source:               ApplyConfigurationSource,
    established_at:       Instant,
    session:              SessionRef,
    applied:              AppliedKind,
    establishing_attempt: EstablishingAttemptLookup,
    flow:                 EstablishedFlow,
    datum_arrivals:       EstablishedSessionDatumArrivals,
}

impl EstablishedSession {
    fn new(
        source: ApplyConfigurationSource,
        established_at: Instant,
        session: SessionRef,
        applied: AppliedKind,
        establishing_attempt: EstablishingAttemptLookup,
        policy: BindingPolicy,
        datum_arrivals: EstablishedSessionDatumArrivals,
    ) -> Self {
        let mut established = Self {
            source,
            established_at,
            session,
            applied,
            establishing_attempt,
            flow: EstablishedFlow::new(policy.flow_expectation(), established_at),
            datum_arrivals,
        };
        established.credit_latest_datum_arrival();
        established
    }

    fn credit_latest_datum_arrival(&mut self) {
        match self.datum_arrivals.take_latest() {
            SessionDatumArrivalEvidence::ObservedAt(observed_at) => {
                self.flow.record_datum_arrival(observed_at);
            },
            SessionDatumArrivalEvidence::TransportActivityAt(observed_at) => {
                self.flow.record_transport_activity(observed_at);
            },
            SessionDatumArrivalEvidence::NoDatumObserved => {},
        }
    }
}

/// Kernel-owned facts needed to validate and finish one active driver operation.
#[derive(Clone, Debug)]
pub(crate) struct ActiveAttemptRecord {
    pub(crate) role:    RoleKey,
    pub(crate) entity:  Entity,
    pub(crate) driver:  DriverId,
    pub(crate) attempt: AuthorizedApplyAttempt,
}

/// Kernel-owned guards for one established driver session.
#[derive(Clone, Debug)]
pub(crate) struct EstablishedSessionRecord {
    pub(crate) role:    RoleKey,
    pub(crate) driver:  DriverId,
    pub(crate) session: SessionRef,
}

/// What one continuous-flow judgment moved.
#[derive(Debug)]
pub(crate) enum ContinuousFlowJudgment {
    /// Testimony was credited without moving any published role status, and no session is owed a
    /// release. Nothing a consumer of the binding register can observe changed.
    NothingPublished,
    /// Published role status moved. Each retained record names a session whose stall was published
    /// on an earlier update and which is now owed its release.
    Published {
        release_required: Vec<EstablishedSessionRecord>,
    },
}

/// Whether a stopped role's failed device has departed since the stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReacquisitionProgress {
    StillAvailable,
    Gone,
}

/// Terminal role state with only the fields valid for each stop reason.
#[derive(Clone, Debug)]
enum StoppedState {
    RepeatedFailures {
        failures:      NonZeroU32,
        reacquisition: ReacquisitionProgress,
        last_ending:   AttemptEnding,
    },
    Unsupported {
        failure: DriverStopReason,
    },
}

/// Kernel-owned lifecycle state for one registered role.
#[derive(Debug)]
enum RoleState {
    Waiting(WaitingState),
    Applying(Box<ActiveApply>),
    Established(EstablishedSession),
    Stopped(StoppedState),
}

/// One role's authored binding and every kernel-owned fact scoped to its registration lifetime.
struct RegisteredRole {
    binding:         Binding,
    entity:          Entity,
    generation:      BindingGeneration,
    application_run: RegistrationApplicationRun,
    state:           RoleState,
}

impl RegisteredRole {
    const fn registered(binding: Binding, entity: Entity, generation: BindingGeneration) -> Self {
        Self {
            binding,
            entity,
            generation,
            application_run: RegistrationApplicationRun::NotStarted,
            state: RoleState::Waiting(WaitingState::registered()),
        }
    }

    #[cfg(test)]
    fn set_state(&mut self, state: RoleState) { self.state = state; }

    fn driver_cleanup(&self) -> DriverCleanup {
        match &self.state {
            RoleState::Applying(active_apply) => DriverCleanup::Applying {
                driver:     self.binding.driver,
                attempt:    active_apply.attempt.reference(),
                generation: active_apply.attempt.generation(),
            },
            RoleState::Established(session) => DriverCleanup::Established {
                driver:  self.binding.driver,
                session: session.session,
            },
            RoleState::Waiting(_) | RoleState::Stopped(_) => DriverCleanup::None,
        }
    }
}

/// Observable transition produced by `Bindings::set_wait`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitTransition {
    Entered,
    ReasonChanged,
    BoundCrossed,
    Recovered,
    Unchanged,
}

/// Availability conclusion being projected onto one role's wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AvailabilityWaitTarget {
    Present,
    Unavailable,
}

/// Lifecycle-safe operation for one availability wait projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AvailabilityWaitAction {
    /// Replace or refresh the role's current wait.
    SetWait,
    /// Retain the unavailable wait as the active attempt's cancellation successor.
    StageForApplyingRole,
    /// Preserve an established session, stopped role, or absent registration.
    PreserveLifecycle,
}

fn wait_bound_crossed(
    wait_start: WaitStart,
    now: Instant,
    wait_bound_input: WaitBoundInput,
) -> bool {
    match wait_start {
        WaitStart::BeforeFirstSchedule => false,
        WaitStart::At(started_at) => {
            now >= started_at
                && matches!(wait_bound_input, WaitBoundInput::Until(deadline) if now >= deadline)
        },
    }
}

pub(crate) const fn waiting_condition_for_work(waiting_work: WaitingWork) -> WaitingCondition {
    match waiting_work {
        WaitingWork::Nothing => WaitingCondition::NewRegistration,
        WaitingWork::RestorationOwed | WaitingWork::RegistrationOwed => {
            WaitingCondition::NewRegistration
        },
        WaitingWork::ReapplyRequestOwed => WaitingCondition::ApplicationReapply,
    }
}

fn waiting_state_from_lifecycle(
    state: RoleState,
    now: Instant,
    condition: WaitingCondition,
) -> (WaitingState, WaitTransition) {
    match state {
        RoleState::Waiting(mut waiting_state) => {
            if waiting_state.condition.same_identity(&condition) {
                let crossed = waiting_state.bound_state == WaitBoundState::Within
                    && wait_bound_crossed(
                        waiting_state.wait_start,
                        now,
                        waiting_state.published_bound(),
                    );
                if crossed {
                    waiting_state.bound_state = WaitBoundState::Crossed { crossed_at: now };
                    (waiting_state, WaitTransition::BoundCrossed)
                } else {
                    (waiting_state, WaitTransition::Unchanged)
                }
            } else {
                waiting_state.condition = condition;
                waiting_state.revision_retry_bound = waiting_state.condition.bound();
                waiting_state.wait_start = WaitStart::At(now);
                waiting_state.bound_state = WaitBoundState::Within;
                (waiting_state, WaitTransition::ReasonChanged)
            }
        },
        RoleState::Applying(active_apply) => {
            let (failures, waiting_work) = match active_apply.continuation {
                ApplyContinuation::Establish => {
                    (active_apply.failures_before_attempt, WaitingWork::Nothing)
                },
                ApplyContinuation::Wait(waiting_state)
                | ApplyContinuation::DeviceUnavailable(waiting_state) => {
                    (waiting_state.failures, waiting_state.waiting_work)
                },
            };
            (
                WaitingState {
                    failures,
                    retry_pacing: RetryPacing::Ready,
                    revision_retry_bound: condition.bound(),
                    condition,
                    wait_start: WaitStart::At(now),
                    bound_state: WaitBoundState::Within,
                    waiting_work,
                },
                WaitTransition::Entered,
            )
        },
        RoleState::Established(_) => (cleared_wait(condition, now), WaitTransition::Entered),
        RoleState::Stopped(_) => (cleared_wait(condition, now), WaitTransition::Recovered),
    }
}

/// A wait with no inherited failure run, for a role arriving from a state that carries none.
const fn cleared_wait(condition: WaitingCondition, now: Instant) -> WaitingState {
    WaitingState {
        failures: RoleApplyFailureRun::Clear,
        retry_pacing: RetryPacing::Ready,
        revision_retry_bound: condition.bound(),
        condition,
        wait_start: WaitStart::At(now),
        bound_state: WaitBoundState::Within,
        waiting_work: WaitingWork::Nothing,
    }
}

impl Deref for RegisteredRole {
    type Target = Binding;

    fn deref(&self) -> &Self::Target { &self.binding }
}

impl DerefMut for RegisteredRole {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.binding }
}

#[derive(Default)]
struct RegisteredRoles(HashMap<RoleKey, RegisteredRole>);

impl RegisteredRoles {
    fn contains_key(&self, role: &RoleKey) -> bool { self.0.contains_key(role) }

    fn get(&self, role: &RoleKey) -> Option<&RegisteredRole> { self.0.get(role) }

    fn get_mut(&mut self, role: &RoleKey) -> Option<&mut RegisteredRole> { self.0.get_mut(role) }

    fn insert(
        &mut self,
        role: RoleKey,
        registered_role: impl Into<RegisteredRole>,
    ) -> Option<RegisteredRole> {
        self.0.insert(role, registered_role.into())
    }

    fn remove(&mut self, role: &RoleKey) -> Option<RegisteredRole> { self.0.remove(role) }

    fn keys(&self) -> impl Iterator<Item = &RoleKey> { self.0.keys() }
}

/// Role records, endpoint ownership, and bounded lifecycle handoff owned by the kernel.
///
/// `Bindings` retains authored intent even when no live device entity exists. Its private reverse
/// indexes make duplicate endpoint ownership unavailable through checked registration methods,
/// while `PendingBindingTransitions` retains only the next frame's lifecycle work.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct Bindings {
    #[reflect(ignore, default = "default_bindings_by_role")]
    by_role:                  RegisteredRoles,
    #[reflect(ignore, default = "default_owner_by_endpoint")]
    owner_by_endpoint:        HashMap<DeviceEndpoint, RoleKey>,
    #[reflect(ignore, default = "default_roles_by_device")]
    roles_by_device:          HashMap<DeviceKey, Vec<RoleKey>>,
    #[reflect(ignore, default = "default_generation_counter")]
    next_generation:          u64,
    #[reflect(ignore, default = "default_session_counter")]
    next_session:             u64,
    #[reflect(ignore, default = "PendingBindingTransitions::default")]
    pending_transitions:      PendingBindingTransitions,
    #[reflect(ignore, default = "default_transition_sequence")]
    next_transition_sequence: u64,
    #[reflect(ignore, default = "default_role_status_clock")]
    role_status_clock:        RoleStatusClock,
    #[reflect(ignore, default = "Vec::new")]
    pending_status_changes:   Vec<RoleStatusChange>,
}

#[derive(Clone, Copy)]
enum RoleStatusClock {
    NotInstalled,
    Installed(RiggingRuntimeClock),
}

const fn default_role_status_clock() -> RoleStatusClock { RoleStatusClock::NotInstalled }

impl Default for Bindings {
    fn default() -> Self {
        Self {
            by_role:                  RegisteredRoles::default(),
            owner_by_endpoint:        HashMap::new(),
            roles_by_device:          HashMap::new(),
            next_generation:          0,
            next_session:             0,
            pending_transitions:      PendingBindingTransitions::default(),
            next_transition_sequence: 0,
            role_status_clock:        RoleStatusClock::NotInstalled,
            pending_status_changes:   Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoleStatusChange {
    pub(crate) role: RoleKey,
    pub(crate) from: RoleStatusView,
    pub(crate) to:   RoleStatusView,
}

fn default_bindings_by_role() -> RegisteredRoles { RegisteredRoles::default() }

fn default_owner_by_endpoint() -> HashMap<DeviceEndpoint, RoleKey> { HashMap::new() }

fn default_roles_by_device() -> HashMap<DeviceKey, Vec<RoleKey>> { HashMap::new() }

const fn default_generation_counter() -> u64 { 0 }

const fn default_session_counter() -> u64 { 0 }

/// What a waiting role is owed, distinct from why it is waiting.
///
/// Stored rather than derived: the attempt systems select requested intent versus a restore from
/// this value, and configuration capture is suppressed on `Self::RestorationOwed` instead of being
/// re-derived from recovery policy and attempt history at each of those call sites, where the two
/// derivations would eventually disagree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum WaitingWork {
    /// Nothing is owed. The role is waiting for usable, authorized hardware.
    #[default]
    Nothing,
    /// A last-known-good restoration is owed and runs as soon as the role is authorized. Capture is
    /// suppressed until it completes, because reading a value back before the owed one has been
    /// reapplied would record the endpoint's current state as the last one known to work.
    RestorationOwed,
    /// The kernel still holds this role's saved configuration and will send it when application
    /// code triggers `crate::ReapplyConfiguration` — the one move that clears this hold.
    ///
    /// Recorded on departure for `crate::RecoveryPolicy::ReapplyOnRequest`, which asked for exactly
    /// this, and on an established-session loss under `crate::OnSessionLoss::ReportOnly`, where the
    /// device is still present but the application asked the kernel not to open a replacement.
    ///
    /// The hold itself is what distinguishes those from `crate::RecoveryPolicy::ReapplyOnReturn`:
    /// without it a departed role returns to `Nothing`, reaches `WaitingRole::Hardware`, and has
    /// its authored request dispatched automatically, which is the automatic reapply both asked
    /// the kernel not to perform.
    ///
    /// A role's *first* apply is unaffected: a newly registered binding has no recorded work, so it
    /// answers `Nothing` and reaches `WaitingRole::Hardware` as before. This state is recorded
    /// only after a departure or a report-only session loss.
    ReapplyRequestOwed,
    /// The saved configuration was discarded at the departure, so no request can restart this role
    /// — only registering a binding carrying a fresh configuration will.
    ///
    /// Recorded on departure for `crate::RecoveryPolicy::Forget`, which asked the kernel to drop
    /// the value. It is a distinct state from `Self::ReapplyRequestOwed` because the two are owed
    /// different things: triggering `crate::ReapplyConfiguration` against a role in this state does
    /// nothing, there being nothing left to send.
    RegistrationOwed,
}

impl WaitingWork {
    /// Whether the kernel is holding this role until application code acts.
    ///
    /// Both owed states answer `true` — they differ in which move clears the hold, not in whether
    /// the kernel starts anything meanwhile. A caller that needs only whether a role may start
    /// reads this, so a later third owed state cannot be missed at one of those call sites.
    #[must_use]
    pub const fn holds_for_application(self) -> bool {
        matches!(self, Self::ReapplyRequestOwed | Self::RegistrationOwed)
    }
}

const fn default_transition_sequence() -> u64 { 0 }

/// Monotonic order attached to lifecycle handoff entries.
///
/// This sequence orders changes submitted before a frame drain; it is not a historical log and
/// consumers cannot use it to recover work after the bounded handoff releases an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Reflect)]
#[reflect(opaque)]
pub(crate) struct BindingTransitionSequence(u64);

/// One accepted binding lifecycle change awaiting the next frame's internal processing.
#[derive(Debug, PartialEq, Eq, Reflect)]
pub(crate) enum BindingTransition {
    /// A newly registered role needs a binding entity during the next lifecycle stage.
    Registered {
        /// Ordering number for this accepted operation.
        sequence: BindingTransitionSequence,
        /// Authored role whose binding was registered.
        role:     RoleKey,
    },
    /// A replacement displaced a prior binding whose in-flight work is handled later.
    Replaced {
        /// Ordering number for this accepted operation.
        sequence:         BindingTransitionSequence,
        /// Authored role whose binding was replaced.
        role:             RoleKey,
        /// Role entity retained by the displaced registration.
        displaced_entity: Entity,
        /// Durable endpoint authorized by the displaced registration.
        endpoint:         DeviceEndpoint,
        /// Driver work retained from the displaced registration.
        cleanup:          DriverCleanup,
    },
    /// A retired role needs entity cleanup and any later attempt-abort processing.
    Retired {
        /// Ordering number for this accepted operation.
        sequence: BindingTransitionSequence,
        /// Authored role whose binding was retired.
        role:     RoleKey,
        /// Exact address the retired role had authorized before its indexes were removed.
        endpoint: DeviceEndpoint,
        /// Role entity whose kernel-owned registration lifetime ended.
        entity:   Entity,
        /// Driver work retained from the retired registration.
        cleanup:  DriverCleanup,
    },
}

/// Driver callback owed by a binding replacement or retirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum DriverCleanup {
    /// The displaced registration owned no client operation.
    None,
    /// The driver must cancel one applying operation.
    Applying {
        driver:     DriverId,
        attempt:    AttemptRef,
        generation: BindingGeneration,
    },
    /// The driver must release one established session.
    Established {
        driver:  DriverId,
        session: SessionRef,
    },
}

struct PendingBindingTransitions {
    capacity: NonZeroUsize,
    queue:    VecDeque<BindingTransition>,
}

impl Default for PendingBindingTransitions {
    fn default() -> Self {
        Self {
            capacity: NonZeroUsize::new(DEFAULT_PENDING_TRANSITION_CAPACITY)
                .unwrap_or(NonZeroUsize::MIN),
            queue:    VecDeque::new(),
        }
    }
}

impl PendingBindingTransitions {
    fn has_capacity(&self) -> bool { self.queue.len() < self.capacity.get() }

    fn push(&mut self, binding_transition: BindingTransition) {
        self.queue.push_back(binding_transition);
    }
}

impl Bindings {
    /// Select the only lifecycle-safe operation for an availability wait projection.
    pub(crate) fn availability_wait_action(
        &self,
        role: &RoleKey,
        target: AvailabilityWaitTarget,
    ) -> AvailabilityWaitAction {
        let Some(registered_role) = self.by_role.get(role) else {
            return AvailabilityWaitAction::PreserveLifecycle;
        };
        match (&registered_role.state, target) {
            (RoleState::Waiting(_), _)
            | (RoleState::Established(_), AvailabilityWaitTarget::Unavailable) => {
                AvailabilityWaitAction::SetWait
            },
            (RoleState::Applying(_), AvailabilityWaitTarget::Unavailable) => {
                AvailabilityWaitAction::StageForApplyingRole
            },
            (
                RoleState::Applying(_) | RoleState::Established(_),
                AvailabilityWaitTarget::Present,
            )
            | (RoleState::Stopped(_), _) => AvailabilityWaitAction::PreserveLifecycle,
        }
    }

    pub(crate) const fn install_role_status_clock(&mut self, runtime_clock: RiggingRuntimeClock) {
        self.role_status_clock = RoleStatusClock::Installed(runtime_clock);
    }

    pub(crate) fn projected_status(&self, role: &RoleKey) -> Result<RoleStatusView, BindingError> {
        let registered_role = self
            .by_role
            .get(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        let RoleStatusClock::Installed(runtime_clock) = self.role_status_clock else {
            return Err(BindingError::RoleStatusClockNotInstalled { role: role.clone() });
        };
        Ok(project_status(registered_role, runtime_clock))
    }

    pub(crate) fn take_status_changes(&mut self) -> Vec<RoleStatusChange> {
        std::mem::take(&mut self.pending_status_changes)
    }

    pub(crate) const fn has_status_changes(&self) -> bool {
        !self.pending_status_changes.is_empty()
    }

    pub(crate) fn wait_bound_requires_update(&self, now: Instant) -> bool {
        self.by_role.0.values().any(|registered_role| {
            let RoleState::Waiting(waiting_state) = &registered_role.state else {
                return false;
            };
            waiting_state.bound_state == WaitBoundState::Within
                && wait_bound_crossed(
                    waiting_state.wait_start,
                    now,
                    waiting_state.published_bound(),
                )
        })
    }

    pub(crate) fn cross_wait_bounds(&mut self, now: Instant) -> Vec<RoleKey> {
        let mut waits = Vec::with_capacity(self.by_role.0.len());
        for (role, registered_role) in &self.by_role.0 {
            match &registered_role.state {
                RoleState::Waiting(waiting_state) => {
                    waits.push((role.clone(), waiting_state.condition.clone()));
                },
                RoleState::Applying(_) | RoleState::Established(_) | RoleState::Stopped(_) => {},
            }
        }
        waits
            .into_iter()
            .filter_map(|(role, condition)| {
                (self.set_wait(&role, now, condition) == WaitTransition::BoundCrossed)
                    .then_some(role)
            })
            .collect()
    }

    fn mutate_status<Output>(
        &mut self,
        role: &RoleKey,
        mutation: impl FnOnce(&mut RegisteredRole) -> Output,
    ) -> Option<Output> {
        let runtime_clock = match self.role_status_clock {
            RoleStatusClock::NotInstalled => None,
            RoleStatusClock::Installed(runtime_clock) => Some(runtime_clock),
        };
        let (output, status_change) = {
            let registered_role = self.by_role.get_mut(role)?;
            let before = runtime_clock.map(|clock| project_status(registered_role, clock));
            let output = mutation(registered_role);
            let status_change = before.and_then(|from| {
                let to = project_status(registered_role, runtime_clock?);
                (from != to).then(|| RoleStatusChange {
                    role: role.clone(),
                    from,
                    to,
                })
            });
            (output, status_change)
        };
        if let Some(status_change) = status_change {
            self.pending_status_changes.push(status_change);
        }
        Some(output)
    }

    fn set_role_state(&mut self, role: &RoleKey, state: RoleState) {
        let _ = self.mutate_status(role, |registered_role| {
            registered_role.state = state;
        });
    }

    fn reserve_registration(
        &self,
        binding: &Binding,
    ) -> Result<ReservedBindingTransition, BindingError> {
        if self.by_role.contains_key(&binding.role) {
            return Err(BindingError::RoleAlreadyBound {
                role: binding.role.clone(),
            });
        }
        if let Some(owner) = self.owner_by_endpoint.get(&binding.endpoint) {
            return Err(BindingError::EndpointAlreadyOwned {
                endpoint: binding.endpoint.clone(),
                owner:    owner.clone(),
            });
        }
        self.reserve_transition()
    }

    fn register_reserved(
        &mut self,
        binding: Binding,
        entity: Entity,
        reserved_transition: ReservedBindingTransition,
    ) {
        let role = binding.role.clone();
        let endpoint = binding.endpoint.clone();
        let device_key = endpoint.device.clone();
        self.owner_by_endpoint.insert(endpoint, role.clone());
        self.roles_by_device
            .entry(device_key)
            .or_default()
            .push(role.clone());
        let generation = self.allocate_generation();
        self.by_role.insert(
            role.clone(),
            RegisteredRole::registered(binding, entity, generation),
        );
        self.enqueue(BindingTransitionKind::Registered, role, reserved_transition);
    }

    #[cfg(test)]
    pub(crate) fn register(&mut self, binding: Binding) -> Result<(), BindingError> {
        let reserved_transition = self.reserve_registration(&binding)?;
        self.register_reserved(binding, Entity::PLACEHOLDER, reserved_transition);
        Ok(())
    }

    /// Replace one binding only after proving its new endpoint is not owned by another role.
    ///
    /// The old endpoint remains owned until the proposed endpoint and transition handoff both
    /// pass their checks, so an error cannot leave a role unbound or corrupt either reverse index.
    ///
    /// # Errors
    ///
    /// Returns `BindingError` when no old binding exists, a different role owns the proposed
    /// endpoint, or the transition handoff has no space for this replacement.
    pub(crate) fn replace(&mut self, binding: Binding) -> Result<Binding, BindingError> {
        let old_binding =
            self.by_role
                .get(&binding.role)
                .ok_or_else(|| BindingError::RoleNotBound {
                    role: binding.role.clone(),
                })?;
        if let Some(owner) = self.owner_by_endpoint.get(&binding.endpoint)
            && owner != &binding.role
        {
            return Err(BindingError::EndpointAlreadyOwned {
                endpoint: binding.endpoint,
                owner:    owner.clone(),
            });
        }
        let reserved_transition = self.reserve_transition()?;

        let role = binding.role.clone();
        let displaced_entity = old_binding.entity;
        let cleanup = old_binding.driver_cleanup();
        let old_endpoint = old_binding.endpoint.clone();
        let new_endpoint = binding.endpoint.clone();
        let generation = self.allocate_generation();
        let displaced = self
            .by_role
            .insert(
                role.clone(),
                RegisteredRole::registered(binding, displaced_entity, generation),
            )
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;

        if old_endpoint != new_endpoint {
            let new_device_key = new_endpoint.device.clone();
            self.owner_by_endpoint.remove(&old_endpoint);
            self.remove_role_from_device(&old_endpoint.device, &role);
            self.owner_by_endpoint.insert(new_endpoint, role.clone());
            self.roles_by_device
                .entry(new_device_key)
                .or_default()
                .push(role.clone());
        }
        self.enqueue(
            BindingTransitionKind::Replaced {
                displaced_entity,
                endpoint: old_endpoint,
                cleanup,
            },
            role,
            reserved_transition,
        );

        Ok(displaced.binding)
    }

    /// Replace one role through the same typed driver-configuration boundary as registration.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError`] when the role is not registered, the endpoint conflicts, or the
    /// lifecycle handoff cannot accept the replacement.
    pub fn replace_authoring<Configuration>(
        &mut self,
        authoring: BindingAuthoring<Configuration>,
    ) -> Result<Binding, BindingError>
    where
        Configuration: Reflect,
    {
        self.replace(authoring.erase())
    }

    /// Move one role's endpoint onto an adopted device key, keeping the endpoint's own address.
    ///
    /// The adoption path for `crate::IdentityDecisions`: a human has decided the unit that arrived
    /// into the departed one's attachment *is* the unit this role should address, and the durable
    /// key indexed in `Self::owner_by_endpoint` and `Self::roles_by_device` has to move with that
    /// decision. Rewriting only the saved key elsewhere would leave the role resolving
    /// `crate::DeviceResolution::NotResolved` for good.
    ///
    /// This applies `Self::replace`'s ownership rule without asking for a whole replacement
    /// `Binding`: the value is not `Clone`, so an adoption that had to hand one over could not keep
    /// the role's authored request and last-known-good configuration. Everything else follows
    /// `Self::replace` — the role goes back to its private waiting state and its per-role failure
    /// and readability history is dropped, because that history describes the unit the role is no
    /// longer addressing.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::EndpointAlreadyOwned` when another role already holds the adopted
    /// endpoint, `BindingError::RoleNotBound` when the role was retired, and
    /// `BindingError::PendingTransitionCapacityReached` when the transition handoff is full.
    pub(crate) fn readdress(
        &mut self,
        role: &RoleKey,
        device: DeviceKey,
    ) -> Result<(), BindingError> {
        let binding = self
            .by_role
            .get(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        let old_endpoint = binding.endpoint.clone();
        let new_endpoint = DeviceEndpoint {
            device,
            id: old_endpoint.id.clone(),
        };
        if let Some(owner) = self.owner_by_endpoint.get(&new_endpoint)
            && owner != role
        {
            return Err(BindingError::EndpointAlreadyOwned {
                endpoint: new_endpoint,
                owner:    owner.clone(),
            });
        }
        if old_endpoint == new_endpoint {
            return Ok(());
        }
        let reserved_transition = self.reserve_transition()?;
        let displaced_entity = binding.entity;
        let cleanup = binding.driver_cleanup();

        let registered_role = self
            .by_role
            .get_mut(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        registered_role.binding.endpoint = new_endpoint.clone();
        let new_device_key = new_endpoint.device.clone();
        self.owner_by_endpoint.remove(&old_endpoint);
        self.remove_role_from_device(&old_endpoint.device, role);
        self.owner_by_endpoint.insert(new_endpoint, role.clone());
        self.roles_by_device
            .entry(new_device_key)
            .or_default()
            .push(role.clone());
        let generation = self.allocate_generation();
        let _ = self.mutate_status(role, |registered_role| {
            registered_role.generation = generation;
            registered_role.application_run = RegistrationApplicationRun::NotStarted;
            registered_role.state = RoleState::Waiting(WaitingState::registered());
        });
        self.enqueue(
            BindingTransitionKind::Replaced {
                displaced_entity,
                endpoint: old_endpoint,
                cleanup,
            },
            role.clone(),
            reserved_transition,
        );

        Ok(())
    }

    /// Checks whether every role on `saved` can move to `adopted` as one operation.
    ///
    /// Camera clones use different endpoint parts on one physical device. Checking the
    /// complete device address prevents an application from moving one clone while a
    /// conflicting destination or a full lifecycle handoff leaves its siblings behind.
    /// This method changes no binding, index, lifecycle state, or transition sequence.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::EndpointAlreadyOwned` when a destination endpoint belongs to a
    /// role that is not moving, `BindingError::PendingTransitionCapacityReached` when all role
    /// replacements do not fit together, or `BindingError::TransitionSequenceExhausted` when
    /// the complete replacement set cannot receive unique transition identities.
    pub fn validate_device_readdress(
        &self,
        saved: &DeviceKey,
        adopted: &DeviceKey,
    ) -> Result<(), BindingError> {
        self.prepare_device_readdress(saved, adopted).map(|_| ())
    }

    /// Moves every role on `saved` to the corresponding endpoint on `adopted` atomically.
    ///
    /// Each role keeps its endpoint part, driver, requested configuration, and last-known-good
    /// configuration. The roles return to their private waiting state, and their device-specific
    /// attempt, retry, and readability state is cleared just as it is for `Self::readdress`.
    /// A device with no roles is a successful no-op.
    ///
    /// # Errors
    ///
    /// Returns the same preflight errors as `Self::validate_device_readdress`. Every check and
    /// every transition reservation completes before the first binding or index is changed.
    pub fn readdress_device(
        &mut self,
        saved: &DeviceKey,
        adopted: DeviceKey,
    ) -> Result<(), BindingError> {
        let prepared = self.prepare_device_readdress(saved, &adopted)?;
        if prepared.is_empty() {
            return Ok(());
        }
        let roles = prepared
            .iter()
            .map(|readdress| readdress.role.clone())
            .collect::<Vec<_>>();
        for readdress in prepared {
            let Some(registered_role) = self.by_role.get_mut(&readdress.role) else {
                continue;
            };
            let displaced_entity = registered_role.entity;
            let cleanup = registered_role.driver_cleanup();
            registered_role.binding.endpoint = readdress.adopted_endpoint.clone();
            self.owner_by_endpoint.remove(&readdress.saved_endpoint);
            self.owner_by_endpoint
                .insert(readdress.adopted_endpoint, readdress.role.clone());
            let generation = self.allocate_generation();
            let _ = self.mutate_status(&readdress.role, |registered_role| {
                registered_role.generation = generation;
                registered_role.application_run = RegistrationApplicationRun::NotStarted;
                registered_role.state = RoleState::Waiting(WaitingState::registered());
            });
            self.enqueue(
                BindingTransitionKind::Replaced {
                    displaced_entity,
                    endpoint: readdress.saved_endpoint,
                    cleanup,
                },
                readdress.role,
                readdress.reserved_transition,
            );
        }
        self.roles_by_device.remove(saved);
        self.roles_by_device
            .entry(adopted)
            .or_default()
            .extend(roles);
        Ok(())
    }

    fn prepare_device_readdress(
        &self,
        saved: &DeviceKey,
        adopted: &DeviceKey,
    ) -> Result<Vec<PreparedDeviceReaddress>, BindingError> {
        if saved == adopted {
            return Ok(Vec::new());
        }
        let roles = self.roles_by_device.get(saved).cloned().unwrap_or_default();
        for role in &roles {
            let binding = self
                .by_role
                .get(role)
                .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
            let adopted_endpoint = DeviceEndpoint {
                device: adopted.clone(),
                id:     binding.endpoint.id.clone(),
            };
            if let Some(owner) = self.owner_by_endpoint.get(&adopted_endpoint)
                && owner != role
            {
                return Err(BindingError::EndpointAlreadyOwned {
                    endpoint: adopted_endpoint,
                    owner:    owner.clone(),
                });
            }
        }
        let reservations = self.reserve_transitions(roles.len())?;
        roles
            .into_iter()
            .zip(reservations)
            .map(|(role, reserved_transition)| {
                let binding = self
                    .by_role
                    .get(&role)
                    .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
                Ok(PreparedDeviceReaddress {
                    adopted_endpoint: DeviceEndpoint {
                        device: adopted.clone(),
                        id:     binding.endpoint.id.clone(),
                    },
                    saved_endpoint: binding.endpoint.clone(),
                    role,
                    reserved_transition,
                })
            })
            .collect()
    }

    /// Report which other role, if any, already holds the endpoint an adoption would move `role`
    /// onto.
    ///
    /// `crate::IdentityDecisions` caches this answer on each standing question, because application
    /// code answering a question holds only that resource and an adoption that quietly took an
    /// endpoint from another role is the outcome the register must never produce. A role that is
    /// unbound, or that already owns the endpoint itself, reads as `EndpointOwner::Unowned`:
    /// neither is a conflict an operator has to resolve.
    pub(crate) fn candidate_endpoint_owner(
        &self,
        role: &RoleKey,
        candidate: &DeviceKey,
    ) -> EndpointOwner {
        let Some(binding) = self.by_role.get(role) else {
            return EndpointOwner::Unowned;
        };
        let candidate_endpoint = DeviceEndpoint {
            device: candidate.clone(),
            id:     binding.endpoint.id.clone(),
        };

        self.owner_by_endpoint
            .get(&candidate_endpoint)
            .filter(|owner| *owner != role)
            .map_or(EndpointOwner::Unowned, |owner| {
                EndpointOwner::OwnedBy(owner.clone())
            })
    }

    /// Retire an authored role and remove every ownership index entry that selected it.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::PendingTransitionCapacityReached` when this effective retirement
    /// cannot be retained for later lifecycle processing. Retiring a role that is already absent
    /// succeeds with `RetirementOutcome::AlreadyUnbound` and creates no transition.
    pub fn retire(&mut self, role: &RoleKey) -> Result<RetirementOutcome, BindingError> {
        if !self.by_role.contains_key(role) {
            return Ok(RetirementOutcome::AlreadyUnbound);
        }
        let reserved_transition = self.reserve_transition()?;

        let registered_role = self
            .by_role
            .remove(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        let cleanup = registered_role.driver_cleanup();
        self.owner_by_endpoint
            .remove(&registered_role.binding.endpoint);
        self.remove_role_from_device(&registered_role.binding.endpoint.device, role);
        self.enqueue(
            BindingTransitionKind::Retired {
                endpoint: registered_role.binding.endpoint.clone(),
                entity: registered_role.entity,
                cleanup,
            },
            role.clone(),
            reserved_transition,
        );

        Ok(RetirementOutcome::Retired(Box::new(
            registered_role.binding,
        )))
    }

    /// Read the generation of the binding currently installed under `role`.
    ///
    /// `None` means the role has no binding at all — retired or never registered. Callers compare
    /// the answer with an attempt's stamped [`BindingGeneration`]; equality means the attempt was
    /// dispatched for the binding standing now.
    pub(crate) fn generation(&self, role: &RoleKey) -> Option<BindingGeneration> {
        self.by_role
            .get(role)
            .map(|registered_role| registered_role.generation)
    }

    /// Allocates the next binding generation from the process-lifetime counter.
    ///
    /// Never reused while the process runs, so an attempt stamped under a superseded binding can
    /// never collide with a later installation under the same role name.
    const fn allocate_generation(&mut self) -> BindingGeneration {
        self.next_generation += 1;
        BindingGeneration(self.next_generation)
    }

    const fn allocate_session(&mut self) -> SessionRef {
        self.next_session = self.next_session.saturating_add(1);
        SessionRef::new(self.next_session)
    }

    pub(crate) const fn issue_session(&mut self) -> SessionRef { self.allocate_session() }

    pub(crate) fn active_attempts(&self) -> Vec<ActiveAttemptRecord> {
        self.by_role
            .0
            .iter()
            .filter_map(|(role, registered_role)| {
                let RoleState::Applying(active_apply) = &registered_role.state else {
                    return None;
                };
                Some(ActiveAttemptRecord {
                    role:    role.clone(),
                    entity:  registered_role.entity,
                    driver:  registered_role.binding.driver,
                    attempt: active_apply.attempt.clone(),
                })
            })
            .collect()
    }

    pub(crate) fn established_sessions(&self) -> Vec<EstablishedSessionRecord> {
        self.by_role
            .0
            .iter()
            .filter_map(|(role, registered_role)| {
                let RoleState::Established(session) = &registered_role.state else {
                    return None;
                };
                Some(EstablishedSessionRecord {
                    role:    role.clone(),
                    driver:  registered_role.binding.driver,
                    session: session.session,
                })
            })
            .collect()
    }

    pub(crate) fn established_session(
        &self,
        role: &RoleKey,
        session: SessionRef,
    ) -> Option<EstablishedSessionRecord> {
        let registered_role = self.by_role.get(role)?;
        let RoleState::Established(established) = &registered_role.state else {
            return None;
        };
        (established.session == session).then(|| EstablishedSessionRecord {
            role: role.clone(),
            driver: registered_role.binding.driver,
            session,
        })
    }

    /// Report whether this update has testimony, a strict expiry crossing, or an owed stalled
    /// release to apply. Healthy sessions inside their interval remain read-only.
    pub(crate) fn continuous_flow_update_due(&self, now: FrameClockReading) -> bool {
        self.by_role.0.values().any(|registered_role| {
            let RoleState::Established(established) = &registered_role.state else {
                return false;
            };
            if !established.flow.is_monitored() {
                return false;
            }
            established.datum_arrivals.has_pending()
                || match now {
                    FrameClockReading::Measurable(now) => established.flow.judgment_due(now),
                    FrameClockReading::NotYetAdvanced => false,
                }
        })
    }

    /// Credit every current lease and select sessions whose previously published stall is owed a
    /// release. A newly crossed session remains established for this update.
    ///
    /// Crediting testimony rewrites only kernel-private flow bookkeeping, and at data rates it
    /// happens on nearly every frame. The returned judgment is what tells the caller whether this
    /// update actually moved published state, so a credit-only frame leaves the binding register's
    /// change tick alone.
    pub(crate) fn judge_continuous_flow(
        &mut self,
        now: FrameClockReading,
    ) -> ContinuousFlowJudgment {
        let RoleStatusClock::Installed(runtime_clock) = self.role_status_clock else {
            return ContinuousFlowJudgment::NothingPublished;
        };
        let published_before = self.pending_status_changes.len();
        let mut release_required = Vec::new();
        for (role, registered_role) in &mut self.by_role.0 {
            let RoleState::Established(established) = &registered_role.state else {
                continue;
            };
            if !established.flow.is_monitored() {
                continue;
            }
            let before = project_status(registered_role, runtime_clock);
            let RoleState::Established(established) = &mut registered_role.state else {
                continue;
            };
            established.credit_latest_datum_arrival();
            let expiry = match now {
                FrameClockReading::Measurable(now) => established.flow.evaluate_expiry(now),
                FrameClockReading::NotYetAdvanced => EstablishedFlowExpiry::Current,
            };
            if let EstablishedFlowExpiry::ReleaseRequired(_) = expiry {
                release_required.push(EstablishedSessionRecord {
                    role:    role.clone(),
                    driver:  registered_role.binding.driver,
                    session: established.session,
                });
            }
            let to = project_status(registered_role, runtime_clock);
            if before != to {
                self.pending_status_changes.push(RoleStatusChange {
                    role: role.clone(),
                    from: before,
                    to,
                });
            }
        }
        if self.pending_status_changes.len() == published_before && release_required.is_empty() {
            return ContinuousFlowJudgment::NothingPublished;
        }
        ContinuousFlowJudgment::Published { release_required }
    }

    pub(crate) fn active_attempt(
        &self,
        role: &RoleKey,
        attempt: AttemptRef,
    ) -> Option<ActiveAttemptRecord> {
        self.active_attempts()
            .into_iter()
            .find(|active| active.role == *role && active.attempt.reference() == attempt)
    }

    pub(crate) fn active_configuration(
        &self,
        role: &RoleKey,
        attempt: AttemptRef,
    ) -> Result<&dyn Reflect, BindingError> {
        let registered_role = self
            .by_role
            .get(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        let RoleState::Applying(active_apply) = &registered_role.state else {
            return Err(BindingError::RoleNotApplying { role: role.clone() });
        };
        if active_apply.attempt.reference() != attempt {
            return Err(BindingError::RoleNotApplying { role: role.clone() });
        }
        active_apply.source.configuration(&registered_role.binding)
    }

    pub(crate) fn accept_success(
        &mut self,
        role: &RoleKey,
        attempt: AttemptRef,
        established_at: Instant,
        session: SessionRef,
        datum_arrivals: SessionDatumArrivalReceiver,
        applied: ErasedApplied,
    ) -> bool {
        self.mutate_status(role, move |registered_role| {
            let replaced = std::mem::replace(
                &mut registered_role.state,
                RoleState::Waiting(WaitingState::registered()),
            );
            let RoleState::Applying(active_apply) = replaced else {
                registered_role.state = replaced;
                return false;
            };
            if active_apply.attempt.reference() != attempt {
                registered_role.state = RoleState::Applying(active_apply);
                return false;
            }

            let applied_kind = match applied {
                ErasedApplied::AsDispatched => {
                    if active_apply.source == ApplyConfigurationSource::Requested {
                        registered_role.binding.last_known_good =
                            LastKnownGoodConfiguration::MatchesRequested;
                    }
                    AppliedKind::AsDispatched
                },
                ErasedApplied::DiffersFromDispatched(configuration) => {
                    registered_role.binding.last_known_good =
                        LastKnownGoodConfiguration::DiffersFromDispatched(configuration);
                    AppliedKind::DiffersFromDispatched
                },
            };
            registered_role.application_run.end(AttemptEnding::Reported(
                DriverOutcomeStatus::Succeeded(applied_kind),
            ));
            let policy = registered_role.binding.policy();
            registered_role.state = RoleState::Established(EstablishedSession::new(
                active_apply.source,
                established_at,
                session,
                applied_kind,
                EstablishingAttemptLookup::EstablishedBy(attempt),
                policy,
                EstablishedSessionDatumArrivals::Lease(datum_arrivals),
            ));
            true
        })
        .unwrap_or(false)
    }

    pub(crate) fn update_established_configuration(
        &mut self,
        role: &RoleKey,
        session: SessionRef,
        configuration: Box<dyn Reflect>,
    ) -> bool {
        self.mutate_status(role, move |registered_role| {
            let RoleState::Established(established) = &mut registered_role.state else {
                return false;
            };
            if established.session != session {
                return false;
            }
            established.applied = AppliedKind::DiffersFromDispatched;
            registered_role.binding.last_known_good =
                LastKnownGoodConfiguration::DiffersFromDispatched(configuration);
            true
        })
        .unwrap_or(false)
    }

    pub(crate) fn start_apply(
        &mut self,
        role: &RoleKey,
        attempt: AuthorizedApplyAttempt,
        source: ApplyConfigurationSource,
    ) {
        let _ = self.mutate_status(role, |registered_role| {
            let replaced = std::mem::replace(
                &mut registered_role.state,
                RoleState::Waiting(WaitingState::registered()),
            );
            let (failures_before_attempt, continuation) = match replaced {
                RoleState::Waiting(waiting_state) => {
                    let failures = waiting_state.failures.clone();
                    let continuation = if waiting_state.waiting_work == WaitingWork::Nothing {
                        ApplyContinuation::Establish
                    } else {
                        ApplyContinuation::Wait(waiting_state)
                    };
                    (failures, continuation)
                },
                RoleState::Established(_) => {
                    (RoleApplyFailureRun::Clear, ApplyContinuation::Establish)
                },
                RoleState::Applying(_) | RoleState::Stopped(_) => {
                    registered_role.state = replaced;
                    return;
                },
            };
            registered_role.application_run.start();
            registered_role.state = RoleState::Applying(Box::new(ActiveApply {
                attempt,
                source,
                failures_before_attempt,
                continuation,
            }));
        });
    }

    /// Borrow one binding without exposing a mutable path around its role lifecycle views.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::RoleNotBound` when the role has no retained authored binding.
    pub fn binding(&self, role: &RoleKey) -> Result<&Binding, BindingError> {
        self.by_role
            .get(role)
            .map(|registered_role| &registered_role.binding)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })
    }

    /// Return the entity owned by a registered role.
    ///
    /// Client runtime code should retain the entity returned by registration in its relationship;
    /// this accessor supports diagnostics and test fixtures without recreating a side index.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError::RoleNotBound`] when the role is not registered.
    pub fn role_entity(&self, role: &RoleKey) -> Result<Entity, BindingError> {
        self.by_role
            .get(role)
            .map(|registered_role| registered_role.entity)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })
    }

    /// Report whether a retained role routes through this typed driver registration.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError::RoleNotBound`] when the role is not registered.
    pub fn is_routed_by<Configuration>(
        &self,
        role: &RoleKey,
        driver: EndpointDriverRegistration<Configuration>,
    ) -> Result<bool, BindingError> {
        self.binding(role)
            .map(|binding| binding.driver == driver.driver_id())
    }

    /// Iterate the retained roles routed through one typed driver registration.
    pub fn roles_routed_by<Configuration>(
        &self,
        driver: EndpointDriverRegistration<Configuration>,
    ) -> impl Iterator<Item = &RoleKey> {
        self.by_role.0.iter().filter_map(move |(role, registered)| {
            (registered.binding.driver == driver.driver_id()).then_some(role)
        })
    }

    pub(crate) fn registered_role_entities(&self) -> impl Iterator<Item = (&RoleKey, Entity)> {
        self.by_role
            .0
            .iter()
            .map(|(role, registered_role)| (role, registered_role.entity))
    }

    fn replace_role_entity(&mut self, role: &RoleKey, replacement: Entity) {
        if let Some(registered_role) = self.by_role.get_mut(role) {
            registered_role.entity = replacement;
        }
    }

    /// Create or update the wait retained for `role` at one schedule instant.
    pub(crate) fn set_wait(
        &mut self,
        role: &RoleKey,
        now: Instant,
        condition: WaitingCondition,
    ) -> WaitTransition {
        self.mutate_status(role, |registered_role| {
            let replaced = std::mem::replace(
                &mut registered_role.state,
                RoleState::Waiting(WaitingState::registered()),
            );
            let (waiting_state, wait_transition) =
                waiting_state_from_lifecycle(replaced, now, condition);
            registered_role.state = RoleState::Waiting(waiting_state);
            wait_transition
        })
        .unwrap_or(WaitTransition::Unchanged)
    }

    /// Report whether installing `condition` would change the retained wait or cross its bound.
    pub(crate) fn wait_requires_update(
        &self,
        role: &RoleKey,
        now: Instant,
        condition: &WaitingCondition,
    ) -> bool {
        let Some(RegisteredRole {
            state: RoleState::Waiting(waiting_state),
            ..
        }) = self.by_role.get(role)
        else {
            return true;
        };
        if !waiting_state.condition.same_identity(condition) {
            return true;
        }
        waiting_state.bound_state == WaitBoundState::Within
            && wait_bound_crossed(
                waiting_state.wait_start,
                now,
                waiting_state.condition.bound(),
            )
    }

    #[cfg(test)]
    pub(crate) fn record_test_reporter_wait(
        &mut self,
        role: &RoleKey,
        key: DeviceKey,
        reporter: ReporterId,
        now: Instant,
        deadline: Instant,
    ) -> WaitTransition {
        self.set_wait(
            role,
            now,
            WaitingCondition::Reporter(ReporterWait::AwaitingFirstReport(TimedReporterWait {
                key,
                reporters: NonEmptyReporterIds {
                    first: reporter,
                    rest:  Vec::new(),
                },
                bound: WaitBoundInput::Until(deadline),
            })),
        )
    }

    fn role_apply_failure_run(&self, role: &RoleKey) -> RoleApplyFailureRun {
        let Some(registered_role) = self.by_role.get(role) else {
            return RoleApplyFailureRun::Clear;
        };
        match &registered_role.state {
            RoleState::Waiting(waiting_state) => waiting_state.failures.clone(),
            RoleState::Applying(active_apply) => active_apply.failures_before_attempt.clone(),
            RoleState::Stopped(StoppedState::RepeatedFailures {
                failures,
                last_ending,
                ..
            }) => RoleApplyFailureRun::Consecutive {
                failures:    *failures,
                last_ending: last_ending.clone(),
            },
            RoleState::Established(_) | RoleState::Stopped(StoppedState::Unsupported { .. }) => {
                RoleApplyFailureRun::Clear
            },
        }
    }

    fn set_role_apply_failure_run(&mut self, role: &RoleKey, failures: RoleApplyFailureRun) {
        let _ = self.mutate_status(role, |registered_role| match &mut registered_role.state {
            RoleState::Waiting(waiting_state) => {
                waiting_state.failures = failures;
            },
            RoleState::Applying(active_apply) => {
                active_apply.failures_before_attempt = failures;
            },
            RoleState::Established(_) | RoleState::Stopped(_) => {},
        });
    }

    fn set_role_retry_pacing(&mut self, role: &RoleKey, retry_pacing: RetryPacing) {
        let _ = self.mutate_status(role, |registered_role| match &mut registered_role.state {
            RoleState::Waiting(waiting_state) => {
                waiting_state.retry_pacing = retry_pacing;
                waiting_state.revision_retry_bound = waiting_state.condition.bound();
            },
            RoleState::Applying(_) | RoleState::Established(_) | RoleState::Stopped(_) => {},
        });
    }

    fn finish_successful_apply(
        &mut self,
        role: &RoleKey,
        established_at: Instant,
        establishing_attempt: EstablishingAttemptLookup,
        applied: AppliedKind,
    ) {
        let session = self.allocate_session();
        let _ = self.mutate_status(role, |registered_role| {
            let replaced = std::mem::replace(
                &mut registered_role.state,
                RoleState::Waiting(WaitingState::registered()),
            );
            let RoleState::Applying(active_apply) = replaced else {
                registered_role.state = replaced;
                return;
            };

            let restoration_completed =
                active_apply.source == ApplyConfigurationSource::LastKnownGood;
            if let ApplyContinuation::Wait(mut waiting_state)
            | ApplyContinuation::DeviceUnavailable(mut waiting_state) = active_apply.continuation
                && !(restoration_completed
                    && waiting_state.waiting_work == WaitingWork::RestorationOwed)
            {
                waiting_state.failures = RoleApplyFailureRun::Clear;
                waiting_state.retry_pacing = RetryPacing::Ready;
                registered_role.state = RoleState::Waiting(waiting_state);
                return;
            }

            let policy = registered_role.binding.policy();
            registered_role.state = RoleState::Established(EstablishedSession::new(
                active_apply.source,
                established_at,
                session,
                applied,
                establishing_attempt,
                policy,
                EstablishedSessionDatumArrivals::NoLeaseIssued,
            ));
        });
    }

    fn record_successful_apply_ending(
        &mut self,
        role: &RoleKey,
        now: FrameClockReading,
        session_authority: SuccessfulApplySessionAuthority,
    ) {
        self.set_role_apply_failure_run(role, RoleApplyFailureRun::Clear);
        let Some(registered_role) = self.by_role.get(role) else {
            return;
        };
        let RoleState::Applying(active_apply) = &registered_role.state else {
            return;
        };
        let established_at = match now {
            FrameClockReading::Measurable(schedule_now) => schedule_now,
            FrameClockReading::NotYetAdvanced => active_apply.attempt.started_at,
        };
        let establishing_attempt = match session_authority {
            SuccessfulApplySessionAuthority::EstablishingAttempt => {
                EstablishingAttemptLookup::EstablishedBy(active_apply.attempt.reference)
            },
            SuccessfulApplySessionAuthority::NoEstablishingAttempt => {
                EstablishingAttemptLookup::NotEstablished
            },
        };
        let applied = match session_authority {
            SuccessfulApplySessionAuthority::EstablishingAttempt => AppliedKind::AsDispatched,
            SuccessfulApplySessionAuthority::NoEstablishingAttempt => {
                AppliedKind::DiffersFromDispatched
            },
        };
        self.finish_successful_apply(role, established_at, establishing_attempt, applied);
    }

    fn stop_role_after_repeated_failures(
        &mut self,
        role: &RoleKey,
        failures: NonZeroU32,
        last_ending: AttemptEnding,
    ) {
        self.set_role_state(
            role,
            RoleState::Stopped(StoppedState::RepeatedFailures {
                failures,
                reacquisition: ReacquisitionProgress::StillAvailable,
                last_ending,
            }),
        );
    }

    fn stop_role_for_driver_repair(&mut self, role: &RoleKey, failure: DriverStopReason) {
        self.set_role_state(
            role,
            RoleState::Stopped(StoppedState::Unsupported { failure }),
        );
    }

    /// Iterate every retained role whose endpoint names `device_key`.
    ///
    /// Several roles can address different endpoints of one device, so callers receive every
    /// role instead of a convenient but unsafe first match.
    pub fn roles_for(&self, device_key: &DeviceKey) -> impl Iterator<Item = &RoleKey> {
        self.roles_by_device.get(device_key).into_iter().flatten()
    }

    /// Read what one role is owed while it waits.
    ///
    /// Answers `WaitingWork::Nothing` for a role nobody has recorded work against, including one
    /// that is not waiting at all: owing a restoration is something the kernel records, so an
    /// unrecorded role owes nothing.
    #[must_use]
    pub fn waiting_work(&self, role: &RoleKey) -> WaitingWork {
        self.by_role.get(role).map_or(
            WaitingWork::Nothing,
            |registered_role| match &registered_role.state {
                RoleState::Waiting(waiting_state) => waiting_state.waiting_work,
                RoleState::Applying(active_apply) => match &active_apply.continuation {
                    ApplyContinuation::Establish => WaitingWork::Nothing,
                    ApplyContinuation::Wait(waiting_state)
                    | ApplyContinuation::DeviceUnavailable(waiting_state) => {
                        waiting_state.waiting_work
                    },
                },
                RoleState::Established(_) | RoleState::Stopped(_) => WaitingWork::Nothing,
            },
        )
    }

    /// Return a role whose device departed to its private waiting state.
    ///
    /// Without this the work `Self::set_waiting_work` records is unreachable: `WaitingWork` is only
    /// ever consulted through the waiting view, so a role left established
    /// after its unit left never reaches `WaitingRole::Restoration` or
    /// `WaitingRole::ApplicationRequest`, and every `crate::RecoveryPolicy` variant behaves
    /// identically — the departed unit's return applies nothing at all.
    ///
    /// Only an established role moves, because it is the one state whose meaning the departure
    /// falsified: the role no longer has a present usable unit. Applying is ended by the abort
    /// path, which writes waiting itself; stopped is re-armed by
    /// `Self::observe_stopped_role_endpoint`, which needs the departure to stay visible for one
    /// more pass; and retirement never reactivates.
    pub(crate) fn await_departed_device(&mut self, role: &RoleKey, now: FrameClockReading) {
        let established = self.by_role.get(role).is_some_and(|registered_role| {
            matches!(registered_role.state, RoleState::Established(_))
        });
        if !established {
            return;
        }
        match now {
            FrameClockReading::Measurable(now) => {
                self.set_wait(role, now, WaitingCondition::NewRegistration);
            },
            FrameClockReading::NotYetAdvanced => {
                self.set_role_state(role, RoleState::Waiting(WaitingState::registered()));
            },
        }
    }

    /// Record what one role is owed while it waits.
    pub(crate) fn set_waiting_work(&mut self, role: &RoleKey, waiting_work: WaitingWork) {
        let _ = self.mutate_status(role, |registered_role| match &mut registered_role.state {
            RoleState::Waiting(waiting_state) => {
                waiting_state.waiting_work = waiting_work;
                if matches!(
                    waiting_state.condition,
                    WaitingCondition::ApplicationReapply
                ) || matches!(
                    waiting_work,
                    WaitingWork::ReapplyRequestOwed | WaitingWork::RegistrationOwed
                ) {
                    waiting_state.condition = waiting_condition_for_work(waiting_work);
                    waiting_state.revision_retry_bound = waiting_state.condition.bound();
                    waiting_state.bound_state = WaitBoundState::Within;
                }
            },
            RoleState::Applying(active_apply) => {
                if waiting_work == WaitingWork::Nothing {
                    active_apply.continuation = ApplyContinuation::Establish;
                } else {
                    match &mut active_apply.continuation {
                        ApplyContinuation::Wait(waiting_state)
                        | ApplyContinuation::DeviceUnavailable(waiting_state) => {
                            waiting_state.waiting_work = waiting_work;
                        },
                        ApplyContinuation::Establish => {
                            active_apply.continuation = ApplyContinuation::Wait(WaitingState {
                                failures: active_apply.failures_before_attempt.clone(),
                                retry_pacing: RetryPacing::Ready,
                                revision_retry_bound: waiting_condition_for_work(waiting_work)
                                    .bound(),
                                condition: waiting_condition_for_work(waiting_work),
                                wait_start: WaitStart::At(active_apply.attempt.started_at),
                                bound_state: WaitBoundState::Within,
                                waiting_work,
                            });
                        },
                    }
                }
            },
            RoleState::Established(_) | RoleState::Stopped(_) => {},
        });
    }

    /// Retain the hardware wait that an applying role must enter after cancellation finishes.
    pub(crate) fn stage_device_unavailability_wait(
        &mut self,
        role: &RoleKey,
        now: Instant,
        condition: WaitingCondition,
    ) {
        let _ = self.mutate_status(role, |registered_role| {
            let RoleState::Applying(active_apply) = &mut registered_role.state else {
                return;
            };
            let (failures, waiting_work) = match &active_apply.continuation {
                ApplyContinuation::Establish => (
                    active_apply.failures_before_attempt.clone(),
                    WaitingWork::Nothing,
                ),
                ApplyContinuation::Wait(waiting_state)
                | ApplyContinuation::DeviceUnavailable(waiting_state) => {
                    (waiting_state.failures.clone(), waiting_state.waiting_work)
                },
            };
            active_apply.continuation = ApplyContinuation::DeviceUnavailable(WaitingState {
                failures,
                retry_pacing: RetryPacing::Ready,
                revision_retry_bound: condition.bound(),
                condition,
                wait_start: WaitStart::At(now),
                bound_state: WaitBoundState::Within,
                waiting_work,
            });
        });
    }

    /// Clear a role's owed reapply request by turning it into the restoration it asked for.
    ///
    /// Only `WaitingWork::ReapplyRequestOwed` reaches here; the caller enforces that, because a
    /// role owing a registration had its saved value dropped at the departure and has nothing left
    /// to send. A role with no saved value falls back to `WaitingWork::Nothing`, which lets its
    /// authored request dispatch: the application asked for the endpoint to be driven, and the only
    /// thing left to drive it with is what the application authored.
    pub(crate) fn request_reapply(&mut self, role: &RoleKey) {
        let established = self
            .by_role
            .get(role)
            .is_some_and(|binding| binding.last_known_good.is_established());
        let waiting_work = if established {
            WaitingWork::RestorationOwed
        } else {
            WaitingWork::Nothing
        };
        self.set_waiting_work(role, waiting_work);
    }

    pub(crate) fn forget_last_known_good(&mut self, role: &RoleKey) {
        let _ = self.mutate_status(role, |registered_role| {
            registered_role.binding.last_known_good = LastKnownGoodConfiguration::NotEstablished;
        });
    }

    /// Record a kernel-invalidated attempt without classifying it as a driver outcome.
    ///
    /// A revision advance already supplies the successor attempt's new authorization revision, so
    /// that invalidation returns the role to a dispatch-ready wait. Other invalidations retain the
    /// existing retry pacing, including a device-unavailability wait carried by the active apply.
    pub(crate) fn record_invalidated_attempt_ending(
        &mut self,
        role: &RoleKey,
        ended_generation: BindingGeneration,
        invalidation: AttemptInvalidation,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) {
        if self.generation(role) != Some(ended_generation) {
            return;
        }
        self.record_registration_application_ending(
            role,
            ended_generation,
            AttemptEnding::Invalidated(invalidation),
        );
        if invalidation == AttemptInvalidation::RevisionAdvanced {
            if let FrameClockReading::Measurable(schedule_now) = now {
                self.set_wait(role, schedule_now, WaitingCondition::NewRegistration);
            }
            return;
        }
        self.record_aborted_attempt_ending(role, device_revision, now);
    }

    /// Record a driver-reported attempt outcome and escalate or clear this role's failure run.
    ///
    /// A success clears the count outright — that is what "self-clears on recovery" means.
    pub(crate) fn record_attempt_ending(
        &mut self,
        role: &RoleKey,
        ended_generation: BindingGeneration,
        outcome: DriverOutcomeStatus,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) {
        // A reported outcome may only affect the binding generation that dispatched its attempt. A
        // stale outcome — the role was replaced or retired while the attempt was in flight — must
        // not install a retry gate, count a failure, or clear a run against the binding standing
        // now: `Self::replace` deliberately clears that history, and an outcome that landed
        // afterwards would silently re-pollute it.
        if self.generation(role) != Some(ended_generation) {
            return;
        }
        self.record_registration_application_ending(
            role,
            ended_generation,
            AttemptEnding::Reported(outcome.clone()),
        );
        let schedule_now = match now {
            FrameClockReading::Measurable(schedule_now) => Some(schedule_now),
            FrameClockReading::NotYetAdvanced => None,
        };
        match outcome {
            DriverOutcomeStatus::Succeeded(AppliedKind::AsDispatched) => {
                self.record_successful_apply_ending(
                    role,
                    now,
                    SuccessfulApplySessionAuthority::EstablishingAttempt,
                );
            },
            DriverOutcomeStatus::Succeeded(AppliedKind::DiffersFromDispatched) => {
                self.record_successful_apply_ending(
                    role,
                    now,
                    SuccessfulApplySessionAuthority::NoEstablishingAttempt,
                );
            },
            DriverOutcomeStatus::Aborted(_) => {
                self.record_aborted_attempt_ending(role, device_revision, now);
            },
            DriverOutcomeStatus::Failed(error) => match error.apply_failure_disposition() {
                ApplyFailureDisposition::Reconsider => {
                    self.clear_retry_pacing(role);
                    if let Some(schedule_now) = schedule_now {
                        self.set_wait(role, schedule_now, WaitingCondition::NewRegistration);
                    }
                },
                ApplyFailureDisposition::AwaitClearance => {
                    if let Some(binding) = self.by_role.get(role) {
                        let retry_gate =
                            RetryGate::from_policy(binding.retry, device_revision, now);
                        if let Some(schedule_now) = schedule_now {
                            self.set_wait(
                                role,
                                schedule_now,
                                WaitingCondition::KernelRetry(retry_gate),
                            );
                        }
                        self.set_role_retry_pacing(role, RetryPacing::Blocked(retry_gate));
                    }
                },
                ApplyFailureDisposition::Fault => {
                    let last_ending = AttemptEnding::Reported(DriverOutcomeStatus::Failed(error));
                    let failure_run = self
                        .role_apply_failure_run(role)
                        .incremented(last_ending.clone());
                    let consecutive = failure_run.count();
                    if consecutive >= CONSECUTIVE_FAILURE_LIMIT {
                        let failures = match &failure_run {
                            RoleApplyFailureRun::Consecutive { failures, .. } => *failures,
                            RoleApplyFailureRun::Clear => NonZeroU32::MIN,
                        };
                        self.stop_role_after_repeated_failures(role, failures, last_ending);
                    } else if let Some(binding) = self.by_role.get(role) {
                        let retry_gate =
                            RetryGate::from_policy(binding.retry, device_revision, now);
                        if let Some(schedule_now) = schedule_now {
                            self.set_wait(
                                role,
                                schedule_now,
                                WaitingCondition::KernelRetry(retry_gate),
                            );
                        }
                        self.set_role_apply_failure_run(role, failure_run);
                        self.set_role_retry_pacing(role, RetryPacing::Blocked(retry_gate));
                    }
                },
                ApplyFailureDisposition::Stop(failure) => {
                    self.stop_role_for_driver_repair(role, failure);
                },
            },
        }
    }

    pub(crate) fn record_registration_application_ending(
        &mut self,
        role: &RoleKey,
        ended_generation: BindingGeneration,
        ending: AttemptEnding,
    ) {
        if self.generation(role) != Some(ended_generation) {
            return;
        }
        let _ = self.mutate_status(role, |registered_role| {
            registered_role.application_run.end(ending);
        });
    }

    fn record_aborted_attempt_ending(
        &mut self,
        role: &RoleKey,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) {
        let device_unavailability_wait = self.by_role.get(role).and_then(|registered| {
            let RoleState::Applying(active_apply) = &registered.state else {
                return None;
            };
            let ApplyContinuation::DeviceUnavailable(waiting_state) = &active_apply.continuation
            else {
                return None;
            };
            Some(waiting_state.clone())
        });
        if let Some(waiting_state) = device_unavailability_wait {
            self.set_role_state(role, RoleState::Waiting(waiting_state));
            return;
        }
        if let Some(binding) = self.by_role.get(role) {
            let retry_gate = RetryGate::from_policy(binding.retry, device_revision, now);
            if let FrameClockReading::Measurable(schedule_now) = now {
                self.set_wait(
                    role,
                    schedule_now,
                    WaitingCondition::KernelRetry(retry_gate),
                );
            }
            self.set_role_retry_pacing(role, RetryPacing::Blocked(retry_gate));
        }
    }

    /// Apply an established role's session-loss policy after the caller validates its exact
    /// process-local device handle and revision.
    ///
    /// `Recreate` uses the ordinary waiting path and installs the role's normal retry gate before
    /// apply selection can run. A readable last-known-good value is restored; otherwise the
    /// retained authored request is the only configuration available. `ReportOnly` leaves the
    /// role waiting on application action and installs no retry that could open a replacement.
    pub(crate) fn apply_session_loss(
        &mut self,
        role: &RoleKey,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) -> SessionLossApplication {
        let Some(binding) = self.by_role.get(role) else {
            return SessionLossApplication::BindingAbsent;
        };
        let on_loss = binding.on_loss;
        let retry = binding.retry;
        let has_last_known_good = binding.last_known_good.is_established();
        match on_loss {
            OnSessionLoss::Recreate => {
                let retry_gate = RetryGate::from_policy(retry, device_revision, now);
                if let FrameClockReading::Measurable(schedule_now) = now {
                    self.set_wait(
                        role,
                        schedule_now,
                        WaitingCondition::KernelRetry(retry_gate),
                    );
                }
                self.set_role_retry_pacing(role, RetryPacing::Blocked(retry_gate));
                self.set_waiting_work(
                    role,
                    if has_last_known_good {
                        WaitingWork::RestorationOwed
                    } else {
                        WaitingWork::Nothing
                    },
                );
            },
            OnSessionLoss::ReportOnly => {
                if let FrameClockReading::Measurable(schedule_now) = now {
                    self.set_wait(role, schedule_now, WaitingCondition::ApplicationReapply);
                }
                self.set_waiting_work(role, WaitingWork::ReapplyRequestOwed);
                self.set_role_retry_pacing(role, RetryPacing::Ready);
            },
        }
        SessionLossApplication::Applied
    }

    /// Pace the next dispatch for a role whose apply never reached a working driver.
    ///
    /// An unregistered driver and a configuration-contract mismatch are not attempt failures — no
    /// attempt ran, nothing touched the device — so they neither escalate the role nor clear its
    /// run. They still have to be paced: the role stays waiting, so without a
    /// gate the kernel would re-dispatch and be re-refused on every frame for the life of the
    /// binding, which is the same unbounded retry `crate::RetryOn` exists to stop.
    pub(crate) fn record_dispatch_refused(
        &mut self,
        role: &RoleKey,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
        wait_bound: Instant,
        waiting_condition: WaitingCondition,
    ) -> WaitTransition {
        let Some(binding) = self.by_role.get(role) else {
            return WaitTransition::Unchanged;
        };
        let retry_gate = RetryGate::from_policy(binding.retry, device_revision, now);
        self.mutate_status(role, |registered_role| match now {
            FrameClockReading::Measurable(schedule_now) => {
                let previous_bound = match &registered_role.state {
                    RoleState::Waiting(waiting_state) => waiting_state.published_bound(),
                    RoleState::Applying(_) | RoleState::Established(_) | RoleState::Stopped(_) => {
                        WaitBoundInput::Unbounded
                    },
                };
                let replaced = std::mem::replace(
                    &mut registered_role.state,
                    RoleState::Waiting(WaitingState::registered()),
                );
                let (mut waiting_state, transition) =
                    waiting_state_from_lifecycle(replaced, schedule_now, waiting_condition);
                waiting_state.retry_pacing = RetryPacing::Blocked(retry_gate);
                waiting_state.revision_retry_bound = WaitBoundInput::Until(wait_bound);
                if previous_bound != waiting_state.published_bound() {
                    waiting_state.bound_state = WaitBoundState::Within;
                }
                registered_role.state = RoleState::Waiting(waiting_state);
                transition
            },
            FrameClockReading::NotYetAdvanced => {
                if let RoleState::Waiting(waiting_state) = &mut registered_role.state {
                    let transition = if waiting_state.condition.same_identity(&waiting_condition) {
                        WaitTransition::Unchanged
                    } else {
                        WaitTransition::ReasonChanged
                    };
                    waiting_state.condition = waiting_condition;
                    waiting_state.retry_pacing = RetryPacing::Blocked(retry_gate);
                    waiting_state.revision_retry_bound = WaitBoundInput::Until(wait_bound);
                    waiting_state.bound_state = WaitBoundState::Within;
                    transition
                } else {
                    WaitTransition::Unchanged
                }
            },
        })
        .unwrap_or(WaitTransition::Unchanged)
    }

    /// Read what one role is waiting for before another attempt may be dispatched.
    pub(crate) fn retry_pacing(&self, role: &RoleKey) -> RetryPacing {
        let retained =
            self.by_role
                .get(role)
                .and_then(|registered_role| match &registered_role.state {
                    RoleState::Waiting(waiting_state) => Some(waiting_state.retry_pacing),
                    RoleState::Applying(_) | RoleState::Established(_) | RoleState::Stopped(_) => {
                        None
                    },
                });
        retained.unwrap_or(RetryPacing::Ready)
    }

    /// Make `role` immediately eligible for reconsideration without changing
    /// its attempt-failure history or lifecycle state.
    fn clear_retry_pacing(&mut self, role: &RoleKey) {
        self.set_role_retry_pacing(role, RetryPacing::Ready);
    }

    /// Dispatch for a role the kernel stopped after three consecutive failures.
    ///
    /// The explicit half of the rule: the other way out is a successful attempt after
    /// reacquisition. Restarting clears the failure count so the role gets a full run again rather
    /// than stopping on its next single failure.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::RoleNotBound` when the role has no retained binding, and
    /// `BindingError::RoleNotStopped` when it was never stopped, so a mistaken restart cannot
    /// silently cancel an in-flight attempt.
    pub fn restart_role(&mut self, role: &RoleKey) -> Result<(), BindingError> {
        let registered_role = self
            .by_role
            .get(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        let stopped = matches!(registered_role.state, RoleState::Stopped(_));
        if !stopped {
            return Err(BindingError::RoleNotStopped { role: role.clone() });
        }
        self.set_role_state(role, RoleState::Waiting(WaitingState::registered()));

        Ok(())
    }

    /// Record how a stopped role's endpoint reads this frame and re-arm it once its device returns.
    ///
    /// This is the other half of the escalation rule: a role stopped after three consecutive
    /// failures leaves that state on an explicit
    /// `Self::restart_role`, or on a successful attempt after reacquisition.
    /// Reacquisition is what this method watches for — the endpoint has to have gone
    /// `EndpointAvailability::Gone` and come back before another attempt is dispatched, so a wedged
    /// device that never leaves is not retried at frame rate. The failure count is deliberately
    /// left standing, so the returning device gets exactly one more attempt: it succeeds and clears
    /// the run, or it fails and the role stops again without a fourth dispatch.
    ///
    /// Roles in any other state are ignored, so a caller can pass every registered role.
    pub(crate) fn observe_stopped_role_endpoint(
        &mut self,
        role: &RoleKey,
        endpoint_availability: EndpointAvailability,
    ) {
        let mut rearmed_failures = RoleApplyFailureRun::Clear;
        let mut rearmed = false;
        if let Some(registered_role) = self.by_role.get_mut(role) {
            match &mut registered_role.state {
                RoleState::Stopped(StoppedState::RepeatedFailures {
                    failures,
                    reacquisition,
                    last_ending,
                }) => match (*reacquisition, endpoint_availability) {
                    (_, EndpointAvailability::Gone) => {
                        *reacquisition = ReacquisitionProgress::Gone;
                        return;
                    },
                    (ReacquisitionProgress::Gone, EndpointAvailability::Available) => {
                        rearmed_failures = RoleApplyFailureRun::Consecutive {
                            failures:    *failures,
                            last_ending: last_ending.clone(),
                        };
                        rearmed = true;
                    },
                    (ReacquisitionProgress::StillAvailable, EndpointAvailability::Available) => {
                        return;
                    },
                },
                RoleState::Stopped(StoppedState::Unsupported { .. }) => return,
                RoleState::Waiting(_) | RoleState::Applying(_) | RoleState::Established(_) => {},
            }
        }
        if rearmed {
            let mut waiting_state = WaitingState::registered();
            waiting_state.failures = rearmed_failures;
            self.set_role_state(role, RoleState::Waiting(waiting_state));
        }
    }

    /// Iterate every role that currently has a retained binding.
    pub(crate) fn registered_roles(&self) -> impl Iterator<Item = &RoleKey> { self.by_role.keys() }

    /// Iterate the durable device keys addressed by at least one retained role.
    pub(crate) fn bound_device_keys(&self) -> impl Iterator<Item = &DeviceKey> {
        self.roles_by_device.keys()
    }

    /// Return the value an offline authoring interface can show for one retained role.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::RoleNotBound` when the role was never registered or was retired.
    pub fn configuration_for(
        &self,
        role: &RoleKey,
    ) -> Result<AvailableConfiguration<'_>, BindingError> {
        let binding = self.binding(role)?;
        Ok(match &binding.last_known_good {
            LastKnownGoodConfiguration::NotEstablished => {
                AvailableConfiguration::Requested(binding.requested.configuration())
            },
            LastKnownGoodConfiguration::MatchesRequested => {
                AvailableConfiguration::LastKnownGood(binding.requested.configuration())
            },
            LastKnownGoodConfiguration::DiffersFromDispatched(configuration) => {
                AvailableConfiguration::LastKnownGood(configuration.as_ref())
            },
        })
    }

    /// Change the bounded lifecycle handoff capacity without discarding already accepted work.
    ///
    /// # Errors
    ///
    /// Returns `BindingCapacityError` when the requested capacity is smaller than the number of
    /// lifecycle transitions currently awaiting the next frame drain.
    #[cfg(feature = "test-support")]
    pub fn set_pending_transition_capacity(
        &mut self,
        capacity: NonZeroUsize,
    ) -> Result<(), BindingCapacityError> {
        let pending = self.pending_transitions.queue.len();
        if capacity.get() < pending {
            return Err(BindingCapacityError::BelowPendingCount { capacity, pending });
        }

        self.pending_transitions.capacity = capacity;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) const fn mark_transition_sequence_exhausted(&mut self) {
        self.next_transition_sequence = u64::MAX;
    }

    fn has_pending_transitions(&self) -> bool { !self.pending_transitions.queue.is_empty() }

    pub(crate) fn pending_transitions(
        &self,
    ) -> impl DoubleEndedIterator<Item = &BindingTransition> + ExactSizeIterator {
        self.pending_transitions.queue.iter()
    }

    fn take_pending_transitions(&mut self) -> VecDeque<BindingTransition> {
        std::mem::take(&mut self.pending_transitions.queue)
    }

    fn reserve_transition(&self) -> Result<ReservedBindingTransition, BindingError> {
        if !self.pending_transitions.has_capacity() {
            return Err(BindingError::PendingTransitionCapacityReached);
        }
        let next_transition_sequence = self
            .next_transition_sequence
            .checked_add(1)
            .ok_or(BindingError::TransitionSequenceExhausted)?;

        Ok(ReservedBindingTransition {
            sequence: BindingTransitionSequence(self.next_transition_sequence),
            next_transition_sequence,
        })
    }

    fn reserve_transitions(
        &self,
        count: usize,
    ) -> Result<Vec<ReservedBindingTransition>, BindingError> {
        let Some(pending) = self.pending_transitions.queue.len().checked_add(count) else {
            return Err(BindingError::PendingTransitionCapacityReached);
        };
        if pending > self.pending_transitions.capacity.get() {
            return Err(BindingError::PendingTransitionCapacityReached);
        }
        let mut reservations = Vec::with_capacity(count);
        let mut sequence = self.next_transition_sequence;
        for _ in 0..count {
            let Some(next_transition_sequence) = sequence.checked_add(1) else {
                return Err(BindingError::TransitionSequenceExhausted);
            };
            reservations.push(ReservedBindingTransition {
                sequence: BindingTransitionSequence(sequence),
                next_transition_sequence,
            });
            sequence = next_transition_sequence;
        }
        Ok(reservations)
    }

    fn enqueue(
        &mut self,
        binding_transition_kind: BindingTransitionKind,
        role: RoleKey,
        reserved_transition: ReservedBindingTransition,
    ) {
        let ReservedBindingTransition {
            sequence,
            next_transition_sequence,
        } = reserved_transition;
        self.next_transition_sequence = next_transition_sequence;
        let binding_transition = match binding_transition_kind {
            BindingTransitionKind::Registered => BindingTransition::Registered { sequence, role },
            BindingTransitionKind::Replaced {
                displaced_entity,
                endpoint,
                cleanup,
            } => BindingTransition::Replaced {
                sequence,
                role,
                displaced_entity,
                endpoint,
                cleanup,
            },
            BindingTransitionKind::Retired {
                endpoint,
                entity,
                cleanup,
            } => BindingTransition::Retired {
                sequence,
                role,
                endpoint,
                entity,
                cleanup,
            },
        };
        self.pending_transitions.push(binding_transition);
    }

    fn remove_role_from_device(&mut self, device_key: &DeviceKey, role: &RoleKey) {
        let remove_device_entry = if let Some(roles) = self.roles_by_device.get_mut(device_key) {
            roles.retain(|stored_role| stored_role != role);
            roles.is_empty()
        } else {
            false
        };
        if remove_device_entry {
            self.roles_by_device.remove(device_key);
        }
    }
}

fn project_status(
    registered_role: &RegisteredRole,
    runtime_clock: RiggingRuntimeClock,
) -> RoleStatusView {
    match &registered_role.state {
        RoleState::Waiting(waiting_state) => {
            RoleStatusView::Waiting(project_waiting_status(waiting_state, runtime_clock))
        },
        RoleState::Applying(active_apply) => RoleStatusView::Applying {
            attempt:         active_apply.attempt.reference,
            since:           runtime_clock.time_at(active_apply.attempt.started_at),
            deadline:        runtime_clock.time_at(active_apply.attempt.deadline),
            source:          match active_apply.source {
                ApplyConfigurationSource::Requested => ApplySourceView::Requested,
                ApplyConfigurationSource::LastKnownGood => ApplySourceView::LastKnownGood,
            },
            application_run: registered_role.application_run.view(),
        },
        RoleState::Established(session) => RoleStatusView::Established {
            since:   runtime_clock.time_at(session.established_at),
            session: session.session,
            applied: session.applied,
            flow:    session.flow.view(runtime_clock),
        },
        RoleState::Stopped(StoppedState::RepeatedFailures {
            failures,
            last_ending,
            ..
        }) => RoleStatusView::Stopped(StoppedStatusView::RepeatedFailures {
            failures:     *failures,
            last_ending:  project_attempt_ending(last_ending),
            resumes_when: ResumeCondition::ExplicitRestartOrReacquisition,
        }),
        RoleState::Stopped(StoppedState::Unsupported { failure }) => {
            RoleStatusView::Stopped(StoppedStatusView::Unsupported {
                reason:       DriverStopReasonView::from(failure),
                resumes_when: ResumeCondition::ExplicitRestart,
            })
        },
    }
}

fn project_waiting_status(
    waiting_state: &WaitingState,
    runtime_clock: RiggingRuntimeClock,
) -> WaitingStatusView {
    let timing = wait_timing(
        waiting_state,
        runtime_clock,
        waiting_state.condition.bound(),
    );
    let retry = retry_schedule_view(waiting_state.retry_pacing, runtime_clock);
    match &waiting_state.condition {
        WaitingCondition::Reporter(reporter_wait) => {
            WaitingStatusView::Reporter(project_hardware_wait(reporter_wait, timing, runtime_clock))
        },
        WaitingCondition::KernelRetry(retry_gate) => WaitingStatusView::KernelRetry {
            timing:   wait_timing(
                waiting_state,
                runtime_clock,
                retry_gate_bound(*retry_gate, waiting_state.revision_retry_bound),
            ),
            gate:     retry_gate_view(*retry_gate, runtime_clock),
            failures: match &waiting_state.failures {
                RoleApplyFailureRun::Clear => RoleApplyFailureRunView::Clear,
                RoleApplyFailureRun::Consecutive {
                    failures,
                    last_ending,
                } => RoleApplyFailureRunView::Consecutive {
                    failures:    *failures,
                    last_ending: project_attempt_ending(last_ending),
                },
            },
        },
        WaitingCondition::ApplicationReapply => WaitingStatusView::ApplicationReapply { timing },
        WaitingCondition::ApplicationReporterEnable { reporters } => {
            WaitingStatusView::ApplicationReporterEnable {
                reporters: reporters.reporter_refs(),
                timing,
            }
        },
        WaitingCondition::NewRegistration => WaitingStatusView::NewRegistration { timing },
        WaitingCondition::OperatorDecision { role, candidate } => {
            WaitingStatusView::OperatorDecision {
                role: role.clone(),
                candidate: candidate.clone(),
                timing,
            }
        },
        WaitingCondition::ClaimRelease { holder } => WaitingStatusView::ClaimRelease {
            holder: ClaimHolderView::from(holder.clone()),
            timing,
        },
        WaitingCondition::DriverRepair { error } => WaitingStatusView::DriverRepair {
            error: DriverContractFailureView::from(error),
            timing: retry_timing(waiting_state, runtime_clock),
            retry,
        },
        WaitingCondition::ApplicationCapabilityRegistrationRequired { failure } => {
            WaitingStatusView::ApplicationCapabilityRegistrationRequired {
                failure: failure.clone(),
                timing,
            }
        },
        WaitingCondition::ApplicationReporterRegistrationRequired { key } => {
            WaitingStatusView::ApplicationReporterRegistrationRequired {
                key: key.clone(),
                timing,
            }
        },
        WaitingCondition::ApplicationDeviceEnableRequired { key } => {
            WaitingStatusView::ApplicationDeviceEnableRequired {
                key: key.clone(),
                timing,
            }
        },
        WaitingCondition::ApplicationPermissionRequired { gate } => {
            WaitingStatusView::ApplicationPermissionRequired {
                gate: PermissionGateView::from(gate),
                timing,
            }
        },
        WaitingCondition::ApplicationBindingRepairRequired => {
            WaitingStatusView::ApplicationBindingRepairRequired {
                timing: retry_timing(waiting_state, runtime_clock),
                retry,
            }
        },
        WaitingCondition::ApplicationTargetAttachmentRequired => {
            WaitingStatusView::ApplicationTargetAttachmentRequired {
                timing: retry_timing(waiting_state, runtime_clock),
                retry,
            }
        },
        WaitingCondition::KernelStateRepairRequired => {
            WaitingStatusView::KernelStateRepairRequired { timing }
        },
    }
}

fn project_hardware_wait(
    reporter_wait: &ReporterWait,
    timing: WaitTiming,
    runtime_clock: RiggingRuntimeClock,
) -> HardwareWait {
    match reporter_wait {
        ReporterWait::AwaitingFirstReport(wait) => HardwareWait::AwaitingFirstReport {
            key: wait.key.clone(),
            reporters: wait.reporters.reporter_refs(),
            timing,
        },
        ReporterWait::Unconfirmed(unconfirmed) => HardwareWait::Unconfirmed {
            key: unconfirmed.wait.key.clone(),
            basis: unconfirmed.basis.clone(),
            reporters: unconfirmed.wait.reporters.reporter_refs(),
            timing,
        },
        ReporterWait::Unreachable(wait) => HardwareWait::Unreachable {
            key: wait.key.clone(),
            reporters: wait.reporters.reporter_refs(),
            timing,
        },
        ReporterWait::DepartureGrace {
            key,
            deadline,
            evidence,
        } => HardwareWait::DepartureGrace {
            key:      key.clone(),
            deadline: runtime_clock.time_at(*deadline),
            evidence: *evidence,
        },
        ReporterWait::Absent(wait) => HardwareWait::Absent {
            key:            wait.key.clone(),
            established_by: wait.evidence,
        },
    }
}

pub(crate) fn project_attempt_ending(attempt_ending: &AttemptEnding) -> crate::AttemptEndingView {
    match attempt_ending {
        AttemptEnding::Reported(outcome) => {
            AttemptEndingView::Reported(crate::AttemptOutcomeView::from(outcome))
        },
        AttemptEnding::Invalidated(invalidation) => {
            AttemptEndingView::Invalidated((*invalidation).into())
        },
        AttemptEnding::ContractFailed(failure) => {
            AttemptEndingView::ContractFailed(DriverContractFailureView::from(failure))
        },
    }
}

fn retry_schedule_view(
    retry_pacing: RetryPacing,
    runtime_clock: RiggingRuntimeClock,
) -> RetryScheduleView {
    match retry_pacing {
        RetryPacing::Ready => RetryScheduleView::Ready,
        RetryPacing::Blocked(retry_gate) => {
            RetryScheduleView::Blocked(retry_gate_view(retry_gate, runtime_clock))
        },
    }
}

fn retry_gate_view(retry_gate: RetryGate, runtime_clock: RiggingRuntimeClock) -> RetryGateView {
    match retry_gate {
        RetryGate::AwaitingRevision(device_revision) => RetryGateView::DeviceRevisionChanged {
            from: match device_revision {
                DeviceRevisionLookup::Retired => DeviceRevisionGateView::DeviceRetired,
                DeviceRevisionLookup::Retained(device_revision) => {
                    DeviceRevisionGateView::Retained(DeviceRevisionView::from_revision(
                        device_revision,
                    ))
                },
            },
        },
        RetryGate::AwaitingInstant(retry_at) => RetryGateView::TimeReached {
            retry_at: runtime_clock.time_at(retry_at),
        },
    }
}

const fn retry_gate_bound(
    retry_gate: RetryGate,
    revision_retry_bound: WaitBoundInput,
) -> WaitBoundInput {
    match retry_gate {
        RetryGate::AwaitingRevision(_) => revision_retry_bound,
        RetryGate::AwaitingInstant(retry_at) => WaitBoundInput::Until(retry_at),
    }
}

fn retry_timing(waiting_state: &WaitingState, runtime_clock: RiggingRuntimeClock) -> WaitTiming {
    match waiting_state.retry_pacing {
        RetryPacing::Ready => wait_timing(waiting_state, runtime_clock, WaitBoundInput::Unbounded),
        RetryPacing::Blocked(retry_gate) => wait_timing(
            waiting_state,
            runtime_clock,
            retry_gate_bound(retry_gate, waiting_state.revision_retry_bound),
        ),
    }
}

fn wait_timing(
    waiting_state: &WaitingState,
    runtime_clock: RiggingRuntimeClock,
    wait_bound_input: WaitBoundInput,
) -> WaitTiming {
    let since = match waiting_state.wait_start {
        WaitStart::BeforeFirstSchedule => RiggingRuntimeTime::from_elapsed(Duration::ZERO),
        WaitStart::At(started_at) => runtime_clock.time_at(started_at),
    };
    match wait_bound_input {
        WaitBoundInput::Unbounded => WaitTiming::Unbounded { since },
        WaitBoundInput::Until(deadline) => match waiting_state.bound_state {
            WaitBoundState::Within => WaitTiming::Bounded {
                since,
                deadline: runtime_clock.time_at(deadline),
            },
            WaitBoundState::Crossed { crossed_at } => WaitTiming::Overdue {
                since,
                deadline: runtime_clock.time_at(deadline),
                crossed_at: runtime_clock.time_at(crossed_at),
            },
        },
    }
}

enum BindingTransitionKind {
    Registered,
    Replaced {
        displaced_entity: Entity,
        endpoint:         DeviceEndpoint,
        cleanup:          DriverCleanup,
    },
    Retired {
        endpoint: DeviceEndpoint,
        entity:   Entity,
        cleanup:  DriverCleanup,
    },
}

struct ReservedBindingTransition {
    sequence:                 BindingTransitionSequence,
    next_transition_sequence: u64,
}

struct PreparedDeviceReaddress {
    role:                RoleKey,
    saved_endpoint:      DeviceEndpoint,
    adopted_endpoint:    DeviceEndpoint,
    reserved_transition: ReservedBindingTransition,
}

/// Result of retiring one role while distinguishing a repeated request from an effective change.
pub enum RetirementOutcome {
    /// The retained binding was removed and marked retired before it was returned.
    Retired(Box<Binding>),
    /// No binding remained for this role, so no lifecycle work was added.
    AlreadyUnbound,
}

/// Recoverable failure from a checked binding operation or state-issued request.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BindingError {
    /// The submitted role already has an authored binding whose endpoint ownership must remain.
    #[error("role `{role}` is already bound")]
    RoleAlreadyBound {
        /// Existing application role that rejected a second binding record.
        role: RoleKey,
    },
    /// An operation required a binding for this role, but none remains registered.
    #[error("role `{role}` is not bound")]
    RoleNotBound {
        /// Application role that did not select a retained binding record.
        role: RoleKey,
    },
    /// A lifecycle operation required the role's currently active apply.
    #[error("role `{role}` has no active apply")]
    RoleNotApplying {
        /// Bound role whose apply had already ended or had not started.
        role: RoleKey,
    },
    /// Role status projection was requested before the rigging runtime clock was installed.
    #[error("role `{role}` has no installed role-status runtime clock")]
    RoleStatusClockNotInstalled {
        /// Bound role whose status cannot be projected without the shared runtime time origin.
        role: RoleKey,
    },
    /// A different role already owns the proposed endpoint, so two drivers cannot race it.
    #[error("endpoint `{endpoint:?}` is already owned by role `{owner}`")]
    EndpointAlreadyOwned {
        /// Endpoint the operation proposed for a second role.
        endpoint: DeviceEndpoint,
        /// Retained role whose binding already owns `endpoint`.
        owner:    RoleKey,
    },
    /// The lifecycle handoff is full, so accepting another authored mutation would lose work.
    #[error("pending binding transition capacity has been reached")]
    PendingTransitionCapacityReached,
    /// The next lifecycle handoff sequence cannot advance without reusing a prior transition id.
    #[error("binding transition sequence is exhausted")]
    TransitionSequenceExhausted,
    /// A restore was requested before a safe readback established a value to restore.
    #[error("role `{role}` has no last-known-good configuration")]
    LastKnownGoodNotEstablished {
        /// Role whose configuration remains authored intent rather than endpoint evidence.
        role: RoleKey,
    },
    /// The configured device is offline, so passive discovery may observe it but no driver call
    /// may be issued for its endpoint.
    #[error("configured device `{device_key:?}` is offline")]
    ConfiguredDeviceOffline {
        /// Durable key whose authored offline mode blocks operational requests.
        device_key: DeviceKey,
    },
    /// A restart was requested for a role the kernel had not stopped, which would have cancelled
    /// whatever that role was doing instead.
    #[error("role `{role}` was not stopped after repeated failures")]
    RoleNotStopped {
        /// Role whose lifecycle state is not stopped.
        role: RoleKey,
    },
}

/// Failure from attempting to lower lifecycle handoff capacity below already pending work.
#[cfg(feature = "test-support")]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BindingCapacityError {
    /// The requested capacity cannot retain the transitions already waiting for a frame drain.
    #[error("pending transition count {pending} exceeds requested capacity {capacity}")]
    BelowPendingCount {
        /// Capacity the caller requested for future lifecycle changes.
        capacity: NonZeroUsize,
        /// Number of transitions that must remain retained before the next drain.
        pending:  usize,
    },
}

/// Whether an established role retains the exact attempt that established its current session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EstablishingAttemptLookup {
    /// This globally unique attempt established the role's current session.
    EstablishedBy(AttemptRef),
    /// No successful attempt currently establishes a session for this role.
    NotEstablished,
}

/// Result of applying a session-loss policy after its guards were selected.
pub(crate) enum SessionLossApplication {
    /// The binding still existed and its authored policy was applied.
    Applied,
    /// The binding was absent when mutation was attempted.
    BindingAbsent,
}

/// Whether one role's durable endpoint currently resolves to a device the kernel may drive.
///
/// A named reading rather than a `bool`, because it is the reacquisition signal a stopped role
/// waits on: `Gone` covers an endpoint that resolves to nothing and one whose device is retained
/// but no longer present, and both are the same fact for that decision — the unit the role was
/// failing against is not the unit it would be dispatched against next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndpointAvailability {
    /// The endpoint resolves to a device the current reconcile pass reads as present.
    Available,
    /// The endpoint resolves to nothing, or to a device that is no longer present.
    Gone,
}

/// What one role is waiting for before another attempt may be dispatched after a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryPacing {
    /// Nothing paces this role: it has not failed, or its last attempt succeeded.
    Ready,
    /// The role failed and this gate has not opened yet.
    Blocked(RetryGate),
}

impl RetryPacing {
    /// Report whether an attempt may be dispatched for this role on this frame.
    pub(crate) fn permits_dispatch(
        self,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) -> bool {
        match self {
            Self::Ready => true,
            Self::Blocked(retry_gate) => retry_gate.opened(device_revision, now),
        }
    }
}

/// Configuration source selected for one authorized dispatch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ApplyConfigurationSource {
    Requested,
    LastKnownGood,
}

impl ApplyConfigurationSource {
    fn configuration(self, binding: &Binding) -> Result<&dyn Reflect, BindingError> {
        match self {
            Self::Requested => Ok(binding.requested.configuration()),
            Self::LastKnownGood => binding
                .last_known_good
                .as_reflect(binding.requested.configuration())
                .map_err(|_| BindingError::LastKnownGoodNotEstablished {
                    role: binding.role.clone(),
                }),
        }
    }

    pub(crate) fn dispatch_configuration(
        self,
        binding: &Binding,
    ) -> Result<&dyn Reflect, BindingError> {
        self.configuration(binding)
    }
}

/// Whether authored inventory permits driver operations for a configured device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum ConfiguredDeviceMode {
    /// The kernel may later ask a driver to capture, apply, and poll this device's endpoints.
    Managed,
    /// Reporters may enumerate the device passively, but no driver operation may touch it.
    Offline,
}

/// What a person recognizes one authored device by.
#[derive(Component, Clone, Debug, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub enum AuthoredDeviceName {
    /// The unit reported this name. `among` separates units reporting the same one.
    Reported {
        /// Name supplied by the unit itself.
        name:  ReportedDeviceName,
        /// Domain-formatted distinction between units that report the same `name`.
        among: DeviceNameDisambiguation,
    },
    /// Nothing named this unit, so what distinguishes it stands in for a name.
    Measured(DeviceNameDisambiguationText),
    /// Nothing about this unit can name it, so a person did.
    ///
    /// A DMX fixture and a laser zone report no vendor, product, or model text at all.
    Operator(OperatorAssignedDeviceName),
}

impl AuthoredDeviceName {
    /// Merge current evidence without replacing a person-assigned name.
    ///
    /// A reported name outranks a measured one. When a measured observation follows a saved
    /// reported name, the reported text survives while the new measurement refreshes its
    /// disambiguation.
    #[must_use]
    pub fn updated_from_observation(&self, observed_device_name: ObservedDeviceName) -> Self {
        match (self, observed_device_name) {
            (Self::Operator(name), _) => Self::Operator(name.clone()),
            (Self::Reported { name, .. }, ObservedDeviceName::Measured(disambiguation_text)) => {
                Self::Reported {
                    name:  name.clone(),
                    among: DeviceNameDisambiguation::Distinguishing(disambiguation_text),
                }
            },
            (_, ObservedDeviceName::Reported { name, among }) => Self::Reported { name, among },
            (_, ObservedDeviceName::Measured(disambiguation_text)) => {
                Self::Measured(disambiguation_text)
            },
        }
    }
}

impl From<ObservedDeviceName> for AuthoredDeviceName {
    fn from(observed_device_name: ObservedDeviceName) -> Self {
        match observed_device_name {
            ObservedDeviceName::Reported { name, among } => Self::Reported { name, among },
            ObservedDeviceName::Measured(disambiguation_text) => {
                Self::Measured(disambiguation_text)
            },
        }
    }
}

/// What separates two units of one class that a person would otherwise confuse.
///
/// Rigging never interprets the value. The domain that authored the device formats it, because
/// what distinguishes two displays is not what distinguishes two cameras or two fixtures.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum DeviceNameDisambiguation {
    /// A domain-formatted value that differs between otherwise identical units.
    Distinguishing(DeviceNameDisambiguationText),
    /// This class has nothing that separates two identical units.
    Indistinguishable,
}

/// A device name derived from current evidence. A person's name can never be observed.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum ObservedDeviceName {
    /// The unit reported this name, with any current distinction among identical reports.
    Reported {
        /// Name supplied by the unit itself.
        name:  ReportedDeviceName,
        /// Domain-formatted distinction between units that report the same `name`.
        among: DeviceNameDisambiguation,
    },
    /// Nothing named this unit, so current evidence distinguishing it stands in for a name.
    Measured(DeviceNameDisambiguationText),
}

/// Whether an inventory entry already has an operator-facing authored name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Reflect)]
pub enum ConfiguredDeviceName {
    /// The entry carries the name by which a person recognizes this authored device.
    Named(AuthoredDeviceName),
    /// No saved component or current evidence has derived a name for this entry yet.
    #[default]
    NeverDerived,
}

/// Which role already holds one endpoint, kept as a named state rather than an absent role.
///
/// Read by `crate::IdentityDecisions` before it records an adoption, where "nobody owns it" and
/// "another role owns it" lead to opposite answers for the operator.
#[derive(Clone, Debug, Default, PartialEq, Eq, Reflect)]
pub(crate) enum EndpointOwner {
    /// No other role owns the endpoint, so an adoption may move onto it.
    #[default]
    Unowned,
    /// This role owns the endpoint, so an adoption would have to take it away and does not.
    OwnedBy(RoleKey),
}

/// Authored device inventory entry that exists independently of reporter activation and entities.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub struct ConfiguredDevice {
    /// Durable identity the application authored without creating a live device entity.
    pub key:  DeviceKey,
    /// Operational rule that leaves passive connection evidence visible when offline.
    pub mode: ConfiguredDeviceMode,
    /// Operator-facing name retained with this entry's membership and operational mode.
    pub name: ConfiguredDeviceName,
}

/// Connectivity conclusion retained for one authored device without changing its operational mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum ConfiguredDeviceConnection {
    /// No enabled reporter completed a discovery capable of observing this authored key.
    NotObserved,
    /// Current passive reporter evidence contains this authored key.
    Present,
    /// Relevant complete reporter evidence omitted this key.
    Absent,
    /// Relevant evidence expired or discovery failed before absence could be established.
    Unreachable,
}

/// Authored durable hardware inventory and passive connectivity conclusions.
///
/// Adding an entry does not register, enable, or run a reporter, and it never creates a live
/// `DeviceId`. Reconciliation updates `ConfiguredDeviceConnection` from retained reporter
/// evidence while this resource keeps offline operation separate from connection visibility.
#[derive(Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct HardwareInventory {
    #[reflect(ignore, default = "default_configured_devices")]
    configured:  HashMap<DeviceKey, ConfiguredDevice>,
    #[reflect(ignore, default = "default_configured_device_connections")]
    connections: HashMap<DeviceKey, ConfiguredDeviceConnection>,
}

fn default_configured_devices() -> HashMap<DeviceKey, ConfiguredDevice> { HashMap::new() }

fn default_configured_device_connections() -> HashMap<DeviceKey, ConfiguredDeviceConnection> {
    HashMap::new()
}

impl HardwareInventory {
    /// Retain one authored device without enabling reporters or creating a device entity.
    pub fn configure(&mut self, configured_device: ConfiguredDevice) {
        let device_key = configured_device.key.clone();
        self.configured
            .insert(device_key.clone(), configured_device);
        self.connections
            .entry(device_key)
            .or_insert(ConfiguredDeviceConnection::NotObserved);
    }

    /// Replace every authored entry of one device kind, dropping keys this application no
    /// longer authors.
    ///
    /// Connection conclusions survive for keys that survive and are dropped with the keys that
    /// do not, so a unit nobody authors any more leaves no conclusion behind.
    ///
    /// # Errors
    ///
    /// Returns [`HardwareInventoryError::DeviceKindMismatch`] when any entry's key does not carry
    /// `kind`. A replacement scoped to one kind cannot be trusted to remove what it did not
    /// enumerate.
    pub fn replace_authored_devices(
        &mut self,
        kind: DeviceKind,
        devices: impl IntoIterator<Item = ConfiguredDevice>,
    ) -> Result<(), HardwareInventoryError> {
        let mut replacements = HashMap::new();
        for configured_device in devices {
            if configured_device.key.kind != kind {
                return Err(HardwareInventoryError::DeviceKindMismatch {
                    expected_kind: kind,
                    device_key:    configured_device.key,
                });
            }
            replacements.insert(configured_device.key.clone(), configured_device);
        }

        self.configured.retain(|device_key, _| {
            device_key.kind != kind || replacements.contains_key(device_key)
        });
        self.connections.retain(|device_key, _| {
            device_key.kind != kind || replacements.contains_key(device_key)
        });
        for (device_key, configured_device) in replacements {
            self.configured
                .insert(device_key.clone(), configured_device);
            self.connections
                .entry(device_key)
                .or_insert(ConfiguredDeviceConnection::NotObserved);
        }

        Ok(())
    }

    /// Move one authored entry and its connection conclusion onto an adopted durable key.
    ///
    /// Called with the binding rewrite in `Bindings::readdress`, because the two are keyed the same
    /// way: an adoption that moved the binding and left inventory holding the old key would leave
    /// the authored operation mode attached to a unit nothing addresses any more.
    ///
    /// A saved key nobody authored has nothing to move, which is not a failure — inventory records
    /// the application's decisions, and having made none is not one.
    pub(crate) fn readdress(&mut self, saved: &DeviceKey, candidate: DeviceKey) {
        let Some(mut configured_device) = self.configured.remove(saved) else {
            return;
        };
        let connection = self
            .connections
            .remove(saved)
            .unwrap_or(ConfiguredDeviceConnection::NotObserved);
        configured_device.key = candidate.clone();
        self.configured.insert(candidate.clone(), configured_device);
        self.connections.insert(candidate, connection);
    }

    /// Borrow one configured device and its authored operation mode.
    ///
    /// # Errors
    ///
    /// Returns `HardwareInventoryError::DeviceNotConfigured` when no authored entry uses this
    /// durable key.
    pub fn configured_device(
        &self,
        device_key: &DeviceKey,
    ) -> Result<&ConfiguredDevice, HardwareInventoryError> {
        self.configured
            .get(device_key)
            .ok_or_else(|| HardwareInventoryError::DeviceNotConfigured {
                device_key: device_key.clone(),
            })
    }

    /// Read the passive connection conclusion retained for one authored device.
    ///
    /// # Errors
    ///
    /// Returns `HardwareInventoryError::DeviceNotConfigured` for a key that application code did
    /// not author into this inventory.
    pub fn connection(
        &self,
        device_key: &DeviceKey,
    ) -> Result<ConfiguredDeviceConnection, HardwareInventoryError> {
        self.configured_device(device_key)?;
        self.connections.get(device_key).copied().ok_or_else(|| {
            HardwareInventoryError::DeviceNotConfigured {
                device_key: device_key.clone(),
            }
        })
    }

    /// Iterate every durable key application code authored into this inventory.
    ///
    /// Reconciliation walks these rather than the reported device set: an authored unit that no
    /// reporter has ever named still has a connection conclusion to record, and it is exactly the
    /// case a walk over live evidence would miss.
    pub(crate) fn configured_keys(&self) -> impl Iterator<Item = &DeviceKey> {
        self.configured.keys()
    }

    /// Iterate every named authored device of one physical kind.
    ///
    /// The key and name come from the same [`ConfiguredDevice`] entry, so callers cannot observe
    /// a name whose inventory membership has already been replaced or retired.
    pub fn named_devices(
        &self,
        kind: DeviceKind,
    ) -> impl Iterator<Item = (&DeviceKey, &AuthoredDeviceName)> {
        self.configured
            .iter()
            .filter(move |(device_key, _)| device_key.kind == kind)
            .filter_map(
                |(device_key, configured_device)| match &configured_device.name {
                    ConfiguredDeviceName::Named(authored_device_name) => {
                        Some((device_key, authored_device_name))
                    },
                    ConfiguredDeviceName::NeverDerived => None,
                },
            )
    }

    /// Record what current reporter evidence says about one authored device's connectivity.
    ///
    /// Connection is separate from `ConfiguredDeviceMode`: recording that an offline unit is
    /// plugged in neither enables a reporter nor authorizes anything to drive it.
    ///
    /// # Errors
    ///
    /// Returns `HardwareInventoryError::DeviceNotConfigured` for a key that application code did
    /// not author into this inventory.
    pub fn set_connection(
        &mut self,
        device_key: &DeviceKey,
        connection: ConfiguredDeviceConnection,
    ) -> Result<(), HardwareInventoryError> {
        self.configured_device(device_key)?;
        self.connections.insert(device_key.clone(), connection);
        Ok(())
    }

    /// Report whether an endpoint's durable device may receive driver traffic at all.
    ///
    /// Callers that must answer this before taking mutable access — the safe-capture pass reads it
    /// to check whether a frame has work before it borrows `Bindings` mutably — need the same
    /// answer the typed role views enforce, and a second copy of the offline rule would let the two
    /// drift.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::ConfiguredDeviceOffline` for a device inventory marks offline.
    pub(crate) fn ensure_operational(&self, device_key: &DeviceKey) -> Result<(), BindingError> {
        match self.configured.get(device_key) {
            Some(ConfiguredDevice {
                mode: ConfiguredDeviceMode::Offline,
                ..
            }) => Err(BindingError::ConfiguredDeviceOffline {
                device_key: device_key.clone(),
            }),
            Some(ConfiguredDevice {
                mode: ConfiguredDeviceMode::Managed,
                ..
            })
            | None => Ok(()),
        }
    }
}

/// The binding entity's current link to the live device entity its endpoint resolves to.
///
/// Present only while the durable `DeviceEndpoint` names a device the kernel currently retains, so
/// its absence is exactly "this role has no live hardware right now". It is a relationship rather
/// than a second ownership map because Bevy then maintains `ResolvedBindings` on the device side
/// for free, and replacing the link moves the binding between reverse collections with no
/// bookkeeping that could drift from the authored record in `Bindings`.
///
/// It deliberately omits `linked_spawn`: despawning a departed device must remove this link and
/// nothing else. Despawning the binding entity would erase the role's retained policy and
/// configuration, which is the state that makes a returning unit recoverable at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Component, Reflect)]
#[relationship(relationship_target = ResolvedBindings)]
#[reflect(Component, PartialEq)]
pub struct ResolvedToDevice(Entity);

impl ResolvedToDevice {
    /// Link one binding entity to the device entity its durable endpoint currently resolves to.
    pub(crate) const fn new(device: Entity) -> Self { Self(device) }

    /// Read the device entity this link currently points at.
    #[must_use]
    pub const fn device(self) -> Entity { self.0 }
}

/// Every binding entity whose endpoint currently resolves to this live device entity.
///
/// Bevy maintains this collection from `ResolvedToDevice`, so a tool holding a device entity can
/// walk to every role using that unit — the Stream Deck's three bound endpoints, or a display
/// shared by two window roles — without the kernel keeping a second entity index that could
/// disagree with the authored record. It reports live resolution only; `Bindings::roles_for`
/// remains authoritative for durable ownership and for roles whose device is absent, and the
/// relationship cannot enforce endpoint uniqueness because it targets the whole device rather than
/// one endpoint of it.
#[derive(Debug, Component, Reflect)]
#[relationship_target(relationship = ResolvedToDevice)]
#[reflect(Component)]
pub struct ResolvedBindings(Vec<Entity>);

/// Every binding transition accepted before this frame's drain, in the order they were accepted.
///
/// One drain per frame moves `Bindings::take_pending_transitions` in here so the binding-entity
/// stage, the attempt aborts, and the public event stage all read one identical ordered
/// list. Reading `Bindings` directly from three stages would let the first drain hide the
/// registration from the other two. Entries are never removed one at a time: the event stage clears
/// the whole batch once it has emitted this frame's transitions, and `drain_binding_transitions`
/// replaces the contents wholesale on the next frame regardless, so a missing `clear` cannot strand
/// entries past the frame that produced them.
#[derive(Debug, Default, Resource)]
pub(crate) struct BindingTransitionBatch {
    transitions: Vec<BindingTransition>,
}

impl BindingTransitionBatch {
    /// Read this frame's accepted transitions in the order `Bindings` sequenced them.
    pub(crate) fn transitions(&self) -> &[BindingTransition] { &self.transitions }

    /// Drop this frame's transitions once the last consumer has read them.
    pub(crate) fn clear(&mut self) { self.transitions.clear(); }
}

/// Move every binding transition accepted since the last frame into this frame's shared batch.
///
/// Registration and retirement are application work, not discovery work, so this runs whether or
/// not a reporter completed a scan: it is ordered only `before` reconciliation, which returns early
/// on a settled frame and would otherwise strand an accepted transition until the next scan landed.
/// Operations submitted after this system runs stay in `Bindings` and are drained next frame.
///
/// A frame with nothing to move leaves `Bindings` untouched rather than taking an empty queue
/// through `ResMut`, so change detection on the resource still means "an authored operation was
/// accepted" for a once-per-change event stage or a Bevy Remote Protocol resource watch.
pub(crate) fn drain_binding_transitions(
    mut bindings: ResMut<Bindings>,
    mut binding_transition_batch: ResMut<BindingTransitionBatch>,
) {
    if bindings.has_pending_transitions() {
        binding_transition_batch.transitions = bindings.take_pending_transitions().into();
    } else if !binding_transition_batch.transitions.is_empty() {
        binding_transition_batch.clear();
    }
}

/// Refresh and recover the entity owned by each registered role.
///
/// Registration reserves the entity in the caller's system. This pass only replaces an entity
/// removed outside the kernel and refreshes components removed or changed through dynamic access.
pub(crate) fn reconcile_role_entities(
    mut commands: Commands,
    mut bindings: ResMut<Bindings>,
    mut lost_role_entities: ResMut<LostRegisteredRoleEntities>,
    mut lost_relationships: ResMut<LostRoleRelationships>,
    mut mirrors: Query<(&mut RecoveryPolicy, &mut RoleEndpoint), With<RoleKey>>,
    live_entities: Query<()>,
) {
    let registered = bindings
        .registered_role_entities()
        .map(|(role, entity)| (role.clone(), entity))
        .collect::<Vec<_>>();
    let mut replacements = Vec::new();
    for (role, entity) in registered {
        let Ok(binding) = bindings.binding(&role) else {
            continue;
        };
        let Ok((mut recovery_policy, mut role_endpoint)) = mirrors.get_mut(entity) else {
            if live_entities.get(entity).is_ok() {
                commands.entity(entity).insert(role_entity_bundle(binding));
            } else if entity == Entity::PLACEHOLDER || lost_role_entities.0.remove(&role) {
                let replacement = commands.spawn(role_entity_bundle(binding)).id();
                for lost in lost_relationships.0.remove(&role).unwrap_or_default() {
                    commands.queue(move |world: &mut World| {
                        (lost.retarget)(world, lost.source, replacement);
                    });
                }
                replacements.push((role, replacement));
            }
            continue;
        };
        if *recovery_policy != binding.recovery {
            *recovery_policy = binding.recovery;
        }
        if role_endpoint.endpoint() != &binding.endpoint {
            *role_endpoint = RoleEndpoint::new(binding.endpoint.clone());
        }
    }
    for (role, replacement) in replacements {
        bindings.replace_role_entity(&role, replacement);
    }
}

/// Retire role entities after every owed driver callback has received a live entity.
pub(crate) fn retire_role_entities(
    mut commands: Commands,
    binding_transition_batch: Res<BindingTransitionBatch>,
    mut lost_role_entities: ResMut<LostRegisteredRoleEntities>,
    mut lost_relationships: ResMut<LostRoleRelationships>,
) {
    for binding_transition in binding_transition_batch.transitions() {
        match binding_transition {
            BindingTransition::Registered { .. } | BindingTransition::Replaced { .. } => {},
            BindingTransition::Retired {
                role,
                endpoint,
                entity,
                ..
            } => {
                lost_role_entities.0.remove(role);
                lost_relationships.0.remove(role);
                commands.entity(*entity).despawn();
                commands.trigger(RetiredRoleChanged {
                    role:     role.clone(),
                    endpoint: endpoint.clone(),
                    change:   RetiredRoleChange::Retired,
                });
            },
        }
    }
}

/// Failure from reading or replacing authored hardware inventory.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HardwareInventoryError {
    /// The requested durable key has no authored inventory record in this application.
    #[error("device `{device_key:?}` is not configured")]
    DeviceNotConfigured {
        /// Key that did not select a `ConfiguredDevice` inventory entry.
        device_key: DeviceKey,
    },
    /// A kind-scoped replacement received an entry outside the kind it owns.
    #[error("device `{device_key:?}` does not carry replacement kind `{expected_kind:?}`")]
    DeviceKindMismatch {
        /// Kind the replacement operation is allowed to author.
        expected_kind: DeviceKind,
        /// Entry whose key carries a different device kind.
        device_key:    DeviceKey,
    },
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::any::TypeId;
    use std::error::Error;
    use std::num::NonZeroU32;
    use std::num::NonZeroUsize;
    use std::time::Duration;

    use bevy::MinimalPlugins;
    use bevy::app::App;
    use bevy::app::Update;
    use bevy::ecs::change_detection::DetectChanges;
    use bevy::ecs::entity::Entity;
    use bevy::ecs::observer::On;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::ecs::relationship::Relationship;
    use bevy::ecs::relationship::RelationshipTarget;
    use bevy::ecs::schedule::IntoScheduleConfigs;
    use bevy::platform::time::Instant;
    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::World;
    use bevy::time::Real;
    use bevy::time::Time;
    use bevy::time::TimeUpdateStrategy;

    use super::ApplyDeadline;
    use super::Binding;
    use super::BindingCapacityError;
    use super::BindingError;
    use super::BindingTransition;
    use super::BindingTransitionBatch;
    use super::BindingTransitionSequence;
    use super::Bindings;
    use super::ConfiguredDevice;
    use super::HardwareInventory;
    use super::RequestedConfiguration;
    use super::ResolvedBindings;
    use super::ResolvedToDevice;
    use super::RetirementOutcome;
    use super::WaitingWork;
    use super::drain_binding_transitions;
    use super::reconcile_role_entities;
    use super::role_entity_bundle;
    use crate::AttemptEnding;
    use crate::AttemptInvalidation;
    use crate::AttemptRef;
    use crate::DeviceAccessError;
    use crate::DeviceEndpoint;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DeviceRevisionLookup;
    use crate::DiscoveryCadence;
    use crate::DriverAbortReason;
    use crate::DriverContractFailureReport;
    use crate::DriverOutcomeStatus;
    use crate::DriverStopReason;
    use crate::EndpointId;
    use crate::FlowExpectation;
    use crate::LastKnownGoodConfiguration;
    use crate::OnAbort;
    use crate::OnSessionLoss;
    use crate::PartName;
    use crate::RecoveryPolicy;
    use crate::ReporterActivation;
    use crate::ReporterCoverage;
    use crate::ReporterId;
    use crate::RetiredRoleChanged;
    use crate::RetryOn;
    use crate::RiggingAppExt;
    use crate::RiggingLimits;
    use crate::RiggingPlugin;
    use crate::RoleKey;
    use crate::RoleStatusView;
    use crate::UnconfirmedBasis;
    use crate::WaitTiming;
    use crate::WaitingStatusView;
    use crate::reconcile::FrameClockReading;
    use crate::registration::DriverId;
    use crate::registration::RegisteredReporter;
    use crate::registration::ReporterContribution;
    use crate::scheme::AuthoredId;

    #[derive(Component, Reflect)]
    struct TestConfiguration(u8);

    #[derive(Component)]
    #[relationship(relationship_target = RetirementTestRiggingRoleClients)]
    struct RetirementTestRiggingRole(Entity);

    #[derive(Component)]
    #[relationship_target(relationship = RetirementTestRiggingRole)]
    struct RetirementTestRiggingRoleClients(Vec<Entity>);

    #[test]
    fn duplicate_role_and_endpoint_registration_preserve_the_first_binding()
    -> Result<(), Box<dyn Error>> {
        let endpoint = display_endpoint("studio-display")?;
        let first_role = RoleKey::new("primary-window")?;
        let second_role = RoleKey::new("secondary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(first_role.clone(), endpoint.clone()))?;

        assert!(matches!(
            bindings.register(binding(first_role.clone(), display_endpoint("other-display")?)),
            Err(BindingError::RoleAlreadyBound { role }) if role == first_role
        ));
        assert!(matches!(
            bindings.register(binding(second_role, endpoint)),
            Err(BindingError::EndpointAlreadyOwned { owner, .. }) if owner == first_role
        ));
        assert_eq!(
            bindings.roles_for(&device_key("studio-display")?).count(),
            1
        );
        assert!(bindings.binding(&first_role).is_ok());

        Ok(())
    }

    #[test]
    fn failed_replace_keeps_each_existing_reverse_index() -> Result<(), Box<dyn Error>> {
        let first_role = RoleKey::new("primary-window")?;
        let second_role = RoleKey::new("secondary-window")?;
        let first_endpoint = display_endpoint("studio-display")?;
        let second_endpoint = display_endpoint("edit-display")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(first_role.clone(), first_endpoint.clone()))?;
        bindings.register(binding(second_role.clone(), second_endpoint.clone()))?;

        assert!(matches!(
            bindings.replace(binding(first_role.clone(), second_endpoint.clone())),
            Err(BindingError::EndpointAlreadyOwned { owner, .. }) if owner == second_role
        ));
        assert_eq!(bindings.binding(&first_role)?.endpoint, first_endpoint);
        assert_eq!(bindings.binding(&second_role)?.endpoint, second_endpoint);
        assert_eq!(
            bindings.roles_for(&device_key("studio-display")?).count(),
            1
        );
        assert_eq!(bindings.roles_for(&device_key("edit-display")?).count(), 1);

        Ok(())
    }

    #[test]
    fn successful_replace_releases_only_its_old_endpoint() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let old_endpoint = display_endpoint("studio-display")?;
        let new_endpoint = display_endpoint("edit-display")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), old_endpoint.clone()))?;

        let displaced = bindings.replace(binding(role.clone(), new_endpoint.clone()))?;

        assert_eq!(displaced.endpoint, old_endpoint);
        assert_eq!(bindings.binding(&role)?.endpoint, new_endpoint);
        assert_eq!(
            bindings.roles_for(&device_key("studio-display")?).count(),
            0
        );
        assert_eq!(bindings.roles_for(&device_key("edit-display")?).count(), 1);

        Ok(())
    }

    #[test]
    fn retirement_is_idempotent_and_removes_every_index() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let endpoint = display_endpoint("studio-display")?;
        let device_key = endpoint.device.clone();
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), endpoint))?;

        let retirement = bindings.retire(&role)?;

        assert!(matches!(retirement, RetirementOutcome::Retired(_)));
        assert!(matches!(
            bindings.retire(&role)?,
            RetirementOutcome::AlreadyUnbound
        ));
        assert!(matches!(
            bindings.binding(&role),
            Err(BindingError::RoleNotBound { .. })
        ));
        assert_eq!(bindings.roles_for(&device_key).count(), 0);

        Ok(())
    }

    #[test]
    fn one_device_can_serve_several_roles_at_distinct_endpoints() -> Result<(), Box<dyn Error>> {
        let device_key = device_key("control-panel")?;
        let first_role = RoleKey::new("cut")?;
        let second_role = RoleKey::new("fade")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(
            first_role,
            DeviceEndpoint {
                device: device_key.clone(),
                id:     EndpointId::Part(crate::PartName::new("key/1")?),
            },
        ))?;
        bindings.register(binding(
            second_role,
            DeviceEndpoint {
                device: device_key.clone(),
                id:     EndpointId::Part(crate::PartName::new("key/2")?),
            },
        ))?;

        assert_eq!(bindings.roles_for(&device_key).count(), 2);

        Ok(())
    }

    #[test]
    fn transitions_are_monotonic_and_hold_only_lifecycle_metadata() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;
        bindings.replace(binding(role.clone(), display_endpoint("edit-display")?))?;
        let _ = bindings.retire(&role)?;
        let _ = bindings.retire(&role)?;

        let transitions = bindings.take_pending_transitions();
        let sequences = transitions
            .iter()
            .map(|binding_transition| match binding_transition {
                BindingTransition::Registered {
                    sequence,
                    role: transition_role,
                }
                | BindingTransition::Replaced {
                    sequence,
                    role: transition_role,
                    ..
                }
                | BindingTransition::Retired {
                    sequence,
                    role: transition_role,
                    ..
                } => {
                    assert_eq!(transition_role, &role);
                    sequence.0
                },
            })
            .collect::<Vec<_>>();

        assert_eq!(sequences, vec![0, 1, 2]);

        Ok(())
    }

    #[test]
    fn configured_transition_capacity_keeps_register_replace_and_retire_atomic()
    -> Result<(), Box<dyn Error>> {
        let first_role = RoleKey::new("primary-window")?;
        let second_role = RoleKey::new("secondary-window")?;
        let third_role = RoleKey::new("tertiary-window")?;
        let first_endpoint = display_endpoint("studio-display")?;
        let second_endpoint = display_endpoint("edit-display")?;
        let third_endpoint = display_endpoint("presentation-display")?;
        let replacement_endpoint = display_endpoint("replacement-display")?;
        let mut bindings = Bindings::default();
        let capacity = NonZeroUsize::new(2).ok_or("nonzero capacity")?;
        bindings.set_pending_transition_capacity(capacity)?;
        bindings.register(binding(first_role.clone(), first_endpoint.clone()))?;
        bindings.register(binding(second_role.clone(), second_endpoint.clone()))?;

        assert_eq!(
            bindings.register(binding(third_role.clone(), third_endpoint.clone())),
            Err(BindingError::PendingTransitionCapacityReached)
        );
        assert!(matches!(
            bindings.replace(binding(first_role.clone(), replacement_endpoint.clone())),
            Err(BindingError::PendingTransitionCapacityReached)
        ));
        assert!(matches!(
            bindings.retire(&first_role),
            Err(BindingError::PendingTransitionCapacityReached)
        ));
        assert_eq!(bindings.binding(&first_role)?.endpoint, first_endpoint);
        assert_eq!(bindings.binding(&second_role)?.endpoint, second_endpoint);
        assert!(matches!(
            bindings.binding(&third_role),
            Err(BindingError::RoleNotBound { .. })
        ));
        assert_eq!(
            bindings.owner_by_endpoint.get(&first_endpoint),
            Some(&first_role)
        );
        assert_eq!(
            bindings.owner_by_endpoint.get(&second_endpoint),
            Some(&second_role)
        );
        assert!(!bindings.owner_by_endpoint.contains_key(&third_endpoint));
        assert!(
            !bindings
                .owner_by_endpoint
                .contains_key(&replacement_endpoint)
        );
        assert_eq!(
            bindings.roles_by_device.get(&first_endpoint.device),
            Some(&vec![first_role.clone()])
        );
        assert_eq!(
            bindings.roles_by_device.get(&second_endpoint.device),
            Some(&vec![second_role])
        );
        assert!(
            !bindings
                .roles_by_device
                .contains_key(&third_endpoint.device)
        );
        assert!(
            !bindings
                .roles_by_device
                .contains_key(&replacement_endpoint.device)
        );
        assert_eq!(
            bindings.set_pending_transition_capacity(NonZeroUsize::MIN),
            Err(BindingCapacityError::BelowPendingCount {
                capacity: NonZeroUsize::MIN,
                pending:  2,
            })
        );

        Ok(())
    }

    #[test]
    fn device_readdress_moves_all_endpoint_parts_together() -> Result<(), Box<dyn Error>> {
        let saved = device_key("saved-camera")?;
        let adopted = device_key("adopted-camera")?;
        let first_role = RoleKey::new("camera/first-clone")?;
        let second_role = RoleKey::new("camera/second-clone")?;
        let first_part = EndpointId::Part(PartName::new("tool/1")?);
        let second_part = EndpointId::Part(PartName::new("tool/2")?);
        let mut bindings = Bindings::default();
        bindings.register(binding(
            first_role.clone(),
            DeviceEndpoint {
                device: saved.clone(),
                id:     first_part.clone(),
            },
        ))?;
        bindings.register(binding(
            second_role.clone(),
            DeviceEndpoint {
                device: saved.clone(),
                id:     second_part.clone(),
            },
        ))?;

        bindings.validate_device_readdress(&saved, &adopted)?;
        bindings.readdress_device(&saved, adopted.clone())?;

        assert_eq!(
            bindings.binding(&first_role)?.endpoint,
            DeviceEndpoint {
                device: adopted.clone(),
                id:     first_part,
            }
        );
        assert_eq!(
            bindings.binding(&second_role)?.endpoint,
            DeviceEndpoint {
                device: adopted.clone(),
                id:     second_part,
            }
        );
        assert_eq!(
            bindings.roles_for(&saved).collect::<Vec<_>>(),
            Vec::<&RoleKey>::new()
        );
        assert_eq!(bindings.roles_for(&adopted).count(), 2);
        Ok(())
    }

    #[test]
    fn readdressed_roles_begin_a_new_registration_application_run() -> Result<(), Box<dyn Error>> {
        let saved = device_key("saved-camera")?;
        let adopted = device_key("adopted-camera")?;
        let first_role = RoleKey::new("camera/first-clone")?;
        let second_role = RoleKey::new("camera/second-clone")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(
            first_role.clone(),
            DeviceEndpoint {
                device: saved.clone(),
                id:     EndpointId::Part(PartName::new("tool/1")?),
            },
        ))?;
        bindings.register(binding(
            second_role.clone(),
            DeviceEndpoint {
                device: saved.clone(),
                id:     EndpointId::Part(PartName::new("tool/2")?),
            },
        ))?;
        for role in [&first_role, &second_role] {
            bindings
                .by_role
                .get_mut(role)
                .ok_or("registered role is absent")?
                .application_run = super::RegistrationApplicationRun::Ended {
                applications: NonZeroU32::new(2).ok_or("two is nonzero")?,
                last_ending:  AttemptEnding::Invalidated(AttemptInvalidation::RoleRetired),
            };
            assert!(matches!(
                bindings
                    .by_role
                    .get(role)
                    .ok_or("registered role is absent")?
                    .application_run
                    .view(),
                crate::RegistrationApplicationRunView::Reapplying { applications, .. }
                    if applications.get() == 2
            ));
        }

        bindings.readdress(&first_role, adopted.clone())?;
        bindings.readdress_device(&saved, adopted)?;

        for role in [&first_role, &second_role] {
            assert_eq!(
                bindings
                    .by_role
                    .get(role)
                    .ok_or("registered role is absent")?
                    .application_run
                    .view(),
                crate::RegistrationApplicationRunView::Initial
            );
        }
        Ok(())
    }

    #[test]
    fn device_readdress_capacity_refusal_changes_no_role_or_index() -> Result<(), Box<dyn Error>> {
        let saved = device_key("saved-camera")?;
        let adopted = device_key("adopted-camera")?;
        let first_role = RoleKey::new("camera/first-clone")?;
        let second_role = RoleKey::new("camera/second-clone")?;
        let first_endpoint = DeviceEndpoint {
            device: saved.clone(),
            id:     EndpointId::Part(PartName::new("tool/1")?),
        };
        let second_endpoint = DeviceEndpoint {
            device: saved.clone(),
            id:     EndpointId::Part(PartName::new("tool/2")?),
        };
        let mut bindings = Bindings::default();
        bindings.register(binding(first_role.clone(), first_endpoint.clone()))?;
        bindings.register(binding(second_role.clone(), second_endpoint.clone()))?;
        bindings.set_pending_transition_capacity(
            NonZeroUsize::new(3).ok_or("nonzero transition capacity")?,
        )?;

        assert_eq!(
            bindings.validate_device_readdress(&saved, &adopted),
            Err(BindingError::PendingTransitionCapacityReached)
        );
        assert_eq!(
            bindings.readdress_device(&saved, adopted.clone()),
            Err(BindingError::PendingTransitionCapacityReached)
        );
        assert_eq!(bindings.binding(&first_role)?.endpoint, first_endpoint);
        assert_eq!(bindings.binding(&second_role)?.endpoint, second_endpoint);
        assert_eq!(bindings.roles_for(&saved).count(), 2);
        assert_eq!(bindings.roles_for(&adopted).count(), 0);
        Ok(())
    }

    #[test]
    fn default_transition_capacity_rejects_another_registration_without_index_mutation()
    -> Result<(), Box<dyn Error>> {
        let mut bindings = Bindings::default();

        for index in 0..super::DEFAULT_PENDING_TRANSITION_CAPACITY {
            let role = RoleKey::new(format!("default-capacity-role-{index}"))?;
            let endpoint = display_endpoint(&format!("default-capacity-device-{index}"))?;
            bindings.register(binding(role, endpoint))?;
        }

        let overflow_role = RoleKey::new("default-capacity-overflow")?;
        let overflow_endpoint = display_endpoint("default-capacity-overflow-device")?;
        assert_eq!(
            bindings.register(binding(overflow_role.clone(), overflow_endpoint.clone())),
            Err(BindingError::PendingTransitionCapacityReached)
        );
        assert!(matches!(
            bindings.binding(&overflow_role),
            Err(BindingError::RoleNotBound { .. })
        ));
        assert!(!bindings.owner_by_endpoint.contains_key(&overflow_endpoint));
        assert!(
            !bindings
                .roles_by_device
                .contains_key(&overflow_endpoint.device)
        );
        assert_eq!(
            bindings.pending_transitions.queue.len(),
            super::DEFAULT_PENDING_TRANSITION_CAPACITY
        );

        Ok(())
    }

    #[test]
    fn transition_sequence_exhaustion_keeps_all_binding_indexes_unchanged()
    -> Result<(), Box<dyn Error>> {
        let first_role = RoleKey::new("last-sequence-role")?;
        let first_endpoint = display_endpoint("last-sequence-device")?;
        let second_role = RoleKey::new("exhausted-sequence-role")?;
        let second_endpoint = display_endpoint("exhausted-sequence-device")?;
        let mut bindings = Bindings {
            next_transition_sequence: u64::MAX - 1,
            ..Default::default()
        };

        bindings.register(binding(first_role.clone(), first_endpoint.clone()))?;
        assert!(matches!(
            bindings.pending_transitions.queue.front(),
            Some(BindingTransition::Registered { sequence, role })
                if sequence.0 == u64::MAX - 1 && role == &first_role
        ));
        assert_eq!(bindings.next_transition_sequence, u64::MAX);

        assert_eq!(
            bindings.register(binding(second_role.clone(), second_endpoint.clone())),
            Err(BindingError::TransitionSequenceExhausted)
        );
        assert_eq!(bindings.binding(&first_role)?.endpoint, first_endpoint);
        assert!(matches!(
            bindings.binding(&second_role),
            Err(BindingError::RoleNotBound { .. })
        ));
        assert_eq!(
            bindings.owner_by_endpoint.get(&first_endpoint),
            Some(&first_role)
        );
        assert!(!bindings.owner_by_endpoint.contains_key(&second_endpoint));
        assert_eq!(
            bindings.roles_by_device.get(&first_endpoint.device),
            Some(&vec![first_role])
        );
        assert!(
            !bindings
                .roles_by_device
                .contains_key(&second_endpoint.device)
        );
        assert_eq!(bindings.next_transition_sequence, u64::MAX);

        Ok(())
    }

    #[test]
    fn binding_inventory_and_reflected_configuration_types_register_automatically() {
        let app = App::new();
        let world = app.world();
        let type_registry = world.resource::<AppTypeRegistry>().read();

        for type_id in [
            TypeId::of::<Bindings>(),
            TypeId::of::<HardwareInventory>(),
            TypeId::of::<Binding>(),
            TypeId::of::<ConfiguredDevice>(),
        ] {
            assert!(type_registry.contains(type_id));
        }
        drop(type_registry);
    }

    fn binding(role: RoleKey, endpoint: DeviceEndpoint) -> Binding {
        Binding {
            role,
            endpoint,
            driver: DriverId(0),
            recovery: RecoveryPolicy::default(),
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            requested: RequestedConfiguration::new(TestConfiguration(3)),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline: ApplyDeadline::ProcessDefault,
            flow_expectation: FlowExpectation::NotMonitored,
        }
    }

    fn display_endpoint(value: &str) -> Result<DeviceEndpoint, Box<dyn Error>> {
        Ok(DeviceEndpoint {
            device: device_key(value)?,
            id:     EndpointId::Whole,
        })
    }

    fn device_key(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new(value)?,
            },
        })
    }

    // --- binding entities, the frame batch, and the resolved-device relationship ---

    /// Build an app with the kernel plugin and one authored role bound to a fresh endpoint.
    fn app_with_role(role: &str) -> Result<(App, RoleKey), Box<dyn Error>> {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let role = RoleKey::new(role)?;
        let binding = test_binding(role.clone(), endpoint_named(role.as_str())?);
        let reserved_transition = app
            .world()
            .resource::<Bindings>()
            .reserve_registration(&binding)?;
        let entity = app.world_mut().spawn(role_entity_bundle(&binding)).id();
        app.world_mut()
            .resource_mut::<Bindings>()
            .register_reserved(binding, entity, reserved_transition);

        Ok((app, role))
    }

    fn endpoint_named(value: &str) -> Result<DeviceEndpoint, Box<dyn Error>> {
        Ok(DeviceEndpoint {
            device: DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored {
                    value: AuthoredId::new(value)?,
                },
            },
            id:     EndpointId::Whole,
        })
    }

    fn test_binding(role: RoleKey, endpoint: DeviceEndpoint) -> Binding {
        Binding {
            role,
            endpoint,
            driver: DriverId(0),
            recovery: RecoveryPolicy::Forget,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            requested: RequestedConfiguration::new(()),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline: ApplyDeadline::ProcessDefault,
            flow_expectation: FlowExpectation::NotMonitored,
        }
    }

    fn registered_entity(app: &App, role: &RoleKey) -> Entity {
        app.world()
            .resource::<Bindings>()
            .role_entity(role)
            .unwrap_or_else(|_| panic!("role `{role}` has no binding entity"))
    }

    #[derive(Default, Resource)]
    struct ObservedRoleRetirements(Vec<(RoleKey, DeviceEndpoint)>);

    fn observe_role_retired(
        retired_role_changed: On<RetiredRoleChanged>,
        mut observed: ResMut<ObservedRoleRetirements>,
    ) {
        observed.0.push((
            retired_role_changed.role.clone(),
            retired_role_changed.endpoint.clone(),
        ));
    }

    #[test]
    fn registration_spawns_one_binding_entity_per_role_with_no_reporter_running()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        let second_role = RoleKey::new("window/inspector")?;
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(test_binding(
                second_role.clone(),
                endpoint_named(second_role.as_str())?,
            ))?;

        app.update();

        assert_eq!(
            app.world()
                .resource::<Bindings>()
                .registered_role_entities()
                .count(),
            2
        );
        assert_ne!(
            registered_entity(&app, &role),
            registered_entity(&app, &second_role)
        );
        assert!(
            app.world()
                .get::<crate::RoleStatus>(registered_entity(&app, &role))
                .is_some()
        );
        assert!(
            app.world()
                .get::<crate::RoleStatus>(registered_entity(&app, &second_role))
                .is_some()
        );
        assert!(
            app.world()
                .resource::<Bindings>()
                .role_entity(&RoleKey::new("window/never-registered")?)
                .is_err()
        );

        Ok(())
    }

    #[test]
    fn a_binding_entity_outlives_every_frame_in_which_its_role_has_no_device()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);

        for _ in 0..4 {
            app.update();
        }

        assert_eq!(registered_entity(&app, &role), entity);
        assert!(app.world().get_entity(entity).is_ok());
        assert!(app.world().get::<crate::RoleStatus>(entity).is_some());

        Ok(())
    }

    #[test]
    fn a_binding_entity_stripped_of_its_mirrors_is_repaired_and_stays_indexed()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);

        // A Bevy Remote Protocol *remove* takes the mirrored components off a live entity, which is
        // what separates this from a despawn: the role is still registered.
        app.world_mut()
            .entity_mut(entity)
            .remove::<(RoleKey, RecoveryPolicy, crate::RoleStatus)>();
        app.update();

        assert_eq!(registered_entity(&app, &role), entity);
        assert_eq!(app.world().get::<RoleKey>(entity), Some(&role));
        assert_eq!(
            app.world().get::<RecoveryPolicy>(entity),
            Some(&RecoveryPolicy::Forget)
        );
        assert!(app.world().get::<crate::RoleStatus>(entity).is_some());

        Ok(())
    }

    #[test]
    fn retirement_despawns_the_binding_entity_on_a_frame_with_no_reporter_completion()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);
        app.world_mut().resource_mut::<Bindings>().retire(&role)?;

        app.update();

        assert!(
            app.world()
                .resource::<Bindings>()
                .role_entity(&role)
                .is_err()
        );
        assert!(app.world().get_entity(entity).is_err());

        Ok(())
    }

    #[test]
    fn retirement_removes_lost_role_entity_and_relationship_recovery_records()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.register_rigging_role_relationship::<RetirementTestRiggingRole>();
        app.update();
        let role_entity = registered_entity(&app, &role);
        app.world_mut()
            .spawn(RetirementTestRiggingRole(role_entity));

        assert!(app.world_mut().despawn(role_entity));
        assert!(
            app.world()
                .resource::<super::LostRegisteredRoleEntities>()
                .0
                .contains(&role)
        );
        assert!(
            app.world()
                .resource::<super::LostRoleRelationships>()
                .0
                .contains_key(&role)
        );

        app.world_mut().resource_mut::<Bindings>().retire(&role)?;
        app.update();

        assert!(
            !app.world()
                .resource::<super::LostRegisteredRoleEntities>()
                .0
                .contains(&role)
        );
        assert!(
            !app.world()
                .resource::<super::LostRoleRelationships>()
                .0
                .contains_key(&role)
        );
        Ok(())
    }

    #[test]
    fn retirement_announces_the_role_and_endpoint_when_its_queued_transition_applies()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.init_resource::<ObservedRoleRetirements>()
            .add_observer(observe_role_retired);
        app.update();
        let endpoint = app
            .world()
            .resource::<Bindings>()
            .binding(&role)?
            .endpoint
            .clone();

        app.world_mut().resource_mut::<Bindings>().retire(&role)?;
        assert!(
            app.world()
                .resource::<ObservedRoleRetirements>()
                .0
                .is_empty()
        );

        app.update();

        assert_eq!(
            app.world().resource::<ObservedRoleRetirements>().0,
            vec![(role, endpoint)]
        );
        Ok(())
    }

    #[test]
    fn one_drain_moves_every_pending_transition_in_sequence_and_later_work_waits_a_frame()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        let late_role = RoleKey::new("window/late")?;
        app.world_mut().resource_mut::<Bindings>().retire(&role)?;
        // The batch lives only inside the frame that drained it, so the sequences have to be read
        // from a system rather than from the world once the frame has ended.
        app.init_resource::<ObservedBatches>().add_systems(
            Update,
            observe_batch
                .after(reconcile_role_entities)
                .before(crate::reconcile::reconcile),
        );

        app.update();

        let sequences: Vec<u64> = app.world().resource::<ObservedBatches>().0[0]
            .iter()
            .map(|sequence| sequence.0)
            .collect();
        assert_eq!(sequences, vec![0, 1]);
        assert!(
            app.world_mut()
                .resource_mut::<Bindings>()
                .take_pending_transitions()
                .is_empty()
        );

        // Submitted after this frame's drain: it stays in `Bindings` until the next frame.
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(test_binding(
                late_role.clone(),
                endpoint_named(late_role.as_str())?,
            ))?;
        let reserved = app.world().resource::<Bindings>().role_entity(&late_role)?;
        assert!(app.world().get_entity(reserved).is_err());

        app.update();

        assert_eq!(app.world().resource::<ObservedBatches>().0[1].len(), 1);
        assert!(
            app.world()
                .get_entity(registered_entity(&app, &late_role))
                .is_ok()
        );

        Ok(())
    }

    #[derive(Default, Resource)]
    struct FramesWithChangedBindings(usize);

    fn count_frames_with_changed_bindings(
        bindings: Res<Bindings>,
        mut frames_with_changed_bindings: ResMut<FramesWithChangedBindings>,
    ) {
        if bindings.is_changed() {
            frames_with_changed_bindings.0 += 1;
        }
    }

    #[test]
    fn a_frame_with_no_submitted_binding_operation_leaves_bindings_unchanged()
    -> Result<(), Box<dyn Error>> {
        let (mut app, _) = app_with_role("window/main")?;
        app.init_resource::<FramesWithChangedBindings>()
            .add_systems(
                Update,
                count_frames_with_changed_bindings.after(drain_binding_transitions),
            );

        app.update();

        assert_eq!(app.world().resource::<FramesWithChangedBindings>().0, 1);

        for _ in 0..3 {
            app.update();
        }

        // The drain took nothing on those frames, so it never asked `Bindings` for mutable access.
        assert_eq!(app.world().resource::<FramesWithChangedBindings>().0, 1);

        Ok(())
    }

    #[derive(Default, Resource)]
    struct ObservedBatches(Vec<Vec<BindingTransitionSequence>>);

    fn observe_batch(
        binding_transition_batch: Res<BindingTransitionBatch>,
        mut observed_batches: ResMut<ObservedBatches>,
    ) {
        observed_batches.0.push(
            binding_transition_batch
                .transitions()
                .iter()
                .map(|binding_transition| match binding_transition {
                    BindingTransition::Registered { sequence, .. }
                    | BindingTransition::Replaced { sequence, .. }
                    | BindingTransition::Retired { sequence, .. } => *sequence,
                })
                .collect(),
        );
    }

    #[test]
    fn entity_lifecycle_attempts_and_events_observe_one_identical_ordered_batch()
    -> Result<(), Box<dyn Error>> {
        let (mut app, _) = app_with_role("window/main")?;
        // Three stand-ins for the binding-entity stage, the attempt aborts, and the event stage:
        // each reads the batch after the drain and none of them removes an entry.
        app.init_resource::<ObservedBatches>().add_systems(
            Update,
            (observe_batch, observe_batch, observe_batch)
                .chain()
                .after(reconcile_role_entities)
                .before(crate::reconcile::reconcile),
        );

        app.update();

        let observed_batches = app.world().resource::<ObservedBatches>();
        assert_eq!(observed_batches.0.len(), 3);
        assert!(
            observed_batches
                .0
                .iter()
                .all(|observed| observed == &observed_batches.0[0])
        );
        assert_eq!(observed_batches.0[0].len(), 1);
        // The batch survives every consumer inside the frame and is emptied by the clearing system
        // ordered after `crate::RiggingSystems::Apply`, so no later frame reads a stale transition.
        assert!(
            app.world()
                .resource::<BindingTransitionBatch>()
                .transitions()
                .is_empty()
        );

        Ok(())
    }

    #[test]
    fn a_reflection_write_to_the_mirrored_recovery_policy_is_overwritten_next_reconcile()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);
        assert_eq!(
            app.world().get::<RecoveryPolicy>(entity),
            Some(&RecoveryPolicy::Forget)
        );

        // What a Bevy Remote Protocol mutation does: write the mirrored component directly.
        *app.world_mut()
            .get_mut::<RecoveryPolicy>(entity)
            .expect("the binding entity mirrors its recovery policy") =
            RecoveryPolicy::ReapplyOnReturn;

        app.update();

        assert_eq!(
            app.world().get::<RecoveryPolicy>(entity),
            Some(&RecoveryPolicy::Forget)
        );
        assert_eq!(
            app.world().resource::<Bindings>().binding(&role)?.recovery,
            RecoveryPolicy::Forget
        );

        Ok(())
    }

    #[test]
    fn resolving_and_replacing_the_link_maintains_the_device_reverse_collection()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);
        let first_device = app.world_mut().spawn_empty().id();
        let second_device = app.world_mut().spawn_empty().id();

        app.world_mut()
            .entity_mut(entity)
            .insert(<ResolvedToDevice as Relationship>::from(first_device));

        assert_eq!(
            resolved_binding_entities(app.world(), first_device),
            vec![entity]
        );

        app.world_mut()
            .entity_mut(entity)
            .insert(<ResolvedToDevice as Relationship>::from(second_device));

        assert!(resolved_binding_entities(app.world(), first_device).is_empty());
        assert_eq!(
            resolved_binding_entities(app.world(), second_device),
            vec![entity]
        );

        Ok(())
    }

    fn resolved_binding_entities(world: &World, device: Entity) -> Vec<Entity> {
        world
            .get::<ResolvedBindings>(device)
            .map(|resolved_bindings| resolved_bindings.iter().collect())
            .unwrap_or_default()
    }

    #[test]
    fn despawning_a_live_device_removes_the_link_and_leaves_its_binding_entities_alive()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);
        let device = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(entity)
            .insert(<ResolvedToDevice as Relationship>::from(device));

        app.world_mut().entity_mut(device).despawn();

        assert!(app.world().get_entity(entity).is_ok());
        assert!(app.world().get::<ResolvedToDevice>(entity).is_none());
        assert_eq!(
            app.world().resource::<Bindings>().role_entity(&role),
            Ok(entity)
        );
        assert!(app.world().resource::<Bindings>().binding(&role).is_ok());

        Ok(())
    }

    #[test]
    fn two_roles_on_one_device_share_a_reverse_collection_while_duplicates_stay_rejected()
    -> Result<(), Box<dyn Error>> {
        let device_key = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("stream-deck")?,
            },
        };
        let key_endpoint = DeviceEndpoint {
            device: device_key.clone(),
            id:     EndpointId::Part(PartName::new("key/3")?),
        };
        let dial_endpoint = DeviceEndpoint {
            device: device_key,
            id:     EndpointId::Part(PartName::new("dial/1")?),
        };
        let key_role = RoleKey::new("deck/key")?;
        let dial_role = RoleKey::new("deck/dial")?;
        let duplicate_role = RoleKey::new("deck/duplicate")?;
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        {
            let mut bindings = app.world_mut().resource_mut::<Bindings>();
            bindings.register(test_binding(key_role.clone(), key_endpoint.clone()))?;
            bindings.register(test_binding(dial_role.clone(), dial_endpoint))?;
            assert!(matches!(
                bindings.register(test_binding(duplicate_role, key_endpoint)),
                Err(BindingError::EndpointAlreadyOwned { .. })
            ));
        }

        app.update();

        let device = app.world_mut().spawn_empty().id();
        for role in [&key_role, &dial_role] {
            let entity = registered_entity(&app, role);
            app.world_mut()
                .entity_mut(entity)
                .insert(<ResolvedToDevice as Relationship>::from(device));
        }

        let resolved = resolved_binding_entities(app.world(), device);
        assert_eq!(resolved.len(), 2);
        assert!(resolved.contains(&registered_entity(&app, &key_role)));
        assert!(resolved.contains(&registered_entity(&app, &dial_role)));

        Ok(())
    }

    #[test]
    fn binding_entity_components_register_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();

        for type_id in [
            TypeId::of::<RoleKey>(),
            TypeId::of::<RecoveryPolicy>(),
            TypeId::of::<ResolvedToDevice>(),
            TypeId::of::<ResolvedBindings>(),
        ] {
            assert!(type_registry.contains(type_id));
            assert!(
                type_registry
                    .get_type_data::<ReflectComponent>(type_id)
                    .is_some()
            );
        }

        drop(type_registry);
    }

    /// A different reading of the same role's device, which is what opens a gate waiting on one.
    fn changed(device_revision: crate::DeviceRevisionLookup) -> crate::DeviceRevisionLookup {
        match device_revision {
            DeviceRevisionLookup::Retired => {
                DeviceRevisionLookup::Retained(crate::DeviceRevision::default())
            },
            DeviceRevisionLookup::Retained(device_revision) => {
                DeviceRevisionLookup::Retained(device_revision.advanced())
            },
        }
    }

    fn transport_failure() -> DeviceAccessError {
        DeviceAccessError::Transport {
            detail: "the device transport failed".to_owned(),
        }
    }

    fn measurable() -> FrameClockReading {
        FrameClockReading::Measurable(bevy::platform::time::Instant::now())
    }

    /// Read the generation the role's binding currently holds, as an ending dispatched under that
    /// binding would carry it.
    fn generation_now(bindings: &Bindings, role: &RoleKey) -> super::BindingGeneration {
        bindings
            .generation(role)
            .expect("the test registered a binding for this role")
    }

    fn authorized_attempt(
        bindings: &Bindings,
        role: &RoleKey,
        endpoint: DeviceEndpoint,
        attempt: u64,
        started_at: Instant,
    ) -> super::AuthorizedApplyAttempt {
        super::AuthorizedApplyAttempt::new(
            AttemptRef::new(attempt),
            generation_now(bindings, role),
            endpoint,
            crate::DeviceId::new(1),
            crate::DeviceRevision::default(),
            started_at,
            started_at + Duration::from_secs(5),
        )
    }

    #[test]
    fn set_wait_preserves_identity_reports_reason_changes_and_crosses_one_schedule_bound()
    -> Result<(), Box<dyn Error>> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(1)));
        app.update();
        let first_frame = app
            .world()
            .resource::<Time<Real>>()
            .last_update()
            .ok_or("manual time did not advance")?;

        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;
        let deadline = first_frame + Duration::from_secs(2);
        assert_eq!(
            bindings.set_wait(
                &role,
                first_frame,
                super::WaitingCondition::KernelRetry(super::RetryGate::AwaitingInstant(deadline)),
            ),
            super::WaitTransition::ReasonChanged
        );
        let wait_start = match &bindings
            .by_role
            .get(&role)
            .ok_or("registered role is absent")?
            .state
        {
            super::RoleState::Waiting(waiting) => waiting.wait_start,
            _ => return Err("set_wait did not enter Waiting".into()),
        };
        assert_eq!(wait_start, super::WaitStart::At(first_frame));

        app.update();
        let before_deadline = app
            .world()
            .resource::<Time<Real>>()
            .last_update()
            .ok_or("manual time did not advance")?;
        assert_eq!(
            bindings.set_wait(
                &role,
                before_deadline,
                super::WaitingCondition::KernelRetry(super::RetryGate::AwaitingInstant(deadline)),
            ),
            super::WaitTransition::Unchanged
        );
        let retained_wait_start = match &bindings
            .by_role
            .get(&role)
            .ok_or("registered role is absent")?
            .state
        {
            super::RoleState::Waiting(waiting) => waiting.wait_start,
            _ => return Err("set_wait did not retain Waiting".into()),
        };
        assert_eq!(retained_wait_start, wait_start);

        app.update();
        let at_deadline = app
            .world()
            .resource::<Time<Real>>()
            .last_update()
            .ok_or("manual time did not advance")?;
        assert_eq!(
            bindings.set_wait(
                &role,
                at_deadline,
                super::WaitingCondition::KernelRetry(super::RetryGate::AwaitingInstant(deadline)),
            ),
            super::WaitTransition::BoundCrossed
        );
        assert_eq!(
            bindings.set_wait(
                &role,
                at_deadline,
                super::WaitingCondition::KernelRetry(super::RetryGate::AwaitingInstant(deadline)),
            ),
            super::WaitTransition::Unchanged
        );
        assert_eq!(
            bindings.set_wait(
                &role,
                at_deadline,
                super::WaitingCondition::ApplicationReapply,
            ),
            super::WaitTransition::ReasonChanged
        );

        Ok(())
    }

    fn bindings_with_status_clock(
        role_name: &str,
    ) -> Result<(Bindings, RoleKey, Instant), Box<dyn Error>> {
        let now = bevy::platform::time::Instant::now();
        let role = RoleKey::new(role_name)?;
        let mut bindings = Bindings::default();
        bindings.install_role_status_clock(crate::RiggingRuntimeClock::starting_at(now));
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;
        Ok((bindings, role, now))
    }

    #[test]
    fn recreate_session_loss_enters_kernel_retry_without_application_work()
    -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, _) = bindings_with_status_clock("recreate-session-loss")?;
        bindings
            .by_role
            .get_mut(&role)
            .ok_or("registered role is absent")?
            .binding
            .on_loss = OnSessionLoss::Recreate;

        assert!(matches!(
            bindings.apply_session_loss(
                &role,
                DeviceRevisionLookup::Retained(crate::DeviceRevision::default()),
                measurable(),
            ),
            super::SessionLossApplication::Applied
        ));
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Waiting(crate::WaitingStatusView::KernelRetry { .. })
        ));
        assert_eq!(bindings.waiting_work(&role), WaitingWork::Nothing);
        assert!(matches!(
            bindings.retry_pacing(&role),
            super::RetryPacing::Blocked(_)
        ));
        Ok(())
    }

    #[test]
    fn session_loss_for_a_retired_device_paces_on_the_device_returning()
    -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, _) = bindings_with_status_clock("retired-device-session-loss")?;
        bindings
            .by_role
            .get_mut(&role)
            .ok_or("registered role is absent")?
            .binding
            .on_loss = OnSessionLoss::Recreate;

        assert!(matches!(
            bindings.apply_session_loss(&role, DeviceRevisionLookup::Retired, measurable()),
            super::SessionLossApplication::Applied
        ));
        let super::RetryPacing::Blocked(gate) = bindings.retry_pacing(&role) else {
            return Err("a role whose device retired is still owed its pacing".into());
        };
        assert!(!gate.opened(DeviceRevisionLookup::Retired, measurable()));
        assert!(gate.opened(
            DeviceRevisionLookup::Retained(crate::DeviceRevision::default()),
            measurable()
        ));
        Ok(())
    }

    #[test]
    fn report_only_session_loss_waits_for_application_reapply() -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, _) = bindings_with_status_clock("report-only-session-loss")?;
        bindings
            .by_role
            .get_mut(&role)
            .ok_or("registered role is absent")?
            .binding
            .on_loss = OnSessionLoss::ReportOnly;

        assert!(matches!(
            bindings.apply_session_loss(
                &role,
                DeviceRevisionLookup::Retained(crate::DeviceRevision::default()),
                measurable(),
            ),
            super::SessionLossApplication::Applied
        ));
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Waiting(crate::WaitingStatusView::ApplicationReapply { .. })
        ));
        assert_eq!(
            bindings.waiting_work(&role),
            WaitingWork::ReapplyRequestOwed
        );
        assert_eq!(bindings.retry_pacing(&role), super::RetryPacing::Ready);
        Ok(())
    }

    #[test]
    fn a_bound_role_without_the_runtime_clock_reports_the_missing_clock()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("clockless-role")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        assert!(matches!(
            bindings.projected_status(&role),
            Err(crate::BindingError::RoleStatusClockNotInstalled { role: affected })
                if affected == role
        ));
        Ok(())
    }

    #[test]
    fn set_wait_queues_one_changed_status_view() -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, now) = bindings_with_status_clock("set-wait")?;

        assert_eq!(
            bindings.set_wait(&role, now, super::WaitingCondition::ApplicationReapply),
            super::WaitTransition::ReasonChanged
        );

        let changes = bindings.take_status_changes();
        assert!(matches!(
            changes.as_slice(),
            [super::RoleStatusChange {
                from: crate::RoleStatusView::Waiting(
                    crate::WaitingStatusView::NewRegistration { .. }
                ),
                to: crate::RoleStatusView::Waiting(
                    crate::WaitingStatusView::ApplicationReapply { .. }
                ),
                ..
            }]
        ));
        Ok(())
    }

    #[test]
    fn set_role_apply_failure_run_queues_one_changed_status_view() -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, now) = bindings_with_status_clock("failure-run")?;
        let retry_at = now + Duration::from_secs(5);
        bindings.set_wait(
            &role,
            now,
            super::WaitingCondition::KernelRetry(super::RetryGate::AwaitingInstant(retry_at)),
        );
        bindings.take_status_changes();

        bindings.set_role_apply_failure_run(
            &role,
            super::RoleApplyFailureRun::Consecutive {
                failures:    NonZeroU32::MIN,
                last_ending: AttemptEnding::Invalidated(AttemptInvalidation::RoleRetired),
            },
        );

        let changes = bindings.take_status_changes();
        assert!(matches!(
            changes.as_slice(),
            [super::RoleStatusChange {
                from: crate::RoleStatusView::Waiting(
                    crate::WaitingStatusView::KernelRetry {
                        failures: crate::RoleApplyFailureRunView::Clear,
                        ..
                    }
                ),
                to: crate::RoleStatusView::Waiting(
                    crate::WaitingStatusView::KernelRetry {
                        failures: crate::RoleApplyFailureRunView::Consecutive { failures, .. },
                        ..
                    }
                ),
                ..
            }] if *failures == NonZeroU32::MIN
        ));
        Ok(())
    }

    #[test]
    fn set_role_retry_pacing_queues_one_changed_status_view() -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, now) = bindings_with_status_clock("retry-pacing")?;
        let retry_at = now + Duration::from_secs(5);
        bindings.set_wait(
            &role,
            now,
            super::WaitingCondition::DriverRepair {
                error: DriverContractFailureReport::DriverNotRegistered {
                    driver: DriverId(7),
                },
            },
        );
        bindings.take_status_changes();

        bindings.set_role_retry_pacing(
            &role,
            super::RetryPacing::Blocked(super::RetryGate::AwaitingInstant(retry_at)),
        );

        let changes = bindings.take_status_changes();
        assert!(matches!(
            changes.as_slice(),
            [super::RoleStatusChange {
                from: crate::RoleStatusView::Waiting(crate::WaitingStatusView::DriverRepair {
                    retry: crate::RetryScheduleView::Ready,
                    ..
                }),
                to: crate::RoleStatusView::Waiting(crate::WaitingStatusView::DriverRepair {
                    retry: crate::RetryScheduleView::Blocked(
                        crate::RetryGateView::TimeReached { .. }
                    ),
                    timing: crate::WaitTiming::Bounded { .. },
                    ..
                }),
                ..
            }]
        ));
        Ok(())
    }

    #[test]
    fn set_waiting_work_queues_one_changed_status_view() -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, _) = bindings_with_status_clock("waiting-work")?;

        bindings.set_waiting_work(&role, WaitingWork::ReapplyRequestOwed);

        let changes = bindings.take_status_changes();
        assert!(matches!(
            changes.as_slice(),
            [super::RoleStatusChange {
                from: crate::RoleStatusView::Waiting(
                    crate::WaitingStatusView::NewRegistration { .. }
                ),
                to: crate::RoleStatusView::Waiting(
                    crate::WaitingStatusView::ApplicationReapply { .. }
                ),
                ..
            }]
        ));
        Ok(())
    }

    fn assert_equal_status_mutation_queues_nothing_and_leaves_component_tick_untouched(
        role_name: &str,
        mutation: impl FnOnce(&mut Bindings, &RoleKey, Instant) -> Result<(), Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role(role_name)?;
        app.update();
        app.update();
        let now = bevy::platform::time::Instant::now();
        let binding_entity = registered_entity(&app, &role);
        let tick_before = app
            .world()
            .entity(binding_entity)
            .get_ref::<crate::RoleStatus>()
            .ok_or("the binding entity has no readable role status")?
            .last_changed();

        {
            let mut bindings = app.world_mut().resource_mut::<Bindings>();
            mutation(&mut bindings, &role, now)?;
            assert!(!bindings.has_status_changes());
        }
        app.update();

        let tick_after = app
            .world()
            .entity(binding_entity)
            .get_ref::<crate::RoleStatus>()
            .ok_or("the binding entity lost its readable role status")?
            .last_changed();
        assert_eq!(tick_before, tick_after);
        Ok(())
    }

    #[test]
    fn equal_set_wait_queues_nothing_and_leaves_the_component_tick_untouched()
    -> Result<(), Box<dyn Error>> {
        assert_equal_status_mutation_queues_nothing_and_leaves_component_tick_untouched(
            "equal-set-wait",
            |bindings, role, now| {
                let condition = match &bindings
                    .by_role
                    .get(role)
                    .ok_or("the registered role is absent")?
                    .state
                {
                    super::RoleState::Waiting(waiting_state) => waiting_state.condition.clone(),
                    _ => return Err("the registered role is not waiting".into()),
                };
                assert_eq!(
                    bindings.set_wait(role, now, condition),
                    super::WaitTransition::Unchanged
                );
                Ok(())
            },
        )
    }

    #[test]
    fn equal_set_role_apply_failure_run_queues_nothing_and_leaves_the_component_tick_untouched()
    -> Result<(), Box<dyn Error>> {
        assert_equal_status_mutation_queues_nothing_and_leaves_component_tick_untouched(
            "equal-failure-run",
            |bindings, role, _| {
                let failures = bindings.role_apply_failure_run(role);
                bindings.set_role_apply_failure_run(role, failures);
                Ok(())
            },
        )
    }

    #[test]
    fn equal_status_mutation_queues_nothing_and_leaves_the_component_tick_untouched()
    -> Result<(), Box<dyn Error>> {
        assert_equal_status_mutation_queues_nothing_and_leaves_component_tick_untouched(
            "equal-retry-pacing",
            |bindings, role, _| {
                let retry_pacing = bindings.retry_pacing(role);
                bindings.set_role_retry_pacing(role, retry_pacing);
                Ok(())
            },
        )
    }

    #[test]
    fn equal_set_waiting_work_queues_nothing_and_leaves_the_component_tick_untouched()
    -> Result<(), Box<dyn Error>> {
        assert_equal_status_mutation_queues_nothing_and_leaves_component_tick_untouched(
            "equal-waiting-work",
            |bindings, role, _| {
                let waiting_work = match &bindings
                    .by_role
                    .get(role)
                    .ok_or("the registered role is absent")?
                    .state
                {
                    super::RoleState::Waiting(waiting_state) => waiting_state.waiting_work,
                    _ => return Err("the registered role is not waiting".into()),
                };
                bindings.set_waiting_work(role, waiting_work);
                Ok(())
            },
        )
    }

    #[test]
    fn a_revision_gated_refusal_queues_one_bounded_status_with_its_cause()
    -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, now) = bindings_with_status_clock("refused-dispatch")?;
        let wait_bound = now + Duration::from_secs(5);

        assert_eq!(
            bindings.record_dispatch_refused(
                &role,
                DeviceRevisionLookup::Retained(crate::DeviceRevision::default()),
                FrameClockReading::Measurable(now),
                wait_bound,
                super::WaitingCondition::ApplicationBindingRepairRequired,
            ),
            super::WaitTransition::ReasonChanged
        );

        let changes = bindings.take_status_changes();
        assert!(matches!(
            changes.as_slice(),
            [super::RoleStatusChange {
                to: crate::RoleStatusView::Waiting(
                    crate::WaitingStatusView::ApplicationBindingRepairRequired {
                        timing: crate::WaitTiming::Bounded { .. },
                        retry:  crate::RetryScheduleView::Blocked(
                            crate::RetryGateView::DeviceRevisionChanged { .. }
                        ),
                    }
                ),
                ..
            }]
        ));
        assert_eq!(
            bindings.record_dispatch_refused(
                &role,
                DeviceRevisionLookup::Retained(crate::DeviceRevision::default()),
                FrameClockReading::Measurable(now),
                wait_bound,
                super::WaitingCondition::ApplicationBindingRepairRequired,
            ),
            super::WaitTransition::Unchanged
        );
        assert!(!bindings.has_status_changes());
        Ok(())
    }

    fn only_status_change(
        changes: &[super::RoleStatusChange],
    ) -> Result<&super::RoleStatusChange, Box<dyn Error>> {
        let [change] = changes else {
            return Err("the mutation did not queue exactly one status change".into());
        };
        Ok(change)
    }

    fn application_binding_repair_timing(
        status: &crate::RoleStatusView,
    ) -> Result<&crate::WaitTiming, Box<dyn Error>> {
        let RoleStatusView::Waiting(WaitingStatusView::ApplicationBindingRepairRequired {
            timing,
            ..
        }) = status
        else {
            return Err("the role is not waiting for application binding repair".into());
        };
        Ok(timing)
    }

    #[test]
    fn a_renewed_refusal_restarts_timing_and_crosses_its_later_bound() -> Result<(), Box<dyn Error>>
    {
        let (mut bindings, role, now) = bindings_with_status_clock("renewed-refusal")?;
        let initial_duration = Duration::from_secs(5);
        let renewed_duration = Duration::from_secs(10);
        let initial_deadline = now + initial_duration;
        let renewed_deadline = now + renewed_duration;
        let condition = super::WaitingCondition::ApplicationBindingRepairRequired;

        assert_eq!(
            bindings.record_dispatch_refused(
                &role,
                DeviceRevisionLookup::Retained(crate::DeviceRevision::default()),
                FrameClockReading::Measurable(now),
                initial_deadline,
                condition.clone(),
            ),
            super::WaitTransition::ReasonChanged
        );
        bindings.take_status_changes();
        assert_eq!(
            bindings.cross_wait_bounds(initial_deadline),
            std::slice::from_ref(&role)
        );
        let initial_crossing = bindings.take_status_changes();
        let initial_crossing = only_status_change(&initial_crossing)?;
        let WaitTiming::Overdue {
            deadline,
            crossed_at,
            ..
        } = application_binding_repair_timing(&initial_crossing.to)?
        else {
            return Err("the initial bound did not publish as overdue".into());
        };
        assert_eq!(deadline.elapsed(), initial_duration);
        assert_eq!(crossed_at.elapsed(), initial_duration);

        assert_eq!(
            bindings.record_dispatch_refused(
                &role,
                DeviceRevisionLookup::Retained(crate::DeviceRevision::default()),
                FrameClockReading::Measurable(initial_deadline),
                renewed_deadline,
                condition,
            ),
            super::WaitTransition::Unchanged
        );
        let renewal = bindings.take_status_changes();
        let renewal = only_status_change(&renewal)?;
        let WaitTiming::Overdue {
            deadline: first_deadline,
            crossed_at: first_crossed_at,
            ..
        } = application_binding_repair_timing(&renewal.from)?
        else {
            return Err("renewal did not replace an overdue bound".into());
        };
        let WaitTiming::Bounded {
            deadline: second_deadline,
            ..
        } = application_binding_repair_timing(&renewal.to)?
        else {
            return Err("renewal did not publish the later bounded deadline".into());
        };
        assert_eq!(first_deadline.elapsed(), initial_duration);
        assert_eq!(first_crossed_at.elapsed(), initial_duration);
        assert_eq!(second_deadline.elapsed(), renewed_duration);

        assert_eq!(bindings.cross_wait_bounds(renewed_deadline), [role]);
        let renewed_crossing = bindings.take_status_changes();
        let renewed_crossing = only_status_change(&renewed_crossing)?;
        let WaitTiming::Overdue {
            deadline,
            crossed_at,
            ..
        } = application_binding_repair_timing(&renewed_crossing.to)?
        else {
            return Err("the renewed bound did not publish as overdue".into());
        };
        assert_eq!(deadline.elapsed(), renewed_duration);
        assert_eq!(crossed_at.elapsed(), renewed_duration);
        Ok(())
    }

    #[test]
    fn recording_waiting_work_preserves_the_active_wait() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;
        let started_at = bevy::platform::time::Instant::now();
        let deadline = started_at + Duration::from_secs(5);
        bindings.set_wait(
            &role,
            started_at,
            super::WaitingCondition::KernelRetry(super::RetryGate::AwaitingInstant(deadline)),
        );

        bindings.set_waiting_work(&role, WaitingWork::RestorationOwed);

        let super::RoleState::Waiting(waiting) = &bindings
            .by_role
            .get(&role)
            .ok_or("registered role is absent")?
            .state
        else {
            return Err("recording work did not retain Waiting".into());
        };
        assert_eq!(waiting.wait_start, super::WaitStart::At(started_at));
        assert!(matches!(
            waiting.condition,
            super::WaitingCondition::KernelRetry(super::RetryGate::AwaitingInstant(retained))
                if retained == deadline
        ));
        assert_eq!(waiting.waiting_work, WaitingWork::RestorationOwed);

        Ok(())
    }

    #[test]
    fn reporter_wait_constructors_derive_their_bounds() -> Result<(), Box<dyn Error>> {
        let cadence = DiscoveryCadence::Periodic {
            interval: Duration::from_secs(3),
        };
        let coverage = ReporterCoverage::MatchingEvidenceOnly;
        let reporter = RegisteredReporter {
            reporter:                 ReporterId(7),
            activation:               ReporterActivation::Enabled,
            cadence:                  &cadence,
            first_complete_set_bound: Duration::from_secs(11),
            coverage:                 &coverage,
            contribution:             ReporterContribution::AwaitingFirstCompleteSet,
        };
        let key = display_endpoint("studio-display")?.device;
        let now = bevy::platform::time::Instant::now();

        let super::ReporterWait::AwaitingFirstReport(awaiting) =
            super::ReporterWait::awaiting_first_report(key.clone(), now, &reporter, &[])
        else {
            return Err("the first-report constructor returned another wait".into());
        };
        assert_eq!(
            awaiting.bound,
            super::WaitBoundInput::Until(now + Duration::from_secs(11))
        );

        let limits = RiggingLimits::default();
        let super::ReporterWait::Unconfirmed(unconfirmed) = super::ReporterWait::unconfirmed(
            key,
            now,
            &reporter,
            &[],
            &limits,
            UnconfirmedBasis::NoFreshEvidence,
        ) else {
            return Err("the unconfirmed constructor returned another wait".into());
        };
        assert_eq!(
            unconfirmed.wait.bound,
            super::WaitBoundInput::Until(now + Duration::from_secs(3) + limits.report_grace)
        );

        Ok(())
    }

    #[test]
    fn apply_failure_run_moves_through_applying_waiting_and_stopped() -> Result<(), Box<dyn Error>>
    {
        let role = RoleKey::new("primary-window")?;
        let endpoint = display_endpoint("studio-display")?;
        let now = bevy::platform::time::Instant::now();
        let reading = FrameClockReading::Measurable(now);
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), endpoint.clone()))?;

        for attempt in 1..=3 {
            let authorized = authorized_attempt(&bindings, &role, endpoint.clone(), attempt, now);
            bindings.start_apply(
                &role,
                authorized,
                super::ApplyConfigurationSource::Requested,
            );
            assert!(matches!(
                bindings
                    .by_role
                    .get(&role)
                    .ok_or("registered role is absent")?
                    .state,
                super::RoleState::Applying(_)
            ));
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                DriverOutcomeStatus::Failed(transport_failure()),
                device_revision,
                reading,
            );
            if attempt < 3 {
                assert!(matches!(
                    bindings
                        .by_role
                        .get(&role)
                        .ok_or("registered role is absent")?
                        .state,
                    super::RoleState::Waiting(_)
                ));
            }
        }
        assert!(matches!(
            bindings
                .by_role
                .get(&role)
                .ok_or("registered role is absent")?
                .state,
            super::RoleState::Stopped(super::StoppedState::RepeatedFailures { .. })
        ));

        Ok(())
    }

    #[test]
    fn unsupported_stop_keeps_the_attempt_failure_run_clear() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let endpoint = display_endpoint("studio-display")?;
        let now = bevy::platform::time::Instant::now();
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), endpoint.clone()))?;
        let authorized = authorized_attempt(&bindings, &role, endpoint, 1, now);
        bindings.start_apply(
            &role,
            authorized,
            super::ApplyConfigurationSource::Requested,
        );

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            DriverOutcomeStatus::Failed(DeviceAccessError::Unsupported {
                detail: "the test platform has no driver".to_owned(),
            }),
            DeviceRevisionLookup::Retained(crate::DeviceRevision::default()),
            FrameClockReading::Measurable(now),
        );

        assert_eq!(
            bindings.role_apply_failure_run(&role),
            super::RoleApplyFailureRun::Clear
        );
        assert!(matches!(
            bindings
                .by_role
                .get(&role)
                .ok_or("registered role is absent")?
                .state,
            super::RoleState::Stopped(super::StoppedState::Unsupported { .. })
        ));

        Ok(())
    }

    #[test]
    fn entering_a_wait_from_a_live_state_is_not_reported_as_recovery() -> Result<(), Box<dyn Error>>
    {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;
        let now = bevy::platform::time::Instant::now();

        let registered_role = bindings
            .by_role
            .get_mut(&role)
            .ok_or("registered role is absent")?;
        let policy = registered_role.binding.policy();
        registered_role.set_state(super::RoleState::Established(
            super::EstablishedSession::new(
                super::ApplyConfigurationSource::Requested,
                now,
                super::SessionRef::new(0),
                super::AppliedKind::AsDispatched,
                super::EstablishingAttemptLookup::NotEstablished,
                policy,
                super::EstablishedSessionDatumArrivals::NoLeaseIssued,
            ),
        ));
        assert_eq!(
            bindings.set_wait(&role, now, super::WaitingCondition::ApplicationReapply),
            super::WaitTransition::Entered,
            "a role losing its established session has regressed into waiting, not recovered"
        );

        let registered_role = bindings
            .by_role
            .get_mut(&role)
            .ok_or("registered role is absent")?;
        registered_role.set_state(super::RoleState::Stopped(
            super::StoppedState::Unsupported {
                failure: DriverStopReason::Unsupported {
                    detail: String::from("the test driver does not support this operation"),
                },
            },
        ));
        assert_eq!(
            bindings.set_wait(&role, now, super::WaitingCondition::ApplicationReapply),
            super::WaitTransition::Recovered,
            "a role leaving a stop to wait again has recovered"
        );

        Ok(())
    }

    #[test]
    fn restart_role_clears_both_stopped_variants() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        for stopped in [
            super::StoppedState::RepeatedFailures {
                failures:      NonZeroU32::new(3).ok_or("failure count must be nonzero")?,
                reacquisition: super::ReacquisitionProgress::StillAvailable,
                last_ending:   AttemptEnding::Reported(DriverOutcomeStatus::Failed(
                    transport_failure(),
                )),
            },
            super::StoppedState::Unsupported {
                failure: DriverStopReason::Unsupported {
                    detail: String::from("the test driver does not support this operation"),
                },
            },
        ] {
            let registered_role = bindings
                .by_role
                .get_mut(&role)
                .ok_or("registered role is absent")?;
            registered_role.set_state(super::RoleState::Stopped(stopped));
            bindings.restart_role(&role)?;
            assert!(matches!(
                bindings
                    .by_role
                    .get(&role)
                    .ok_or("registered role is absent")?
                    .state,
                super::RoleState::Waiting(super::WaitingState {
                    failures: super::RoleApplyFailureRun::Clear,
                    retry_pacing: super::RetryPacing::Ready,
                    ..
                })
            ));
        }

        Ok(())
    }

    #[test]
    fn a_driver_reported_abort_gates_its_retry_without_counting_toward_escalation()
    -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, _) = bindings_with_status_clock("primary-window")?;
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            DriverOutcomeStatus::Aborted(DriverAbortReason::OperationEnded),
            device_revision,
            measurable(),
        );
        // The driver-reported abort is terminal for this frame, so dispatch later in this apply
        // chain finds no work until the retry policy opens its gate.
        assert!(
            !bindings
                .retry_pacing(&role)
                .permits_dispatch(device_revision, measurable())
        );
        assert!(
            bindings
                .retry_pacing(&role)
                .permits_dispatch(changed(device_revision), measurable())
        );
        // Three aborts in a row still leave the role dispatchable: only failures escalate.
        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                DriverOutcomeStatus::Aborted(DriverAbortReason::OperationEnded),
                device_revision,
                measurable(),
            );
        }
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Waiting(_)
        ));

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            DriverOutcomeStatus::Failed(transport_failure()),
            device_revision,
            measurable(),
        );
        let super::RetryPacing::Blocked(retry_gate) = bindings.retry_pacing(&role) else {
            return Err("a failed attempt under RetryOn::NewRevision must install a gate".into());
        };
        assert!(!retry_gate.opened(device_revision, measurable()));
        assert!(retry_gate.opened(changed(device_revision), measurable()));

        Ok(())
    }

    #[test]
    fn an_interval_retry_policy_waits_on_the_clock_rather_than_on_a_new_revision()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.retry = RetryOn::Interval(std::time::Duration::from_hours(1));
        bindings.register(configured_binding)?;

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            DriverOutcomeStatus::Failed(transport_failure()),
            device_revision,
            measurable(),
        );

        // A new revision does not shorten an interval: the two policies measure different things.
        assert!(
            !bindings
                .retry_pacing(&role)
                .permits_dispatch(changed(device_revision), measurable())
        );

        Ok(())
    }

    #[test]
    fn three_consecutive_failures_stop_dispatch_until_a_restart_or_a_success()
    -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, _) = bindings_with_status_clock("primary-window")?;
        let mut device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());

        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                DriverOutcomeStatus::Failed(transport_failure()),
                device_revision,
                measurable(),
            );
            device_revision = changed(device_revision);
            assert!(matches!(
                bindings.projected_status(&role)?,
                crate::RoleStatusView::Waiting(_)
            ));
        }
        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            DriverOutcomeStatus::Failed(transport_failure()),
            device_revision,
            measurable(),
        );

        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Stopped(_)
        ));
        // A stopped role selects no waiting view, so no fourth attempt can be dispatched.
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Stopped(_)
        ));

        bindings.restart_role(&role)?;

        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Waiting(_)
        ));
        assert_eq!(bindings.retry_pacing(&role), super::RetryPacing::Ready);

        Ok(())
    }

    #[test]
    fn a_stopped_role_waits_for_its_device_to_leave_and_return_before_another_attempt()
    -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, _) = bindings_with_status_clock("primary-window")?;
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());
        for _ in 0..3 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                DriverOutcomeStatus::Failed(transport_failure()),
                device_revision,
                measurable(),
            );
        }
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Stopped(_)
        ));

        // A device that never leaves is never retried, however many frames read it as available.
        for _ in 0..3 {
            bindings.observe_stopped_role_endpoint(&role, super::EndpointAvailability::Available);
        }
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Stopped(_)
        ));

        bindings.observe_stopped_role_endpoint(&role, super::EndpointAvailability::Gone);
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Stopped(_)
        ));

        bindings.observe_stopped_role_endpoint(&role, super::EndpointAvailability::Available);

        // Reacquired: one more attempt is dispatched, and the run of failures is still standing, so
        // a further failure stops the role again without a second dispatch.
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Waiting(_)
        ));
        assert_eq!(bindings.retry_pacing(&role), super::RetryPacing::Ready);

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            DriverOutcomeStatus::Succeeded(crate::AppliedKind::AsDispatched),
            device_revision,
            measurable(),
        );

        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                DriverOutcomeStatus::Failed(transport_failure()),
                device_revision,
                measurable(),
            );
        }
        // The success cleared the run, so two later failures are two and not five.
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Waiting(_)
        ));

        Ok(())
    }

    #[test]
    fn a_successful_attempt_clears_the_failures_counted_before_it() -> Result<(), Box<dyn Error>> {
        let (mut bindings, role, _) = bindings_with_status_clock("primary-window")?;
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());

        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                DriverOutcomeStatus::Failed(transport_failure()),
                device_revision,
                measurable(),
            );
        }
        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            DriverOutcomeStatus::Succeeded(crate::AppliedKind::AsDispatched),
            device_revision,
            measurable(),
        );
        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                DriverOutcomeStatus::Failed(transport_failure()),
                device_revision,
                measurable(),
            );
        }

        // Two failures after the recovery is two, not five: the count is consecutive.
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Waiting(_)
        ));

        Ok(())
    }

    #[test]
    fn a_restart_is_refused_for_a_role_that_was_never_stopped() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        assert!(matches!(
            bindings.restart_role(&role),
            Err(BindingError::RoleNotStopped { .. })
        ));

        Ok(())
    }

    #[test]
    fn one_authorized_dispatch_accepts_requested_and_last_known_good_configuration()
    -> Result<(), Box<dyn Error>> {
        let mut configured_binding = binding(
            RoleKey::new("primary-window")?,
            display_endpoint("studio-display")?,
        );
        let source = super::ApplyConfigurationSource::Requested;

        assert!(source.dispatch_configuration(&configured_binding).is_ok());

        configured_binding.last_known_good = LastKnownGoodConfiguration::MatchesRequested;
        assert!(source.dispatch_configuration(&configured_binding).is_ok());

        Ok(())
    }
}
