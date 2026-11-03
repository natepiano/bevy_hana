use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Children;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Mesh;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;

use super::context::FitOverlayCameraContext;
use super::context::FitOverlayEmptyReason;
use crate::CurrentFitTarget;
use crate::fit::AdmittedFitBounds;
use crate::fit::FitBoundsSource;
use crate::fit::geometry;
use crate::fit::geometry::FitMeshQuery;
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
    fit_target: (Option<&CurrentFitTarget>, Option<&AdmittedFitBounds>),
    mesh_query: &FitMeshQuery,
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

    let Some(current_target) = fit_target.0 else {
        return FitOverlayFrame::Empty(FitOverlayEmptyReason::MissingCurrentFitTarget);
    };

    let vertices = match fit_target.1.map(|bounds| &bounds.0) {
        Some(FitBoundsSource::WorldBounds(points)) => points.clone(),
        Some(FitBoundsSource::MeasureTarget) | None => {
            let Some((vertices, _)) = geometry::extract_mesh_vertices(
                current_target.0,
                children_query,
                mesh_query,
                global_transform_query,
                meshes,
            ) else {
                return FitOverlayFrame::Empty(FitOverlayEmptyReason::MissingMesh);
            };
            vertices
        },
    };

    FitOverlayLayout::from_vertices(context, camera_global, projection, vertices)
}

#[cfg(test)]
mod tests {
    use bevy::camera::RenderTargetInfo;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use bevy::window::WindowRef;

    use super::*;
    use crate::CameraBasis;
    use crate::FitBounds;
    use crate::OrbitCam;
    use crate::ZoomToFit;
    use crate::animation::AnimationPlugin;
    use crate::fit::FitPlugin;

    fn overlay_vertices(app: &mut App, camera: Entity) -> Result<Vec<Vec3>, String> {
        app.world_mut()
            .run_system_once(
                move |cameras: Query<(
                    &Camera,
                    &RenderTarget,
                    &GlobalTransform,
                    &Projection,
                    &CurrentFitTarget,
                    &AdmittedFitBounds,
                )>,
                      mesh_query: FitMeshQuery,
                      children: Query<&Children>,
                      poses: Query<&GlobalTransform>,
                      meshes: Res<Assets<Mesh>>| {
                    let (lens, target, pose, projection, current, bounds) =
                        cameras.get(camera).map_err(|error| error.to_string())?;
                    match resolve_fit_overlay_frame(
                        camera,
                        lens,
                        target,
                        None,
                        None,
                        pose,
                        projection,
                        (Some(current), Some(bounds)),
                        &mesh_query,
                        &children,
                        &poses,
                        &meshes,
                    ) {
                        FitOverlayFrame::Visible(layout) => Ok(layout.vertices),
                        FitOverlayFrame::Empty(reason) => Err(format!("overlay: {reason:?}")),
                    }
                },
            )
            .map_err(|error| error.to_string())?
    }

    #[test]
    fn overlay_silhouette_and_margins_use_admitted_bounds_including_retargets() -> Result<(), String>
    {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .add_plugins((AnimationPlugin, FitPlugin));
        let window = app.world_mut().spawn_empty().id();
        let target = app
            .world_mut()
            .spawn((
                FitBounds {
                    center:       Vec3::splat(-100.0),
                    half_extents: Vec3::splat(50.0),
                },
                GlobalTransform::IDENTITY,
            ))
            .id();
        let mut lens = Camera::default();
        lens.computed.target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(1280, 720),
            scale_factor:  1.0,
        });
        let camera = app
            .world_mut()
            .spawn((
                lens,
                RenderTarget::Window(WindowRef::Entity(window)),
                OrbitCam::from_pose(Vec3::ZERO, (0.0, 0.0), 8.0),
                CameraBasis::Y_UP,
                Projection::Perspective(PerspectiveProjection::default()),
                Transform::from_xyz(0.0, 0.0, 8.0),
                GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 8.0)),
            ))
            .id();
        for offset in [0.0, 0.5] {
            let points = vec![
                Vec3::new(-1.0 + offset, -1.0, 0.0),
                Vec3::new(1.0 + offset, 1.0, 0.0),
            ];
            app.world_mut()
                .trigger(ZoomToFit::new(camera, target).world_bounds(points.clone()));
            app.update();
            assert_eq!(overlay_vertices(&mut app, camera)?, points);
        }
        Ok(())
    }
}
