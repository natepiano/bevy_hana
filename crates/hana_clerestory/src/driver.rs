//! Kernel endpoint driver for Clerestory-managed windows.

#[cfg(test)]
use std::collections::HashMap;

use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::World;
use hana_rigging::prelude::Applied;
use hana_rigging::prelude::ApplyContext;
use hana_rigging::prelude::AttemptFinish;
use hana_rigging::prelude::AttemptInvalidation;
#[cfg(test)]
use hana_rigging::prelude::AttemptLookup;
use hana_rigging::prelude::AttemptRecord;
use hana_rigging::prelude::AttemptRef;
use hana_rigging::prelude::BegunAttempt;
use hana_rigging::prelude::CancelledAttempt;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::DriverAbortReason;
use hana_rigging::prelude::DriverCleanupRoleEntity;
use hana_rigging::prelude::DriverLedger;
use hana_rigging::prelude::DriverRecords;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointDriverRegistration;
use hana_rigging::prelude::EstablishedContext;
use hana_rigging::prelude::Establishing;
use hana_rigging::prelude::Establishment;
use hana_rigging::prelude::EstablishmentRefusal;
use hana_rigging::prelude::InFlightAttemptMut;
use hana_rigging::prelude::ReleasedLease;
use hana_rigging::prelude::RoleKey;
#[cfg(test)]
use hana_rigging::prelude::SessionLookup;
use hana_rigging::prelude::SessionRecord;
use hana_rigging::prelude::SessionRef;
use hana_rigging::prelude::SessionReleaseCause;
use hana_rigging::prelude::TargetResolution;
use hana_rigging::prelude::TargetResolutionContext;
use hana_rigging::prelude::TargetWait;
#[cfg(test)]
use hana_rigging_scripted::RecordedCleanup;
#[cfg(test)]
use hana_rigging_scripted::RecordedCleanups;

use crate::WindowRevealDisposition;
use crate::managed;
use crate::monitors::LiveDisplayEndpoint;
use crate::monitors::MonitorReporterId;
use crate::persistence::EstablishedWindowPlacement;
use crate::restore;
use crate::restore::RestorePreparation;
use crate::restore::WindowRestoreAttempt;
use crate::visibility::SavedDisplayRevealWait;

/// Live window and monitor facts resolved before the kernel issues an attempt.
pub(crate) struct WindowPlacementTarget {
    pub(crate) window:  Entity,
    pub(crate) monitor: Entity,
}

/// The records this driver asks the ledger to keep for it.
///
/// The ledger holds one attempt record from `begin_attempt` until the attempt ends, and one
/// session record from establishment until the driver discards it, which is why
/// [`WindowRoleDriverState`] keeps no attempt-keyed or role-keyed map of its own. Every window
/// fact this driver used to store beside the ledger is now read back through a ledger verb, so
/// the two records can no longer disagree about what the driver is placing.
struct WindowRecords;

impl DriverRecords for WindowRecords {
    type Attempt = WindowRestoreRecord;
    type Session = EstablishedWindow;
}

/// Window lifetime associated with a restore attempt the driver has not finished with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionWindowLifetime {
    /// The window entity still exists.
    Present(Entity),
    /// The window entity ended while this attempt still had a claim on it.
    Ended(Entity),
}

/// What one window placement attempt is placing, and whether that window still exists.
///
/// One record carries the attempt through both of the ledger's attempt states, because both
/// states need it: while the attempt applies, `preparation` is what the restore pipeline rebuilds
/// its target from every frame until the window settles; once its completion is queued, the same
/// record's `lifetime` is the judgement establishment is made from. A window can die between the
/// completion the kernel accepted and the callback that establishes it, and nothing in
/// `hana_rigging` can see that.
///
/// It holds no [`AttemptCompletion`](hana_rigging::prelude::AttemptCompletion): the one-use
/// authority to end the attempt belongs to the ledger. Splitting the two leaves this record pure
/// data — it can be read, rebuilt, or dropped without any risk of ending the attempt.
pub(crate) struct WindowRestoreRecord {
    preparation: RestorePreparation,
    lifetime:    CompletionWindowLifetime,
}

/// The window an established role is placing, as the ledger's session record holds it.
///
/// A named record rather than a bare `Entity` in the ledger's session slot, because the slot is
/// the only place this fact now lives and a reader has to learn from the type which of a role's
/// three entities it is: the window this role's session placed, not the role entity the kernel
/// keys the binding by and not the monitor the placement targeted.
struct EstablishedWindow(Entity);

/// What the ledger's attempt record holds for one attempt the restore pipeline is asking about.
///
/// Replaces the bare `Option<&RestorePreparation>` this driver used to answer, which conflated an
/// attempt this driver never heard of with one whose preparation was already spent on a success.
/// The pipeline treats both as "nothing to prepare", but only one of them is a defect if it ever
/// starts happening, and an absence cannot say which it is.
pub(crate) enum RestoreRecord<'preparation> {
    /// The attempt is applying and this preparation is what its target is built from.
    UnderPreparation(&'preparation RestorePreparation),
    /// The attempt reported success; its completion is queued for the kernel to establish.
    CompletionQueued,
    /// No attempt in this driver's ledger carries that reference.
    AttemptUnknown,
}

/// What the ledger handed back for an attempt the kernel cancelled.
///
/// Replaces the bare `Option<Entity>` the cancel path used to answer. The first variant names a
/// window whose restore work is still on the entity and has to come off; the second says the
/// cancel named no attempt this role is running, and the driver's window is not to be touched.
#[must_use]
enum CancelledRestore {
    /// The cancelled attempt had this window under restore; its restore work is still on it.
    WindowUnderRestore(Entity),
    /// No attempt this role is running carries that reference.
    AttemptUnknown,
}

/// One aborted attempt's restore work, still standing on the window that attempt named.
///
/// The ledger ends an attempt inside itself and has no reach into the world, so the four restore
/// markers the attempt left are still on its window when [`WindowRoleDriverState::abort_window`]
/// returns. The pair named here is exactly what the strip needs: the window the markers are on,
/// and the attempt they were filed under, which [`restore::remove_window_restore_work`] matches
/// against the marker before removing anything.
struct AbandonedRestoreWork {
    window:  Entity,
    attempt: AttemptRef,
}

/// Every restore claim [`WindowRoleDriverState::abort_window`] ended, for its caller to strip.
///
/// Not dropped in place, because only one of `abort_window`'s two callers is a window that is
/// going away. `on_primary_window_removed` fires on `Remove<PrimaryWindow>`, and a registered name
/// keeps that entity managed and alive: its `TargetPosition`, `PreparedPositionMeaning` and
/// `X11FrameCompensated` would stand on a live window forever, and a stale `TargetPosition` keeps
/// `prepare_driver_restore_targets` — whose query is `Without<TargetPosition>` — from ever
/// re-targeting it. A window that really is despawning makes each strip a no-op, so both callers
/// take the same path.
#[must_use]
pub(crate) struct AbandonedWindowRestoreWork(Vec<AbandonedRestoreWork>);

impl AbandonedWindowRestoreWork {
    /// Take every abandoned attempt's restore work off the window it named.
    ///
    /// Queued rather than applied here because both callers are observers holding no `&mut World`,
    /// and routed through [`restore::remove_window_restore_work`] so that helper stays the single
    /// owner of the marker list. Queue it before any other command the observer issues for the
    /// same window: the strip matches the marker's own attempt, and a `WindowRestoreAttempt`
    /// removed first would leave the rest of that attempt's markers standing.
    pub(crate) fn strip(self, commands: &mut Commands) {
        if self.0.is_empty() {
            return;
        }
        commands.queue(move |world: &mut World| {
            for abandoned in self.0 {
                restore::remove_window_restore_work(world, abandoned.window, abandoned.attempt);
            }
        });
    }
}

/// What the ledger's session record holds for one role after the kernel settled an establishment.
enum WindowEstablishment {
    /// The role holds a session over this live window, and the session record names it.
    Established(Entity),
    /// No session was filed, and the ledger holds no window for this role from this attempt.
    NoSessionFiled,
}

/// The window driver's state, which is the ledger and nothing else.
///
/// Every kernel-issued authority — the retained
/// [`AttemptCompletion`](hana_rigging::prelude::AttemptCompletion) for each in-flight attempt and
/// the [`SessionLease`](hana_rigging::prelude::SessionLease) for each established role — and every
/// window fact this driver owns now live in the same place, because the ledger carries the
/// driver's own records alongside them. The settled geometry a readback produced is not among
/// them: it goes to the kernel as [`Applied::DiffersFromDispatched`] and is retained there as the
/// binding's last-known-good configuration.
#[derive(Default, Resource)]
pub(crate) struct WindowRoleDriverState {
    ledger:   DriverLedger<EstablishedWindowPlacement, WindowRecords>,
    /// Every cleanup verb the kernel sent this driver, per role, oldest first.
    #[cfg(test)]
    cleanups: HashMap<RoleKey, Vec<RecordedCleanup>>,
}

/// Process-local identifier the rigging kernel issued for `WindowEndpointDriver`.
#[derive(Resource)]
pub(crate) struct WindowDriverId(pub(crate) EndpointDriverRegistration<EstablishedWindowPlacement>);

impl WindowRoleDriverState {
    #[must_use]
    pub(crate) fn restore_record(&self, attempt: AttemptRef) -> RestoreRecord<'_> {
        match self.ledger.attempt_record(attempt) {
            AttemptRecord::Applying(record) => RestoreRecord::UnderPreparation(&record.preparation),
            AttemptRecord::CompletionQueued(_) => RestoreRecord::CompletionQueued,
            AttemptRecord::AttemptUnknown => RestoreRecord::AttemptUnknown,
        }
    }

    /// Record one cleanup verb the kernel sent this driver, whether or not the driver owned it.
    ///
    /// Every call is logged, including the ones the guards below then decline to act on: the
    /// conformance walk asserts what the driver *received*, and a release naming a session a
    /// successor already replaced is exactly the case this driver used to get wrong.
    #[cfg(test)]
    fn record_cleanup(&mut self, role: &RoleKey, cleanup: RecordedCleanup) {
        self.cleanups.entry(role.clone()).or_default().push(cleanup);
    }

    pub(crate) fn finish_as_dispatched(&mut self, attempt: AttemptRef) {
        self.ledger.succeed_attempt(attempt, Applied::AsDispatched);
    }

    pub(crate) fn finish_with_readback(
        &mut self,
        attempt: AttemptRef,
        placement: EstablishedWindowPlacement,
    ) {
        self.ledger
            .succeed_attempt(attempt, Applied::DiffersFromDispatched(placement));
    }

    /// Report that the display an applying attempt was placing onto is no longer live.
    ///
    /// The record comes back so the caller can undo what the attempt left in the world; every
    /// caller strips the window's restore markers itself, which is the whole of that undoing.
    pub(crate) fn fail_attempt(
        &mut self,
        attempt: AttemptRef,
        error: DeviceAccessError,
    ) -> AttemptFinish<WindowRestoreRecord> {
        self.ledger.fail_attempt(attempt, error)
    }

    /// End an applying attempt the driver's own work has run out for, and take its record back.
    pub(crate) fn abort_attempt(
        &mut self,
        attempt: AttemptRef,
    ) -> AttemptFinish<WindowRestoreRecord> {
        self.ledger
            .abort_attempt(attempt, DriverAbortReason::OperationEnded)
    }

    /// End every claim this driver's ledger holds on a window entity whose lifetime is ending.
    ///
    /// An applying attempt is aborted outright; a queued completion keeps its record and has its
    /// lifetime marked [`CompletionWindowLifetime::Ended`], because only establishment can spend
    /// the completion the kernel is still holding, and that is where the ended window is reported.
    /// The applying attempts are collected before any is aborted: the walk holds the ledger
    /// mutably, and aborting inside it would end the very iteration that finds the rest.
    ///
    /// The restore work each aborted attempt left in the world comes back rather than being
    /// dropped — see [`AbandonedWindowRestoreWork`] for why the caller has to strip it.
    pub(crate) fn abort_window(&mut self, window: Entity) -> AbandonedWindowRestoreWork {
        let mut applying = Vec::new();
        for (attempt, _, record) in self.ledger.attempts_mut() {
            match record {
                InFlightAttemptMut::Applying(record) if record.window() == window => {
                    applying.push(attempt);
                },
                InFlightAttemptMut::CompletionQueued(record) => record.end_window_lifetime(window),
                InFlightAttemptMut::Applying(_) => {},
            }
        }
        let mut abandoned = Vec::new();
        for attempt in applying {
            match self.abort_attempt(attempt) {
                AttemptFinish::Finished { record } => abandoned.push(AbandonedRestoreWork {
                    window: record.window(),
                    attempt,
                }),
                // Each attempt was read out of an applying slot one loop above and nothing since
                // has touched the ledger, so neither refusal is reachable from here.
                AttemptFinish::AlreadyFinished | AttemptFinish::AttemptUnknown => {},
            }
        }
        AbandonedWindowRestoreWork(abandoned)
    }

    /// Give back the ledger's record for a cancelled attempt, on the ledger's own verdict.
    ///
    /// A cancel naming the wrong role, or an attempt no slot carries, hands nothing back and
    /// must leave this role's window exactly as it is: acting on the cause alone would strip the
    /// restore work of an attempt that is still running.
    fn cancel(&mut self, role: &RoleKey, attempt: AttemptRef) -> CancelledRestore {
        match self.ledger.cancel_attempt(role, attempt) {
            CancelledAttempt::Applying { record }
            | CancelledAttempt::CompletionQueued { record } => {
                CancelledRestore::WindowUnderRestore(record.window())
            },
            CancelledAttempt::WrongRole | CancelledAttempt::Unknown => {
                CancelledRestore::AttemptUnknown
            },
        }
    }

    /// Judge the window behind an accepted completion, then take the kernel's lease.
    ///
    /// Whether the window this attempt placed still exists is a fact only the attempt's own record
    /// holds, so it is read first and handed to the ledger as [`Establishing`]. The reading has to
    /// come first because `establish_lease` spends the very completion the record sits behind:
    /// judge, then establish, then read the session record back.
    fn establish(
        &mut self,
        context: EstablishedContext<'_, EstablishedWindowPlacement>,
    ) -> WindowEstablishment {
        let role = context.role().clone();
        let hardware = match self.ledger.attempt_record(context.attempt()) {
            AttemptRecord::CompletionQueued(record) => record.hardware_at_establishment(&role),
            // A superseded attempt has no record of its own left to speak for, and an
            // establishment against an applying attempt is not a state the kernel produces.
            // Neither has a window to answer for, and the ledger refuses both on the spent
            // completion alone, so the judgement they carry never reaches a lease.
            AttemptRecord::Applying(_) | AttemptRecord::AttemptUnknown => Establishing::Live,
        };
        let established = self.ledger.establish_lease(context, hardware, |record| {
            EstablishedWindow(record.window())
        });
        match established {
            // The lease is filed, either cleanly or over a predecessor the kernel never released
            // and never took back. The predecessor's record is dropped rather than acted on: its
            // window is this same role's window, so hiding it would blank the very window the
            // successor session just took over.
            Establishment::Established { .. }
            | Establishment::EstablishedOverUnreleased { .. }
            | Establishment::EstablishedOverRetained { .. } => {
                match self.ledger.session_record(&role) {
                    SessionRecord::Holding(established) => {
                        WindowEstablishment::Established(established.window())
                    },
                    // A filed lease the ledger cannot name a window for cannot happen: it files one
                    // only by converting the attempt record the judgement above was read from.
                    SessionRecord::LossReported(_)
                    | SessionRecord::Retained(_)
                    | SessionRecord::NoSession => WindowEstablishment::NoSessionFiled,
                }
            },
            // Neither refusal leaves a session. `NoQueuedCompletion` means a successor already
            // displaced this attempt's record, so the successor's window is keyed by the
            // successor's own attempt. `SessionEnded` hands back the record for a window that is
            // already gone, so no hide runs, no `SavedDisplayRevealWait` arms, and no lease was
            // filed for a later `release_session` to name.
            Establishment::Refused(
                EstablishmentRefusal::NoQueuedCompletion
                | EstablishmentRefusal::SessionEnded { .. },
            ) => WindowEstablishment::NoSessionFiled,
        }
    }

    /// Give the role's lease back to the ledger and let its window record go with it.
    ///
    /// The answer is the guard on everything the release path does to the window itself: only
    /// [`ReleasedLease::Released`] says this release named the session the role still held. The
    /// released record is discarded in the same breath rather than retained, because the hide
    /// below finds the window through the role entity — a retained record would be a second name
    /// for a window nothing reads it for.
    fn release(&mut self, role: &RoleKey, session: SessionRef) -> ReleasedLease {
        let released = self.ledger.release_lease(role, session);
        if matches!(released, ReleasedLease::Released) {
            let _ = self.ledger.discard_retained(role);
        }
        released
    }

    pub(crate) fn configuration_changed(
        &mut self,
        role: &RoleKey,
        configuration: EstablishedWindowPlacement,
    ) {
        self.ledger.change_configuration(role, configuration);
    }
}

impl WindowRestoreRecord {
    const fn new(preparation: RestorePreparation) -> Self {
        let window = preparation.window();
        Self {
            preparation,
            lifetime: CompletionWindowLifetime::Present(window),
        }
    }

    const fn window(&self) -> Entity {
        match self.lifetime {
            CompletionWindowLifetime::Present(window) | CompletionWindowLifetime::Ended(window) => {
                window
            },
        }
    }

    /// Whether the window this attempt placed is still there to establish a session over.
    ///
    /// Read before [`DriverLedger::establish_lease`], because that call spends the queued
    /// completion this record sits behind: afterwards the driver could no longer tell a live
    /// window from one that ended.
    fn hardware_at_establishment(&self, role: &RoleKey) -> Establishing {
        match self.lifetime {
            CompletionWindowLifetime::Present(_) => Establishing::Live,
            CompletionWindowLifetime::Ended(ended) => {
                Establishing::Ended(DeviceAccessError::Absent {
                    detail: format!(
                        "window {ended} for role {role} ended before its session was established"
                    ),
                })
            },
        }
    }

    fn end_window_lifetime(&mut self, window: Entity) {
        if self.lifetime == CompletionWindowLifetime::Present(window) {
            self.lifetime = CompletionWindowLifetime::Ended(window);
        }
    }
}

impl EstablishedWindow {
    const fn window(&self) -> Entity { self.0 }
}

/// Take a superseded attempt's restore markers off the window its ledger record names.
///
/// The ledger hands the displaced record back but has no reach into the world, and the four
/// restore markers the abandoned attempt left are still on its window. Only
/// [`EndpointDriver::start_apply`]'s `&mut World` can take those off, and it has to do so before
/// the successor's marker goes on: both may name the same window,
/// [`restore::remove_window_restore_work`] matches on the marker's own attempt, and a strip
/// running second would find the successor's marker, decline, and leave a stale `TargetPosition`
/// standing — which keeps `prepare_driver_restore_targets`, whose query is
/// `Without<TargetPosition>`, from ever building the successor a target.
fn strip_superseded_restore_work(
    world: &mut World,
    superseded: AttemptRef,
    displaced: WindowRestoreRecord,
) {
    restore::remove_window_restore_work(world, displaced.window(), superseded);
}

/// Which attempt the window driver's ledger records for one role.
///
/// A free function rather than a method, because the state is private to this module and the
/// in-crate tests that read it are siblings: this and [`window_session_lookup`] are the whole
/// surface they need now that the ledger is the driver's only record.
#[cfg(test)]
pub(crate) fn window_attempt_lookup(world: &World, role: &RoleKey) -> AttemptLookup {
    world
        .resource::<WindowRoleDriverState>()
        .ledger
        .attempt_of(role)
}

/// Whether the window driver's ledger holds anything at all, under any role.
///
/// The per-role readers above answer for a role the test already names, which is what a test
/// asserting an attempt was withheld cannot use: an attempt filed under a role key nobody
/// expected is invisible to them and is precisely the defect worth catching. This walks every
/// role's slot and lease through the ledger's own whole-ledger answer.
#[cfg(test)]
pub(crate) fn window_ledger_is_empty(world: &World) -> bool {
    world
        .resource::<WindowRoleDriverState>()
        .ledger
        .holds_no_role()
}

/// Which session the window driver's ledger records for one role.
#[cfg(test)]
pub(crate) fn window_session_lookup(world: &World, role: &RoleKey) -> SessionLookup {
    world
        .resource::<WindowRoleDriverState>()
        .ledger
        .session_of(role)
}

/// Every cleanup verb the kernel has sent this driver for one role, oldest first.
#[cfg(test)]
pub(crate) fn recorded_window_cleanups(world: &World, role: &RoleKey) -> RecordedCleanups {
    world
        .resource::<WindowRoleDriverState>()
        .cleanups
        .get(role)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// Applies `EstablishedWindowPlacement` values after the rigging kernel authorizes a display.
pub(crate) struct WindowEndpointDriver;

impl EndpointDriver for WindowEndpointDriver {
    type Configuration = EstablishedWindowPlacement;
    type Target = WindowPlacementTarget;

    fn resolve_target(
        &mut self,
        world: &mut World,
        context: &TargetResolutionContext<'_>,
        _: &Self::Configuration,
    ) -> TargetResolution<Self::Target> {
        let Some(window) = managed::window_for_role_entity(world, context.role_entity()) else {
            return TargetResolution::Deferred(TargetWait::ApplicationRoleAttachmentRequired);
        };
        let reporter = world.resource::<MonitorReporterId>().get();
        let live_display_endpoint =
            match context.required_capability::<LiveDisplayEndpoint>(world, reporter) {
                Ok(live_display_endpoint) => live_display_endpoint.clone(),
                Err(unavailable) => {
                    return TargetResolution::Deferred(unavailable.target_wait(world));
                },
            };
        TargetResolution::Reached(WindowPlacementTarget {
            window,
            monitor: live_display_endpoint.monitor,
        })
    }

    fn start_apply(
        &mut self,
        world: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        configuration: &Self::Configuration,
        target: Self::Target,
    ) {
        let role = context.target().role().clone();
        let window = target.window;
        let record = WindowRestoreRecord::new(RestorePreparation::new(
            role.clone(),
            target,
            configuration.clone(),
        ));
        let begun = world
            .resource_mut::<WindowRoleDriverState>()
            .ledger
            .begin_attempt(context, record);
        let attempt = begun.attempt();
        if let BegunAttempt::Superseding {
            superseded,
            displaced,
            ..
        } = begun
        {
            strip_superseded_restore_work(world, superseded, displaced);
        }
        let restore_attempt = WindowRestoreAttempt::for_role(role, attempt);
        world.entity_mut(window).insert(restore_attempt);
    }

    fn established(
        &mut self,
        world: &mut World,
        context: EstablishedContext<'_, Self::Configuration>,
    ) {
        let role_entity = context.role_entity();
        let establishment = world
            .resource_mut::<WindowRoleDriverState>()
            .establish(context);
        if let WindowEstablishment::Established(window) = establishment {
            debug_assert_eq!(
                Some(window),
                managed::window_for_role_entity(world, role_entity)
            );
        }
    }

    fn cancel_apply(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        invalidation: AttemptInvalidation,
    ) {
        let cancelled = {
            let mut state = world.resource_mut::<WindowRoleDriverState>();
            #[cfg(test)]
            state.record_cleanup(
                role,
                RecordedCleanup::AttemptCancelled {
                    role_entity,
                    attempt,
                    invalidation,
                },
            );
            #[cfg(not(test))]
            let _ = (role_entity, invalidation);
            state.cancel(role, attempt)
        };
        match cancelled {
            CancelledRestore::WindowUnderRestore(window) => {
                restore::remove_window_restore_work(world, window, attempt);
            },
            CancelledRestore::AttemptUnknown => {},
        }
    }

    fn release_session(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    ) {
        let released = {
            let mut state = world.resource_mut::<WindowRoleDriverState>();
            #[cfg(test)]
            state.record_cleanup(
                role,
                RecordedCleanup::SessionReleased {
                    role_entity,
                    session,
                    cause: cause.clone(),
                },
            );
            state.release(role, session)
        };
        // A release naming a session a successor already replaced is not this role's window to
        // hide; only `Released` says the driver still held the session being ended.
        match released {
            ReleasedLease::Released => {},
            ReleasedLease::OtherSessionEstablished | ReleasedLease::NotEstablished => return,
        }
        // Releasing the record above is the whole job for every combination but one: a live role
        // entity whose display went away. `Removed` arrives two ways. With `BindingReplaced` or
        // `RoleRetired` no hide is wanted — Clerestory reaches retirement from the window's own
        // removal, which has already torn the window down, and a replacement rebinds the window
        // rather than hiding it. With a loss cause it means the kernel found the recorded role
        // entity already gone (`DriverCleanupRoleEntity::checked`); a window outlives its role
        // entity (`WindowRiggingRole` has no `linked_spawn`), but `window_for_role_entity` reads
        // the relationship off a live role entity and has none, so the hide is skipped and the
        // window stays as it is.
        let (
            SessionReleaseCause::DeviceUnavailable { .. }
            | SessionReleaseCause::ReportedLoss
            | SessionReleaseCause::FlowStalled,
            DriverCleanupRoleEntity::Live(role_entity),
        ) = (cause, role_entity)
        else {
            return;
        };
        let Some(window) = managed::window_for_role_entity(world, role_entity) else {
            return;
        };
        let Ok(mut window_entity) = world.get_entity_mut(window) else {
            return;
        };
        if let Some(mut window) = window_entity.get_mut::<Window>() {
            window.visible = false;
        }
        window_entity
            .remove::<WindowRevealDisposition>()
            .insert(SavedDisplayRevealWait::default());
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::UVec2;
    use bevy::window::OnMonitor;

    use super::*;
    use crate::monitors::CurrentMonitor;
    use crate::monitors::Monitors;
    use crate::restore::InjectedWinitWindows;
    use crate::restore::TargetPosition;
    use crate::tests;

    /// [`strip_superseded_restore_work`] takes every restore marker off the window it was named.
    ///
    /// What this proves, and what it does not. It exercises the helper directly, on a record built
    /// here, so it pins the helper's own property: given an attempt reference and a record naming
    /// a window, every one of the four restore markers filed under that attempt comes off. It does
    /// **not** prove [`EndpointDriver::start_apply`] calls it with the record the ledger handed
    /// back, because no test can reach that call: [`BegunAttempt::Superseding`] is what routes
    /// `start_apply` here, and no harness in this crate can make the kernel produce it.
    ///
    /// Measured rather than assumed. When the kernel ends an applying or queued attempt whose
    /// role entity is still alive — a binding replaced or retired
    /// (`cleanup_binding_transition_driver_state`), a revalidation failure, an overrun, a driver
    /// contract failure — it calls `cancel_apply`, and this driver's
    /// [`WindowRoleDriverState::cancel`] empties the ledger's slot, so the successor's
    /// `begin_attempt` finds it idle and answers `BegunAttempt::Fresh`. The retire-despawn-update
    /// recipe was run against `ProductionPluginHarness` and behaves that way: one update after
    /// the retire the ledger holds nothing and the window's markers are already gone. The one
    /// kernel path that skips `cancel_apply` — an invalidated attempt whose role entity is
    /// already gone — is the path `Superseding` exists for, and it needs a successor attempt on a
    /// role with no entity, which no harness here can stage. `ApplyContext::new` is `pub(crate)`
    /// to `hana_rigging`, so the supersede cannot be synthesized by hand either.
    ///
    /// Why the helper is worth pinning anyway: the record names the window, but the four restore
    /// markers the attempt left are in the world, where the ledger cannot reach. Left standing, a
    /// stale `TargetPosition` keeps `check_restore_settling` running against geometry no attempt
    /// is chasing and keeps `prepare_driver_restore_targets` — whose query is
    /// `Without<TargetPosition>` — from ever building the successor a target.
    #[test]
    fn strip_superseded_restore_work_removes_the_named_attempt_s_markers() -> Result<(), String> {
        let mut fixture = tests::applying_window_role()?;
        let dispatched = {
            let state = fixture.app.world().resource::<WindowRoleDriverState>();
            let RestoreRecord::UnderPreparation(preparation) =
                state.restore_record(fixture.attempt)
            else {
                return Err(String::from(
                    "the fixture's attempt holds no preparation to displace",
                ));
            };
            preparation.dispatched().clone()
        };
        // Both markers have to be standing before the strip runs, or the assertions below pass
        // against a window that never carried them. `WindowRestoreAttempt` is there already —
        // `applying_window_role` stops on the frame it appears — but `TargetPosition` is not, and
        // two things `prepare_driver_restore_targets` requires are what withhold it. Its query
        // filters on `OnMonitor`, which the harness's primary window never receives, and it skips
        // any window with no native window behind it, which a harness with no winit cannot
        // supply. Supplying both here is what makes the strip below have something to
        // remove.
        let monitor = fixture
            .app
            .world()
            .resource::<Monitors>()
            .iter()
            .find(|live| {
                fixture
                    .app
                    .world()
                    .get::<CurrentMonitor>(fixture.window)
                    .is_some_and(|current| current.descriptor == *live.descriptor)
            })
            .map(|live| live.entity)
            .ok_or_else(|| {
                String::from("the fixture's window sits on no monitor the topology reports")
            })?;
        fixture
            .app
            .world_mut()
            .entity_mut(fixture.window)
            .insert(OnMonitor(monitor));
        fixture
            .app
            .world_mut()
            .init_resource::<InjectedWinitWindows>();
        fixture
            .app
            .world_mut()
            .resource_mut::<InjectedWinitWindows>()
            .insert(fixture.window, UVec2::ZERO);
        fixture.app.update();
        assert!(
            fixture
                .app
                .world()
                .get::<WindowRestoreAttempt>(fixture.window)
                .is_some(),
            "the fixture never put a restore marker on its window"
        );
        assert!(
            fixture
                .app
                .world()
                .get::<TargetPosition>(fixture.window)
                .is_some(),
            "the fixture never built a target for the attempt, so removing one proves nothing"
        );

        let displaced = WindowRestoreRecord::new(RestorePreparation::new(
            fixture.role.clone(),
            WindowPlacementTarget {
                window: fixture.window,
                monitor,
            },
            dispatched,
        ));
        strip_superseded_restore_work(fixture.app.world_mut(), fixture.attempt, displaced);

        assert!(
            fixture
                .app
                .world()
                .get::<WindowRestoreAttempt>(fixture.window)
                .is_none(),
            "the superseded attempt's restore marker is still on its window"
        );
        assert!(
            fixture
                .app
                .world()
                .get::<TargetPosition>(fixture.window)
                .is_none(),
            "the superseded attempt's target is still on its window"
        );
        Ok(())
    }

    /// A cancel the ledger does not credit to this role must leave the driver's record alone.
    ///
    /// The ledger's verdict is the only thing that says the cancel is this role's: a cancel
    /// naming another role hands no record back, so nothing names a window to strip, and
    /// [`CancelledRestore::AttemptUnknown`] is answered from that.
    #[test]
    fn a_cancel_naming_another_role_leaves_this_role_s_preparation_in_place() -> Result<(), String>
    {
        let mut fixture = tests::applying_window_role()?;
        let other_role = RoleKey::new("another-window-role")
            .map_err(|error| format!("failed to create a second role key: {error}"))?;

        let cancelled = fixture
            .app
            .world_mut()
            .resource_mut::<WindowRoleDriverState>()
            .cancel(&other_role, fixture.attempt);

        assert!(matches!(cancelled, CancelledRestore::AttemptUnknown));
        assert!(matches!(
            fixture
                .app
                .world()
                .resource::<WindowRoleDriverState>()
                .restore_record(fixture.attempt),
            RestoreRecord::UnderPreparation(_)
        ));
        assert!(
            matches!(
                window_attempt_lookup(fixture.app.world(), &fixture.role),
                AttemptLookup::Applying(applying) if applying == fixture.attempt
            ),
            "a cancel naming another role swept this role's attempt record"
        );
        assert_eq!(
            fixture
                .app
                .world()
                .get::<WindowRestoreAttempt>(fixture.window)
                .map(WindowRestoreAttempt::attempt),
            Some(fixture.attempt),
            "a cancel naming another role took the restore marker off this role's window"
        );

        // The same cancel addressed to the role the ledger really carries is what the record
        // comes off for, so the guard above is discriminating and not simply inert.
        let owned = fixture
            .app
            .world_mut()
            .resource_mut::<WindowRoleDriverState>()
            .cancel(&fixture.role, fixture.attempt);

        assert!(matches!(
            owned,
            CancelledRestore::WindowUnderRestore(window) if window == fixture.window
        ));
        assert!(matches!(
            fixture
                .app
                .world()
                .resource::<WindowRoleDriverState>()
                .restore_record(fixture.attempt),
            RestoreRecord::AttemptUnknown
        ));
        assert!(matches!(
            window_attempt_lookup(fixture.app.world(), &fixture.role),
            AttemptLookup::Idle
        ));
        Ok(())
    }

    /// A release naming a session the role no longer holds must not touch the window.
    ///
    /// The window driver hides a window and arms [`SavedDisplayRevealWait`] when its display goes
    /// away, and the kernel can deliver that release after a successor session has already been
    /// established over the same window. Acting on the cause alone blanks a window that is live,
    /// which is why the hide is gated on [`ReleasedLease::Released`] and nothing else.
    #[test]
    fn a_stale_release_leaves_a_re_established_window_visible() -> Result<(), String> {
        let mut fixture = tests::re_established_window_role()?;
        {
            let mut window_entity = fixture
                .app
                .world_mut()
                .get_entity_mut(fixture.window)
                .map_err(|error| format!("the re-established window is gone: {error}"))?;
            {
                let mut window = window_entity
                    .get_mut::<Window>()
                    .ok_or_else(|| String::from("the re-established window lost its Window"))?;
                window.visible = true;
            }
            window_entity.remove::<SavedDisplayRevealWait>();
        }

        WindowEndpointDriver.release_session(
            fixture.app.world_mut(),
            &fixture.role,
            DriverCleanupRoleEntity::Live(fixture.role_entity),
            fixture.stale_session,
            SessionReleaseCause::ReportedLoss,
        );

        assert!(
            fixture
                .app
                .world()
                .get::<Window>(fixture.window)
                .is_some_and(|window| window.visible),
            "a stale release hid the window its successor session had re-established"
        );
        assert!(
            fixture
                .app
                .world()
                .get::<SavedDisplayRevealWait>(fixture.window)
                .is_none(),
            "a stale release armed a reveal wait on a window whose session is live"
        );
        assert!(matches!(
            window_session_lookup(fixture.app.world(), &fixture.role),
            SessionLookup::Holding(session) if session == fixture.live_session
        ));
        Ok(())
    }
}
