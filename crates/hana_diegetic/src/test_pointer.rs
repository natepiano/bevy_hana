//! Real pointer input shared by application, module and diegetic tests.
//!
//! Mount this source with `#[path]` in a test module. Input goes through the
//! production picking schedules and size gate; no hit or interaction is fabricated.
//! Move/press/release enqueue input. Call the fixture frame step to dispatch it.
//! Drag accepts that same step so each movement passes through the real backend.
#![allow(
    clippy::expect_used,
    reason = "invalid pointer fixtures must fail immediately"
)]
use bevy::camera::RenderTarget;
use bevy::camera::RenderTargetInfo;
use bevy::ecs::world::EntityRef;
use bevy::picking::InteractionPlugin;
use bevy::picking::PickingPlugin;
use bevy::picking::PickingSettings;
use bevy::picking::pointer::Location;
use bevy::picking::pointer::PointerAction;
use bevy::picking::pointer::PointerId;
use bevy::picking::pointer::PointerInput;
use bevy::picking::pointer::PointerLocation;
use bevy::prelude::*;
use bevy::window::WindowRef;

/// Installs the ordinary picking pipeline before a headless app is finished.
pub(crate) fn install(app: &mut App) {
    if !app.is_plugin_added::<bevy::input::InputPlugin>() {
        app.add_plugins(bevy::input::InputPlugin);
    }
    if !app.is_plugin_added::<PickingPlugin>() {
        app.add_plugins(PickingPlugin);
    }
    if !app.is_plugin_added::<InteractionPlugin>() {
        app.add_plugins(InteractionPlugin);
    }
    app.add_message::<bevy::window::WindowEvent>()
        .add_message::<bevy::window::CursorMoved>();
    if !app.is_plugin_added::<bevy::picking::input::PointerInputPlugin>()
        && app
            .world_mut()
            .query::<&PointerId>()
            .iter(app.world())
            .all(|id| *id != PointerId::Mouse)
    {
        app.world_mut()
            .spawn((PointerId::Mouse, PointerLocation::default()));
    }
}

/// Moves a real window cursor and sends its raw input through production picking.
pub(crate) fn move_cursor(app: &mut App, window: Entity, position: Vec2) {
    move_cursor_world(app.world_mut(), window, position);
}

/// Cursor writer usable by a fixture that follows a moving target before picking.
pub(crate) fn move_cursor_world(world: &mut World, window: Entity, position: Vec2) {
    let target = RenderTarget::Window(WindowRef::Entity(window))
        .normalize(None)
        .expect("pointer window");
    move_on_target(world, window, target, position);
}

fn move_on_target(
    world: &mut World,
    window: Entity,
    target: bevy::camera::NormalizedRenderTarget,
    position: Vec2,
) {
    enable_input(world);

    let delta = world
        .get::<Window>(window)
        .expect("pointer window")
        .cursor_position()
        .map_or(Vec2::ZERO, |previous| position - previous);
    let event = bevy::window::CursorMoved {
        window,
        position,
        delta: Some(delta),
    };
    world
        .get_mut::<Window>(window)
        .expect("hover window")
        .set_cursor_position(Some(position));
    // Feed PointerInput once. PointerInputPlugin translates WindowEvent into
    // another raw Move; writing both would dispatch the same movement twice.
    world.write_message(event);
    let location = Location { target, position };
    world.insert_resource(TestCursor {
        window,
        location: location.clone(),
    });
    world.write_message(PointerInput::new(
        PointerId::Mouse,
        location,
        PointerAction::Move { delta },
    ));
}

/// Presses at the current cursor; production picking decides who owns the press.
pub(crate) fn press(app: &mut App, button: PointerButton) {
    action(app, PointerAction::Press(button));
}

/// Releases the current pointer through the same input pipeline.
pub(crate) fn release(app: &mut App, button: PointerButton) {
    action(app, PointerAction::Release(button));
}

fn action(app: &mut App, action: PointerAction) {
    enable_input(app.world_mut());
    let location = app.world().resource::<TestCursor>().location.clone();
    app.world_mut()
        .write_message(PointerInput::new(PointerId::Mouse, location, action));
}

fn enable_input(world: &mut World) {
    let mut settings = world.resource_mut::<PickingSettings>();
    settings.is_enabled = true;
    settings.is_input_enabled = true;
    settings.is_hover_enabled = true;
    settings.is_window_picking_enabled = true;
}

/// Returns the explicit window of a picking camera (including a primary-window camera).
pub(crate) fn window(world: &World, camera: Entity) -> Entity {
    match world.get::<RenderTarget>(camera) {
        Some(RenderTarget::Window(WindowRef::Entity(window))) => Some(*window),
        Some(RenderTarget::Window(WindowRef::Primary)) | None => world
            .iter_entities()
            .find(EntityRef::contains::<bevy::window::PrimaryWindow>)
            .as_ref()
            .map(EntityRef::id),
        _ => None,
    }
    .expect("window input requires a window picking camera")
}

/// Projects a world point with the camera that actually owns the cursor.
pub(crate) fn move_world(app: &mut App, camera: Entity, point: Vec3) {
    refresh_camera(app, camera);
    let world = app.world();
    let position = world
        .get::<Camera>(camera)
        .expect("hover camera")
        .world_to_viewport(
            world
                .get::<GlobalTransform>(camera)
                .expect("hover camera pose"),
            point,
        )
        .expect("hover target projects");
    let target = world.get::<RenderTarget>(camera).cloned();
    if let Some(RenderTarget::Image(image)) = target {
        let host = current_window(world);
        move_on_target(
            app.world_mut(),
            host,
            RenderTarget::Image(image)
                .normalize(None)
                .expect("image preview target"),
            position,
        );
    } else {
        let window = window(world, camera);
        move_cursor(app, window, position);
    }
}

/// Moves clear of all scene controls without editing the hover map.
pub(crate) fn leave(app: &mut App, camera: Entity) {
    let target = app.world().get::<RenderTarget>(camera).cloned();
    if let Some(RenderTarget::Image(image)) = target {
        let host = current_window(app.world());
        move_on_target(
            app.world_mut(),
            host,
            RenderTarget::Image(image)
                .normalize(None)
                .expect("image preview target"),
            Vec2::splat(-1000.0),
        );
    } else {
        let window = window(app.world(), camera);
        move_cursor(app, window, Vec2::splat(-1000.0));
    }
}

/// Reuses the primary window installed by `DefaultPlugins` instead of creating a
/// second one. Bevy's viewport hit test requires exactly one primary window,
/// including for cameras with explicit window or image targets.
pub(crate) fn primary_window(app: &mut App, size: UVec2) -> Entity {
    let existing = app
        .world_mut()
        .query_filtered::<Entity, With<bevy::window::PrimaryWindow>>()
        .iter(app.world())
        .collect::<Vec<_>>();
    assert!(
        existing.len() <= 1,
        "pointer fixture has multiple primary windows"
    );
    let window = existing.first().copied().unwrap_or_else(|| {
        app.world_mut()
            .spawn((Window::default(), bevy::window::PrimaryWindow))
            .id()
    });
    app.world_mut()
        .get_mut::<Window>(window)
        .expect("primary window")
        .resolution
        .set_physical_resolution(size.x, size.y);
    window
}

/// Seeds a window camera for fixtures without the renderer's camera update system.
pub(crate) fn camera(app: &mut App, window: Entity, pose: Transform) -> Entity {
    if app
        .world_mut()
        .query_filtered::<Entity, With<bevy::window::PrimaryWindow>>()
        .iter(app.world())
        .next()
        .is_none()
    {
        app.world_mut()
            .entity_mut(window)
            .insert(bevy::window::PrimaryWindow);
    }
    let size = app
        .world()
        .get::<Window>(window)
        .expect("camera window")
        .physical_size();
    // A focused fixture view owns input ahead of any application overview.
    let order = app
        .world_mut()
        .query::<&Camera>()
        .iter(app.world())
        .map(|camera| camera.order)
        .max()
        .unwrap_or(-1)
        + 1;
    let mut camera = Camera { order, ..default() };
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: size,
        scale_factor:  1.0,
    });
    let mut projection = Projection::Perspective(PerspectiveProjection::default());
    projection.update(size.as_vec2().x, size.as_vec2().y);
    camera.computed.clip_from_view = projection.get_clip_from_view();
    app.world_mut()
        .spawn((
            camera,
            Camera3d::default(),
            projection,
            RenderTarget::Window(WindowRef::Entity(window)),
            pose,
            GlobalTransform::from(pose),
        ))
        .id()
}

/// Installs an actual raycast backend in a minimal, renderer-free fixture.
pub(crate) fn install_mesh(app: &mut App) {
    if !app.is_plugin_added::<AssetPlugin>() {
        app.add_plugins(AssetPlugin::default());
    }
    if !app.is_plugin_added::<bevy::mesh::MeshPlugin>() {
        app.add_plugins(bevy::mesh::MeshPlugin);
    }
    if !app.is_plugin_added::<TransformPlugin>() {
        app.add_plugins(TransformPlugin);
    }
    if !app.is_plugin_added::<bevy::camera::visibility::VisibilityPlugin>() {
        app.add_plugins(bevy::camera::visibility::VisibilityPlugin);
    }
    install(app);
    if !app.is_plugin_added::<bevy::picking::mesh_picking::MeshPickingPlugin>() {
        app.add_plugins(bevy::picking::mesh_picking::MeshPickingPlugin);
    }
    // Headless fixtures do not run a renderer's view-visibility pass. Geometry,
    // ray generation, intersection and hover dispatch remain the real backend.
    app.world_mut()
        .resource_mut::<bevy::picking::mesh_picking::MeshPickingSettings>()
        .ray_cast_visibility = bevy::picking::mesh_picking::ray_cast::RayCastVisibility::Any;
}

/// Supplies the camera update normally provided by a renderer in bare fixtures.
fn refresh_camera(app: &mut App, camera: Entity) {
    if app.is_plugin_added::<bevy::render::RenderPlugin>() {
        return;
    }
    if matches!(
        app.world().get::<RenderTarget>(camera),
        Some(RenderTarget::Image(_))
    ) {
        return;
    }
    let window = window(app.world(), camera);
    let view = app.world().get::<Window>(window).expect("camera window");
    let info = RenderTargetInfo {
        physical_size: view.physical_size(),
        scale_factor:  view.resolution.scale_factor(),
    };
    let clip = app.world().get::<Projection>(camera).map(|projection| {
        let mut projection = projection.clone();
        let size = app
            .world()
            .get::<Camera>(camera)
            .and_then(|camera| camera.viewport.as_ref())
            .map_or(info.physical_size, |viewport| viewport.physical_size)
            .as_vec2();
        projection.update(size.x, size.y);
        projection.get_clip_from_view()
    });
    let mut lens = app
        .world_mut()
        .get_mut::<Camera>(camera)
        .expect("picking camera");
    lens.computed.target_info = Some(info);
    if let Some(clip) = clip {
        lens.computed.clip_from_view = clip;
    }
}

/// Window hosting the latest test cursor, including an image preview.
#[derive(Resource)]
struct TestCursor {
    window:   Entity,
    location: Location,
}

/// Returns the window that actually received the latest cursor input.
pub(crate) fn current_window(world: &World) -> Entity {
    world.get_resource::<TestCursor>().map_or_else(
        || {
            world
                .iter_entities()
                .find(EntityRef::contains::<Window>)
                .expect("hover window")
                .id()
        },
        |cursor| cursor.window,
    )
}

#[cfg(test)]
mod tests {
    use bevy::picking::hover::HoverMap;
    use bevy::picking::hover::PickingInteraction;

    use super::*;

    #[test]
    fn test_pointer_delivers_mesh_hover_press_release_and_leave() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        install_mesh(&mut app);
        let size = UVec2::new(800, 600);
        let window = primary_window(&mut app, size);
        assert_eq!(primary_window(&mut app, size), window);
        let camera = camera(&mut app, window, Transform::from_xyz(0.0, 0.0, 5.0));
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::default());
        let target = app
            .world_mut()
            .spawn((
                Mesh3d(mesh),
                Transform::default(),
                Visibility::Visible,
                Pickable::default(),
                PickingInteraction::None,
            ))
            .id();
        for _ in 0..3 {
            app.update();
        }
        move_world(&mut app, camera, Vec3::ZERO);
        app.update();
        assert!(app.world().resource::<HoverMap>()[&PointerId::Mouse].contains_key(&target));
        press(&mut app, PointerButton::Primary);
        app.update();
        assert_eq!(
            app.world().get::<PickingInteraction>(target),
            Some(&PickingInteraction::Pressed)
        );
        release(&mut app, PointerButton::Primary);
        app.update();
        assert_eq!(
            app.world().get::<PickingInteraction>(target),
            Some(&PickingInteraction::Hovered)
        );
        leave(&mut app, camera);
        app.update();
        assert!(
            !app.world()
                .resource::<HoverMap>()
                .get(&PointerId::Mouse)
                .is_some_and(|hits| hits.contains_key(&target))
        );
        assert_eq!(current_window(app.world()), window);
    }
}
