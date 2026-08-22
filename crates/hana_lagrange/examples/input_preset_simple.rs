//! Minimal `OrbitCam::simple_mouse` preset example.

use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::OrbitCam;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, LagrangePlugin))
        .add_systems(Startup, spawn_camera)
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Transform::from_xyz(0.0, 1.5, 5.0), OrbitCam::simple_mouse()));
}
