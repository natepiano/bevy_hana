//! Ordered endpoint-attempt and established-session lifecycle.

use std::collections::VecDeque;
use std::time::Duration;

use bevy::ecs::change_detection::DetectChangesMut;
use bevy::log::warn;
use bevy::platform::time::Instant;
use bevy::prelude::Entity;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::time::Real;
use bevy::time::Time;

use crate::AppliedKind;
use crate::AttemptEnding;
use crate::AttemptInvalidation;
use crate::AttemptRef;
use crate::Attempts;
use crate::BindingError;
use crate::BindingGeneration;
use crate::Bindings;
use crate::Claim;
use crate::DeviceAccessError;
use crate::DeviceRef;
use crate::DeviceResolution;
use crate::DeviceStateLookup;
use crate::Devices;
use crate::DriverContractError;
use crate::DriverContractFailureReport;
use crate::DriverOutcomeStatus;
use crate::EndedRegistrationLifetime;
use crate::HardwareInventory;
use crate::KeyAvailability;
use crate::LastKnownGoodConfiguration;
use crate::LiveRoleChange;
use crate::LiveRoleChanged;
use crate::OnAbort;
use crate::Presence;
use crate::RegistrationAttemptEnded;
use crate::ReporterActivation;
use crate::RiggingLimits;
use crate::RoleKey;
use crate::RoleStatusView;
use crate::SessionReleaseCause;
use crate::TargetResolution;
use crate::TargetResolutionContext;
use crate::TargetWait;
use crate::UnconfirmedBasis;
use crate::WaitingWork;
use crate::binding;
use crate::binding::ActiveAttemptRecord;
use crate::binding::ApplyConfigurationSource;
use crate::binding::AuthorizedApplyAttempt;
use crate::binding::BindingTransition;
use crate::binding::BindingTransitionBatch;
use crate::binding::ContinuousFlowJudgment;
use crate::binding::DriverCleanup;
use crate::binding::EndpointAvailability;
use crate::binding::NonEmptyReporterIds;
use crate::binding::ReporterWait;
use crate::binding::WaitingCondition;
use crate::contract::DriverReports;
use crate::contract::ErasedApplied;
use crate::contract::ErasedDriverCompletion;
use crate::contract::ErasedSessionReport;
use crate::contract::QueuedCompletion;
use crate::contract::QueuedSessionReport;
use crate::contract::ResolvedDeviceContext;
use crate::devices::ApplyAuthorizationError;
use crate::devices::DeviceEntityLookup;
use crate::devices::DeviceRevision;
use crate::devices::DeviceRevisionLookup;
use crate::devices::PriorKeyAvailability;
use crate::reconcile::FrameClockReading;
use crate::registration::ApplyPermit;
use crate::registration::Drivers;
use crate::registration::RegisteredReporter;
use crate::registration::ReporterContribution;
use crate::registration::Reporters;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptValidity {
    Holds,
    Invalidated(AttemptInvalidation),
}

enum ApplyDispatchOutcome {
    Started,
    Deferred(WaitingCondition),
    Rejected(ApplyDispatchRejection),
}

enum ApplyDispatchRejection {
    Binding(crate::BindingError),
    DriverContract(DriverContractError),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReporterWaitKind {
    AwaitingFirstReport,
    Unconfirmed,
    Unreachable,
    Absent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptEndingRegistrationLifetime {
    Live(Entity),
    Displaced,
    Retired,
}

enum AttemptEndingPublicationDestination {
    Live(Entity),
    AwaitingRoleEntityRecovery,
    RegistrationRetirementRequired,
    Ended(EndedRegistrationLifetime),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptEndingPublicationReadiness {
    ReadyAfterStatusPublication,
    ReadyAfterAttemptLifecycle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AttemptEndingRoleEntityRecovery {
    #[default]
    NotAwaiting,
    AwaitingNextPass,
    PassCompleted,
}

/// One terminal attempt result retained until its ordering predecessor has been published.
struct PendingAttemptEndingPublication {
    role:                 RoleKey,
    endpoint:             crate::DeviceEndpoint,
    generation:           BindingGeneration,
    attempt:              AttemptRef,
    ending:               AttemptEnding,
    lifetime:             AttemptEndingRegistrationLifetime,
    readiness:            AttemptEndingPublicationReadiness,
    role_entity_recovery: AttemptEndingRoleEntityRecovery,
}

impl PendingAttemptEndingPublication {
    fn live(retained: &ActiveAttemptRecord, ending: AttemptEnding) -> Self {
        Self {
            role: retained.role.clone(),
            endpoint: retained.attempt.endpoint().clone(),
            generation: retained.attempt.generation(),
            attempt: retained.attempt.reference(),
            ending,
            lifetime: AttemptEndingRegistrationLifetime::Live(retained.entity),
            readiness: AttemptEndingPublicationReadiness::ReadyAfterStatusPublication,
            role_entity_recovery: AttemptEndingRoleEntityRecovery::NotAwaiting,
        }
    }

    const fn ended(
        role: RoleKey,
        endpoint: crate::DeviceEndpoint,
        generation: BindingGeneration,
        attempt: AttemptRef,
        ending: AttemptEnding,
        lifetime: AttemptEndingRegistrationLifetime,
    ) -> Self {
        Self {
            role,
            endpoint,
            generation,
            attempt,
            ending,
            lifetime,
            readiness: AttemptEndingPublicationReadiness::ReadyAfterAttemptLifecycle,
            role_entity_recovery: AttemptEndingRoleEntityRecovery::NotAwaiting,
        }
    }

    fn finish_attempt_lifecycle(&mut self) {
        if self.readiness == AttemptEndingPublicationReadiness::ReadyAfterAttemptLifecycle {
            self.readiness = AttemptEndingPublicationReadiness::ReadyAfterStatusPublication;
        }
    }

    fn finish_role_entity_recovery_pass(&mut self) {
        if self.role_entity_recovery == AttemptEndingRoleEntityRecovery::AwaitingNextPass {
            self.role_entity_recovery = AttemptEndingRoleEntityRecovery::PassCompleted;
        }
    }

    fn await_next_role_entity_recovery_pass(&mut self) {
        if self.role_entity_recovery == AttemptEndingRoleEntityRecovery::NotAwaiting {
            self.role_entity_recovery = AttemptEndingRoleEntityRecovery::AwaitingNextPass;
        }
    }
}

#[derive(Default, Resource)]
pub(crate) struct PendingAttemptEndingPublications(Vec<PendingAttemptEndingPublication>);

struct AttemptLifecycleResources<'a> {
    bindings:           &'a mut Bindings,
    drivers:            &'a mut Drivers,
    devices:            &'a Devices,
    hardware_inventory: &'a HardwareInventory,
    reports:            &'a DriverReports,
    now:                FrameClockReading,
    apply_overrun:      Duration,
    endings:            Vec<PendingAttemptEndingPublication>,
}

struct BindingTransitionDriverCleanup {
    role:          RoleKey,
    role_entity:   DriverCleanupRoleEntity,
    endpoint:      crate::DeviceEndpoint,
    cleanup:       DriverCleanup,
    invalidation:  AttemptInvalidation,
    release_cause: SessionReleaseCause,
    lifetime:      AttemptEndingRegistrationLifetime,
}

/// Whether the role entity still exists when the kernel calls a driver's cleanup.
///
/// **Why the entity is not simply an [`Entity`].** A role entity despawned outside the kernel used
/// to strand its driver: an established session was never released, so the hardware stayed open
/// with no observer and no way back. Passing a bare `Entity` made that failure invisible, because
/// the driver could not tell a live entity from a dangling id and the kernel could only warn. This
/// type puts the fact in the signature: a driver handles [`Self::Removed`] exactly as
/// [`Self::Live`] for its own records and hardware, and skips only the work that needs the entity.
///
/// [`Self::Live`] is checked at construction: it is built in exactly one place,
/// `Self::checked`, which asks the world whether the id is still spawned. A driver may therefore
/// take the promise literally and read or write the entity it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverCleanupRoleEntity {
    /// The role entity is still spawned and may be read or written for this callback only.
    Live(Entity),
    /// The role entity is gone at the moment cleanup runs; only entity-free work can be done.
    ///
    /// The full precondition, because a plain despawn does not produce it: role-entity recovery
    /// (`reconcile_role_entities`, which runs immediately before
    /// `cleanup_binding_transition_driver_state`) re-spawns the entity of a binding that is still
    /// registered, so a role entity despawned on its own comes back and reaches the driver as
    /// `Live`. `Removed` arrives when the entity is gone at the moment the binding is replaced or
    /// retired — nothing is left to re-spawn it — or when a loss, stall, or invalidation finds
    /// that the id [`Bindings`] still records no longer exists (see `Self::checked`).
    ///
    /// To reproduce it in a test: retire the role, despawn its entity, run one update.
    Removed,
}

impl DriverCleanupRoleEntity {
    /// Classify a role entity id against the world that is about to hand it to a driver.
    ///
    /// [`Bindings::role_entity`] answers from a by-role map holding whatever id was registered, so
    /// it returns `Ok` for an entity that has since been despawned: a role entity that loses its
    /// `RoleKey` before it is despawned leaves no lost-entity record for recovery to act on, and
    /// the dead id stays in [`Bindings`] for the life of the binding. Passing that id on as
    /// [`Self::Live`] would be a lie the contract doc invites the driver to act on, and reading a
    /// despawned entity panics. Every `Live` is built here, so a dead entity reaches the driver as
    /// [`Self::Removed`] instead.
    pub(crate) fn checked(world: &World, role_entity: Entity) -> Self {
        if world.get_entity(role_entity).is_ok() {
            Self::Live(role_entity)
        } else {
            Self::Removed
        }
    }
}

fn binding_transition_driver_cleanup(world: &World) -> Vec<BindingTransitionDriverCleanup> {
    world
        .resource::<BindingTransitionBatch>()
        .transitions()
        .iter()
        .filter_map(|transition| {
            let (role, role_entity, endpoint, cleanup, invalidation, release_cause, lifetime) =
                match transition {
                    BindingTransition::Registered { .. } => return None,
                    BindingTransition::Replaced {
                        role,
                        displaced_entity,
                        endpoint,
                        cleanup,
                        ..
                    } => {
                        let role_entity = world
                            .resource::<Bindings>()
                            .role_entity(role)
                            .unwrap_or(*displaced_entity);
                        (
                            role.clone(),
                            role_entity,
                            endpoint.clone(),
                            *cleanup,
                            AttemptInvalidation::BindingReplaced,
                            SessionReleaseCause::BindingReplaced,
                            AttemptEndingRegistrationLifetime::Displaced,
                        )
                    },
                    BindingTransition::Retired {
                        role,
                        endpoint,
                        entity,
                        cleanup,
                        ..
                    } => (
                        role.clone(),
                        *entity,
                        endpoint.clone(),
                        *cleanup,
                        AttemptInvalidation::RoleRetired,
                        SessionReleaseCause::RoleRetired,
                        AttemptEndingRegistrationLifetime::Retired,
                    ),
                };
            if cleanup == DriverCleanup::None {
                return None;
            }
            let role_entity = DriverCleanupRoleEntity::checked(world, role_entity);
            Some(BindingTransitionDriverCleanup {
                role,
                role_entity,
                endpoint,
                cleanup,
                invalidation,
                release_cause,
                lifetime,
            })
        })
        .collect()
}

/// Cancel or release client work before a replaced or retired role entity can be removed.
pub(crate) fn cleanup_binding_transition_driver_state(world: &mut World) {
    let cleanup = binding_transition_driver_cleanup(world);
    if cleanup.is_empty() {
        return;
    }
    world.resource_scope::<Drivers, _>(|world, mut drivers| {
        for cleanup in cleanup {
            let BindingTransitionDriverCleanup {
                role,
                role_entity,
                endpoint,
                cleanup,
                invalidation,
                release_cause,
                lifetime,
            } = cleanup;
            match cleanup {
                DriverCleanup::None => {},
                DriverCleanup::Applying {
                    driver,
                    attempt,
                    generation,
                } => {
                    // The driver is called whether or not the entity survived: an attempt whose
                    // role entity vanished still has hardware started, and only the driver can
                    // undo it.
                    if let Err(error) = drivers.cancel_apply(
                        world,
                        driver,
                        &role,
                        role_entity,
                        attempt,
                        invalidation,
                    ) {
                        warn!("role `{role}`: driver cleanup failed: {error}");
                    }
                    world
                        .resource_mut::<PendingAttemptEndingPublications>()
                        .0
                        .push(PendingAttemptEndingPublication::ended(
                            role,
                            endpoint,
                            generation,
                            attempt,
                            AttemptEnding::Invalidated(invalidation),
                            lifetime,
                        ));
                },
                // A despawned role entity is the case this most has to reach: without the call
                // the session stays open in the driver forever, with nothing left to observe it.
                DriverCleanup::Established { driver, session } => {
                    if let Err(error) = drivers.release_session(
                        world,
                        driver,
                        &role,
                        role_entity,
                        session,
                        release_cause,
                    ) {
                        warn!("role `{role}`: driver cleanup failed: {error}");
                    }
                },
            }
        }
    });
}

/// Advance all driver-owned authorities in the contract's fixed lifecycle order.
pub(crate) fn advance_attempt_lifecycle(world: &mut World) {
    for publication in &mut world.resource_mut::<PendingAttemptEndingPublications>().0 {
        publication.finish_attempt_lifecycle();
        publication.finish_role_entity_recovery_pass();
    }
    rearm_reacquired_roles(world);

    let reports = world.resource::<DriverReports>().clone();
    let completions = reports.drain_completions();
    let session_reports = reports.drain_session_reports();
    let now = FrameClockReading::from(world.resource::<Time<Real>>());
    let apply_overrun = world.resource::<RiggingLimits>().apply_overrun;
    let has_startable_roles = !startable_roles(world).is_empty();
    let has_active_attempts = !world.resource::<Bindings>().active_attempts().is_empty();
    if completions.is_empty()
        && session_reports.is_empty()
        && !has_startable_roles
        && !has_active_attempts
    {
        return;
    }
    let endings = world.resource_scope::<Bindings, _>(|world, mut bindings| {
        world.resource_scope::<Drivers, _>(|world, mut drivers| {
            world.resource_scope::<Devices, _>(|world, devices| {
                world.resource_scope::<HardwareInventory, _>(|world, hardware_inventory| {
                    AttemptLifecycleResources {
                        bindings: &mut bindings,
                        drivers: &mut drivers,
                        devices: &devices,
                        hardware_inventory: &hardware_inventory,
                        reports: &reports,
                        now,
                        apply_overrun,
                        endings: Vec::new(),
                    }
                    .advance(world, completions, session_reports)
                })
            })
        })
    });

    world
        .resource_mut::<PendingAttemptEndingPublications>()
        .0
        .extend(endings);
}

/// Credit continuous-flow testimony, publish newly crossed stalls, and release stalls retained
/// from the prior update.
///
/// A monitored session testifies on nearly every frame once its device is producing data, and
/// crediting that testimony rewrites nothing a consumer of the binding register can observe. The
/// judgment reports whether it published anything, and only then is the register marked changed:
/// borrowing it mutably on every datum frame would re-project every managed window's placement and
/// break the settled-frame write counts at data rate.
pub(crate) fn judge_continuous_flow(world: &mut World) {
    let now = FrameClockReading::from(world.resource::<Time<Real>>());
    if !world.resource::<Bindings>().continuous_flow_update_due(now) {
        return;
    }
    let mut bindings = world.resource_mut::<Bindings>();
    let ContinuousFlowJudgment::Published { release_required } = bindings
        .bypass_change_detection()
        .judge_continuous_flow(now)
    else {
        return;
    };
    bindings.set_changed();
    if release_required.is_empty() {
        return;
    }

    world.resource_scope::<Bindings, _>(|world, mut bindings| {
        world.resource_scope::<Drivers, _>(|world, mut drivers| {
            world.resource_scope::<Devices, _>(|world, devices| {
                for retained in release_required {
                    let Some(established) =
                        bindings.established_session(&retained.role, retained.session)
                    else {
                        continue;
                    };
                    // A stall crossing fires its loss exactly once because the session always
                    // leaves `Established` here. Nothing below may skip that: the flow judge is
                    // selected by retained state rather than a consumed queue entry, so a role
                    // parked in `Established(Stalled)` is re-selected, re-released, and re-warned
                    // on every following update for the life of the binding.
                    if let Ok(role_entity) = bindings.role_entity(&established.role) {
                        let role_entity = DriverCleanupRoleEntity::checked(world, role_entity);
                        let _ = drivers.release_session(
                            world,
                            established.driver,
                            &established.role,
                            role_entity,
                            established.session,
                            SessionReleaseCause::FlowStalled,
                        );
                    } else {
                        warn!(
                            "role `{}`: stalled session has no live role entity",
                            established.role
                        );
                    }
                    let device_revision =
                        role_device_revision(&devices, &bindings, &established.role);
                    let _ = bindings.apply_session_loss(&established.role, device_revision, now);
                }
            });
        });
    });
}

impl AttemptLifecycleResources<'_> {
    fn advance(
        mut self,
        world: &mut World,
        completions: VecDeque<QueuedCompletion>,
        session_reports: VecDeque<QueuedSessionReport>,
    ) -> Vec<PendingAttemptEndingPublication> {
        // (1) Synchronous start errors are recorded while starting successors below.
        // (2) Revalidate every non-time guard before accepting any completion.
        self.revalidate_attempts(world);
        // (3) Accept on-time completions for attempts whose guards still hold.
        let refused = self.accept_completions(world, completions);
        // (4) Only a strict crossing of the hard end exhausts an unfinished attempt.
        self.expire_overdue_attempts(world);
        // (5) Late, duplicate, unknown, and superseded results are refused here.
        record_refused_completions(refused);
        process_session_reports(
            world,
            self.bindings,
            self.drivers,
            self.devices,
            session_reports,
            self.now,
        );
        // (6) Resolution precedes issuance for every successor.
        self.start_successors(world);
        self.endings
    }

    fn revalidate_attempts(&mut self, world: &mut World) {
        for retained in self.bindings.active_attempts() {
            let AttemptValidity::Invalidated(cause) = attempt_validity(
                &retained,
                self.bindings,
                self.devices,
                self.hardware_inventory,
            ) else {
                continue;
            };
            cancel_attempt(world, self.bindings, self.drivers, &retained, cause);
            finish_invalidated(
                self.bindings,
                self.devices,
                &retained,
                cause,
                self.now,
                &mut self.endings,
            );
        }
    }

    fn accept_completions(
        &mut self,
        world: &mut World,
        completions: VecDeque<QueuedCompletion>,
    ) -> Vec<QueuedCompletion> {
        let mut refused = Vec::new();
        for queued in completions {
            let Some(retained) = self.bindings.active_attempt(&queued.role, queued.attempt) else {
                refused.push(queued);
                continue;
            };
            if queued.completed_at > retained.attempt.deadline() + self.apply_overrun {
                refused.push(queued);
                continue;
            }
            accept_completion(
                world,
                self.bindings,
                self.drivers,
                self.devices,
                self.reports,
                retained,
                queued,
                self.now,
                &mut self.endings,
            );
        }
        refused
    }

    fn expire_overdue_attempts(&mut self, world: &mut World) {
        let FrameClockReading::Measurable(measured) = self.now else {
            return;
        };
        let overdue = self
            .bindings
            .active_attempts()
            .into_iter()
            .filter(|retained| measured > retained.attempt.deadline() + self.apply_overrun)
            .collect::<Vec<_>>();
        for retained in overdue {
            cancel_attempt(
                world,
                self.bindings,
                self.drivers,
                &retained,
                AttemptInvalidation::OverrunExhausted,
            );
            finish_invalidated(
                self.bindings,
                self.devices,
                &retained,
                AttemptInvalidation::OverrunExhausted,
                self.now,
                &mut self.endings,
            );
        }
    }

    fn start_successors(&mut self, world: &mut World) {
        let startable = startable_roles_from(world, self.bindings, self.devices, self.now);
        for role in startable {
            if let Some(ending) = start_one_apply(
                world,
                self.bindings,
                self.drivers,
                self.devices,
                self.hardware_inventory,
                self.reports,
                &role,
                self.now,
            ) {
                self.endings.push(ending);
            }
        }
    }
}

fn record_refused_completions(refused: Vec<QueuedCompletion>) {
    for queued in refused {
        warn!(
            "role `{}`: refused completion for inactive attempt {}",
            queued.role,
            queued.attempt.get()
        );
    }
}

fn attempt_validity(
    retained: &ActiveAttemptRecord,
    bindings: &Bindings,
    devices: &Devices,
    hardware_inventory: &HardwareInventory,
) -> AttemptValidity {
    let attempt = &retained.attempt;
    match bindings.generation(&retained.role) {
        None => return AttemptValidity::Invalidated(AttemptInvalidation::RoleRetired),
        Some(generation) if generation != attempt.generation() => {
            return AttemptValidity::Invalidated(AttemptInvalidation::BindingReplaced);
        },
        Some(_) => {},
    }
    if devices.resolve(&attempt.endpoint().device)
        != DeviceResolution::Resolved(attempt.device_id())
    {
        return AttemptValidity::Invalidated(AttemptInvalidation::DeviceChanged);
    }
    let DeviceStateLookup::Retained(device_state) = devices.state(attempt.device_id()) else {
        return AttemptValidity::Invalidated(AttemptInvalidation::DeviceNotPresent);
    };
    if !matches!(
        devices.key_availability(&device_state.key),
        PriorKeyAvailability::Published(crate::KeyAvailability::Present(_))
    ) {
        return AttemptValidity::Invalidated(AttemptInvalidation::DeviceNotPresent);
    }
    match device_state.claim {
        Claim::Held | Claim::Free | Claim::NotApplicable => {},
        Claim::Contended { .. } | Claim::Blocked { .. } => {
            return AttemptValidity::Invalidated(AttemptInvalidation::ClaimLost);
        },
    }
    if !device_state.verdict.identified() {
        return AttemptValidity::Invalidated(AttemptInvalidation::IdentityNoLongerConfirmed);
    }
    if hardware_inventory
        .ensure_operational(&attempt.endpoint().device)
        .is_err()
    {
        return AttemptValidity::Invalidated(AttemptInvalidation::InventoryWithdrewTheDevice);
    }
    if devices.revision(attempt.device_id())
        != DeviceRevisionLookup::Retained(attempt.device_revision())
    {
        return AttemptValidity::Invalidated(AttemptInvalidation::RevisionAdvanced);
    }
    AttemptValidity::Holds
}

fn cancel_attempt(
    world: &mut World,
    bindings: &Bindings,
    drivers: &mut Drivers,
    retained: &ActiveAttemptRecord,
    cause: AttemptInvalidation,
) {
    let Ok(role_entity) = bindings.role_entity(&retained.role) else {
        warn!(
            "role `{}`: canceled attempt has no live role entity",
            retained.role
        );
        return;
    };
    let role_entity = DriverCleanupRoleEntity::checked(world, role_entity);
    let _ = drivers.cancel_apply(
        world,
        retained.driver,
        &retained.role,
        role_entity,
        retained.attempt.reference(),
        cause,
    );
}

fn finish_invalidated(
    bindings: &mut Bindings,
    devices: &Devices,
    retained: &ActiveAttemptRecord,
    cause: AttemptInvalidation,
    now: FrameClockReading,
    endings: &mut Vec<PendingAttemptEndingPublication>,
) {
    apply_abort_policy(bindings, &retained.role, cause);
    bindings.record_invalidated_attempt_ending(
        &retained.role,
        retained.attempt.generation(),
        cause,
        role_device_revision(devices, bindings, &retained.role),
        now,
    );
    endings.push(PendingAttemptEndingPublication::live(
        retained,
        AttemptEnding::Invalidated(cause),
    ));
}

fn accept_completion(
    world: &mut World,
    bindings: &mut Bindings,
    drivers: &mut Drivers,
    devices: &Devices,
    reports: &DriverReports,
    retained: ActiveAttemptRecord,
    queued: QueuedCompletion,
    now: FrameClockReading,
    endings: &mut Vec<PendingAttemptEndingPublication>,
) {
    match queued.completion {
        ErasedDriverCompletion::Succeeded(applied) => {
            let applied_kind = match &applied {
                ErasedApplied::AsDispatched => AppliedKind::AsDispatched,
                ErasedApplied::DiffersFromDispatched(_) => AppliedKind::DiffersFromDispatched,
            };
            let session = bindings.issue_session();
            let Ok(role_entity) = bindings.role_entity(&retained.role) else {
                warn!(
                    "role `{}`: successful attempt has no live role entity",
                    retained.role
                );
                return;
            };
            match drivers.established(
                world,
                retained.driver,
                &retained.role,
                role_entity,
                retained.attempt.reference(),
                session,
                reports,
            ) {
                Ok(datum_arrivals) => {
                    if bindings.accept_success(
                        &retained.role,
                        retained.attempt.reference(),
                        queued.completed_at,
                        session,
                        datum_arrivals,
                        applied,
                    ) {
                        endings.push(PendingAttemptEndingPublication::live(
                            &retained,
                            AttemptEnding::Reported(DriverOutcomeStatus::Succeeded(applied_kind)),
                        ));
                    }
                },
                Err(error) => {
                    cancel_attempt(
                        world,
                        bindings,
                        drivers,
                        &retained,
                        AttemptInvalidation::DriverContractFailed,
                    );
                    record_contract_failure(bindings, devices, &retained, error, now, endings);
                },
            }
        },
        ErasedDriverCompletion::Failed(error) => {
            let outcome = DriverOutcomeStatus::Failed(error);
            bindings.record_attempt_ending(
                &retained.role,
                retained.attempt.generation(),
                outcome.clone(),
                role_device_revision(devices, bindings, &retained.role),
                now,
            );
            endings.push(PendingAttemptEndingPublication::live(
                &retained,
                AttemptEnding::Reported(outcome),
            ));
        },
        ErasedDriverCompletion::Aborted(reason) => {
            let outcome = DriverOutcomeStatus::Aborted(reason);
            bindings.record_attempt_ending(
                &retained.role,
                retained.attempt.generation(),
                outcome.clone(),
                role_device_revision(devices, bindings, &retained.role),
                now,
            );
            endings.push(PendingAttemptEndingPublication::live(
                &retained,
                AttemptEnding::Reported(outcome),
            ));
        },
    }
}

fn record_contract_failure(
    bindings: &mut Bindings,
    devices: &Devices,
    retained: &ActiveAttemptRecord,
    error: DriverContractError,
    now: FrameClockReading,
    endings: &mut Vec<PendingAttemptEndingPublication>,
) {
    let report = DriverContractFailureReport::from(error);
    bindings.record_registration_application_ending(
        &retained.role,
        retained.attempt.generation(),
        AttemptEnding::ContractFailed(report.clone()),
    );
    bindings.record_dispatch_refused(
        &retained.role,
        role_device_revision(devices, bindings, &retained.role),
        now,
        retained.attempt.deadline(),
        WaitingCondition::DriverRepair {
            error: report.clone(),
        },
    );
    endings.push(PendingAttemptEndingPublication::live(
        retained,
        AttemptEnding::ContractFailed(report),
    ));
}

fn process_session_reports(
    world: &mut World,
    bindings: &mut Bindings,
    drivers: &mut Drivers,
    devices: &Devices,
    reports: impl IntoIterator<Item = QueuedSessionReport>,
    now: FrameClockReading,
) {
    for queued in reports {
        match queued.report {
            ErasedSessionReport::ConfigurationChanged(configuration) => {
                let _ = bindings.update_established_configuration(
                    &queued.role,
                    queued.session,
                    configuration,
                );
            },
            ErasedSessionReport::Loss(_error) => {
                let Some(established) =
                    bindings.established_sessions().into_iter().find(|session| {
                        session.role == queued.role && session.session == queued.session
                    })
                else {
                    continue;
                };
                let Ok(role_entity) = bindings.role_entity(&established.role) else {
                    warn!(
                        "role `{}`: released session has no live role entity",
                        established.role
                    );
                    continue;
                };
                let role_entity = DriverCleanupRoleEntity::checked(world, role_entity);
                let _ = drivers.release_session(
                    world,
                    established.driver,
                    &established.role,
                    role_entity,
                    established.session,
                    SessionReleaseCause::ReportedLoss,
                );
                let device_revision = role_device_revision(devices, bindings, &established.role);
                let _ = bindings.apply_session_loss(&established.role, device_revision, now);
            },
        }
    }
}

struct PreparedApply<'a> {
    role:            &'a RoleKey,
    endpoint:        crate::DeviceEndpoint,
    driver:          crate::DriverId,
    generation:      crate::BindingGeneration,
    role_entity:     Entity,
    source:          ApplyConfigurationSource,
    device_id:       crate::DeviceId,
    device_revision: DeviceRevision,
    started_at:      Instant,
    deadline:        Instant,
    resolved_device: ResolvedDeviceContext,
    permit:          ApplyPermit,
}

fn start_one_apply(
    world: &mut World,
    bindings: &mut Bindings,
    drivers: &mut Drivers,
    devices: &Devices,
    hardware_inventory: &HardwareInventory,
    reports: &DriverReports,
    role: &RoleKey,
    now: FrameClockReading,
) -> Option<PendingAttemptEndingPublication> {
    let FrameClockReading::Measurable(started_at) = now else {
        return None;
    };
    let binding = bindings.binding(role).ok()?;
    let endpoint = binding.endpoint.clone();
    let driver = binding.driver;
    let generation = bindings.generation(role)?;
    let role_entity = bindings.role_entity(role).ok()?;
    let apply_deadline = binding.apply_deadline;
    let source = match bindings.waiting_work(role) {
        WaitingWork::RestorationOwed => ApplyConfigurationSource::LastKnownGood,
        WaitingWork::Nothing => ApplyConfigurationSource::Requested,
        WaitingWork::ReapplyRequestOwed | WaitingWork::RegistrationOwed => return None,
    };

    let DeviceResolution::Resolved(device_id) = devices.resolve(&endpoint.device) else {
        bindings.set_wait(
            role,
            started_at,
            unresolved_device_wait(world, devices, &endpoint.device, started_at),
        );
        return None;
    };
    let DeviceRevisionLookup::Retained(device_revision) = devices.revision(device_id) else {
        return None;
    };
    let DeviceEntityLookup::Projected(device_entity) = devices.entity(device_id) else {
        bindings.set_wait(
            role,
            started_at,
            WaitingCondition::KernelStateRepairRequired,
        );
        return None;
    };
    let permit = devices.authorize_service(device_id);
    let permit = match permit {
        Ok(permit) => permit,
        Err(error) => {
            let wait = authorization_wait(world, devices, device_id, &error, started_at);
            bindings.set_wait(role, started_at, wait);
            return None;
        },
    };
    if let Err(error) = hardware_inventory.ensure_operational(&endpoint.device) {
        record_prestart_rejection(
            bindings,
            role,
            device_revision,
            now,
            started_at,
            ApplyDispatchRejection::Binding(error),
        );
        return None;
    }
    let deadline = started_at
        + apply_deadline
            .resolve(world.resource::<RiggingLimits>())
            .duration();
    let resolved_device = ResolvedDeviceContext::new(
        device_entity,
        DeviceRef::from_device_id(device_id),
        device_revision,
    );
    dispatch_prepared_apply(
        world,
        bindings,
        drivers,
        devices,
        reports,
        PreparedApply {
            role,
            endpoint,
            driver,
            generation,
            role_entity,
            source,
            device_id,
            device_revision,
            started_at,
            deadline,
            resolved_device,
            permit,
        },
        now,
    )
}

fn dispatch_prepared_apply(
    world: &mut World,
    bindings: &mut Bindings,
    drivers: &mut Drivers,
    devices: &Devices,
    reports: &DriverReports,
    prepared: PreparedApply<'_>,
    now: FrameClockReading,
) -> Option<PendingAttemptEndingPublication> {
    let configuration = match prepared
        .source
        .dispatch_configuration(bindings.binding(prepared.role).ok()?)
    {
        Ok(configuration) => configuration,
        Err(error) => {
            record_prestart_rejection(
                bindings,
                prepared.role,
                prepared.device_revision,
                now,
                prepared.deadline,
                ApplyDispatchRejection::Binding(error),
            );
            return None;
        },
    };
    let target_context = TargetResolutionContext::new(
        prepared.role,
        prepared.role_entity,
        &prepared.endpoint,
        prepared.resolved_device,
    );
    let dispatch =
        match drivers.resolve_target(world, prepared.driver, &target_context, configuration) {
            Ok(TargetResolution::Reached(target)) => {
                let attempt = world.resource_mut::<Attempts>().issue().ok()?;
                let authorized = AuthorizedApplyAttempt::new(
                    attempt,
                    prepared.generation,
                    prepared.endpoint.clone(),
                    prepared.device_id,
                    prepared.device_revision,
                    prepared.started_at,
                    prepared.deadline,
                );
                if let Err(rejection) =
                    release_replaced_session(world, bindings, drivers, prepared.role)
                {
                    record_prestart_rejection(
                        bindings,
                        prepared.role,
                        prepared.device_revision,
                        now,
                        prepared.deadline,
                        rejection,
                    );
                    return None;
                }
                bindings.start_apply(prepared.role, authorized, prepared.source);
                let configuration = bindings.active_configuration(prepared.role, attempt).ok()?;
                let context = TargetResolutionContext::new(
                    prepared.role,
                    prepared.role_entity,
                    &prepared.endpoint,
                    prepared.resolved_device,
                );
                match drivers.start_apply(
                    world,
                    prepared.driver,
                    context,
                    prepared.deadline,
                    prepared.permit,
                    attempt,
                    reports,
                    configuration,
                    target,
                ) {
                    Ok(()) => ApplyDispatchOutcome::Started,
                    Err(error) => {
                        let retained = bindings.active_attempt(prepared.role, attempt)?;
                        cancel_attempt(
                            world,
                            bindings,
                            drivers,
                            &retained,
                            AttemptInvalidation::DriverContractFailed,
                        );
                        reports.refuse_attempt(attempt);
                        return Some(record_started_contract_failure(
                            bindings, devices, retained, error, now,
                        ));
                    },
                }
            },
            Ok(TargetResolution::Deferred(wait)) => ApplyDispatchOutcome::Deferred(
                waiting_condition_for_target(world, &prepared.endpoint, wait, prepared.started_at),
            ),
            Err(error) => {
                ApplyDispatchOutcome::Rejected(ApplyDispatchRejection::DriverContract(error))
            },
        };
    record_dispatch_outcome(bindings, &prepared, dispatch, now)
}

fn record_dispatch_outcome(
    bindings: &mut Bindings,
    prepared: &PreparedApply<'_>,
    dispatch: ApplyDispatchOutcome,
    now: FrameClockReading,
) -> Option<PendingAttemptEndingPublication> {
    match dispatch {
        ApplyDispatchOutcome::Started => None,
        ApplyDispatchOutcome::Deferred(waiting) => {
            bindings.set_wait(prepared.role, prepared.started_at, waiting);
            None
        },
        ApplyDispatchOutcome::Rejected(rejection) => {
            record_prestart_rejection(
                bindings,
                prepared.role,
                prepared.device_revision,
                now,
                prepared.deadline,
                rejection,
            );
            None
        },
    }
}

fn release_replaced_session(
    world: &mut World,
    bindings: &Bindings,
    drivers: &mut Drivers,
    role: &RoleKey,
) -> Result<(), ApplyDispatchRejection> {
    let Some(established) = bindings
        .established_sessions()
        .into_iter()
        .find(|established| established.role == *role)
    else {
        return Ok(());
    };
    let role_entity = bindings
        .role_entity(role)
        .map_err(ApplyDispatchRejection::Binding)?;
    let role_entity = DriverCleanupRoleEntity::checked(world, role_entity);
    drivers
        .release_session(
            world,
            established.driver,
            role,
            role_entity,
            established.session,
            SessionReleaseCause::ReplacementApply,
        )
        .map_err(ApplyDispatchRejection::DriverContract)
}

fn record_started_contract_failure(
    bindings: &mut Bindings,
    devices: &Devices,
    retained: ActiveAttemptRecord,
    error: DriverContractError,
    now: FrameClockReading,
) -> PendingAttemptEndingPublication {
    let report = DriverContractFailureReport::from(error);
    bindings.record_dispatch_refused(
        &retained.role,
        role_device_revision(devices, bindings, &retained.role),
        now,
        retained.attempt.deadline(),
        WaitingCondition::DriverRepair {
            error: report.clone(),
        },
    );
    PendingAttemptEndingPublication::live(&retained, AttemptEnding::ContractFailed(report))
}

fn record_prestart_rejection(
    bindings: &mut Bindings,
    role: &RoleKey,
    device_revision: DeviceRevision,
    now: FrameClockReading,
    wait_bound: Instant,
    rejection: ApplyDispatchRejection,
) {
    match rejection {
        ApplyDispatchRejection::Binding(error) => {
            warn!("role `{role}`: binding refused apply: {error}");
            bindings.record_dispatch_refused(
                role,
                DeviceRevisionLookup::Retained(device_revision),
                now,
                wait_bound,
                WaitingCondition::ApplicationBindingRepairRequired,
            );
        },
        ApplyDispatchRejection::DriverContract(error) => {
            let report = DriverContractFailureReport::from(error);
            bindings.record_dispatch_refused(
                role,
                DeviceRevisionLookup::Retained(device_revision),
                now,
                wait_bound,
                WaitingCondition::DriverRepair { error: report },
            );
        },
    }
}

fn waiting_condition_for_target(
    world: &World,
    endpoint: &crate::DeviceEndpoint,
    wait: TargetWait,
    now: Instant,
) -> WaitingCondition {
    match wait {
        TargetWait::Reporter { reporter, error } => {
            let kind = target_reporter_wait_kind(world, reporter, &error);
            reporter_wait_for_ids(world, kind, &endpoint.device, &[reporter], now)
        },
        TargetWait::ApplicationRoleAttachmentRequired => {
            WaitingCondition::ApplicationTargetAttachmentRequired
        },
        TargetWait::ApplicationCapabilityRegistrationRequired { failure } => {
            WaitingCondition::ApplicationCapabilityRegistrationRequired { failure }
        },
    }
}

/// Names the reporter state behind a driver target that is not ready yet.
///
/// A missing projected capability is not proof that the physical device is absent. Before the
/// driver's reporter completes its first set, it is an awaiting-first-report wait; after a set
/// completes without the capability, it is unconfirmed evidence. Other access errors mean the
/// reporter currently cannot reach a usable target.
fn target_reporter_wait_kind(
    world: &World,
    reporter: crate::ReporterId,
    error: &DeviceAccessError,
) -> ReporterWaitKind {
    if !matches!(error, DeviceAccessError::Absent { .. }) {
        return ReporterWaitKind::Unreachable;
    }
    world
        .resource::<Reporters>()
        .registered_reporters()
        .find(|registered| registered.reporter == reporter)
        .map_or(
            ReporterWaitKind::Unconfirmed,
            |registered| match registered.contribution {
                ReporterContribution::AwaitingFirstCompleteSet => {
                    ReporterWaitKind::AwaitingFirstReport
                },
                ReporterContribution::Completed { .. } => ReporterWaitKind::Unconfirmed,
            },
        )
}

fn apply_abort_policy(
    bindings: &mut Bindings,
    role: &RoleKey,
    attempt_invalidation: AttemptInvalidation,
) {
    if attempt_invalidation != AttemptInvalidation::RevisionAdvanced {
        return;
    }
    let reverts = bindings.binding(role).is_ok_and(|binding| {
        binding.on_abort == OnAbort::Revert
            && !matches!(
                binding.last_known_good,
                LastKnownGoodConfiguration::NotEstablished
            )
    });
    if reverts {
        bindings.set_waiting_work(role, WaitingWork::RestorationOwed);
    }
}

pub(crate) fn publish_pending_attempt_endings(world: &mut World) {
    let ready = {
        let mut pending = world.resource_mut::<PendingAttemptEndingPublications>();
        let mut ready = Vec::new();
        for publication in std::mem::take(&mut pending.0) {
            match publication.readiness {
                AttemptEndingPublicationReadiness::ReadyAfterStatusPublication => {
                    ready.push(publication);
                },
                AttemptEndingPublicationReadiness::ReadyAfterAttemptLifecycle => {
                    pending.0.push(publication);
                },
            }
        }
        ready
    };
    for mut publication in ready {
        match publication.lifetime {
            AttemptEndingRegistrationLifetime::Live(binding) => {
                match attempt_ending_publication_destination(
                    world,
                    &publication.role,
                    &publication.endpoint,
                    publication.generation,
                    binding,
                    publication.role_entity_recovery,
                ) {
                    AttemptEndingPublicationDestination::Live(binding) => {
                        let ending_view = binding::project_attempt_ending(&publication.ending);
                        world.entity_mut(binding).insert(publication.ending);
                        world.trigger(LiveRoleChanged {
                            binding,
                            role: publication.role,
                            change: LiveRoleChange::AttemptEnded {
                                attempt: publication.attempt,
                                ending:  ending_view,
                            },
                        });
                    },
                    AttemptEndingPublicationDestination::Ended(lifetime) => {
                        publish_ended_registration_attempt(world, publication, lifetime);
                    },
                    AttemptEndingPublicationDestination::AwaitingRoleEntityRecovery => {
                        publication.await_next_role_entity_recovery_pass();
                        world
                            .resource_mut::<PendingAttemptEndingPublications>()
                            .0
                            .push(publication);
                    },
                    AttemptEndingPublicationDestination::RegistrationRetirementRequired => {
                        retire_registration_for_pending_attempt_ending(world, publication);
                    },
                }
            },
            AttemptEndingRegistrationLifetime::Displaced => {
                publish_ended_registration_attempt(
                    world,
                    publication,
                    EndedRegistrationLifetime::Displaced,
                );
            },
            AttemptEndingRegistrationLifetime::Retired => {
                publish_ended_registration_attempt(
                    world,
                    publication,
                    EndedRegistrationLifetime::Retired,
                );
            },
        }
    }
}

fn retire_registration_for_pending_attempt_ending(
    world: &mut World,
    publication: PendingAttemptEndingPublication,
) {
    match world.resource_mut::<Bindings>().retire(&publication.role) {
        Ok(_) => {
            world
                .resource_mut::<PendingAttemptEndingPublications>()
                .0
                .push(publication);
        },
        Err(error @ BindingError::PendingTransitionCapacityReached) => {
            warn!(
                "role `{}`: registration retirement after entity recovery failed: {error}",
                publication.role
            );
            world
                .resource_mut::<PendingAttemptEndingPublications>()
                .0
                .push(publication);
        },
        Err(error @ BindingError::TransitionSequenceExhausted) => {
            warn!(
                "role `{}`: registration retirement after entity recovery failed permanently: \
                 {error}",
                publication.role
            );
            publish_ended_registration_attempt(
                world,
                publication,
                EndedRegistrationLifetime::RetirementBlocked,
            );
        },
        Err(error) => {
            // `Bindings::retire` confirms `Bindings::by_role` membership before its only fallible
            // call, `Bindings::reserve_transition`, which returns only the two variants above. The
            // shared `BindingError` return type requires this unreachable arm.
            warn!(
                "role `{}`: registration retirement after entity recovery failed unexpectedly: \
                 {error}",
                publication.role
            );
            publish_ended_registration_attempt(
                world,
                publication,
                EndedRegistrationLifetime::RetirementBlocked,
            );
        },
    }
}

fn attempt_ending_publication_destination(
    world: &World,
    role: &RoleKey,
    endpoint: &crate::DeviceEndpoint,
    generation: BindingGeneration,
    binding: Entity,
    role_entity_recovery: AttemptEndingRoleEntityRecovery,
) -> AttemptEndingPublicationDestination {
    let bindings = world.resource::<Bindings>();
    if world.get_entity(binding).is_err() {
        let Some(current_generation) = bindings.generation(role) else {
            return AttemptEndingPublicationDestination::Ended(EndedRegistrationLifetime::Retired);
        };
        if current_generation != generation {
            return AttemptEndingPublicationDestination::Ended(
                EndedRegistrationLifetime::Displaced,
            );
        }
        if let Ok(current_entity) = bindings.role_entity(role)
            && world.get_entity(current_entity).is_ok()
        {
            return AttemptEndingPublicationDestination::Live(current_entity);
        }
        return match role_entity_recovery {
            AttemptEndingRoleEntityRecovery::NotAwaiting
            | AttemptEndingRoleEntityRecovery::AwaitingNextPass => {
                AttemptEndingPublicationDestination::AwaitingRoleEntityRecovery
            },
            AttemptEndingRoleEntityRecovery::PassCompleted => {
                AttemptEndingPublicationDestination::RegistrationRetirementRequired
            },
        };
    }
    let ended_lifetime = bindings
        .pending_transitions()
        .find_map(|transition| match transition {
            BindingTransition::Replaced {
                role: transition_role,
                displaced_entity,
                endpoint: transition_endpoint,
                ..
            } if transition_role == role
                && *displaced_entity == binding
                && transition_endpoint == endpoint =>
            {
                Some(EndedRegistrationLifetime::Displaced)
            },
            BindingTransition::Retired {
                role: transition_role,
                endpoint: transition_endpoint,
                entity,
                ..
            } if transition_role == role
                && *entity == binding
                && transition_endpoint == endpoint =>
            {
                Some(EndedRegistrationLifetime::Retired)
            },
            BindingTransition::Registered { .. }
            | BindingTransition::Replaced { .. }
            | BindingTransition::Retired { .. } => None,
        });
    if let Some(ended_lifetime) = ended_lifetime {
        return AttemptEndingPublicationDestination::Ended(ended_lifetime);
    }
    AttemptEndingPublicationDestination::Live(binding)
}

fn publish_ended_registration_attempt(
    world: &mut World,
    publication: PendingAttemptEndingPublication,
    lifetime: EndedRegistrationLifetime,
) {
    world.trigger(RegistrationAttemptEnded {
        role: publication.role,
        endpoint: publication.endpoint,
        attempt: publication.attempt,
        ending: binding::project_attempt_ending(&publication.ending),
        lifetime,
    });
}

fn rearm_reacquired_roles(world: &mut World) {
    let stopped = stopped_role_endpoints(world);
    if stopped.is_empty() {
        return;
    }
    world.resource_scope::<Bindings, _>(|_world, mut bindings| {
        for (role, endpoint_availability) in stopped {
            bindings.observe_stopped_role_endpoint(&role, endpoint_availability);
        }
    });
}

fn stopped_role_endpoints(world: &World) -> Vec<(RoleKey, EndpointAvailability)> {
    let bindings = world.resource::<Bindings>();
    let devices = world.resource::<Devices>();
    bindings
        .registered_roles()
        .filter_map(|role| {
            let binding = bindings.binding(role).ok()?;
            matches!(
                bindings.projected_status(role),
                Ok(RoleStatusView::Stopped(_))
            )
            .then(|| {
                (
                    role.clone(),
                    endpoint_availability(devices, &binding.endpoint.device),
                )
            })
        })
        .collect()
}

fn endpoint_availability(devices: &Devices, device_key: &crate::DeviceKey) -> EndpointAvailability {
    let DeviceResolution::Resolved(_) = devices.resolve(device_key) else {
        return EndpointAvailability::Gone;
    };
    match devices.key_availability(device_key) {
        PriorKeyAvailability::Published(KeyAvailability::Present(_)) => {
            EndpointAvailability::Available
        },
        PriorKeyAvailability::NeverPublished | PriorKeyAvailability::Published(_) => {
            EndpointAvailability::Gone
        },
    }
}

fn role_device_revision(
    devices: &Devices,
    bindings: &Bindings,
    role: &RoleKey,
) -> DeviceRevisionLookup {
    let Ok(binding) = bindings.binding(role) else {
        return DeviceRevisionLookup::Retired;
    };
    match devices.resolve(&binding.endpoint.device) {
        DeviceResolution::NotResolved => DeviceRevisionLookup::Retired,
        DeviceResolution::Resolved(device_id) => devices.revision(device_id),
    }
}

fn startable_roles(world: &World) -> Vec<RoleKey> {
    let bindings = world.resource::<Bindings>();
    let devices = world.resource::<Devices>();
    let now = FrameClockReading::from(world.resource::<Time<Real>>());
    startable_roles_from(world, bindings, devices, now)
}

fn startable_roles_from(
    world: &World,
    bindings: &Bindings,
    devices: &Devices,
    now: FrameClockReading,
) -> Vec<RoleKey> {
    bindings
        .registered_roles()
        .filter(|role| {
            bindings
                .retry_pacing(role)
                .permits_dispatch(role_device_revision(devices, bindings, role), now)
                && !bindings.waiting_work(role).holds_for_application()
                && !bindings
                    .pending_transitions()
                    .any(|transition| match transition {
                        BindingTransition::Registered {
                            role: transitioned, ..
                        }
                        | BindingTransition::Replaced {
                            role: transitioned, ..
                        }
                        | BindingTransition::Retired {
                            role: transitioned, ..
                        } => transitioned == *role,
                    })
                && matches!(
                    bindings.projected_status(role),
                    Ok(RoleStatusView::Waiting(_))
                )
                && endpoint_wait_requires_update(world, bindings, devices, role, now)
        })
        .cloned()
        .collect()
}

fn endpoint_wait_requires_update(
    world: &World,
    bindings: &Bindings,
    devices: &Devices,
    role: &RoleKey,
    now: FrameClockReading,
) -> bool {
    let FrameClockReading::Measurable(measured) = now else {
        return false;
    };
    let Ok(binding) = bindings.binding(role) else {
        return false;
    };
    if let DeviceResolution::Resolved(_) = devices.resolve(&binding.endpoint.device) {
        return match devices.key_availability(&binding.endpoint.device) {
            PriorKeyAvailability::Published(KeyAvailability::Present(_))
            | PriorKeyAvailability::NeverPublished => true,
            PriorKeyAvailability::Published(availability) => {
                let condition =
                    availability_wait(world, &binding.endpoint.device, availability, measured);
                bindings.wait_requires_update(role, measured, &condition)
            },
        };
    }
    let condition = unresolved_device_wait(world, devices, &binding.endpoint.device, measured);
    bindings.wait_requires_update(role, measured, &condition)
}

fn unresolved_device_wait(
    world: &World,
    devices: &Devices,
    device_key: &crate::DeviceKey,
    now: Instant,
) -> WaitingCondition {
    if let PriorKeyAvailability::Published(availability) = devices.key_availability(device_key) {
        return availability_wait(world, device_key, availability, now);
    }
    let reporters = world.resource::<Reporters>();
    let mut awaiting = Vec::new();
    let mut completed = Vec::new();
    let mut disabled = Vec::new();
    for registered in reporters
        .registered_reporters()
        .filter(|reporter| reporter.coverage.establishes_absence_for(device_key))
    {
        if registered.activation == ReporterActivation::Disabled {
            disabled.push(registered.reporter);
            continue;
        }
        match &registered.contribution {
            ReporterContribution::AwaitingFirstCompleteSet => awaiting.push(registered),
            ReporterContribution::Completed { .. } => completed.push(registered),
        }
    }
    if !awaiting.is_empty() {
        return reporter_wait(
            ReporterWaitKind::AwaitingFirstReport,
            device_key,
            now,
            awaiting,
            world.resource::<RiggingLimits>(),
        );
    }
    if completed.is_empty()
        && let Some((first, rest)) = disabled.split_first()
    {
        return WaitingCondition::ApplicationReporterEnable {
            reporters: NonEmptyReporterIds::from_reporter_ids(*first, rest),
        };
    }
    reporter_wait(
        ReporterWaitKind::Absent,
        device_key,
        now,
        completed,
        world.resource::<RiggingLimits>(),
    )
}

fn authorization_wait(
    world: &World,
    devices: &Devices,
    device_id: crate::DeviceId,
    error: &ApplyAuthorizationError,
    now: Instant,
) -> WaitingCondition {
    let DeviceStateLookup::Retained(state) = devices.state(device_id) else {
        return WaitingCondition::NewRegistration;
    };
    match error {
        ApplyAuthorizationError::NotPresent { .. } => {
            if let PriorKeyAvailability::Published(availability) =
                devices.key_availability(&state.key)
            {
                return availability_wait(world, &state.key, availability, now);
            }
            let kind = match state.presence {
                Presence::Unreachable { .. } => ReporterWaitKind::Unreachable,
                Presence::Absent => ReporterWaitKind::Absent,
                Presence::Present => return WaitingCondition::NewRegistration,
            };
            reporter_wait_for_ids(world, kind, &state.key, &state.contributors, now)
        },
        ApplyAuthorizationError::ClaimUnavailable { .. } => match &state.claim {
            Claim::Contended { holder } => WaitingCondition::ClaimRelease {
                holder: holder.clone(),
            },
            Claim::Blocked { gate } => {
                WaitingCondition::ApplicationPermissionRequired { gate: gate.clone() }
            },
            Claim::Held | Claim::Free | Claim::NotApplicable => {
                WaitingCondition::KernelStateRepairRequired
            },
        },
        ApplyAuthorizationError::IdentityNotProven { .. } => reporter_wait_for_ids(
            world,
            ReporterWaitKind::Unconfirmed,
            &state.key,
            &state.contributors,
            now,
        ),
        ApplyAuthorizationError::DeviceRetired { .. } => WaitingCondition::NewRegistration,
        ApplyAuthorizationError::Offline { .. } => {
            WaitingCondition::ApplicationDeviceEnableRequired {
                key: state.key.clone(),
            }
        },
    }
}

fn reporter_wait_for_ids(
    world: &World,
    kind: ReporterWaitKind,
    device_key: &crate::DeviceKey,
    reporter_ids: &[crate::ReporterId],
    now: Instant,
) -> WaitingCondition {
    let reporters = world.resource::<Reporters>();
    let registered = reporters
        .registered_reporters()
        .filter(|reporter| reporter_ids.contains(&reporter.reporter))
        .collect();
    reporter_wait(
        kind,
        device_key,
        now,
        registered,
        world.resource::<RiggingLimits>(),
    )
}

fn reporter_wait(
    kind: ReporterWaitKind,
    device_key: &crate::DeviceKey,
    now: Instant,
    registered_reporters: Vec<RegisteredReporter<'_>>,
    rigging_limits: &RiggingLimits,
) -> WaitingCondition {
    let Some((first, rest)) = registered_reporters.split_first() else {
        return WaitingCondition::ApplicationReporterRegistrationRequired {
            key: device_key.clone(),
        };
    };
    let reporter_wait = match kind {
        ReporterWaitKind::AwaitingFirstReport => {
            ReporterWait::awaiting_first_report(device_key.clone(), now, first, rest)
        },
        ReporterWaitKind::Unconfirmed => ReporterWait::unconfirmed(
            device_key.clone(),
            now,
            first,
            rest,
            rigging_limits,
            UnconfirmedBasis::NoFreshEvidence,
        ),
        ReporterWaitKind::Unreachable => {
            ReporterWait::unreachable(device_key.clone(), now, first, rest, rigging_limits)
        },
        ReporterWaitKind::Absent => {
            let established_by = std::iter::once(first)
                .chain(rest)
                .filter_map(|reporter| match reporter.contribution {
                    ReporterContribution::Completed { batch, .. } => {
                        Some(crate::RetirementEvidence::new(
                            crate::ReporterRef::from_reporter_id(reporter.reporter),
                            batch,
                        ))
                    },
                    ReporterContribution::AwaitingFirstCompleteSet => None,
                })
                .min_by_key(|evidence| evidence.reporter.get());
            let Some(established_by) = established_by else {
                return WaitingCondition::ApplicationReporterRegistrationRequired {
                    key: device_key.clone(),
                };
            };
            ReporterWait::absent(device_key.clone(), established_by)
        },
    };
    WaitingCondition::Reporter(reporter_wait)
}

pub(crate) fn availability_wait(
    world: &World,
    device_key: &crate::DeviceKey,
    availability: &crate::KeyAvailability,
    now: Instant,
) -> WaitingCondition {
    let reporters = world.resource::<Reporters>();
    let rigging_limits = world.resource::<RiggingLimits>();
    match availability {
        KeyAvailability::Present(_) => WaitingCondition::NewRegistration,
        KeyAvailability::DepartureGrace {
            deadline, evidence, ..
        } => WaitingCondition::Reporter(ReporterWait::departure_grace(
            device_key.clone(),
            world
                .resource::<crate::RiggingRuntimeClock>()
                .instant_at(*deadline),
            *evidence,
        )),
        KeyAvailability::AwaitingFirstReport {
            reporters: reporter_refs,
            ..
        } => reporter_wait_for_refs(
            reporters,
            reporter_refs.as_slice(),
            device_key,
            now,
            rigging_limits,
            ReporterWaitKind::AwaitingFirstReport,
            UnconfirmedBasis::NoFreshEvidence,
        ),
        KeyAvailability::Unconfirmed { basis, .. } => {
            let reporter_refs = match basis {
                UnconfirmedBasis::NoFreshEvidence => reporters
                    .registered_reporters()
                    .filter(|reporter| reporter.coverage.establishes_absence_for(device_key))
                    .map(|reporter| crate::ReporterRef::from_reporter_id(reporter.reporter))
                    .collect::<Vec<_>>(),
                UnconfirmedBasis::UncoveredAbsence { reporter, .. } => vec![*reporter],
            };
            reporter_wait_for_refs(
                reporters,
                &reporter_refs,
                device_key,
                now,
                rigging_limits,
                ReporterWaitKind::Unconfirmed,
                basis.clone(),
            )
        },
        KeyAvailability::Unreachable {
            reporters: reporter_refs,
            ..
        } => reporter_wait_for_refs(
            reporters,
            reporter_refs.as_slice(),
            device_key,
            now,
            rigging_limits,
            ReporterWaitKind::Unreachable,
            UnconfirmedBasis::NoFreshEvidence,
        ),
        KeyAvailability::Absent { established_by, .. } => {
            WaitingCondition::Reporter(ReporterWait::absent(device_key.clone(), *established_by))
        },
    }
}

fn reporter_wait_for_refs(
    reporters: &Reporters,
    reporter_refs: &[crate::ReporterRef],
    device_key: &crate::DeviceKey,
    now: Instant,
    rigging_limits: &RiggingLimits,
    kind: ReporterWaitKind,
    basis: crate::UnconfirmedBasis,
) -> WaitingCondition {
    let registered = reporters
        .registered_reporters()
        .filter(|reporter| {
            reporter_refs.contains(&crate::ReporterRef::from_reporter_id(reporter.reporter))
        })
        .collect::<Vec<_>>();
    let registered = if kind == ReporterWaitKind::AwaitingFirstReport {
        let registered_ids = registered
            .iter()
            .map(|reporter| reporter.reporter)
            .collect::<Vec<_>>();
        let enabled = registered
            .into_iter()
            .filter(|reporter| reporter.activation == ReporterActivation::Enabled)
            .collect::<Vec<_>>();
        if enabled.is_empty()
            && let Some((first, rest)) = registered_ids.split_first()
        {
            return WaitingCondition::ApplicationReporterEnable {
                reporters: NonEmptyReporterIds::from_reporter_ids(*first, rest),
            };
        }
        enabled
    } else {
        registered
    };
    let Some((first, rest)) = registered.split_first() else {
        return WaitingCondition::ApplicationReporterRegistrationRequired {
            key: device_key.clone(),
        };
    };
    let reporter_wait = match kind {
        ReporterWaitKind::AwaitingFirstReport => {
            ReporterWait::awaiting_first_report(device_key.clone(), now, first, rest)
        },
        ReporterWaitKind::Unconfirmed => {
            ReporterWait::unconfirmed(device_key.clone(), now, first, rest, rigging_limits, basis)
        },
        ReporterWaitKind::Unreachable => {
            ReporterWait::unreachable(device_key.clone(), now, first, rest, rigging_limits)
        },
        ReporterWaitKind::Absent => return WaitingCondition::KernelStateRepairRequired,
    };
    WaitingCondition::Reporter(reporter_wait)
}

/// Empty the frame's accepted lifecycle changes once every reader has run.
pub(crate) fn clear_binding_transitions(world: &mut World) {
    if world
        .resource::<BindingTransitionBatch>()
        .transitions()
        .is_empty()
    {
        return;
    }
    world.resource_mut::<BindingTransitionBatch>().clear();
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::num::NonZeroUsize;
    use std::time::Duration;

    use bevy::app::App;
    use bevy::app::Update;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;

    use super::AttemptEndingPublicationReadiness;
    use super::AttemptEndingRegistrationLifetime;
    use super::AttemptEndingRoleEntityRecovery;
    use super::PendingAttemptEndingPublication;
    use super::PendingAttemptEndingPublications;
    use super::publish_pending_attempt_endings;
    use crate::ApplyDeadline;
    use crate::AttemptEnding;
    use crate::AttemptEndingView;
    use crate::AttemptInvalidation;
    use crate::AttemptInvalidationView;
    use crate::AttemptRef;
    use crate::AuthoredId;
    use crate::BindingAuthoring;
    use crate::BindingPolicy;
    use crate::Bindings;
    use crate::DeviceEndpoint;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DriverId;
    use crate::EndedRegistrationLifetime;
    use crate::EndpointDriverRegistration;
    use crate::EndpointId;
    use crate::OnAbort;
    use crate::OnSessionLoss;
    use crate::RecoveryPolicy;
    use crate::RegistrationAttemptEnded;
    use crate::RetryOn;
    use crate::RoleKey;
    use crate::binding;
    use crate::binding::BindingTransitionBatch;
    use crate::binding::LostRegisteredRoleEntities;
    use crate::register_binding;

    #[derive(Debug, PartialEq, Eq)]
    struct ObservedEndedRegistrationAttempt {
        role:     RoleKey,
        endpoint: DeviceEndpoint,
        attempt:  AttemptRef,
        ending:   AttemptEndingView,
        lifetime: EndedRegistrationLifetime,
    }

    #[derive(Default, Resource)]
    struct ObservedEndedRegistrationAttempts(Vec<ObservedEndedRegistrationAttempt>);

    fn observe_ended_registration_attempt(
        event: On<RegistrationAttemptEnded>,
        mut observed: ResMut<ObservedEndedRegistrationAttempts>,
    ) {
        observed.0.push(ObservedEndedRegistrationAttempt {
            role:     event.role.clone(),
            endpoint: event.endpoint.clone(),
            attempt:  event.attempt,
            ending:   event.ending.clone(),
            lifetime: event.lifetime,
        });
    }

    #[test]
    fn exhausted_retirement_sequence_publishes_the_recorded_ending_once()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("unretirable-registration")?;
        let endpoint = DeviceEndpoint {
            device: DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored {
                    value: AuthoredId::new("unretirable-device")?,
                },
            },
            id:     EndpointId::Whole,
        };
        let attempt = AttemptRef::new(31);
        let mut app = App::new();
        app.init_resource::<Bindings>()
            .init_resource::<LostRegisteredRoleEntities>()
            .init_resource::<PendingAttemptEndingPublications>()
            .init_resource::<ObservedEndedRegistrationAttempts>()
            .add_observer(observe_ended_registration_attempt);
        let binding = register_binding(
            app.world_mut(),
            BindingAuthoring::new(
                role.clone(),
                endpoint.clone(),
                EndpointDriverRegistration::<()>::new(DriverId(0)),
                (),
                BindingPolicy::new(
                    RecoveryPolicy::default(),
                    RetryOn::Interval(Duration::ZERO),
                    OnAbort::default(),
                    OnSessionLoss::default(),
                    ApplyDeadline::ProcessDefault,
                ),
            ),
        )?;
        let generation = app
            .world()
            .resource::<Bindings>()
            .generation(&role)
            .ok_or("the registered role has no generation")?;
        assert!(app.world_mut().despawn(binding));
        app.world_mut()
            .resource_mut::<Bindings>()
            .mark_transition_sequence_exhausted();
        app.world_mut()
            .resource_mut::<PendingAttemptEndingPublications>()
            .0
            .push(PendingAttemptEndingPublication {
                role: role.clone(),
                endpoint: endpoint.clone(),
                generation,
                attempt,
                ending: AttemptEnding::Invalidated(AttemptInvalidation::DeviceNotPresent),
                lifetime: AttemptEndingRegistrationLifetime::Live(binding),
                readiness: AttemptEndingPublicationReadiness::ReadyAfterStatusPublication,
                role_entity_recovery: AttemptEndingRoleEntityRecovery::PassCompleted,
            });

        publish_pending_attempt_endings(app.world_mut());
        publish_pending_attempt_endings(app.world_mut());

        assert!(app.world().resource::<Bindings>().binding(&role).is_ok());
        assert_eq!(
            app.world()
                .resource::<ObservedEndedRegistrationAttempts>()
                .0,
            vec![ObservedEndedRegistrationAttempt {
                role,
                endpoint,
                attempt,
                ending: AttemptEndingView::Invalidated(AttemptInvalidationView::DeviceNotPresent,),
                lifetime: EndedRegistrationLifetime::RetirementBlocked,
            }]
        );
        Ok(())
    }

    #[test]
    fn full_retirement_transition_queue_requeues_then_publishes_once() -> Result<(), Box<dyn Error>>
    {
        let role = RoleKey::new("capacity-blocked-retirement")?;
        let endpoint = DeviceEndpoint {
            device: DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored {
                    value: AuthoredId::new("capacity-blocked-device")?,
                },
            },
            id:     EndpointId::Whole,
        };
        let attempt = AttemptRef::new(32);
        let mut app = App::new();
        app.init_resource::<Bindings>()
            .init_resource::<BindingTransitionBatch>()
            .init_resource::<LostRegisteredRoleEntities>()
            .init_resource::<PendingAttemptEndingPublications>()
            .init_resource::<ObservedEndedRegistrationAttempts>()
            .add_systems(Update, binding::drain_binding_transitions)
            .add_observer(observe_ended_registration_attempt);
        let binding = register_binding(
            app.world_mut(),
            BindingAuthoring::new(
                role.clone(),
                endpoint.clone(),
                EndpointDriverRegistration::<()>::new(DriverId(0)),
                (),
                BindingPolicy::new(
                    RecoveryPolicy::default(),
                    RetryOn::Interval(Duration::ZERO),
                    OnAbort::default(),
                    OnSessionLoss::default(),
                    ApplyDeadline::ProcessDefault,
                ),
            ),
        )?;
        let generation = app
            .world()
            .resource::<Bindings>()
            .generation(&role)
            .ok_or("the registered role has no generation")?;
        app.world_mut()
            .resource_mut::<Bindings>()
            .set_pending_transition_capacity(NonZeroUsize::MIN)?;
        assert!(app.world_mut().despawn(binding));
        app.world_mut()
            .resource_mut::<PendingAttemptEndingPublications>()
            .0
            .push(PendingAttemptEndingPublication {
                role: role.clone(),
                endpoint: endpoint.clone(),
                generation,
                attempt,
                ending: AttemptEnding::Invalidated(AttemptInvalidation::DeviceNotPresent),
                lifetime: AttemptEndingRegistrationLifetime::Live(binding),
                readiness: AttemptEndingPublicationReadiness::ReadyAfterStatusPublication,
                role_entity_recovery: AttemptEndingRoleEntityRecovery::PassCompleted,
            });

        publish_pending_attempt_endings(app.world_mut());

        assert!(
            app.world()
                .resource::<ObservedEndedRegistrationAttempts>()
                .0
                .is_empty()
        );
        assert!(app.world().resource::<Bindings>().binding(&role).is_ok());

        app.update();
        publish_pending_attempt_endings(app.world_mut());
        publish_pending_attempt_endings(app.world_mut());
        publish_pending_attempt_endings(app.world_mut());

        assert!(app.world().resource::<Bindings>().binding(&role).is_err());
        assert_eq!(
            app.world()
                .resource::<ObservedEndedRegistrationAttempts>()
                .0,
            vec![ObservedEndedRegistrationAttempt {
                role,
                endpoint,
                attempt,
                ending: AttemptEndingView::Invalidated(AttemptInvalidationView::DeviceNotPresent,),
                lifetime: EndedRegistrationLifetime::Retired,
            }]
        );
        Ok(())
    }
}
