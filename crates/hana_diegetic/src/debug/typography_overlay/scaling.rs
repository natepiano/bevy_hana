use super::TypographyOverlay;
use crate::debug::constants::ARROW_GAP_RATIO;
use crate::debug::constants::ARROW_SPACING_RATIO;
use crate::debug::constants::ARROWHEAD_RATIO;
use crate::debug::constants::BBOX_MIN_WORLD_RATIO;
use crate::debug::constants::DOT_RADIUS_RATIO;
use crate::debug::constants::LABEL_GAP_RATIO;
use crate::debug::constants::METRIC_LINE_BORDER_MULT;
use crate::debug::constants::THIN_LINE_WIDTH;

/// Convert layout Y-down to world Y-up, with anchor offset.
pub(super) fn layout_to_world_y(layout_y: f32, anchor_y: f32, scale: f32) -> f32 {
    -(layout_y - anchor_y) * scale
}

/// Computes the uniform spacing between arrow columns from the first
/// glyph's advance width.
pub(super) const fn arrow_spacing(advance: f32) -> f32 { advance * ARROW_SPACING_RATIO }

/// Scale factor for converting font-size-relative ratios to world units.
pub(super) fn font_scale(font_size: f32, scale: f32) -> f32 { font_size * scale }

/// Dot radius in world units, scaled to the font size.
pub(super) fn dot_radius(font_size: f32, scale: f32) -> f32 {
    DOT_RADIUS_RATIO * font_scale(font_size, scale)
}

/// Arrowhead line length in world units, scaled to the font size.
pub(super) fn arrowhead_size(font_size: f32, scale: f32) -> f32 {
    ARROWHEAD_RATIO * font_scale(font_size, scale)
}

/// Arrow gap in world units, scaled to the font size.
pub(super) fn arrow_gap(font_size: f32, scale: f32) -> f32 {
    ARROW_GAP_RATIO * font_scale(font_size, scale)
}

/// Label gap in world units, scaled to the font size.
pub(super) fn label_gap(font_size: f32, scale: f32) -> f32 {
    LABEL_GAP_RATIO * font_scale(font_size, scale)
}

/// Border width for panel-backed glyph boxes in world units.
pub(super) fn bbox_border_width(overlay: &TypographyOverlay, font_size: f32, scale: f32) -> f32 {
    let min_world = font_scale(font_size, scale) * BBOX_MIN_WORLD_RATIO;
    let from_line_width = overlay.line_width.max(THIN_LINE_WIDTH) * min_world;
    from_line_width.max(min_world)
}

/// Thickness for panel-backed callout line segments in world units.
pub(super) fn callout_line_thickness(
    overlay: &TypographyOverlay,
    font_size: f32,
    scale: f32,
) -> f32 {
    bbox_border_width(overlay, font_size, scale)
}

/// Border width for panel-backed horizontal metric lines in world units.
pub(super) fn metric_line_border_width(
    overlay: &TypographyOverlay,
    font_size: f32,
    scale: f32,
) -> f32 {
    METRIC_LINE_BORDER_MULT * bbox_border_width(overlay, font_size, scale)
}
