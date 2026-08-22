//! Demonstrates the three one-shot camera triggers `ZoomToFit`, `LookAt`, and
//! `LookAtAndZoomToFit` — each constructed with `(camera, target)` entities and
//! fired through `Commands::trigger`. The target cube is spawned at startup and
//! its entity stashed in a resource so the keyboard system can name it.
//!
//! Controls:
//!   Z - `ZoomToFit` the cube (frames without changing look direction)
//!   L - `LookAt` the cube (rotates camera in place)
//!   K - `LookAtAndZoomToFit` the cube (rotates + frames)
//!   H - Return to the camera home pose
//!
//! Compare K vs Z: both frame the target, but `LookAtAndZoomToFit` also
//! changes the orbit focus to the target, while `ZoomToFit` keeps the
//! current focus and only adjusts radius.

use std::time::Duration;

use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::Face;
use fairy_dust::TitleBar;
use fairy_dust::cube_face_label;
use hana_lagrange::AnimationBegin;
use hana_lagrange::AnimationEnd;
use hana_lagrange::AnimationSource;
use hana_lagrange::LookAt;
use hana_lagrange::LookAtAndZoomToFit;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::ZoomBegin;
use hana_lagrange::ZoomEnd;
use hana_lagrange::ZoomToFit;

const EXAMPLE_TITLE: &str = "Zoom to Fit";

// Camera home pose.
const HOME_PITCH: f32 = 0.46;
const HOME_MARGIN: f32 = 0.7;

// Cube placement (reference cube left of origin, target cube right).
const CUBE_SIZE: f32 = 1.0;
const CUBE_Y: f32 = CUBE_SIZE / 2.0 + 0.05;
const CUBE_X_OFFSET: f32 = 8.0 / 6.0;

const REFERENCE_CUBE_COLOR: Color = Color::srgb(0.5, 0.5, 0.5);
const REFERENCE_CUBE_TRANSLATION: Vec3 = Vec3::new(-CUBE_X_OFFSET, CUBE_Y, 0.0);

const TARGET_COLOR: Color = Color::srgb(0.8, 0.7, 0.6);
const TARGET_TRANSLATION: Vec3 = Vec3::new(CUBE_X_OFFSET, CUBE_Y, 0.0);

// Cube-face labels.
const REFERENCE_LABEL: &str = "Home";
const TARGET_LABEL: &str = "Look At Me";
// HUD chip strings (also keyboard hints).
const ZOOM_CONTROL: &str = "Z ZoomToFit";
const LOOK_CONTROL: &str = "L LookAt";
const LOOK_AND_ZOOM_CONTROL: &str = "K LookAtAndZoomToFit";

// Trigger tuning.
const FIT_DURATION: Duration = Duration::from_millis(800);
const FIT_MARGIN: f32 = 0.15;
const LOOK_AT_DURATION: Duration = Duration::from_millis(600);

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .size(CUBE_SIZE)
        .color(REFERENCE_CUBE_COLOR)
        .transform(Transform::from_translation(REFERENCE_CUBE_TRANSLATION))
        .face_label(Face::Front, REFERENCE_LABEL)
        .insert(CameraHomeTarget)
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_camera_home()
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(ZOOM_CONTROL)
                .control(LOOK_CONTROL)
                .control(LOOK_AND_ZOOM_CONTROL),
        )
        .wire_chip_to_events::<ZoomBegin, ZoomEnd>(ZOOM_CONTROL)
        .wire_chip_to_events_filtered::<AnimationBegin, AnimationEnd, _, _>(
            LOOK_CONTROL,
            |e| e.source == AnimationSource::LookAt,
            |e| e.source == AnimationSource::LookAt,
        )
        .wire_chip_to_events_filtered::<AnimationBegin, AnimationEnd, _, _>(
            LOOK_AND_ZOOM_CONTROL,
            |e| e.source == AnimationSource::LookAtAndZoomToFit,
            |e| e.source == AnimationSource::LookAtAndZoomToFit,
        )
        .with_camera_control_panel()
        .add_systems(Startup, spawn_target)
        .with_shortcut(KeyCode::KeyZ, zoom_to_fit_target)
        .with_shortcut(KeyCode::KeyL, look_at_target)
        .with_shortcut(KeyCode::KeyK, look_at_and_zoom_to_fit_target)
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// TRIGGERS — ZoomToFit / LookAt / LookAtAndZoomToFit. Each is constructed with
// `(camera, target)` entities and fired through `Commands::trigger`.
// ═════════════════════════════════════════════════════════════════════════════

#[derive(Component)]
struct Target;

#[derive(Resource)]
struct TargetEntity(Entity);

// Spawn the cube the triggers will frame / look at, and stash its entity in
// a resource so keyboard_input can read it later.
fn spawn_target(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let entity = commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(CUBE_SIZE)))),
            MeshMaterial3d(materials.add(StandardMaterial::from(TARGET_COLOR))),
            Transform::from_translation(TARGET_TRANSLATION),
            Target,
        ))
        .with_children(|parent| {
            parent.spawn(cube_face_label(Face::Front, TARGET_LABEL, CUBE_SIZE));
        })
        .id();
    commands.insert_resource(TargetEntity(entity));
}

// Z fires ZoomToFit (radius only). Each handler is a plain system wired to its
// key by `with_shortcut`, which only fires it while no modifier is held.
fn zoom_to_fit_target(
    mut commands: Commands,
    camera_query: Query<Entity, With<OrbitCam>>,
    target: Res<TargetEntity>,
) {
    let Ok(camera) = camera_query.single() else {
        return;
    };
    commands.trigger(
        ZoomToFit::new(camera, target.0)
            .margin(FIT_MARGIN)
            .duration(FIT_DURATION),
    );
}

// L fires LookAt (rotation only).
fn look_at_target(
    mut commands: Commands,
    camera_query: Query<Entity, With<OrbitCam>>,
    target: Res<TargetEntity>,
) {
    let Ok(camera) = camera_query.single() else {
        return;
    };
    commands.trigger(LookAt::new(camera, target.0).duration(LOOK_AT_DURATION));
}

// K fires LookAtAndZoomToFit (rotation + radius + focus move).
fn look_at_and_zoom_to_fit_target(
    mut commands: Commands,
    camera_query: Query<Entity, With<OrbitCam>>,
    target: Res<TargetEntity>,
) {
    let Ok(camera) = camera_query.single() else {
        return;
    };
    commands.trigger(
        LookAtAndZoomToFit::new(camera, target.0)
            .margin(FIT_MARGIN)
            .duration(FIT_DURATION),
    );
}
