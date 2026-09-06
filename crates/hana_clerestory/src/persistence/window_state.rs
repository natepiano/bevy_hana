//! Saved window state types for persistence serialization.

#![allow(
    clippy::used_underscore_binding,
    reason = "false positive on enum variant fields"
)]

use std::ops::Deref;

use bevy::prelude::IVec2;
use bevy::prelude::Reflect;
use bevy::prelude::UVec2;
use bevy::window::VideoMode;
use bevy::window::VideoModeSelection;
use bevy::window::WindowMode;
use hana_rigging::prelude::BindingPolicy;
use hana_rigging::prelude::DeviceKey;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::constants::DEFAULT_SCALE_FACTOR;
use crate::monitors::CurrentMonitorIndex;
use crate::monitors::DisplayFingerprint;
use crate::monitors::DisplayIdentity;

/// Saved video mode for exclusive fullscreen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub(crate) struct SavedVideoMode {
    pub(super) physical_size:           UVec2,
    pub(super) bit_depth:               u16,
    pub(super) refresh_rate_millihertz: u32,
}

impl SavedVideoMode {
    /// Convert to Bevy's `VideoMode`.
    #[must_use]
    const fn to_video_mode(&self) -> VideoMode {
        VideoMode {
            physical_size:           self.physical_size,
            bit_depth:               self.bit_depth,
            refresh_rate_millihertz: self.refresh_rate_millihertz,
        }
    }
}

/// Serializable exclusive-fullscreen video-mode choice.
///
/// A saved exclusive fullscreen window either follows the display's current mode or requests the
/// exact mode that was active when Clerestory observed it. A bare optional mode would make those
/// two instructions indistinguishable at a persistence boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub(crate) enum SavedFullscreenVideoMode {
    /// Re-enter exclusive fullscreen using the target display's current video mode.
    Current,
    /// Re-enter exclusive fullscreen using this exact observed video mode.
    Specific(SavedVideoMode),
}

/// Serializable window mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub(crate) enum SavedWindowMode {
    Windowed,
    BorderlessFullscreen,
    /// Exclusive fullscreen with a named current-or-specific video-mode request.
    Fullscreen {
        /// Video-mode instruction that does not rely on an implicit absent value.
        video_mode: SavedFullscreenVideoMode,
    },
}

impl SavedWindowMode {
    /// Convert to Bevy's `WindowMode` with the given monitor index.
    #[must_use]
    pub(crate) const fn to_window_mode(&self, monitor_index: CurrentMonitorIndex) -> WindowMode {
        let monitor_selection = monitor_index.selection();
        match self {
            Self::Windowed => WindowMode::Windowed,
            Self::BorderlessFullscreen => WindowMode::BorderlessFullscreen(monitor_selection),
            Self::Fullscreen {
                video_mode: SavedFullscreenVideoMode::Current,
            } => WindowMode::Fullscreen(monitor_selection, VideoModeSelection::Current),
            Self::Fullscreen {
                video_mode: SavedFullscreenVideoMode::Specific(saved),
            } => WindowMode::Fullscreen(
                monitor_selection,
                VideoModeSelection::Specific(saved.to_video_mode()),
            ),
        }
    }

    /// Check if this is a fullscreen mode (borderless or exclusive).
    #[must_use]
    pub(crate) const fn is_fullscreen(&self) -> bool { !matches!(self, Self::Windowed) }
}

impl From<&WindowMode> for SavedWindowMode {
    fn from(mode: &WindowMode) -> Self {
        match mode {
            WindowMode::Windowed => Self::Windowed,
            WindowMode::BorderlessFullscreen(_) => Self::BorderlessFullscreen,
            WindowMode::Fullscreen(_, video_mode_selection) => Self::Fullscreen {
                video_mode: match video_mode_selection {
                    VideoModeSelection::Current => SavedFullscreenVideoMode::Current,
                    VideoModeSelection::Specific(mode) => {
                        SavedFullscreenVideoMode::Specific(SavedVideoMode {
                            physical_size:           mode.physical_size,
                            bit_depth:               mode.bit_depth,
                            refresh_rate_millihertz: mode.refresh_rate_millihertz,
                        })
                    },
                },
            },
        }
    }
}

/// Where a persisted window should be placed.
///
/// A monitor's logical origin is a function of the scale in force when a coordinate was
/// written, so an absolute logical desktop coordinate silently relocates the window when any
/// monitor's scale changes between save and restore. `MonitorOffset` is measured from the
/// monitor's own corner and is therefore scale-independent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) enum PersistedPosition {
    /// Logical-pixel offset of the window's top-left corner from its monitor's top-left corner.
    MonitorOffset(IVec2),
    /// A pre-v3 absolute desktop coordinate that has not been rebased onto a monitor yet.
    Unrebased(UnrebasedDesktopPosition),
    /// Nothing usable was saved: the platform withholds window position (Wayland), the window
    /// was compositor-placed, or a saved coordinate was rejected as no longer plausible.
    Unpositioned,
}

/// A pre-v3 absolute logical desktop coordinate paired with the monitor scale that wrote it.
///
/// The two are meaningless apart — reconstructing where the window actually sat requires the
/// scale in force at save time, not the live one. Kept in this form until a live monitor layout
/// is available to rebase against, and re-serialized unchanged until then so every launch
/// rebases from the same pair instead of freezing one layout's approximation into the file.
///
/// Construction is confined to [`UnrebasedDesktopPosition::from_legacy`]: the fields are private,
/// so no other module can write the struct literal, and `Deserialize` is routed through
/// `UnrebasedWire` so a hand-edited or corrupt file cannot bypass validation either.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "UnrebasedWire", into = "UnrebasedWire")]
pub(crate) struct UnrebasedDesktopPosition {
    logical:        IVec2,
    captured_scale: f64,
}

impl UnrebasedDesktopPosition {
    /// Sole constructor. Rejects a `captured_scale` that is not finite and greater than zero.
    ///
    /// Every consumer divides or multiplies by this value, and `ToI32` saturates rather than
    /// failing: `0.0` yields `±inf`/`NaN` and lands the window at `i32::MIN` or the origin, and
    /// a negative scale mirrors the coordinate across the monitor corner.
    pub(super) fn from_legacy(logical: IVec2, captured_scale: f64) -> Option<Self> {
        (captured_scale.is_finite() && captured_scale > 0.0).then_some(Self {
            logical,
            captured_scale,
        })
    }

    /// Build a legacy coordinate from outside `persistence`, for tests that exercise the rebase.
    /// Routes through `from_legacy`, so it validates exactly as decode does.
    #[cfg(test)]
    pub(crate) fn from_test_legacy(logical: IVec2, captured_scale: f64) -> Option<Self> {
        Self::from_legacy(logical, captured_scale)
    }

    /// The absolute logical desktop coordinate as written.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn logical(self) -> IVec2 { self.logical }

    /// The monitor scale in force when the coordinate was written.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn captured_scale(self) -> f64 { self.captured_scale }
}

/// Wire form of [`UnrebasedDesktopPosition`]. Exists so the derived `Deserialize` cannot act as a
/// second, unvalidated constructor — serde ignores field privacy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct UnrebasedWire {
    logical:        IVec2,
    #[serde(default = "default_monitor_scale")]
    captured_scale: f64,
}

/// Rejected `captured_scale` from a persisted `Unrebased` entry.
#[derive(Debug, Error)]
#[error("persisted captured_scale {0} is not a finite number greater than zero")]
pub(crate) struct InvalidCapturedScale(f64);

impl TryFrom<UnrebasedWire> for UnrebasedDesktopPosition {
    type Error = InvalidCapturedScale;

    fn try_from(wire: UnrebasedWire) -> Result<Self, Self::Error> {
        Self::from_legacy(wire.logical, wire.captured_scale)
            .ok_or(InvalidCapturedScale(wire.captured_scale))
    }
}

impl From<UnrebasedDesktopPosition> for UnrebasedWire {
    fn from(unrebased: UnrebasedDesktopPosition) -> Self {
        Self {
            logical:        unrebased.logical,
            captured_scale: unrebased.captured_scale,
        }
    }
}

/// Frozen v4 display identity retained only for decoding old files and unresolved v5 targets.
///
/// v4 stored a private fingerprint but could not state whether the evidence was reporter-issued
/// or synthesized. Keeping that distinction out of this wire adapter prevents an old file from
/// producing a stronger key than the fresh reporter scan supports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PersistedDisplayIdentityV4 {
    /// A v4 fingerprint that can be compared with fresh reporter evidence.
    Fingerprinted(PersistedDisplayFingerprintV4),
    /// A v4 record that never carried durable display evidence.
    #[default]
    Anonymous,
}

impl From<PersistedDisplayIdentityV4> for DisplayIdentity {
    fn from(persisted: PersistedDisplayIdentityV4) -> Self {
        match persisted {
            PersistedDisplayIdentityV4::Fingerprinted(fingerprint) => {
                Self::Fingerprinted(DisplayFingerprint::from_digest(fingerprint.0))
            },
            PersistedDisplayIdentityV4::Anonymous => Self::Anonymous,
        }
    }
}

/// Frozen v4 fingerprint wrapper preserving the RON tuple-newtype representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistedDisplayFingerprintV4(pub(crate) u64);

/// Durable target retained by the v5 persistence format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PersistedWindowTargetV5 {
    /// Fresh reporter evidence already classified the physical display as this exact kernel key.
    Classified(DeviceKey),
    /// A legacy file had no current exact evidence and must wait for a later reporter association.
    AwaitingLegacyEvidence(PersistedDisplayIdentityV4),
}

/// Saved window state persisted by the live v5 format.
///
/// Sizes are in **logical pixels** — the size the window covers on screen as the desktop numbers
/// it, independent of scale factor. Restore converts them to physical pixels using the target
/// monitor's live scale. Position is scale-independent by construction; see
/// [`PersistedPosition`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PersistedWindowState {
    /// Exact display authority, or a retained v4 target that needs fresh reporter evidence.
    pub(crate) target:            PersistedWindowTargetV5,
    /// Placement of the window relative to the target display.
    pub(crate) position:          PersistedPosition,
    /// Content area width in logical pixels (excludes window decoration).
    pub(crate) logical_width:     u32,
    /// Content area height in logical pixels (excludes window decoration).
    pub(crate) logical_height:    u32,
    #[serde(rename = "mode")]
    pub(crate) saved_window_mode: SavedWindowMode,
    #[serde(default)]
    pub(crate) app_name:          String,
}

/// Policy source carried by one loaded window entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadedBindingPolicy {
    /// The current save format recorded every authored policy field.
    Saved(BindingPolicy),
    /// A v5 or older entry relies on the window's existing recovery markers and the historical
    /// defaults for every other policy field.
    LegacyWindowMarkers,
}

/// One saved window placement together with the policy that governs its kernel binding.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PersistedWindowPlacement {
    pub(crate) window_state:          PersistedWindowState,
    pub(crate) loaded_binding_policy: LoadedBindingPolicy,
}

impl PersistedWindowPlacement {
    /// Pair a decoded or projected window state with its policy source.
    #[must_use]
    pub(crate) const fn new(
        window_state: PersistedWindowState,
        loaded_binding_policy: LoadedBindingPolicy,
    ) -> Self {
        Self {
            window_state,
            loaded_binding_policy,
        }
    }
}

impl Deref for PersistedWindowPlacement {
    type Target = PersistedWindowState;

    fn deref(&self) -> &Self::Target { &self.window_state }
}

impl From<PersistedWindowState> for PersistedWindowPlacement {
    fn from(window_state: PersistedWindowState) -> Self {
        Self::new(window_state, LoadedBindingPolicy::LegacyWindowMarkers)
    }
}

/// Result of reading one role from `PersistedWindowPlacements`.
pub(crate) enum PersistedWindowPlacementLookup<'a> {
    /// The save file contains this placement and policy source.
    Saved(&'a PersistedWindowPlacement),
    /// The role has no saved window placement.
    NotSaved,
}

#[cfg(test)]
impl PersistedWindowPlacementLookup<'_> {
    #[must_use]
    pub(crate) const fn is_saved(&self) -> bool { matches!(self, Self::Saved(_)) }

    #[must_use]
    pub(crate) const fn is_not_saved(&self) -> bool { matches!(self, Self::NotSaved) }
}

/// Default monitor scale for deserialization of legacy files missing the field.
pub(super) const fn default_monitor_scale() -> f64 { DEFAULT_SCALE_FACTOR }
