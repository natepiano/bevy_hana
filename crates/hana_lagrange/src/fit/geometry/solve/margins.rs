use crate::fit::constants::DEGENERATE_EXTENT_THRESHOLD;
use crate::fit::constants::HORIZONTAL_DIMENSION_LABEL;
use crate::fit::constants::VERTICAL_DIMENSION_LABEL;
use crate::fit::geometry::projection::ScreenSpaceBounds;

/// Returns the zoom margin multiplier (1.0 / (1.0 - margin)).
/// For example, a margin of 0.08 returns 1.087 (8% margin).
pub(super) const fn zoom_margin_multiplier(margin: f32) -> f32 { 1.0 / (1.0 - margin) }

/// Computes the target margins for the constraining dimension based on aspect ratios.
/// Returns `(target_margin_x, target_margin_y)`.
pub(super) const fn calculate_target_margins(
    bounds: &ScreenSpaceBounds,
    zoom_multiplier: f32,
) -> (f32, f32) {
    let horizontal_extent = bounds.max_normalized_x - bounds.min_normalized_x;
    let vertical_extent = bounds.max_normalized_y - bounds.min_normalized_y;

    // Guard against degenerate screen-space extents (edge-on flat objects).
    // When one dimension is near-zero, fit based on the non-degenerate dimension only.
    // Setting the target margin to the full half-extent ensures the degenerate
    // dimension never constrains the binary search.
    if vertical_extent < DEGENERATE_EXTENT_THRESHOLD {
        let target_x = bounds.half_extent_x / zoom_multiplier;
        (bounds.half_extent_x - target_x, bounds.half_extent_y)
    } else if horizontal_extent < DEGENERATE_EXTENT_THRESHOLD {
        let target_y = bounds.half_extent_y / zoom_multiplier;
        (bounds.half_extent_x, bounds.half_extent_y - target_y)
    } else {
        let boundary_aspect = horizontal_extent / vertical_extent;
        let screen_aspect = bounds.half_extent_x / bounds.half_extent_y;

        // If boundary is wider (relative to height) than screen, width constrains.
        let width_constrains = boundary_aspect > screen_aspect;

        let (target_edge_x, target_edge_y) = if width_constrains {
            let target_x = bounds.half_extent_x / zoom_multiplier;
            let target_y = target_x / boundary_aspect;
            (target_x, target_y)
        } else {
            let target_y = bounds.half_extent_y / zoom_multiplier;
            let target_x = target_y * boundary_aspect;
            (target_x, target_y)
        };

        (
            bounds.half_extent_x - target_edge_x,
            bounds.half_extent_y - target_edge_y,
        )
    }
}

/// Determines which screen dimension constrains the fit and returns the current margin,
/// target margin, and dimension label.
pub(super) const fn find_constraining_margin(
    bounds: &ScreenSpaceBounds,
    target_margin_x: f32,
    target_margin_y: f32,
) -> (f32, f32, &'static str) {
    let horizontal_min_margin = bounds.left_margin.min(bounds.right_margin);
    let vertical_min_margin = bounds.top_margin.min(bounds.bottom_margin);
    let vertical_extent = bounds.max_normalized_y - bounds.min_normalized_y;
    let horizontal_extent = bounds.max_normalized_x - bounds.min_normalized_x;

    if vertical_extent < DEGENERATE_EXTENT_THRESHOLD {
        (
            horizontal_min_margin,
            target_margin_x,
            HORIZONTAL_DIMENSION_LABEL,
        )
    } else if horizontal_extent < DEGENERATE_EXTENT_THRESHOLD {
        (
            vertical_min_margin,
            target_margin_y,
            VERTICAL_DIMENSION_LABEL,
        )
    } else if horizontal_min_margin < vertical_min_margin {
        (
            horizontal_min_margin,
            target_margin_x,
            HORIZONTAL_DIMENSION_LABEL,
        )
    } else {
        (
            vertical_min_margin,
            target_margin_y,
            VERTICAL_DIMENSION_LABEL,
        )
    }
}
