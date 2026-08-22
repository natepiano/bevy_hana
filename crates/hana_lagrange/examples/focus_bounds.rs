//! Demonstrates how to keep the camera focus inside a cuboid by setting the
//! `OrbitCam` pan operation's `RegionLimit`. The Left/Right arrows resize the
//! bounds; the gold marker and label track the clamped focus.
//!
//! Controls:
//!   P      - Pause cube spin
//!   ← / →  - Shrink / grow the focus bounds cuboid

use std::f32::consts::FRAC_PI_2;
use std::f32::consts::PI;

use bevy::color::LinearRgba;
use bevy::prelude::AlphaMode;
use bevy::prelude::Assets;
use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Gizmos;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Name;
use bevy::prelude::PostStartup;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Sphere;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::prelude::error;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::Face;
use fairy_dust::TitleBar;
use fairy_dust::TitleChipActivation;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor as PanelAnchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::PanelBuildError;
use hana_diegetic::Sizing;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_diegetic::default_panel_material;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::RegionLimit;

const EXAMPLE_TITLE: &str = "Focus Bounds";

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .key_light_illuminance(KEY_LIGHT_ILLUMINANCE)
        .with_ground_plane()
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(Transform::from_translation(CUBE_TRANSLATION))
        .insert((Cube, CameraHomeTarget))
        .with_orbit_cam_preset(configure_camera, OrbitCamPreset::blender_like())
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(PAUSE_CONTROL)
                .control(BOUNDS_SMALLER_CONTROL)
                .control(BOUNDS_LARGER_CONTROL),
        )
        .wire_chip_to_activation::<AnimationPause>(PAUSE_CONTROL)
        .with_camera_control_panel()
        .init_resource::<AnimationPause>()
        .init_resource::<FocusBounds>()
        .add_systems(PostStartup, (spawn_story_panels, spawn_focus_marker))
        // `P` and the arrow keys run through Fairy Dust's shortcut binding,
        // which fires each only when no modifier is held.
        .with_shortcut(KeyCode::KeyP, toggle_pause)
        .with_shortcut(KeyCode::ArrowLeft, shrink_focus_bounds)
        .with_shortcut(KeyCode::ArrowRight, grow_focus_bounds)
        .add_systems(
            Update,
            (
                sync_focus_bounds,
                sync_focus_marker,
                rotate_cube,
                draw_focus_bounds_gizmo,
            )
                .chain(),
        )
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// FOCUS BOUNDS — apply, resize, and visualize the OrbitCam focus cuboid.
// ═════════════════════════════════════════════════════════════════════════════
//
// How it works: configure_camera sets the pan operation's RegionLimit (a cuboid
// centered on an origin) onto the OrbitCam at startup. shrink_focus_bounds /
// grow_focus_bounds (bound to ← / → through Fairy Dust's shortcut binding)
// adjust the FocusBounds resource; sync_focus_bounds writes the updated size
// back to every OrbitCam each frame. draw_focus_bounds_gizmo draws the bounds
// cuboid; the FocusMarker entity follows the camera's clamped focus point and
// carries a billboarded world-text label.

const BOUNDS_GIZMO_COLOR: Color = Color::linear_rgb(1.6, 1.6, 1.5);
const BOUNDS_LARGER_CONTROL: &str = "→ Increase";
const BOUNDS_SMALLER_CONTROL: &str = "← Decrease";
const FOCUS_BOUNDS_INITIAL_SIZE: f32 = 1.0;
const FOCUS_BOUNDS_MAX_SIZE: f32 = 3.0;
const FOCUS_BOUNDS_MIN_SIZE: f32 = 0.5;
const FOCUS_BOUNDS_ORIGIN: Vec3 = Vec3::splat(1.0);
const FOCUS_BOUNDS_STEP: f32 = 0.25;
const FOCUS_LABEL_COLOR: Color = Color::linear_rgb(3.5, 2.8, 0.2);
const FOCUS_LABEL_OFFSET: f32 = 0.12;
const FOCUS_LABEL_SIZE: f32 = 0.055;
const FOCUS_LABEL_TEXT: &str = "focus";
const FOCUS_MARKER_COLOR: Color = Color::linear_rgb(3.5, 2.8, 0.2);
const FOCUS_MARKER_NAME: &str = "Camera focus marker";
const FOCUS_MARKER_RADIUS: f32 = 0.035;

#[derive(Component)]
struct Cube;

#[derive(Component)]
struct FocusLabel;

#[derive(Component)]
struct FocusMarker;

#[derive(Resource)]
struct FocusBounds {
    size: f32,
}

impl Default for FocusBounds {
    fn default() -> Self {
        Self {
            size: FOCUS_BOUNDS_INITIAL_SIZE,
        }
    }
}

const fn configure_camera(camera: &mut OrbitCam) {
    apply_focus_bounds(camera, FOCUS_BOUNDS_INITIAL_SIZE);
}

const fn apply_focus_bounds(camera: &mut OrbitCam, size: f32) {
    *camera.pan.limit_mut() = RegionLimit::Cuboid {
        origin: FOCUS_BOUNDS_ORIGIN,
        cuboid: Cuboid::new(size, size, size),
    };
}

fn shrink_focus_bounds(mut bounds: ResMut<FocusBounds>) {
    resize_focus_bounds(&mut bounds, -FOCUS_BOUNDS_STEP);
}

fn grow_focus_bounds(mut bounds: ResMut<FocusBounds>) {
    resize_focus_bounds(&mut bounds, FOCUS_BOUNDS_STEP);
}

fn resize_focus_bounds(bounds: &mut FocusBounds, step: f32) {
    bounds.size = (bounds.size + step).clamp(FOCUS_BOUNDS_MIN_SIZE, FOCUS_BOUNDS_MAX_SIZE);
}

fn sync_focus_bounds(bounds: Res<FocusBounds>, mut cameras: Query<&mut OrbitCam>) {
    if !bounds.is_changed() {
        return;
    }
    for mut camera in &mut cameras {
        apply_focus_bounds(&mut camera, bounds.size);
    }
}

fn spawn_focus_marker(
    mut commands: Commands,
    cameras: Query<&OrbitCam>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };

    commands
        .spawn((
            FocusMarker,
            Name::new(FOCUS_MARKER_NAME),
            Mesh3d(meshes.add(Sphere::new(FOCUS_MARKER_RADIUS))),
            MeshMaterial3d(materials.add(focus_marker_material())),
            Transform::from_translation(camera.pan.current().0),
        ))
        .with_children(|marker| {
            marker.spawn((
                FocusLabel,
                Name::new("Camera focus label"),
                DiegeticText::world(FOCUS_LABEL_TEXT)
                    .size(FOCUS_LABEL_SIZE)
                    .color(FOCUS_LABEL_COLOR)
                    .anchor(PanelAnchor::BottomCenter)
                    .unlit()
                    .transform(Transform::from_translation(Vec3::Y * FOCUS_LABEL_OFFSET))
                    .build(),
            ));
        });
}

fn sync_focus_marker(
    cameras: Query<(&OrbitCam, &GlobalTransform)>,
    mut markers: Query<&mut Transform, (With<FocusMarker>, Without<FocusLabel>)>,
    mut labels: Query<&mut Transform, (With<FocusLabel>, Without<FocusMarker>)>,
) {
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let Ok(mut marker) = markers.single_mut() else {
        return;
    };

    marker.translation = camera.pan.current().0;

    let billboard = camera_transform.rotation();
    let screen_up = billboard * Vec3::Y;
    for mut label in &mut labels {
        label.translation = screen_up * FOCUS_LABEL_OFFSET;
        label.rotation = billboard;
    }
}

fn focus_marker_material() -> StandardMaterial {
    let mut emissive: LinearRgba = FOCUS_MARKER_COLOR.into();
    emissive.red *= 4.0;
    emissive.green *= 4.0;
    emissive.blue *= 4.0;

    StandardMaterial {
        base_color: FOCUS_MARKER_COLOR,
        emissive,
        unlit: true,
        ..default()
    }
}

fn draw_focus_bounds_gizmo(bounds: Res<FocusBounds>, mut gizmos: Gizmos) {
    gizmos.cube(
        Transform::from_translation(FOCUS_BOUNDS_ORIGIN).with_scale(Vec3::splat(bounds.size)),
        BOUNDS_GIZMO_COLOR,
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// CUBE FACE STORY PANELS — diegetic panels narrate the focus bounds API.
// ═════════════════════════════════════════════════════════════════════════════

const FACE_PANEL_COLOR: Color = Color::linear_rgb(0.45, 1.25, 6.0);
const FACE_PANEL_OFFSET: f32 = CUBE_SIZE * 0.5 + 0.006;
const FACE_PANEL_PADDING: f32 = 0.06;
const FACE_PANEL_ROW_GAP: f32 = 0.025;
const FACE_PANEL_SIZE: f32 = CUBE_SIZE * 0.88;
const FACE_PANEL_TEXT_SIZE: f32 = 56.0;
const STORY_PANEL_NAME: &str = "Focus bounds story panel";

const BOUNDS_SHAPE_LINES: &[&str] = &["OrbitCam pan", "limit is a", "RegionLimit cuboid"];
const BOUNDS_ORIGIN_LINES: &[&str] = &["the cuboid origin", "places it in", "world space"];
const PAN_CLAMP_LINES: &[&str] = &["panning clamps", "camera focus", "inside the cuboid"];
const RESIZE_LINES: &[&str] = &["Left and Right", "resize the", "bounds cuboid"];
const TOP_LABEL_LINES: &[&str] = &["Focus", "Bounds"];
const BOTTOM_LABEL_LINES: &[&str] = &["P pauses", "cube spin"];

fn spawn_story_panels(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cubes: Query<Entity, With<Cube>>,
) {
    let Ok(cube) = cubes.single() else {
        return;
    };

    let panel_material = materials.add(transparent_panel_material());
    let text_material = materials.add(emissive_text_material());
    commands.entity(cube).with_children(|parent| {
        spawn_story_panel(
            parent,
            Face::Front,
            BOUNDS_SHAPE_LINES,
            panel_material.clone(),
            text_material.clone(),
        );
        spawn_story_panel(
            parent,
            Face::Right,
            BOUNDS_ORIGIN_LINES,
            panel_material.clone(),
            text_material.clone(),
        );
        spawn_story_panel(
            parent,
            Face::Back,
            PAN_CLAMP_LINES,
            panel_material.clone(),
            text_material.clone(),
        );
        spawn_story_panel(
            parent,
            Face::Left,
            RESIZE_LINES,
            panel_material.clone(),
            text_material.clone(),
        );
        spawn_story_panel(
            parent,
            Face::Top,
            TOP_LABEL_LINES,
            panel_material.clone(),
            text_material.clone(),
        );
        spawn_story_panel(
            parent,
            Face::Bottom,
            BOTTOM_LABEL_LINES,
            panel_material.clone(),
            text_material.clone(),
        );
    });
}

fn spawn_story_panel(
    parent: &mut ChildSpawnerCommands,
    face: Face,
    lines: &'static [&'static str],
    panel_material: Handle<StandardMaterial>,
    text_material: Handle<StandardMaterial>,
) {
    match story_panel(lines, panel_material, text_material) {
        Ok(panel) => {
            parent.spawn((
                Name::new(STORY_PANEL_NAME),
                panel,
                face_panel_transform(face),
            ));
        },
        Err(error) => {
            error!("focus_bounds: failed to build cube face panel: {error}");
        },
    }
}

fn face_panel_transform(face: Face) -> Transform {
    match face {
        Face::Front => Transform::from_xyz(0.0, 0.0, FACE_PANEL_OFFSET),
        Face::Right => Transform::from_xyz(FACE_PANEL_OFFSET, 0.0, 0.0)
            .with_rotation(Quat::from_rotation_y(FRAC_PI_2)),
        Face::Back => Transform::from_xyz(0.0, 0.0, -FACE_PANEL_OFFSET)
            .with_rotation(Quat::from_rotation_y(PI)),
        Face::Left => Transform::from_xyz(-FACE_PANEL_OFFSET, 0.0, 0.0)
            .with_rotation(Quat::from_rotation_y(-FRAC_PI_2)),
        Face::Top => Transform::from_xyz(0.0, FACE_PANEL_OFFSET, 0.0)
            .with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
        Face::Bottom => Transform::from_xyz(0.0, -FACE_PANEL_OFFSET, 0.0)
            .with_rotation(Quat::from_rotation_x(FRAC_PI_2)),
    }
}

fn story_panel(
    lines: &'static [&'static str],
    panel_material: Handle<StandardMaterial>,
    text_material: Handle<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    DiegeticPanel::world()
        .size(FACE_PANEL_SIZE, FACE_PANEL_SIZE)
        .font_unit(Unit::Millimeters)
        .anchor(PanelAnchor::Center)
        .material(panel_material)
        .text_material(text_material)
        .with_tree(build_story_panel_tree(lines))
        .build()
}

fn transparent_panel_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::NONE,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default_panel_material()
    }
}

fn emissive_text_material() -> StandardMaterial {
    let mut emissive: LinearRgba = FACE_PANEL_COLOR.into();
    emissive.red *= 4.0;
    emissive.green *= 4.0;
    emissive.blue *= 4.0;

    StandardMaterial {
        base_color: Color::NONE,
        emissive,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default_panel_material()
    }
}

fn build_story_panel_tree(lines: &'static [&'static str]) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(FACE_PANEL_SIZE))
            .height(Sizing::fixed(FACE_PANEL_SIZE))
            .alignment(AlignX::Center, AlignY::Center)
            .gap(FACE_PANEL_ROW_GAP)
            .padding(Padding::all(FACE_PANEL_PADDING))
            .clip(),
    );

    let text = TextStyle::new(FACE_PANEL_TEXT_SIZE)
        .with_color(FACE_PANEL_COLOR)
        .with_align(TextAlign::Center)
        .with_shadow_mode(GlyphShadowMode::None);
    for line in lines {
        builder.text((*line, text.clone()));
    }

    builder.build()
}

// ═════════════════════════════════════════════════════════════════════════════
// SCENE SCAFFOLDING — camera home pose, lighting, cube placement.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = fairy_dust::example_cube_on_ground(0.1);

const HOME_MARGIN: f32 = 0.5;
const HOME_PITCH: f32 = 0.3;
const HOME_YAW: f32 = 0.0;

const KEY_LIGHT_ILLUMINANCE: f32 = 2_500.0;

// ═════════════════════════════════════════════════════════════════════════════
// CUBE SPIN — decorative cube rotation with a pause toggle.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_SPIN_DEGREES_PER_SECOND: f32 = 8.0;
const PAUSE_CONTROL: &str = "P Pause";

#[derive(Resource, Default)]
struct AnimationPause {
    paused: bool,
}

impl AnimationPause {
    const fn control_activation(&self) -> ControlActivation {
        if self.paused {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    }
}

impl TitleChipActivation for AnimationPause {
    fn activation(&self) -> ControlActivation { self.control_activation() }
}

fn toggle_pause(mut pause: ResMut<AnimationPause>) { pause.paused = !pause.paused; }

fn rotate_cube(
    time: Res<Time>,
    pause: Res<AnimationPause>,
    mut cube_query: Query<&mut Transform, With<Cube>>,
) {
    if pause.paused {
        return;
    }

    if let Ok(mut cube_transform) = cube_query.single_mut() {
        cube_transform.rotate_y(-CUBE_SPIN_DEGREES_PER_SECOND.to_radians() * time.delta_secs());
    }
}
