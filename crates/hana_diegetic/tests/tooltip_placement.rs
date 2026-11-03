//! Placement as observed by an application using the public tooltip API.
#![allow(
    clippy::expect_used,
    reason = "invalid headless test fixtures should fail immediately"
)]

use std::time::Duration;

use bevy::camera::PerspectiveProjection;
use bevy::camera::Projection;
use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::InheritedVisibility;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::visibility::ViewVisibility;
use bevy::ecs::system::RunSystemOnce;
use bevy::mesh::Mesh3d;
use bevy::picking::hover::PickingInteraction;
use bevy::picking::mesh_picking::ray_cast::MeshRayCast;
use bevy::picking::mesh_picking::ray_cast::MeshRayCastSettings;
use bevy::picking::mesh_picking::ray_cast::RayCastVisibility;
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::transform::TransformPlugin;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::ClearWidgetFocus;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::Dimension;
use hana_diegetic::El;
use hana_diegetic::HeadlessDiegeticUiPlugin;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::PanelAnchorOffset;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelPlane;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::RequestWidgetFocus;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Tooltip;
use hana_diegetic::TooltipShown;
use hana_valence::AnchorFrame;
use hana_valence::AnchorPose;
use hana_valence::AnchorSite;
use hana_valence::AnchoredTo;
use hana_valence::Displacement;
use hana_valence::ResolvedAnchorGeometry;

const VIEW: Vec2 = Vec2::new(800.0, 600.0);
const MARGIN: f32 = 8.0;

#[derive(Resource, Default)]
struct Shows(Vec<Entity>);

fn record_show(event: On<TooltipShown>, mut shows: ResMut<Shows>) {
    shows.0.push(event.event_target());
}

struct Scene {
    app:     App,
    view:    Vec2,
    window:  Entity,
    camera:  Entity,
    host:    Entity,
    control: Entity,
    tooltip: Entity,
}

impl Scene {
    fn new(control_position: Vec3, declaration: Tooltip) -> Self {
        Self::with_view(VIEW, control_position, declaration)
    }

    fn with_view(view: Vec2, control_position: Vec3, declaration: Tooltip) -> Self {
        let mut tree = LayoutBuilder::new(80.0, 40.0);
        tree.with(
            El::new()
                .size(80.0, 40.0)
                .button("control")
                .tooltip(declaration),
            |_| {},
        );
        let host = (DiegeticPanel::world()
            .size(Pt(80.0), Pt(40.0))
            .world_width(0.8)
            .with_tree(tree.build())
            .build())
        .expect("host panel");
        Self::with_host(view, Transform::from_translation(control_position), host)
    }

    /// Opens a scene around `host`, whose widget named `control` carries the tooltip.
    fn with_host(view: Vec2, host_transform: Transform, host: DiegeticPanel) -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Shows>()
            .add_observer(record_show)
            .add_plugins(HeadlessDiegeticUiPlugin);
        let mut window_component = Window::default();
        window_component.resolution.set(view.x, view.y);
        let window = app
            .world_mut()
            .spawn((window_component, PrimaryWindow))
            .id();
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: view.as_uvec2(),
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(view.x, view.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        let camera_transform = Transform::from_xyz(0.0, 0.0, 5.0);
        let camera = app
            .world_mut()
            .spawn((
                camera,
                RenderTarget::Window(WindowRef::Entity(window)),
                camera_transform,
                GlobalTransform::from(camera_transform),
            ))
            .id();

        let host = app.world_mut().spawn((host, host_transform)).id();
        for _ in 0..4 {
            app.update();
        }
        let control = ((app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(host, &PanelElementId::named("control"))
            }))
        .expect("widget query"))
        .expect("control widget");
        let tooltip = (app
            .world()
            .get::<hana_diegetic::Tooltips>(control)
            .and_then(|controllers| controllers.iter().next()))
        .expect("tooltip controller");
        Self {
            app,
            view,
            window,
            camera,
            host,
            control,
            tooltip,
        }
    }

    fn show(&mut self) {
        self.app.world_mut().trigger(RequestWidgetFocus {
            window: self.window,
            widget: self.control,
        });
        for _ in 0..10 {
            self.app.update();
        }
    }

    fn shown_count(&self) -> usize { self.app.world().resource::<Shows>().0.len() }

    fn screen_panel(&mut self, anchor: Anchor, size: Vec2) -> Entity {
        let panel = (DiegeticPanel::screen()
            .size(Px(size.x), Px(size.y))
            .anchor(anchor)
            .window_entity(self.window)
            .with_tree(LayoutBuilder::new(size.x, size.y).build())
            .build())
        .expect("screen panel");
        let entity = self.app.world_mut().spawn(panel).id();
        for _ in 0..2 {
            self.app.update();
        }
        entity
    }

    fn bounds(&self, panel_entity: Entity) -> Rect {
        let world = self.app.world();
        let panel = (world.get::<DiegeticPanel>(panel_entity)).expect("panel");
        let transform = (world.get::<GlobalTransform>(panel_entity)).expect("panel transform");
        let plane = (PanelPlane::from_panel(panel, transform)).expect("panel plane");
        let camera = (world.get::<Camera>(self.camera)).expect("camera");
        let camera_transform =
            (world.get::<GlobalTransform>(self.camera)).expect("camera transform");
        let points = [
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomRight,
            Anchor::BottomLeft,
        ]
        .map(|anchor| {
            (camera.world_to_viewport(camera_transform, plane.point(anchor)))
                .expect("projected panel corner")
        });
        let min = (points.into_iter().reduce(Vec2::min)).expect("corners");
        let max = (points.into_iter().reduce(Vec2::max)).expect("corners");
        Rect::from_corners(min, max)
    }

    fn assert_visible_inside_view(&self) -> Rect {
        assert_eq!(self.shown_count(), 1, "tooltip must reveal exactly once");
        let bounds = self.bounds(self.tooltip);
        assert!(
            bounds.min.x >= MARGIN - 1.0 && bounds.min.y >= MARGIN - 1.0,
            "tooltip begins outside placement area: {bounds:?}"
        );
        assert!(
            bounds.max.x <= self.view.x - MARGIN + 1.0
                && bounds.max.y <= self.view.y - MARGIN + 1.0,
            "tooltip ends outside placement area: {bounds:?}"
        );
        bounds
    }
}

fn sized_tooltip(width: f32, height: f32) -> Tooltip {
    let mut tooltip = Tooltip::new(El::new()).show_after(Duration::ZERO);
    tooltip.with(El::new().size(width, height), |_| {});
    tooltip
}

fn clock_tooltip_scene(button_y: f32) -> Scene {
    let declaration = host_scaled_tooltip(120.0, 30.0)
        .source_anchor(Anchor::TopRight)
        .target_anchor(Anchor::BottomRight);
    let mut tree = LayoutBuilder::new(240.0, 180.0);
    tree.with(El::column().size(240.0, 180.0), |column| {
        column.with(El::new().size(240.0, button_y), |_| {});
        column.with(El::row().size(240.0, 24.0), |row| {
            row.with(El::new().size(100.0, 24.0), |_| {});
            row.with(
                El::new()
                    .size(80.0, 24.0)
                    .button("control")
                    .tooltip(declaration),
                |button| {
                    button.text("Clock");
                },
            );
        });
    });
    let panel = DiegeticPanel::world()
        .size(Pt(240.0), Pt(180.0))
        .anchor(Anchor::Center)
        .world_width(2.4)
        .with_tree(tree.build())
        .build()
        .expect("Clock card");
    let mut scene = Scene::with_host(VIEW, Transform::default(), panel);
    let card = Rect::from_center_size(Vec2::ZERO, Vec2::new(2.4, 1.8));
    scene.app.world_mut().entity_mut(scene.control).insert(
        hana_diegetic::TooltipCardOccupiedRegions::new(scene.host, card, Vec::new()).inside_only(),
    );
    scene.show();
    scene
}

fn assert_clock_tooltip_clears_its_button(scene: &Scene, button_y: f32) -> Rect {
    let painted = scene.assert_painted_inside(scene.bounds(scene.host), "Clock card");
    let button = scene.host_rect(Vec2::new(100.0, button_y), Vec2::new(80.0, 24.0));
    assert!(
        painted.intersect(button).is_empty(),
        "tooltip must clear its Clock button and label: tooltip={painted:?}, button={button:?}"
    );
    button
}

#[test]
fn authored_clock_tooltip_side_wins_when_it_fits() {
    let scene = clock_tooltip_scene(50.0);
    let button = assert_clock_tooltip_clears_its_button(&scene, 50.0);
    let body = scene.bounds(scene.tooltip);
    assert!(
        body.min.y > button.max.y,
        "authored below side: {body:?} {button:?}"
    );
    assert!(
        (body.max.x - button.max.x).abs() < 1.0,
        "authored right anchors must stay aligned: {body:?} {button:?}"
    );
}

#[test]
fn authored_clock_tooltip_fallback_clears_its_button_and_label() {
    let scene = clock_tooltip_scene(140.0);
    let button = assert_clock_tooltip_clears_its_button(&scene, 140.0);
    let body = scene.bounds(scene.tooltip);
    assert!(
        body.max.y < button.min.y,
        "below cannot fit; use above: {body:?} {button:?}"
    );
}

#[test]
fn right_edge_flips_left_and_stays_inside_the_placement_area() {
    let tooltip = sized_tooltip(40.0, 20.0)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    let mut scene = Scene::new(Vec3::new(2.3, 0.0, 0.0), tooltip);
    scene.show();
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.max.x <= control.min.x + 1.0,
        "right preference should flip left: {tip:?} {control:?}"
    );
}

#[test]
fn bottom_edge_flips_above_with_default_attachment() {
    let mut scene = Scene::new(Vec3::new(0.0, -1.85, 0.0), sized_tooltip(45.0, 25.0));
    scene.show();
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.max.y <= control.min.y + 1.0,
        "below preference should flip above: {tip:?} {control:?}"
    );
}

#[test]
fn a_corner_tooltip_shifts_without_losing_its_control() {
    let mut scene = Scene::new(Vec3::new(-2.3, 1.8, 0.0), sized_tooltip(90.0, 20.0));
    scene.show();
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.min.x <= control.center().x && tip.max.x >= control.center().x,
        "shift must leave the control anchor within the tooltip span: {tip:?} {control:?}"
    );
}

#[test]
fn docked_help_panel_reserves_the_bottom_band() {
    let mut scene = Scene::new(Vec3::new(-1.3, -0.85, 0.0), sized_tooltip(70.0, 25.0));
    scene.screen_panel(Anchor::BottomLeft, Vec2::new(360.0, 140.0));
    scene.show();
    let tip = scene.assert_visible_inside_view();
    assert!(
        tip.max.y <= VIEW.y - 140.0 - MARGIN + 1.0,
        "the help panel reserves a bottom band: {tip:?}"
    );
}

#[test]
fn docked_camera_panel_reserves_the_right_band() {
    let tooltip = sized_tooltip(70.0, 25.0)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    let mut scene = Scene::new(Vec3::new(1.2, 0.0, 0.0), tooltip);
    scene.screen_panel(Anchor::CenterRight, Vec2::new(180.0, VIEW.y));
    scene.show();
    let tip = scene.assert_visible_inside_view();
    assert!(
        tip.max.x <= VIEW.x - 180.0 - MARGIN + 1.0,
        "the camera panel reserves a right band: {tip:?}"
    );
}

#[test]
fn floating_screen_panel_cannot_cover_a_new_tooltip() {
    let mut scene = Scene::new(Vec3::new(0.0, 0.2, 0.0), sized_tooltip(70.0, 20.0));
    scene.screen_panel(Anchor::Center, Vec2::new(180.0, 90.0));
    scene.show();
    let tip = scene.assert_visible_inside_view();
    let overlay = Rect::from_center_size(VIEW / 2.0, Vec2::new(180.0, 90.0));
    assert!(
        tip.max.x <= overlay.min.x
            || tip.min.x >= overlay.max.x
            || tip.max.y <= overlay.min.y
            || tip.min.y >= overlay.max.y,
        "floating overlay intersects tooltip: {tip:?} {overlay:?}"
    );
}

#[test]
fn reference_fully_under_a_docked_panel_is_hidden() {
    let mut scene = Scene::new(Vec3::new(-1.1, -1.5, 0.0), sized_tooltip(50.0, 20.0));
    scene.screen_panel(Anchor::BottomLeft, Vec2::new(VIEW.x, 220.0));
    scene.show();
    assert_eq!(
        scene.shown_count(),
        0,
        "the control is fully behind the help band"
    );
}

#[test]
fn a_scene_mesh_between_camera_and_preferred_side_forces_a_flip() {
    let mut scene = Scene::new(Vec3::ZERO, sized_tooltip(60.0, 20.0));
    let mesh = scene
        .app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 0.25, 0.02));
    let occluder = scene
        .app
        .world_mut()
        .spawn((
            Mesh3d(mesh),
            Aabb {
                center:       Vec3A::ZERO,
                half_extents: Vec3A::new(0.5, 0.125, 0.01),
            },
            InheritedVisibility::VISIBLE,
            ViewVisibility::VISIBLE,
            Transform::from_xyz(0.0, -0.3, 2.0),
        ))
        .id();
    for _ in 0..2 {
        scene.app.update();
    }
    scene.show();
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.max.y <= control.min.y + 1.0,
        "the preferred side is behind the mesh: {tip:?} {control:?}"
    );
    let world = scene.app.world();
    let panel = (world.get::<DiegeticPanel>(scene.tooltip)).expect("tooltip panel");
    let transform = (world.get::<GlobalTransform>(scene.tooltip)).expect("tooltip transform");
    let plane = (PanelPlane::from_panel(panel, transform)).expect("tooltip plane");
    let camera_origin = (world.get::<GlobalTransform>(scene.camera))
        .expect("camera transform")
        .translation();
    let sample_points = [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::BottomLeft,
        Anchor::Center,
    ]
    .map(|anchor| plane.point(anchor));
    let clear = (scene
        .app
        .world_mut()
        .run_system_once(move |mut ray_cast: MeshRayCast| {
            sample_points.into_iter().all(|point| {
                let ray_delta = point - camera_origin;
                let direction = (Dir3::new(ray_delta)).expect("ray direction");
                let only_occluder = |entity| entity == occluder;
                let settings = MeshRayCastSettings::default()
                    .with_visibility(RayCastVisibility::Visible)
                    .with_filter(&only_occluder);
                ray_cast
                    .cast_ray(Ray3d::new(camera_origin, direction), &settings)
                    .iter()
                    .all(|(_, hit)| hit.distance >= ray_delta.length() - 0.001)
            })
        }))
    .expect("sample raycast");
    assert!(
        clear,
        "one of the visible tooltip's corners or centre is behind the scene mesh"
    );
}

#[test]
fn words_reflow_to_a_narrower_side_and_still_fit_vertically() {
    let mut declaration = Tooltip::new(El::new()).show_after(Duration::ZERO);
    declaration.text(
        Text::new("alpha beta gamma delta ".repeat(50), TextStyle::new(18.0)).wrap(TextWrap::Words),
    );
    let mut wide = Scene::new(Vec3::ZERO, declaration.clone());
    wide.show();
    let wide_bounds = wide.assert_visible_inside_view();
    let mut narrow = Scene::with_view(Vec2::new(140.0, 300.0), Vec3::ZERO, declaration);
    narrow.show();
    let tip = narrow.assert_visible_inside_view();
    assert!(
        tip.height() > wide_bounds.height() + 0.25,
        "reflow should make the narrow tooltip taller: wide={wide_bounds:?}, narrow={tip:?}"
    );
}

#[test]
fn an_unbreakable_tooltip_that_cannot_fit_is_never_shown() {
    let mut declaration = Tooltip::new(El::new()).show_after(Duration::ZERO);
    declaration.text(Text::new("W".repeat(2_000), TextStyle::new(18.0)).wrap(TextWrap::None));
    declaration.with(El::new().size(20_000.0, 20_000.0), |_| {});
    let mut scene = Scene::new(Vec3::ZERO, declaration);
    scene.show();
    assert_eq!(scene.shown_count(), 0);
}

#[test]
fn a_reference_outside_the_camera_view_cannot_reveal() {
    let mut scene = Scene::new(Vec3::new(20.0, 0.0, 0.0), sized_tooltip(40.0, 20.0));
    scene.show();
    assert_eq!(scene.shown_count(), 0);
}

#[test]
fn the_reveal_uses_the_camera_position_from_that_frame() {
    let tooltip = sized_tooltip(40.0, 20.0)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    let mut scene = Scene::new(Vec3::new(2.3, 0.0, 0.0), tooltip);
    (scene.app.world_mut().get_mut::<Transform>(scene.camera))
        .expect("camera transform")
        .translation
        .x = 2.3;
    scene.show();
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.min.x >= control.max.x - 1.0,
        "camera moved before reveal, so the preferred right side fits: {tip:?} {control:?}"
    );
}

#[test]
fn a_new_show_redecides_the_side_after_a_camera_pan() {
    let tooltip = sized_tooltip(40.0, 20.0)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    let mut scene = Scene::new(Vec3::new(2.3, 0.0, 0.0), tooltip);
    scene.show();
    assert!(scene.bounds(scene.tooltip).center().x < scene.bounds(scene.host).center().x);
    scene.app.world_mut().trigger(ClearWidgetFocus {
        window: scene.window,
    });
    for _ in 0..2 {
        scene.app.update();
    }
    (scene.app.world_mut().get_mut::<Transform>(scene.camera))
        .expect("camera transform")
        .translation
        .x = 2.3;
    scene.show();
    let tip = scene.bounds(scene.tooltip);
    let control = scene.bounds(scene.host);
    assert_eq!(
        scene.shown_count(),
        2,
        "a second show should emit a second event"
    );
    assert!(
        tip.center().x > control.center().x,
        "the new show should choose right after the pan: {tip:?} {control:?}"
    );
}

#[test]
fn hovering_noninteractive_child_keeps_the_parent_tooltip() {
    let mut scene = Scene::new(Vec3::ZERO, sized_tooltip(40.0, 20.0));
    scene.show();
    scene.assert_visible_inside_view();
    let title = scene
        .app
        .world_mut()
        .spawn((ChildOf(scene.control), PickingInteraction::Hovered))
        .id();
    scene.app.world_mut().trigger(ClearWidgetFocus {
        window: scene.window,
    });
    for _ in 0..2 {
        scene.app.update();
    }
    assert_eq!(
        scene.app.world().get::<Visibility>(scene.tooltip),
        Some(&Visibility::Inherited),
        "hover over child {title:?} should keep parent tooltip visible"
    );
    assert_eq!(scene.shown_count(), 1);
}

#[test]
fn moving_the_control_after_reveal_carries_the_tooltip_rigidly() {
    let mut scene = Scene::new(Vec3::ZERO, sized_tooltip(45.0, 20.0));
    scene.show();
    scene.assert_visible_inside_view();
    let before = scene.bounds(scene.tooltip).center() - scene.bounds(scene.host).center();
    (scene.app.world_mut().get_mut::<Transform>(scene.host))
        .expect("host transform")
        .translation
        .x += 0.5;
    for _ in 0..3 {
        scene.app.update();
    }
    let after = scene.bounds(scene.tooltip).center() - scene.bounds(scene.host).center();
    assert!(
        (after - before).length() < 1.0,
        "relative position changed: {before:?} -> {after:?}"
    );
    assert_eq!(scene.shown_count(), 1, "a move must not reveal again");
}

#[test]
fn world_tooltip_fits_and_reveals_while_the_camera_moves_every_frame() {
    let mut declaration = Tooltip::new(El::column()).show_after(Duration::ZERO);
    declaration.text("Put tools back");
    let mut scene = Scene::new(Vec3::ZERO, declaration);
    scene.app.world_mut().trigger(RequestWidgetFocus {
        window: scene.window,
        widget: scene.control,
    });
    for frame in 0..40 {
        scene
            .app
            .world_mut()
            .get_mut::<Transform>(scene.camera)
            .expect("camera pose")
            .translation
            .z += 0.0025;
        scene.app.update();
        if let Some(panel) = scene.app.world().get::<DiegeticPanel>(scene.tooltip) {
            eprintln!(
                "WORLD_HELP_FRAME {frame} fitted {}x{} shows {}",
                panel.world_width(),
                panel.world_height(),
                scene.shown_count()
            );
            if scene.shown_count() > 0 {
                assert!(
                    panel.world_width() > 0.0 && panel.world_height() > 0.0,
                    "a revealed help lost its fitted dimensions during motion"
                );
            }
        }
    }
    let panel = scene
        .app
        .world()
        .get::<DiegeticPanel>(scene.tooltip)
        .expect("help panel");
    assert!(
        panel.world_width() > 0.0 && panel.world_height() > 0.0,
        "moving-camera help never reaches fitted dimensions"
    );
    scene.assert_visible_inside_view();
    for _ in 0..8 {
        scene.app.update();
    }
    scene.assert_visible_inside_view();
    let settled = scene
        .app
        .world()
        .get::<DiegeticPanel>(scene.tooltip)
        .expect("settled help");
    let settled_size = Vec2::new(settled.world_width(), settled.world_height());
    let final_camera = *scene
        .app
        .world()
        .get::<Transform>(scene.camera)
        .expect("settled camera");
    let mut declaration = Tooltip::new(El::column()).show_after(Duration::ZERO);
    declaration.text("Put tools back");
    let mut fresh = Scene::new(Vec3::ZERO, declaration);
    fresh
        .app
        .world_mut()
        .entity_mut(fresh.camera)
        .insert(final_camera);
    fresh.show();
    let panel = fresh
        .app
        .world()
        .get::<DiegeticPanel>(fresh.tooltip)
        .expect("fresh help");
    let fresh_size = Vec2::new(panel.world_width(), panel.world_height());
    assert!(
        settled_size.abs_diff_eq(fresh_size, 0.000_001),
        "settled raster grid differs from a fresh fit: {settled_size:?} vs {fresh_size:?}"
    );
}

#[test]
fn panning_after_reveal_keeps_the_chosen_side_even_when_it_leaves_view() {
    let tooltip = sized_tooltip(40.0, 20.0)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    let mut scene = Scene::new(Vec3::new(2.3, 0.0, 0.0), tooltip);
    scene.show();
    scene.assert_visible_inside_view();
    let original_side =
        scene.bounds(scene.tooltip).center().x - scene.bounds(scene.host).center().x;
    (scene.app.world_mut().get_mut::<Transform>(scene.camera))
        .expect("camera transform")
        .translation
        .x += 8.0;
    for _ in 0..3 {
        scene.app.update();
    }
    let moved_tip = scene.bounds(scene.tooltip);
    let current_side = moved_tip.center().x - scene.bounds(scene.host).center().x;
    assert!(
        moved_tip.max.x < 0.0,
        "the pan must take the tooltip off-screen: {moved_tip:?}"
    );
    assert!(
        original_side < 0.0 && current_side < 0.0,
        "pan changed the chosen side"
    );
    assert_eq!(scene.shown_count(), 1);
}

#[test]
fn orbiting_after_reveal_does_not_move_or_reveal_the_tooltip_again() {
    let mut scene = Scene::new(Vec3::ZERO, sized_tooltip(60.0, 20.0));
    scene.show();
    scene.assert_visible_inside_view();
    let before = (scene.app.world().get::<GlobalTransform>(scene.tooltip))
        .expect("tooltip transform")
        .translation();
    (scene.app.world_mut().get_mut::<Transform>(scene.camera))
        .expect("camera transform")
        .rotation = Quat::from_rotation_y(1.5);
    for _ in 0..4 {
        scene.app.update();
    }
    let after = (scene.app.world().get::<GlobalTransform>(scene.tooltip))
        .expect("tooltip transform")
        .translation();
    assert!(
        (after - before).length() < 1e-5,
        "camera orbit changed a frozen world tooltip: {before:?} -> {after:?}"
    );
    assert_eq!(scene.shown_count(), 1);
    assert_eq!(
        scene.app.world().get::<Visibility>(scene.tooltip),
        Some(&Visibility::Inherited)
    );
}

#[test]
fn second_reveal_redecides_after_a_pan_while_visible() {
    let tooltip = sized_tooltip(40.0, 20.0)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    let mut scene = Scene::new(Vec3::new(2.3, 0.0, 0.0), tooltip);
    scene.show();
    assert!(scene.bounds(scene.tooltip).center().x < scene.bounds(scene.host).center().x);
    scene
        .app
        .world_mut()
        .get_mut::<Transform>(scene.camera)
        .expect("camera")
        .translation
        .x = 2.3;
    for _ in 0..3 {
        scene.app.update();
    }
    scene.app.world_mut().trigger(ClearWidgetFocus {
        window: scene.window,
    });
    for _ in 0..3 {
        scene.app.update();
    }
    scene.show();
    assert_eq!(scene.shown_count(), 2);
    assert!(scene.bounds(scene.tooltip).center().x > scene.bounds(scene.host).center().x);
}

#[test]
fn mesh_moved_on_reveal_frame_blocks_the_preferred_side() {
    let mut scene = Scene::new(
        Vec3::ZERO,
        sized_tooltip(60.0, 20.0).show_after(Duration::from_millis(100)),
    );
    let mesh = scene
        .app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 0.25, 0.02));
    let occluder = scene
        .app
        .world_mut()
        .spawn((
            Mesh3d(mesh),
            Aabb {
                center:       Vec3A::ZERO,
                half_extents: Vec3A::new(0.5, 0.125, 0.01),
            },
            InheritedVisibility::VISIBLE,
            ViewVisibility::VISIBLE,
            Transform::from_xyz(100.0, -0.3, 2.0),
        ))
        .id();
    scene.app.update();
    scene.app.world_mut().trigger(RequestWidgetFocus {
        window: scene.window,
        widget: scene.control,
    });
    for _ in 0..3 {
        scene.app.update();
    }
    scene
        .app
        .world_mut()
        .get_mut::<Transform>(occluder)
        .expect("occluder")
        .translation
        .x = 0.0;
    for _ in 0..8 {
        scene.app.update();
    }
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.max.y <= control.min.y + 1.0,
        "moved mesh must force flip: {tip:?} {control:?}"
    );
}

#[test]
fn anchor_resolver_moves_mesh_on_reveal_frame_before_reveal_check() {
    let mut scene = Scene::new(
        Vec3::ZERO,
        sized_tooltip(60.0, 20.0).show_after(Duration::from_millis(100)),
    );
    let geometry =
        ResolvedAnchorGeometry::try_new([(AnchorSite::Center, AnchorFrame::default())], [])
            .expect("centre anchor geometry");
    let anchor_transform = Transform::from_xyz(0.0, -0.3, 2.0);
    let anchor = scene
        .app
        .world_mut()
        .spawn((
            geometry.clone(),
            anchor_transform,
            GlobalTransform::from(anchor_transform),
        ))
        .id();
    let mesh = scene
        .app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 0.25, 0.02));
    let occluder = scene
        .app
        .world_mut()
        .spawn((
            Mesh3d(mesh),
            Aabb {
                center:       Vec3A::ZERO,
                half_extents: Vec3A::new(0.5, 0.125, 0.01),
            },
            InheritedVisibility::VISIBLE,
            ViewVisibility::VISIBLE,
            geometry,
            AnchoredTo::new(anchor, AnchorSite::Center, AnchorSite::Center),
            AnchorPose {
                translation: Displacement::from(Vec3::new(100.0, 0.0, 0.0)),
                ..Default::default()
            },
            Transform::from_xyz(100.0, -0.3, 2.0),
            GlobalTransform::from(Transform::from_xyz(100.0, -0.3, 2.0)),
        ))
        .id();
    scene.app.update();
    scene.app.world_mut().trigger(RequestWidgetFocus {
        window: scene.window,
        widget: scene.control,
    });
    for _ in 0..4 {
        scene.app.update();
    }
    assert_eq!(
        scene.shown_count(),
        0,
        "timer has not reached the reveal frame"
    );
    scene
        .app
        .world_mut()
        .get_mut::<AnchorPose>(occluder)
        .expect("anchored occluder pose")
        .translation = Displacement::from(Vec3::ZERO);
    scene.app.update();
    assert_eq!(
        scene.shown_count(),
        0,
        "resolver-moved mesh must withhold the stale placement"
    );
    for _ in 0..8 {
        scene.app.update();
    }
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.max.y <= control.min.y + 1.0,
        "resolved mesh must force a flip: {tip:?} {control:?}"
    );
}

#[test]
fn resolver_moved_mesh_covering_control_withholds_reveal() {
    let mut scene = Scene::new(
        Vec3::ZERO,
        sized_tooltip(60.0, 20.0).show_after(Duration::from_millis(100)),
    );
    let geometry =
        ResolvedAnchorGeometry::try_new([(AnchorSite::Center, AnchorFrame::default())], [])
            .expect("centre anchor geometry");
    let anchor_transform = Transform::from_xyz(0.0, 0.0, 2.0);
    let anchor = scene
        .app
        .world_mut()
        .spawn((
            geometry.clone(),
            anchor_transform,
            GlobalTransform::from(anchor_transform),
        ))
        .id();
    let mesh = scene
        .app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 0.5, 0.02));
    let occluder = scene
        .app
        .world_mut()
        .spawn((
            Mesh3d(mesh),
            Aabb {
                center:       Vec3A::ZERO,
                half_extents: Vec3A::new(0.5, 0.25, 0.01),
            },
            InheritedVisibility::VISIBLE,
            ViewVisibility::VISIBLE,
            geometry,
            AnchoredTo::new(anchor, AnchorSite::Center, AnchorSite::Center),
            AnchorPose {
                translation: Displacement::from(Vec3::new(100.0, 0.0, 0.0)),
                ..Default::default()
            },
            Transform::from_xyz(100.0, 0.0, 2.0),
            GlobalTransform::from(Transform::from_xyz(100.0, 0.0, 2.0)),
        ))
        .id();
    scene.app.update();
    scene.app.world_mut().trigger(RequestWidgetFocus {
        window: scene.window,
        widget: scene.control,
    });
    for _ in 0..4 {
        scene.app.update();
    }
    assert_eq!(scene.shown_count(), 0);
    scene
        .app
        .world_mut()
        .get_mut::<AnchorPose>(occluder)
        .expect("anchored occluder pose")
        .translation = Displacement::from(Vec3::ZERO);
    for _ in 0..8 {
        scene.app.update();
    }
    assert_eq!(scene.shown_count(), 0, "covered control must stay hidden");
}

#[test]
fn hidden_docked_panel_does_not_reserve_the_right_band() {
    let tooltip = sized_tooltip(70.0, 25.0)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    let mut scene = Scene::new(Vec3::new(1.2, 0.0, 0.0), tooltip);
    let panel = scene.screen_panel(Anchor::CenterRight, Vec2::new(180.0, VIEW.y));
    scene
        .app
        .world_mut()
        .entity_mut(panel)
        .insert(Visibility::Hidden);
    for _ in 0..2 {
        scene.app.update();
    }
    scene.show();
    let tip = scene.assert_visible_inside_view();
    assert!(tip.min.x >= scene.bounds(scene.host).max.x - 1.0);
}

#[test]
fn floating_panel_fully_covering_the_control_suppresses_reveal() {
    let mut scene = Scene::new(Vec3::ZERO, sized_tooltip(50.0, 20.0));
    scene.screen_panel(Anchor::Center, Vec2::new(300.0, 160.0));
    scene.show();
    assert_eq!(
        scene.shown_count(),
        0,
        "control={:?}",
        scene.bounds(scene.host)
    );
}

#[test]
fn world_reflows_into_room_before_far_floating_panel() {
    let mut tooltip = Tooltip::new(El::new())
        .show_after(Duration::ZERO)
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    tooltip.text(Text::new("alpha ".repeat(28), TextStyle::new(20.0)).wrap(TextWrap::Words));
    let mut scene = Scene::new(Vec3::new(-2.2, 0.0, 0.0), tooltip);
    let overlay = DiegeticPanel::screen()
        .size(Px(150.0), Px(160.0))
        .anchor(Anchor::Center)
        .screen_position(450.0, 300.0)
        .window_entity(scene.window)
        .camera_order(1)
        .with_tree(LayoutBuilder::new(150.0, 160.0).build())
        .build()
        .expect("floating overlay");
    scene.app.world_mut().spawn(overlay);
    for _ in 0..2 {
        scene.app.update();
    }
    scene.show();
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.min.x >= control.max.x - 1.0,
        "tooltip should use the right gap: {tip:?} {control:?}"
    );
    assert!(
        tip.max.x <= 450.0 - 75.0 - MARGIN + 1.0,
        "tooltip overlaps floating panel: {tip:?}"
    );
    assert!(tip.width() > 100.0, "tooltip did not use the gap: {tip:?}");
}

#[test]
fn moving_overlay_away_restores_waiting_world_tooltip_width() {
    let mut tooltip = Tooltip::new(El::new())
        .show_after(Duration::from_millis(500))
        .source_anchor(Anchor::CenterLeft)
        .target_anchor(Anchor::CenterRight);
    tooltip.text(Text::new("alpha ".repeat(100), TextStyle::new(20.0)).wrap(TextWrap::Words));
    let mut scene = Scene::new(Vec3::new(-2.2, 0.0, 0.0), tooltip);
    for y in [245.0, 355.0] {
        let blocker = DiegeticPanel::screen()
            .size(Px(100.0), Px(80.0))
            .anchor(Anchor::Center)
            .screen_position(100.0, y)
            .window_entity(scene.window)
            .camera_order(1)
            .with_tree(LayoutBuilder::new(100.0, 80.0).build())
            .build()
            .expect("above or below blocker");
        scene.app.world_mut().spawn(blocker);
    }
    let overlay = DiegeticPanel::screen()
        .size(Px(150.0), Px(160.0))
        .anchor(Anchor::Center)
        .screen_position(450.0, 300.0)
        .window_entity(scene.window)
        .camera_order(1)
        .with_tree(LayoutBuilder::new(150.0, 160.0).build())
        .build()
        .expect("floating overlay");
    let overlay = scene.app.world_mut().spawn(overlay).id();
    scene.app.update();
    scene.app.world_mut().trigger(RequestWidgetFocus {
        window: scene.window,
        widget: scene.control,
    });
    for _ in 0..8 {
        scene.app.update();
    }
    assert_eq!(scene.shown_count(), 0);
    let narrow = scene
        .app
        .world()
        .get::<DiegeticPanel>(scene.tooltip)
        .expect("tooltip")
        .width();
    assert!(
        narrow < 7200.0,
        "fixture needs side-room reflow: width={narrow}, control={:?}, tooltip={:?}",
        scene.bounds(scene.host),
        scene.bounds(scene.tooltip),
    );
    assert!(
        scene
            .app
            .world_mut()
            .get_mut::<DiegeticPanel>(overlay)
            .expect("overlay")
            .set_screen_position(Vec2::new(750.0, 300.0))
    );
    for _ in 0..30 {
        scene.app.update();
    }
    let expanded = scene
        .app
        .world()
        .get::<DiegeticPanel>(scene.tooltip)
        .expect("tooltip")
        .width();
    assert!(
        expanded > narrow + 50.0,
        "world tooltip stayed narrow: {narrow} -> {expanded}"
    );
    assert_eq!(
        scene.shown_count(),
        1,
        "tooltip should reveal after overlay moves"
    );
}

#[test]
fn mesh_outside_camera_layers_does_not_occlude_tooltip() {
    let mut scene = Scene::new(Vec3::ZERO, sized_tooltip(60.0, 20.0));
    let mesh = scene
        .app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 0.25, 0.02));
    scene.app.world_mut().spawn((
        Mesh3d(mesh),
        RenderLayers::layer(1),
        Aabb {
            center:       Vec3A::ZERO,
            half_extents: Vec3A::new(0.5, 0.125, 0.01),
        },
        InheritedVisibility::VISIBLE,
        ViewVisibility::VISIBLE,
        Transform::from_xyz(0.0, -0.3, 2.0),
    ));
    for _ in 0..2 {
        scene.app.update();
    }
    scene.show();
    let tip = scene.assert_visible_inside_view();
    let control = scene.bounds(scene.host);
    assert!(
        tip.min.y >= control.max.y - 1.0,
        "unrendered mesh cannot force flip: {tip:?} {control:?}"
    );
}

#[test]
fn fixture_errors_panic_without_aborting_the_test_binary() {
    assert!(!include_str!("tooltip_placement.rs").contains(concat!("std::process::", "abort")));
}

/// Tooltip layout pixels the default shadow paints past the body on every side.
const SHADOW_BLUR_SUPPORT_PX: f32 = 8.0;
/// Tooltip layout pixels the default shadow paints below the body past its blur.
const SHADOW_DOWNWARD_OFFSET_PX: f32 = 3.0;
const CARD_WIDTH: f32 = 200.0;
const CARD_HEIGHT: f32 = 120.0;
const CARD_HOST_WIDTH: f32 = 360.0;
const CARD_WORLD_PER_POINT: f32 = 0.01;
/// Puts the card's right edge 2.02 m right of the camera axis: inside a 900 px window, past
/// the right margin of a 750 px one.
const CARD_HOST_X: f32 = 0.02 + CARD_HOST_WIDTH * CARD_WORLD_PER_POINT / 2.0;

impl Scene {
    /// What the tooltip paints, in viewport pixels: its body's corners moved out in its own
    /// plane by the shadow's reach, then projected, so a tilted tooltip's shadow tilts with it.
    fn painted_bounds(&self) -> Rect { self.painted_bounds_after_move(Vec3::ZERO) }

    fn painted_bounds_after_move(&self, movement: Vec3) -> Rect {
        let world = self.app.world();
        let panel = (world.get::<DiegeticPanel>(self.tooltip)).expect("tooltip panel");
        let transform = (world.get::<GlobalTransform>(self.tooltip)).expect("tooltip transform");
        let plane = (PanelPlane::from_panel(panel, transform)).expect("tooltip plane");
        let local_per_layout_px = Dimension::from(Px(1.0)).to_points(1.0) * panel.points_to_world();
        let blur = SHADOW_BLUR_SUPPORT_PX * local_per_layout_px;
        let below = SHADOW_DOWNWARD_OFFSET_PX.mul_add(local_per_layout_px, blur);
        let camera = (world.get::<Camera>(self.camera)).expect("camera");
        let camera_transform =
            (world.get::<GlobalTransform>(self.camera)).expect("camera transform");
        let corners = [
            (Anchor::TopLeft, Vec3::new(-blur, blur, 0.0)),
            (Anchor::TopRight, Vec3::new(blur, blur, 0.0)),
            (Anchor::BottomRight, Vec3::new(blur, -below, 0.0)),
            (Anchor::BottomLeft, Vec3::new(-blur, -below, 0.0)),
        ]
        .map(|(anchor, outset)| {
            let corner =
                plane.point(anchor) + transform.affine().transform_vector3(outset) + movement;
            (camera.world_to_viewport(camera_transform, corner)).expect("projected painted corner")
        });
        let min = (corners.into_iter().reduce(Vec2::min)).expect("corners");
        let max = (corners.into_iter().reduce(Vec2::max)).expect("corners");
        Rect::from_corners(min, max)
    }

    fn assert_painted_inside(&self, area: Rect, name: &str) -> Rect {
        assert_eq!(self.shown_count(), 1, "tooltip must reveal exactly once");
        let painted = self.painted_bounds();
        assert!(
            painted.min.x >= area.min.x - 1.0
                && painted.min.y >= area.min.y - 1.0
                && painted.max.x <= area.max.x + 1.0
                && painted.max.y <= area.max.y + 1.0,
            "painted tooltip leaves {name}: painted {painted:?}, {name} {area:?}"
        );
        painted
    }

    fn placement_area(&self) -> Rect { Rect::from_corners(Vec2::splat(MARGIN), self.view - MARGIN) }

    /// Projects a rectangle given in host-panel points, from the panel's top-left corner.
    fn host_rect(&self, top_left: Vec2, size: Vec2) -> Rect {
        let world = self.app.world();
        let panel = (world.get::<DiegeticPanel>(self.host)).expect("host panel");
        let transform = (world.get::<GlobalTransform>(self.host)).expect("host transform");
        let plane = (PanelPlane::from_panel(panel, transform)).expect("host plane");
        let per_point = plane.size().x / panel.width();
        let camera = (world.get::<Camera>(self.camera)).expect("camera");
        let camera_transform =
            (world.get::<GlobalTransform>(self.camera)).expect("camera transform");
        let corners = [
            top_left,
            top_left + Vec2::new(size.x, 0.0),
            top_left + size,
            top_left + Vec2::new(0.0, size.y),
        ]
        .map(|point| {
            let world_point = plane.origin() + plane.right() * point.x * per_point
                - plane.up() * point.y * per_point;
            (camera.world_to_viewport(camera_transform, world_point)).expect("projected host point")
        });
        let min = (corners.into_iter().reduce(Vec2::min)).expect("corners");
        let max = (corners.into_iter().reduce(Vec2::max)).expect("corners");
        Rect::from_corners(min, max)
    }

    fn slider_centre(&self) -> Vec2 {
        self.host_rect(card_slider_top_left(), Vec2::new(60.0, 20.0))
            .center()
    }
}

#[derive(Clone, Copy)]
enum CardMark {
    TooltipBoundary,
    Unmarked,
}

const fn card_slider_top_left() -> Vec2 { Vec2::new(130.0, 70.0) }

/// A tooltip holding a `width` by `height` block above one line of words. Its unstyled words
/// draw it in the host's points, so its frame and shadow scale with the host as a panel's row
/// tooltips do.
fn host_scaled_tooltip(width: f32, height: f32) -> Tooltip {
    let mut tooltip = Tooltip::new(El::column()).show_after(Duration::ZERO);
    tooltip.with(El::new().size(width, height), |_| {});
    tooltip.text("tip");
    tooltip
}

/// A tooltip authored above the slider, `width` host points wide.
fn above_tooltip(width: f32) -> Tooltip {
    host_scaled_tooltip(width, 20.0)
        .source_anchor(Anchor::BottomCenter)
        .target_anchor(Anchor::TopCenter)
}

/// A host panel wider than its card: the card holds a slider 10 points from its right edge,
/// and a plain column runs past the card to the right.
fn card_host(mark: CardMark, tooltip: Tooltip) -> DiegeticPanel {
    let card = El::column().size(CARD_WIDTH, CARD_HEIGHT);
    let card = match mark {
        CardMark::TooltipBoundary => card.tooltip_boundary(),
        CardMark::Unmarked => card,
    };
    let mut tree = LayoutBuilder::new(CARD_HOST_WIDTH, CARD_HEIGHT);
    tree.with(El::row().size(CARD_HOST_WIDTH, CARD_HEIGHT), |row| {
        row.with(card, |card| {
            card.with(El::new().size(CARD_WIDTH, card_slider_top_left().y), |_| {});
            card.with(El::row().size(CARD_WIDTH, 20.0), |slider_row| {
                slider_row.with(El::new().size(card_slider_top_left().x, 20.0), |_| {});
                slider_row.with(
                    El::new()
                        .size(60.0, 20.0)
                        .slider("control", 0.0..=1.0)
                        .tooltip(tooltip),
                    |_| {},
                );
            });
        });
        row.with(
            El::column().size(CARD_HOST_WIDTH - CARD_WIDTH, CARD_HEIGHT),
            |_| {},
        );
    });
    (DiegeticPanel::world()
        .size(Pt(CARD_HOST_WIDTH), Pt(CARD_HEIGHT))
        .anchor(Anchor::Center)
        .world_width(CARD_HOST_WIDTH * CARD_WORLD_PER_POINT)
        .with_tree(tree.build())
        .build())
    .expect("card host panel")
}

fn card_scene(view: Vec2, mark: CardMark, tooltip: Tooltip) -> Scene {
    let mut scene = Scene::with_host(
        view,
        Transform::from_xyz(CARD_HOST_X, 0.0, 0.0),
        card_host(mark, tooltip),
    );
    scene.show();
    scene
}

#[test]
fn world_tooltip_shadow_stays_inside_the_margin_at_every_edge() {
    let cases = [
        (
            "bottom",
            Vec3::new(0.0, -1.85, 0.0),
            host_scaled_tooltip(45.0, 25.0),
        ),
        ("top", Vec3::new(0.0, 1.85, 0.0), above_tooltip(45.0)),
        (
            "right",
            Vec3::new(2.3, 0.0, 0.0),
            host_scaled_tooltip(40.0, 20.0)
                .source_anchor(Anchor::CenterLeft)
                .target_anchor(Anchor::CenterRight),
        ),
        (
            "top-left corner",
            Vec3::new(-2.3, 1.8, 0.0),
            host_scaled_tooltip(90.0, 20.0),
        ),
    ];
    for (edge, position, tooltip) in cases {
        let mut scene = Scene::new(position, tooltip);
        scene.show();
        scene.assert_painted_inside(scene.placement_area(), &format!("the {edge} margin"));
    }
}

#[test]
fn world_tooltip_shadow_stays_clear_of_a_docked_panel() {
    let mut scene = Scene::new(Vec3::new(-1.3, -0.85, 0.0), host_scaled_tooltip(70.0, 25.0));
    scene.screen_panel(Anchor::BottomLeft, Vec2::new(360.0, 140.0));
    scene.show();
    let painted = scene.assert_painted_inside(scene.placement_area(), "the placement area");
    assert!(
        painted.max.y <= VIEW.y - 140.0 - MARGIN + 1.0,
        "the shadow reaches into the docked help panel: {painted:?}"
    );
}

#[test]
fn tilted_world_tooltip_shadow_stays_inside_the_margin() {
    fn tilted_scene(x: f32) -> Scene {
        let tooltip = above_tooltip(60.0).offset(PanelAnchorOffset::new(0.0, -20.0));
        let mut scene = Scene::new(Vec3::new(x, -1.85, 0.0), tooltip);
        scene
            .app
            .world_mut()
            .get_mut::<Transform>(scene.host)
            .expect("host transform")
            .rotation = Quat::from_rotation_y(0.9) * Quat::from_rotation_x(-0.6);
        scene.app.update();
        scene.show();
        scene
    }
    let scene = tilted_scene(0.0);
    let area = scene.placement_area();
    let movement = Vec3::new(-2.9, 0.0, 0.0);
    let unplaced = scene.painted_bounds_after_move(movement);
    assert!(
        unplaced.min.x < area.min.x - 2.0,
        "the move must put the tilted shadow past the margin: {unplaced:?}, {area:?}"
    );
    let scene = tilted_scene(movement.x);
    scene.assert_painted_inside(area, "the margin of a tilted card after its edge slide");
}

#[test]
fn card_tooltip_stays_inside_its_marked_card_and_the_window() {
    for view in [Vec2::new(900.0, 800.0), Vec2::new(750.0, 800.0)] {
        let scene = card_scene(view, CardMark::TooltipBoundary, above_tooltip(100.0));
        let card = scene.host_rect(Vec2::ZERO, Vec2::new(CARD_WIDTH, CARD_HEIGHT));
        let card_area = Rect::from_corners(card.min + MARGIN, card.max - MARGIN);
        let painted = scene.assert_painted_inside(
            card_area.intersect(scene.placement_area()),
            &format!("the card inside a {view} window"),
        );
        let slider = scene.slider_centre();
        assert!(
            painted.min.x <= slider.x && painted.max.x >= slider.x,
            "the shift lost the slider at {view}: {painted:?} {slider:?}"
        );
        let body = scene.bounds(scene.tooltip);
        assert!(
            body.max.y <= slider.y,
            "the tooltip left its authored side above the slider at {view}: {body:?}"
        );
    }
}

#[test]
fn tooltip_wider_than_its_card_crosses_only_into_the_placement_area() {
    let mut scene = Scene::with_host(
        Vec2::new(900.0, 800.0),
        Transform::from_xyz(CARD_HOST_X, 0.0, 0.0),
        card_host(CardMark::TooltipBoundary, above_tooltip(300.0)),
    );
    scene.screen_panel(Anchor::CenterRight, Vec2::new(40.0, 800.0));
    scene.show();
    let card = scene.host_rect(Vec2::ZERO, Vec2::new(CARD_WIDTH, CARD_HEIGHT));
    let area = Rect::from_corners(
        Vec2::splat(MARGIN),
        Vec2::new(900.0 - 40.0 - MARGIN, 800.0 - MARGIN),
    );
    let painted = scene.assert_painted_inside(area, "the placement area beside the docked panel");
    assert!(
        painted.width() > card.width(),
        "fixture needs a tooltip wider than its card: {painted:?} {card:?}"
    );
    assert!(
        painted.min.x < card.min.x + MARGIN || painted.max.x > card.max.x - MARGIN,
        "a tooltip wider than its card must cross a card edge: {painted:?} {card:?}"
    );
}

#[test]
fn unmarked_card_keeps_placement_centred_over_the_slider() {
    let scene = card_scene(
        Vec2::new(900.0, 800.0),
        CardMark::Unmarked,
        above_tooltip(100.0),
    );
    let tip = scene.assert_visible_inside_view();
    let card = scene.host_rect(Vec2::ZERO, Vec2::new(CARD_WIDTH, CARD_HEIGHT));
    let slider = scene.slider_centre();
    assert!(
        (tip.center().x - slider.x).abs() <= 1.0,
        "with no marked ancestor the tooltip stays centred over its slider: {tip:?} {slider:?}"
    );
    assert!(
        tip.max.x > card.max.x,
        "fixture needs a tooltip wider than its offset room: {tip:?} {card:?}"
    );
}

impl Scene {
    /// A screen tooltip's body in window pixels, y down, from its overlay transform.
    fn screen_body(&self) -> Rect {
        let world = self.app.world();
        let panel = (world.get::<DiegeticPanel>(self.tooltip)).expect("tooltip panel");
        let translation = (world.get::<GlobalTransform>(self.tooltip))
            .expect("tooltip transform")
            .translation();
        let (offset_x, offset_y) = panel.anchor_offsets();
        let centre = 0.5 * self.view;
        let min = Vec2::new(
            translation.x + centre.x - offset_x,
            centre.y - translation.y - offset_y,
        );
        Rect::from_corners(min, min + Vec2::new(panel.width(), panel.height()))
    }

    /// A screen tooltip's painted bounds, body plus shadow, in window pixels, y down.
    fn screen_painted(&self) -> Rect {
        let body = self.screen_body();
        Rect {
            min: body.min - Vec2::splat(SHADOW_BLUR_SUPPORT_PX),
            max: body.max
                + Vec2::new(
                    SHADOW_BLUR_SUPPORT_PX,
                    SHADOW_BLUR_SUPPORT_PX + SHADOW_DOWNWARD_OFFSET_PX,
                ),
        }
    }
}

#[test]
fn screen_tooltip_painted_bounds_clear_its_source_for_overlapping_anchors() {
    for (source, target) in [
        (Anchor::TopRight, Anchor::BottomRight),
        (Anchor::Center, Anchor::Center),
    ] {
        let tooltip = Tooltip::new(El::column().size(Px(100.0), Px(30.0)))
            .source_anchor(source)
            .target_anchor(target)
            .offset(PanelAnchorOffset::new(0.0, 0.0))
            .show_after(Duration::ZERO);
        let mut tree = LayoutBuilder::with_root(
            El::column()
                .size(400.0, 240.0)
                .padding(Padding::new(140.0, 0.0, 100.0, 0.0)),
        );
        tree.with(
            El::new()
                .size(120.0, 40.0)
                .button("control")
                .tooltip(tooltip),
            |_| {},
        );
        let host = DiegeticPanel::screen()
            .size(Px(400.0), Px(240.0))
            .anchor(Anchor::Center)
            .with_tree(tree.build())
            .build()
            .expect("screen host");
        let mut scene = Scene::with_host(VIEW, Transform::IDENTITY, host);
        scene.show();
        assert_eq!(
            scene.shown_count(),
            1,
            "ample room for help below the source"
        );
        let painted = scene.screen_painted();
        let control = Rect::from_center_size(VIEW * 0.5, Vec2::new(120.0, 40.0));
        assert!(
            painted.intersect(control).is_empty(),
            "screen help including shadow must clear its source: {source:?}->{target:?}, {painted:?}, {control:?}"
        );
    }
}

#[test]
fn screen_tooltip_shadow_stays_inside_the_margin_in_a_corner() {
    let tooltip = Tooltip::new(El::column().size(Px(160.0), Px(30.0))).show_after(Duration::ZERO);
    let host = (DiegeticPanel::screen()
        .size(Px(120.0), Px(40.0))
        .anchor(Anchor::BottomRight)
        .with_tree(rebuild_tree(tooltip))
        .build())
    .expect("screen host");
    let mut scene = Scene::with_host(VIEW, Transform::IDENTITY, host);
    scene.show();
    assert_eq!(scene.shown_count(), 1, "tooltip must reveal exactly once");
    let painted = scene.screen_painted();
    let area = scene.placement_area();
    assert!(
        painted.min.cmpge(area.min - 1.0).all() && painted.max.cmple(area.max + 1.0).all(),
        "painted screen tooltip leaves the placement area: painted {painted:?}, area {area:?}"
    );
}

fn rebuild_tree(tooltip: Tooltip) -> LayoutTree {
    let mut tree = LayoutBuilder::new(120.0, 40.0);
    tree.with(
        El::new()
            .size(120.0, 40.0)
            .button("control")
            .tooltip(tooltip),
        |_| {},
    );
    tree.build()
}

#[test]
fn rebuilding_screen_tooltip_keeps_its_control_anchor() {
    let wide = Tooltip::new(El::column().size(Px(160.0), Px(30.0))).show_after(Duration::ZERO);
    let narrow = Tooltip::new(El::column().size(Px(140.0), Px(30.0))).show_after(Duration::ZERO);
    let host = (DiegeticPanel::screen()
        .size(Px(120.0), Px(40.0))
        .anchor(Anchor::BottomRight)
        .with_tree(rebuild_tree(wide.clone()))
        .build())
    .expect("screen host");
    let mut scene = Scene::with_host(VIEW, Transform::IDENTITY, host);
    scene.show();
    assert_eq!(scene.shown_count(), 1, "initial tooltip must reveal");

    for (rebuild, next) in [narrow.clone(), wide, narrow].into_iter().enumerate() {
        let old_controller = scene.tooltip;
        scene
            .app
            .world_mut()
            .commands()
            .set_tree(scene.host, rebuild_tree(next))
            .expect("rebuild screen host");
        scene.show();

        let controller = scene
            .app
            .world()
            .get::<hana_diegetic::Tooltips>(scene.control)
            .and_then(|tooltips| tooltips.iter().next())
            .expect("rebuilt tooltip controller");
        assert_ne!(
            controller, old_controller,
            "rebuild {rebuild} reused the old controller"
        );
        scene.tooltip = controller;
        assert_eq!(
            scene.shown_count(),
            rebuild + 2,
            "rebuild {rebuild} did not reveal the new tooltip"
        );
        assert_eq!(
            scene.app.world().resource::<Shows>().0.last(),
            Some(&controller),
            "rebuild {rebuild} revealed a different controller"
        );

        let body = scene.screen_body();
        let control = Rect::from_corners(VIEW - Vec2::new(120.0, 40.0), VIEW);
        assert!(
            body.min.x <= control.center().x && body.max.x >= control.center().x,
            "rebuild {rebuild} lost the control horizontally: {body:?} {control:?}"
        );
        assert!(
            body.max.y >= control.min.y - 20.0 && body.max.y <= control.min.y + 1.0,
            "rebuild {rebuild} left the tooltip away from the control: {body:?} {control:?}"
        );
    }
}

#[test]
fn screen_tooltip_slides_inside_its_marked_ancestor_and_clear_of_a_docked_panel() {
    // A host centred in the window: its left half is a marked card, with a button near the
    // card's right edge whose tooltip, centred on the button, would cross that edge.
    let host_size = Vec2::new(400.0, 240.0);
    let card_size = Vec2::new(200.0, 240.0);
    let button_top_left = Vec2::new(150.0, 10.0);
    let button_size = Vec2::new(40.0, 20.0);
    let tooltip = Tooltip::new(El::column().size(Px(120.0), Px(60.0)))
        .show_after(Duration::ZERO)
        .source_anchor(Anchor::BottomCenter)
        .target_anchor(Anchor::TopCenter);
    let mut tree = LayoutBuilder::new(host_size.x, host_size.y);
    tree.with(El::row().size(host_size.x, host_size.y), |row| {
        row.with(
            El::column()
                .size(card_size.x, card_size.y)
                .tooltip_boundary(),
            |card| {
                card.with(El::new().size(card_size.x, button_top_left.y), |_| {});
                card.with(El::row().size(card_size.x, button_size.y), |button_row| {
                    button_row.with(El::new().size(button_top_left.x, button_size.y), |_| {});
                    button_row.with(
                        El::new()
                            .size(button_size.x, button_size.y)
                            .button("control")
                            .tooltip(tooltip),
                        |_| {},
                    );
                });
            },
        );
        row.with(
            El::column().size(host_size.x - card_size.x, host_size.y),
            |_| {},
        );
    });
    let host = (DiegeticPanel::screen()
        .size(Px(host_size.x), Px(host_size.y))
        .anchor(Anchor::Center)
        .with_tree(tree.build())
        .build())
    .expect("screen host");
    let mut scene = Scene::with_host(VIEW, Transform::IDENTITY, host);
    // A band docked across the top leaves no room above the button, so the tooltip goes below.
    let band_height = 140.0;
    scene.screen_panel(Anchor::TopCenter, Vec2::new(VIEW.x, band_height));
    scene.show();
    assert_eq!(scene.shown_count(), 1, "tooltip must reveal exactly once");

    let body = scene.screen_body();
    let painted = scene.screen_painted();
    let host_top_left = (VIEW - host_size) / 2.0;
    let card = Rect::from_corners(host_top_left, host_top_left + card_size);
    let button = Rect::from_corners(
        host_top_left + button_top_left,
        host_top_left + button_top_left + button_size,
    );
    let area = Rect::from_corners(Vec2::new(MARGIN, band_height + MARGIN), VIEW - MARGIN);
    let card_area = Rect::from_corners(card.min + MARGIN, card.max - MARGIN).intersect(area);
    assert!(
        painted.min.cmpge(card_area.min - 1.0).all()
            && painted.max.cmple(card_area.max + 1.0).all(),
        "painted screen tooltip leaves its marked card or meets the docked band: painted \
         {painted:?}, card {card:?}, area {area:?}"
    );
    assert!(
        body.min.y >= button.max.y,
        "the docked band moves the tooltip below its button: {body:?} {button:?}"
    );
    assert!(
        body.min.x <= button.center().x && body.max.x >= button.center().x,
        "the slide keeps the button within the tooltip span: {body:?} {button:?}"
    );
}

/// The bordered screen host's border width.
const HOST_BORDER: f32 = 2.0;

/// The side of its control a tooltip is placed on.
#[derive(Clone, Copy, Debug)]
enum Placement {
    Left,
    Bottom,
}

impl Placement {
    /// The tooltip's anchor, and the control's anchor it attaches to.
    const fn anchors(self) -> (Anchor, Anchor) {
        match self {
            Self::Left => (Anchor::CenterRight, Anchor::CenterLeft),
            Self::Bottom => (Anchor::TopCenter, Anchor::BottomCenter),
        }
    }
}

/// A bordered screen host centred in the view. A marked card narrower than the tooltip fills
/// its top-left, with the control near the card's bottom-left corner, so the tooltip placed
/// Left or Bottom of it would cross the host's border. Returns the scene and the host's outer
/// edge.
fn bordered_host_scene(placement: Placement) -> (Scene, Rect) {
    let host_size = Vec2::new(240.0, 150.0);
    let (source, target) = placement.anchors();
    let tooltip = Tooltip::new(El::column().size(Px(140.0), Px(28.0)))
        .show_after(Duration::ZERO)
        .source_anchor(source)
        .target_anchor(target);
    let mut tree = LayoutBuilder::with_root(
        El::row()
            .size(host_size.x, host_size.y)
            .padding(Padding::all(10.0))
            .border(Border::all(HOST_BORDER, Color::WHITE)),
    );
    tree.with(El::column().size(100.0, 126.0).tooltip_boundary(), |card| {
        card.with(El::new().size(100.0, 100.0), |_| {});
        card.with(El::row().size(100.0, 20.0), |button_row| {
            button_row.with(El::new().size(6.0, 20.0), |_| {});
            button_row.with(
                El::new()
                    .size(30.0, 20.0)
                    .button("control")
                    .tooltip(tooltip),
                |_| {},
            );
        });
    });
    let host = (DiegeticPanel::screen()
        .size(Px(host_size.x), Px(host_size.y))
        .anchor(Anchor::Center)
        .with_tree(tree.build())
        .build())
    .expect("bordered screen host");
    let host_top_left = (VIEW - host_size) / 2.0;
    (
        Scene::with_host(VIEW, Transform::IDENTITY, host),
        Rect::from_corners(host_top_left, host_top_left + host_size),
    )
}

/// Placed on `placement`'s side, the tooltip's painted bounds lie inside its host's inner edge
/// inset by the margin, or outside its outer edge outset by the margin: never across the
/// border.
fn assert_painted_clear_of_host_border(placement: Placement) {
    let (mut scene, outer) = bordered_host_scene(placement);
    scene.show();
    assert_eq!(scene.shown_count(), 1, "tooltip must reveal exactly once");
    let painted = scene.screen_painted();
    let inside = outer.inflate(1.0 - HOST_BORDER - MARGIN);
    let beside = outer.inflate(MARGIN - 1.0);
    assert!(
        (inside.contains(painted.min) && inside.contains(painted.max))
            || painted.intersect(beside).is_empty(),
        "{placement:?} tooltip painted {painted:?} crosses its host's {HOST_BORDER} px border or \
         its {MARGIN} px margin: host {outer:?}"
    );
}

#[test]
fn screen_tooltip_placed_left_stays_clear_of_its_host_border() {
    assert_painted_clear_of_host_border(Placement::Left);
}

#[test]
fn screen_tooltip_placed_below_stays_clear_of_its_host_border() {
    assert_painted_clear_of_host_border(Placement::Bottom);
}

/// The outer edge of its host a tooltip leaves through.
#[derive(Clone, Copy, Debug)]
enum HostEdge {
    Top,
    Bottom,
}

/// A bordered screen host 120 px wide, centred in the view, holds a control `control_top`
/// pixels below its border and centred across it. Its 140 px tooltip, placed Bottom, crosses
/// the host's border on every side beside the control, so it moves out past the host through
/// `edge`, the outer edge nearest the control: its painted bounds lie exactly the margin past
/// that edge, centred on the control.
fn assert_too_wide_tooltip_leaves_its_host_through(control_top: f32, edge: HostEdge) {
    let host_size = Vec2::new(120.0, 150.0);
    let control_size = Vec2::new(30.0, 20.0);
    let tooltip = Tooltip::new(El::column().size(Px(140.0), Px(28.0))).show_after(Duration::ZERO);
    let mut tree = LayoutBuilder::with_root(
        El::column()
            .size(host_size.x, host_size.y)
            .padding(Padding::new(
                0.5f32.mul_add(host_size.x - control_size.x, -HOST_BORDER),
                0.0,
                control_top,
                0.0,
            ))
            .border(Border::all(HOST_BORDER, Color::WHITE)),
    );
    tree.with(
        El::new()
            .size(control_size.x, control_size.y)
            .button("control")
            .tooltip(tooltip),
        |_| {},
    );
    let host = (DiegeticPanel::screen()
        .size(Px(host_size.x), Px(host_size.y))
        .anchor(Anchor::Center)
        .with_tree(tree.build())
        .build())
    .expect("narrow screen host");
    let mut scene = Scene::with_host(VIEW, Transform::IDENTITY, host);
    scene.show();
    assert_eq!(scene.shown_count(), 1, "tooltip must reveal exactly once");

    let outer = Rect::from_center_size(0.5 * VIEW, host_size);
    let painted = scene.screen_painted();
    let past_edge = match edge {
        HostEdge::Top => outer.min.y - MARGIN - painted.max.y,
        HostEdge::Bottom => painted.min.y - outer.max.y - MARGIN,
    };
    assert!(
        past_edge.abs() <= 1.0,
        "tooltip painted {painted:?} should lie {MARGIN} px past its host's {edge:?} edge, \
         and no further: {past_edge} px off, host {outer:?}"
    );
    assert!(
        (painted.center().x - outer.center().x).abs() <= 1.0,
        "tooltip painted {painted:?} should stay centred on its control: host {outer:?}"
    );
}

#[test]
fn screen_tooltip_too_wide_for_its_host_moves_out_past_its_host_border() {
    assert_too_wide_tooltip_leaves_its_host_through(118.0, HostEdge::Bottom);
}

#[test]
fn screen_tooltip_too_wide_for_its_host_leaves_by_the_edge_nearest_its_control() {
    assert_too_wide_tooltip_leaves_its_host_through(12.0, HostEdge::Top);
}
