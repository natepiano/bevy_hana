//! A pointer press on a widget belongs to the widget until that press ends.

use bevy::asset::AssetApp;
use bevy::asset::AssetPlugin;
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::PerspectiveProjection;
use bevy::camera::Projection;
use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::ecs::system::RunSystemOnce;
use bevy::gizmos::GizmoAsset;
use bevy::input::InputSystems;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::input::mouse::MouseScrollPixelsPerLine;
use bevy::input::touch::TouchInput;
use bevy::input::touch::TouchPhase;
use bevy::input::touch::Touches;
use bevy::input::touch::touch_screen_input_system;
use bevy::picking::InteractionPlugin;
use bevy::picking::PickingPlugin;
use bevy::picking::backend::HitData;
use bevy::picking::backend::PointerHits;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerAction;
use bevy::picking::pointer::PointerButton;
use bevy::picking::pointer::PointerId;
use bevy::picking::pointer::PointerInput;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::Shader;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use fairy_dust::WidgetDragClaimPlugin;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::ControlMetrics;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::El;
use hana_diegetic::ImeBuiltInFieldKind;
use hana_diegetic::ImeBuiltInFieldSpec;
use hana_diegetic::ImeEditableFieldSpec;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Mm;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelWidgetReader;
use hana_diegetic::PanelWidgetWriter;
use hana_diegetic::Sizing;
use hana_diegetic::Slider;
use hana_diegetic::SliderState;
use hana_diegetic::TextStyle;
use hana_diegetic::WidgetDisabled;
use hana_diegetic::WidgetInteractivity;
use hana_diegetic::slider_self_update;
use hana_lagrange::CameraDragClaim;
use hana_lagrange::CameraInputRoutingConfig;
use hana_lagrange::FreeCam;
use hana_lagrange::FreeCamInputMode;
use hana_lagrange::FreeCamPreset;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;

type TestResult<T> = Result<T, &'static str>;

const VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

struct Scene {
    app:            App,
    window:         Entity,
    camera:         Entity,
    slider:         Entity,
    button:         Entity,
    field:          Entity,
    target:         NormalizedRenderTarget,
    thumb_position: Vec2,
}

fn widget_tree() -> LayoutTree {
    let metrics = ControlMetrics::from_label_size(13.0_f32);
    let mut builder = LayoutBuilder::new(200.0, 80.0);
    builder.with(
        El::overlay()
            .size(metrics.slider.width, metrics.slider.thumb.height)
            .alignment(AlignX::Left, AlignY::Center)
            .widget("level", Slider::new(0.0..=1.0).value(0.0)),
        |builder| {
            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(metrics.slider.thumb.height))
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| {
                    builder.with(
                        builder
                            .child(El::overlay())
                            .id("track")
                            .width(Sizing::GROW)
                            .height(Sizing::fixed(metrics.slider.track_height))
                            .corner_radius(CornerRadius::all(metrics.slider.track_radius()))
                            .scroll_x(0.0),
                        |builder| {
                            builder.with(El::new().size(1.0, 1.0), |_| {});
                        },
                    );
                },
            );
            builder.with(
                El::overlay()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.with(
                        builder
                            .child(El::overlay())
                            .id("thumb")
                            .size(metrics.slider.thumb.width, metrics.slider.thumb.height)
                            .corner_radius(CornerRadius::all(metrics.slider.thumb.corner_radius()))
                            .scroll_x(0.0)
                            .slider_thumb(),
                        |builder| {
                            builder.with(El::new().size(1.0, 1.0), |_| {});
                        },
                    );
                },
            );
        },
    );
    builder.with(El::new().size(30.0, 12.0).button("action"), |_| {});
    builder.with(
        El::new().size(50.0, 12.0).editable_field(
            "name",
            ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text)),
            TextStyle::new(13.0),
        ),
        |_| {},
    );
    builder.build()
}

fn thumb_viewport_position(world: &World, panel: Entity, camera: Entity) -> TestResult<Vec2> {
    let authored = world.get::<DiegeticPanel>(panel).ok_or("panel remains")?;
    let computed = world
        .get::<ComputedDiegeticPanel>(panel)
        .ok_or("panel layout solves")?;
    let thumb = computed
        .scroll_records()
        .iter()
        .find(|record| record.id == PanelElementId::named("thumb"))
        .ok_or("thumb bounds solve")?;
    let (x, y) = thumb.bounds.center();
    let scale = authored.layout_unit().to_points() * authored.points_to_world();
    let (anchor_x, anchor_y) = authored.anchor_offsets();
    let local = Vec3::new(
        x.mul_add(scale, -anchor_x),
        y.mul_add(-scale, anchor_y),
        0.0,
    );
    let world_position = world
        .get::<GlobalTransform>(panel)
        .ok_or("panel has a world transform")?
        .affine()
        .transform_point3(local);
    world
        .get::<Camera>(camera)
        .ok_or("camera remains")?
        .world_to_viewport(
            world
                .get::<GlobalTransform>(camera)
                .ok_or("camera has a world transform")?,
            world_position,
        )
        .map_err(|_| "thumb projects into viewport")
}

fn scene(free_camera: bool) -> TestResult<Scene> {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Shader>()
        .init_asset::<Mesh>()
        .init_asset::<GizmoAsset>()
        .init_asset::<Image>()
        .init_asset::<ShaderBuffer>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<MouseScrollPixelsPerLine>()
        .init_resource::<Touches>()
        .add_message::<TouchInput>()
        .add_systems(PreUpdate, touch_screen_input_system.in_set(InputSystems))
        .insert_resource(DiegeticTextMeasurer::default())
        .add_plugins((
            PickingPlugin,
            InteractionPlugin,
            LagrangePlugin,
            DiegeticUiPlugin,
            WidgetDragClaimPlugin,
        ))
        .add_observer(slider_self_update);
    app.finish();

    let window = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    let target = RenderTarget::Window(WindowRef::Primary)
        .normalize(Some(window))
        .ok_or("primary window has a normalized target")?;
    app.world_mut().spawn(PointerId::Mouse);

    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: VIEWPORT.as_uvec2(),
        scale_factor:  1.0,
    });
    let mut projection = Projection::Perspective(PerspectiveProjection::default());
    projection.update(VIEWPORT.x, VIEWPORT.y);
    camera.computed.clip_from_view = projection.get_clip_from_view();
    let transform = Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
    let camera = if free_camera {
        app.world_mut()
            .spawn((
                FreeCam::default(),
                FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse()),
                camera,
                transform,
                GlobalTransform::from(transform),
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id()
    } else {
        app.world_mut()
            .spawn((
                OrbitCam::default(),
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                camera,
                transform,
                GlobalTransform::from(transform),
                RenderTarget::Window(WindowRef::Primary),
            ))
            .id()
    };
    app.insert_resource(CameraInputRoutingConfig::explicit(camera));

    let panel = DiegeticPanel::world()
        .size(Mm(200.0), Mm(80.0))
        .world_height(0.5)
        .with_tree(widget_tree())
        .build()
        .map_err(|_| "test panel builds")?;
    let panel = app.world_mut().spawn(panel).id();
    app.update();

    let thumb_position = thumb_viewport_position(app.world(), panel, camera)?;

    let widget = |app: &mut App, name| {
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named(name))
            })
            .map_err(|_| "widget lookup runs")?
            .ok_or("widget reifies")
    };
    let slider = widget(&mut app, "level")?;
    let button = widget(&mut app, "action")?;
    let field = widget(&mut app, "name")?;
    Ok(Scene {
        app,
        window,
        camera,
        slider,
        button,
        field,
        target,
        thumb_position,
    })
}

#[test]
fn disabled_widget_primary_press_still_claims_camera_drag() -> TestResult<()> {
    let mut scene = scene(false)?;
    let button = scene.button;
    assert!(
        scene
            .app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.override_interactivity(button, WidgetInteractivity::Disabled)
            })
            .map_err(|_| "interactivity override runs")?
    );
    scene.app.update();
    assert!(scene.app.world().get::<WidgetDisabled>(button).is_some());

    press_on(&mut scene, button);
    scene.app.update();
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Claimed
    );
    release(&mut scene);
    Ok(())
}

#[test]
fn touch_widget_press_claims_and_releases_when_touch_ends() -> TestResult<()> {
    const TOUCH_ID: u64 = 21;
    let mut scene = scene(false)?;
    let position = scene.thumb_position;
    scene.app.world_mut().write_message(TouchInput {
        phase: TouchPhase::Started,
        position,
        window: scene.window,
        force: None,
        id: TOUCH_ID,
    });
    scene.app.update();
    let location = pointer_location(&scene, position);
    scene.app.world_mut().trigger(PointerPress {
        entity:  scene.slider,
        pointer: Pointer::new(PointerId::Touch(TOUCH_ID), location),
        button:  PointerButton::Primary,
        hit:     HitData::new(scene.camera, 0.0, None, None),
        count:   1,
    });
    scene.app.world_mut().flush();
    assert_eq!(
        *scene
            .app
            .world()
            .resource::<hana_diegetic::WidgetPrimaryPressOwnership>(),
        hana_diegetic::WidgetPrimaryPressOwnership::HeldByWidget
    );
    scene.app.update();
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Claimed
    );

    scene.app.world_mut().write_message(TouchInput {
        phase: TouchPhase::Ended,
        position,
        window: scene.window,
        force: None,
        id: TOUCH_ID,
    });
    scene.app.update();
    assert_eq!(
        *scene
            .app
            .world()
            .resource::<hana_diegetic::WidgetPrimaryPressOwnership>(),
        hana_diegetic::WidgetPrimaryPressOwnership::Unclaimed
    );
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Unclaimed
    );
    Ok(())
}

fn hover(scene: &mut Scene, entity: Entity) {
    scene.app.world_mut().write_message(PointerHits::new(
        PointerId::Mouse,
        vec![(entity, HitData::new(scene.camera, 0.0, None, None))],
        0.0,
    ));
}

fn motion(scene: &mut Scene) {
    scene
        .app
        .world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::new(35.0, -18.0);
}

fn queue_primary_press(scene: &mut Scene, widget: Entity) {
    hover(scene, widget);
    let location = pointer_location(scene, scene.thumb_position);
    scene.app.world_mut().write_message(PointerInput::new(
        PointerId::Mouse,
        location,
        PointerAction::Press(PointerButton::Primary),
    ));
}

fn press_on(scene: &mut Scene, widget: Entity) {
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    hover(scene, widget);
    scene.app.update();
    let press_location = pointer_location(scene, scene.thumb_position);
    scene.app.world_mut().trigger(PointerPress {
        entity:  widget,
        pointer: Pointer::new(PointerId::Mouse, press_location),
        button:  PointerButton::Primary,
        hit:     HitData::new(scene.camera, 0.0, None, None),
        count:   1,
    });
    scene.app.world_mut().flush();
    assert_eq!(
        *scene
            .app
            .world()
            .resource::<hana_diegetic::WidgetPrimaryPressOwnership>(),
        hana_diegetic::WidgetPrimaryPressOwnership::HeldByWidget,
        "the global observer sees a widget press even if the widget stops propagation"
    );
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

fn release(scene: &mut Scene) {
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    scene
        .app
        .world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::ZERO;
    scene.app.update();
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Unclaimed
    );
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
}

fn pointer_location(scene: &Scene, position: Vec2) -> Location {
    Location {
        target: scene.target.clone(),
        position,
    }
}

fn assert_orbit_pose_unchanged(before: OrbitCam, held: &OrbitCam) {
    assert_eq!(held.orbit.current(), before.orbit.current());
    assert_eq!(held.orbit.target(), before.orbit.target());
    assert_eq!(held.pan.current(), before.pan.current());
    assert_eq!(held.pan.target(), before.pan.target());
    assert_eq!(held.zoom.current(), before.zoom.current());
    assert_eq!(held.zoom.target(), before.zoom.target());
}

#[test]
fn slider_thumb_press_claims_camera_before_first_frame_motion() -> TestResult<()> {
    let mut scene = scene(false)?;
    let before = *scene
        .app
        .world()
        .get::<OrbitCam>(scene.camera)
        .ok_or("orbit camera")?;
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    let slider = scene.slider;
    queue_primary_press(&mut scene, slider);
    motion(&mut scene);
    scene.app.update();

    assert_eq!(
        *scene
            .app
            .world()
            .resource::<hana_diegetic::WidgetPrimaryPressOwnership>(),
        hana_diegetic::WidgetPrimaryPressOwnership::HeldByWidget,
        "picking emits the widget press during this update"
    );
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Claimed,
        "the bridge claims the drag during the press update"
    );
    let held = scene
        .app
        .world()
        .get::<OrbitCam>(scene.camera)
        .ok_or("orbit camera")?;
    assert_orbit_pose_unchanged(before, held);
    Ok(())
}

#[test]
fn widget_press_released_within_one_frame_does_not_claim_camera() -> TestResult<()> {
    let mut scene = scene(false)?;
    let mut buttons = scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>();
    buttons.press(MouseButton::Left);
    buttons.release(MouseButton::Left);
    let slider = scene.slider;
    queue_primary_press(&mut scene, slider);
    scene.app.update();

    assert_eq!(
        *scene
            .app
            .world()
            .resource::<hana_diegetic::WidgetPrimaryPressOwnership>(),
        hana_diegetic::WidgetPrimaryPressOwnership::Unclaimed
    );
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Unclaimed
    );
    Ok(())
}

#[test]
fn slider_thumb_drag_changes_slider_without_orbiting_then_empty_space_orbits() -> TestResult<()> {
    let mut scene = scene(false)?;
    let before = *scene
        .app
        .world()
        .get::<OrbitCam>(scene.camera)
        .ok_or("orbit camera")?;
    let slider = scene.slider;
    press_on(&mut scene, slider);
    let drag_location = pointer_location(&scene, scene.thumb_position + Vec2::new(300.0, 0.0));
    scene.app.world_mut().trigger(PointerDrag {
        entity:   scene.slider,
        pointer:  Pointer::new(PointerId::Mouse, drag_location),
        button:   PointerButton::Primary,
        distance: Vec2::new(300.0, 0.0),
        delta:    Vec2::new(300.0, 0.0),
    });
    scene.app.world_mut().flush();
    motion(&mut scene);
    hover(&mut scene, slider);
    scene.app.update();
    assert!(
        scene
            .app
            .world()
            .get::<SliderState>(scene.slider)
            .ok_or("slider state")?
            .value()
            > 0.0
    );
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Claimed
    );
    let held = scene
        .app
        .world()
        .get::<OrbitCam>(scene.camera)
        .ok_or("orbit camera")?;
    assert_orbit_pose_unchanged(before, held);
    release(&mut scene);

    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    scene.app.update();
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    motion(&mut scene);
    scene.app.update();
    let after = scene
        .app
        .world()
        .get::<OrbitCam>(scene.camera)
        .ok_or("orbit camera")?;
    assert_ne!(
        after.orbit.target(),
        before.orbit.target(),
        "empty-space drag orbits"
    );
    Ok(())
}

#[test]
fn button_and_field_presses_hold_the_camera_claim_until_release() -> TestResult<()> {
    let mut scene = scene(false)?;
    for widget in [scene.button, scene.field] {
        let before = *scene
            .app
            .world()
            .get::<OrbitCam>(scene.camera)
            .ok_or("orbit camera")?;
        press_on(&mut scene, widget);
        motion(&mut scene);
        hover(&mut scene, widget);
        scene.app.update();
        assert_eq!(
            *scene.app.world().resource::<CameraDragClaim>(),
            CameraDragClaim::Claimed
        );
        let held = scene
            .app
            .world()
            .get::<OrbitCam>(scene.camera)
            .ok_or("orbit camera")?;
        assert_orbit_pose_unchanged(before, held);
        release(&mut scene);
    }
    Ok(())
}

#[test]
fn text_field_press_stays_claimed_after_pointer_leaves() -> TestResult<()> {
    let mut scene = scene(false)?;
    let field = scene.field;
    press_on(&mut scene, field);
    let camera = scene.camera;
    hover(&mut scene, camera);
    motion(&mut scene);
    scene.app.update();
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Claimed
    );
    release(&mut scene);
    Ok(())
}

#[test]
fn an_unclaimed_empty_space_drag_orbits() -> TestResult<()> {
    let mut scene = scene(false)?;
    let before = scene
        .app
        .world()
        .get::<OrbitCam>(scene.camera)
        .ok_or("orbit camera")?
        .orbit
        .target();
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    scene.app.update();
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    motion(&mut scene);
    scene.app.update();
    let after = scene
        .app
        .world()
        .get::<OrbitCam>(scene.camera)
        .ok_or("orbit camera")?
        .orbit
        .target();
    assert_ne!(after, before);
    Ok(())
}

#[test]
fn free_camera_right_drag_is_withheld_during_a_widget_press() -> TestResult<()> {
    let mut scene = scene(true)?;
    let before = scene
        .app
        .world()
        .get::<FreeCam>(scene.camera)
        .ok_or("free camera")?
        .look
        .target();
    let button = scene.button;
    press_on(&mut scene, button);
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Right);
    motion(&mut scene);
    hover(&mut scene, button);
    scene.app.update();
    assert_eq!(
        *scene.app.world().resource::<CameraDragClaim>(),
        CameraDragClaim::Claimed
    );
    let held = scene
        .app
        .world()
        .get::<FreeCam>(scene.camera)
        .ok_or("free camera")?;
    assert_eq!(held.look.target(), before);
    scene
        .app
        .world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Right);
    release(&mut scene);
    Ok(())
}
