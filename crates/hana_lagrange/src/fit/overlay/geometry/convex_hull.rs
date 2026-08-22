use bevy::prelude::Vec3;

use crate::fit::geometry;
use crate::fit::geometry::ProjectionBasis;
use crate::fit::geometry::ProjectionMode;

/// 2D cross product for three points (for convex hull turn detection).
fn cross_2d(o: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - o.0).mul_add(b.1 - o.1, -(a.1 - o.1) * (b.0 - o.0))
}

/// Andrew's monotone chain algorithm for 2D convex hull.
/// Returns hull vertices in counter-clockwise order.
pub(in crate::fit::overlay) fn convex_hull_2d(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut sorted: Vec<(f32, f32)> = points.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    sorted.dedup();

    if sorted.len() <= 1 {
        return sorted;
    }

    let mut lower: Vec<(f32, f32)> = Vec::new();
    for &p in &sorted {
        while lower.len() >= 2 && cross_2d(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0
        {
            lower.pop();
        }
        lower.push(p);
    }

    let mut upper: Vec<(f32, f32)> = Vec::new();
    for &p in sorted.iter().rev() {
        while upper.len() >= 2 && cross_2d(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0
        {
            upper.pop();
        }
        upper.push(p);
    }

    lower.pop();
    upper.pop();

    lower.extend(upper);
    lower
}

/// Projects world-space vertices to 2D normalized screen space.
///
/// For perspective, divides by depth. For orthographic, uses raw camera-space coordinates.
pub(in crate::fit::overlay) fn project_vertices_to_2d(
    vertices: &[Vec3],
    camera: &ProjectionBasis,
    projection_mode: ProjectionMode,
) -> Vec<(f32, f32)> {
    vertices
        .iter()
        .filter_map(|v| {
            let (normalized_x, normalized_y, _) =
                geometry::project_point(*v, camera, projection_mode)?;
            Some((normalized_x, normalized_y))
        })
        .collect()
}
