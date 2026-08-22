//! Emissive outlines with bloom and oscillating intensity.

use bevy::camera::Hdr;
use bevy::color::palettes::css::BLUE;
use bevy::color::palettes::css::RED;
use bevy::color::palettes::css::SILVER;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::FixedUpdate;
use bevy::prelude::ImagePlugin;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Plane3d;
use bevy::prelude::PluginGroup;
use bevy::prelude::PointLight;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::default;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;

// animation
const ROTATION_X_SPEED: f32 = 1.0;
const ROTATION_Y_SPEED: f32 = 0.5;

// camera
const CAMERA_FOCUS: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const CAMERA_POSITION: Vec3 = Vec3::new(3.0, 2.0, 3.0);

// lighting
const LIGHT_INTENSITY: f32 = 10_000_000.0;
const LIGHT_POSITION: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const LIGHT_RANGE: f32 = 100.0;
const LIGHT_SHADOW_DEPTH_BIAS: f32 = 0.2;

// scene
const GLOW_SINE_REMAP_OFFSET: f32 = 0.5;
const GLOW_SINE_REMAP_SCALE: f32 = 0.5;
const GROUND_SIZE: f32 = 50.0;
const GROUND_SUBDIVISIONS: u32 = 10;
const INITIAL_OUTLINE_WIDTH: f32 = 10.0;
const OUTLINE_GLOW_INTENSITY: f32 = 20.0;
const OUTLINE_GLOW_PERIOD: f32 = 0.2;
const OUTLINED_CUBE_POSITION: Vec3 = Vec3::new(0.0, 1.0, 0.0);

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(ImagePlugin::default_nearest()),
            LagrangePlugin,
            LiminalPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, (rotate, oscillate_intensity))
        .run();
}

#[derive(Component)]
struct OutlineGlow {
    intensity: f32,
    period:    f32,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(CAMERA_POSITION).looking_at(CAMERA_FOCUS, Vec3::Y),
        OrbitCam::default(),
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        OutlineCamera,
        Camera::default(),
        Hdr,
        Bloom::default(),
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

    commands.spawn((
        Mesh3d(meshes.add(Cuboid::default())),
        MeshMaterial3d(materials.add(Color::from(BLUE))),
        Transform::from_translation(OUTLINED_CUBE_POSITION),
        // Add an `Outline` component built from `Outline::jump_flood`.
        Outline::jump_flood(INITIAL_OUTLINE_WIDTH)
            .with_color(Color::from(RED))
            .build(),
        OutlineGlow {
            intensity: OUTLINE_GLOW_INTENSITY,
            period:    OUTLINE_GLOW_PERIOD,
        },
    ));
}

fn rotate(mut outline_query: Query<&mut Transform, With<Outline>>, time: Res<Time>) {
    for mut transform in &mut outline_query {
        let rotation = Quat::from_rotation_y(time.delta_secs() * ROTATION_Y_SPEED)
            * Quat::from_rotation_x(time.delta_secs() * ROTATION_X_SPEED);

        transform.rotation *= rotation;
    }
}

fn oscillate_intensity(
    mut outline_glow_query: Query<(&mut Outline, &OutlineGlow)>,
    time: Res<Time>,
) {
    for (mut outline, glow) in &mut outline_glow_query {
        let t = (time.elapsed_secs() / glow.period)
            .sin()
            .mul_add(GLOW_SINE_REMAP_SCALE, GLOW_SINE_REMAP_OFFSET);

        outline.intensity = glow.intensity * t;
    }
}
