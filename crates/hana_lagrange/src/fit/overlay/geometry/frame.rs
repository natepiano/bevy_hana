use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Children;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;

use super::context::FitOverlayCameraContext;
use super::context::FitOverlayEmptyReason;
use crate::CurrentFitTarget;
use crate::fit::geometry;
use crate::fit::geometry::ProjectionBasis;
use crate::fit::geometry::ProjectionMode;
use crate::fit::geometry::ScreenSpaceBounds;

/// Desired overlay frame for one source camera.
pub(in crate::fit::overlay) enum FitOverlayFrame {
    /// The source camera has enough data to render the overlay.
    Visible(Box<FitOverlayLayout>),
    /// The source camera should have no generated visuals this frame.
    Empty(FitOverlayEmptyReason),
}

/// Computed overlay layout for one visible source camera.
pub(in crate::fit::overlay) struct FitOverlayLayout {
    /// Source camera render context.
    pub context:         FitOverlayCameraContext,
    /// Extracted world-space target vertices.
    pub vertices:        Vec<Vec3>,
    /// Projected target bounds in screen space.
    pub bounds:          ScreenSpaceBounds,
    /// Source camera basis vectors.
    pub camera_basis:    ProjectionBasis,
    /// Average projected target depth used for overlay placement.
    pub average_depth:   f32,
    /// Projection mode used by overlay coordinate conversion.
    pub projection_mode: ProjectionMode,
    /// Logical viewport size for label placement.
    pub viewport_size:   Vec2,
}

impl FitOverlayLayout {
    pub(in crate::fit::overlay) fn from_vertices(
        context: FitOverlayCameraContext,
        camera_global: &GlobalTransform,
        projection: &Projection,
        vertices: Vec<Vec3>,
    ) -> FitOverlayFrame {
        let viewport_size = context.viewport_size();
        let Some(aspect_ratio) = geometry::projection_aspect_ratio(projection, Some(viewport_size))
        else {
            return FitOverlayFrame::Empty(FitOverlayEmptyReason::UnsupportedProjection);
        };

        let Some((bounds, depths)) =
            ScreenSpaceBounds::from_points(&vertices, camera_global, projection, aspect_ratio)
        else {
            return FitOverlayFrame::Empty(FitOverlayEmptyReason::UnprojectableBounds);
        };

        let Some(average_depth) = depths.average else {
            return FitOverlayFrame::Empty(FitOverlayEmptyReason::MissingDepths);
        };

        let projection_mode = match projection {
            Projection::Perspective(_) => ProjectionMode::Perspective,
            Projection::Orthographic(_) => ProjectionMode::Orthographic,
            Projection::Custom(_) => {
                return FitOverlayFrame::Empty(FitOverlayEmptyReason::UnsupportedProjection);
            },
        };

        FitOverlayFrame::Visible(Box::new(Self {
            context,
            vertices,
            bounds,
            camera_basis: ProjectionBasis::from(camera_global),
            average_depth,
            projection_mode,
            viewport_size,
        }))
    }
}

pub(in crate::fit::overlay) fn resolve_fit_overlay_frame(
    camera: Entity,
    camera_component: &Camera,
    render_target: &RenderTarget,
    render_layers: Option<&RenderLayers>,
    primary_window: Option<Entity>,
    camera_global: &GlobalTransform,
    projection: &Projection,
    current_target: Option<&CurrentFitTarget>,
    mesh_query: &Query<&Mesh3d>,
    children_query: &Query<&Children>,
    global_transform_query: &Query<&GlobalTransform>,
    meshes: &Assets<Mesh>,
) -> FitOverlayFrame {
    let context = match FitOverlayCameraContext::resolve(
        camera,
        camera_component,
        render_target,
        render_layers,
        primary_window,
    ) {
        Ok(context) => context,
        Err(reason) => return FitOverlayFrame::Empty(reason),
    };

    let Some(current_target) = current_target else {
        return FitOverlayFrame::Empty(FitOverlayEmptyReason::MissingCurrentFitTarget);
    };

    let Some((vertices, _)) = geometry::extract_mesh_vertices(
        current_target.0,
        children_query,
        mesh_query,
        global_transform_query,
        meshes,
    ) else {
        return FitOverlayFrame::Empty(FitOverlayEmptyReason::MissingMesh);
    };

    FitOverlayLayout::from_vertices(context, camera_global, projection, vertices)
}
