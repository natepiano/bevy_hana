use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchoredTo;
use hana_valence::ResolvedAnchorGeometry;

use super::controller::TooltipFor;
use super::declaration::TooltipPhase;
use super::materialization;
use super::materialization::MaterializedTooltip;
use super::materialization::TooltipPresentationCamera;
use super::materialization::TooltipPresentationSpace;
use super::materialization::TooltipReadiness;
use super::materialization::TooltipWidthConstraint;
use super::placement;
use super::placement::TooltipPlacementResult;
use super::placement::TooltipPlacementState;
use super::placement::WorldTooltipRevealOcclusion;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::PanelSpace;
use crate::panel::ResolvedScreenPanelPosition;
use crate::panel::{self};
use crate::screen_space::ScreenAnchorTarget;
use crate::screen_space::ScreenAttachmentResolveDiagnostics;
use crate::screen_space::{self};
use crate::widgets::PanelWidget;
use crate::widgets::WidgetAnchorRect;
use crate::widgets::WidgetOf;

#[derive(SystemParam)]
pub(super) struct TooltipReadinessInputChanges<'w, 's> {
    controllers: Query<
        'w,
        's,
        (),
        (
            With<MaterializedTooltip>,
            Or<(
                Added<MaterializedTooltip>,
                Changed<TooltipFor>,
                Changed<DiegeticPanel>,
                Changed<ComputedDiegeticPanel>,
                Changed<TooltipPresentationCamera>,
                Changed<TooltipWidthConstraint>,
                Changed<TooltipPlacementState>,
                Changed<ResolvedScreenPanelPosition>,
                Changed<AnchoredTo>,
                Changed<PanelAttachmentAuthored>,
            )>,
        ),
    >,
    panels: Query<
        'w,
        's,
        (
            Ref<'static, DiegeticPanel>,
            Ref<'static, Transform>,
            Ref<'static, GlobalTransform>,
            Ref<'static, ResolvedScreenPanelPosition>,
        ),
        (
            Without<MaterializedTooltip>,
            Or<(
                Changed<DiegeticPanel>,
                Changed<Transform>,
                Changed<GlobalTransform>,
                Changed<ResolvedScreenPanelPosition>,
            )>,
        ),
    >,
    geometry_targets: Query<
        'w,
        's,
        (),
        (
            Without<MaterializedTooltip>,
            With<ResolvedAnchorGeometry>,
            Or<(
                Changed<ResolvedAnchorGeometry>,
                Changed<GlobalTransform>,
                Changed<WidgetOf>,
                Changed<WidgetAnchorRect>,
            )>,
        ),
    >,
    cameras: Query<
        'w,
        's,
        (),
        (
            With<Camera>,
            Or<(
                Changed<Camera>,
                Changed<GlobalTransform>,
                Changed<Transform>,
                Changed<RenderTarget>,
                Changed<RenderLayers>,
            )>,
        ),
    >,
    windows:                     Query<'w, 's, (), Changed<Window>>,
    screen_targets:              Query<'w, 's, (), Changed<ScreenAnchorTarget>>,
    authored_attachments:        Query<'w, 's, (), Changed<PanelAttachmentAuthored>>,
    world_attachments:           Query<'w, 's, (), Changed<AnchoredTo>>,
    removed_geometry:            RemovedComponents<'w, 's, ResolvedAnchorGeometry>,
    removed_presentation_camera: RemovedComponents<'w, 's, TooltipPresentationCamera>,
    removed_camera:              RemovedComponents<'w, 's, Camera>,
    removed_render_target:       RemovedComponents<'w, 's, RenderTarget>,
    removed_render_layers:       RemovedComponents<'w, 's, RenderLayers>,
    removed_window:              RemovedComponents<'w, 's, Window>,
    removed_screen_target:       RemovedComponents<'w, 's, ScreenAnchorTarget>,
    removed_attachment:          RemovedComponents<'w, 's, PanelAttachmentAuthored>,
    removed_world_attachment:    RemovedComponents<'w, 's, AnchoredTo>,
    removed_transform:           RemovedComponents<'w, 's, Transform>,
    removed_global_transform:    RemovedComponents<'w, 's, GlobalTransform>,
}

pub(super) fn tooltip_readiness_inputs_changed(
    mut inputs: TooltipReadinessInputChanges<'_, '_>,
) -> bool {
    inputs.controllers.iter().next().is_some()
        || inputs
            .panels
            .iter()
            .any(|(panel, transform, global_transform, resolved_position)| {
                panel.is_changed()
                    || match PanelSpace::from(panel.coordinate_space()) {
                        PanelSpace::World => global_transform.is_changed(),
                        PanelSpace::Screen => {
                            transform.is_changed() || resolved_position.is_changed()
                        },
                    }
            })
        || inputs.geometry_targets.iter().next().is_some()
        || inputs.cameras.iter().next().is_some()
        || inputs.windows.iter().next().is_some()
        || inputs.screen_targets.iter().next().is_some()
        || inputs.authored_attachments.iter().next().is_some()
        || inputs.world_attachments.iter().next().is_some()
        || inputs.removed_geometry.read().next().is_some()
        || inputs.removed_presentation_camera.read().next().is_some()
        || inputs.removed_camera.read().next().is_some()
        || inputs.removed_render_target.read().next().is_some()
        || inputs.removed_render_layers.read().next().is_some()
        || inputs.removed_window.read().next().is_some()
        || inputs.removed_screen_target.read().next().is_some()
        || inputs.removed_attachment.read().next().is_some()
        || inputs.removed_world_attachment.read().next().is_some()
        || inputs.removed_transform.read().next().is_some()
        || inputs.removed_global_transform.read().next().is_some()
}

pub(super) fn tooltip_readiness_or_reveal(
    inputs: TooltipReadinessInputChanges<'_, '_>,
    phases: Query<&TooltipPhase>,
) -> bool {
    tooltip_readiness_inputs_changed(inputs)
        || phases
            .iter()
            .any(|phase| matches!(phase, TooltipPhase::WaitingToShow(_)))
}

pub(super) fn finalize_tooltip_readiness(
    mut controllers: Query<(
        Entity,
        &TooltipFor,
        &MaterializedTooltip,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &TooltipPlacementState,
        &GlobalTransform,
        &TooltipPhase,
        &mut TooltipReadiness,
        Option<&TooltipPresentationCamera>,
        Option<&ResolvedScreenPanelPosition>,
        Option<&AnchoredTo>,
        Option<&PanelAttachmentAuthored>,
    )>,
    panels: Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    widgets: Query<(&WidgetOf, &WidgetAnchorRect), With<PanelWidget>>,
    screen_targets: Query<&ScreenAnchorTarget>,
    windows: Query<&Window>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    screen_diagnostics: Option<Res<ScreenAttachmentResolveDiagnostics>>,
    world_diagnostics: Option<Res<AnchorResolveDiagnostics>>,
    mut reveal_occlusion: WorldTooltipRevealOcclusion,
) {
    for (
        entity,
        tooltip_for,
        materialized,
        panel,
        computed,
        placement,
        global_transform,
        phase,
        mut readiness,
        presentation_camera,
        screen_position,
        world_attachment,
        authored_attachment,
    ) in &mut controllers
    {
        let placement_ready = placement.result == TooltipPlacementResult::Fits;
        let target_matches = authored_attachment
            .is_some_and(|attachment| attachment.target() == materialized.target)
            && tooltip_for.target() == materialized.target;
        let layout_ready = computed.is_solved();
        let resolved = match materialized.space() {
            PanelSpace::World => world_attachment.is_some_and(|attachment| {
                let Some(world_diagnostics) = world_diagnostics.as_deref() else {
                    return false;
                };
                panel::world_attachment_is_ready(
                    entity,
                    materialized.target,
                    attachment,
                    world_diagnostics,
                )
            }),
            PanelSpace::Screen => screen_position.is_some_and(|position| {
                let Some(screen_diagnostics) = screen_diagnostics.as_deref() else {
                    return false;
                };
                screen_space::screen_attachment_is_ready(entity, position, screen_diagnostics)
            }),
        };
        let presentation_matches =
            materialized_panel_presentation_matches(panel, materialized, &windows, &primary_window);
        let target_presentation_matches = match materialized.space() {
            PanelSpace::World => true,
            PanelSpace::Screen => placement::screen_tooltip_target(
                materialized.target,
                &panels,
                &widgets,
                &screen_targets,
                &windows,
                &primary_window,
            )
            .is_some_and(|target| target.presentation_matches(materialized)),
        };
        let next = if layout_ready
            && placement_ready
            && target_matches
            && resolved
            && presentation_matches
            && target_presentation_matches
            && global_transform.affine().is_finite()
            && (materialized.space() != PanelSpace::World
                || !matches!(phase, TooltipPhase::WaitingToShow(_))
                || presentation_camera.is_some_and(|presentation_camera| {
                    reveal_occlusion.clear(entity, materialized, panel, presentation_camera.camera)
                })) {
            TooltipReadiness::Ready
        } else {
            TooltipReadiness::Pending
        };
        if *readiness != next {
            *readiness = next;
        }
    }
}

fn materialized_panel_presentation_matches(
    panel: &DiegeticPanel,
    materialized: &MaterializedTooltip,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> bool {
    if panel.layout_unit() != materialized.layout_unit {
        return false;
    }
    match panel.coordinate_space() {
        CoordinateSpace::World { .. } => {
            materialized.presentation == TooltipPresentationSpace::World
        },
        CoordinateSpace::Screen {
            camera_order,
            render_layers,
            window,
            ..
        } => materialization::live_window(*window, windows, primary_window).is_some_and(|window| {
            materialized.presentation
                == TooltipPresentationSpace::Screen {
                    window,
                    camera_order: *camera_order,
                }
                && materialized.render_layers == *render_layers
        }),
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::PerspectiveProjection;
    use bevy::camera::Projection;
    use bevy::camera::RenderTarget;
    use bevy::camera::RenderTargetInfo;
    use bevy::camera::primitives::Aabb;
    use bevy::camera::visibility::RenderLayers;
    use bevy::mesh::Mesh3d;
    use bevy::prelude::App;
    use bevy::prelude::Camera;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::Vec3A;
    use bevy::prelude::Visibility;
    use bevy::prelude::Window;
    use bevy::transform::TransformPlugin;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowRef;
    use hana_valence::AnchorResolveDiagnostics;
    use hana_valence::AnchorSite;
    use hana_valence::AnchoredHere;
    use hana_valence::AnchoredTo;
    use hana_valence::ResolvedAnchorGeometry;

    use super::*;
    use crate::HeadlessLayoutPlugin;
    use crate::PanelDefaults;
    use crate::PanelPicking;
    use crate::layout::Anchor;
    use crate::layout::El;
    use crate::layout::Px;
    use crate::layout::Unit;
    use crate::panel::CoordinateSpace;
    use crate::panel::DiegeticPanel;
    use crate::panel::PanelAnchorOffset;
    use crate::panel::PanelScreenBounds;
    use crate::panel::World;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::declaration::PrepareTooltip;
    use crate::widgets::tooltip::placement::TooltipPlacementRunCount;
    use crate::widgets::tooltip::placement::TooltipPlacementState;
    use crate::widgets::tooltip::*;

    const GEOMETRY_EPSILON: f32 = 1e-4;

    const MATERIALIZATION_UPDATES: usize = 5;

    const SMALL_SCREEN_VIEWPORT: Vec2 = Vec2::new(240.0, 180.0);

    const TEST_CAMERA_DISTANCE: f32 = 5.0;

    const TEST_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

    fn materialization_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app
    }

    fn update_materialization(app: &mut App) {
        for _ in 0..MATERIALIZATION_UPDATES {
            app.update();
        }
    }

    fn mesh_target(app: &mut App, half_extents: Vec3A) -> Entity {
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center: Vec3A::ZERO,
                    half_extents,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        target
    }

    fn standalone_world_tooltip(app: &mut App, target: Entity, tooltip: Tooltip) -> Entity {
        let handle = TooltipTargetEntity::<World>::from_validated(target);
        app.world_mut().commands().spawn_tooltip(handle, tooltip)
    }

    fn prepare_tooltip(app: &mut App, controller: Entity) {
        app.world_mut()
            .entity_mut(controller)
            .insert(PrepareTooltip);
    }

    fn seeded_perspective_camera() -> Camera { seeded_perspective_camera_for(TEST_VIEWPORT) }

    fn seeded_perspective_camera_for(viewport: Vec2) -> Camera {
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: viewport.as_uvec2(),
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(viewport.x, viewport.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        camera
    }

    fn spawn_presentation_camera(
        app: &mut App,
        window: Entity,
        render_layers: RenderLayers,
    ) -> Entity {
        let transform = Transform::from_xyz(0.0, 0.0, TEST_CAMERA_DISTANCE);
        app.world_mut()
            .spawn((
                seeded_perspective_camera(),
                RenderTarget::Window(WindowRef::Entity(window)),
                render_layers,
                transform,
                GlobalTransform::from(transform),
            ))
            .id()
    }

    fn fixed_size_tooltip(size: Vec2) -> Tooltip {
        let mut tooltip = Tooltip::new(El::new());
        tooltip.with(El::new().size(size.x, size.y), |_| {});
        tooltip
    }

    fn mesh_anchor_center(app: &App, target: Entity) -> Option<Vec3> {
        app.world()
            .get::<ResolvedAnchorGeometry>(target)
            .and_then(|geometry| geometry.frame(AnchorSite::Center).ok())
            .map(|point| point.position().into_inner())
    }

    #[test]
    fn unavailable_mesh_provider_and_current_diagnostic_keep_tooltip_pending() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        let handle = app
            .world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, fixed_size_tooltip(Vec2::new(30.0, 10.0)));
        app.update();
        assert!(app.world().get::<AnchoredHere>(target).is_none());
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());

        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);
        assert!(app.world().get::<MaterializedTooltip>(controller).is_some());
        assert!(app.world().get::<AnchoredTo>(controller).is_some());
        assert!(
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .any(|entry| entry.source == controller)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );

        app.world_mut().entity_mut(target).insert(Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::splat(0.5),
        });
        update_materialization(&mut app);
        assert_eq!(
            mesh_anchor_center(&app, target),
            Some(Vec3::new(0.0, 0.0, 0.5))
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}, diagnostics={:?}",
            app.world().get::<TooltipPlacementState>(controller),
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .copied()
                .collect::<Vec<_>>(),
        );

        app.world_mut().entity_mut(target).remove::<Aabb>();
        update_materialization(&mut app);
        assert!(
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .any(|entry| entry.source == controller)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn current_world_graph_failures_invalidate_and_recover_readiness() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target = mesh_target(&mut app, Vec3A::splat(0.5));
        let controller =
            standalone_world_tooltip(&mut app, target, fixed_size_tooltip(Vec2::new(0.02, 0.01)));
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}, diagnostics={:?}",
            app.world().get::<TooltipPlacementState>(controller),
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .copied()
                .collect::<Vec<_>>()
        );

        app.world_mut().entity_mut(target).insert(AnchoredTo::new(
            controller,
            AnchorSite::from(Anchor::Center),
            AnchorSite::from(Anchor::Center),
        ));
        app.update();
        assert!(
            app.world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .any(|entry| entry.source == controller)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        app.world_mut().entity_mut(target).remove::<AnchoredTo>();
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut()
            .entity_mut(target)
            .remove::<GlobalTransform>();
        app.update();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        let target_transform = app
            .world()
            .get::<Transform>(target)
            .copied()
            .unwrap_or_default();
        app.world_mut()
            .entity_mut(target)
            .insert(GlobalTransform::from(target_transform));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
    }

    #[test]
    fn general_screen_target_transfers_secondary_window_context_and_becomes_ready() {
        let mut app = materialization_app();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let window = app
            .world_mut()
            .spawn(Window {
                resolution: TEST_VIEWPORT.as_uvec2().into(),
                ..Default::default()
            })
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(350.0, 260.0), Vec2::new(100.0, 80.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let target_data = crate::screen_space::ScreenAnchorTarget::new(
            bounds,
            window,
            37,
            RenderLayers::layer(6),
            Unit::Pixels,
        );
        let handle = app
            .world_mut()
            .commands()
            .screen_anchor_target(target, target_data);
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, fixed_size_tooltip(Vec2::new(120.0, 40.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let materialized = app.world().get::<MaterializedTooltip>(controller);
        assert!(materialized.is_some());
        if let Some(materialized) = materialized {
            assert_eq!(
                materialized.presentation,
                TooltipPresentationSpace::Screen {
                    window,
                    camera_order: 37
                }
            );
            assert_eq!(materialized.render_layers, RenderLayers::layer(6));
            assert_eq!(materialized.layout_unit(), Unit::Pixels);
        }
        let coordinate_space = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::coordinate_space);
        assert!(matches!(
            coordinate_space,
            Some(CoordinateSpace::Screen {
                window: WindowRef::Entity(entity),
                camera_order: 37,
                ..
            }) if *entity == window
        ));
        assert!(
            app.world()
                .get::<crate::panel::ResolvedScreenPanelPosition>(controller)
                .is_some_and(|position| position.anchor_position.is_some())
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        assert_eq!(
            app.world().get::<PanelPicking>(controller),
            Some(&PanelPicking::PASS_THROUGH)
        );
        let local = app.world().get::<Transform>(controller);
        let global = app.world().get::<GlobalTransform>(controller);
        assert!(local.is_some());
        assert!(global.is_some());
        if let (Some(local), Some(global)) = (local, global) {
            assert!(
                global
                    .translation()
                    .abs_diff_eq(local.translation, GEOMETRY_EPSILON)
            );
        }
    }

    #[test]
    fn changed_screen_target_camera_order_and_render_layers_invalidate_readiness() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(350.0, 260.0), Vec2::new(100.0, 80.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let camera_order = 37;
        let render_layers = RenderLayers::layer(6);
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                camera_order,
                render_layers.clone(),
                Unit::Pixels,
            ),
        );
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, fixed_size_tooltip(Vec2::new(120.0, 40.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                camera_order + 1,
                render_layers.clone(),
                Unit::Pixels,
            ));
        app.update();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                camera_order,
                render_layers,
                Unit::Pixels,
            ));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                camera_order,
                RenderLayers::layer(7),
                Unit::Pixels,
            ));
        app.update();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn direct_standalone_component_replacement_does_not_update_materialized_state() {
        let mut app = materialization_app();
        app.init_resource::<TooltipPlacementRunCount>();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let initial_bounds = PanelScreenBounds::new(Vec2::new(80.0, 60.0), Vec2::new(40.0, 20.0));
        let moved_bounds = PanelScreenBounds::new(Vec2::new(140.0, 100.0), Vec2::new(40.0, 20.0));
        assert!(initial_bounds.is_ok());
        assert!(moved_bounds.is_ok());
        let (Some(initial_bounds), Some(moved_bounds)) = (initial_bounds.ok(), moved_bounds.ok())
        else {
            return;
        };
        let camera_order = 5;
        let render_layers = RenderLayers::layer(3);
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                initial_bounds,
                window,
                camera_order,
                render_layers.clone(),
                Unit::Pixels,
            ),
        );
        let original = fixed_size_tooltip(Vec2::new(60.0, 30.0))
            .source_anchor(Anchor::BottomLeft)
            .target_anchor(Anchor::TopRight)
            .offset(PanelAnchorOffset::new(Px(4.0), Px(6.0)));
        let original_attachment = original.attachment();
        let controller = app.world_mut().commands().spawn_tooltip(handle, original);
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        app.update();
        app.update();

        let original_panel_size = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(|panel| Vec2::new(panel.width(), panel.height()));
        let settled_runs = app.world().resource::<TooltipPlacementRunCount>().screen;
        let replacement = fixed_size_tooltip(Vec2::new(140.0, 70.0))
            .source_anchor(Anchor::TopCenter)
            .target_anchor(Anchor::BottomCenter);
        app.world_mut().entity_mut(controller).insert(replacement);
        app.update();
        assert_eq!(
            app.world().resource::<TooltipPlacementRunCount>().screen,
            settled_runs,
        );

        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                moved_bounds,
                window,
                camera_order,
                render_layers,
                Unit::Pixels,
            ));
        app.update();

        let materialized = app.world().get::<MaterializedTooltip>(controller);
        let live_tooltip = app.world().get::<Tooltip>(controller);
        assert!(materialized.is_some());
        assert!(live_tooltip.is_some());
        if let (Some(materialized), Some(live_tooltip)) = (materialized, live_tooltip) {
            assert_eq!(materialized.authored_attachment, original_attachment);
            assert_ne!(materialized.authored_attachment, live_tooltip.attachment());
        }
        assert_eq!(
            app.world()
                .get::<TooltipPlacementState>(controller)
                .map(|placement| placement.result),
            Some(crate::widgets::tooltip::placement::TooltipPlacementResult::Fits),
        );
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(controller)
                .map(|panel| Vec2::new(panel.width(), panel.height())),
            original_panel_size,
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
        );
    }
}
