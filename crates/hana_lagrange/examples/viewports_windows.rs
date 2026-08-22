//! Demonstrates composing multiple independent `OrbitCam`s in one app:
//! `Camera::order` layers a minimap overlay on top of the main view;
//! `Camera::viewport` clips that overlay to a square in the top-right corner;
//! `RenderTarget::Window` aims a third camera at a second OS window spawned
//! via `hana_clerestory::ManagedWindow`; and
//! `ResolvedCameraInputRoute::routed_camera()` resolves which camera the
//! cursor is currently over so input goes to that one.
//!
//! The primary window has a full-size view and a minimap overlay in the
//! top-right corner. A second OS window shows a separate camera angle.
//!
//! Controls:
//!   H - Home the camera the cursor is currently over.

use std::time::Duration;

use bevy::camera::RenderTarget;
use bevy::camera::Viewport;
use bevy::prelude::Camera;
use bevy::prelude::ClearColorConfig;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::MessageReader;
use bevy::prelude::Name;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Single;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::UVec2;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::window::ClosingWindow;
use bevy::window::WindowRef;
use bevy::window::WindowResized;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeEntity;
use fairy_dust::CameraHomeTarget;
use fairy_dust::Face;
use fairy_dust::TitleBar;
use hana_clerestory::ManagedWindow;
use hana_lagrange::AnimateToFit;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;

// app / title bar
const EXAMPLE_TITLE: &str = "Viewports & Windows";

// camera
const MINIMAP_CAMERA_ORDER: isize = 1;
const MINIMAP_CAMERA_TRANSLATION: Vec3 = Vec3::new(1.0, 1.5, 4.0);
const PRIMARY_CAMERA_TRANSLATION: Vec3 = Vec3::new(0.0, 0.5, 5.0);
const SECOND_WINDOW_CAMERA_TRANSLATION: Vec3 = Vec3::new(5.0, 1.5, 7.0);

// cube
const BACK_FACE_LABEL: &str = "BACK";
const BOTTOM_FACE_LABEL: &str = "BOTTOM";
const CUBE_COLOR: Color = Color::srgb(0.8, 0.7, 0.6);
const CUBE_SIZE: f32 = 1.0;
const CUBE_TRANSLATION: Vec3 = Vec3::new(0.0, 0.8, 0.0);
const FACE_LABEL_COLOR: Color = Color::srgb(0.9, 0.3, 0.1);
const FACE_LABEL_SIZE: f32 = 0.15;
const FRONT_FACE_LABEL: &str = "FRONT";
const LEFT_FACE_LABEL: &str = "LEFT";
const RIGHT_FACE_LABEL: &str = "RIGHT";
const TOP_FACE_LABEL: &str = "TOP";

// home pose
const PRIMARY_HOME_YAW: f32 = 0.0;
const PRIMARY_HOME_PITCH: f32 = 0.46;

// home animation
const HOME_MARGIN: f32 = 0.2;
const HOME_PROXY_READY_EPSILON: f32 = 0.001;

// viewport
const MINIMAP_VIEWPORT_DIVISOR: u32 = 5;

// window
const MINIMAP_CAMERA_NAME: &str = "Minimap";
const PRIMARY_CAMERA_NAME: &str = "Main";
const SECOND_WINDOW_CAMERA_NAME: &str = "Second window";
const SECOND_WINDOW_NAME: &str = "second_window";
const SECOND_WINDOW_TITLE: &str = "Second window";

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(Transform::from_translation(CUBE_TRANSLATION))
        .face_text(
            Face::Front,
            FRONT_FACE_LABEL,
            FACE_LABEL_SIZE,
            FACE_LABEL_COLOR,
        )
        .face_text(
            Face::Back,
            BACK_FACE_LABEL,
            FACE_LABEL_SIZE,
            FACE_LABEL_COLOR,
        )
        .face_text(Face::Top, TOP_FACE_LABEL, FACE_LABEL_SIZE, FACE_LABEL_COLOR)
        .face_text(
            Face::Bottom,
            BOTTOM_FACE_LABEL,
            FACE_LABEL_SIZE,
            FACE_LABEL_COLOR,
        )
        .face_text(
            Face::Left,
            LEFT_FACE_LABEL,
            FACE_LABEL_SIZE,
            FACE_LABEL_COLOR,
        )
        .face_text(
            Face::Right,
            RIGHT_FACE_LABEL,
            FACE_LABEL_SIZE,
            FACE_LABEL_COLOR,
        )
        .insert(CameraHomeTarget)
        .with_camera_home()
        .yaw(PRIMARY_HOME_YAW)
        .pitch(PRIMARY_HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft),
        )
        .with_camera_control_panel()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                cleanup_cameras_on_window_close,
                set_camera_viewports,
                home_main_camera_on_startup,
            ),
        )
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// MULTI-CAMERA SETUP — composing OrbitCam with Camera::order, Camera::viewport,
// RenderTarget::Window, and ResolvedCameraInputRoute.
//
// How it works:
//   1. `setup` (Startup) spawns three OrbitCams. The main camera renders the whole primary window.
//      The minimap camera has `order: 1` and a transparent clear so it composites on top of the
//      main view; its `viewport` is left None here and filled in by `set_camera_viewports`. The
//      second camera carries `RenderTarget::Window(WindowRef::Entity(...))` so its output goes to
//      the second OS window. Each camera's `OrbitCamHomePose` is captured by Lagrange from its
//      spawn pose.
//   2. `set_camera_viewports` (Update) listens for `WindowResized` and resets the minimap camera's
//      `Camera::viewport` to a square in physical pixels. Viewports are physical-pixel rects, so
//      they must be recomputed each resize.
//   3. `home_main_camera_on_startup` (Update, fires once) waits until the Fairy Dust home proxy has
//      settled at `CUBE_TRANSLATION`, then fires `AnimateToFit` on the main camera with
//      `Duration::ZERO` so the scene opens already framed.
//   4. Lagrange's filled home binding handles H for the routed camera. Non-routed cameras have
//      inactive input contexts, so only the hovered camera receives home.
//   5. `cleanup_cameras_on_window_close` (Update) despawns any camera whose `RenderTarget::Window`
//      references a `ClosingWindow`, so Bevy's camera system doesn't panic on a stale render
//      target.
// ═════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
struct MainCamera;

#[derive(Component)]
struct MinimapCamera;

// Spawns the three OrbitCams. The main camera renders the full primary window;
// the minimap camera renders on top of it with a higher `Camera::order` and a
// transparent clear so its viewport (set later by `set_camera_viewports`)
// composites over the main view. The second camera renders to its own OS
// window via `RenderTarget::Window`.
fn setup(mut commands: Commands) {
    commands.spawn((
        Name::new(PRIMARY_CAMERA_NAME),
        Transform::from_translation(PRIMARY_CAMERA_TRANSLATION),
        orbit_cam_default(),
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        MainCamera,
    ));

    commands.spawn((
        Name::new(MINIMAP_CAMERA_NAME),
        Transform::from_translation(MINIMAP_CAMERA_TRANSLATION),
        Camera {
            order: MINIMAP_CAMERA_ORDER,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        orbit_cam_default(),
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        MinimapCamera,
    ));

    let second_window = commands
        .spawn((
            Window {
                title: SECOND_WINDOW_TITLE.to_owned(),
                ..default()
            },
            ManagedWindow {
                name: SECOND_WINDOW_NAME.into(),
            },
        ))
        .id();

    commands.spawn((
        Name::new(SECOND_WINDOW_CAMERA_NAME),
        Transform::from_translation(SECOND_WINDOW_CAMERA_TRANSLATION),
        Camera::default(),
        RenderTarget::Window(WindowRef::Entity(second_window)),
        orbit_cam_default(),
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
    ));
}

// Fires AnimateToFit on the main camera once the home proxy entity has
// settled at CUBE_TRANSLATION. Runs every frame until it fires once, then
// short-circuits via the `fired` local.
fn home_main_camera_on_startup(
    mut commands: Commands,
    mut fired: Local<bool>,
    home: Option<Res<CameraHomeEntity>>,
    main: Query<Entity, With<MainCamera>>,
    transforms: Query<&Transform>,
) {
    if *fired {
        return;
    }
    let Some(home) = home else {
        return;
    };
    let Ok(camera) = main.single() else {
        return;
    };
    let Ok(home_transform) = transforms.get(home.0) else {
        return;
    };
    if home_transform.translation.distance(CUBE_TRANSLATION) > HOME_PROXY_READY_EPSILON {
        return;
    }
    commands.trigger(
        AnimateToFit::new(camera, home.0)
            .yaw(PRIMARY_HOME_YAW)
            .pitch(PRIMARY_HOME_PITCH)
            .duration(Duration::ZERO)
            .margin(HOME_MARGIN),
    );
    *fired = true;
}

/// Despawns cameras whose render-target window is marked `ClosingWindow`.
/// Prevents `camera_system` from panicking on a stale `RenderTarget`.
fn cleanup_cameras_on_window_close(
    mut commands: Commands,
    closing: Query<Entity, With<ClosingWindow>>,
    cameras: Query<(Entity, &RenderTarget)>,
) {
    for (camera_entity, target) in &cameras {
        if let RenderTarget::Window(WindowRef::Entity(window)) = target
            && closing.get(*window).is_ok()
        {
            commands.entity(camera_entity).despawn();
        }
    }
}

// Resizes the minimap viewport to a square in the top-right corner each time
// the primary window resizes — the minimap camera's `viewport` is set in
// physical pixels, so it must be recomputed on every WindowResized.
fn set_camera_viewports(
    windows: Query<&Window>,
    mut resize_events: MessageReader<WindowResized>,
    mut right_camera: Single<&mut Camera, With<MinimapCamera>>,
) {
    for resize_event in resize_events.read() {
        let Ok(window) = windows.get(resize_event.window) else {
            continue;
        };
        let size = window.resolution.physical_width() / MINIMAP_VIEWPORT_DIVISOR;
        right_camera.viewport = Some(Viewport {
            physical_position: UVec2::new(window.resolution.physical_width() - size, 0),
            physical_size: UVec2::new(size, size),
            ..default()
        });
    }
}

fn orbit_cam_default() -> OrbitCam { OrbitCam::default() }
