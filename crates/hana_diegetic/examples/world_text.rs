//! @generated `bevy_example_template`
//! `WorldText` example — standalone MSDF text in world space.
//!
//! Demonstrates `WorldText` on a ground plane, on each face of a cube, and on
//! an anchor demo panel. Press `X`/`Y`/`Z` to rotate the anchor panel and the
//! labeled cube around the matching local axis; press `H` to return to the
//! home camera pose.
//!
//! This example runs OIT (`.with_stable_transparency()`), which forces
//! `Msaa::Off` so blended world text can sort correctly.
//!
//! # Code layout
//!
//! Read top-to-bottom for the demonstrated API:
//!   - `main()` wires the app (cube + face text, stable-transparency OIT, title-bar chips).
//!   - **WORLD TEXT** section spawns the anchor demo and the ground label.
//!   - **ROTATION ANIMATION** at the end is decorative scaffolding.

use bevy::light::NotShadowCaster;
use bevy::prelude::AlphaMode;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Event;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Sphere;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use fairy_dust::CameraHomeTarget;
use fairy_dust::Face;
use fairy_dust::TitleBar;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticText;
use hana_lagrange::OrbitCamPreset;

// Camera home pose.
const HOME_YAW: f32 = 0.015;
const HOME_PITCH: f32 = 0.2;

// Title-bar control labels (also the keys for chip wiring).
const X_ROTATE_CONTROL: &str = "X Rotate";
const Y_ROTATE_CONTROL: &str = "Y Rotate";
const Z_ROTATE_CONTROL: &str = "Z Rotate";

// Rotation animation.
const ROTATION_SPEED: f32 = 1.5;

// Cube + per-face label styling.
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = Vec3::new(-2.5, 1.0, 2.5);
const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_FACE_TEXT_SIZE: f32 = 0.16;
const CUBE_FACE_TEXT_COLOR: Color = fairy_dust::CUBE_FACE_PANEL_BLUE;
const CUBE_YAW: f32 = 20.0;

// Ground-plane label.
const GROUND_TEXT_SIZE: f32 = 0.48;
const GROUND_TEXT_TRANSLATION: Vec3 = Vec3::new(0.0, 0.001, 1.35);

// Translucent backdrop behind the anchor demo labels.
const ANCHOR_FRAME_COLOR: Color = Color::srgba(0.08, 0.08, 0.08, 0.18);
const ANCHOR_FRAME_SIZE: Vec3 = Vec3::new(3.6, 2.0, 0.18);
const ANCHOR_FRAME_LOCAL_OFFSET: Vec3 = Vec3::new(0.0, -0.2, -0.1);

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(
            Transform::from_translation(CUBE_TRANSLATION)
                .with_rotation(Quat::from_rotation_y(CUBE_YAW.to_radians())),
        )
        .insert((CameraHomeTarget, DemoCube))
        .face_text(
            Face::Front,
            "FRONT",
            CUBE_FACE_TEXT_SIZE,
            CUBE_FACE_TEXT_COLOR,
        )
        .face_text(
            Face::Back,
            "BACK",
            CUBE_FACE_TEXT_SIZE,
            CUBE_FACE_TEXT_COLOR,
        )
        .face_text(Face::Top, "TOP", CUBE_FACE_TEXT_SIZE, CUBE_FACE_TEXT_COLOR)
        .face_text(
            Face::Bottom,
            "BOTTOM",
            CUBE_FACE_TEXT_SIZE,
            CUBE_FACE_TEXT_COLOR,
        )
        .face_text(
            Face::Left,
            "LEFT",
            CUBE_FACE_TEXT_SIZE,
            CUBE_FACE_TEXT_COLOR,
        )
        .face_text(
            Face::Right,
            "RIGHT",
            CUBE_FACE_TEXT_SIZE,
            CUBE_FACE_TEXT_COLOR,
        )
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_stable_transparency()
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .with_title_bar(
            TitleBar::new()
                .with_title("World Text")
                .with_anchor(Anchor::TopLeft)
                .control(X_ROTATE_CONTROL)
                .control(Y_ROTATE_CONTROL)
                .control(Z_ROTATE_CONTROL),
        )
        .wire_chip_to_events_filtered::<RotationBegin, RotationEnd, _, _>(
            X_ROTATE_CONTROL,
            |e| e.axis == Vec3::X,
            |e| e.axis == Vec3::X,
        )
        .wire_chip_to_events_filtered::<RotationBegin, RotationEnd, _, _>(
            Y_ROTATE_CONTROL,
            |e| e.axis == Vec3::Y,
            |e| e.axis == Vec3::Y,
        )
        .wire_chip_to_events_filtered::<RotationBegin, RotationEnd, _, _>(
            Z_ROTATE_CONTROL,
            |e| e.axis == Vec3::Z,
            |e| e.axis == Vec3::Z,
        )
        .with_camera_control_panel()
        .init_resource::<AnchorRotation>()
        .add_systems(Startup, setup)
        .add_systems(Update, rotate_anchor_demo)
        // X / Y / Z run through Fairy Dust's shortcut binding, which fires each
        // only when no modifier is held.
        .with_shortcut(KeyCode::KeyX, rotate_x)
        .with_shortcut(KeyCode::KeyY, rotate_y)
        .with_shortcut(KeyCode::KeyZ, rotate_z)
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// WORLD TEXT — spawning [`WorldText`] entities with [`TextStyle`] and the
// nine [`Anchor`] variants. This is the primary API the example demonstrates;
// the labels on the cube faces are added in `main()` via `face_text`.
// ═════════════════════════════════════════════════════════════════════════════

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    spawn_anchor_demo(&mut commands, &mut meshes, &mut materials);
    spawn_ground_text(&mut commands);
}

/// Spawns the anchor demo: a translucent backdrop plane, title, instructions,
/// nine anchor-point labels with red dot markers, and the `AnchorDemoText`
/// components.
fn spawn_anchor_demo(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
) {
    let demo_center = Vec3::new(2.0, 1.5, -0.5);
    let demo_rotation = Quat::from_rotation_y(-15.0_f32.to_radians());

    // Backdrop frame plane — sits behind the anchor labels in the demo's
    // local Z, slightly transparent, ground-plane color.
    commands.spawn((
        CameraHomeTarget,
        Mesh3d(meshes.add(Cuboid::new(
            ANCHOR_FRAME_SIZE.x,
            ANCHOR_FRAME_SIZE.y,
            ANCHOR_FRAME_SIZE.z,
        ))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: ANCHOR_FRAME_COLOR,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        Transform::from_translation(demo_center + demo_rotation * ANCHOR_FRAME_LOCAL_OFFSET)
            .with_rotation(demo_rotation),
        NotShadowCaster,
    ));

    let sphere_mesh = meshes.add(Sphere::new(0.025));
    let sphere_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.2, 0.2),
        unlit: true,
        ..default()
    });

    // Title.
    commands.spawn(
        DiegeticText::world("Text Anchors")
            .size(0.16)
            .color(Color::srgb(0.7, 0.8, 1.0))
            .anchor(Anchor::TopCenter)
            .transform(
                Transform::from_translation(demo_center + demo_rotation * Vec3::new(0.0, 1.4, 0.0))
                    .with_rotation(demo_rotation),
            )
            .build(),
    );

    commands.spawn((
        Mesh3d(sphere_mesh.clone()),
        MeshMaterial3d(sphere_material.clone()),
        Transform::from_translation(demo_center + demo_rotation * Vec3::new(-0.60, 1.10, 0.01)),
    ));
    commands.spawn(
        DiegeticText::world(" = Transform translation")
            .size(0.10)
            .color(Color::WHITE)
            .anchor(Anchor::TopLeft)
            .transform(
                Transform::from_translation(
                    demo_center + demo_rotation * Vec3::new(-0.55, 1.15, 0.0),
                )
                .with_rotation(demo_rotation),
            )
            .build(),
    );

    let anchor_demo = [
        (Anchor::TopLeft, "TopLeft", -1.3, 0.5),
        (Anchor::TopCenter, "TopCenter", 0.0, 0.5),
        (Anchor::TopRight, "TopRight", 1.3, 0.5),
        (Anchor::CenterLeft, "CenterLeft", -1.3, -0.2),
        (Anchor::Center, "Center", 0.0, -0.2),
        (Anchor::CenterRight, "CenterRight", 1.3, -0.2),
        (Anchor::BottomLeft, "BottomLeft", -1.3, -0.9),
        (Anchor::BottomCenter, "BottomCenter", 0.0, -0.9),
        (Anchor::BottomRight, "BottomRight", 1.3, -0.9),
    ];

    for (anchor, text, local_x, local_y) in anchor_demo {
        let local_offset = Vec3::new(local_x, local_y, 0.01);
        let world_pos = demo_center + demo_rotation * local_offset;

        // Sphere at the anchor origin.
        commands.spawn((
            Mesh3d(sphere_mesh.clone()),
            MeshMaterial3d(sphere_material.clone()),
            Transform::from_translation(world_pos),
        ));

        // Text with the given anchor.
        commands.spawn((
            DiegeticText::world(text)
                .size(0.125)
                .color(Color::WHITE)
                .anchor(anchor)
                .transform(Transform::from_translation(world_pos).with_rotation(demo_rotation))
                .build(),
            AnchorDemoText {
                position:      world_pos,
                base_rotation: demo_rotation,
            },
        ));
    }
}

/// Spawns the "GROUND PLANE" label flat on the ground plane, shifted toward
/// the home camera.
fn spawn_ground_text(commands: &mut Commands) {
    commands.spawn((
        CameraHomeTarget,
        DiegeticText::world("GROUND PLANE")
            .size(GROUND_TEXT_SIZE)
            .color(Color::srgb(0.9, 0.9, 0.1))
            .transform(
                Transform::from_translation(GROUND_TEXT_TRANSLATION)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            )
            .build(),
    ));
}

// ═════════════════════════════════════════════════════════════════════════════
// ROTATION ANIMATION — supporting (decorative). Driving the anchor demo and
// the labeled cube around X/Y/Z so the viewer can see anchor points stay
// fixed while the text rotates. Not part of the demonstrated `WorldText` API;
// safe to skim.
// ═════════════════════════════════════════════════════════════════════════════

/// Fired when an axis rotation of the anchor demo begins.
#[derive(Event)]
struct RotationBegin {
    axis: Vec3,
}

/// Fired when an axis rotation of the anchor demo completes a full revolution.
#[derive(Event)]
struct RotationEnd {
    axis: Vec3,
}

/// Marker for anchor demo text entities that can rotate around their anchors.
#[derive(Component)]
struct AnchorDemoText {
    /// The world-space position of the anchor point (stays fixed during rotation).
    position:      Vec3,
    /// The base rotation of the demo panel.
    base_rotation: Quat,
}

#[derive(Resource, Default)]
struct AnchorRotation {
    /// Current rotation angle in radians (0..TAU). `None` = not rotating.
    angle:     Option<f32>,
    /// Which local axis to rotate around.
    axis:      Vec3,
    /// Axis a shortcut asked to rotate around, consumed by `rotate_anchor_demo`
    /// on the next frame it is idle.
    requested: Option<Vec3>,
}

/// Marker for the cube entity so the rotation system can find it.
#[derive(Component)]
struct DemoCube;

/// `X` / `Y` / `Z` request a full rotation around that local axis through Fairy
/// Dust's shortcut binding; `rotate_anchor_demo` starts it on the next idle
/// frame. Each fires only when no modifier is held.
fn rotate_x(mut state: ResMut<AnchorRotation>) { state.requested = Some(Vec3::X); }

fn rotate_y(mut state: ResMut<AnchorRotation>) { state.requested = Some(Vec3::Y); }

fn rotate_z(mut state: ResMut<AnchorRotation>) { state.requested = Some(Vec3::Z); }

/// Starts a full rotation around the requested local axis, then drives it each
/// frame. `Anchor` demo texts rotate around their anchor point (red dot stays
/// fixed); the cube rotates around its own center on the same axis. Fires
/// [`RotationBegin`]/[`RotationEnd`] so the title bar chips highlight while it
/// runs. A request that arrives mid-rotation is dropped.
fn rotate_anchor_demo(
    time: Res<Time>,
    mut commands: Commands,
    mut state: ResMut<AnchorRotation>,
    mut texts: Query<(&AnchorDemoText, &mut Transform), Without<DemoCube>>,
    mut cube: Query<&mut Transform, With<DemoCube>>,
    mut cube_base_rotation: Local<Option<Quat>>,
) {
    let requested = state.requested.take();
    if state.angle.is_none()
        && let Some(axis) = requested
    {
        state.angle = Some(0.0);
        state.axis = axis;
        if let Ok(cube_t) = cube.single() {
            *cube_base_rotation = Some(cube_t.rotation);
        }
        commands.trigger(RotationBegin { axis });
    }

    let Some(angle) = state.angle.as_mut() else {
        return;
    };

    *angle = time.delta_secs().mul_add(ROTATION_SPEED, *angle);
    let current_angle = *angle;
    let axis = state.axis;

    if current_angle >= std::f32::consts::TAU {
        for (demo, mut transform) in &mut texts {
            *transform =
                Transform::from_translation(demo.position).with_rotation(demo.base_rotation);
        }
        if let (Ok(mut cube_t), Some(base)) = (cube.single_mut(), *cube_base_rotation) {
            cube_t.rotation = base;
        }
        state.angle = None;
        *cube_base_rotation = None;
        commands.trigger(RotationEnd { axis });
        return;
    }

    let rot = Quat::from_axis_angle(axis, current_angle);

    for (demo, mut transform) in &mut texts {
        transform.rotation = demo.base_rotation * rot;
    }

    if let (Ok(mut cube_t), Some(base)) = (cube.single_mut(), *cube_base_rotation) {
        cube_t.rotation = base * rot;
    }
}
