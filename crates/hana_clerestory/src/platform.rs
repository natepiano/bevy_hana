//! Runtime platform detection.
//!
//! Every platform-specific branch in this crate goes through a method on
//! [`Platform`], so no call site writes its own `cfg!()` or `is_wayland()` check.
//!
//! On macOS and Windows the variant is known at compile time. On Linux the
//! binary can run under either Wayland or X11, so the variant is detected
//! at startup from the `WAYLAND_DISPLAY` environment variable.

#[cfg(target_os = "linux")]
use std::env::var;

use bevy::prelude::Resource;
use bevy::window::WindowMode;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::DeviceIdSource;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::Digest;
use hana_rigging::prelude::SchemeName;

use super::constants::SCALE_FACTOR_EPSILON;
#[cfg(target_os = "linux")]
use super::constants::WAYLAND_DISPLAY_ENVIRONMENT_VARIABLE;
use super::monitors::DisplayDeviceEvidence;
use super::monitors::DisplayIdentityEvidence;
use super::persistence::EstablishedWindowPosition;
use super::persistence::SavedWindowMode;
use super::reporter::DisplayKeyClassification;
use super::restore::FullscreenRestoreState;
use super::restore::MonitorScaleStrategy;
use super::restore::WindowRestoreState;

/// The display platform, detected once at startup and inserted as a [`Resource`].
///
/// Each method below answers one platform question the window restore path asks.
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

#[cfg(target_os = "macos")]
const CORE_GRAPHICS_SUCCESS: i32 = 0;

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
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

    /// The platform the crate's in-process fixtures run under.
    ///
    /// The fixtures assert on window placement, which needs a platform that
    /// reports window position. X11 is that platform, without the clamping
    /// macOS adds. Reading the developer's session through `detect()` made
    /// the same fixtures see `WindowPosition::Automatic` under a Wayland
    /// desktop, where no position is available.
    ///
    /// `ProductionPluginHarness` pins `MacOs` instead: its windowed restore
    /// under `workaround-winit-4445` would wait on the `_NET_FRAME_EXTENTS`
    /// reply that X11 frame compensation needs.
    #[cfg(test)]
    pub(crate) const FIXTURE: Self = Self::X11;

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

    /// Whether `target` and `actual` window modes count as a match during settle comparison.
    ///
    /// On Wayland, winit does not implement exclusive fullscreen and falls back to
    /// borderless fullscreen, so the settle comparison accepts that substitution.
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

    /// Whether restore preparation must wait for X11 frame compensation before placement.
    ///
    /// `compensate_target_position` subtracts the `_NET_FRAME_EXTENTS` top from the saved
    /// position and inserts the `X11FrameCompensated` token that gates
    /// `place_window_at_saved_geometry`. Only a windowed restore carries a title bar to
    /// subtract, so a fullscreen restore is marked compensated at preparation, as is every
    /// platform whose `outer_position()` already reports frame coordinates.
    #[must_use]
    pub(crate) const fn awaits_frame_compensation(
        self,
        saved_window_mode: &SavedWindowMode,
    ) -> bool {
        self.needs_frame_compensation() && !saved_window_mode.is_fullscreen()
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
    /// In all three cases the recorded `starting_scale` is not the scale of the monitor the
    /// window was created on, so initial restore preparation re-reads the window's actual
    /// `base_scale_factor()` and recomputes the strategy.
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

#[cfg(target_os = "macos")]
pub(crate) fn empty_current_display_list_error(observed_count: usize) -> DeviceAccessError {
    let mut active_display_count = 0;
    // SAFETY: a zero-capacity query writes only `display_count`; the active-display pointer is null
    // because no display identifiers are requested by this count query.
    let error_code =
        unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &raw mut active_display_count) };
    if error_code != CORE_GRAPHICS_SUCCESS {
        return discovery_transport_error(&format!(
            "CGGetActiveDisplayList failed with CGError code {error_code} while \
             {observed_count} observed displays remained"
        ));
    }

    discovery_transport_error(&format!(
        "winit returned no current display handles while CGGetActiveDisplayList reported \
         {active_display_count} active displays and {observed_count} observed displays remained"
    ))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn empty_current_display_list_error(observed_count: usize) -> DeviceAccessError {
    discovery_transport_error(&format!(
        "winit returned no current display handles while {observed_count} observed displays \
         remained"
    ))
}

/// Apply the reporter's durable display-key rules to one display's evidence.
#[cfg(all(target_os = "macos", not(test)))]
pub(crate) const fn classify_display_key(
    evidence: &DisplayDeviceEvidence,
    _: &SchemeName,
) -> DisplayKeyClassification {
    match &evidence.identity_evidence {
        DisplayIdentityEvidence::Synthesized {
            display_fingerprint,
            ..
        } => DisplayKeyClassification::Keyed(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(display_fingerprint.get()),
            },
        }),
        DisplayIdentityEvidence::Unavailable { .. } => DisplayKeyClassification::MatchEvidenceOnly,
    }
}

/// Apply the reporter's durable display-key rules to one display's evidence.
#[cfg(any(test, not(target_os = "macos")))]
pub(crate) fn classify_display_key(
    evidence: &DisplayDeviceEvidence,
    edid_serial_scheme: &SchemeName,
) -> DisplayKeyClassification {
    match &evidence.identity_evidence {
        #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
        DisplayIdentityEvidence::ReportedSerial(value) => {
            DisplayKeyClassification::Keyed(DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Reported {
                    scheme: edid_serial_scheme.clone(),
                    value:  value.clone(),
                },
            })
        },
        DisplayIdentityEvidence::Synthesized {
            display_fingerprint,
            ..
        } => DisplayKeyClassification::Keyed(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(display_fingerprint.get()),
            },
        }),
        DisplayIdentityEvidence::Unavailable { .. } => DisplayKeyClassification::MatchEvidenceOnly,
    }
}

fn discovery_transport_error(detail: &str) -> DeviceAccessError {
    DeviceAccessError::Transport {
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::IVec2;

    use super::*;
    use crate::persistence::SavedFullscreenVideoMode;

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
    fn only_a_windowed_restore_waits_for_x11_frame_compensation() {
        let fullscreen_modes = [
            SavedWindowMode::BorderlessFullscreen,
            SavedWindowMode::Fullscreen {
                video_mode: SavedFullscreenVideoMode::Current,
            },
        ];

        for platform in [
            Platform::MacOs,
            Platform::Windows,
            Platform::X11,
            Platform::Wayland,
        ] {
            // A fullscreen restore has no title bar to subtract, so nothing waits on the
            // `_NET_FRAME_EXTENTS` query.
            for saved_window_mode in &fullscreen_modes {
                assert!(!platform.awaits_frame_compensation(saved_window_mode));
            }
            // A windowed restore waits exactly when the platform compensates frames.
            assert_eq!(
                platform.awaits_frame_compensation(&SavedWindowMode::Windowed),
                platform.needs_frame_compensation(),
            );
        }

        // X11 is that platform whenever the workaround is compiled in.
        #[cfg(feature = "workaround-winit-4445")]
        assert!(Platform::X11.awaits_frame_compensation(&SavedWindowMode::Windowed));
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
