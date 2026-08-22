//! Runtime platform detection.
//!
//! Consolidates all platform-specific behavior branching into a single enum
//! with methods, replacing scattered `cfg!()` and `is_wayland()` checks.
//!
//! On macOS and Windows the variant is known at compile time. On Linux the
//! binary can run under either Wayland or X11, so the variant is detected
//! at startup from the `WAYLAND_DISPLAY` environment variable.

#[cfg(target_os = "linux")]
use std::env::var;

use bevy::prelude::Resource;
use bevy::window::WindowMode;

use super::constants::SCALE_FACTOR_EPSILON;
#[cfg(target_os = "linux")]
use super::constants::WAYLAND_DISPLAY_ENVIRONMENT_VARIABLE;
use super::persistence::EstablishedWindowPosition;
use super::persistence::SavedWindowMode;
use super::restore::FullscreenRestoreState;
use super::restore::MonitorScaleStrategy;
use super::restore::WindowRestoreState;

/// The display platform, detected once at startup and inserted as a [`Resource`].
///
/// All platform-specific window restoration behavior is expressed as methods on
/// this enum rather than ad-hoc `cfg!()` / `is_wayland()` checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Resource)]
pub enum Platform {
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
    /// Linux running an X11 session.
    X11,
    /// Linux running a Wayland session.
    Wayland,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReturnCapability {
    Supported,
    Unsupported,
}

impl Platform {
    /// Detect the current platform.
    ///
    /// On Linux, checks `WAYLAND_DISPLAY` to distinguish Wayland from X11.
    /// On macOS and Windows the result is compile-time constant.
    #[must_use]
    #[cfg(target_os = "macos")]
    pub const fn detect() -> Self { Self::MacOs }

    /// Detect the current platform.
    ///
    /// On Linux, checks `WAYLAND_DISPLAY` to distinguish Wayland from X11.
    /// On macOS and Windows the result is compile-time constant.
    #[must_use]
    #[cfg(target_os = "windows")]
    pub const fn detect() -> Self { Self::Windows }

    /// Detect the current platform.
    ///
    /// On Linux, checks `WAYLAND_DISPLAY` to distinguish Wayland from X11.
    /// On macOS and Windows the result is compile-time constant.
    #[must_use]
    #[cfg(target_os = "linux")]
    pub fn detect() -> Self {
        if var(WAYLAND_DISPLAY_ENVIRONMENT_VARIABLE).is_ok_and(|value| !value.is_empty()) {
            Self::Wayland
        } else {
            Self::X11
        }
    }

    /// Detect the current platform.
    ///
    /// On Linux, checks `WAYLAND_DISPLAY` to distinguish Wayland from X11.
    /// On macOS and Windows the result is compile-time constant.
    #[must_use]
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    pub fn detect() -> Self { compile_error!("Unsupported platform") }

    /// Whether this is the Linux X11 platform.
    #[must_use]
    pub const fn is_x11(self) -> bool { matches!(self, Self::X11) }

    /// Whether this is the Linux Wayland platform.
    #[must_use]
    pub const fn is_wayland(self) -> bool { matches!(self, Self::Wayland) }

    /// Whether window position is available from the windowing system.
    ///
    /// Wayland does not expose window position to clients (`wl_surface` has no
    /// position API, and winit returns `(0, 0)`). All other platforms provide it.
    #[must_use]
    pub const fn position_available(self) -> bool { !matches!(self, Self::Wayland) }

    /// Whether an automatic return can reproduce the captured placement.
    #[must_use]
    pub(crate) const fn fallback_return_capability(
        self,
        position: EstablishedWindowPosition,
        saved_window_mode: &SavedWindowMode,
    ) -> ReturnCapability {
        match (self, saved_window_mode, position) {
            (_, SavedWindowMode::Fullscreen { .. }, _) => ReturnCapability::Unsupported,
            (_, SavedWindowMode::Windowed, EstablishedWindowPosition::Restorable { .. }) => {
                ReturnCapability::Supported
            },
            (_, SavedWindowMode::Windowed, EstablishedWindowPosition::CompositorControlled) => {
                ReturnCapability::Unsupported
            },
            (
                Self::MacOs | Self::Windows | Self::X11 | Self::Wayland,
                SavedWindowMode::BorderlessFullscreen,
                _,
            ) => ReturnCapability::Supported,
        }
    }

    /// Whether the given target and actual window modes should be considered a match
    /// during settle comparison.
    ///
    /// On Wayland, exclusive fullscreen is not supported by winit and falls back to
    /// borderless fullscreen. The settle system must accept this substitution.
    #[must_use]
    pub fn modes_match(self, target: WindowMode, actual: WindowMode) -> bool {
        target == actual
            || (matches!(self, Self::Wayland)
                && matches!(target, WindowMode::Fullscreen(..))
                && matches!(actual, WindowMode::BorderlessFullscreen(..)))
    }

    /// Whether the primary window should be hidden on startup to prevent a flash
    /// at the default position before restore completes.
    ///
    /// On Linux X11 with frame extent compensation (`workaround-winit-4445`),
    /// the window must stay visible so `_NET_FRAME_EXTENTS` can be queried.
    /// All other platforms hide the window.
    #[must_use]
    pub const fn should_hide_on_startup(self) -> bool {
        #[cfg(feature = "workaround-winit-4445")]
        {
            // `Platform::X11` keeps the window visible so `_NET_FRAME_EXTENTS` can be queried.
            !matches!(self, Self::X11)
        }
        #[cfg(not(feature = "workaround-winit-4445"))]
        {
            true
        }
    }

    /// Whether X11 frame extent compensation is needed.
    ///
    /// Only applies to Linux X11 with the `workaround-winit-4445` feature,
    /// where `outer_position()` is offset by the title bar height.
    #[must_use]
    pub const fn needs_frame_compensation(self) -> bool {
        #[cfg(feature = "workaround-winit-4445")]
        {
            matches!(self, Self::X11)
        }
        #[cfg(not(feature = "workaround-winit-4445"))]
        {
            false
        }
    }

    /// Whether position readback is reliable for settle comparison.
    ///
    /// On X11 with `workaround-winit-4445`, the target position is in frame
    /// coordinates (compensated by `frame_top`), but `Window.position` reports
    /// the client area position (the W6 bug). The two reference frames differ
    /// by exactly the title bar height, so position comparison always fails.
    /// Other platforms have consistent position readback.
    #[must_use]
    pub const fn position_reliable_for_settle(self) -> bool { !self.needs_frame_compensation() }

    /// Whether saved position should be clamped to monitor bounds.
    ///
    /// macOS clamps because it may resize/reposition windows that extend beyond
    /// the screen and does not allow windows to span monitors. All other platforms
    /// preserve the exact saved position.
    #[must_use]
    pub const fn should_clamp_position(self) -> bool { matches!(self, Self::MacOs) }

    /// Whether exclusive fullscreen should fall back to borderless.
    ///
    /// On Wayland, winit ignores exclusive fullscreen requests, so the library
    /// restores as borderless fullscreen instead.
    #[must_use]
    pub const fn exclusive_fullscreen_fallback(self) -> bool { matches!(self, Self::Wayland) }

    /// Determine the fullscreen restore state for cross-monitor fullscreen restore.
    ///
    /// - **Windows** (with `workaround-winit-3124`): `WaitForSurface` — DX12 exclusive fullscreen
    ///   needs the surface to be ready.
    /// - **X11**: `MoveToMonitor` — compositor needs time to process position before fullscreen
    ///   mode is applied.
    /// - **macOS**: `LeaveFullscreen` — leave any existing fullscreen Space before entering
    ///   fullscreen on the target monitor.
    /// - **Wayland**: `ApplyMode` — apply fullscreen directly.
    #[must_use]
    pub(crate) const fn fullscreen_restore_state(self) -> FullscreenRestoreState {
        #[cfg(feature = "workaround-winit-3124")]
        if matches!(self, Self::Windows) {
            return FullscreenRestoreState::WaitForSurface;
        }
        match self {
            Self::MacOs => FullscreenRestoreState::LeaveFullscreen,
            Self::X11 => FullscreenRestoreState::MoveToMonitor,
            Self::Windows | Self::Wayland => FullscreenRestoreState::ApplyMode,
        }
    }

    /// Determine the monitor scale strategy for cross-DPI window restore.
    ///
    /// - Without `workaround-winit-4440`: always `ApplyUnchanged`.
    /// - **Wayland**: handles DPI natively → `ApplyUnchanged`.
    /// - **Same scale**: no cross-DPI issue → `ApplyUnchanged`.
    /// - **Windows**: position unaffected, size goes through scale conversion →
    ///   `CompensateSizeOnly` with two-phase approach.
    /// - **macOS / X11**: both position and size affected → `LowerToHigher` or `HigherToLower`
    ///   depending on scale direction.
    #[must_use]
    pub(crate) fn scale_strategy(
        self,
        starting_scale: f64,
        target_scale: f64,
    ) -> MonitorScaleStrategy {
        if !cfg!(feature = "workaround-winit-4440") {
            return MonitorScaleStrategy::ApplyUnchanged;
        }

        if matches!(self, Self::Wayland) {
            return MonitorScaleStrategy::ApplyUnchanged;
        }

        if (starting_scale - target_scale).abs() < SCALE_FACTOR_EPSILON {
            MonitorScaleStrategy::ApplyUnchanged
        } else if matches!(self, Self::Windows) {
            MonitorScaleStrategy::CompensateSizeOnly(WindowRestoreState::NeedInitialMove)
        } else if starting_scale < target_scale {
            MonitorScaleStrategy::LowerToHigher
        } else {
            MonitorScaleStrategy::HigherToLower(WindowRestoreState::NeedInitialMove)
        }
    }

    /// Whether managed windows need scale strategy recalculation on creation.
    ///
    /// A managed (secondary) window is created on the monitor where the focused
    /// window currently is, not necessarily the monitor whose scale was sampled
    /// when its `TargetPosition` was computed:
    ///
    /// - **Windows**: new windows may be placed on the OS primary display rather than the monitor
    ///   where the parent/launching window is.
    /// - **macOS / X11**: the primary window migrates to its own restore target during startup, so
    ///   a secondary window spawned alongside it is born on the primary's post-move monitor — a
    ///   different scale than the primary's launch monitor recorded by `RestoreTargetBuilder` for
    ///   `TargetPosition::starting_scale`.
    ///
    /// In all three cases the `starting_scale` assumption is wrong, so initial restore
    /// preparation re-reads the window's actual `base_scale_factor()` and recomputes the strategy.
    /// Once a two-phase strategy advances beyond `NeedInitialMove`, its stored starting scale and
    /// strategy are retained until that restore finishes.
    ///
    /// Enabled for every non-Wayland platform. The recompute only runs when the
    /// measured scale differs from `starting_scale`, so on setups where all
    /// monitors share one scale factor (the common X11 case, where winit derives
    /// a single global scale from `Xft.dpi`) the branch is unreachable and this is
    /// a no-op. Wayland is excluded: it handles DPI natively and exposes no window
    /// position, so the cross-DPI strategies never apply there.
    #[must_use]
    pub const fn needs_managed_scale_fixup(self) -> bool {
        matches!(self, Self::Windows | Self::MacOs | Self::X11)
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::IVec2;

    use super::*;

    #[test]
    fn automatic_return_rejects_exclusive_fullscreen_on_every_platform() {
        let restorable = EstablishedWindowPosition::Restorable {
            logical_offset: IVec2::ZERO,
        };
        let positions = [restorable, EstablishedWindowPosition::CompositorControlled];

        for platform in [
            Platform::MacOs,
            Platform::Windows,
            Platform::X11,
            Platform::Wayland,
        ] {
            for position in positions {
                assert_eq!(
                    platform.fallback_return_capability(
                        position,
                        &SavedWindowMode::Fullscreen {
                            video_mode: crate::persistence::SavedFullscreenVideoMode::Current,
                        },
                    ),
                    ReturnCapability::Unsupported,
                );
            }
        }
    }

    #[test]
    fn automatic_windowed_return_requires_a_restorable_position() {
        for platform in [
            Platform::MacOs,
            Platform::Windows,
            Platform::X11,
            Platform::Wayland,
        ] {
            assert_eq!(
                platform.fallback_return_capability(
                    EstablishedWindowPosition::Restorable {
                        logical_offset: IVec2::ZERO,
                    },
                    &SavedWindowMode::Windowed,
                ),
                ReturnCapability::Supported,
            );
            assert_eq!(
                platform.fallback_return_capability(
                    EstablishedWindowPosition::CompositorControlled,
                    &SavedWindowMode::Windowed,
                ),
                ReturnCapability::Unsupported,
            );
        }
    }

    #[test]
    fn automatic_borderless_return_remains_monitor_targeted() {
        let restorable = EstablishedWindowPosition::Restorable {
            logical_offset: IVec2::ZERO,
        };
        let positions = [restorable, EstablishedWindowPosition::CompositorControlled];

        for platform in [
            Platform::MacOs,
            Platform::Windows,
            Platform::X11,
            Platform::Wayland,
        ] {
            for position in positions {
                assert_eq!(
                    platform.fallback_return_capability(
                        position,
                        &SavedWindowMode::BorderlessFullscreen,
                    ),
                    ReturnCapability::Supported,
                );
            }
        }
    }
}
