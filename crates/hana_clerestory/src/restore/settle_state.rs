//! Settle checking logic.
//!
//! After a window restore is applied, monitors the actual window state each frame
//! to confirm the compositor delivered matching values (or detect mismatches).

use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::IVec2;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use bevy::prelude::WindowPosition;
use bevy::prelude::With;
use bevy::prelude::debug;
use bevy::prelude::warn;
use bevy::window::WindowMode;
use hana_kana::ToI32;
use hana_kana::ToU32;
use hana_rigging::prelude::AttemptOutcome;
use hana_rigging::prelude::RoleKey;

use super::RestorePreparation;
use super::RestorePreparationSource;
use super::WindowApplyConfiguration;
use super::target_position::PreparedPositionMeaning;
use super::target_position::TargetPosition;
use super::winit_info::X11FrameCompensated;
use crate::Platform;
use crate::constants::MILLIS_PER_SECOND;
use crate::constants::PRIMARY_MONITOR_INDEX;
use crate::constants::SETTLE_STABILITY_SECS;
use crate::constants::SETTLE_TIMEOUT_SECS;
use crate::driver::WindowDriverAttemptResults;
use crate::events::ExpectedLogicalPosition;
use crate::events::ExpectedPhysicalPosition;
use crate::events::ObservedLogicalPosition;
use crate::events::ObservedPhysicalPosition;
use crate::events::WindowRestoreMismatch;
use crate::events::WindowRestored;
use crate::monitors::CurrentMonitor;
use crate::monitors::CurrentMonitorIndex;
use crate::recovery::WindowFallbackRecoveryState;

/// Window-state observation used to detect changes between frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
struct SettleObservation {
    physical_position: Option<IVec2>,
    physical_size:     UVec2,
    window_mode:       WindowMode,
    monitor:           CurrentMonitorIndex,
}

/// Tracks the two-timer settling state after restore completes.
#[derive(Debug, Clone, Reflect)]
pub(crate) struct SettleState {
    /// Hard deadline timer — fires mismatch if stability is never reached.
    total_timeout:    Timer,
    /// Resets whenever any compared value changes between frames.
    stability_timer:  Timer,
    /// Last frame's compared values, used to detect changes.
    last_observation: Option<SettleObservation>,
}

impl SettleState {
    /// Create a new settle state with default durations.
    #[must_use]
    pub(super) fn new() -> Self {
        Self {
            total_timeout:    Timer::from_seconds(SETTLE_TIMEOUT_SECS, TimerMode::Once),
            stability_timer:  Timer::from_seconds(SETTLE_STABILITY_SECS, TimerMode::Once),
            last_observation: None,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimeoutState {
    Active,
    TimedOut,
}

#[derive(Clone, Copy)]
enum ChangeHandling {
    Skip,
    Continue,
}

/// Bundled actual values for settle mismatch reporting.
struct SettleActual {
    settle_observation: SettleObservation,
    scale:              f64,
    logical_size:       UVec2,
}

/// Extracted target values for settle resolution, avoiding too-many-arguments.
struct SettleTarget {
    physical_position: Option<IVec2>,
    logical_position:  Option<IVec2>,
    logical_size:      UVec2,
    physical_size:     UVec2,
    window_mode:       WindowMode,
    monitor:           CurrentMonitorIndex,
    scale:             f64,
    position_meaning:  PreparedPositionMeaning,
}

impl SettleTarget {
    fn from_target_position(
        target_position: &TargetPosition,
        position_meaning: PreparedPositionMeaning,
        platform: Platform,
    ) -> Self {
        let position_available = platform.position_available();
        Self {
            physical_position: position_available
                .then_some(target_position.physical_position)
                .flatten(),
            logical_position: position_available
                .then_some(target_position.logical_position)
                .flatten(),
            logical_size: target_position.logical_size,
            physical_size: target_position.physical_size,
            window_mode: target_position
                .saved_window_mode
                .to_window_mode(target_position.monitor_index),
            monitor: target_position.monitor_index,
            scale: target_position.target_scale,
            position_meaning,
        }
    }
}

const fn expected_physical_position(target: &SettleTarget) -> ExpectedPhysicalPosition {
    match (target.position_meaning, target.physical_position) {
        (PreparedPositionMeaning::Specified, Some(position)) => {
            ExpectedPhysicalPosition::Specified(position)
        },
        (PreparedPositionMeaning::PlatformCannotPosition, _) => {
            ExpectedPhysicalPosition::PlatformCannotPosition
        },
        (PreparedPositionMeaning::NotSaved, _) | (PreparedPositionMeaning::Specified, None) => {
            ExpectedPhysicalPosition::NotSaved
        },
        (PreparedPositionMeaning::DiscardedLegacy, _) => ExpectedPhysicalPosition::DiscardedLegacy,
    }
}

const fn expected_logical_position(target: &SettleTarget) -> ExpectedLogicalPosition {
    match (target.position_meaning, target.logical_position) {
        (PreparedPositionMeaning::Specified, Some(position)) => {
            ExpectedLogicalPosition::Specified(position)
        },
        (PreparedPositionMeaning::PlatformCannotPosition, _) => {
            ExpectedLogicalPosition::PlatformCannotPosition
        },
        (PreparedPositionMeaning::NotSaved, _) | (PreparedPositionMeaning::Specified, None) => {
            ExpectedLogicalPosition::NotSaved
        },
        (PreparedPositionMeaning::DiscardedLegacy, _) => ExpectedLogicalPosition::DiscardedLegacy,
    }
}

fn observed_physical_position(position: Option<IVec2>) -> ObservedPhysicalPosition {
    position.map_or(
        ObservedPhysicalPosition::PlatformCannotReport,
        ObservedPhysicalPosition::Observed,
    )
}

fn observed_logical_position(
    physical_position: Option<IVec2>,
    scale: f64,
) -> ObservedLogicalPosition {
    physical_position.map_or(ObservedLogicalPosition::PlatformCannotReport, |position| {
        ObservedLogicalPosition::Observed(IVec2::new(
            (f64::from(position.x) / scale).round().to_i32(),
            (f64::from(position.y) / scale).round().to_i32(),
        ))
    })
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
            physical_position,
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
    target_physical_position: Option<IVec2>,
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
    let skip_position = is_fullscreen
        || target_physical_position.is_none()
        || !platform.position_reliable_for_settle();
    let position_matches =
        skip_position || target_physical_position == settle_observation.physical_position;
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
    timeout_state: TimeoutState,
) -> ChangeHandling {
    let changed = settle.last_observation.as_ref() != Some(&settle_observation);
    if changed {
        if settle.last_observation.is_some() {
            debug!(
                "[check_restore_settling] [{role}] {total_elapsed_ms:.0}ms: values changed, \
                 resetting stability timer"
            );
        }
        settle.stability_timer.reset();
        settle.last_observation = Some(settle_observation);
        match timeout_state {
            TimeoutState::TimedOut => ChangeHandling::Continue,
            TimeoutState::Active => ChangeHandling::Skip,
        }
    } else {
        ChangeHandling::Continue
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
/// Runs while `TargetPosition` entities exist (same gate as `restore_windows`).
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
            &RestorePreparation,
            &PreparedPositionMeaning,
        ),
        With<X11FrameCompensated>,
    >,
    platform: Res<Platform>,
    mut results: ResMut<WindowDriverAttemptResults>,
    mut fallback: ResMut<WindowFallbackRecoveryState>,
) {
    for (
        entity,
        mut target_position,
        window,
        current_monitor,
        restore_preparation,
        position_meaning,
    ) in &mut windows
    {
        let settle_target =
            SettleTarget::from_target_position(&target_position, *position_meaning, *platform);
        let role = restore_preparation.role().clone();
        let (current_observation, actual_scale) =
            observe_actual_state(window, current_monitor, *platform);

        let Some(settle) = target_position.settle_state.as_mut() else {
            continue;
        };
        settle.total_timeout.tick(time.delta());
        settle.stability_timer.tick(time.delta());

        let total_elapsed_ms = settle.total_timeout.elapsed_secs() * MILLIS_PER_SECOND;
        let stability_elapsed_ms = settle.stability_timer.elapsed_secs() * MILLIS_PER_SECOND;
        let timeout_state = if settle.total_timeout.is_finished() {
            TimeoutState::TimedOut
        } else {
            TimeoutState::Active
        };

        if matches!(
            detect_settle_change(
                settle,
                current_observation,
                &role,
                total_elapsed_ms,
                timeout_state,
            ),
            ChangeHandling::Skip
        ) {
            continue;
        }
        let stable = settle.stability_timer.is_finished();
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
                restore_preparation,
                &settle_target,
                total_elapsed_ms,
                stability_elapsed_ms,
                &mut results,
                &mut fallback,
            );
        } else if stable || timeout_state == TimeoutState::TimedOut {
            emit_settle_mismatch(
                &mut commands,
                entity,
                restore_preparation,
                &settle_target,
                &build_settle_actual(window, current_observation, actual_scale),
                total_elapsed_ms,
                &mut results,
                &mut fallback,
            );
        }
    }
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
    restore_preparation: &RestorePreparation,
    settle_target: &SettleTarget,
    total_elapsed_ms: f32,
    stability_elapsed_ms: f32,
    results: &mut WindowDriverAttemptResults,
    fallback: &mut WindowFallbackRecoveryState,
) {
    let role = restore_preparation.role().clone();
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
    record_attempt_outcome(results, restore_preparation, AttemptOutcome::Succeeded);
    fallback.finish(&role);
    commands
        .entity(entity)
        .remove::<TargetPosition>()
        .remove::<RestorePreparation>()
        .remove::<WindowApplyConfiguration>()
        .remove::<PreparedPositionMeaning>()
        .remove::<X11FrameCompensated>();
}

/// Emit `WindowRestoreMismatch` and clean up `TargetPosition` when the window settled away from
/// the requested placement.
fn emit_settle_mismatch(
    commands: &mut Commands,
    entity: Entity,
    restore_preparation: &RestorePreparation,
    settle_target: &SettleTarget,
    settle_actual: &SettleActual,
    total_elapsed_ms: f32,
    results: &mut WindowDriverAttemptResults,
    fallback: &mut WindowFallbackRecoveryState,
) {
    let role = restore_preparation.role().clone();
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
        actual_physical_position: observed_physical_position(
            settle_actual.settle_observation.physical_position,
        ),
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
    // The window is stable somewhere the apply did not put it — macOS constrained the frame to
    // fit the display, or the user dragged it mid-restore. Either way that placement is reality
    // and reality wins: `Substituted` settles the role as `Ready` with no captured value, so the
    // kernel's next safe capture reads the window as it actually is and persistence records it.
    // `Failed` would instead park the role and silently disable window persistence for the whole
    // session.
    record_attempt_outcome(results, restore_preparation, AttemptOutcome::Substituted);
    fallback.finish(&role);
    commands
        .entity(entity)
        .remove::<TargetPosition>()
        .remove::<RestorePreparation>()
        .remove::<WindowApplyConfiguration>()
        .remove::<PreparedPositionMeaning>()
        .remove::<X11FrameCompensated>();
}

/// Store one driver outcome only for the kernel attempt that requested this preparation.
fn record_attempt_outcome(
    results: &mut WindowDriverAttemptResults,
    preparation: &RestorePreparation,
    outcome: AttemptOutcome,
) {
    let RestorePreparationSource::KernelAttempt(attempt) = preparation.source();
    results.record(attempt, outcome);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(position_meaning: PreparedPositionMeaning) -> SettleTarget {
        SettleTarget {
            physical_position: Some(IVec2::new(20, 40)),
            logical_position: Some(IVec2::new(10, 20)),
            logical_size: UVec2::new(800, 600),
            physical_size: UVec2::new(1_600, 1_200),
            window_mode: WindowMode::Windowed,
            monitor: CurrentMonitorIndex::from_current_enumeration(2),
            scale: 2.0,
            position_meaning,
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
            observed_logical_position(None, 2.0),
            ObservedLogicalPosition::PlatformCannotReport
        );
        assert_eq!(
            observed_physical_position(Some(IVec2::new(20, 40))),
            ObservedPhysicalPosition::Observed(IVec2::new(20, 40))
        );
        assert_eq!(
            observed_logical_position(Some(IVec2::new(20, 40)), 2.0),
            ObservedLogicalPosition::Observed(IVec2::new(10, 20))
        );
    }
}
