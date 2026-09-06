//! Window target preparation using kernel-issued role and attempt identity.

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
#[cfg(test)]
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::World;
use bevy::window::OnMonitor;
use hana_rigging::prelude::AttemptRef;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::RoleKey;

use super::target_position;
use super::target_position::PreparedPositionMeaning;
use super::target_position::RestoreDiagnostics;
use super::target_position::TargetPosition;
use super::winit_info;
#[cfg(test)]
use super::winit_info::InjectedWinitWindows;
use super::winit_info::X11FrameCompensated;
use crate::Platform;
use crate::driver::RestoreRecord;
use crate::driver::WindowPlacementTarget;
use crate::driver::WindowRoleDriverState;
use crate::monitors;
use crate::monitors::CurrentMonitor;
#[cfg(test)]
use crate::monitors::MonitorDescriptor;
use crate::monitors::Monitors;
use crate::persistence::EstablishedWindowPlacement;
#[cfg(test)]
use crate::persistence::PersistedPosition;
#[cfg(test)]
use crate::persistence::PersistedWindowState;
use crate::platform::ReturnCapability;
use crate::recovery::WindowFallbackRecoveryState;

/// Driver-owned preparation for one window placement attempt.
///
/// This value lives in the driver's ledger record, not on either the role entity or the window. It
/// is the driver's own record of what the attempt was told to place and where — the facts the
/// restore pipeline rebuilds its target from every frame until the window settles.
///
/// It holds no [`AttemptCompletion`](hana_rigging::prelude::AttemptCompletion): the one-use
/// authority to end the attempt belongs to the ledger, which retains it from `begin_attempt` and
/// spends it in `succeed_attempt`, `fail_attempt`, or `abort_attempt`. Splitting the two leaves
/// this value pure data — it can be read, rebuilt, or dropped without any risk of ending the
/// attempt, and the attempt can only ever be ended once, by the ledger, however many times the
/// pipeline runs.
pub(crate) struct RestorePreparation {
    role:       RoleKey,
    target:     WindowPlacementTarget,
    dispatched: EstablishedWindowPlacement,
}

/// Copyable window-entity reference to one driver-owned restore preparation.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(crate) struct WindowRestoreAttempt {
    role:   RoleKey,
    source: RestorePreparationSource,
}

/// Authority that requested one window-specific target preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum RestorePreparationSource {
    /// The kernel issued this attempt and remains the authority for its lifecycle.
    KernelAttempt(AttemptRef),
}

impl RestorePreparation {
    pub(crate) const fn new(
        role: RoleKey,
        target: WindowPlacementTarget,
        dispatched: EstablishedWindowPlacement,
    ) -> Self {
        Self {
            role,
            target,
            dispatched,
        }
    }

    #[must_use]
    pub(crate) const fn role(&self) -> &RoleKey { &self.role }

    #[must_use]
    pub(crate) const fn window(&self) -> Entity { self.target.window }

    #[must_use]
    pub(crate) const fn target(&self) -> &WindowPlacementTarget { &self.target }

    #[must_use]
    pub(crate) const fn dispatched(&self) -> &EstablishedWindowPlacement { &self.dispatched }
}

impl WindowRestoreAttempt {
    #[must_use]
    pub(crate) const fn for_role(role: RoleKey, attempt: AttemptRef) -> Self {
        Self {
            role,
            source: RestorePreparationSource::KernelAttempt(attempt),
        }
    }

    #[must_use]
    pub(crate) const fn role(&self) -> &RoleKey { &self.role }

    #[must_use]
    pub(crate) const fn source(&self) -> RestorePreparationSource { self.source }

    #[must_use]
    pub(crate) const fn attempt(&self) -> AttemptRef {
        match self.source {
            RestorePreparationSource::KernelAttempt(attempt) => attempt,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(role: RoleKey) -> Self { Self::for_role(role, AttemptRef::default()) }
}

/// Build one driver-requested target from the monitor facts resolved before attempt issuance.
pub(crate) fn prepare_driver_restore_targets(
    mut commands: Commands,
    preparations: Query<
        (Entity, &WindowRestoreAttempt, &OnMonitor, &CurrentMonitor),
        (With<Window>, Without<TargetPosition>),
    >,
    monitors: Res<Monitors>,
    platform: Res<Platform>,
    mut driver_state: ResMut<WindowRoleDriverState>,
    mut fallback: ResMut<WindowFallbackRecoveryState>,
    _: NonSendMarker,
    #[cfg(test)] injected_windows: Option<Res<InjectedWinitWindows>>,
) {
    for (entity, restore_attempt, on_monitor, current_monitor) in &preparations {
        if monitors::exact_monitor_association(on_monitor, current_monitor, &monitors).is_none() {
            continue;
        }
        let Some(native_window_info) = winit_info::native_window_info(
            entity,
            #[cfg(test)]
            injected_windows.as_deref(),
        ) else {
            continue;
        };
        let attempt = restore_attempt.attempt();
        // An attempt whose success is already queued for the kernel and an attempt this driver
        // never issued are different facts with one consequence here: no preparation is
        // outstanding for this window, so the marker asking for a target is stale and comes off.
        let (role, target_monitor, configuration) = match driver_state.restore_record(attempt) {
            RestoreRecord::UnderPreparation(preparation) => (
                preparation.role().clone(),
                preparation.target().monitor,
                preparation.dispatched().clone(),
            ),
            RestoreRecord::CompletionQueued | RestoreRecord::AttemptUnknown => {
                commands.entity(entity).remove::<WindowRestoreAttempt>();
                continue;
            },
        };
        let current_target_descriptor = monitors
            .iter()
            .find(|monitor| monitor.entity == target_monitor)
            .map(|monitor| *monitor.descriptor);
        let Some(descriptor) = current_target_descriptor else {
            fallback.mark_missing(role.clone());
            // The record comes back so nothing it left behind is dropped in silence; the marker
            // it put on this window comes off on the next line, which is the whole of that.
            let _ = driver_state.fail_attempt(
                attempt,
                DeviceAccessError::Absent {
                    detail: format!("authorized display for role {role} is no longer live"),
                },
            );
            commands.entity(entity).remove::<WindowRestoreAttempt>();
            continue;
        };
        let target_position = target_position::compute_established_target_position(
            &configuration,
            &descriptor,
            native_window_info.physical_decoration(),
            current_monitor.scale,
            *platform,
        );
        let position_meaning = target_position::prepared_established_position_meaning(
            &configuration,
            &descriptor,
            *platform,
        );
        let diagnostics = RestoreDiagnostics {
            starting_monitor_index: current_monitor.index,
            starting_scale:         current_monitor.scale,
            target_scale:           target_position.target_scale,
            monitor_scale_strategy: target_position.monitor_scale_strategy.clone(),
        };
        // A windowed X11 restore waits for `compensate_target_position` to subtract the title
        // bar height. Every other restore carries no frame to subtract, so
        // `place_window_at_saved_geometry` may run immediately.
        let awaits_frame_compensation =
            platform.awaits_frame_compensation(&target_position.saved_window_mode);
        if current_monitor.descriptor != descriptor
            && matches!(
                platform.fallback_return_capability(
                    configuration.position,
                    &configuration.saved_window_mode,
                ),
                ReturnCapability::Supported
            )
        {
            fallback.mark_on_fallback(role);
        }
        commands
            .entity(entity)
            .insert((target_position, diagnostics, position_meaning));

        if !awaits_frame_compensation {
            commands.entity(entity).insert(X11FrameCompensated);
        }
    }
}

pub(crate) fn remove_window_restore_work(world: &mut World, window: Entity, attempt: AttemptRef) {
    let matches_attempt = world
        .get::<WindowRestoreAttempt>(window)
        .is_some_and(|restore_attempt| restore_attempt.attempt() == attempt);
    if !matches_attempt {
        return;
    }
    let Ok(mut window_entity) = world.get_entity_mut(window) else {
        return;
    };
    window_entity.remove::<(
        WindowRestoreAttempt,
        TargetPosition,
        PreparedPositionMeaning,
        X11FrameCompensated,
    )>();
}

#[cfg(test)]
fn resolve_legacy_coordinate_monitor<'a>(
    persisted: &PersistedWindowState,
    monitors: &'a Monitors,
) -> Option<&'a MonitorDescriptor> {
    let PersistedPosition::Unrebased(unrebased) = persisted.position else {
        return None;
    };
    let center = target_position::reconstructed_legacy_window_center(
        unrebased,
        UVec2::new(persisted.logical_width, persisted.logical_height),
    );
    let mut containing = monitors
        .iter()
        .map(|monitor| monitor.descriptor)
        .filter(|descriptor| target_position::monitor_contains_physical_point(descriptor, center));
    let unique = containing.next()?;
    if containing.next().is_some() {
        return None;
    }
    Some(unique)
}

#[cfg(test)]
fn resolve_persisted_monitor<'a>(
    persisted: &PersistedWindowState,
    monitors: &'a Monitors,
) -> Option<&'a MonitorDescriptor> {
    resolve_legacy_coordinate_monitor(persisted, monitors)
}

#[cfg(test)]
mod tests {
    use bevy::prelude::IVec2;
    use bevy::prelude::UVec2;

    use super::*;
    use crate::monitors::MonitorDescriptor;
    use crate::persistence::PersistedDisplayIdentityV4;
    use crate::persistence::PersistedWindowTargetV5;
    use crate::persistence::SavedWindowMode;
    use crate::persistence::UnrebasedDesktopPosition;

    fn descriptor(index: usize, position: IVec2, size: UVec2) -> MonitorDescriptor {
        crate::monitors::MonitorDescriptor::for_current_enumeration(index, 1.0, position, size)
    }

    fn legacy_state(position: IVec2) -> Option<PersistedWindowState> {
        Some(PersistedWindowState {
            position:          PersistedPosition::Unrebased(
                UnrebasedDesktopPosition::from_test_legacy(position, 1.0)?,
            ),
            logical_width:     800,
            logical_height:    600,
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedDisplayIdentityV4::Anonymous,
            ),
            saved_window_mode: SavedWindowMode::Windowed,
            app_name:          "test".into(),
        })
    }

    #[test]
    fn legacy_coordinate_selects_its_unique_geometric_monitor_not_the_first() {
        let first = descriptor(0, IVec2::ZERO, UVec2::new(1_000, 1_000));
        let second = descriptor(1, IVec2::new(1_000, 0), UVec2::new(1_000, 1_000));
        let monitors = Monitors::from_test_monitors([
            (Entity::from_bits(1), first),
            (Entity::from_bits(2), second),
        ]);
        let Some(persisted) = legacy_state(IVec2::new(1_100, 100)) else {
            return;
        };

        assert_eq!(
            resolve_persisted_monitor(&persisted, &monitors),
            Some(&second)
        );
    }

    #[test]
    fn ambiguous_legacy_geometry_is_rejected_instead_of_choosing_one_monitor() {
        let first = descriptor(0, IVec2::ZERO, UVec2::new(2_000, 2_000));
        let second = descriptor(1, IVec2::ZERO, UVec2::new(2_000, 2_000));
        let monitors = Monitors::from_test_monitors([
            (Entity::from_bits(1), first),
            (Entity::from_bits(2), second),
        ]);
        let Some(persisted) = legacy_state(IVec2::new(100, 100)) else {
            return;
        };

        assert_eq!(resolve_persisted_monitor(&persisted, &monitors), None);
    }

    #[test]
    fn unmatched_saved_display_and_coordinate_never_fall_back_to_a_live_monitor() {
        let only = descriptor(0, IVec2::ZERO, UVec2::new(1_000, 1_000));
        let monitors = Monitors::from_test_monitors([(Entity::from_bits(1), only)]);
        let Some(persisted) = legacy_state(IVec2::new(4_000, 4_000)) else {
            return;
        };

        assert_eq!(resolve_persisted_monitor(&persisted, &monitors), None);
    }
}
