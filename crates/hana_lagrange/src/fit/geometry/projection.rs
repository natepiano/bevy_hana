//! Projection, screen-space, and mesh-extraction helpers.

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::Assets;
use bevy::prelude::Children;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use hana_kana::Position;
use hana_kana::ToF32;

use crate::fit::constants::MIN_VISIBLE_DEPTH;

// ============================================================================
// Projection basis
// ============================================================================

/// Projection basis vectors extracted from a `GlobalTransform`.
/// Bundles the position and orientation vectors that are frequently passed together.
pub(in crate::fit) struct ProjectionBasis {
    pub position: Position,
    pub right:    Vec3,
    pub up:       Vec3,
    pub forward:  Vec3,
}

impl From<&GlobalTransform> for ProjectionBasis {
    fn from(global: &GlobalTransform) -> Self {
        let rotation = global.rotation();
        Self {
            position: Position(global.translation()),
            right:    rotation * Vec3::X,
            up:       rotation * Vec3::Y,
            forward:  rotation * Vec3::NEG_Z,
        }
    }
}

// ============================================================================
// Projection utilities
// ============================================================================

/// Projection-derived parameters for screen-space normalization.
/// Consolidates the extraction of half extents and projection type from a `Projection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::fit) enum ProjectionMode {
    Perspective,
    Orthographic,
}

struct ProjectionParameters {
    /// Half visible extent in x (perspective: `half_tan_horizontal_fov`, ortho: `area.width()/2`)
    half_extent_x: f32,
    /// Half visible extent in y (perspective: `half_tan_vertical_fov`, ortho: `area.height()/2`)
    half_extent_y: f32,
    /// Projection mode for the current camera.
    mode:          ProjectionMode,
}

impl ProjectionParameters {
    /// Extracts projection parameters from a `Projection` and viewport aspect ratio.
    /// Returns `None` for unsupported projection variants.
    fn from_projection(projection: &Projection, viewport_aspect: f32) -> Option<Self> {
        let projection_parameters = match projection {
            Projection::Perspective(p) => {
                let half_tan_vertical_fov = (p.fov * 0.5).tan();
                Some((
                    half_tan_vertical_fov * viewport_aspect,
                    half_tan_vertical_fov,
                    ProjectionMode::Perspective,
                ))
            },
            Projection::Orthographic(o) => Some((
                o.area.width() * 0.5,
                o.area.height() * 0.5,
                ProjectionMode::Orthographic,
            )),
            Projection::Custom(_) => None,
        };
        let (half_extent_x, half_extent_y, mode) = projection_parameters?;
        Some(Self {
            half_extent_x,
            half_extent_y,
            mode,
        })
    }
}

/// Projects a world-space point to normalized screen coordinates.
///
/// Returns `(normalized_x, normalized_y, depth)` or `None` if the point is behind the camera
/// (perspective only — orthographic points are always valid).
pub(in crate::fit) fn project_point(
    point: Vec3,
    camera: &ProjectionBasis,
    projection_mode: ProjectionMode,
) -> Option<(f32, f32, f32)> {
    let relative = point - *camera.position;
    let depth = relative.dot(camera.forward);
    let is_visible = match projection_mode {
        ProjectionMode::Perspective => depth > MIN_VISIBLE_DEPTH,
        ProjectionMode::Orthographic => true,
    };
    if !is_visible {
        return None;
    }
    let x = relative.dot(camera.right);
    let y = relative.dot(camera.up);
    let (normalized_x, normalized_y) = match projection_mode {
        ProjectionMode::Orthographic => (x, y),
        ProjectionMode::Perspective => (x / depth, y / depth),
    };
    Some((normalized_x, normalized_y, depth))
}

/// Extracts the aspect ratio from a `Projection`, using `viewport_size` for
/// perspective when available, falling back to `PerspectiveProjection::aspect_ratio`.
///
/// Returns `None` for orthographic projections with zero-height area or unknown
/// projection variants.
pub(in crate::fit) fn projection_aspect_ratio(
    projection: &Projection,
    viewport_size: Option<Vec2>,
) -> Option<f32> {
    match projection {
        Projection::Perspective(p) => Some(viewport_size.map_or(p.aspect_ratio, |s| s.x / s.y)),
        Projection::Orthographic(o) => {
            let area = o.area;
            if area.height().abs() < f32::EPSILON {
                return None;
            }
            Some(area.width() / area.height())
        },
        Projection::Custom(_) => None,
    }
}

// ============================================================================
// Screen-space bounds
// ============================================================================

/// Depths of the extreme projected points, tracked during the projection loop.
/// Used by the fit algorithm for perspective-correct centering (harmonic mean)
/// and by `fit_overlay` for average depth of retained line placement.
#[derive(Debug, Clone)]
pub(in crate::fit) struct PointDepths {
    pub min_x:   f32,
    pub max_x:   f32,
    pub min_y:   f32,
    pub max_y:   f32,
    #[cfg_attr(
        not(feature = "fit_overlay"),
        allow(dead_code, reason = "overlay rendering consumes this depth metric")
    )]
    pub average: Option<f32>,
}

/// Screen-space bounds of a set of projected points, with margin distances
/// from each screen edge.
#[derive(Debug, Clone)]
pub(in crate::fit) struct ScreenSpaceBounds {
    /// Distance from left edge (positive = inside, negative = outside)
    pub left_margin:      f32,
    /// Distance from right edge (positive = inside, negative = outside)
    pub right_margin:     f32,
    /// Distance from top edge (positive = inside, negative = outside)
    pub top_margin:       f32,
    /// Distance from bottom edge (positive = inside, negative = outside)
    pub bottom_margin:    f32,
    /// Minimum normalized x coordinate in screen space
    pub min_normalized_x: f32,
    /// Maximum normalized x coordinate in screen space
    pub max_normalized_x: f32,
    /// Minimum normalized y coordinate in screen space
    pub min_normalized_y: f32,
    /// Maximum normalized y coordinate in screen space
    pub max_normalized_y: f32,
    /// Half visible extent in x (perspective: `half_tan_horizontal_fov`, ortho: `area.width()/2`)
    pub half_extent_x:    f32,
    /// Half visible extent in y (perspective: `half_tan_vertical_fov`, ortho: `area.height()/2`)
    pub half_extent_y:    f32,
}

impl ScreenSpaceBounds {
    /// Projects world-space points to normalized screen space and computes margins.
    /// Returns `None` if any point is behind the camera (perspective only).
    #[allow(
        clippy::similar_names,
        reason = "min/max pairs per axis follow a consistent naming pattern"
    )]
    pub(in crate::fit) fn from_points(
        points: &[Vec3],
        camera_global: &GlobalTransform,
        projection: &Projection,
        viewport_aspect: f32,
    ) -> Option<(Self, PointDepths)> {
        let ProjectionParameters {
            half_extent_x,
            half_extent_y,
            mode,
        } = ProjectionParameters::from_projection(projection, viewport_aspect)?;

        let camera_basis = ProjectionBasis::from(camera_global);

        let mut min_normalized_x = f32::INFINITY;
        let mut max_normalized_x = f32::NEG_INFINITY;
        let mut min_normalized_y = f32::INFINITY;
        let mut max_normalized_y = f32::NEG_INFINITY;
        let mut min_x = 0.0_f32;
        let mut max_x = 0.0_f32;
        let mut min_y = 0.0_f32;
        let mut max_y = 0.0_f32;
        let mut depth_sum = 0.0_f32;
        let mut depth_count = 0_usize;

        for point in points {
            let (normalized_x, normalized_y, depth) = project_point(*point, &camera_basis, mode)?;

            depth_sum += depth;
            depth_count += 1;

            if normalized_x < min_normalized_x {
                min_normalized_x = normalized_x;
                min_x = depth;
            }
            if normalized_x > max_normalized_x {
                max_normalized_x = normalized_x;
                max_x = depth;
            }
            if normalized_y < min_normalized_y {
                min_normalized_y = normalized_y;
                min_y = depth;
            }
            if normalized_y > max_normalized_y {
                max_normalized_y = normalized_y;
                max_y = depth;
            }
        }

        let left_margin = min_normalized_x - (-half_extent_x);
        let right_margin = half_extent_x - max_normalized_x;
        let bottom_margin = min_normalized_y - (-half_extent_y);
        let top_margin = half_extent_y - max_normalized_y;

        let bounds = Self {
            left_margin,
            right_margin,
            top_margin,
            bottom_margin,
            min_normalized_x,
            max_normalized_x,
            min_normalized_y,
            max_normalized_y,
            half_extent_x,
            half_extent_y,
        };

        let depths = PointDepths {
            min_x,
            max_x,
            min_y,
            max_y,
            average: if depth_count == 0 {
                None
            } else {
                Some(depth_sum / depth_count.to_f32())
            },
        };

        Some((bounds, depths))
    }
}

// ============================================================================
// Mesh utilities
// ============================================================================

/// Extracts world-space vertex positions from all meshes on an entity and its descendants.
/// Returns `(vertices, geometric_center)` where `geometric_center` is the root entity's
/// `GlobalTransform` translation.
pub(in crate::fit) fn extract_mesh_vertices(
    entity: Entity,
    children_query: &Query<&Children>,
    mesh_query: &Query<&Mesh3d>,
    global_transform_query: &Query<&GlobalTransform>,
    meshes: &Assets<Mesh>,
) -> Option<(Vec<Vec3>, Vec3)> {
    let mesh_entities: Vec<Entity> = std::iter::once(entity)
        .chain(children_query.iter_descendants(entity))
        .filter(|e| mesh_query.get(*e).is_ok())
        .collect();

    if mesh_entities.is_empty() {
        return None;
    }

    let mut all_vertices = Vec::new();

    for mesh_entity in &mesh_entities {
        let Ok(mesh3d) = mesh_query.get(*mesh_entity) else {
            continue;
        };
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            continue;
        };
        let Ok(global_transform) = global_transform_query.get(*mesh_entity) else {
            continue;
        };
        let Some(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(VertexAttributeValues::as_float3)
        else {
            continue;
        };

        all_vertices.extend(
            positions
                .iter()
                .map(|pos| global_transform.transform_point(Vec3::from_array(*pos))),
        );
    }

    if all_vertices.is_empty() {
        return None;
    }

    let geometric_center = global_transform_query
        .get(entity)
        .map_or(Vec3::ZERO, GlobalTransform::translation);

    Some((all_vertices, geometric_center))
}
