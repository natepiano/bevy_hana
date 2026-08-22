use bevy::prelude::GlobalTransform;
use bevy::prelude::Projection;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;

use super::radius_search::FitParameters;
use crate::fit::constants::CENTERING_MAX_ITERATIONS;
use crate::fit::constants::CENTERING_TOLERANCE;
use crate::fit::geometry::anchor::FitAnchor;
use crate::fit::geometry::projection::ProjectionMode;
use crate::fit::geometry::projection::ScreenSpaceBounds;

pub(super) fn viewport_can_map_pixels(viewport_size: Option<Vec2>) -> bool {
    viewport_size.is_some_and(|size| size.x > f32::EPSILON && size.y > f32::EPSILON)
}

/// Shifts the camera focus so the projected bounding box is centered on screen.
///
/// For perspective, each correction step uses the harmonic mean of the depths of the two
/// extreme points per dimension. This is the exact inverse of perspective projection.
///
/// For orthographic, centering is depth-independent (`centering_depth` = 1.0), so the shift
/// is a direct 1:1 world-unit correction.
pub(super) fn refine_focus_centering(
    points: &[Vec3],
    initial_focus: Vec3,
    radius: f32,
    projection: &Projection,
    parameters: &FitParameters,
) -> Vec3 {
    refine_focus_anchoring(
        points,
        initial_focus,
        radius,
        projection,
        parameters,
        FitAnchor::Center,
        Vec2::ZERO,
    )
}

/// Shifts the camera focus so the selected projected bounds anchor lands on
/// the matching viewport anchor.
///
/// The final `offset_px` uses screen coordinates: positive x moves right,
/// positive y moves down.
pub(super) fn refine_focus_anchoring(
    points: &[Vec3],
    initial_focus: Vec3,
    radius: f32,
    projection: &Projection,
    parameters: &FitParameters,
    anchor: FitAnchor,
    offset_px: Vec2,
) -> Vec3 {
    let rotation = parameters.rotation;
    let aspect_ratio = parameters.aspect_ratio;
    let orthographic_fixed_distance = parameters.orthographic_fixed_distance;
    let projection_mode = parameters.projection_mode;
    let camera_right = rotation * Vec3::X;
    let camera_up = rotation * Vec3::Y;

    let camera_distance = orthographic_fixed_distance.unwrap_or(radius);

    let mut focus = initial_focus;
    for _ in 0..CENTERING_MAX_ITERATIONS {
        let camera_position = focus + rotation * Vec3::new(0.0, 0.0, camera_distance);
        let camera_global = GlobalTransform::from(
            Transform::from_translation(camera_position).with_rotation(rotation),
        );
        let Some((bounds, depths)) =
            ScreenSpaceBounds::from_points(points, &camera_global, projection, aspect_ratio)
        else {
            break;
        };
        let current_anchor = bounds_anchor_point(&bounds, anchor);
        let target_anchor =
            viewport_anchor_point(&bounds, parameters.viewport_size, anchor, offset_px);
        let delta = current_anchor - target_anchor;
        if delta.x.abs() < CENTERING_TOLERANCE && delta.y.abs() < CENTERING_TOLERANCE {
            break;
        }

        // Lateral correction depths: perspective uses harmonic mean for
        // perspective-correct placement. Ortho uses 1.0 since projection is
        // depth-independent.
        let (correction_depth_x, correction_depth_y) = match projection_mode {
            ProjectionMode::Orthographic => (1.0, 1.0),
            ProjectionMode::Perspective => (
                2.0 * depths.min_x * depths.max_x / (depths.min_x + depths.max_x),
                2.0 * depths.min_y * depths.max_y / (depths.min_y + depths.max_y),
            ),
        };

        focus +=
            camera_right * delta.x * correction_depth_x + camera_up * delta.y * correction_depth_y;
    }
    focus
}

fn bounds_anchor_point(bounds: &ScreenSpaceBounds, anchor: FitAnchor) -> Vec2 {
    let (anchor_x, anchor_y) = anchor.offset_fraction();
    let x = (bounds.max_normalized_x - bounds.min_normalized_x)
        .mul_add(anchor_x, bounds.min_normalized_x);
    let y = (bounds.min_normalized_y - bounds.max_normalized_y)
        .mul_add(anchor_y, bounds.max_normalized_y);
    Vec2::new(x, y)
}

fn viewport_anchor_point(
    bounds: &ScreenSpaceBounds,
    viewport_size: Option<Vec2>,
    anchor: FitAnchor,
    offset_px: Vec2,
) -> Vec2 {
    let (anchor_x, anchor_y) = anchor.offset_fraction();
    let viewport_width = bounds.half_extent_x * 2.0;
    let viewport_height = bounds.half_extent_y * 2.0;
    let x = -bounds.half_extent_x + viewport_width * anchor_x;
    let y = bounds.half_extent_y - viewport_height * anchor_y;
    let offset = normalized_pixel_offset(bounds, viewport_size, offset_px);

    Vec2::new(x + offset.x, y + offset.y)
}

fn normalized_pixel_offset(
    bounds: &ScreenSpaceBounds,
    viewport_size: Option<Vec2>,
    offset_px: Vec2,
) -> Vec2 {
    let Some(viewport_size) = viewport_size else {
        return Vec2::ZERO;
    };
    if !viewport_can_map_pixels(Some(viewport_size)) {
        return Vec2::ZERO;
    }

    Vec2::new(
        offset_px.x / viewport_size.x * bounds.half_extent_x * 2.0,
        -offset_px.y / viewport_size.y * bounds.half_extent_y * 2.0,
    )
}
