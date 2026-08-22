use bevy::prelude::Color;

// label constants
/// Color used for the "screen space bounds" label.
pub(super) const BOUNDS_LABEL_COLOR: Color = Color::srgb(1.0, 1.0, 0.0);
pub(super) const BOUNDS_LABEL_TEXT: &str = "screen space bounds";
/// Font size used for all debug labels.
pub(super) const LABEL_FONT_SIZE: f32 = 11.0;
/// Pixel offset used to keep labels off line endpoints and screen edges.
pub(super) const LABEL_PIXEL_OFFSET: f32 = 8.0;
/// UI stack order for fit-overlay labels.
pub(super) const OVERLAY_LABEL_Z_INDEX: i32 = 1_000_000;

// overlay constants
/// Default line width for retained fit-target overlay lines.
pub(super) const DEFAULT_OVERLAY_LINE_WIDTH: f32 = 2.0;
/// Color for balanced fit margins.
pub(super) const OVERLAY_BALANCED_COLOR: Color = Color::srgb(0.0, 1.0, 0.0);
/// Color for the screen-aligned fit rectangle.
pub(super) const OVERLAY_RECTANGLE_COLOR: Color = Color::srgb(1.0, 1.0, 0.0);
/// Color for the fit-target silhouette hull.
pub(super) const OVERLAY_SILHOUETTE_COLOR: Color = Color::srgb(1.0, 0.5, 0.0);
/// Color for unbalanced fit margins.
pub(super) const OVERLAY_UNBALANCED_COLOR: Color = Color::srgb(1.0, 0.0, 0.0);
/// Multiplier converting a fraction (0.0–1.0) into a percentage (0–100).
pub(super) const PERCENT_MULTIPLIER: f32 = 100.0;
