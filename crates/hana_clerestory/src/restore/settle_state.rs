//! Settle checking logic.
//!
//! After a window restore is applied, monitors the actual window state each frame
//! to confirm the compositor delivered matching values (or detect mismatches).

use std::time::Duration;

use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::IVec2;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Time;
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use bevy::prelude::WindowPosition;
use bevy::prelude::With;
use bevy::prelude::debug;
use bevy::prelude::warn;
use bevy::window::WindowMode;
use hana_kana::ToI32;
use hana_kana::ToU32;
use hana_rigging::prelude::RoleKey;

use super::WindowRestoreAttempt;
use super::target_position::PreparedPositionMeaning;
use super::target_position::TargetPosition;
use super::target_position::WindowSettleProgress;
use super::winit_info::X11FrameCompensated;
use crate::Platform;
use crate::constants::MILLIS_PER_SECOND;
use crate::constants::PRIMARY_MONITOR_INDEX;
use crate::constants::SETTLE_STABILITY_SECS;
use crate::constants::SETTLE_TIMEOUT_SECS;
use crate::deadline::OperatingSystemWorkDeadline;
use crate::deadline::OperatingSystemWorkDeadlineStatus;
use crate::deadline::WindowStabilityInterval;
use crate::deadline::WindowStabilityIntervalStatus;
use crate::driver::WindowRoleDriverState;
use crate::events::ExpectedLogicalPosition;
use crate::events::ExpectedPhysicalPosition;
use crate::events::ObservedLogicalPosition;
use crate::events::ObservedPhysicalPosition;
use crate::events::WindowRestoreMismatch;
use crate::events::WindowRestored;
use crate::monitors::CurrentMonitor;
use crate::monitors::CurrentMonitorIndex;
use crate::persistence::EstablishedWindowPlacement;
use crate::recovery::WindowFallbackRecoveryState;

/// Window-state observation used to detect changes between frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
struct SettleObservation {
    physical_position: ObservedPhysicalPosition,
    physical_size:     UVec2,
    window_mode:       WindowMode,
    monitor:           CurrentMonitorIndex,
}

/// Tracks the two-timer settling state after restore completes.
#[derive(Debug, Clone, Reflect)]
pub(crate) struct SettleState {
    /// Hard deadline timer — fires mismatch if stability is never reached.
    total_timeout:    OperatingSystemWorkDeadline,
    /// Resets whenever any compared value changes between frames.
    stability_timer:  WindowStabilityInterval,
    /// Last frame's compared values, used to detect changes.
    last_observation: SettleObservationHistory,
    /// Total settle duration retained only for diagnostics.
    total_elapsed:    Duration,
}

#[derive(Debug, Clone, Reflect)]
enum SettleObservationHistory {
    NotObserved,
    Previous(SettleObservation),
}

impl SettleState {
    /// Create a new settle state with default durations.
    #[must_use]
    pub(super) fn new() -> Self {
        Self {
            total_timeout:    OperatingSystemWorkDeadline::new(SETTLE_TIMEOUT_SECS),
            stability_timer:  WindowStabilityInterval::new(SETTLE_STABILITY_SECS),
            last_observation: SettleObservationHistory::NotObserved,
            total_elapsed:    Duration::ZERO,
        }
    }
}

#[derive(Clone, Copy)]
enum Comparison {
    Match,
    Mismatch,
}

impl From<bool> for Comparison {
    fn from(matches: bool) -> Self { if matches { Self::Match } else { Self::Mismatch } }
}

impl Comparison {
    const fn is_match(self) -> bool { matches!(self, Self::Match) }
}

struct SettleComparison {
    position: Comparison,
    size:     Comparison,
    mode:     Comparison,
    monitor:  Comparison,
}

impl SettleComparison {
    const fn all_match(&self) -> bool {
        self.position.is_match()
            && self.size.is_match()
            && self.mode.is_match()
            && self.monitor.is_match()
    }
}

#[derive(Clone, Copy)]
enum ChangeHandling {
    Skip,
    ChangedAtDeadline,
    Unchanged,
}

/// Bundled actual values for settle mismatch reporting.
struct SettleActual {
    settle_observation: SettleObservation,
    scale:              f64,
    logical_size:       UVec2,
}

/// Placement information available when a window settles away from its requested target.
enum SettlePlacementEvidence {
    /// The live window and current monitor produced an exact placement value.
    ReadBack(EstablishedWindowPlacement),
    /// The settle ended without a current monitor from which to produce a placement.
    NotProduced,
}

/// Target values for settle resolution, grouped so they pass as one argument.
struct SettleTarget {
    physical_position: ExpectedPhysicalPosition,
    logical_position:  ExpectedLogicalPosition,
    logical_size:      UVec2,
    physical_size:     UVec2,
    window_mode:       WindowMode,
    monitor:           CurrentMonitorIndex,
    scale:             f64,
}

impl SettleTarget {
    const fn from_target_position(
        target_position: &TargetPosition,
        position_meaning: PreparedPositionMeaning,
        platform: Platform,
    ) -> Self {
        let physical_position = match position_meaning {
            PreparedPositionMeaning::Specified if platform.position_available() => {
                match target_position.physical_position() {
                    specified @ ExpectedPhysicalPosition::Specified(_) => specified,
                    ExpectedPhysicalPosition::PlatformCannotPosition
                    | ExpectedPhysicalPosition::NotSaved
                    | ExpectedPhysicalPosition::DiscardedLegacy => {
                        ExpectedPhysicalPosition::NotSaved
                    },
                }
            },
            PreparedPositionMeaning::Specified
            | PreparedPositionMeaning::PlatformCannotPosition => {
                ExpectedPhysicalPosition::PlatformCannotPosition
            },
            PreparedPositionMeaning::NotSaved => ExpectedPhysicalPosition::NotSaved,
            PreparedPositionMeaning::DiscardedLegacy => ExpectedPhysicalPosition::DiscardedLegacy,
        };
        let logical_position = match position_meaning {
            PreparedPositionMeaning::Specified if platform.position_available() => {
                match target_position.logical_position() {
                    specified @ ExpectedLogicalPosition::Specified(_) => specified,
                    ExpectedLogicalPosition::PlatformCannotPosition
                    | ExpectedLogicalPosition::NotSaved
                    | ExpectedLogicalPosition::DiscardedLegacy => ExpectedLogicalPosition::NotSaved,
                }
            },
            PreparedPositionMeaning::Specified
            | PreparedPositionMeaning::PlatformCannotPosition => {
                ExpectedLogicalPosition::PlatformCannotPosition
            },
            PreparedPositionMeaning::NotSaved => ExpectedLogicalPosition::NotSaved,
            PreparedPositionMeaning::DiscardedLegacy => ExpectedLogicalPosition::DiscardedLegacy,
        };
        Self {
            physical_position,
            logical_position,
            logical_size: target_position.logical_size,
            physical_size: target_position.physical_size,
            window_mode: target_position
                .saved_window_mode
                .to_window_mode(target_position.monitor_index),
            monitor: target_position.monitor_index,
            scale: target_position.target_scale,
        }
    }
}

const fn expected_physical_position(target: &SettleTarget) -> ExpectedPhysicalPosition {
    target.physical_position
}

const fn expected_logical_position(target: &SettleTarget) -> ExpectedLogicalPosition {
    target.logical_position
}

fn observed_physical_position(position: Option<IVec2>) -> ObservedPhysicalPosition {
    position.map_or(
        ObservedPhysicalPosition::PlatformCannotReport,
        ObservedPhysicalPosition::Observed,
    )
}

fn observed_logical_position(
    physical_position: ObservedPhysicalPosition,
    scale: f64,
) -> ObservedLogicalPosition {
    match physical_position {
        ObservedPhysicalPosition::Observed(position) => {
            ObservedLogicalPosition::Observed(IVec2::new(
                (f64::from(position.x) / scale).round().to_i32(),
                (f64::from(position.y) / scale).round().to_i32(),
            ))
        },
        ObservedPhysicalPosition::PlatformCannotReport => {
            ObservedLogicalPosition::PlatformCannotReport
        },
    }
}

/// Observe the current window state, returning compared values and the actual scale factor.
///
/// Scale is tracked separately because it is informational rather than part of settling.
fn observe_actual_state(
    window: &Window,
    current_monitor: Option<&CurrentMonitor>,
    platform: Platform,
) -> (SettleObservation, f64) {
    let physical_position = if platform.position_available() {
        match window.position {
            WindowPosition::At(p) => Some(IVec2::new(p.x, p.y)),
            _ => None,
        }
    } else {
        None
    };
    let physical_size = UVec2::new(
        window.resolution.physical_width(),
        window.resolution.physical_height(),
    );
    (
        SettleObservation {
            physical_position: observed_physical_position(physical_position),
            physical_size,
            window_mode: window.mode,
            monitor: current_monitor.map_or_else(
                || CurrentMonitorIndex::from_current_enumeration(PRIMARY_MONITOR_INDEX),
                |current_monitor| current_monitor.descriptor.index,
            ),
        },
        f64::from(window.resolution.scale_factor()),
    )
}

/// Check whether actual window state matches the target for settle purposes.
///
/// Fullscreen modes skip position and size comparison — the window fills the
/// monitor so the stored position/size are irrelevant. On macOS, borderless
/// fullscreen reports position offset by the menu bar height; on X11 (W6),
/// frame vs client coords differ. The physical size can also differ when
/// scales differ between backends (e.g. Wayland scale 1 vs `XWayland` scale 2).
fn check_settle_matches(
    target_position: &TargetPosition,
    target_physical_position: ExpectedPhysicalPosition,
    target_physical_size: UVec2,
    target_window_mode: WindowMode,
    target_monitor: CurrentMonitorIndex,
    settle_observation: &SettleObservation,
    platform: Platform,
) -> SettleComparison {
    let is_fullscreen = target_position.saved_window_mode.is_fullscreen();
    // Skip position comparison when:
    // - fullscreen (window fills monitor; saved position is irrelevant)
    // - no saved position (window was anchored via `WindowPosition::Centered`; the resulting `At`
    //   position is OS-chosen and not part of the comparison)
    // - X11 W6 frame-vs-client coordinate mismatch
    let position_matches = is_fullscreen
        || !platform.position_reliable_for_settle()
        || !matches!(
            target_physical_position,
            ExpectedPhysicalPosition::Specified(_)
        )
        || matches!(
            (target_physical_position, settle_observation.physical_position),
            (
                ExpectedPhysicalPosition::Specified(expected),
                ObservedPhysicalPosition::Observed(observed)
            ) if expected == observed
        );
    let size_match = is_fullscreen || target_physical_size == settle_observation.physical_size;
    let mode_match = platform.modes_match(target_window_mode, settle_observation.window_mode);
    let monitor_match = target_monitor == settle_observation.monitor;
    SettleComparison {
        position: position_matches.into(),
        size:     size_match.into(),
        mode:     mode_match.into(),
        monitor:  monitor_match.into(),
    }
}

/// Detect whether the settle observation changed from the previous frame and reset the
/// stability timer if so.
fn detect_settle_change(
    settle: &mut SettleState,
    settle_observation: SettleObservation,
    role: &RoleKey,
    total_elapsed_ms: f32,
    deadline_status: OperatingSystemWorkDeadlineStatus,
) -> ChangeHandling {
    let changed = match &settle.last_observation {
        SettleObservationHistory::NotObserved => true,
        SettleObservationHistory::Previous(previous) => previous != &settle_observation,
    };
    if changed {
        if matches!(
            &settle.last_observation,
            SettleObservationHistory::Previous(_)
        ) {
            debug!(
                "[check_restore_settling] [{role}] {total_elapsed_ms:.0}ms: values changed, \
                 resetting stability timer"
            );
        }
        settle.stability_timer.reset();
        settle.last_observation = SettleObservationHistory::Previous(settle_observation);
        match deadline_status {
            OperatingSystemWorkDeadlineStatus::Expired => ChangeHandling::ChangedAtDeadline,
            OperatingSystemWorkDeadlineStatus::Pending => ChangeHandling::Skip,
        }
    } else {
        ChangeHandling::Unchanged
    }
}

/// Check settling windows each frame using a two-timer approach.
///
/// - **Stability timer** (200ms): resets whenever any compared value changes. Once values stay
///   stable for 200ms the settle resolves: `WindowRestored` when they match the target,
///   `WindowRestoreMismatch` when they do not. A stable mismatch is already final — the OS has
///   finished placing the window somewhere else (a constrained position, a user drag mid-restore) —
///   so waiting longer cannot change it, only delay the capture of where the window really is.
/// - **Total timeout** (2s): hard deadline. Fires `WindowRestoreMismatch` if stability is never
///   reached during startup restoration.
///
/// Runs while `TargetPosition` entities exist (same gate as `place_window_at_saved_geometry`).
/// Only processes entities that have a `settle_state` set.
pub(crate) fn check_restore_settling(
    mut commands: Commands,
    time: Res<Time>,
    mut windows: Query<
        (
            Entity,
            &mut TargetPosition,
            &Window,
            Option<&CurrentMonitor>,
            &WindowRestoreAttempt,
            &PreparedPositionMeaning,
        ),
        With<X11FrameCompensated>,
    >,
    platform: Res<Platform>,
    mut driver_state: ResMut<WindowRoleDriverState>,
    mut fallback: ResMut<WindowFallbackRecoveryState>,
) {
    for (entity, mut target_position, window, current_monitor, restore_attempt, position_meaning) in
        &mut windows
    {
        let settle_target =
            SettleTarget::from_target_position(&target_position, *position_meaning, *platform);
        let role = restore_attempt.role().clone();
        let (current_observation, actual_scale) =
            observe_actual_state(window, current_monitor, *platform);

        let WindowSettleProgress::Settling(settle) = &mut target_position.window_settle_progress
        else {
            continue;
        };
        let deadline_status = settle.total_timeout.advance(time.delta());
        let stability_status = settle.stability_timer.advance(time.delta());
        settle.total_elapsed += time.delta();

        let total_elapsed_ms = settle.total_elapsed.as_secs_f32() * MILLIS_PER_SECOND;
        let stability_elapsed_ms = settle.stability_timer.elapsed_secs() * MILLIS_PER_SECOND;
        let change_handling = detect_settle_change(
            settle,
            current_observation,
            &role,
            total_elapsed_ms,
            deadline_status,
        );
        let stable = match change_handling {
            ChangeHandling::Skip => continue,
            ChangeHandling::ChangedAtDeadline => false,
            ChangeHandling::Unchanged => {
                matches!(stability_status, WindowStabilityIntervalStatus::Stable)
            },
        };
        let comparison = check_settle_matches(
            &target_position,
            settle_target.physical_position,
            settle_target.physical_size,
            settle_target.window_mode,
            settle_target.monitor,
            &current_observation,
            *platform,
        );
        debug!(
            "[check_restore_settling] [{role}] {total_elapsed_ms:.0}ms (stable: {stability_elapsed_ms:.0}ms): \
             position={} size={} mode={} monitor={} | \
             size: {} vs {}, \
             mode: {:?} vs {:?}, \
             monitor: {} vs {}, \
             scale: {} vs {actual_scale}",
            comparison.position.is_match(),
            comparison.size.is_match(),
            comparison.mode.is_match(),
            comparison.monitor.is_match(),
            settle_target.physical_size,
            current_observation.physical_size,
            settle_target.window_mode,
            current_observation.window_mode,
            settle_target.monitor,
            current_observation.monitor,
            settle_target.scale,
        );

        if stable && comparison.all_match() {
            emit_settle_success(
                &mut commands,
                entity,
                restore_attempt,
                &settle_target,
                total_elapsed_ms,
                stability_elapsed_ms,
                &mut driver_state,
                &mut fallback,
            );
        } else if stable || matches!(deadline_status, OperatingSystemWorkDeadlineStatus::Expired) {
            let placement_evidence =
                current_monitor.map_or(SettlePlacementEvidence::NotProduced, |current_monitor| {
                    SettlePlacementEvidence::ReadBack(established_placement_from_readback(
                        window,
                        current_monitor,
                        &current_observation,
                        *platform,
                    ))
                });
            emit_settle_mismatch(
                &mut commands,
                entity,
                restore_attempt,
                &settle_target,
                &build_settle_actual(window, current_observation, actual_scale),
                placement_evidence,
                total_elapsed_ms,
                &mut driver_state,
                &mut fallback,
            );
        }
    }
}

fn established_placement_from_readback(
    window: &Window,
    current_monitor: &CurrentMonitor,
    current_observation: &SettleObservation,
    platform: Platform,
) -> EstablishedWindowPlacement {
    let physical_position = match current_observation.physical_position {
        ObservedPhysicalPosition::Observed(position) => Some(position),
        ObservedPhysicalPosition::PlatformCannotReport => None,
    };
    crate::persistence::EstablishedWindowPlacement::from_readback(
        window,
        current_monitor,
        physical_position,
        platform,
    )
}

fn build_settle_actual(
    window: &Window,
    settle_observation: SettleObservation,
    scale: f64,
) -> SettleActual {
    SettleActual {
        settle_observation,
        scale,
        logical_size: UVec2::new(
            window.resolution.width().to_u32(),
            window.resolution.height().to_u32(),
        ),
    }
}

/// Emit `WindowRestored` and clean up `TargetPosition` when settle succeeds.
fn emit_settle_success(
    commands: &mut Commands,
    entity: Entity,
    restore_attempt: &WindowRestoreAttempt,
    settle_target: &SettleTarget,
    total_elapsed_ms: f32,
    stability_elapsed_ms: f32,
    driver_state: &mut WindowRoleDriverState,
    fallback: &mut WindowFallbackRecoveryState,
) {
    let role = restore_attempt.role().clone();
    debug!(
        "[check_restore_settling] [{role}] Settled after {total_elapsed_ms:.0}ms \
         (stable for {stability_elapsed_ms:.0}ms)"
    );
    let restored = WindowRestored {
        entity,
        role: role.clone(),
        physical_position: expected_physical_position(settle_target),
        logical_position: expected_logical_position(settle_target),
        logical_size: settle_target.logical_size,
        physical_size: settle_target.physical_size,
        window_mode: settle_target.window_mode,
        monitor_index: settle_target.monitor.adapter_value(),
    };
    commands.trigger(restored);
    driver_state.finish_as_dispatched(restore_attempt.attempt());
    fallback.finish(&role);
    commands
        .entity(entity)
        .remove::<TargetPosition>()
        .remove::<WindowRestoreAttempt>()
        .remove::<PreparedPositionMeaning>()
        .remove::<X11FrameCompensated>();
}

/// Emit `WindowRestoreMismatch` and clean up `TargetPosition` when the window settled away from
/// the requested placement.
fn emit_settle_mismatch(
    commands: &mut Commands,
    entity: Entity,
    restore_attempt: &WindowRestoreAttempt,
    settle_target: &SettleTarget,
    settle_actual: &SettleActual,
    placement_evidence: SettlePlacementEvidence,
    total_elapsed_ms: f32,
    driver_state: &mut WindowRoleDriverState,
    fallback: &mut WindowFallbackRecoveryState,
) {
    let role = restore_attempt.role().clone();
    warn!(
        "[check_restore_settling] [{role}] Settle timeout after {total_elapsed_ms:.0}ms — \
        mismatch remains: \
         position: {:?} vs {:?}, \
         size: {} vs {}, \
         mode: {:?} vs {:?}, \
         monitor: {} vs {}, \
         scale: {} vs {}",
        settle_target.physical_position,
        settle_actual.settle_observation.physical_position,
        settle_target.physical_size,
        settle_actual.settle_observation.physical_size,
        settle_target.window_mode,
        settle_actual.settle_observation.window_mode,
        settle_target.monitor,
        settle_actual.settle_observation.monitor,
        settle_target.scale,
        settle_actual.scale,
    );
    let mismatch = WindowRestoreMismatch {
        entity,
        role: role.clone(),
        expected_physical_position: expected_physical_position(settle_target),
        actual_physical_position: settle_actual.settle_observation.physical_position,
        expected_logical_position: expected_logical_position(settle_target),
        actual_logical_position: observed_logical_position(
            settle_actual.settle_observation.physical_position,
            settle_actual.scale,
        ),
        expected_physical_size: settle_target.physical_size,
        actual_physical_size: settle_actual.settle_observation.physical_size,
        expected_logical_size: settle_target.logical_size,
        actual_logical_size: settle_actual.logical_size,
        expected_window_mode: settle_target.window_mode,
        actual_window_mode: settle_actual.settle_observation.window_mode,
        expected_monitor: settle_target.monitor.adapter_value(),
        actual_monitor: settle_actual.settle_observation.monitor.adapter_value(),
        expected_scale: settle_target.scale,
        actual_scale: settle_actual.scale,
    };
    commands.trigger(mismatch);
    match placement_evidence {
        SettlePlacementEvidence::ReadBack(placement) => {
            driver_state.finish_with_readback(restore_attempt.attempt(), placement);
        },
        SettlePlacementEvidence::NotProduced => {
            // The record comes back so nothing it left behind is dropped in silence; the markers
            // it put on this window come off a few lines below, which is the whole of that.
            let _ = driver_state.abort_attempt(restore_attempt.attempt());
        },
    }
    fallback.finish(&role);
    commands
        .entity(entity)
        .remove::<TargetPosition>()
        .remove::<WindowRestoreAttempt>()
        .remove::<PreparedPositionMeaning>()
        .remove::<X11FrameCompensated>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(position_meaning: PreparedPositionMeaning) -> SettleTarget {
        let (physical_position, logical_position) = match position_meaning {
            PreparedPositionMeaning::Specified => (
                ExpectedPhysicalPosition::Specified(IVec2::new(20, 40)),
                ExpectedLogicalPosition::Specified(IVec2::new(10, 20)),
            ),
            PreparedPositionMeaning::PlatformCannotPosition => (
                ExpectedPhysicalPosition::PlatformCannotPosition,
                ExpectedLogicalPosition::PlatformCannotPosition,
            ),
            PreparedPositionMeaning::NotSaved => (
                ExpectedPhysicalPosition::NotSaved,
                ExpectedLogicalPosition::NotSaved,
            ),
            PreparedPositionMeaning::DiscardedLegacy => (
                ExpectedPhysicalPosition::DiscardedLegacy,
                ExpectedLogicalPosition::DiscardedLegacy,
            ),
        };
        SettleTarget {
            physical_position,
            logical_position,
            logical_size: UVec2::new(800, 600),
            physical_size: UVec2::new(1_600, 1_200),
            window_mode: WindowMode::Windowed,
            monitor: CurrentMonitorIndex::from_current_enumeration(2),
            scale: 2.0,
        }
    }

    #[test]
    fn expected_event_positions_preserve_every_absence_reason() {
        let specified = target(PreparedPositionMeaning::Specified);
        assert_eq!(
            expected_physical_position(&specified),
            ExpectedPhysicalPosition::Specified(IVec2::new(20, 40))
        );
        assert_eq!(
            expected_logical_position(&specified),
            ExpectedLogicalPosition::Specified(IVec2::new(10, 20))
        );

        let unavailable = target(PreparedPositionMeaning::PlatformCannotPosition);
        assert_eq!(
            expected_physical_position(&unavailable),
            ExpectedPhysicalPosition::PlatformCannotPosition
        );
        assert_eq!(
            expected_logical_position(&unavailable),
            ExpectedLogicalPosition::PlatformCannotPosition
        );

        let not_saved = target(PreparedPositionMeaning::NotSaved);
        assert_eq!(
            expected_physical_position(&not_saved),
            ExpectedPhysicalPosition::NotSaved
        );
        assert_eq!(
            expected_logical_position(&not_saved),
            ExpectedLogicalPosition::NotSaved
        );

        let discarded = target(PreparedPositionMeaning::DiscardedLegacy);
        assert_eq!(
            expected_physical_position(&discarded),
            ExpectedPhysicalPosition::DiscardedLegacy
        );
        assert_eq!(
            expected_logical_position(&discarded),
            ExpectedLogicalPosition::DiscardedLegacy
        );
    }

    #[test]
    fn observed_event_positions_distinguish_platform_absence_from_coordinates() {
        assert_eq!(
            observed_physical_position(None),
            ObservedPhysicalPosition::PlatformCannotReport
        );
        assert_eq!(
            observed_logical_position(ObservedPhysicalPosition::PlatformCannotReport, 2.0),
            ObservedLogicalPosition::PlatformCannotReport
        );
        assert_eq!(
            observed_physical_position(Some(IVec2::new(20, 40))),
            ObservedPhysicalPosition::Observed(IVec2::new(20, 40))
        );
        assert_eq!(
            observed_logical_position(ObservedPhysicalPosition::Observed(IVec2::new(20, 40)), 2.0,),
            ObservedLogicalPosition::Observed(IVec2::new(10, 20))
        );
    }
}
