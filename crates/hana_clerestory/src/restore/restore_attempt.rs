//! Window target preparation using kernel-issued role and attempt identity.

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use bevy::prelude::World;
use bevy::window::OnMonitor;
use bevy::winit::WinitMonitors;
use hana_rigging::prelude::AttemptRef;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::RoleKey;

use super::target_position;
use super::target_position::NativeWindowReadbackPosition;
use super::target_position::PreparedPositionMeaning;
use super::target_position::RestoreDiagnostics;
use super::target_position::TargetPosition;
use super::winit_info;
#[cfg(any(test, feature = "test"))]
use super::winit_info::InjectedWinitWindows;
use super::winit_info::NativeMonitorAssociation;
use super::winit_info::NativeWindowInfo;
use super::winit_info::X11FrameCompensated;
use crate::Platform;
use crate::driver::RestoreRecord;
use crate::driver::WindowPlacementTarget;
use crate::driver::WindowRoleDriverState;
use crate::monitors;
use crate::monitors::CurrentMonitor;
use crate::monitors::CurrentMonitorEntity;
use crate::monitors::MonitorDescriptor;
use crate::monitors::Monitors;
use crate::persistence::PersistedPosition;
use crate::persistence::PersistedWindowState;
use crate::persistence::WindowPlacementConfiguration;
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
    dispatched: WindowPlacementConfiguration,
}

/// Copyable window-entity reference to one driver-owned restore preparation.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(crate) struct WindowRestoreAttempt {
    role:   RoleKey,
    source: RestorePreparationSource,
}

/// Exact physical readback retained for a windowed return on the same monitor.
#[derive(Component)]
pub(crate) struct RetainedWindowedPhysicalPosition {
    position: NativeWindowReadbackPosition,
    monitor:  MonitorDescriptor,
}

impl RetainedWindowedPhysicalPosition {
    fn apply_to(
        &self,
        target: &mut TargetPosition,
        monitor: &MonitorDescriptor,
        configuration: &WindowPlacementConfiguration,
    ) {
        if self.monitor == *monitor && !configuration.saved_window_mode().is_fullscreen() {
            target.retain_physical_windowed_position(self.position);
        }
    }
    pub(crate) fn retain(
        world: &mut World,
        entity: Entity,
        position: NativeWindowReadbackPosition,
        monitor: MonitorDescriptor,
    ) {
        world.entity_mut(entity).insert(Self { position, monitor });
    }

    pub(crate) fn clear(world: &mut World, entity: Entity) {
        world.entity_mut(entity).remove::<Self>();
    }
}

/// Authority that requested one window-specific target preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum RestorePreparationSource {
    /// The kernel issued this attempt and remains the authority for its lifecycle.
    KernelAttempt(AttemptRef),
}

enum RestoreMonitorObservation {
    Live(CurrentMonitor),
    Pending,
}

/// Whether Bevy has synchronized an `OnMonitor` association onto this window yet.
///
/// Bevy writes `OnMonitor` only after a `Window` change it watched, so its absence is a
/// synchronization Hana is still waiting for — never an assertion that the window sits on no
/// monitor.
enum SynchronizedMonitorAssociation {
    /// Bevy has synchronized this window onto that exact monitor entity.
    Synchronized(Entity),
    /// No association has been synchronized onto this window yet.
    AwaitingSynchronization,
}

impl SynchronizedMonitorAssociation {
    /// Convert the association Bevy writes only after a `Window` change it watched.
    fn from_component(on_monitor: Option<&OnMonitor>) -> Self {
        on_monitor.map_or(Self::AwaitingSynchronization, |on_monitor| {
            Self::Synchronized(on_monitor.0)
        })
    }
}

/// The monitor observation this window still carries from an earlier preparation.
///
/// Restore keeps the last observation on the window so an unchanged one is not rewritten every
/// frame. Its absence means nothing has been observed for this window yet, so any new
/// observation is a change worth writing.
enum RetainedMonitorObservation {
    /// The window carries this observation from an earlier preparation.
    Retained(CurrentMonitor),
    /// The window carries no observation, so the first one to arrive is written.
    NotRetained,
}

impl RetainedMonitorObservation {
    /// Convert the observation an earlier preparation left on this window.
    fn from_component(existing: Option<&CurrentMonitor>) -> Self {
        existing.map_or(Self::NotRetained, |existing| Self::Retained(*existing))
    }
}

impl RestorePreparation {
    pub(crate) const fn new(
        role: RoleKey,
        target: WindowPlacementTarget,
        dispatched: WindowPlacementConfiguration,
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
    pub(crate) const fn dispatched(&self) -> &WindowPlacementConfiguration { &self.dispatched }
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

/// Refresh exact native monitor observations and prepare each driver's missing placement target.
pub(crate) fn prepare_driver_restore_targets(
    mut commands: Commands,
    preparations: Query<(
        Entity,
        &WindowRestoreAttempt,
        &Window,
        Option<&OnMonitor>,
        Option<&CurrentMonitor>,
        Has<TargetPosition>,
        Option<&RetainedWindowedPhysicalPosition>,
    )>,
    monitors: Res<Monitors>,
    winit_monitors: Res<WinitMonitors>,
    platform: Res<Platform>,
    mut driver_state: ResMut<WindowRoleDriverState>,
    mut fallback: ResMut<WindowFallbackRecoveryState>,
    _: NonSendMarker,
    #[cfg(any(test, feature = "test"))] injected_windows: Option<Res<InjectedWinitWindows>>,
) {
    for (entity, restore_attempt, window, on_monitor, existing, has_target, retained) in
        &preparations
    {
        let NativeWindowInfo::Measured(native_window_frame) = winit_info::native_window_info(
            entity,
            #[cfg(any(test, feature = "test"))]
            injected_windows.as_deref(),
        ) else {
            if has_target {
                clear_restore_monitor_observation(&mut commands, entity);
            }
            continue;
        };
        let native_association = winit_info::native_monitor_association(
            entity,
            &monitors,
            &winit_monitors,
            #[cfg(any(test, feature = "test"))]
            injected_windows.as_deref(),
        );
        let RestoreMonitorObservation::Live(current_monitor) = refresh_restore_monitor_observation(
            &mut commands,
            entity,
            window,
            native_association,
            SynchronizedMonitorAssociation::from_component(on_monitor),
            RetainedMonitorObservation::from_component(existing),
            &monitors,
        ) else {
            continue;
        };
        // Monitor arrival can also occur during fullscreen or cross-DPI placement. Keep its
        // observation current while retaining the target dispatched for this attempt.
        if has_target {
            continue;
        }
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
                    detail: format!("authorized monitor for role {role} is no longer live"),
                },
            );
            commands.entity(entity).remove::<WindowRestoreAttempt>();
            continue;
        };
        let mut target_position = target_position::compute_established_target_position(
            &configuration,
            &descriptor,
            native_window_frame.physical_decoration(),
            current_monitor.scale,
            *platform,
        );
        if let Some(retained) = retained {
            retained.apply_to(&mut target_position, &descriptor, &configuration);
        }
        let position_meaning = target_position::prepared_established_position_meaning(
            &configuration,
            &descriptor,
            *platform,
        );
        let diagnostics = restore_diagnostics(&current_monitor, &target_position);
        // A windowed X11 restore waits for `compensate_target_position` to subtract the title
        // bar height. Every other restore carries no frame to subtract, so
        // `place_window_at_saved_geometry` may run immediately.
        let awaits_frame_compensation =
            platform.awaits_frame_compensation(&target_position.saved_window_mode);
        if current_monitor.descriptor != descriptor
            && matches!(
                platform.fallback_return_capability(
                    configuration.position(),
                    configuration.saved_window_mode(),
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

fn restore_diagnostics(current: &CurrentMonitor, target: &TargetPosition) -> RestoreDiagnostics {
    RestoreDiagnostics {
        starting_monitor_index: current.index,
        starting_scale:         current.scale,
        target_scale:           target.target_scale,
        monitor_scale_strategy: target.monitor_scale_strategy.clone(),
    }
}

fn refresh_restore_monitor_observation(
    commands: &mut Commands,
    entity: Entity,
    window: &Window,
    native_association: NativeMonitorAssociation,
    synchronized: SynchronizedMonitorAssociation,
    retained: RetainedMonitorObservation,
    monitors: &Monitors,
) -> RestoreMonitorObservation {
    let association = match native_association {
        NativeMonitorAssociation::Observed(monitor) => OnMonitor(monitor),
        NativeMonitorAssociation::AwaitingObservation => {
            let SynchronizedMonitorAssociation::Synchronized(monitor) = synchronized else {
                clear_restore_monitor_observation(commands, entity);
                return RestoreMonitorObservation::Pending;
            };
            OnMonitor(monitor)
        },
        NativeMonitorAssociation::AwaitingTopology => {
            clear_restore_monitor_observation(commands, entity);
            return RestoreMonitorObservation::Pending;
        },
    };
    let Some(current_monitor) =
        monitors::current_monitor_from_association(window, &association, monitors)
    else {
        clear_restore_monitor_observation(commands, entity);
        return RestoreMonitorObservation::Pending;
    };
    let association_changed = match synchronized {
        SynchronizedMonitorAssociation::AwaitingSynchronization => true,
        SynchronizedMonitorAssociation::Synchronized(previous) => previous != association.0,
    };
    let observation_changed = match retained {
        RetainedMonitorObservation::NotRetained => true,
        RetainedMonitorObservation::Retained(previous) => {
            monitors::exact_monitor_association(&association, &previous, monitors).is_none()
                || previous.effective_window_mode != current_monitor.effective_window_mode
        },
    };
    if association_changed || observation_changed {
        commands.entity(entity).insert((
            CurrentMonitorEntity::new(association.0),
            current_monitor,
            association,
        ));
    }
    RestoreMonitorObservation::Live(current_monitor)
}

/// Discard geometry derived from an association the native window no longer confirms.
/// The driver retains the attempt and prepares again once an exact live observation arrives.
fn clear_restore_monitor_observation(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<(
        OnMonitor,
        CurrentMonitor,
        CurrentMonitorEntity,
        TargetPosition,
        PreparedPositionMeaning,
        X11FrameCompensated,
    )>();
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

/// The one live monitor whose physical bounds contain a pre-v3 coordinate's reconstructed window
/// center.
///
/// Returns `None` when `persisted.position` is not `PersistedPosition::Unrebased`, when the center
/// lies on no monitor in `Monitors`, and when it lies on several overlapping monitors.
pub(crate) fn resolve_legacy_coordinate_monitor<'a>(
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
mod tests {
    use bevy::prelude::IVec2;

    use super::*;
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
            resolve_legacy_coordinate_monitor(&persisted, &monitors),
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

        assert_eq!(
            resolve_legacy_coordinate_monitor(&persisted, &monitors),
            None
        );
    }

    #[test]
    fn unmatched_saved_display_and_coordinate_never_fall_back_to_a_live_monitor() {
        let only = descriptor(0, IVec2::ZERO, UVec2::new(1_000, 1_000));
        let monitors = Monitors::from_test_monitors([(Entity::from_bits(1), only)]);
        let Some(persisted) = legacy_state(IVec2::new(4_000, 4_000)) else {
            return;
        };

        assert_eq!(
            resolve_legacy_coordinate_monitor(&persisted, &monitors),
            None
        );
    }
}
