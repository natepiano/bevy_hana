use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy_kana::ScreenPosition;

use super::Edge;
use crate::fit::geometry::ProjectionBasis;
use crate::fit::geometry::ProjectionMode;
use crate::fit::geometry::ScreenSpaceBounds;
use crate::fit::overlay::constants::PERCENT_MULTIPLIER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::fit::overlay) enum MarginBalance {
    Balanced,
    Unbalanced,
}

/// Returns whether horizontal margins are balanced within the given tolerance.
pub(in crate::fit::overlay) const fn horizontal_balance(
    bounds: &ScreenSpaceBounds,
    tolerance: f32,
) -> MarginBalance {
    if (bounds.left_margin - bounds.right_margin).abs() < tolerance {
        MarginBalance::Balanced
    } else {
        MarginBalance::Unbalanced
    }
}

/// Returns whether vertical margins are balanced within the given tolerance.
pub(in crate::fit::overlay) const fn vertical_balance(
    bounds: &ScreenSpaceBounds,
    tolerance: f32,
) -> MarginBalance {
    if (bounds.top_margin - bounds.bottom_margin).abs() < tolerance {
        MarginBalance::Balanced
    } else {
        MarginBalance::Unbalanced
    }
}

/// Returns the screen edges in normalized space: (left, right, top, bottom).
const fn screen_edges_normalized(bounds: &ScreenSpaceBounds) -> (f32, f32, f32, f32) {
    (
        -bounds.half_extent_x,
        bounds.half_extent_x,
        bounds.half_extent_y,
        -bounds.half_extent_y,
    )
}

/// Returns the clamped vertical center of the bounds within the screen edges.
const fn clamped_center_y(bounds: &ScreenSpaceBounds, bottom_edge: f32, top_edge: f32) -> f32 {
    bounds
        .min_normalized_y
        .max(bottom_edge)
        .midpoint(bounds.max_normalized_y.min(top_edge))
}

/// Returns the clamped horizontal center of the bounds within the screen edges.
const fn clamped_center_x(bounds: &ScreenSpaceBounds, left_edge: f32, right_edge: f32) -> f32 {
    bounds
        .min_normalized_x
        .max(left_edge)
        .midpoint(bounds.max_normalized_x.min(right_edge))
}

/// Returns the center of a boundary edge in normalized space.
pub(in crate::fit::overlay) const fn boundary_edge_center(
    bounds: &ScreenSpaceBounds,
    edge: Edge,
) -> Option<(f32, f32)> {
    let (left_edge, right_edge, top_edge, bottom_edge) = screen_edges_normalized(bounds);

    match edge {
        Edge::Left if bounds.min_normalized_x > left_edge => Some((
            bounds.min_normalized_x,
            clamped_center_y(bounds, bottom_edge, top_edge),
        )),
        Edge::Right if bounds.max_normalized_x < right_edge => Some((
            bounds.max_normalized_x,
            clamped_center_y(bounds, bottom_edge, top_edge),
        )),
        Edge::Top if bounds.max_normalized_y < top_edge => Some((
            clamped_center_x(bounds, left_edge, right_edge),
            bounds.max_normalized_y,
        )),
        Edge::Bottom if bounds.min_normalized_y > bottom_edge => Some((
            clamped_center_x(bounds, left_edge, right_edge),
            bounds.min_normalized_y,
        )),
        _ => None,
    }
}

/// Returns the center of a screen edge in normalized space.
pub(in crate::fit::overlay) const fn screen_edge_center(
    bounds: &ScreenSpaceBounds,
    edge: Edge,
) -> (f32, f32) {
    let (left_edge, right_edge, top_edge, bottom_edge) = screen_edges_normalized(bounds);

    match edge {
        Edge::Left => (left_edge, clamped_center_y(bounds, bottom_edge, top_edge)),
        Edge::Right => (right_edge, clamped_center_y(bounds, bottom_edge, top_edge)),
        Edge::Top => (clamped_center_x(bounds, left_edge, right_edge), top_edge),
        Edge::Bottom => (clamped_center_x(bounds, left_edge, right_edge), bottom_edge),
    }
}

/// Converts normalized screen-space coordinates to world space.
///
/// For perspective, reverses the perspective divide by multiplying by `average_depth`.
/// For orthographic, coordinates are already in world units — `average_depth` is only
/// used for the forward component to position the gizmo plane.
pub(in crate::fit::overlay) fn normalized_to_world(
    normalized_x: f32,
    normalized_y: f32,
    camera: &ProjectionBasis,
    average_depth: f32,
    projection_mode: ProjectionMode,
) -> Vec3 {
    let (world_x, world_y) = match projection_mode {
        ProjectionMode::Orthographic => (normalized_x, normalized_y),
        ProjectionMode::Perspective => (normalized_x * average_depth, normalized_y * average_depth),
    };
    *camera.position + camera.right * world_x + camera.up * world_y + camera.forward * average_depth
}

/// Returns the margin percentage for a given edge.
/// Percentage represents how much of the screen width/height is margin.
pub(in crate::fit::overlay) const fn margin_percentage(
    bounds: &ScreenSpaceBounds,
    edge: Edge,
) -> f32 {
    let screen_width = 2.0 * bounds.half_extent_x;
    let screen_height = 2.0 * bounds.half_extent_y;

    match edge {
        Edge::Left => (bounds.left_margin / screen_width) * PERCENT_MULTIPLIER,
        Edge::Right => (bounds.right_margin / screen_width) * PERCENT_MULTIPLIER,
        Edge::Top => (bounds.top_margin / screen_height) * PERCENT_MULTIPLIER,
        Edge::Bottom => (bounds.bottom_margin / screen_height) * PERCENT_MULTIPLIER,
    }
}

/// Converts a normalized screen-space coordinate to viewport pixels.
pub(in crate::fit::overlay) fn norm_to_viewport(
    normalized_x: f32,
    normalized_y: f32,
    half_extent_x: f32,
    half_extent_y: f32,
    viewport_size: Vec2,
) -> ScreenPosition {
    ScreenPosition::new(
        f32::midpoint(normalized_x / half_extent_x, 1.0) * viewport_size.x,
        (1.0 - normalized_y / half_extent_y) * 0.5 * viewport_size.y,
    )
}
