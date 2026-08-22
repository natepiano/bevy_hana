//! Demonstrates the simplest `OrbitCam` setup.

use bevy::prelude::Color;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::TitleBar;
use hana_lagrange::OrbitCamPreset;

// app / title bar
const EXAMPLE_TITLE: &str = "Basic";

// camera
const CAMERA_TRANSLATION: Vec3 = Vec3::new(0.0, 1.5, 5.0);

// camera home
const HOME_MARGIN: f32 = 0.5;
const HOME_PITCH: f32 = 0.3;
const HOME_YAW: f32 = 0.0;

// cube
const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = fairy_dust::example_cube_on_ground(0.0);

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
        .insert(CameraHomeTarget)
        .with_orbit_cam_preset_bundle(
            |_| {},
            OrbitCamPreset::simple_mouse(),
            Transform::from_translation(CAMERA_TRANSLATION),
        )
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft),
        )
        .with_camera_control_panel()
        .run();
}
