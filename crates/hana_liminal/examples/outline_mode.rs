//! Side-by-side comparison of outline methods and overlap modes.

use std::f32::consts::PI;

use bevy::color::palettes::css::BLUE;
use bevy::color::palettes::css::GREEN;
use bevy::color::palettes::css::RED;
use bevy::color::palettes::css::SILVER;
use bevy::color::palettes::css::YELLOW;
use bevy::prelude::Alpha;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::FontSize;
use bevy::prelude::ImagePlugin;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Node;
use bevy::prelude::Plane3d;
use bevy::prelude::PluginGroup;
use bevy::prelude::PointLight;
use bevy::prelude::PositionType;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::Sphere;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Text;
use bevy::prelude::TextColor;
use bevy::prelude::TextFont;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Val;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::WindowPlugin;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;
use hana_clerestory::WindowManagerPlugin;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;
use hana_liminal::OutlineMethod;
use hana_liminal::OverlapMode;

// camera and lighting
const CAMERA_FOCUS: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const CAMERA_POSITION: Vec3 = Vec3::new(2.2, 1.2, 2.2);
const CAMERA_RADIUS: f32 = 2.8;
const CAMERA_SMOOTHNESS: f32 = 0.0;
const LIGHT_INTENSITY: f32 = 10_000_000.0;
const LIGHT_POSITION: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const LIGHT_RANGE: f32 = 100.0;
const LIGHT_SHADOW_DEPTH_BIAS: f32 = 0.2;

// display formatting
const PIXEL_WIDTH_DISPLAY_PRECISION: usize = 1;
const WORLD_WIDTH_DISPLAY_PRECISION: usize = 4;

// environment
const GROUND_SIZE: f32 = 50.0;
const GROUND_SUBDIVISIONS: u32 = 10;

// initial overlap modes
const INITIAL_HULL_OVERLAP: OverlapMode = OverlapMode::Merged;
const INITIAL_SHELL_OVERLAP: OverlapMode = OverlapMode::PerMesh;

// initial widths
const INITIAL_HULL_WIDTH_WORLD: f32 = 0.01;
const INITIAL_JUMP_FLOOD_WIDTH_PX: f32 = 5.0;
const INITIAL_SHELL_WIDTH_PX: f32 = 2.0;

// labels
const MODE_LINE_JUMP_FLOOD: &str = "Mode: JumpFlood (M)";
const MODE_LINE_SCREEN_HULL: &str = "Mode: ScreenHull (M)";
const MODE_LINE_WORLD_HULL: &str = "Mode: WorldHull (M)";
const OVERLAP_LABEL_GROUPED: &str = "Grouped";
const OVERLAP_LABEL_MERGED: &str = "Merged";
const OVERLAP_LABEL_PER_MESH: &str = "PerMesh";

// mesh layout
const INTERSECTING_CUBE_POSITION: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const INTERSECTING_SPHERE_POSITION: Vec3 = Vec3::new(-0.5, 1.0, 0.5);
const NON_INTERSECTING_CUBE_EDGE: f32 = 0.6;
const NON_INTERSECTING_CUBE_POSITION: Vec3 = Vec3::new(0.0, 1.0, -4.0);
const NON_INTERSECTING_SPHERE_POSITION: Vec3 = Vec3::new(-0.75, 1.0, -7.8);
const NON_INTERSECTING_SPHERE_RADIUS: f32 = 0.5;

// outline tuning
const CUBE_ROTATION_X: f32 = PI / 5.0;
const CUBE_ROTATION_Y: f32 = PI / 3.0;
const SECONDARY_OUTLINE_INTENSITY: f32 = 10.0;
const TRANSPARENT_BASE_ALPHA: f32 = 0.5;

// ui
const STATUS_TEXT_FONT_SIZE: f32 = 24.0;
const STATUS_TEXT_PADDING: f32 = 10.0;

// width controls
const JUMP_FLOOD_WIDTH_MIN: f32 = 1.0;
const JUMP_FLOOD_WIDTH_STEP: f32 = 1.0;
const SCREEN_HULL_WIDTH_MIN: f32 = 0.5;
const SCREEN_HULL_WIDTH_STEP: f32 = 0.5;
const WORLD_HULL_WIDTH_MAX: f32 = 10.0;
const WORLD_HULL_WIDTH_MIN: f32 = 0.0001;
const WORLD_HULL_WIDTH_SCALE_FACTOR: f32 = 1.2;

// window
const WINDOW_TITLE: &str = "outline_mode - outline mode comparison";

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: WINDOW_TITLE.into(),
                        ..default()
                    }),
                    ..default()
                }),
            BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
            LagrangePlugin,
            LiminalPlugin,
            WindowManagerPlugin,
        ))
        .init_resource::<OutlineModeToggle>()
        .init_resource::<OutlineWidthControl>()
        .init_resource::<OverlapModes>()
        .add_systems(Startup, (setup, setup_ui))
        .add_systems(
            Update,
            (
                (toggle_outline_mode, adjust_outline_width, adjust_overlap),
                update_ui,
            ),
        )
        .run();
}

#[derive(Resource)]
struct OutlineModeToggle {
    outline_method: OutlineMethod,
}

impl Default for OutlineModeToggle {
    fn default() -> Self {
        Self {
            outline_method: OutlineMethod::WorldHull,
        }
    }
}

#[derive(Resource)]
struct OutlineWidthControl {
    jump_flood_width_px: f32,
    hull_width_world:    f32,
    shell_width_px:      f32,
}

impl Default for OutlineWidthControl {
    fn default() -> Self {
        Self {
            jump_flood_width_px: INITIAL_JUMP_FLOOD_WIDTH_PX,
            hull_width_world:    INITIAL_HULL_WIDTH_WORLD,
            shell_width_px:      INITIAL_SHELL_WIDTH_PX,
        }
    }
}

#[derive(Resource)]
struct OverlapModes {
    world_hull:  OverlapMode,
    screen_hull: OverlapMode,
}

impl Default for OverlapModes {
    fn default() -> Self {
        Self {
            world_hull:  INITIAL_HULL_OVERLAP,
            screen_hull: INITIAL_SHELL_OVERLAP,
        }
    }
}

#[derive(Component)]
struct StatusText;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut orbit_cam = OrbitCam::default();
    orbit_cam.pan.snap_to(CAMERA_FOCUS);
    orbit_cam.orbit.set_damping(CAMERA_SMOOTHNESS);
    orbit_cam.pan.set_damping(CAMERA_SMOOTHNESS);
    orbit_cam.zoom.set_damping(CAMERA_SMOOTHNESS);
    // Place the camera at `CAMERA_RADIUS` along the view ray so `FromTransform`
    // derives the focus, radius, and orbit angles to match.
    let camera_position =
        CAMERA_FOCUS + (CAMERA_POSITION - CAMERA_FOCUS).normalize() * CAMERA_RADIUS;
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(camera_position).looking_at(CAMERA_FOCUS, Vec3::Y),
        orbit_cam,
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        OutlineCamera,
    ));

    commands.spawn((
        PointLight {
            shadow_maps_enabled: true,
            intensity: LIGHT_INTENSITY,
            range: LIGHT_RANGE,
            shadow_depth_bias: LIGHT_SHADOW_DEPTH_BIAS,
            ..default()
        },
        Transform::from_translation(LIGHT_POSITION),
    ));

    // ground plane
    commands.spawn((
        Mesh3d(
            meshes.add(
                Plane3d::default()
                    .mesh()
                    .size(GROUND_SIZE, GROUND_SIZE)
                    .subdivisions(GROUND_SUBDIVISIONS),
            ),
        ),
        MeshMaterial3d(materials.add(Color::from(SILVER))),
    ));

    // Intersecting pair: yellow cube (transparent) and blue sphere
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::default())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::from(YELLOW).with_alpha(TRANSPARENT_BASE_ALPHA),
            alpha_mode: AlphaMode::Blend,
            ..default()
        })),
        Transform::from_translation(INTERSECTING_CUBE_POSITION).with_rotation(
            Quat::from_rotation_x(CUBE_ROTATION_X) * Quat::from_rotation_y(CUBE_ROTATION_Y),
        ),
        Outline::world_hull(INITIAL_HULL_WIDTH_WORLD)
            .with_color(Color::from(RED))
            .with_overlap(INITIAL_HULL_OVERLAP)
            .build(),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Sphere::default())),
        MeshMaterial3d(materials.add(Color::from(BLUE))),
        Transform::from_translation(INTERSECTING_SPHERE_POSITION),
        Outline::world_hull(INITIAL_HULL_WIDTH_WORLD)
            .with_color(Color::from(GREEN))
            .with_intensity(SECONDARY_OUTLINE_INTENSITY)
            .with_overlap(INITIAL_HULL_OVERLAP)
            .build(),
    ));

    // Non-intersecting pair: cube in front of sphere (screen overlap only)
    let non_intersect_cube_mat = materials.add(StandardMaterial {
        base_color: Color::from(YELLOW).with_alpha(TRANSPARENT_BASE_ALPHA),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let non_intersect_sphere_mat = materials.add(Color::from(BLUE));

    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(NON_INTERSECTING_SPHERE_RADIUS))),
        MeshMaterial3d(non_intersect_sphere_mat),
        Transform::from_translation(NON_INTERSECTING_SPHERE_POSITION),
        Outline::world_hull(INITIAL_HULL_WIDTH_WORLD)
            .with_color(Color::from(GREEN))
            .with_intensity(SECONDARY_OUTLINE_INTENSITY)
            .with_overlap(INITIAL_HULL_OVERLAP)
            .build(),
    ));

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_length(NON_INTERSECTING_CUBE_EDGE))),
        MeshMaterial3d(non_intersect_cube_mat),
        Transform::from_translation(NON_INTERSECTING_CUBE_POSITION).with_rotation(
            Quat::from_rotation_x(CUBE_ROTATION_X) * Quat::from_rotation_y(CUBE_ROTATION_Y),
        ),
        Outline::world_hull(INITIAL_HULL_WIDTH_WORLD)
            .with_color(Color::from(RED))
            .with_overlap(INITIAL_HULL_OVERLAP)
            .build(),
    ));
}

fn setup_ui(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(STATUS_TEXT_FONT_SIZE),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(STATUS_TEXT_PADDING),
            left: Val::Px(STATUS_TEXT_PADDING),
            ..default()
        },
        StatusText,
    ));
}

fn toggle_outline_mode(
    input: Res<ButtonInput<KeyCode>>,
    width_control: Res<OutlineWidthControl>,
    overlap_modes: Res<OverlapModes>,
    mut mode_toggle: ResMut<OutlineModeToggle>,
    mut outline_query: Query<&mut Outline>,
) {
    if !input.just_pressed(KeyCode::KeyM) {
        return;
    }

    mode_toggle.outline_method = match mode_toggle.outline_method {
        OutlineMethod::JumpFlood => OutlineMethod::WorldHull,
        OutlineMethod::WorldHull => OutlineMethod::ScreenHull,
        OutlineMethod::ScreenHull => OutlineMethod::JumpFlood,
    };

    let (width, overlap_mode) = match mode_toggle.outline_method {
        OutlineMethod::JumpFlood => (width_control.jump_flood_width_px, OverlapMode::Merged),
        OutlineMethod::WorldHull => (width_control.hull_width_world, overlap_modes.world_hull),
        OutlineMethod::ScreenHull => (width_control.shell_width_px, overlap_modes.screen_hull),
    };

    for mut outline in &mut outline_query {
        *outline =
            rebuilt_outline_for_mode(&outline, mode_toggle.outline_method, width, overlap_mode);
    }
}

const fn rebuilt_outline_for_mode(
    current: &Outline,
    outline_method: OutlineMethod,
    width: f32,
    overlap_mode: OverlapMode,
) -> Outline {
    match outline_method {
        OutlineMethod::JumpFlood => Outline::jump_flood(width)
            .with_intensity(current.intensity)
            .with_color(current.color)
            .build(),
        OutlineMethod::WorldHull => Outline::world_hull(width)
            .with_intensity(current.intensity)
            .with_color(current.color)
            .with_overlap(overlap_mode)
            .build(),
        OutlineMethod::ScreenHull => Outline::screen_hull(width)
            .with_intensity(current.intensity)
            .with_color(current.color)
            .with_overlap(overlap_mode)
            .build(),
    }
}

fn adjust_outline_width(
    input: Res<ButtonInput<KeyCode>>,
    mode_toggle: Res<OutlineModeToggle>,
    mut width_control: ResMut<OutlineWidthControl>,
    mut outline_query: Query<&mut Outline>,
) {
    let decrease = input.just_pressed(KeyCode::ArrowLeft);
    let increase = input.just_pressed(KeyCode::ArrowRight);
    if !decrease && !increase {
        return;
    }

    match mode_toggle.outline_method {
        OutlineMethod::JumpFlood => {
            let mut next = width_control.jump_flood_width_px;
            if decrease {
                next = (next - JUMP_FLOOD_WIDTH_STEP).max(JUMP_FLOOD_WIDTH_MIN);
            }
            if increase {
                next += JUMP_FLOOD_WIDTH_STEP;
            }
            width_control.jump_flood_width_px = next;
            for mut outline in &mut outline_query {
                outline.width = next;
            }
        },
        OutlineMethod::WorldHull => {
            let mut next = width_control.hull_width_world;
            if decrease {
                next /= WORLD_HULL_WIDTH_SCALE_FACTOR;
            }
            if increase {
                next *= WORLD_HULL_WIDTH_SCALE_FACTOR;
            }
            width_control.hull_width_world = next.clamp(WORLD_HULL_WIDTH_MIN, WORLD_HULL_WIDTH_MAX);
            for mut outline in &mut outline_query {
                outline.width = width_control.hull_width_world;
            }
        },
        OutlineMethod::ScreenHull => {
            let mut next = width_control.shell_width_px;
            if decrease {
                next = (next - SCREEN_HULL_WIDTH_STEP).max(SCREEN_HULL_WIDTH_MIN);
            }
            if increase {
                next += SCREEN_HULL_WIDTH_STEP;
            }
            width_control.shell_width_px = next;
            for mut outline in &mut outline_query {
                outline.width = next;
            }
        },
    }
}

fn adjust_overlap(
    input: Res<ButtonInput<KeyCode>>,
    mode_toggle: Res<OutlineModeToggle>,
    mut overlap_modes: ResMut<OverlapModes>,
    mut outline_query: Query<&mut Outline>,
) {
    let decrease = input.just_pressed(KeyCode::Minus);
    let increase = input.just_pressed(KeyCode::Equal);
    if !decrease && !increase {
        return;
    }

    let Some(current) = (match mode_toggle.outline_method {
        OutlineMethod::WorldHull => Some(&mut overlap_modes.world_hull),
        OutlineMethod::ScreenHull => Some(&mut overlap_modes.screen_hull),
        OutlineMethod::JumpFlood => None,
    }) else {
        return;
    };

    *current = match *current {
        OverlapMode::Merged => OverlapMode::PerMesh,
        OverlapMode::PerMesh | OverlapMode::Grouped => OverlapMode::Merged,
    };

    let value = *current;
    for mut outline in &mut outline_query {
        outline.overlap_mode = value;
    }
}

fn update_ui(
    mode_toggle: Res<OutlineModeToggle>,
    width_control: Res<OutlineWidthControl>,
    overlap_modes: Res<OverlapModes>,
    mut text_query: Single<&mut Text, With<StatusText>>,
) {
    let mode_line = match mode_toggle.outline_method {
        OutlineMethod::JumpFlood => MODE_LINE_JUMP_FLOOD,
        OutlineMethod::WorldHull => MODE_LINE_WORLD_HULL,
        OutlineMethod::ScreenHull => MODE_LINE_SCREEN_HULL,
    };

    let width_line = match mode_toggle.outline_method {
        OutlineMethod::JumpFlood => {
            format!(
                "Width: {:.pixel_precision$} px (Left / Right)",
                width_control.jump_flood_width_px,
                pixel_precision = PIXEL_WIDTH_DISPLAY_PRECISION
            )
        },
        OutlineMethod::WorldHull => {
            format!(
                "Width: {:.world_precision$} m (Left / Right)",
                width_control.hull_width_world,
                world_precision = WORLD_WIDTH_DISPLAY_PRECISION
            )
        },
        OutlineMethod::ScreenHull => {
            format!(
                "Width: {:.pixel_precision$} px (Left / Right)",
                width_control.shell_width_px,
                pixel_precision = PIXEL_WIDTH_DISPLAY_PRECISION
            )
        },
    };

    let overlap_line = match mode_toggle.outline_method {
        OutlineMethod::JumpFlood => String::new(),
        OutlineMethod::WorldHull => {
            format!(
                "Overlap: {} (- / +)",
                overlap_mode_label(overlap_modes.world_hull)
            )
        },
        OutlineMethod::ScreenHull => {
            format!(
                "Overlap: {} (- / +)",
                overlap_mode_label(overlap_modes.screen_hull)
            )
        },
    };

    text_query.0 = format!("{mode_line}\n{width_line}\n{overlap_line}");
}

const fn overlap_mode_label(overlap_mode: OverlapMode) -> &'static str {
    match overlap_mode {
        OverlapMode::Merged => OVERLAP_LABEL_MERGED,
        OverlapMode::Grouped => OVERLAP_LABEL_GROUPED,
        OverlapMode::PerMesh => OVERLAP_LABEL_PER_MESH,
    }
}
