use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Camera2d;
use bevy::prelude::Camera3d;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::Node;
use bevy::prelude::On;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Remove;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Text;
use bevy::prelude::TextColor;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::ui::UiTargetCamera;
use bevy::window::PrimaryWindow;

use super::config::FitTargetOverlayConfig;
use super::margin_lines;
use super::margin_lines::DrawContext;
use super::ui_camera;
use crate::CurrentFitTarget;
use crate::fit::geometry::ScreenSpaceBounds;
use crate::fit::overlay::FitOverlay;
use crate::fit::overlay::constants::PERCENT_MULTIPLIER;
use crate::fit::overlay::geometry;
use crate::fit::overlay::geometry::FitOverlayFrame;
use crate::fit::overlay::geometry::FitOverlayLayout;
use crate::fit::overlay::render::labels;
use crate::fit::overlay::render::labels::BoundsLabel;
use crate::fit::overlay::render::labels::MarginLabel;
use crate::fit::overlay::render::lines;
use crate::fit::overlay::render::lines::FitOverlayLineContext;
use crate::fit::overlay::render::lines::FitOverlayLineMaterial;
use crate::fit::overlay::render::lines::FitOverlayLineMaterials;
use crate::fit::overlay::render::lines::FitOverlayLineQueryItem;
use crate::fit::overlay::render::lines::FitOverlayLineVisual;
use crate::fit::overlay::render::reconciliation;
use crate::fit::overlay::render::visual::FitOverlayVisual;
use crate::fit::overlay::render::visual::FitOverlayVisualKind;

/// Current screen-space margin percentages for the fit target.
/// Updated every frame by the visualization system.
/// Removed when fit target visualization is disabled.
#[derive(Component, Reflect, Debug, Default, Clone)]
#[reflect(Component)]
pub(in crate::fit::overlay) struct FitMarginPercents {
    /// Left margin as a percentage of screen width.
    pub left:   f32,
    /// Right margin as a percentage of screen width.
    pub right:  f32,
    /// Top margin as a percentage of screen height.
    pub top:    f32,
    /// Bottom margin as a percentage of screen height.
    pub bottom: f32,
}

impl From<&ScreenSpaceBounds> for FitMarginPercents {
    fn from(bounds: &ScreenSpaceBounds) -> Self {
        let screen_width = 2.0 * bounds.half_extent_x;
        let screen_height = 2.0 * bounds.half_extent_y;
        Self {
            left:   (bounds.left_margin / screen_width) * PERCENT_MULTIPLIER,
            right:  (bounds.right_margin / screen_width) * PERCENT_MULTIPLIER,
            top:    (bounds.top_margin / screen_height) * PERCENT_MULTIPLIER,
            bottom: (bounds.bottom_margin / screen_height) * PERCENT_MULTIPLIER,
        }
    }
}

/// Creates the 4 corners of the screen-aligned boundary rectangle in normalized space.
const fn rectangle_points(bounds: &ScreenSpaceBounds) -> [Vec2; 4] {
    [
        Vec2::new(bounds.min_normalized_x, bounds.min_normalized_y),
        Vec2::new(bounds.max_normalized_x, bounds.min_normalized_y),
        Vec2::new(bounds.max_normalized_x, bounds.max_normalized_y),
        Vec2::new(bounds.min_normalized_x, bounds.max_normalized_y),
    ]
}

fn silhouette_points(layout: &FitOverlayLayout) -> Vec<Vec2> {
    let projected = geometry::project_vertices_to_2d(
        &layout.vertices,
        &layout.camera_basis,
        layout.projection_mode,
    );
    geometry::convex_hull_2d(&projected)
        .into_iter()
        .map(|(x, y)| Vec2::new(x, y))
        .collect()
}

/// Observer that cleans up overlay state when `FitOverlay` is removed from a camera.
pub fn on_remove_fit_visualization(
    trigger: On<Remove, FitOverlay>,
    mut commands: Commands,
    visual_query: Query<(Entity, &FitOverlayVisual)>,
) {
    let camera = trigger.entity;
    reconciliation::clear_camera_visuals(&mut commands, camera, &visual_query);
}

/// Draws screen-aligned bounds for all cameras with `FitOverlay`.
pub fn draw_fit_target_bounds(
    mut commands: Commands,
    config: Res<FitTargetOverlayConfig>,
    mut line_materials: ResMut<FitOverlayLineMaterials>,
    camera_query: Query<
        (
            Entity,
            &Camera,
            &RenderTarget,
            Option<&RenderLayers>,
            &GlobalTransform,
            &Projection,
            Option<&CurrentFitTarget>,
        ),
        With<FitOverlay>,
    >,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    all_cameras: Query<(
        Entity,
        &Camera,
        &RenderTarget,
        Option<&Camera2d>,
        Option<&Camera3d>,
    )>,
    mesh_query: Query<&Mesh3d>,
    children_query: Query<&Children>,
    global_transform_query: Query<&GlobalTransform>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FitOverlayLineMaterial>>,
    mut label_query: Query<(
        Entity,
        &MarginLabel,
        &FitOverlayVisual,
        &mut Text,
        &mut Node,
        &mut TextColor,
        &mut UiTargetCamera,
    )>,
    visual_query: Query<(Entity, &FitOverlayVisual)>,
    line_query: Query<FitOverlayLineQueryItem, With<FitOverlayLineVisual>>,
    line_cleanup_query: Query<(Entity, &FitOverlayVisual), With<FitOverlayLineVisual>>,
    mut bounds_label_query: Query<
        (
            Entity,
            &BoundsLabel,
            &FitOverlayVisual,
            &mut Node,
            &mut UiTargetCamera,
        ),
        Without<MarginLabel>,
    >,
) {
    let primary_window = primary_window.single().ok();
    for (
        camera,
        camera_component,
        render_target,
        render_layers,
        camera_global,
        projection,
        current_target,
    ) in &camera_query
    {
        let frame = geometry::resolve_fit_overlay_frame(
            camera,
            camera_component,
            render_target,
            render_layers,
            primary_window,
            camera_global,
            projection,
            current_target,
            &mesh_query,
            &children_query,
            &global_transform_query,
            &meshes,
        );

        match frame {
            FitOverlayFrame::Visible(layout) => {
                let line_index = lines::retained_line_entities(&line_query);
                let mut line_context = FitOverlayLineContext {
                    commands:       &mut commands,
                    meshes:         &mut meshes,
                    materials:      &mut materials,
                    material_cache: &mut line_materials,
                    line_index:     &line_index,
                    layout:         &layout,
                };
                draw_bounds_for_camera(
                    &mut line_context,
                    &config,
                    &layout,
                    ui_camera::label_ui_camera(
                        layout.context.camera,
                        &layout.context.normalized_target,
                        primary_window,
                        all_cameras.iter(),
                    ),
                    &mut label_query,
                    &mut bounds_label_query,
                    &line_cleanup_query,
                );
            },
            FitOverlayFrame::Empty(reason) => {
                reconciliation::clear_empty_frame(&mut commands, camera, reason, &visual_query);
            },
        }
    }
}

/// Draws bounds visualization for a single camera/target pair.
fn draw_bounds_for_camera(
    line_context: &mut FitOverlayLineContext<'_, '_, '_>,
    config: &FitTargetOverlayConfig,
    layout: &FitOverlayLayout,
    ui_camera: Entity,
    label_query: &mut Query<(
        Entity,
        &MarginLabel,
        &FitOverlayVisual,
        &mut Text,
        &mut Node,
        &mut TextColor,
        &mut UiTargetCamera,
    )>,
    bounds_label_query: &mut Query<
        (
            Entity,
            &BoundsLabel,
            &FitOverlayVisual,
            &mut Node,
            &mut UiTargetCamera,
        ),
        Without<MarginLabel>,
    >,
    line_cleanup_query: &Query<(Entity, &FitOverlayVisual), With<FitOverlayLineVisual>>,
) {
    let camera = layout.context.camera;
    let bounds = &layout.bounds;
    let viewport_size = layout.viewport_size;
    let mut desired_line_kinds = Vec::new();

    bevy::log::trace!(
        "drawing fit overlay for {camera:?} with order {} and layers {:?}",
        layout.context.order,
        layout.context.layers
    );

    // Update margin percentages on camera entity for BRP inspection.
    // `try_insert` silently skips if the entity was despawned this frame
    // (e.g. closing a secondary window while visualization is active).
    line_context
        .commands
        .entity(camera)
        .try_insert(FitMarginPercents::from(bounds));

    let rectangle = rectangle_points(bounds);
    line_context.upsert_polyline(
        FitOverlayVisual {
            camera,
            kind: FitOverlayVisualKind::Rectangle,
        },
        &rectangle,
        true,
        config.rectangle_color,
        config.line_width,
    );
    desired_line_kinds.push(FitOverlayVisualKind::Rectangle);

    let silhouette = silhouette_points(layout);
    if silhouette.len() >= 2 {
        line_context.upsert_polyline(
            FitOverlayVisual {
                camera,
                kind: FitOverlayVisualKind::Silhouette,
            },
            &silhouette,
            true,
            config.silhouette_color,
            config.line_width,
        );
        desired_line_kinds.push(FitOverlayVisualKind::Silhouette);
    }

    let upper_left = geometry::norm_to_viewport(
        bounds.min_normalized_x,
        bounds.max_normalized_y,
        bounds.half_extent_x,
        bounds.half_extent_y,
        viewport_size,
    );
    labels::update_or_create_bounds_label(
        line_context.commands,
        bounds_label_query,
        camera,
        ui_camera,
        labels::bounds_label_position(upper_left),
    );

    let draw_context = DrawContext {
        camera,
        ui_camera,
        bounds,
        viewport_size: Some(viewport_size),
    };
    let visible_edges = margin_lines::draw_margin_lines_and_labels(
        line_context,
        label_query,
        &draw_context,
        config,
    );
    desired_line_kinds.extend(
        visible_edges
            .iter()
            .map(|&edge| FitOverlayVisualKind::MarginLine { edge }),
    );

    margin_lines::cleanup_stale_margin_labels(
        line_context.commands,
        label_query,
        camera,
        &visible_edges,
    );
    lines::clear_stale_lines(
        line_context.commands,
        camera,
        &desired_line_kinds,
        line_cleanup_query,
    );
}
