use std::collections::HashMap;
use std::time::Duration;

use bevy::ecs::system::NonSendMarker;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::IVec2;
use bevy::prelude::MessageReader;
#[cfg(target_os = "macos")]
use bevy::prelude::NonSendMut;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Time;
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::debug;
use bevy::prelude::warn;
use bevy::window::WindowMode;
use bevy::window::WindowPosition;
use bevy::window::WindowScaleFactorChanged;
use hana_kana::ToI32;
use hana_kana::ToU32;
#[cfg(test)]
use hana_rigging::prelude::AttemptRef;

use super::strategy::FullscreenRestoreState;
use super::strategy::MonitorScaleStrategy;
use super::strategy::NativeFullscreenState;
use super::strategy::WindowRestoreState;
use super::target::FullscreenMoveDeadline;
use super::target::FullscreenRestoreProgress;
#[cfg(test)]
use super::target::SavedWindowPlacementDecision;
use super::target::TargetPosition;
use super::target::WindowSettleProgress;
use crate::Platform;
use crate::WindowRevealDisposition;
use crate::constants::FULLSCREEN_MONITOR_MOVE_TIMEOUT_SECS;
use crate::constants::MILLIS_PER_SECOND;
use crate::constants::RESTORE_STRATEGY_APPLY_UNCHANGED;
use crate::constants::RESTORE_STRATEGY_LOWER_TO_HIGHER;
use crate::constants::SCALE_CHANGE_WAIT_TIMEOUT_SECS;
use crate::constants::SCALE_FACTOR_EPSILON;
use crate::constants::SETTLE_STABILITY_SECS;
use crate::constants::SETTLE_TIMEOUT_SECS;
use crate::deadline::OperatingSystemWorkDeadline;
use crate::deadline::OperatingSystemWorkDeadlineStatus;
use crate::events::ExpectedPhysicalPosition;
#[cfg(target_os = "macos")]
use crate::macos_tabbing_fix;
#[cfg(target_os = "macos")]
use crate::macos_tabbing_fix::NativeFullscreenObservations;
#[cfg(target_os = "macos")]
use crate::macos_tabbing_fix::NativeFullscreenObserved;
use crate::monitors::CurrentMonitor;
use crate::monitors::CurrentMonitorIndex;
use crate::persistence::SavedWindowMode;
use crate::recovery::WindowFallbackRecoveryState;
use crate::restore::RestorePreparationSource;
use crate::restore::WindowRestoreAttempt;
use crate::restore::settle_state::SettleState;
use crate::restore::winit_info;
#[cfg(test)]
use crate::restore::winit_info::InjectedWinitWindows;
use crate::restore::winit_info::X11FrameCompensated;
use crate::visibility::PlacementAbandoned;

enum SavedGeometryPlacementProgress {
    Complete,
    Waiting,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct ObservedScaleInput {
    provenance: RestorePreparationSource,
    scale:      f64,
}

#[derive(Default, Resource)]
pub(crate) struct ObservedScaleInputs {
    entries: HashMap<Entity, Vec<ObservedScaleInput>>,
}

pub(crate) fn capture_scale_inputs(
    mut messages: MessageReader<WindowScaleFactorChanged>,
    preparations: Query<&WindowRestoreAttempt>,
    mut inputs: ResMut<ObservedScaleInputs>,
) {
    inputs.entries.clear();
    for message in messages.read() {
        let Ok(preparation) = preparations.get(message.window) else {
            continue;
        };
        inputs
            .entries
            .entry(message.window)
            .or_default()
            .push(ObservedScaleInput {
                provenance: preparation.source(),
                scale:      message.scale_factor,
            });
    }
}

fn matching_scale_change(
    entity: Entity,
    current_source: RestorePreparationSource,
    transition_source: RestorePreparationSource,
    target_scale: f64,
    live_scale: f64,
    scale_inputs: &ObservedScaleInputs,
) -> bool {
    current_source == transition_source
        && scale_inputs.entries.get(&entity).is_some_and(|inputs| {
            inputs.iter().any(|input| {
                input.provenance == current_source
                    && (input.scale - target_scale).abs() <= SCALE_FACTOR_EPSILON
            }) && (live_scale - target_scale).abs() <= SCALE_FACTOR_EPSILON
        })
}

/// Whether the window has arrived on the target monitor and that monitor is at `target_scale`.
///
/// This is the backstop for a `WindowScaleFactorChanged` that never arrives. The window cannot
/// already be at `target_scale` on entry — `Platform::scale_strategy` returns `ApplyUnchanged`
/// when the starting and target scales are within `SCALE_FACTOR_EPSILON`, so reaching
/// `WaitingForScaleChange` means the two scales differ by more than that. What is not guaranteed
/// is the event: Windows delivers `WM_DPICHANGED` to a hidden window only because `windows_dpi_fix`
/// forwards it, and a target that matches no live monitor produces no crossing at all.
///
/// The monitor index is checked as well as the scale. Several monitors commonly share one scale,
/// so scale alone accepts a window that drifted onto a *different* display — after which settling
/// compares the window against the target monitor's geometry while it sits somewhere else, and
/// reports a mismatch whose cause is invisible.
fn current_monitor_reached_target_scale(
    current_monitor: Option<&CurrentMonitor>,
    target_scale: f64,
    target_monitor_index: CurrentMonitorIndex,
) -> bool {
    current_monitor.is_some_and(|current_monitor| {
        current_monitor.index == target_monitor_index
            && (current_monitor.descriptor.scale - target_scale).abs() <= SCALE_FACTOR_EPSILON
    })
}

fn correct_initial_starting_scale(
    entity: Entity,
    target_position: &mut TargetPosition,
    window: &Window,
    platform: Platform,
) {
    if !platform.needs_managed_scale_fixup()
        || !matches!(
            &target_position.monitor_scale_strategy,
            MonitorScaleStrategy::ApplyUnchanged
                | MonitorScaleStrategy::LowerToHigher
                | MonitorScaleStrategy::HigherToLower(WindowRestoreState::NeedInitialMove)
                | MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::NeedInitialMove)
        )
    {
        return;
    }

    let actual_scale = f64::from(window.resolution.base_scale_factor());
    if (actual_scale - target_position.starting_scale).abs() <= SCALE_FACTOR_EPSILON {
        return;
    }

    let old_monitor_scale_strategy = target_position.monitor_scale_strategy.clone();
    target_position.starting_scale = actual_scale;
    target_position.monitor_scale_strategy =
        platform.scale_strategy(actual_scale, target_position.target_scale);
    debug!(
        "[place_window_at_saved_geometry] Corrected starting_scale for entity {entity:?}: \
         monitor_scale_strategy: {old_monitor_scale_strategy:?} -> {:?} \
         (actual_scale={actual_scale:.2})",
        target_position.monitor_scale_strategy
    );
}

/// Apply the initial window move to the target monitor.
fn apply_initial_move(target_position: &TargetPosition, window: &mut Window) {
    if target_position.saved_window_mode.is_fullscreen() {
        if let ExpectedPhysicalPosition::Specified(physical_position) =
            target_position.physical_position()
        {
            debug!(
                "[apply_initial_move] Moving to target position {:?} for fullscreen mode {:?}",
                physical_position, target_position.saved_window_mode
            );
            window.position = WindowPosition::At(physical_position);
        } else {
            debug!(
                "[apply_initial_move] No saved position, fullscreen mode {:?} targets monitor {} via WindowMode",
                target_position.saved_window_mode, target_position.monitor_index
            );
        }
        return;
    }

    let ExpectedPhysicalPosition::Specified(physical_position) =
        target_position.physical_position()
    else {
        debug!(
            "[apply_initial_move] No saved position, centering on monitor {}",
            target_position.monitor_index
        );
        window.position = WindowPosition::Centered(target_position.monitor_index.selection());
        return;
    };

    // HigherToLower (macOS/X11 high→low) compensates position by ×ratio
    // (= starting_scale / target_scale, e.g. ×2 for 2x→1x): `set_outer_position` is
    // applied at the starting monitor's scale, so crossing to the half-scale target
    // halves the physical position unless pre-multiplied. Size stays a placeholder —
    // the WaitingForScaleChange → ApplySize phase re-applies the full physical size
    // after the scale change settles.
    let (physical_move_position, physical_move_size) = match target_position.monitor_scale_strategy
    {
        MonitorScaleStrategy::HigherToLower(_) => {
            let ratio = target_position.ratio();
            let physical_compensated_x = (f64::from(physical_position.x) * ratio).to_i32();
            let physical_compensated_y = (f64::from(physical_position.y) * ratio).to_i32();
            debug!(
                "[apply_initial_move] HigherToLower: compensating position {physical_position:?} -> ({physical_compensated_x}, {physical_compensated_y}) (ratio={ratio})",
            );
            (
                IVec2::new(physical_compensated_x, physical_compensated_y),
                target_position.physical_size,
            )
        },
        MonitorScaleStrategy::CompensateSizeOnly(_) => {
            let physical_compensated_size = target_position.compensated_size();
            debug!(
                "[apply_initial_move] CompensateSizeOnly: position={:?} compensated_size={}x{} (ratio={})",
                physical_position,
                physical_compensated_size.x,
                physical_compensated_size.y,
                target_position.ratio()
            );
            (physical_position, physical_compensated_size)
        },
        _ => (physical_position, target_position.physical_size),
    };

    debug!(
        "[apply_initial_move] position={physical_move_position:?} size={}x{} visible={}",
        physical_move_size.x, physical_move_size.y, window.visible
    );

    window.position = WindowPosition::At(physical_move_position);
    window
        .resolution
        .set_physical_resolution(physical_move_size.x, physical_move_size.y);
}

/// Handle the initial move for cross-DPI strategies.
///
/// With a saved position, a compensated position and size are applied on the starting monitor
/// and the strategy moves to `WaitingForScaleChange`, so winit's `WindowScaleFactorChanged`
/// triggers the final `ApplySize` phase at `target_scale`.
///
/// With no saved position, the window is centered on the saved monitor with
/// `WindowPosition::Centered` and sized at the window's live scale factor.
/// `set_physical_resolution` is interpreted at that scale, and both macOS and Windows carry
/// the resulting logical size through the move, so the post-move physical size resolves to
/// `TargetPosition::logical_size * target_scale` — which is `TargetPosition::physical_size`,
/// the value settle compares against. Reading the live scale rather than
/// `TargetPosition::starting_scale` also covers Windows, where `windows_dpi_fix` forwards
/// `WM_DPICHANGED` to the still-hidden window and the crossing can already have happened.
/// The `WindowScaleFactorChanged` -> `WindowRestoreState::ApplySize` transition is
/// skipped because macOS does not fire `WindowScaleFactorChanged` for windows that are
/// still hidden; waiting for it would deadlock. Settle starts immediately and verifies
/// the resulting state.
fn begin_cross_dpi_restore(
    target_position: &mut TargetPosition,
    window: &mut Window,
    source: RestorePreparationSource,
) -> SavedGeometryPlacementProgress {
    if !matches!(
        target_position.physical_position(),
        ExpectedPhysicalPosition::Specified(_)
    ) {
        let live_scale = f64::from(window.resolution.scale_factor());
        let physical_width = (f64::from(target_position.logical_size.x) * live_scale).to_u32();
        let physical_height = (f64::from(target_position.logical_size.y) * live_scale).to_u32();
        debug!(
            "[begin_cross_dpi_restore] no saved position, centering on monitor {} at \
             live_scale={live_scale} (physical {physical_width}x{physical_height} → logical \
             {}x{} after the move to target_scale={})",
            target_position.monitor_index,
            target_position.logical_size.x,
            target_position.logical_size.y,
            target_position.target_scale
        );
        window.position = WindowPosition::Centered(target_position.monitor_index.selection());
        window
            .resolution
            .set_physical_resolution(physical_width, physical_height);
        target_position.window_settle_progress = WindowSettleProgress::Settling(SettleState::new());
        return SavedGeometryPlacementProgress::Complete;
    }

    apply_initial_move(target_position, window);
    target_position.monitor_scale_strategy = match &target_position.monitor_scale_strategy {
        MonitorScaleStrategy::HigherToLower(_) => {
            MonitorScaleStrategy::HigherToLower(WindowRestoreState::WaitingForScaleChange {
                source,
                deadline: OperatingSystemWorkDeadline::new(SCALE_CHANGE_WAIT_TIMEOUT_SECS),
            })
        },
        _ => MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::WaitingForScaleChange {
            source,
            deadline: OperatingSystemWorkDeadline::new(SCALE_CHANGE_WAIT_TIMEOUT_SECS),
        }),
    };
    SavedGeometryPlacementProgress::Waiting
}

fn advance_fullscreen_restore(
    #[cfg(target_os = "macos")] entity: Entity,
    #[cfg(not(target_os = "macos"))] _entity: Entity,
    target_position: &mut TargetPosition,
    window: &mut Window,
    current_monitor: Option<&CurrentMonitor>,
    native_fullscreen: NativeFullscreenState,
    delta: Duration,
) -> SavedGeometryPlacementProgress {
    let progress = std::mem::replace(
        &mut target_position.fullscreen_restore_progress,
        FullscreenRestoreProgress::ReadyForGeometry,
    );
    let FullscreenRestoreProgress::Advancing(fullscreen_restore_state) = progress else {
        return SavedGeometryPlacementProgress::Complete;
    };
    let (placement_progress, next_fullscreen_progress) = match fullscreen_restore_state {
        FullscreenRestoreState::LeaveFullscreen => {
            debug!(
                "[place_window_at_saved_geometry] macOS fullscreen: leaving the current fullscreen Space"
            );
            window.mode = WindowMode::Windowed;
            (
                SavedGeometryPlacementProgress::Waiting,
                FullscreenRestoreProgress::Advancing(
                    FullscreenRestoreState::MoveWindowedToTarget {
                        deadline: FullscreenMoveDeadline::NotRequested,
                    },
                ),
            )
        },
        FullscreenRestoreState::MoveWindowedToTarget { mut deadline } => {
            let progress = advance_move_windowed_to_target(
                target_position.monitor_index,
                target_position.physical_position(),
                window,
                current_monitor,
                native_fullscreen,
                &mut deadline,
                delta,
            );
            let next_state = if matches!(progress, SavedGeometryPlacementProgress::Complete) {
                FullscreenRestoreState::ApplyMode
            } else {
                FullscreenRestoreState::MoveWindowedToTarget { deadline }
            };
            (
                SavedGeometryPlacementProgress::Waiting,
                FullscreenRestoreProgress::Advancing(next_state),
            )
        },
        FullscreenRestoreState::MoveToMonitor => {
            if let ExpectedPhysicalPosition::Specified(position) =
                target_position.physical_position()
            {
                debug!(
                    "[place_window_at_saved_geometry] Fullscreen MoveToMonitor: position={position:?}"
                );
                window.position = WindowPosition::At(position);
            }
            (
                SavedGeometryPlacementProgress::Waiting,
                FullscreenRestoreProgress::Advancing(FullscreenRestoreState::WaitForMove),
            )
        },
        FullscreenRestoreState::WaitForMove => {
            debug!(
                "[place_window_at_saved_geometry] Fullscreen WaitForMove: waiting for compositor"
            );
            (
                SavedGeometryPlacementProgress::Waiting,
                FullscreenRestoreProgress::Advancing(FullscreenRestoreState::ApplyMode),
            )
        },
        FullscreenRestoreState::WaitForSurface => {
            debug!(
                "[place_window_at_saved_geometry] Fullscreen WaitForSurface: waiting for GPU surface"
            );
            (
                SavedGeometryPlacementProgress::Waiting,
                FullscreenRestoreProgress::Advancing(FullscreenRestoreState::ApplyMode),
            )
        },
        FullscreenRestoreState::ApplyMode => (
            SavedGeometryPlacementProgress::Complete,
            FullscreenRestoreProgress::Completed,
        ),
        FullscreenRestoreState::ActivateWindow => {
            #[cfg(target_os = "macos")]
            macos_tabbing_fix::activate_fullscreen_window(entity);
            debug!(
                "[place_window_at_saved_geometry] macOS fullscreen: activated window after mode request"
            );
            (
                SavedGeometryPlacementProgress::Waiting,
                FullscreenRestoreProgress::Advancing(FullscreenRestoreState::WaitForTarget),
            )
        },
        FullscreenRestoreState::WaitForTarget => advance_fullscreen_target_wait(
            target_position.monitor_index,
            current_monitor,
            native_fullscreen,
        ),
    };
    target_position.fullscreen_restore_progress = next_fullscreen_progress;
    placement_progress
}

fn advance_fullscreen_target_wait(
    monitor_index: CurrentMonitorIndex,
    current_monitor: Option<&CurrentMonitor>,
    native_fullscreen: NativeFullscreenState,
) -> (SavedGeometryPlacementProgress, FullscreenRestoreProgress) {
    let target_monitor_reached = current_monitor
        .is_some_and(|current_monitor| current_monitor.descriptor.index == monitor_index);
    if native_fullscreen != NativeFullscreenState::Fullscreen || !target_monitor_reached {
        debug!(
            "[place_window_at_saved_geometry] macOS fullscreen: waiting for fullscreen on target monitor {monitor_index}"
        );
        return (
            SavedGeometryPlacementProgress::Waiting,
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::WaitForTarget),
        );
    }
    debug!(
        "[place_window_at_saved_geometry] macOS fullscreen: AppKit reported fullscreen on target monitor {monitor_index}"
    );
    (
        SavedGeometryPlacementProgress::Complete,
        FullscreenRestoreProgress::ReadyForGeometry,
    )
}

/// Drive `FullscreenRestoreState::MoveWindowedToTarget`: request the windowed window onto
/// `TargetPosition::monitor_index` and finish once it arrives or its deadline expires.
fn advance_move_windowed_to_target(
    monitor_index: CurrentMonitorIndex,
    physical_position: ExpectedPhysicalPosition,
    window: &mut Window,
    current_monitor: Option<&CurrentMonitor>,
    native_fullscreen: NativeFullscreenState,
    move_deadline: &mut FullscreenMoveDeadline,
    delta: Duration,
) -> SavedGeometryPlacementProgress {
    if native_fullscreen != NativeFullscreenState::Windowed {
        debug!(
            "[place_window_at_saved_geometry] macOS fullscreen: waiting for AppKit to finish leaving fullscreen"
        );
        return SavedGeometryPlacementProgress::Waiting;
    }
    let target_monitor_reached = current_monitor
        .is_some_and(|current_monitor| current_monitor.descriptor.index == monitor_index);
    if target_monitor_reached {
        debug!(
            "[place_window_at_saved_geometry] macOS fullscreen: windowed window reached target monitor {}",
            monitor_index
        );
        return SavedGeometryPlacementProgress::Complete;
    }
    // `WindowPosition::At` over `Centered`: bevy's `changed_windows` calls `set_outer_position`
    // only when `Window::position` differs from its cached value, and a window launched with the
    // same `Centered(Index(n))` this phase would re-request never moves at all.
    let move_position = match physical_position {
        ExpectedPhysicalPosition::Specified(position) => WindowPosition::At(position),
        ExpectedPhysicalPosition::PlatformCannotPosition
        | ExpectedPhysicalPosition::NotSaved
        | ExpectedPhysicalPosition::DiscardedLegacy => {
            WindowPosition::Centered(monitor_index.selection())
        },
    };
    debug!(
        "[place_window_at_saved_geometry] macOS fullscreen: moving windowed window to {move_position:?} for target monitor {}",
        monitor_index
    );
    window.position = move_position;
    let FullscreenMoveDeadline::Awaiting(deadline) = move_deadline else {
        *move_deadline = FullscreenMoveDeadline::Awaiting(OperatingSystemWorkDeadline::new(
            FULLSCREEN_MONITOR_MOVE_TIMEOUT_SECS,
        ));
        return SavedGeometryPlacementProgress::Waiting;
    };
    if matches!(
        deadline.advance(delta),
        OperatingSystemWorkDeadlineStatus::Expired
    ) {
        warn!(
            "The windowed window did not reach monitor {} within \
             {FULLSCREEN_MONITOR_MOVE_TIMEOUT_SECS}s (it is on monitor {:?}). Applying the saved \
             fullscreen mode from where it is, so the window becomes visible rather than staying \
             hidden while the move is re-requested.",
            monitor_index,
            current_monitor.map(|current_monitor| current_monitor.descriptor.index),
        );
        return SavedGeometryPlacementProgress::Complete;
    }
    SavedGeometryPlacementProgress::Waiting
}

/// Apply pending window restore. Runs only when entities with `TargetPosition` exist.
pub(crate) fn place_window_at_saved_geometry(
    mut windows: Query<
        (
            Entity,
            &WindowRestoreAttempt,
            &mut TargetPosition,
            &mut Window,
            Option<&CurrentMonitor>,
            Has<WindowRevealDisposition>,
        ),
        With<X11FrameCompensated>,
    >,
    mut commands: Commands,
    _: NonSendMarker,
    #[cfg(target_os = "macos")] mut fullscreen_observations: NonSendMut<
        NativeFullscreenObservations,
    >,
    platform: Res<Platform>,
    scale_inputs: Res<ObservedScaleInputs>,
    time: Option<Res<Time>>,
    mut fallback: ResMut<WindowFallbackRecoveryState>,
    #[cfg(test)] injected_windows: Option<Res<InjectedWinitWindows>>,
) {
    let delta = time.as_deref().map_or(Duration::ZERO, Time::delta);
    for (
        entity,
        restore_preparation,
        mut target_position,
        mut window,
        current_monitor,
        has_reveal_disposition,
    ) in &mut windows
    {
        let native_window_exists = winit_info::native_window_exists(
            entity,
            #[cfg(test)]
            injected_windows.as_deref(),
        );
        #[cfg(target_os = "macos")]
        let native_fullscreen = if native_window_exists
            && *platform == Platform::MacOs
            && target_position.saved_window_mode.is_fullscreen()
        {
            match fullscreen_observations.observe(entity) {
                NativeFullscreenObserved::Fullscreen => NativeFullscreenState::Fullscreen,
                NativeFullscreenObserved::Windowed => NativeFullscreenState::Windowed,
                NativeFullscreenObserved::Unavailable => NativeFullscreenState::Unavailable,
            }
        } else {
            NativeFullscreenState::Unavailable
        };
        #[cfg(not(target_os = "macos"))]
        let native_fullscreen = NativeFullscreenState::Unavailable;
        let restore_status = restore_window(
            entity,
            restore_preparation,
            &mut target_position,
            &mut window,
            &scale_inputs,
            *platform,
            native_window_exists,
            current_monitor,
            native_fullscreen,
            delta,
        );
        if matches!(restore_status, SavedGeometryPlacementProgress::Complete)
            && !has_reveal_disposition
        {
            commands
                .entity(entity)
                .insert(WindowRevealDisposition::SavedGeometryApplied)
                .try_remove::<PlacementAbandoned>();
        }
        if matches!(
            &target_position.window_settle_progress,
            WindowSettleProgress::Settling(_)
        ) {
            fallback.mark_settling(restore_preparation.role().clone());
        }
        #[cfg(target_os = "macos")]
        if matches!(
            &target_position.window_settle_progress,
            WindowSettleProgress::Settling(_)
        ) {
            fullscreen_observations.stop(entity);
        }
    }
}

/// Advance a cross-DPI restore out of `WaitingForScaleChange`.
///
/// Three signals can end the wait, in decreasing order of authority:
///
/// 1. winit's `WindowScaleFactorChanged` ([`matching_scale_change`]) confirms the DPI crossing. On
///    Windows the hidden restore window receives it because `windows_dpi_fix` forwards
///    `WM_DPICHANGED` while the window is hidden.
/// 2. `CurrentMonitor` reports the window on the target monitor at `target_scale`
///    ([`current_monitor_reached_target_scale`]). Arrival is observable even when the event is not
///    delivered, so this covers a lost `WM_DPICHANGED`.
/// 3. [`SCALE_CHANGE_WAIT_TIMEOUT_SECS`] elapses. A target that matches no live monitor never
///    arrives at all; applying the final size anyway reveals the window and lets settle report the
///    mismatch, rather than waiting forever with the window hidden.
fn advance_scale_change_wait(
    entity: Entity,
    restore_preparation: &WindowRestoreAttempt,
    target_position: &mut TargetPosition,
    window: &Window,
    scale_inputs: &ObservedScaleInputs,
    current_monitor: Option<&CurrentMonitor>,
    delta: Duration,
) {
    let (source, wait_expired) = match &mut target_position.monitor_scale_strategy {
        MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::WaitingForScaleChange {
            source,
            deadline,
        })
        | MonitorScaleStrategy::HigherToLower(WindowRestoreState::WaitingForScaleChange {
            source,
            deadline,
        }) => (
            *source,
            matches!(
                deadline.advance(delta),
                OperatingSystemWorkDeadlineStatus::Expired
            ),
        ),
        _ => return,
    };

    let scale_observed = matching_scale_change(
        entity,
        restore_preparation.source(),
        source,
        target_position.target_scale,
        f64::from(window.resolution.base_scale_factor()),
        scale_inputs,
    );
    // Arrival only settles the Windows path: macOS and X11 reposition a `HigherToLower` window in
    // stages, so the target monitor can be reported before the move has finished.
    let arrived_at_target_scale = matches!(
        &target_position.monitor_scale_strategy,
        MonitorScaleStrategy::CompensateSizeOnly(_)
    ) && current_monitor_reached_target_scale(
        current_monitor,
        target_position.target_scale,
        target_position.monitor_index,
    );
    if !(scale_observed || arrived_at_target_scale || wait_expired) {
        return;
    }
    if !(scale_observed || arrived_at_target_scale) {
        warn!(
            "[Restore] entity {entity:?} saw no scale change within {SCALE_CHANGE_WAIT_TIMEOUT_SECS}s \
             (target_scale={}); applying the final size so the window is not left hidden",
            target_position.target_scale
        );
    }

    debug!("[Restore] Leaving WaitingForScaleChange for ApplySize");
    target_position.monitor_scale_strategy = match &target_position.monitor_scale_strategy {
        MonitorScaleStrategy::HigherToLower(_) => {
            MonitorScaleStrategy::HigherToLower(WindowRestoreState::ApplySize)
        },
        _ => MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::ApplySize),
    };
}

fn restore_window(
    entity: Entity,
    restore_preparation: &WindowRestoreAttempt,
    target_position: &mut TargetPosition,
    window: &mut Window,
    scale_inputs: &ObservedScaleInputs,
    platform: Platform,
    native_window_exists: bool,
    current_monitor: Option<&CurrentMonitor>,
    native_fullscreen: NativeFullscreenState,
    delta: Duration,
) -> SavedGeometryPlacementProgress {
    if matches!(
        &target_position.window_settle_progress,
        WindowSettleProgress::Settling(_)
    ) {
        return SavedGeometryPlacementProgress::Waiting;
    }

    if !native_window_exists {
        debug!(
            "[place_window_at_saved_geometry] Skipping entity {entity:?}: winit window not yet created"
        );
        return SavedGeometryPlacementProgress::Waiting;
    }

    correct_initial_starting_scale(entity, target_position, window, platform);

    let macos_fullscreen =
        platform == Platform::MacOs && target_position.saved_window_mode.is_fullscreen();
    if macos_fullscreen
        && matches!(
            advance_fullscreen_restore(
                entity,
                target_position,
                window,
                current_monitor,
                native_fullscreen,
                delta,
            ),
            SavedGeometryPlacementProgress::Waiting
        )
    {
        return SavedGeometryPlacementProgress::Waiting;
    }

    if !macos_fullscreen
        && matches!(
            &target_position.monitor_scale_strategy,
            MonitorScaleStrategy::HigherToLower(WindowRestoreState::NeedInitialMove)
                | MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::NeedInitialMove)
        )
    {
        return begin_cross_dpi_restore(target_position, window, restore_preparation.source());
    }

    advance_scale_change_wait(
        entity,
        restore_preparation,
        target_position,
        window,
        scale_inputs,
        current_monitor,
        delta,
    );

    if !macos_fullscreen
        && matches!(
            advance_fullscreen_restore(
                entity,
                target_position,
                window,
                current_monitor,
                native_fullscreen,
                delta,
            ),
            SavedGeometryPlacementProgress::Waiting
        )
    {
        return SavedGeometryPlacementProgress::Waiting;
    }

    let applying_macos_fullscreen = macos_fullscreen
        && matches!(
            &target_position.fullscreen_restore_progress,
            FullscreenRestoreProgress::Completed
        );
    let restore_status = try_apply_restore(target_position, window, platform);
    if matches!(restore_status, SavedGeometryPlacementProgress::Waiting) {
        return SavedGeometryPlacementProgress::Waiting;
    }
    if applying_macos_fullscreen {
        target_position.fullscreen_restore_progress =
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::ActivateWindow);
        return SavedGeometryPlacementProgress::Complete;
    }

    if matches!(
        &target_position.window_settle_progress,
        WindowSettleProgress::NotStarted
    ) {
        let settle_stability_ms = SETTLE_STABILITY_SECS * MILLIS_PER_SECOND;
        debug!(
            "[place_window_at_saved_geometry] Restore applied, starting settle ({settle_stability_ms:.0}ms stability / {SETTLE_TIMEOUT_SECS:.0}s timeout)"
        );
        target_position.window_settle_progress = WindowSettleProgress::Settling(SettleState::new());
    }
    SavedGeometryPlacementProgress::Complete
}

fn apply_window_geometry(
    window: &mut Window,
    physical_position: ExpectedPhysicalPosition,
    physical_size: UVec2,
    strategy: &str,
    ratio: Option<f64>,
    monitor_index: CurrentMonitorIndex,
) {
    if let ExpectedPhysicalPosition::Specified(physical_position) = physical_position {
        if let Some(ratio) = ratio {
            debug!(
                "[try_apply_restore] position={:?} size={}x{} ({strategy}, ratio={ratio})",
                physical_position, physical_size.x, physical_size.y
            );
        } else {
            debug!(
                "[try_apply_restore] position={:?} size={}x{} ({strategy})",
                physical_position, physical_size.x, physical_size.y
            );
        }
        window.position = WindowPosition::At(physical_position);
    } else {
        if let Some(ratio) = ratio {
            debug!(
                "[try_apply_restore] size={}x{} centered on monitor {monitor_index} ({strategy}, ratio={ratio}, no saved position)",
                physical_size.x, physical_size.y
            );
        } else {
            debug!(
                "[try_apply_restore] size={}x{} centered on monitor {monitor_index} ({strategy}, no saved position)",
                physical_size.x, physical_size.y
            );
        }
        window.position = WindowPosition::Centered(monitor_index.selection());
    }
    window
        .resolution
        .set_physical_resolution(physical_size.x, physical_size.y);
}

fn apply_fullscreen_restore(
    target_position: &TargetPosition,
    window: &mut Window,
    platform: Platform,
) {
    let monitor_index = target_position.monitor_index;

    let window_mode = if platform.exclusive_fullscreen_fallback()
        && matches!(
            target_position.saved_window_mode,
            SavedWindowMode::Fullscreen { .. }
        ) {
        warn!(
            "Exclusive fullscreen is not supported on Wayland, restoring as BorderlessFullscreen"
        );
        WindowMode::BorderlessFullscreen(monitor_index.selection())
    } else {
        target_position
            .saved_window_mode
            .to_window_mode(monitor_index)
    };

    debug!(
        "[Restore] Applying fullscreen mode {:?} on monitor {} -> WindowMode::{:?}",
        target_position.saved_window_mode, monitor_index, window_mode
    );
    debug!(
        "[Restore] Current window state: position={:?} mode={:?}",
        window.position, window.mode
    );

    window.mode = window_mode;
}

fn try_apply_restore(
    target_position: &TargetPosition,
    window: &mut Window,
    platform: Platform,
) -> SavedGeometryPlacementProgress {
    if target_position.saved_window_mode.is_fullscreen() {
        debug!(
            "[try_apply_restore] fullscreen: mode={:?} target_monitor={} current_physical={}x{} current_mode={:?} current_position={:?}",
            target_position.saved_window_mode,
            target_position.monitor_index,
            window.physical_width(),
            window.physical_height(),
            window.mode,
            window.position,
        );
        apply_fullscreen_restore(target_position, window, platform);
        return SavedGeometryPlacementProgress::Complete;
    }

    debug!(
        "[Restore] target_position={:?} target_scale={} monitor_scale_strategy={:?}",
        target_position.physical_position(),
        target_position.target_scale,
        target_position.monitor_scale_strategy
    );

    match &target_position.monitor_scale_strategy {
        MonitorScaleStrategy::ApplyUnchanged => {
            apply_window_geometry(
                window,
                target_position.physical_position(),
                target_position.physical_size,
                RESTORE_STRATEGY_APPLY_UNCHANGED,
                None,
                target_position.monitor_index,
            );
        },
        MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::ApplySize) => {
            debug!(
                "[try_apply_restore] size={}x{} ONLY (CompensateSizeOnly::ApplySize, position already set)",
                target_position.physical_size.x, target_position.physical_size.y
            );
            window.resolution.set_physical_resolution(
                target_position.physical_size.x,
                target_position.physical_size.y,
            );
        },
        MonitorScaleStrategy::CompensateSizeOnly(
            WindowRestoreState::NeedInitialMove | WindowRestoreState::WaitingForScaleChange { .. },
        ) => {
            debug!(
                "[Restore] CompensateSizeOnly: waiting for initial move or ScaleChanged message"
            );
            return SavedGeometryPlacementProgress::Waiting;
        },
        MonitorScaleStrategy::LowerToHigher => {
            // Position still needs ratio compensation: on a low→high cross-scale
            // move, `set_outer_position` is applied at the starting monitor's scale,
            // so the move doubles it. Size must NOT be compensated: as of bevy 0.19,
            // `request_inner_size` resolves at the target monitor's scale, so the
            // `request_inner_size` produces the requested full physical size
            // (compensating it would halve it).
            apply_window_geometry(
                window,
                target_position.compensated_position(),
                target_position.physical_size,
                RESTORE_STRATEGY_LOWER_TO_HIGHER,
                Some(target_position.ratio()),
                target_position.monitor_index,
            );
        },
        MonitorScaleStrategy::HigherToLower(WindowRestoreState::ApplySize) => {
            debug!(
                "[try_apply_restore] size={}x{} ONLY (HigherToLower::ApplySize, position already set)",
                target_position.physical_size.x, target_position.physical_size.y
            );
            window.resolution.set_physical_resolution(
                target_position.physical_size.x,
                target_position.physical_size.y,
            );
        },
        MonitorScaleStrategy::HigherToLower(
            WindowRestoreState::NeedInitialMove | WindowRestoreState::WaitingForScaleChange { .. },
        ) => {
            debug!("[Restore] HigherToLower: waiting for initial move or ScaleChanged message");
            return SavedGeometryPlacementProgress::Waiting;
        },
    }

    SavedGeometryPlacementProgress::Complete
}
#[cfg(test)]
mod scale_change_wait_tests {
    use super::*;
    use crate::MonitorDescriptor;

    const MONITOR_INDEX: usize = 1;
    const STARTING_SCALE: f64 = 1.75;
    const TARGET_SCALE: f64 = 1.5;

    fn waiting_target() -> TargetPosition {
        TargetPosition {
            placement_decision:          SavedWindowPlacementDecision::Restorable {
                physical_position: IVec2::new(1_631, 2_880),
                logical_position:  IVec2::new(1_087, 1_920),
            },
            physical_size:               UVec2::new(1_350, 900),
            logical_size:                UVec2::new(900, 600),
            target_scale:                TARGET_SCALE,
            starting_scale:              STARTING_SCALE,
            monitor_scale_strategy:      MonitorScaleStrategy::CompensateSizeOnly(
                WindowRestoreState::WaitingForScaleChange {
                    source:   RestorePreparationSource::KernelAttempt(AttemptRef::default()),
                    deadline: OperatingSystemWorkDeadline::new(SCALE_CHANGE_WAIT_TIMEOUT_SECS),
                },
            ),
            saved_window_mode:           SavedWindowMode::Windowed,
            monitor_index:               CurrentMonitorIndex::from_current_enumeration(
                MONITOR_INDEX,
            ),
            fullscreen_restore_progress: FullscreenRestoreProgress::ReadyForGeometry,
            window_settle_progress:      WindowSettleProgress::NotStarted,
        }
    }

    fn monitor_at_scale(scale: f64) -> CurrentMonitor {
        CurrentMonitor {
            descriptor:            MonitorDescriptor::for_current_enumeration(
                MONITOR_INDEX,
                scale,
                IVec2::new(1_631, 2_880),
                UVec2::new(3_456, 2_168),
            ),
            effective_window_mode: WindowMode::Windowed,
        }
    }

    fn advance(
        target_position: &mut TargetPosition,
        current_monitor: Option<&CurrentMonitor>,
        delta: Duration,
    ) {
        let Ok(role) = crate::persistence::primary_window_role() else {
            return;
        };
        let preparation = WindowRestoreAttempt::for_test(role);
        advance_scale_change_wait(
            Entity::from_bits(1),
            &preparation,
            target_position,
            &Window::default(),
            &ObservedScaleInputs::default(),
            current_monitor,
            delta,
        );
    }

    fn other_monitor_at_scale(scale: f64) -> CurrentMonitor {
        CurrentMonitor {
            descriptor:            MonitorDescriptor::for_current_enumeration(
                MONITOR_INDEX + 1,
                scale,
                IVec2::new(-3_456, 0),
                UVec2::new(3_456, 2_168),
            ),
            effective_window_mode: WindowMode::Windowed,
        }
    }

    /// Arriving on the target monitor ends the wait even when no `WindowScaleFactorChanged` is
    /// delivered — which happens on Windows whenever `WM_DPICHANGED` does not reach the hidden
    /// restore window.
    #[test]
    fn arrival_at_target_scale_completes_the_wait_without_a_scale_message() {
        let mut target_position = waiting_target();

        advance(
            &mut target_position,
            Some(&monitor_at_scale(TARGET_SCALE)),
            Duration::ZERO,
        );

        assert_eq!(
            target_position.monitor_scale_strategy,
            MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::ApplySize)
        );
    }

    /// Several monitors commonly run at the same scale, so a matching scale on the *wrong*
    /// monitor is not arrival. Accepting it would apply the size and hand the window to settling,
    /// which then compares it against the target monitor's geometry while it sits elsewhere and
    /// reports a mismatch with no indication why.
    #[test]
    fn a_matching_scale_on_a_different_monitor_does_not_complete_the_wait() {
        let mut target_position = waiting_target();

        advance(
            &mut target_position,
            Some(&other_monitor_at_scale(TARGET_SCALE)),
            Duration::ZERO,
        );

        assert!(matches!(
            target_position.monitor_scale_strategy,
            MonitorScaleStrategy::CompensateSizeOnly(
                WindowRestoreState::WaitingForScaleChange { .. }
            )
        ));
    }

    /// A target matching no live monitor never arrives and never changes scale. The deadline has
    /// to end the wait, otherwise the window stays hidden for the life of the process.
    #[test]
    fn expired_wait_applies_size_when_no_signal_ever_arrives() {
        let mut target_position = waiting_target();

        advance(
            &mut target_position,
            None,
            Duration::from_secs_f32(SCALE_CHANGE_WAIT_TIMEOUT_SECS),
        );

        assert_eq!(
            target_position.monitor_scale_strategy,
            MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::ApplySize)
        );
    }

    #[test]
    fn wait_continues_while_the_deadline_is_unexpired_and_no_signal_arrives() {
        let mut target_position = waiting_target();

        advance(
            &mut target_position,
            Some(&monitor_at_scale(STARTING_SCALE)),
            Duration::ZERO,
        );

        assert!(matches!(
            target_position.monitor_scale_strategy,
            MonitorScaleStrategy::CompensateSizeOnly(
                WindowRestoreState::WaitingForScaleChange { .. }
            )
        ));
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::prelude::default;
    use bevy::window::MonitorSelection;
    use bevy::window::WindowResolution;

    use super::*;

    const BORDERLESS_TARGET_POSITION: IVec2 = IVec2::new(-4_256, -2_249);
    const FALLBACK_MONITOR_INDEX: usize = 0;
    const TARGET_MONITOR_INDEX: usize = 2;

    const fn current_monitor(index: usize, scale: f64) -> CurrentMonitor {
        CurrentMonitor {
            descriptor:            crate::MonitorDescriptor::for_current_enumeration(
                index,
                scale,
                IVec2::ZERO,
                UVec2::new(3_440, 1_440),
            ),
            effective_window_mode: WindowMode::BorderlessFullscreen(MonitorSelection::Index(index)),
        }
    }

    /// Mirrors the `no_position_cross_dpi_restore` suite case: 800x600 logical saved with no
    /// position, launched on a 2.0-scale monitor, restoring to a 1.0-scale monitor.
    const fn unpositioned_cross_dpi_target() -> TargetPosition {
        TargetPosition {
            placement_decision:          SavedWindowPlacementDecision::NotSaved,
            physical_size:               UVec2::new(800, 600),
            logical_size:                UVec2::new(800, 600),
            target_scale:                1.0,
            starting_scale:              2.0,
            monitor_scale_strategy:      MonitorScaleStrategy::HigherToLower(
                WindowRestoreState::NeedInitialMove,
            ),
            saved_window_mode:           SavedWindowMode::Windowed,
            monitor_index:               CurrentMonitorIndex::from_current_enumeration(
                TARGET_MONITOR_INDEX,
            ),
            fullscreen_restore_progress: FullscreenRestoreProgress::ReadyForGeometry,
            window_settle_progress:      WindowSettleProgress::NotStarted,
        }
    }

    const fn borderless_target() -> TargetPosition {
        TargetPosition {
            placement_decision:          SavedWindowPlacementDecision::Restorable {
                physical_position: BORDERLESS_TARGET_POSITION,
                logical_position:  BORDERLESS_TARGET_POSITION,
            },
            physical_size:               UVec2::new(3_440, 1_440),
            logical_size:                UVec2::new(3_440, 1_440),
            target_scale:                1.0,
            starting_scale:              1.0,
            monitor_scale_strategy:      MonitorScaleStrategy::ApplyUnchanged,
            saved_window_mode:           SavedWindowMode::BorderlessFullscreen,
            monitor_index:               CurrentMonitorIndex::from_current_enumeration(
                TARGET_MONITOR_INDEX,
            ),
            fullscreen_restore_progress: FullscreenRestoreProgress::Advancing(
                FullscreenRestoreState::LeaveFullscreen,
            ),
            window_settle_progress:      WindowSettleProgress::NotStarted,
        }
    }

    fn advance(
        target: &mut TargetPosition,
        window: &mut Window,
        current_monitor: &CurrentMonitor,
        native_fullscreen: NativeFullscreenState,
    ) -> SavedGeometryPlacementProgress {
        let Ok(role) = crate::persistence::primary_window_role() else {
            return SavedGeometryPlacementProgress::Waiting;
        };
        let preparation = WindowRestoreAttempt::for_test(role);
        restore_window(
            Entity::from_bits(1),
            &preparation,
            target,
            window,
            &ObservedScaleInputs::default(),
            Platform::MacOs,
            true,
            Some(current_monitor),
            native_fullscreen,
            Duration::ZERO,
        )
    }

    fn assert_windowed_move_pending(target: &TargetPosition) {
        assert!(matches!(
            &target.fullscreen_restore_progress,
            FullscreenRestoreProgress::Advancing(
                FullscreenRestoreState::MoveWindowedToTarget { .. }
            )
        ));
    }

    /// `set_physical_resolution` is interpreted at the scale the window is on when the request is
    /// made, and the compositor carries the resulting logical size through the move, so the
    /// request has to be `logical_size * starting_scale` for the window to land on the target
    /// monitor at `TargetPosition::physical_size`. Passing `physical_size` straight through left
    /// the window at `physical_size / starting_scale * target_scale` and settle never reached the
    /// saved size.
    #[test]
    fn an_unpositioned_cross_dpi_restore_requests_the_size_that_survives_the_move() {
        let mut target = unpositioned_cross_dpi_target();
        let mut resolution = WindowResolution::new(1_600, 1_200);
        resolution.set_scale_factor(2.0);
        let mut window = Window {
            resolution,
            visible: false,
            ..default()
        };

        let restore_status = advance(
            &mut target,
            &mut window,
            &current_monitor(FALLBACK_MONITOR_INDEX, 2.0),
            NativeFullscreenState::Windowed,
        );

        let requested_logical =
            window.resolution.physical_size().as_dvec2() / target.starting_scale;
        assert_eq!(
            requested_logical.as_uvec2(),
            target.logical_size,
            "the request must resolve to the saved logical size at the window's current scale"
        );
        assert_eq!(
            (requested_logical * target.target_scale).as_uvec2(),
            target.physical_size,
            "the logical size carried through the move must resolve to the saved physical size"
        );
        assert_eq!(
            window.position,
            WindowPosition::Centered(MonitorSelection::Index(TARGET_MONITOR_INDEX))
        );
        assert!(matches!(
            restore_status,
            SavedGeometryPlacementProgress::Complete
        ));
        assert!(!window.visible);
        assert!(matches!(
            &target.window_settle_progress,
            WindowSettleProgress::Settling(_)
        ));
    }

    #[test]
    fn macos_borderless_retarget_moves_windowed_to_target_before_fullscreen() {
        let fallback = current_monitor(FALLBACK_MONITOR_INDEX, 2.0);
        let target_monitor = current_monitor(TARGET_MONITOR_INDEX, 1.0);
        let mut target = borderless_target();
        let mut window = Window {
            mode: WindowMode::BorderlessFullscreen(MonitorSelection::Index(FALLBACK_MONITOR_INDEX)),
            ..default()
        };

        advance(
            &mut target,
            &mut window,
            &fallback,
            NativeFullscreenState::Fullscreen,
        );
        assert_eq!(window.mode, WindowMode::Windowed);
        assert_windowed_move_pending(&target);

        advance(
            &mut target,
            &mut window,
            &target_monitor,
            NativeFullscreenState::Fullscreen,
        );
        assert_windowed_move_pending(&target);
        assert_eq!(window.position, WindowPosition::Automatic);

        advance(
            &mut target,
            &mut window,
            &fallback,
            NativeFullscreenState::Windowed,
        );
        assert_windowed_move_pending(&target);
        assert_eq!(
            window.position,
            WindowPosition::At(BORDERLESS_TARGET_POSITION)
        );

        advance(
            &mut target,
            &mut window,
            &target_monitor,
            NativeFullscreenState::Windowed,
        );
        assert!(matches!(
            &target.fullscreen_restore_progress,
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::ApplyMode)
        ));

        advance(
            &mut target,
            &mut window,
            &fallback,
            NativeFullscreenState::Windowed,
        );
        assert_eq!(
            window.mode,
            WindowMode::BorderlessFullscreen(MonitorSelection::Index(TARGET_MONITOR_INDEX))
        );
        assert!(matches!(
            &target.fullscreen_restore_progress,
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::ActivateWindow)
        ));
        assert!(matches!(
            &target.window_settle_progress,
            WindowSettleProgress::NotStarted
        ));

        advance(
            &mut target,
            &mut window,
            &fallback,
            NativeFullscreenState::Windowed,
        );
        assert!(matches!(
            &target.fullscreen_restore_progress,
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::WaitForTarget)
        ));

        advance(
            &mut target,
            &mut window,
            &fallback,
            NativeFullscreenState::Fullscreen,
        );
        assert!(matches!(
            &target.window_settle_progress,
            WindowSettleProgress::NotStarted
        ));

        advance(
            &mut target,
            &mut window,
            &target_monitor,
            NativeFullscreenState::Fullscreen,
        );
        assert!(matches!(
            &target.fullscreen_restore_progress,
            FullscreenRestoreProgress::ReadyForGeometry
        ));
        assert!(matches!(
            &target.window_settle_progress,
            WindowSettleProgress::Settling(_)
        ));
    }

    /// A move the compositor never performs must not hold the window hidden forever.
    ///
    /// winit drops a `WindowPosition::Centered` whose `MonitorSelection::Index` it cannot
    /// resolve — which is every index at window-creation time, before `WinitMonitors` is
    /// populated. A window created that way sits on whichever monitor macOS placed it on, while
    /// bevy's cache records the unhonored request, so `MoveWindowedToTarget` re-requesting the
    /// same value is skipped by `changed_windows` and `CurrentMonitor` never reports arrival.
    #[test]
    fn a_windowed_move_that_never_arrives_applies_fullscreen_once_the_deadline_passes() {
        let fallback = current_monitor(FALLBACK_MONITOR_INDEX, 2.0);
        let mut target = borderless_target();
        target.fullscreen_restore_progress =
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::MoveWindowedToTarget {
                deadline: FullscreenMoveDeadline::Awaiting(OperatingSystemWorkDeadline::new(
                    FULLSCREEN_MONITOR_MOVE_TIMEOUT_SECS,
                )),
            });
        let mut window = Window {
            visible: false,
            ..default()
        };

        advance(
            &mut target,
            &mut window,
            &fallback,
            NativeFullscreenState::Windowed,
        );
        assert!(
            matches!(
                &target.fullscreen_restore_progress,
                FullscreenRestoreProgress::Advancing(
                    FullscreenRestoreState::MoveWindowedToTarget { .. }
                )
            ),
            "the phase keeps waiting while its deadline is live"
        );

        let Ok(role) = crate::persistence::primary_window_role() else {
            return;
        };
        let preparation = WindowRestoreAttempt::for_test(role);
        restore_window(
            Entity::from_bits(1),
            &preparation,
            &mut target,
            &mut window,
            &ObservedScaleInputs::default(),
            Platform::MacOs,
            true,
            Some(&fallback),
            NativeFullscreenState::Windowed,
            Duration::from_secs_f32(FULLSCREEN_MONITOR_MOVE_TIMEOUT_SECS),
        );
        assert!(
            matches!(
                &target.fullscreen_restore_progress,
                FullscreenRestoreProgress::Advancing(FullscreenRestoreState::ApplyMode)
            ),
            "an expired deadline applies the fullscreen mode instead of re-requesting the move"
        );

        let restore_status = advance(
            &mut target,
            &mut window,
            &fallback,
            NativeFullscreenState::Windowed,
        );
        assert_eq!(
            window.mode,
            WindowMode::BorderlessFullscreen(MonitorSelection::Index(TARGET_MONITOR_INDEX))
        );
        assert!(matches!(
            restore_status,
            SavedGeometryPlacementProgress::Complete
        ));
        assert!(
            !window.visible,
            "the placement helper leaves reveal ownership to its caller"
        );
    }

    #[test]
    fn first_windowed_move_frame_does_not_charge_its_delta_against_the_deadline() {
        let fallback = current_monitor(FALLBACK_MONITOR_INDEX, 2.0);
        let mut target = borderless_target();
        target.fullscreen_restore_progress =
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::MoveWindowedToTarget {
                deadline: FullscreenMoveDeadline::NotRequested,
            });
        let mut window = Window::default();
        let Ok(role) = crate::persistence::primary_window_role() else {
            return;
        };
        let preparation = WindowRestoreAttempt::for_test(role);
        let stalled_frame_delta = Duration::from_secs_f32(FULLSCREEN_MONITOR_MOVE_TIMEOUT_SECS);

        restore_window(
            Entity::from_bits(1),
            &preparation,
            &mut target,
            &mut window,
            &ObservedScaleInputs::default(),
            Platform::MacOs,
            true,
            Some(&fallback),
            NativeFullscreenState::Windowed,
            stalled_frame_delta,
        );
        assert_eq!(
            window.position,
            WindowPosition::At(BORDERLESS_TARGET_POSITION)
        );
        assert!(matches!(
            &target.fullscreen_restore_progress,
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::MoveWindowedToTarget {
                deadline: FullscreenMoveDeadline::Awaiting(_),
            })
        ));

        window.position = WindowPosition::Automatic;
        restore_window(
            Entity::from_bits(1),
            &preparation,
            &mut target,
            &mut window,
            &ObservedScaleInputs::default(),
            Platform::MacOs,
            true,
            Some(&fallback),
            NativeFullscreenState::Windowed,
            stalled_frame_delta,
        );
        assert_eq!(
            window.position,
            WindowPosition::At(BORDERLESS_TARGET_POSITION)
        );
        assert!(matches!(
            &target.fullscreen_restore_progress,
            FullscreenRestoreProgress::Advancing(FullscreenRestoreState::ApplyMode)
        ));
    }
}
