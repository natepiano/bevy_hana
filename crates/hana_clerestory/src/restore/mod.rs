//! Window restore startup, target state, and settle verification.

mod restore_attempt;
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
pub(crate) use restore_attempt::WindowApplyConfiguration;
pub(crate) use restore_attempt::clear_finished_restore_preparations;
pub(crate) use restore_attempt::prepare_driver_restore_targets;
pub(crate) use settle_state::check_restore_settling;
pub(crate) use target_position::FullscreenRestoreState;
pub(crate) use target_position::MonitorScaleStrategy;
use target_position::ObservedScaleInputs;
#[cfg(any(
    test,
    target_os = "macos",
    all(target_os = "linux", feature = "workaround-winit-4445")
))]
pub(crate) use target_position::TargetPosition;
pub(crate) use target_position::WindowRestoreState;
pub(crate) use target_position::restore_windows;
#[cfg(test)]
pub(crate) use winit_info::InjectedWinitWindows;
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
pub(crate) use winit_info::X11FrameCompensated;
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
pub(crate) use x11_position_fix::compensate_target_position;
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
pub(crate) use x11_position_fix::reapply_compensated_position;

use crate::ClerestoryUpdateSet;
use crate::ClerestoryWindowDriverSet;
use crate::driver;
#[cfg(target_os = "macos")]
use crate::macos_tabbing_fix;
pub(crate) struct RestorePlugin;

impl Plugin for RestorePlugin {
    fn build(&self, app: &mut App) {
        #[cfg(target_os = "macos")]
        app.insert_non_send(crate::macos_tabbing_fix::NativeFullscreenObservations::default())
            .add_observer(macos_tabbing_fix::clear_fullscreen_observation);

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
                    driver::discard_finished_window_attempt_results,
                    clear_finished_restore_preparations,
                    prepare_driver_restore_targets,
                    ApplyDeferred,
                    restore_windows,
                    check_restore_settling,
                    ApplyDeferred,
                )
                    .chain(),
            )
                .in_set(ClerestoryWindowDriverSet::TargetPreparation),
        );
    }
}
