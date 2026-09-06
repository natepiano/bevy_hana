use bevy::prelude::Reflect;

use super::target::FullscreenMoveDeadline;
use crate::deadline::OperatingSystemWorkDeadline;
use crate::restore::RestorePreparationSource;

/// State for `MonitorScaleStrategy::HigherToLower` (high→low DPI restore).
///
/// A high-DPI to low-DPI restore applies position BEFORE size, because Bevy's
/// `changed_windows` system processes size changes before position changes. Applying both in
/// one frame resizes the window while it still sits at the old position, so it extends into
/// the wrong monitor and macOS emits a `WindowScaleFactorChanged` event before the final
/// position is applied.
///
/// Moving a 1x1 window to the final position first puts the window at the target location
/// before `ApplySize` applies the size.
#[derive(Debug, Clone, PartialEq, Eq, Reflect)]
pub(crate) enum WindowRestoreState {
    /// Initial state: window needs to be moved to the target monitor to trigger a scale change.
    /// Handled by `place_window_at_saved_geometry` after driver target preparation, which calls
    /// `apply_initial_move` and transitions to `WaitingForScaleChange`.
    NeedInitialMove,
    /// Position applied with compensation, waiting for `ScaleChanged` message.
    WaitingForScaleChange {
        /// Kernel attempt that began this transition.
        source:   RestorePreparationSource,
        /// Hard bound on waiting for the operating system's scale transition.
        deadline: OperatingSystemWorkDeadline,
    },
    /// Scale changed, ready to apply the final size; the position was already applied during
    /// `NeedInitialMove`.
    ApplySize,
}

/// Phase-based fullscreen restore state machine.
///
/// Fullscreen restore requires platform-specific sequencing:
///
/// - **Linux X11**: Move to target monitor first, wait a frame for the compositor to process, then
///   apply fullscreen mode as a fresh change.
/// - **Linux Wayland**: Apply mode directly (no position available).
/// - **Windows (DX12)**: Wait for surface creation before applying fullscreen (see <https://github.com/rust-windowing/winit/issues/3124>).
/// - **macOS**: Leave the current fullscreen Space, move the windowed window to the target, then
///   enter fullscreen there.
///
/// On X11, `FullscreenRestoreState::MoveToMonitor` must complete before
/// `FullscreenRestoreState::ApplyMode`; setting fullscreen mode in the same
/// frame as position can make the compositor briefly apply fullscreen and then
/// revert it.
#[derive(Debug, Clone, PartialEq, Eq, Reflect)]
pub(crate) enum FullscreenRestoreState {
    /// Ask `AppKit` to leave the current fullscreen Space before changing monitor.
    LeaveFullscreen,
    /// Keep moving the windowed macOS window until it reaches the target monitor.
    MoveWindowedToTarget {
        /// Whether the move has been requested, and the deadline once it has.
        deadline: FullscreenMoveDeadline,
    },
    /// Move window to target monitor position. Skipped on Wayland (no position).
    MoveToMonitor,
    /// Wait for compositor to process the position change (1 frame).
    WaitForMove,
    /// Wait for GPU surface creation (Windows DX12 workaround, winit #3124).
    WaitForSurface,
    /// Apply the fullscreen mode.
    ApplyMode,
    /// Make the macOS window key after winit sends the fullscreen request.
    ActivateWindow,
    /// Wait until `AppKit` reports both fullscreen presentation and the target monitor.
    WaitForTarget,
}

/// Last native fullscreen transition that the platform confirmed as complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeFullscreenState {
    Unavailable,
    Windowed,
    Fullscreen,
}

/// Restore strategy based on scale factor relationship between launch and target monitors.
///
/// # The Problem
///
/// Winit's `request_inner_size` and `set_outer_position` use the current window's scale factor
/// when interpreting coordinates, rather than the target monitor's scale factor. This causes
/// incorrect sizing/positioning when restoring windows across monitors with different DPIs.
///
/// See: <https://github.com/rust-windowing/winit/issues/4440>
///
/// # Platform Differences
///
/// ## Windows
///
/// - **Position**: Winit uses physical coordinates directly - no compensation needed
/// - **Size**: Winit applies scale conversion using current monitor's scale - needs compensation
/// - Strategy: `CompensateSizeOnly` when scales differ
///
/// Note: Windows has a separate issue where `GetWindowRect` includes an invisible
/// resize border (~7-11 pixels). See: <https://github.com/rust-windowing/winit/issues/4107>
///
/// ## macOS / Linux X11
///
/// - **Position**: Winit converts using current monitor's scale - needs compensation
/// - **Size**: Winit converts using current monitor's scale - needs compensation
/// - Strategy: `LowerToHigher` or `HigherToLower` depending on scale relationship
///
/// ## Linux Wayland
///
/// Cannot detect starting monitor or set position, so no compensation is applied.
///
/// # Variants
///
/// - **`ApplyUnchanged`**: Apply position and size directly without compensation.
///
/// - **`CompensateSizeOnly`**: Windows only. Uses two-phase approach via `WindowRestoreState`:
///   1. Apply position directly + compensated size (triggers `WM_DPICHANGED`)
///   2. After scale changes, re-apply exact target size to eliminate rounding errors
///
/// - **`LowerToHigher`**: macOS/Linux X11. Low→High DPI (1x→2x, ratio < 1). Multiply both position
///   and size by ratio.
///
/// - **`HigherToLower`**: macOS/Linux X11. High→Low DPI (2x→1x, ratio > 1). Uses two-phase approach
///   via `WindowRestoreState` to avoid size clamping:
///   1. Move a 1x1 window to final position (compensated) to trigger scale change
///   2. After scale changes, apply size without compensation
#[derive(Debug, Clone, PartialEq, Eq, Reflect)]
pub(crate) enum MonitorScaleStrategy {
    /// Same scale - apply position and size directly.
    ApplyUnchanged,
    /// Windows cross-DPI: position direct, size in two phases.
    CompensateSizeOnly(WindowRestoreState),
    /// Low→High DPI (1x→2x) - apply with compensation (ratio < 1).
    LowerToHigher,
    /// High→Low DPI (2x→1x) - requires two phases (see enum docs).
    HigherToLower(WindowRestoreState),
}
