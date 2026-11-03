use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::ecs::system::RunSystemOnce;
use bevy::input::mouse::MouseScrollUnit;
use bevy::input::touch::TouchPhase;
use bevy::picking::events::PointerPress;
use bevy::picking::hover::PickingInteraction;
use bevy::picking::pointer::PointerAction;
use bevy::picking::pointer::PointerButton;
use bevy::picking::pointer::PointerId;
use bevy::picking::pointer::PointerInput;
use bevy::picking::pointer::PointerLocation;
use bevy::prelude::Camera;
use bevy::prelude::ChildOf;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::On;
use bevy::prelude::PointerScroll;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::UVec2;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::WindowRef;

use super::ButtonLifecycle;
use super::PickingScene;
use super::build_picking_scene;
use super::dispatch_button_click;
use super::hover_state;
use super::pointer_location;
use super::record_center_world;
use super::resolve_hover;
use super::resolve_widget;
use super::widget_record;
use crate::DiegeticPanel;
use crate::El;
use crate::HoveredScrollTargets;
use crate::LayoutBuilder;
use crate::Mm;
use crate::PanelScrolled;
use crate::PointerScrollTarget;
use crate::Slider;
use crate::SliderState;
use crate::WidgetScreenSizeGate;
use crate::widgets::WidgetCaptures;
use crate::widgets::WidgetHitArea;

const INERT_BELOW: f32 = 12.0;
const ABOVE: f32 = 13.0;
const JUST_ABOVE: f32 = 12.5;
const UNDER: f32 = 11.0;
const PHYSICAL_VIEWPORT: UVec2 = UVec2::splat(1000);
const SCALE_FACTOR: f32 = 2.0;
const LOGICAL_HEIGHT: f32 = 500.0;

#[derive(Default, Resource)]
struct OwnerPresses(Vec<Entity>);

fn size_scene() -> PickingScene {
    let mut scene = build_picking_scene();
    scene
        .app
        .insert_resource(WidgetScreenSizeGate::new(INERT_BELOW));
    scene.app.init_resource::<OwnerPresses>();
    let owner = scene.app.world_mut().spawn(Transform::default()).id();
    scene.app.world_mut().entity_mut(owner).observe(
        |press: On<PointerPress>, mut presses: ResMut<OwnerPresses>| {
            presses.0.push(press.original_event_target());
        },
    );
    scene
        .app
        .world_mut()
        .entity_mut(scene.panel)
        .insert(ChildOf(owner));
    scene
}

fn set_screen_size(scene: &mut PickingScene, size: f32) {
    let panel = scene
        .app
        .world()
        .get::<DiegeticPanel>(scene.panel)
        .expect("panel");
    let rect = widget_record(&scene.app, scene.panel, "target").rect();
    let world_side = rect.width.min(rect.height) * panel.points_to_world();
    let half_height = world_side * LOGICAL_HEIGHT / size * 0.5;
    let mut camera = scene.app.world_mut().entity_mut(scene.camera);
    let mut data = camera.get_mut::<Camera>().expect("camera");
    data.computed.target_info = Some(RenderTargetInfo {
        physical_size: PHYSICAL_VIEWPORT,
        scale_factor:  SCALE_FACTOR,
    });
    data.computed.clip_from_view = bevy::math::proj::orthographic(
        -half_height,
        half_height,
        -half_height,
        half_height,
        0.1,
        10.0,
    );
    camera.insert(Transform::from_xyz(0.0, 0.0, 1.0));
    scene.app.update();
}

fn press(scene: &mut PickingScene) {
    let origin = scene.local_hit + Vec3::Z;
    resolve_hover(scene, origin);
    let location = pointer_location(&mut scene.app, scene.pointer);
    scene.app.world_mut().write_message(PointerInput::new(
        scene.pointer,
        location,
        PointerAction::Press(PointerButton::Primary),
    ));
    scene.app.update();
}

#[test]
fn size_gate_under_threshold_press_reaches_owner_without_capture() {
    let mut scene = size_scene();
    set_screen_size(&mut scene, UNDER);
    press(&mut scene);
    assert_eq!(scene.app.world().resource::<ButtonLifecycle>().pressed, 0);
    assert_eq!(
        scene.app.world().resource::<OwnerPresses>().0,
        [scene.panel]
    );
    assert!(
        scene
            .app
            .world()
            .resource::<WidgetCaptures>()
            .widget(scene.pointer)
            .is_none()
    );
}

#[test]
fn size_gate_live_threshold_button_takes_press_and_capture() {
    let mut scene = size_scene();
    set_screen_size(&mut scene, UNDER);
    let origin = scene.local_hit + Vec3::Z;
    resolve_hover(&mut scene, origin);
    set_screen_size(&mut scene, ABOVE);
    press(&mut scene);
    assert_eq!(scene.app.world().resource::<ButtonLifecycle>().pressed, 1);
    assert_eq!(
        scene.app.world().resource::<OwnerPresses>().0,
        Vec::<Entity>::new()
    );
    assert_eq!(
        scene
            .app
            .world()
            .resource::<WidgetCaptures>()
            .widget(scene.pointer),
        Some(scene.widget)
    );
}

#[test]
fn size_gate_hover_uses_current_size_after_growth_and_shrink() {
    let mut scene = size_scene();
    for (size, expected) in [
        (JUST_ABOVE, Some(PickingInteraction::Hovered)),
        (INERT_BELOW, Some(PickingInteraction::Hovered)),
        (UNDER, Some(PickingInteraction::None)),
        (JUST_ABOVE, Some(PickingInteraction::Hovered)),
        (ABOVE, Some(PickingInteraction::Hovered)),
        (JUST_ABOVE, Some(PickingInteraction::Hovered)),
    ] {
        set_screen_size(&mut scene, size);
        let origin = scene.local_hit + Vec3::Z;
        resolve_hover(&mut scene, origin);
        assert_eq!(hover_state(&scene, scene.widget), expected, "size {size}");
    }
}

#[test]
fn size_gate_hover_yields_to_panel_then_returns_to_widget() {
    let mut scene = size_scene();
    set_screen_size(&mut scene, UNDER);
    let origin = scene.local_hit + Vec3::Z;
    resolve_hover(&mut scene, origin);
    assert_ne!(
        hover_state(&scene, scene.widget),
        Some(PickingInteraction::Hovered)
    );
    assert_eq!(
        hover_state(&scene, scene.panel),
        Some(PickingInteraction::Hovered)
    );
    set_screen_size(&mut scene, ABOVE);
    resolve_hover(&mut scene, origin);
    assert_eq!(
        hover_state(&scene, scene.widget),
        Some(PickingInteraction::Hovered)
    );
}

#[test]
fn size_gate_slider_press_also_yields_to_owner() {
    let mut scene = size_scene();
    let mut layout = LayoutBuilder::new(100.0, 50.0);
    layout.with(
        El::new()
            .size(20.0, 10.0)
            .widget("target", Slider::new(0.0..=1.0)),
        |_| {},
    );
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .with_tree(layout.build())
        .build()
        .expect("slider panel");
    scene.app.world_mut().entity_mut(scene.panel).insert(panel);
    scene.app.update();
    scene.app.update();
    scene.widget = resolve_widget(&mut scene.app, scene.panel, "target");
    scene.local_hit = record_center_world(&scene.app, scene.panel, "target");
    set_screen_size(&mut scene, UNDER);
    press(&mut scene);
    assert_eq!(
        scene.app.world().resource::<OwnerPresses>().0,
        [scene.panel]
    );
    assert!(
        scene
            .app
            .world()
            .resource::<WidgetCaptures>()
            .widget(scene.pointer)
            .is_none()
    );
}

fn slider_size_scene() -> PickingScene {
    let mut scene = size_scene();
    scene.app.add_observer(crate::slider_self_update);
    // Slider dragging reprojects through the camera's actual pointer target.
    let world = scene.app.world_mut();
    let window = world
        .query_filtered::<Entity, With<Window>>()
        .single(world)
        .expect("pointer window");
    world
        .entity_mut(scene.camera)
        .insert(RenderTarget::Window(WindowRef::Entity(window)));
    let mut layout = LayoutBuilder::new(100.0, 50.0);
    layout.with(
        El::new()
            .size(20.0, 10.0)
            .widget("target", Slider::new(0.0..=1.0).value(0.5)),
        |_| {},
    );
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .with_tree(layout.build())
        .build()
        .expect("slider panel");
    scene.app.world_mut().entity_mut(scene.panel).insert(panel);
    scene.app.update();
    scene.app.update();
    scene.widget = resolve_widget(&mut scene.app, scene.panel, "target");
    scene.local_hit = record_center_world(&scene.app, scene.panel, "target");
    scene
}

fn assert_slider_points(scene: &PickingScene, expected: f32) {
    let rect = widget_record(&scene.app, scene.panel, "target").rect();
    let lens = scene
        .app
        .world()
        .get::<Camera>(scene.camera)
        .expect("camera");
    let pose = scene
        .app
        .world()
        .get::<GlobalTransform>(scene.camera)
        .expect("camera pose");
    let points = [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x, rect.y + rect.height),
    ]
    .map(|point| {
        lens.world_to_viewport(
            pose,
            super::panel_point_to_world(&scene.app, scene.panel, point),
        )
        .expect("widget projects")
    });
    let measured = points[0]
        .distance(points[1])
        .min(points[0].distance(points[2]));
    assert!(
        (measured - expected).abs() < 0.01,
        "slider measured {measured} points, expected {expected}"
    );
}

fn drag_size_slider(scene: &mut PickingScene) {
    assert_eq!(
        hover_state(scene, scene.widget),
        Some(PickingInteraction::Hovered),
        "the measured slider must receive the real pointer hit"
    );
    let before = scene
        .app
        .world()
        .get::<SliderState>(scene.widget)
        .expect("slider")
        .value();
    let mut location = pointer_location(&mut scene.app, scene.pointer);
    location.position = scene
        .app
        .world()
        .get::<Camera>(scene.camera)
        .expect("camera")
        .world_to_viewport(
            scene
                .app
                .world()
                .get::<GlobalTransform>(scene.camera)
                .expect("camera pose"),
            scene.local_hit,
        )
        .expect("slider projects");
    let pointer = scene.pointer;
    let world = scene.app.world_mut();
    for (id, mut position) in world
        .query::<(&PointerId, &mut PointerLocation)>()
        .iter_mut(world)
    {
        if *id == pointer {
            *position = PointerLocation::new(location.clone());
        }
    }
    scene.app.world_mut().write_message(PointerInput::new(
        pointer,
        location.clone(),
        PointerAction::Press(PointerButton::Primary),
    ));
    scene.app.update();
    let delta = Vec2::new(8.0, 0.0);
    location.position += delta;
    scene.app.world_mut().write_message(PointerInput::new(
        pointer,
        location.clone(),
        PointerAction::Move { delta },
    ));
    scene.app.update();
    let after = scene
        .app
        .world()
        .get::<SliderState>(scene.widget)
        .expect("dragged slider")
        .value();
    assert!(
        (after - before).abs() > 0.01,
        "real picked slider drag must change its value: {before} -> {after}"
    );
    scene.app.world_mut().write_message(PointerInput::new(
        pointer,
        location,
        PointerAction::Release(PointerButton::Primary),
    ));
    scene.app.update();
}

#[test]
fn size_gate_slider_drags_after_continuous_hover_grows_from_eleven_to_thirteen_points() {
    let mut scene = slider_size_scene();
    set_screen_size(&mut scene, UNDER);
    assert_slider_points(&scene, UNDER);
    let origin = scene.local_hit + Vec3::Z;
    resolve_hover(&mut scene, origin);
    assert_ne!(
        hover_state(&scene, scene.widget),
        Some(PickingInteraction::Hovered)
    );
    // The ray stays on the slider through every resize/update: no hover gap.
    set_screen_size(&mut scene, ABOVE);
    assert_slider_points(&scene, ABOVE);
    resolve_hover(&mut scene, origin);
    drag_size_slider(&mut scene);
}

#[test]
fn size_gate_slider_at_twelve_and_a_half_points_drags_on_fresh_hover() {
    let mut scene = slider_size_scene();
    set_screen_size(&mut scene, JUST_ABOVE);
    assert_slider_points(&scene, JUST_ABOVE);
    let origin = scene.local_hit + Vec3::Z;
    resolve_hover(&mut scene, origin);
    drag_size_slider(&mut scene);
}

#[test]
fn size_gate_slider_at_eleven_points_yields_drag_to_owner() {
    let mut scene = slider_size_scene();
    set_screen_size(&mut scene, UNDER);
    assert_slider_points(&scene, UNDER);
    let before = scene
        .app
        .world()
        .get::<SliderState>(scene.widget)
        .expect("slider")
        .value();
    press(&mut scene);
    let mut location = pointer_location(&mut scene.app, scene.pointer);
    let delta = Vec2::new(8.0, 0.0);
    location.position += delta;
    scene.app.world_mut().write_message(PointerInput::new(
        scene.pointer,
        location,
        PointerAction::Move { delta },
    ));
    scene.app.update();
    assert_eq!(
        scene
            .app
            .world()
            .get::<SliderState>(scene.widget)
            .expect("slider")
            .value(),
        before
    );
    assert_eq!(
        scene.app.world().resource::<OwnerPresses>().0,
        [scene.panel]
    );
    assert!(
        scene
            .app
            .world()
            .resource::<WidgetCaptures>()
            .widget(scene.pointer)
            .is_none()
    );
}

#[test]
fn size_gate_default_leaves_tiny_widgets_live() {
    let mut scene = size_scene();
    scene.app.insert_resource(WidgetScreenSizeGate::default());
    set_screen_size(&mut scene, 1.0);
    dispatch_button_click(&mut scene);
    assert_eq!(scene.app.world().resource::<ButtonLifecycle>().clicked, 1);
}

#[test]
fn size_gate_uses_projected_edges_on_a_rotated_panel() {
    let mut scene = size_scene();
    set_screen_size(&mut scene, UNDER);
    scene
        .app
        .world_mut()
        .entity_mut(scene.panel)
        .insert(Transform::from_rotation(
            bevy::prelude::Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
        ));
    scene.app.update();
    scene.local_hit = record_center_world(&scene.app, scene.panel, "target");
    press(&mut scene);
    assert_eq!(scene.app.world().resource::<ButtonLifecycle>().pressed, 0);
    assert_eq!(
        scene.app.world().resource::<OwnerPresses>().0,
        [scene.panel]
    );
}

#[test]
fn size_gate_measures_clipped_hit_area_not_authored_rectangle() {
    let mut scene = size_scene();
    let mut layout = LayoutBuilder::new(100.0, 50.0);
    layout.with(El::row().size(4.0, 10.0).clip(), |layout| {
        layout.with(El::new().size(20.0, 10.0).button("target"), |_| {});
    });
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .with_tree(layout.build())
        .build()
        .expect("clipped panel");
    scene.app.world_mut().entity_mut(scene.panel).insert(panel);
    scene.app.update();
    scene.app.update();
    scene.widget = resolve_widget(&mut scene.app, scene.panel, "target");
    // Calibrate from the authored rectangle: its short side is live-sized,
    // while the narrower visible hit area must be inert.
    set_screen_size(&mut scene, ABOVE);
    let rect = match widget_record(&scene.app, scene.panel, "target").hit_area() {
        WidgetHitArea::Visible(rect) => Some(rect),
        WidgetHitArea::FullyClipped => None,
    }
    .expect("target must be partially visible");
    scene.local_hit = super::panel_point_to_world(&scene.app, scene.panel, rect.center());
    press(&mut scene);
    assert_eq!(scene.app.world().resource::<ButtonLifecycle>().pressed, 0);
    assert_eq!(
        scene.app.world().resource::<OwnerPresses>().0,
        [scene.panel]
    );
}

#[test]
fn size_gate_diagonal_out_of_plane_tilt_yields_to_owner() {
    let mut scene = size_scene();
    let mut layout = LayoutBuilder::new(100.0, 50.0);
    layout.with(El::new().size(20.0, 20.0).button("target"), |_| {});
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .with_tree(layout.build())
        .build()
        .expect("square button panel");
    scene.app.world_mut().entity_mut(scene.panel).insert(panel);
    scene.app.update();
    scene.app.update();
    scene.widget = resolve_widget(&mut scene.app, scene.panel, "target");
    set_screen_size(&mut scene, 100.0);
    // Each projected edge remains about 70.7 pt, but the width is under 2.5 pt.
    scene
        .app
        .world_mut()
        .entity_mut(scene.panel)
        .insert(Transform::from_rotation(
            bevy::prelude::Quat::from_axis_angle(
                Vec3::new(1.0, 1.0, 0.0).normalize(),
                89.0_f32.to_radians(),
            ),
        ));
    scene.app.update();
    let panel_normal = scene
        .app
        .world()
        .get::<GlobalTransform>(scene.panel)
        .expect("tilted panel transform")
        .rotation()
        * Vec3::Z;
    assert!(panel_normal.z.abs() < 0.02, "panel is nearly edge-on");
    scene.local_hit = record_center_world(&scene.app, scene.panel, "target");
    press(&mut scene);
    assert_eq!(scene.app.world().resource::<ButtonLifecycle>().pressed, 0);
    assert_eq!(
        scene.app.world().resource::<OwnerPresses>().0,
        [scene.panel]
    );
}

#[test]
fn size_gate_each_camera_uses_its_current_projected_size() {
    let mut scene = size_scene();
    let first_camera = scene.camera;
    set_screen_size(&mut scene, UNDER);
    let origin = scene.local_hit + Vec3::Z;
    resolve_hover(&mut scene, origin);
    assert_ne!(
        hover_state(&scene, scene.widget),
        Some(PickingInteraction::Hovered)
    );
    scene.camera = scene.app.world_mut().spawn(Camera::default()).id();
    let second_camera = scene.camera;
    set_screen_size(&mut scene, ABOVE);
    resolve_hover(&mut scene, origin);
    assert_eq!(
        hover_state(&scene, scene.widget),
        Some(PickingInteraction::Hovered)
    );
    scene.camera = first_camera;
    set_screen_size(&mut scene, JUST_ABOVE);
    resolve_hover(&mut scene, origin);
    assert_eq!(
        hover_state(&scene, scene.widget),
        Some(PickingInteraction::Hovered)
    );
    scene.camera = second_camera;
    set_screen_size(&mut scene, JUST_ABOVE);
    resolve_hover(&mut scene, origin);
    assert_eq!(
        hover_state(&scene, scene.widget),
        Some(PickingInteraction::Hovered)
    );
}

#[derive(Default, Resource)]
struct OwnerWheels {
    forwarded: Vec<Entity>,
    scrolled:  usize,
}

fn scrolling_scene(library_owned: bool) -> PickingScene {
    let mut scene = size_scene();
    scene.app.init_resource::<OwnerWheels>();
    scene
        .app
        .add_observer(|_: On<PanelScrolled>, mut wheels: ResMut<OwnerWheels>| {
            wheels.scrolled += 1;
        });
    let owner = scene
        .app
        .world()
        .get::<ChildOf>(scene.panel)
        .expect("owner")
        .parent();
    scene.app.world_mut().entity_mut(owner).observe(
        |scroll: On<PointerScroll>, mut wheels: ResMut<OwnerWheels>| {
            wheels.forwarded.push(scroll.original_event_target());
        },
    );
    let mut layout = LayoutBuilder::new(100.0, 50.0);
    let body = El::column().size(20.0, 10.0);
    let body = if library_owned {
        body.scroll_view("body")
    } else {
        body.id("body").scroll_y(0.0)
    };
    layout.with(body, |body| {
        body.with(El::new().size(20.0, 10.0).button("target"), |_| {});
        body.with(El::new().size(20.0, 30.0), |_| {});
    });
    let panel = DiegeticPanel::world()
        .size(Mm(100.0), Mm(50.0))
        .with_tree(layout.build())
        .build()
        .expect("scrolling panel");
    scene.app.world_mut().entity_mut(scene.panel).insert(panel);
    scene.app.update();
    scene.app.update();
    scene.widget = resolve_widget(&mut scene.app, scene.panel, "target");
    scene.local_hit = record_center_world(&scene.app, scene.panel, "target");
    scene
}

fn assert_scroll_size_gate(library_owned: bool) {
    let mut scene = scrolling_scene(library_owned);
    for (size, live) in [
        (UNDER, false),
        (JUST_ABOVE, true),
        (ABOVE, true),
        (JUST_ABOVE, true),
        (UNDER, false),
    ] {
        set_screen_size(&mut scene, size);
        assert_scroll_target(&mut scene, live, size);
        let origin = scene.local_hit + Vec3::Z;
        resolve_hover(&mut scene, origin);
        assert_scroll_target(&mut scene, live, size);
        *scene.app.world_mut().resource_mut::<OwnerWheels>() = OwnerWheels::default();
        let location = pointer_location(&mut scene.app, scene.pointer);
        scene.app.world_mut().write_message(PointerInput::new(
            scene.pointer,
            location,
            PointerAction::Scroll {
                unit:  MouseScrollUnit::Line,
                x:     0.0,
                y:     1.0,
                phase: TouchPhase::Moved,
            },
        ));
        scene.app.update();
        let wheels = scene.app.world().resource::<OwnerWheels>();
        assert_eq!(
            wheels.scrolled,
            usize::from(live && !library_owned),
            "size {size}"
        );
        assert_eq!(wheels.forwarded.len(), usize::from(!live), "size {size}");
        assert_scroll_target(&mut scene, live, size);
    }
    // An unevaluated frame does not change the next size-based verdict.
    scene.app.update();
    set_screen_size(&mut scene, JUST_ABOVE);
    assert_scroll_target(&mut scene, true, JUST_ABOVE);
}

fn assert_scroll_target(scene: &mut PickingScene, live: bool, size: f32) {
    let pointer = scene.pointer;
    let target = scene
        .app
        .world_mut()
        .run_system_once(move |mut targets: HoveredScrollTargets| targets.under(pointer))
        .expect("scroll classification");
    assert_eq!(
        matches!(target, PointerScrollTarget::Scrollable { .. }),
        live,
        "size {size}"
    );
}

#[test]
fn size_gate_tiny_scroll_area_yields_wheel_to_owner() { assert_scroll_size_gate(false); }

#[test]
fn size_gate_tiny_library_scroll_view_yields_wheel_to_owner() { assert_scroll_size_gate(true); }

#[test]
fn size_gate_missing_settings_leaves_scroll_targets_usable() {
    let mut scene = scrolling_scene(false);
    scene
        .app
        .world_mut()
        .remove_resource::<WidgetScreenSizeGate>();
    let origin = scene.local_hit + Vec3::Z;
    resolve_hover(&mut scene, origin);
    let pointer = scene.pointer;
    let target = scene
        .app
        .world_mut()
        .run_system_once(move |mut targets: HoveredScrollTargets| targets.under(pointer))
        .expect("scroll classification without gate settings");
    assert!(matches!(target, PointerScrollTarget::Scrollable { .. }));
}
