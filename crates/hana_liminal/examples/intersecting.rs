//! Outlines on overlapping meshes with depth-aware rendering.

use std::f32::consts::PI;

use bevy::color::palettes::css::BLUE;
use bevy::color::palettes::css::GREEN;
use bevy::color::palettes::css::RED;
use bevy::color::palettes::css::SILVER;
use bevy::color::palettes::css::YELLOW;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Cuboid;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::ImagePlugin;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Msaa;
use bevy::prelude::Plane3d;
use bevy::prelude::PluginGroup;
use bevy::prelude::PointLight;
use bevy::prelude::Quat;
use bevy::prelude::ResMut;
use bevy::prelude::Sphere;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::default;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;

// camera
const CAMERA_FOCUS: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const CAMERA_POSITION: Vec3 = Vec3::new(1.5, 1.0, 1.5);

// lighting
const LIGHT_INTENSITY: f32 = 10_000_000.0;
const LIGHT_POSITION: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const LIGHT_RANGE: f32 = 100.0;
const LIGHT_SHADOW_DEPTH_BIAS: f32 = 0.2;

// scene
const CUBE_POSITION: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const CUBE_ROTATION_X: f32 = PI / 5.0;
const CUBE_ROTATION_Y: f32 = PI / 3.0;
const GROUND_SIZE: f32 = 50.0;
const GROUND_SUBDIVISIONS: u32 = 10;
const OUTLINE_WIDTH: f32 = 10.0;
const SPHERE_OUTLINE_INTENSITY: f32 = 10.0;
const SPHERE_POSITION: Vec3 = Vec3::new(-0.5, 1.0, 0.5);

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(ImagePlugin::default_nearest()),
            LagrangePlugin,
            LiminalPlugin,
        ))
        .add_systems(Startup, setup)
        .run();
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
        DepthPrepass,
        Msaa::Off,
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

    // Yellow cube with red outline
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::default())),
        MeshMaterial3d(materials.add(Color::from(YELLOW))),
        Transform::from_translation(CUBE_POSITION).with_rotation(
            Quat::from_rotation_x(CUBE_ROTATION_X) * Quat::from_rotation_y(CUBE_ROTATION_Y),
        ),
        Outline::jump_flood(OUTLINE_WIDTH)
            .with_color(Color::from(RED))
            .build(),
    ));

    // Blue sphere with green outline
    commands.spawn((
        Mesh3d(meshes.add(Sphere::default())),
        MeshMaterial3d(materials.add(Color::from(BLUE))),
        Transform::from_translation(SPHERE_POSITION),
        Outline::jump_flood(OUTLINE_WIDTH)
            .with_color(Color::from(GREEN))
            .with_intensity(SPHERE_OUTLINE_INTENSITY)
            .build(),
    ));
}
