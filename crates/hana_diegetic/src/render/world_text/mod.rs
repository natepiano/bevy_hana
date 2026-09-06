//! Shared text-entity components.
//!
//! Holds [`TextContent`] — the text-source component carried by every panel-text
//! run, including the single run of a one-element [`DiegeticText`](crate::DiegeticText)
//! label. `TextContent` is also the marker panel-text systems filter on
//! (`With<TextContent>`) to act on run entities; a panel root carries no
//! `TextContent`. All text renders through the panel-text pipeline; there is no
//! separate world-text render path.
//!
//! The readiness signal — [`WorldTextReady`] and the `AwaitingReady` marker —
//! also lives here, and the panel-text pipeline drives it.

#[cfg(feature = "typography_overlay")]
mod overlay_metrics;
mod readiness;

use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::Transform;
use bevy::prelude::Visibility;
#[cfg(feature = "typography_overlay")]
pub(crate) use overlay_metrics::emit_computed_world_text;
pub(crate) use readiness::AwaitingReady;
pub use readiness::WorldTextReady;
pub(crate) use readiness::emit_world_text_ready;

use crate::layout::ShapedLineMetrics;
use crate::layout::TextStyle;

/// Computed layout data for a [`TextContent`] entity, read by the typography
/// debug overlay to draw glyph bounding boxes and metric lines aligned with the
/// rendered text.
///
/// Only available when the `typography_overlay` feature is enabled.
///
/// Populated by
/// [`emit_computed_world_text`](crate::render::emit_computed_world_text) from the
/// panel child's [`PanelTextLayout`](crate::render::PanelTextLayout), so every
/// field is in the same coordinate system the panel-text mesh was built in: the
/// font size and line metrics are in layout points, `scale` converts those points
/// to world meters, and the glyph rects are already in world meters. The overlay
/// reads `scale` / `font_size` / `line_metrics` directly rather than recomputing
/// them, which keeps its boxes and metric lines on the rendered glyphs.
#[cfg(feature = "typography_overlay")]
#[derive(Component, Clone, Debug)]
pub struct ComputedWorldText {
    /// `Anchor` offset Y in layout points (the panel's layout-local anchor).
    pub anchor_y:     f32,
    /// Layout-points-to-world-meters scale (the panel's `points_to_world`).
    pub scale:        f32,
    /// Font size in layout points, matching the panel-text run.
    pub font_size:    f32,
    /// Font identifier of the run, so the overlay draws the rendered font's
    /// metric lines and name rather than reading the panel root's `TextStyle`.
    pub font_id:      u16,
    /// First-line metrics from the same shaping pass, in layout points.
    pub line_metrics: ShapedLineMetrics,
    /// Per-visible-glyph metrics aligned with the rendered text.
    pub glyphs:       Vec<ComputedGlyphMetrics>,
}

/// Overlay-only metrics for one visible glyph in a [`TextContent`] run.
#[cfg(feature = "typography_overlay")]
#[derive(Clone, Debug)]
pub struct ComputedGlyphMetrics {
    /// Ink bounding box `[x, y, width, height]` in world units.
    pub rect:      [f32; 4],
    /// Glyph origin X for overlay callouts, in world units.
    ///
    /// Usually the laid-out glyph origin. When a substituted glyph draws before
    /// that origin, as `JetBrains` Mono coding alternates do, this shifts left to
    /// the visible overhang so the origin/advance callout tracks the displayed
    /// glyph cell.
    pub origin_x:  f32,
    /// Laid-out horizontal advance in world units.
    pub advance_x: f32,
}

/// The per-run text string for a text entity.
///
/// Carried by every panel-text run, including the single run a one-element
/// [`DiegeticText`](crate::DiegeticText) label spawns. For a panel run this is
/// derived output: reification rewrites it from the panel's authoritative `El.text`
/// tree, and shaping reads it. To change a run's string at runtime, write the
/// tree through [`PanelText`](crate::PanelText) / [`DiegeticTextMut`](crate::DiegeticTextMut),
/// not this component — a direct edit is overwritten on the next reification pass.
/// Its presence also marks an entity as a panel-text run (`With<TextContent>`).
/// Style is controlled by the required [`TextStyle`](crate::TextStyle) component
/// (added with defaults if not specified).
#[derive(Component, Clone, Debug, Reflect)]
#[require(TextStyle, Transform, Visibility)]
pub struct TextContent {
    text: String,
}

impl TextContent {
    /// Creates new text content with the given string.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self { Self { text: text.into() } }

    /// Returns the run's string.
    #[must_use]
    pub fn text(&self) -> &str { &self.text }

    /// Replaces the run's string.
    pub fn set_text(&mut self, text: impl Into<String>) { self.text = text.into(); }
}
