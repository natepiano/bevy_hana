//! Restore target planning, state transitions, and restore application.

mod application;
mod strategy;
mod target;

pub(crate) use application::ObservedScaleInputs;
pub(crate) use application::capture_scale_inputs;
pub(crate) use application::place_window_at_saved_geometry;
pub(crate) use strategy::FullscreenRestoreState;
pub(crate) use strategy::MonitorScaleStrategy;
pub(crate) use strategy::WindowRestoreState;
pub(crate) use target::PreparedPositionMeaning;
pub(crate) use target::RestoreDiagnostics;
pub(crate) use target::TargetPosition;
pub(crate) use target::WindowSettleProgress;
pub(crate) use target::compute_established_target_position;
#[cfg(test)]
pub(crate) use target::monitor_contains_physical_point;
pub(crate) use target::prepared_established_position_meaning;
#[cfg(test)]
pub(crate) use target::reconstructed_legacy_window_center;
