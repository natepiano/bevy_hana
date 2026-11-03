//! Window restore startup, target state, and settle verification.

mod restore_attempt;
#[cfg(test)]
mod settle_regression_tests;
mod settle_state;
mod target_position;
mod winit_info;
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
mod x11_position_fix;

use bevy::prelude::App;
use bevy::prelude::ApplyDeferred;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::Time;
use bevy::prelude::Update;
use bevy::time::Virtual;
use hana_rigging::prelude::RiggingSystems;
pub(crate) use restore_attempt::RestorePreparation;
pub(crate) use restore_attempt::RestorePreparationSource;
pub(crate) use restore_attempt::RetainedWindowedPhysicalPosition;
pub(crate) use restore_attempt::WindowRestoreAttempt;
pub(crate) use restore_attempt::prepare_driver_restore_targets;
pub(crate) use restore_attempt::remove_window_restore_work;
pub(crate) use restore_attempt::resolve_legacy_coordinate_monitor;
pub(crate) use settle_state::check_restore_settling;
pub(crate) use target_position::FullscreenRestoreState;
pub(crate) use target_position::MonitorScaleStrategy;
pub(crate) use target_position::NativeFullscreenState;
pub(crate) use target_position::NativeWindowReadbackPosition;
use target_position::ObservedScaleInputs;
#[cfg(test)]
pub(crate) use target_position::TargetPosition;
pub(crate) use target_position::WindowRestoreState;
pub(crate) use target_position::place_window_at_saved_geometry;
pub(crate) use target_position::rebase_legacy_position;
#[cfg(test)]
pub(crate) use winit_info::InjectedWinitWindows;

use crate::ClerestoryUpdateSet;
use crate::ClerestoryWindowDriverSet;
pub(crate) struct RestorePlugin;

impl Plugin for RestorePlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(target_position::clear_fullscreen_observation);

        app.configure_sets(
            Update,
            (
                ClerestoryWindowDriverSet::BindingAuthoring
                    .after(ClerestoryUpdateSet::CurrentMonitor)
                    .in_set(RiggingSystems::Prepare),
                ClerestoryWindowDriverSet::TargetPreparation
                    .after(ClerestoryWindowDriverSet::BindingAuthoring)
                    .in_set(RiggingSystems::Prepare),
            ),
        )
        .init_resource::<ObservedScaleInputs>()
        .init_resource::<Time<Virtual>>();

        app.add_systems(
            Update,
            (
                target_position::capture_scale_inputs,
                (
                    prepare_driver_restore_targets,
                    ApplyDeferred,
                    place_window_at_saved_geometry,
                    check_restore_settling,
                    ApplyDeferred,
                )
                    .chain(),
            )
                .in_set(ClerestoryWindowDriverSet::TargetPreparation),
        );

        #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
        x11_position_fix::register(app);
    }
}
