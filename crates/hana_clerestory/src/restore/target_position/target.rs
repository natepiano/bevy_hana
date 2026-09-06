use bevy::prelude::Component;
use bevy::prelude::IVec2;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::UVec2;
#[cfg(test)]
use bevy::prelude::debug;
#[cfg(test)]
use bevy::prelude::warn;
use hana_kana::ToI32;
use hana_kana::ToU32;

use super::strategy::FullscreenRestoreState;
use super::strategy::MonitorScaleStrategy;
use crate::Platform;
use crate::deadline::OperatingSystemWorkDeadline;
use crate::events::ExpectedLogicalPosition;
use crate::events::ExpectedPhysicalPosition;
use crate::monitors::CurrentMonitorIndex;
use crate::monitors::MonitorDescriptor;
use crate::persistence::EstablishedWindowPlacement;
#[cfg(test)]
use crate::persistence::PersistedPosition;
#[cfg(test)]
use crate::persistence::PersistedWindowState;
use crate::persistence::RestorableWindowPosition;
use crate::persistence::SavedWindowMode;
#[cfg(test)]
use crate::persistence::UnrebasedDesktopPosition;
use crate::restore::settle_state::SettleState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum SavedWindowPlacementDecision {
    #[cfg(test)]
    /// Resolved from a persisted monitor-relative offset. Clamped on macOS.
    PersistedOffset {
        physical_position: IVec2,
        logical_position:  IVec2,
    },
    #[cfg(test)]
    /// The saved record explicitly contained no coordinate.
    NotSaved,
    #[cfg(test)]
    /// A pre-v3 absolute coordinate no longer resolves safely against live geometry.
    DiscardedLegacy,
    /// The kernel configuration rebased through the exact live target display.
    Restorable {
        physical_position: IVec2,
        logical_position:  IVec2,
    },
    CompositorControlled,
}
impl SavedWindowPlacementDecision {
    #[must_use]
    const fn meaning(self, platform: Platform) -> PreparedPositionMeaning {
        if !platform.position_available() {
            return PreparedPositionMeaning::PlatformCannotPosition;
        }
        match self {
            Self::Restorable { .. } => PreparedPositionMeaning::Specified,
            #[cfg(test)]
            Self::PersistedOffset { .. } => PreparedPositionMeaning::Specified,
            #[cfg(test)]
            Self::NotSaved => PreparedPositionMeaning::NotSaved,
            #[cfg(test)]
            Self::DiscardedLegacy => PreparedPositionMeaning::DiscardedLegacy,
            Self::CompositorControlled => PreparedPositionMeaning::PlatformCannotPosition,
        }
    }
}

/// Why target preparation did or did not produce window coordinates.
///
/// Both physical and logical event fields consume this because every variant is meaningful for
/// both coordinate spaces; the public events still use their distinct field-specific enums.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub(crate) enum PreparedPositionMeaning {
    /// Live geometry resolved a coordinate the platform can apply.
    Specified,
    /// The active compositor does not permit client-selected placement.
    PlatformCannotPosition,
    /// The serialized record contained no coordinate.
    NotSaved,
    /// A legacy absolute coordinate no longer resolved safely and was rejected.
    DiscardedLegacy,
}

/// Resolve a persisted position against the live target monitor for migration tests.
///
/// A [`PersistedPosition::MonitorOffset`] is already scale-independent and rebases directly. An
/// [`PersistedPosition::Unrebased`] entry is a pre-v3 absolute desktop coordinate whose origin
/// depended on the layout at save time; [`rebase_legacy_position`] validates it before it is used.
#[must_use]
#[cfg(test)]
fn prepare_persisted_position(
    persisted_position: PersistedPosition,
    logical_size: UVec2,
    target_info: &MonitorDescriptor,
) -> SavedWindowPlacementDecision {
    let logical_offset = match persisted_position {
        PersistedPosition::MonitorOffset(logical_offset) => logical_offset,
        PersistedPosition::Unrebased(unrebased) => {
            match rebase_legacy_position(unrebased, logical_size, target_info) {
                Some(logical_offset) => logical_offset,
                None => return SavedWindowPlacementDecision::DiscardedLegacy,
            }
        },
        PersistedPosition::Unpositioned => {
            return SavedWindowPlacementDecision::NotSaved;
        },
    };

    SavedWindowPlacementDecision::PersistedOffset {
        physical_position: target_info.physical_from_logical_offset(logical_offset),
        logical_position:  target_info.logical_from_logical_offset(logical_offset),
    }
}

/// Rebase an established driver configuration through the one exact live monitor descriptor.
#[must_use]
fn prepare_established_position(
    placement: &EstablishedWindowPlacement,
    target_monitor: &MonitorDescriptor,
) -> SavedWindowPlacementDecision {
    match placement.restorable_position(target_monitor) {
        RestorableWindowPosition::Restorable {
            physical_position,
            logical_position,
        } => SavedWindowPlacementDecision::Restorable {
            physical_position,
            logical_position,
        },
        RestorableWindowPosition::CompositorControlled => {
            SavedWindowPlacementDecision::CompositorControlled
        },
    }
}

/// Explain whether an established configuration can supply a platform position.
#[must_use]
pub(crate) fn prepared_established_position_meaning(
    placement: &EstablishedWindowPlacement,
    target_monitor: &MonitorDescriptor,
    platform: Platform,
) -> PreparedPositionMeaning {
    prepare_established_position(placement, target_monitor).meaning(platform)
}

/// Convert a pre-v3 absolute desktop coordinate into an offset from `target_info`, or reject it.
///
/// The file carries its own detector. Multiplying the saved coordinate by the scale that wrote it
/// reconstructs where the window physically sat at save time, to within one pixel of that scale.
/// Both that reconstruction and the offset error it guards move with the monitor's origin, so
/// testing the reconstruction against the monitor's current rectangle rejects exactly the cases
/// where the offset would put the window somewhere the user never left it:
///
/// - Layout unchanged → lands inside → kept, and the conversion is exact.
/// - Only this monitor's scale changed → its origin is unmoved, the division cancels with no
///   residue → still inside → kept. No false positive on the common cross-DPI case.
/// - A neighbouring monitor's scale change shifted this monitor's origin → falls outside once the
///   shift exceeds half the window → rejected, and the window is placed by the compositor rather
///   than somewhere the user never put it, possibly off every screen.
///
/// The window's reconstructed **center** is what gets tested — not its top-left corner, and not
/// its whole rectangle. Windows and Linux both let a window span monitors, and which monitor such
/// a window belongs to is decided by where its middle sits (`MonitorFromWindow` with
/// `MONITOR_DEFAULTTONEAREST` on Windows, and the equivalent in this crate's own tracking). A
/// window deliberately straddling a boundary has a corner on the *neighbouring* monitor while
/// still belonging to its own; a corner test or a rectangle test would discard that position even
/// though nothing about the layout has changed.
#[cfg(test)]
fn rebase_legacy_position(
    unrebased: UnrebasedDesktopPosition,
    logical_size: UVec2,
    target_info: &MonitorDescriptor,
) -> Option<IVec2> {
    let captured_scale = unrebased.captured_scale();
    let saved_logical = unrebased.logical();
    let reconstructed_center = reconstructed_legacy_window_center(unrebased, logical_size);

    if !monitor_contains_physical_point(target_info, reconstructed_center) {
        warn!(
            "[rebase_legacy_position] Discarding saved position {saved_logical} \
             (monitor_scale {captured_scale}): its center reconstructs to \
             {reconstructed_center}, outside monitor {} at {} size {}. The desktop layout \
             changed while the app was closed; the window will be placed by the compositor \
             instead.",
            target_info.index, target_info.physical_position, target_info.physical_size
        );
        return None;
    }

    Some(saved_logical - target_info.logical_origin_at_scale(captured_scale))
}

/// Reconstruct the physical center encoded by one pre-v3 desktop coordinate.
#[cfg(test)]
pub(crate) fn reconstructed_legacy_window_center(
    unrebased: UnrebasedDesktopPosition,
    logical_size: UVec2,
) -> IVec2 {
    let captured_scale = unrebased.captured_scale();
    let saved_logical = unrebased.logical();
    let reconstructed_corner = IVec2::new(
        (f64::from(saved_logical.x) * captured_scale)
            .round()
            .to_i32(),
        (f64::from(saved_logical.y) * captured_scale)
            .round()
            .to_i32(),
    );
    reconstructed_corner
        + IVec2::new(
            (f64::from(logical_size.x) * captured_scale / 2.0)
                .round()
                .to_i32(),
            (f64::from(logical_size.y) * captured_scale / 2.0)
                .round()
                .to_i32(),
        )
}

/// Whether a physical desktop point lies within a monitor's bounds.
#[cfg(test)]
pub(crate) fn monitor_contains_physical_point(
    descriptor: &MonitorDescriptor,
    physical_point: IVec2,
) -> bool {
    let far_corner = descriptor.physical_position + descriptor.physical_size.as_ivec2();
    physical_point.cmpge(descriptor.physical_position).all()
        && physical_point.cmplt(far_corner).all()
}

/// The target window state, held on the window entity for the duration of a restore.
///
/// The saved record is converted into these typed fields — `IVec2`, `UVec2`,
/// `WindowMode`, and scale factors — while the target is prepared, before any restore
/// system reads them.
///
/// Dimensions stored here are **inner** (content area only), matching what
/// Bevy's `Window.resolution` represents and what the state file records.
/// Outer dimensions (including title bar) are used only during preparation, for
/// clamping calculations.
#[derive(Component, Reflect)]
#[reflect(Component)]
#[type_path = "hana_clerestory::restore"]
pub(crate) struct TargetPosition {
    /// Whether saved coordinates should be applied, and both coordinates when they should.
    pub(super) placement_decision:                 SavedWindowPlacementDecision,
    /// Target size in physical pixels (content area, excluding window decoration).
    pub(in crate::restore) physical_size:          UVec2,
    /// Target size in logical pixels from the saved state.
    pub(in crate::restore) logical_size:           UVec2,
    /// Scale factor of the target monitor.
    pub(in crate::restore) target_scale:           f64,
    /// Scale factor of the monitor where the window starts (keyboard focus monitor).
    pub(super) starting_scale:                     f64,
    /// Strategy for handling scale factor differences between monitors.
    pub(in crate::restore) monitor_scale_strategy: MonitorScaleStrategy,
    /// Window mode to restore.
    pub(in crate::restore) saved_window_mode:      SavedWindowMode,
    /// Target monitor index for fullscreen restore.
    /// On non-Wayland platforms this is derivable from the position; Wayland reports no
    /// window position, so the index is stored explicitly.
    pub(in crate::restore) monitor_index:          CurrentMonitorIndex,
    /// Fullscreen-specific progress that is absent only when no fullscreen step remains.
    pub(super) fullscreen_restore_progress:        FullscreenRestoreProgress,
    /// Settling state. Once `try_apply_restore` has run, this holds the observation of what
    /// the compositor and winit report back, until those values stop changing.
    ///
    /// Two timers run at once:
    /// - **Stability timer** (200ms): resets whenever any compared value changes between frames.
    ///   Fires `WindowRestored` when all values have been stable for 200ms.
    /// - **Total timeout** (2s): hard deadline. If values never stay stable for 200ms
    ///   continuously, fires `WindowRestoreMismatch` with whatever state exists at timeout.
    ///
    /// The stability timer absorbs transient Wayland `wl_surface.enter`/`wl_surface.leave`
    /// reports, where `current_monitor()` briefly returns a different monitor during
    /// fullscreen transitions.
    pub(in crate::restore) window_settle_progress: WindowSettleProgress,
}

#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum FullscreenRestoreProgress {
    /// No fullscreen-specific step prevents normal geometry application.
    ReadyForGeometry,
    /// This fullscreen-specific step is still running.
    Advancing(FullscreenRestoreState),
    /// The fullscreen step finished and normal geometry application may continue.
    Completed,
}

/// Whether the windowed move has been requested yet, and if so what bounds it.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum FullscreenMoveDeadline {
    /// The move has not been requested yet, so nothing is being waited on.
    NotRequested,
    /// The move was requested; this bounds how long the operating system gets.
    Awaiting(OperatingSystemWorkDeadline),
}

#[derive(Clone, Debug, Reflect)]
pub(crate) enum WindowSettleProgress {
    /// Geometry application has not started compositor settling yet.
    NotStarted,
    /// The compositor's resulting window state is being observed for stability.
    Settling(SettleState),
}

impl TargetPosition {
    /// Scale ratio between starting and target monitors.
    #[must_use]
    pub(super) const fn ratio(&self) -> f64 { self.starting_scale / self.target_scale }

    /// Position compensated for scale factor differences.
    ///
    /// Multiplies physical position by the ratio to account for winit dividing by launch scale.
    #[must_use]
    pub(super) fn compensated_position(&self) -> ExpectedPhysicalPosition {
        let ratio = self.ratio();
        match self.physical_position() {
            ExpectedPhysicalPosition::Specified(position) => {
                ExpectedPhysicalPosition::Specified(IVec2::new(
                    (f64::from(position.x) * ratio).to_i32(),
                    (f64::from(position.y) * ratio).to_i32(),
                ))
            },
            position => position,
        }
    }

    #[must_use]
    pub(in crate::restore) const fn physical_position(&self) -> ExpectedPhysicalPosition {
        match self.placement_decision {
            #[cfg(test)]
            SavedWindowPlacementDecision::PersistedOffset {
                physical_position, ..
            } => ExpectedPhysicalPosition::Specified(physical_position),
            SavedWindowPlacementDecision::Restorable {
                physical_position, ..
            } => ExpectedPhysicalPosition::Specified(physical_position),
            SavedWindowPlacementDecision::CompositorControlled => {
                ExpectedPhysicalPosition::PlatformCannotPosition
            },
            #[cfg(test)]
            SavedWindowPlacementDecision::NotSaved => ExpectedPhysicalPosition::NotSaved,
            #[cfg(test)]
            SavedWindowPlacementDecision::DiscardedLegacy => {
                ExpectedPhysicalPosition::DiscardedLegacy
            },
        }
    }

    #[must_use]
    pub(in crate::restore) const fn logical_position(&self) -> ExpectedLogicalPosition {
        match self.placement_decision {
            #[cfg(test)]
            SavedWindowPlacementDecision::PersistedOffset {
                logical_position, ..
            } => ExpectedLogicalPosition::Specified(logical_position),
            SavedWindowPlacementDecision::Restorable {
                logical_position, ..
            } => ExpectedLogicalPosition::Specified(logical_position),
            SavedWindowPlacementDecision::CompositorControlled => {
                ExpectedLogicalPosition::PlatformCannotPosition
            },
            #[cfg(test)]
            SavedWindowPlacementDecision::NotSaved => ExpectedLogicalPosition::NotSaved,
            #[cfg(test)]
            SavedWindowPlacementDecision::DiscardedLegacy => {
                ExpectedLogicalPosition::DiscardedLegacy
            },
        }
    }

    /// Subtract the X11 window-frame top from the saved physical position.
    ///
    /// X11 reports the client-area origin where the frame origin belongs, so an
    /// uncompensated save/restore cycle walks the window down by the title bar height
    /// every time. Decisions that carry no coordinate have nothing to compensate.
    #[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
    pub(in crate::restore) const fn compensate_physical_position(
        &mut self,
        physical_frame_top: i32,
    ) {
        match &mut self.placement_decision {
            SavedWindowPlacementDecision::Restorable {
                physical_position, ..
            } => physical_position.y -= physical_frame_top,
            #[cfg(test)]
            SavedWindowPlacementDecision::PersistedOffset {
                physical_position, ..
            } => physical_position.y -= physical_frame_top,
            SavedWindowPlacementDecision::CompositorControlled => {},
            #[cfg(test)]
            SavedWindowPlacementDecision::NotSaved
            | SavedWindowPlacementDecision::DiscardedLegacy => {},
        }
    }

    /// Size compensated for scale factor differences.
    ///
    /// Multiplies physical size by the ratio to account for winit dividing by launch scale.
    #[must_use]
    pub(super) fn compensated_size(&self) -> UVec2 {
        let ratio = self.ratio();
        UVec2::new(
            (f64::from(self.physical_size.x) * ratio).to_u32(),
            (f64::from(self.physical_size.y) * ratio).to_u32(),
        )
    }
}

/// Durable record of a restore's launch context and the strategy it ran.
///
/// Unlike [`TargetPosition`], this is **not** removed when the restore settles —
/// it stays on the entity so a test can read, via BRP, which monitor the window actually
/// launched on and which [`MonitorScaleStrategy`] ran. On macOS the launch monitor is
/// whichever display the OS spawns the window on, so a cross-DPI test can silently
/// degrade into a same-scale restore; these fields let the test see that degradation and
/// fail on it.
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(Component)]
#[type_path = "hana_clerestory::restore"]
pub(crate) struct RestoreDiagnostics {
    /// Monitor the window launched on, before any restore move.
    pub(in crate::restore) starting_monitor_index: CurrentMonitorIndex,
    /// Scale factor of the launch monitor.
    pub(in crate::restore) starting_scale:         f64,
    /// Scale factor of the restore target monitor.
    pub(in crate::restore) target_scale:           f64,
    /// Strategy chosen from the launch-versus-target scale relationship.
    pub(in crate::restore) monitor_scale_strategy: MonitorScaleStrategy,
}

/// Compute a target from the configuration fields the driver will apply.
#[must_use]
fn compute_target_position(
    logical_size: UVec2,
    saved_window_mode: &SavedWindowMode,
    target_info: &MonitorDescriptor,
    prepared_window_position: SavedWindowPlacementDecision,
    physical_decoration: UVec2,
    starting_scale: f64,
    platform: Platform,
) -> TargetPosition {
    let target_scale = target_info.scale;

    #[cfg(not(test))]
    let _ = physical_decoration;

    // Convert logical → physical using the target monitor's scale factor.
    // This is the single conversion point for size values.
    let physical_width = (f64::from(logical_size.x) * target_scale).to_u32();
    let physical_height = (f64::from(logical_size.y) * target_scale).to_u32();

    #[cfg(test)]
    let physical_outer_width = physical_width + physical_decoration.x;
    #[cfg(test)]
    let physical_outer_height = physical_height + physical_decoration.y;
    let placement_decision = match prepared_window_position {
        #[cfg(test)]
        SavedWindowPlacementDecision::PersistedOffset {
            physical_position,
            logical_position,
        } => {
            let physical_position = clamp_position_to_monitor(
                physical_position.x,
                physical_position.y,
                target_info,
                physical_outer_width,
                physical_outer_height,
                platform,
            );
            SavedWindowPlacementDecision::Restorable {
                physical_position,
                logical_position,
            }
        },
        decision @ (SavedWindowPlacementDecision::Restorable { .. }
        | SavedWindowPlacementDecision::CompositorControlled) => decision,
        #[cfg(test)]
        decision @ (SavedWindowPlacementDecision::NotSaved
        | SavedWindowPlacementDecision::DiscardedLegacy) => decision,
    };

    TargetPosition {
        placement_decision,
        physical_size: UVec2::new(physical_width, physical_height),
        logical_size,
        target_scale,
        starting_scale,
        monitor_scale_strategy: platform.scale_strategy(starting_scale, target_scale),
        saved_window_mode: saved_window_mode.clone(),
        monitor_index: target_info.index,
        fullscreen_restore_progress: if saved_window_mode.is_fullscreen() {
            FullscreenRestoreProgress::Advancing(platform.fullscreen_restore_state())
        } else {
            FullscreenRestoreProgress::ReadyForGeometry
        },
        window_settle_progress: WindowSettleProgress::NotStarted,
    }
}

/// Build a target from one kernel-owned window configuration without a second configuration type.
#[must_use]
pub(crate) fn compute_established_target_position(
    placement: &EstablishedWindowPlacement,
    target_monitor: &MonitorDescriptor,
    physical_decoration: UVec2,
    starting_scale: f64,
    platform: Platform,
) -> TargetPosition {
    compute_target_position(
        placement.logical_size,
        &placement.saved_window_mode,
        target_monitor,
        prepare_established_position(placement, target_monitor),
        physical_decoration,
        starting_scale,
        platform,
    )
}

#[cfg(test)]
fn compute_persisted_target_position(
    saved_window_state: &PersistedWindowState,
    target_info: &MonitorDescriptor,
    prepared_window_position: SavedWindowPlacementDecision,
    physical_decoration: UVec2,
    starting_scale: f64,
    platform: Platform,
) -> TargetPosition {
    compute_target_position(
        UVec2::new(
            saved_window_state.logical_width,
            saved_window_state.logical_height,
        ),
        &saved_window_state.saved_window_mode,
        target_info,
        prepared_window_position,
        physical_decoration,
        starting_scale,
        platform,
    )
}

/// Calculate restored window position, with optional clamping.
///
/// On macOS, clamps to monitor bounds because macOS may resize/reposition windows
/// that extend beyond the screen. macOS does not allow windows to span monitors.
///
/// On Windows and Linux a window can span multiple monitors, so the saved position is
/// applied exactly, without clamping.
#[must_use]
#[cfg(test)]
fn clamp_position_to_monitor(
    physical_saved_x: i32,
    physical_saved_y: i32,
    target_info: &MonitorDescriptor,
    physical_outer_width: u32,
    physical_outer_height: u32,
    platform: Platform,
) -> IVec2 {
    if platform.should_clamp_position() {
        let physical_monitor_right =
            target_info.physical_position.x + target_info.physical_size.x.to_i32();
        let physical_monitor_bottom =
            target_info.physical_position.y + target_info.physical_size.y.to_i32();

        let mut physical_x = physical_saved_x;
        let mut physical_y = physical_saved_y;

        if physical_x + physical_outer_width.to_i32() > physical_monitor_right {
            physical_x = physical_monitor_right - physical_outer_width.to_i32();
        }
        if physical_y + physical_outer_height.to_i32() > physical_monitor_bottom {
            physical_y = physical_monitor_bottom - physical_outer_height.to_i32();
        }
        physical_x = physical_x.max(target_info.physical_position.x);
        physical_y = physical_y.max(target_info.physical_position.y);

        if physical_x != physical_saved_x || physical_y != physical_saved_y {
            debug!(
                "[clamp_position_to_monitor] Clamped: ({physical_saved_x}, {physical_saved_y}) -> ({physical_x}, {physical_y}) for outer size {physical_outer_width}x{physical_outer_height}"
            );
        }

        IVec2::new(physical_x, physical_y)
    } else {
        IVec2::new(physical_saved_x, physical_saved_y)
    }
}
#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use super::*;
    use crate::persistence::PersistedDisplayIdentityV4;
    use crate::persistence::PersistedWindowTargetV5;

    const MONITOR_SIZE: UVec2 = UVec2::new(2_560, 1_440);
    /// Left-hand monitor origin in the layout that produced the observed failure.
    const LEFT_MONITOR_ORIGIN: IVec2 = IVec2::new(-6_880, 0);

    fn monitor(index: usize, scale: f64, physical_position: IVec2) -> MonitorDescriptor {
        MonitorDescriptor::for_current_enumeration(index, scale, physical_position, MONITOR_SIZE)
    }

    fn saved_state(position: PersistedPosition) -> PersistedWindowState {
        PersistedWindowState {
            target: PersistedWindowTargetV5::AwaitingLegacyEvidence(
                PersistedDisplayIdentityV4::Anonymous,
            ),
            position,
            logical_width: 800,
            logical_height: 600,
            saved_window_mode: SavedWindowMode::Windowed,
            app_name: "test".to_string(),
        }
    }

    fn unrebased(logical: IVec2, captured_scale: f64) -> PersistedPosition {
        match UnrebasedDesktopPosition::from_test_legacy(logical, captured_scale) {
            Some(unrebased) => PersistedPosition::Unrebased(unrebased),
            None => panic!("fixture scale {captured_scale} should be accepted"),
        }
    }

    fn target_for(
        position: PersistedPosition,
        target_info: &MonitorDescriptor,
        platform: Platform,
    ) -> TargetPosition {
        let saved = saved_state(position);
        let prepared_window_position = prepare_persisted_position(
            position,
            UVec2::new(saved.logical_width, saved.logical_height),
            target_info,
        );
        compute_persisted_target_position(
            &saved,
            target_info,
            prepared_window_position,
            UVec2::ZERO,
            target_info.scale,
            platform,
        )
    }

    /// The defect, in one test.
    ///
    /// A window saved 120x80 logical pixels into a monitor that later changes scale must come
    /// back to the same spot on that monitor. Under the pre-v3 absolute-coordinate format the
    /// file held `-6880 / 1.0 + 120 = -6760`, and restore recomputed `-6760 * 1.5 = -10140` —
    /// 3260 pixels left of the monitor's own left edge and off every screen in the layout.
    #[test]
    fn scale_change_between_save_and_restore_keeps_the_window_on_its_monitor() {
        let logical_offset = IVec2::new(120, 80);
        let live_monitor = monitor(1, 1.5, LEFT_MONITOR_ORIGIN);

        let target = target_for(
            PersistedPosition::MonitorOffset(logical_offset),
            &live_monitor,
            Platform::Windows,
        );

        assert_eq!(
            target.physical_position(),
            ExpectedPhysicalPosition::Specified(IVec2::new(-6_700, 120))
        );
        let ExpectedPhysicalPosition::Specified(physical_position) = target.physical_position()
        else {
            panic!("a monitor offset always resolves to a position")
        };
        assert!(
            monitor_contains_physical_point(&live_monitor, physical_position),
            "the restored corner must land on the monitor it was saved against"
        );
    }

    /// An offset is scale-independent: the same saved value restores onto the same monitor at
    /// every scale, and always inside it.
    #[test]
    fn the_same_offset_restores_inside_the_monitor_at_every_scale() {
        let logical_offset = IVec2::new(120, 80);
        for origin in [IVec2::ZERO, LEFT_MONITOR_ORIGIN, -LEFT_MONITOR_ORIGIN] {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let target_info = monitor(1, scale, origin);
                let target = target_for(
                    PersistedPosition::MonitorOffset(logical_offset),
                    &target_info,
                    Platform::Windows,
                );

                let expected = origin
                    + IVec2::new(
                        (f64::from(logical_offset.x) * scale).round().to_i32(),
                        (f64::from(logical_offset.y) * scale).round().to_i32(),
                    );
                assert_eq!(
                    target.physical_position(),
                    ExpectedPhysicalPosition::Specified(expected),
                    "origin {origin} at scale {scale}"
                );
                assert!(
                    monitor_contains_physical_point(&target_info, expected),
                    "origin {origin} at scale {scale} put the corner outside its own monitor"
                );
            }
        }
    }

    /// The persisted and captured paths must agree for the same offset and monitor. Guards
    /// against either side re-inlining the arithmetic that `MonitorDescriptor` now owns.
    #[test]
    fn persisted_and_captured_paths_resolve_the_same_offset_identically() {
        let logical_offset = IVec2::new(37, -11);
        let target_info = monitor(1, 1.5, LEFT_MONITOR_ORIGIN);

        let persisted = target_for(
            PersistedPosition::MonitorOffset(logical_offset),
            &target_info,
            Platform::Windows,
        );
        let captured = compute_persisted_target_position(
            &saved_state(PersistedPosition::Unpositioned),
            &target_info,
            SavedWindowPlacementDecision::Restorable {
                physical_position: target_info.physical_from_logical_offset(logical_offset),
                logical_position:  target_info.logical_from_logical_offset(logical_offset),
            },
            UVec2::ZERO,
            target_info.scale,
            Platform::Windows,
        );

        assert_eq!(persisted.physical_position(), captured.physical_position());
        assert_eq!(persisted.logical_position(), captured.logical_position());
    }

    /// The restore is sized and placed by the monitor it is going *to*, never the one the app
    /// launched on.
    ///
    /// Launching from a differently-scaled monitor is the ordinary case — the window manager
    /// opens the app wherever the invoking terminal or editor sits. Sizing from `starting_scale`
    /// would be wrong for every such restore, and no test whose launch monitor is its target can
    /// tell the two apart, so this is the only unit test that separates them.
    #[test]
    fn the_target_monitor_scale_sizes_the_restore_not_the_launch_monitor_scale() {
        let starting_scale = 1.0;
        let logical_offset = IVec2::new(120, 80);
        let target_info = monitor(1, 2.0, LEFT_MONITOR_ORIGIN);
        let position = PersistedPosition::MonitorOffset(logical_offset);
        let saved = saved_state(position);

        let target = compute_persisted_target_position(
            &saved,
            &target_info,
            prepare_persisted_position(
                position,
                UVec2::new(saved.logical_width, saved.logical_height),
                &target_info,
            ),
            UVec2::ZERO,
            starting_scale,
            Platform::Windows,
        );

        assert_eq!(
            target.physical_size,
            UVec2::new(1_600, 1_200),
            "800x600 logical belongs to the target monitor at scale 2.0, not the launch monitor at {starting_scale}"
        );
        assert_eq!(
            target.physical_position(),
            ExpectedPhysicalPosition::Specified(IVec2::new(-6_640, 160)),
            "the offset scales by the target monitor, so the launch scale cannot move the window"
        );
        assert_ne!(
            target.monitor_scale_strategy,
            MonitorScaleStrategy::ApplyUnchanged,
            "a launch scale that differs from the target must select a cross-DPI strategy"
        );
    }

    /// An unchanged layout reconstructs inside the monitor, so the legacy coordinate is kept and
    /// converted exactly: the window returns to the pixel it was saved at.
    #[test]
    fn a_legacy_coordinate_from_an_unchanged_layout_is_kept_and_converts_exactly() {
        let target_info = monitor(1, 1.0, LEFT_MONITOR_ORIGIN);
        // Saved by v2 as round(-6880 / 1.0) + 120 = -6760.
        let position = unrebased(IVec2::new(-6_760, 80), 1.0);

        let target = target_for(position, &target_info, Platform::Windows);

        assert_eq!(
            target.physical_position(),
            ExpectedPhysicalPosition::Specified(IVec2::new(-6_760, 80))
        );
    }

    /// Only the target monitor's own scale changed: its origin did not move, the division
    /// cancels with no residue, and the coordinate is kept. This is the common cross-DPI case
    /// and must not trip the detector.
    #[test]
    fn a_legacy_coordinate_survives_a_scale_change_on_its_own_monitor() {
        let live_monitor = monitor(1, 1.5, LEFT_MONITOR_ORIGIN);
        let position = unrebased(IVec2::new(-6_760, 80), 1.0);

        let target = target_for(position, &live_monitor, Platform::Windows);

        // offset = -6760 - round(-6880 / 1.0) = 120, then rebased at the live scale.
        assert_eq!(
            target.physical_position(),
            ExpectedPhysicalPosition::Specified(IVec2::new(-6_700, 120))
        );
    }

    /// A neighbouring monitor's scale change moved this monitor's origin, so the saved
    /// coordinate no longer describes anywhere on it. Dropped rather than restored off-screen.
    #[test]
    fn a_legacy_coordinate_is_dropped_once_its_monitor_origin_has_moved() {
        // Saved when this monitor sat at -6880; a neighbour's scale change has since shifted it.
        let live_monitor = monitor(1, 1.0, IVec2::new(-3_440, 0));
        let position = unrebased(IVec2::new(-6_760, 80), 1.0);
        let prepared = prepare_persisted_position(position, UVec2::new(800, 600), &live_monitor);

        let target = target_for(position, &live_monitor, Platform::Windows);

        assert_eq!(prepared, SavedWindowPlacementDecision::DiscardedLegacy);
        assert_eq!(
            prepared.meaning(Platform::Windows),
            PreparedPositionMeaning::DiscardedLegacy
        );
        assert_eq!(
            target.physical_position(),
            ExpectedPhysicalPosition::DiscardedLegacy
        );
        assert_eq!(
            target.logical_position(),
            ExpectedLogicalPosition::DiscardedLegacy
        );
    }

    /// `Unpositioned` carries no coordinate on any platform.
    #[test]
    fn an_unpositioned_entry_yields_no_position() {
        let target_info = monitor(0, 2.0, IVec2::ZERO);
        let prepared = prepare_persisted_position(
            PersistedPosition::Unpositioned,
            UVec2::new(800, 600),
            &target_info,
        );

        let target = target_for(
            PersistedPosition::Unpositioned,
            &target_info,
            Platform::Windows,
        );

        assert_eq!(prepared, SavedWindowPlacementDecision::NotSaved);
        assert_eq!(
            prepared.meaning(Platform::Windows),
            PreparedPositionMeaning::NotSaved
        );
        assert_eq!(
            prepared.meaning(Platform::Wayland),
            PreparedPositionMeaning::PlatformCannotPosition
        );
        assert_eq!(
            target.physical_position(),
            ExpectedPhysicalPosition::NotSaved
        );
        assert_eq!(target.logical_position(), ExpectedLogicalPosition::NotSaved);
    }
    /// A window deliberately straddling a monitor boundary keeps its position. Its top-left
    /// corner sits on the *neighbouring* monitor while the window belongs to this one, which is
    /// exactly the case a corner test or a whole-rectangle test would wrongly discard.
    #[test]
    fn a_legacy_coordinate_straddling_the_boundary_is_kept_when_its_center_is_on_the_monitor() {
        // The layout the Linux `monitor_boundary_detection` fixture documents.
        let target_info = MonitorDescriptor::for_current_enumeration(
            1,
            1.0,
            IVec2::new(1_512, 2_880),
            UVec2::new(3_456, 2_160),
        );
        // Corner above the monitor's top edge; center (2000, 3000) just inside it.
        let corner = IVec2::new(1_600, 2_700);
        let position = unrebased(corner, 1.0);

        let target = target_for(position, &target_info, Platform::Windows);

        assert_eq!(
            target.physical_position(),
            ExpectedPhysicalPosition::Specified(corner)
        );
    }

    /// Overhanging the far edge is equally legitimate, for the same reason: the 800x600 window
    /// at this corner runs past both edges of the 2560x1440 monitor, while its center stays on it.
    #[test]
    fn a_legacy_coordinate_near_the_far_edge_is_kept_even_though_the_window_overhangs() {
        let target_info = monitor(0, 1.0, IVec2::ZERO);
        let corner = IVec2::new(2_000, 1_000);
        let position = unrebased(corner, 1.0);

        let target = target_for(position, &target_info, Platform::Windows);

        assert_eq!(
            target.physical_position(),
            ExpectedPhysicalPosition::Specified(corner)
        );
    }
}
