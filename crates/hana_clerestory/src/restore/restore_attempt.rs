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
use bevy::window::OnMonitor;
use hana_rigging::prelude::Attempt;
use hana_rigging::prelude::AttemptId;
use hana_rigging::prelude::AttemptLookup;
use hana_rigging::prelude::AttemptOutcome;
use hana_rigging::prelude::Attempts;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::Devices;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::RoleKey;

use super::target_position;
use super::target_position::RestoreDiagnostics;
use super::target_position::TargetPosition;
use super::winit_info;
#[cfg(test)]
use super::winit_info::InjectedWinitWindows;
use super::winit_info::X11FrameCompensated;
use crate::Platform;
use crate::driver::WindowDriverAttemptResults;
use crate::monitors;
use crate::monitors::CurrentMonitor;
#[cfg(test)]
use crate::monitors::MonitorDescriptor;
use crate::monitors::MonitorDeviceAssociation;
use crate::monitors::MonitorDeviceLookup;
use crate::monitors::Monitors;
use crate::persistence::EstablishedWindowPlacement;
#[cfg(test)]
use crate::persistence::PersistedPosition;
#[cfg(test)]
use crate::persistence::PersistedWindowState;
use crate::platform::ReturnCapability;
use crate::recovery::WindowFallbackRecoveryState;

/// Target preparation tied to a kernel role and, for driver work, its issued attempt.
///
/// This component schedules window-specific preparation only. It contains no status, deadline,
/// generation, device identity, or registry; the kernel's `Attempt` remains authoritative for all
/// of those facts.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub(crate) struct RestorePreparation {
    role:   RoleKey,
    source: RestorePreparationSource,
}

/// Placement supplied by `WindowEndpointDriver::start_apply` for one kernel attempt.
///
/// The component is short-lived driver work. The authoritative configuration remains in the
/// kernel binding, and this copy lets the main-thread target builder prepare the requested window
/// without consulting a persistence adapter or rebuilding any attempt facts.
#[derive(Component, Clone)]
pub(crate) struct WindowApplyConfiguration(EstablishedWindowPlacement);

impl WindowApplyConfiguration {
    #[must_use]
    pub(crate) const fn placement(&self) -> &EstablishedWindowPlacement { &self.0 }
}

impl From<EstablishedWindowPlacement> for WindowApplyConfiguration {
    fn from(placement: EstablishedWindowPlacement) -> Self { Self(placement) }
}

/// Authority that requested one window-specific target preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum RestorePreparationSource {
    /// The kernel issued this attempt and remains the authority for its lifecycle.
    KernelAttempt(AttemptId),
}

impl RestorePreparation {
    /// Prepare window-specific work for the exact attempt issued by the kernel.
    #[must_use]
    pub(crate) fn for_attempt(attempt: &Attempt) -> Self {
        Self {
            role:   attempt.role.clone(),
            source: RestorePreparationSource::KernelAttempt(attempt.id),
        }
    }

    #[must_use]
    pub(crate) const fn role(&self) -> &RoleKey { &self.role }

    #[must_use]
    pub(crate) const fn source(&self) -> RestorePreparationSource { self.source }

    #[cfg(test)]
    pub(crate) fn for_test(role: RoleKey) -> Self {
        Self {
            role,
            source: RestorePreparationSource::KernelAttempt(AttemptId::default()),
        }
    }
}

/// Remove driver-local work whose kernel attempt no longer exists.
pub(crate) fn clear_finished_restore_preparations(
    mut commands: Commands,
    preparations: Query<(Entity, &RestorePreparation)>,
    attempts: Res<Attempts>,
) {
    for (entity, preparation) in &preparations {
        let RestorePreparationSource::KernelAttempt(attempt_id) = preparation.source();
        if matches!(attempts.in_flight(attempt_id), AttemptLookup::Finished) {
            commands
                .entity(entity)
                .remove::<RestorePreparation>()
                .remove::<WindowApplyConfiguration>();
        }
    }
}

/// Build one driver-requested target using the exact endpoint the kernel still retains.
pub(crate) fn prepare_driver_restore_targets(
    mut commands: Commands,
    preparations: Query<
        (
            Entity,
            &RestorePreparation,
            &WindowApplyConfiguration,
            &OnMonitor,
            &CurrentMonitor,
        ),
        (With<Window>, Without<TargetPosition>),
    >,
    attempts: Res<Attempts>,
    association: Res<MonitorDeviceAssociation>,
    devices: Res<Devices>,
    inventory: Res<HardwareInventory>,
    monitors: Res<Monitors>,
    platform: Res<Platform>,
    mut results: ResMut<WindowDriverAttemptResults>,
    mut fallback: ResMut<WindowFallbackRecoveryState>,
    _: NonSendMarker,
    #[cfg(test)] injected_windows: Option<Res<InjectedWinitWindows>>,
) {
    for (entity, preparation, configuration, on_monitor, current_monitor) in &preparations {
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
        let RestorePreparationSource::KernelAttempt(attempt_id) = preparation.source();
        let attempt = match attempts.in_flight(attempt_id) {
            AttemptLookup::InFlight(attempt) => attempt,
            AttemptLookup::Finished => {
                commands
                    .entity(entity)
                    .remove::<RestorePreparation>()
                    .remove::<WindowApplyConfiguration>();
                continue;
            },
        };
        let descriptor = match association.lookup(&devices, &inventory, &attempt.endpoint.device) {
            MonitorDeviceLookup::Live {
                monitor_entity: _,
                descriptor,
            } => descriptor,
            MonitorDeviceLookup::KnownWithoutLiveMonitor => {
                fallback.mark_missing(attempt.role.clone());
                results.record(
                    attempt_id,
                    AttemptOutcome::Failed(DeviceAccessError::Absent {
                        detail: format!(
                            "authorized display for role {} has no live monitor geometry",
                            attempt.role
                        ),
                    }),
                );
                commands
                    .entity(entity)
                    .remove::<RestorePreparation>()
                    .remove::<WindowApplyConfiguration>();
                continue;
            },
            MonitorDeviceLookup::UnknownDevice => {
                results.record(
                    attempt_id,
                    AttemptOutcome::Failed(DeviceAccessError::Transport {
                        detail: format!(
                            "authorized display for role {} is no longer known to Clerestory",
                            attempt.role
                        ),
                    }),
                );
                commands
                    .entity(entity)
                    .remove::<RestorePreparation>()
                    .remove::<WindowApplyConfiguration>();
                continue;
            },
        };
        let target_position = target_position::compute_established_target_position(
            configuration.placement(),
            &descriptor,
            native_window_info.physical_decoration(),
            current_monitor.scale,
            *platform,
        );
        let position_meaning = target_position::prepared_established_position_meaning(
            configuration.placement(),
            &descriptor,
            *platform,
        );
        let diagnostics = RestoreDiagnostics {
            starting_monitor_index: current_monitor.index,
            starting_scale:         current_monitor.scale,
            target_scale:           target_position.target_scale,
            monitor_scale_strategy: target_position.monitor_scale_strategy,
        };
        let needs_immediate_x11_compensation = !target_position.saved_window_mode.is_fullscreen()
            || !platform.needs_frame_compensation();
        if current_monitor.descriptor != descriptor
            && matches!(
                platform.fallback_return_capability(
                    configuration.placement().position,
                    &configuration.placement().saved_window_mode,
                ),
                ReturnCapability::Supported
            )
        {
            fallback.mark_on_fallback(attempt.role.clone());
        }
        commands
            .entity(entity)
            .insert((target_position, diagnostics, position_meaning));

        if needs_immediate_x11_compensation {
            commands.entity(entity).insert(X11FrameCompensated);
        }
    }
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
    use crate::persistence::PersistedPanelIdentityV4;
    use crate::persistence::PersistedWindowTargetV5;
    use crate::persistence::SavedWindowMode;
    use crate::persistence::UnrebasedDesktopPosition;

    fn descriptor(index: usize, position: IVec2, size: UVec2) -> MonitorDescriptor {
        MonitorDescriptor::for_current_enumeration(index, 1.0, position, size)
    }

    fn legacy_state(position: IVec2) -> Option<PersistedWindowState> {
        Some(PersistedWindowState {
            position:          PersistedPosition::Unrebased(
                UnrebasedDesktopPosition::from_test_legacy(position, 1.0)?,
            ),
            logical_width:     800,
            logical_height:    600,
            target:            PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedPanelIdentityV4::Anonymous,
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
    fn unmatched_saved_panel_and_coordinate_never_fall_back_to_a_live_monitor() {
        let only = descriptor(0, IVec2::ZERO, UVec2::new(1_000, 1_000));
        let monitors = Monitors::from_test_monitors([(Entity::from_bits(1), only)]);
        let Some(persisted) = legacy_state(IVec2::new(4_000, 4_000)) else {
            return;
        };

        assert_eq!(resolve_persisted_monitor(&persisted, &monitors), None);
    }
}
