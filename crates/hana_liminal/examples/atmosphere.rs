//! `Outline::jump_flood` and `Outline::screen_hull` drawn by an `OutlineCamera`
//! that also carries `AtmosphereSettings`, so Bevy's physically-based sky and
//! both outline methods render through the same camera.
//!
//! `AtmosphereSettings` adds the atmosphere's LUTs and sampler to the camera's
//! view bind group. The outline passes read their pipeline key from Bevy's
//! `ViewKeyCache`, so they specialize against that same view layout instead of
//! against a key they assemble themselves.
//!
//! The camera also carries `Smaa`, because neither outline method can smooth its
//! own edges here. `Outline::screen_hull` is extruded geometry drawn into the main
//! target, so it antialiases with the camera's `Msaa` — which `with_stable_transparency`
//! turns off. `Outline::jump_flood` gets nothing from `Msaa` at all: its mask pass
//! is single-sampled by construction (see `mask_pipeline.rs`, which forces
//! `MultisampleState::default()` because averaging seed UVs across samples yields a
//! UV pointing at neither seed), so the silhouette it floods from is pixel-center
//! occupancy and the distances it produces are quantized to `sqrt(i^2 + j^2)`. On an
//! axis-aligned edge a width-6 outline covers exactly `d = 1..5` and the next pixel
//! out is rejected at `d = 6`, leaving no fractional coverage for a distance-field
//! ramp to act on. Sub-pixel edges have to be reconstructed from the composited
//! image, which is what `Smaa` does.
//!
//! Controls:
//!   H - Return to the camera home pose

use bevy::anti_alias::smaa::Smaa;
use bevy::light::Atmosphere;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::pbr::AtmosphereSettings;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::ResMut;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DescriptionPanel;
use fairy_dust::Face;
use fairy_dust::TitleBar;
use hana_lagrange::OrbitCamPreset;
use hana_liminal::LiminalPlugin;
use hana_liminal::Outline;
use hana_liminal::OutlineCamera;

// camera home
const HOME_MARGIN: f32 = 0.3;
const HOME_PITCH: f32 = 0.03;
const HOME_YAW: f32 = -0.35;

// cube face labels
const HULL_LABEL: &str = "Screen Hull";
const JUMP_FLOOD_LABEL: &str = "Jump Flood";

// cubes
const CUBE_SIZE: f32 = 1.0;
const CUBE_X_OFFSET: f32 = 1.1;
const CUBE_Y: f32 = CUBE_SIZE / 2.0 + 0.1;
const HULL_TRANSLATION: Vec3 = Vec3::new(CUBE_X_OFFSET, CUBE_Y, 0.0);
const JUMP_FLOOD_TRANSLATION: Vec3 = Vec3::new(-CUBE_X_OFFSET, CUBE_Y, 0.0);

// description panel
const DESCRIPTION_LINES: [&str; 2] = [
    "Both outline methods draw through the same",
    "camera that renders Bevy's sky.",
];
const DESCRIPTION_TITLE: &str = "Outlines against the sky";

// hud
const EXAMPLE_TITLE: &str = "Atmosphere";

// outlines
const HULL_COLOR: Color = Color::srgb(0.15, 0.95, 0.35);
const JUMP_FLOOD_COLOR: Color = Color::srgb(0.95, 0.2, 0.9);
const OUTLINE_WIDTH: f32 = 6.0;

// scattering medium resolution
const PHASE_RESOLUTION: u32 = 256;
const TRANSMITTANCE_RESOLUTION: u32 = 256;

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        // `AtmosphereSettings` requires `Hdr`. Fairy Dust renders diegetic panel
        // content through a chain of cameras, and any camera left in LDR clamps
        // the sky's over-bright colors at that step, which leaves the 3D view
        // black; `with_hdr` sets `Hdr` on every camera in that chain.
        .with_hdr()
        .with_save_window_position()
        .add_plugins(LiminalPlugin)
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .size(CUBE_SIZE)
        .color(fairy_dust::EXAMPLE_CUBE_COLOR)
        .transform(Transform::from_translation(JUMP_FLOOD_TRANSLATION))
        .face_label(Face::Front, JUMP_FLOOD_LABEL)
        .insert((
            CameraHomeTarget,
            Outline::jump_flood(OUTLINE_WIDTH)
                .with_color(JUMP_FLOOD_COLOR)
                .build(),
        ))
        .with_cube()
        .size(CUBE_SIZE)
        .color(fairy_dust::EXAMPLE_CUBE_COLOR)
        .transform(Transform::from_translation(HULL_TRANSLATION))
        .face_label(Face::Front, HULL_LABEL)
        .insert((
            CameraHomeTarget,
            Outline::screen_hull(OUTLINE_WIDTH)
                .with_color(HULL_COLOR)
                .build(),
        ))
        .with_orbit_cam_preset_bundle(
            |_| {},
            OrbitCamPreset::blender_like(),
            (
                OutlineCamera,
                AtmosphereSettings::default(),
                Smaa::default(),
            ),
        )
        // Keeps the coplanar face labels legible. It also forces `Msaa::Off` on
        // the camera, which is why the camera bundle above carries `Smaa`.
        .with_stable_transparency()
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft),
        )
        .with_description_panel(
            DescriptionPanel::new(DESCRIPTION_TITLE)
                .with_fit_width()
                .lines(DESCRIPTION_LINES),
        )
        .with_camera_control_panel()
        .add_systems(Startup, spawn_atmosphere)
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// ATMOSPHERE — spawning the `Atmosphere` planet that the camera's
// `AtmosphereSettings` renders behind both outlined cubes.
// ═════════════════════════════════════════════════════════════════════════════

// How it works: `Atmosphere` describes the planet and lives on its own entity,
// while the camera bundle in `main` carries `AtmosphereSettings`, which selects
// the LUT sizes and contributes the LUT and sampler entries to the camera's view
// bind group. Both components stay in place for the life of the app: Bevy 0.19.1
// leaves stale render state behind when either one is torn down, so the sky is
// set up once at startup and never removed.

fn spawn_atmosphere(
    mut commands: Commands,
    mut scattering_media: ResMut<Assets<ScatteringMedium>>,
) {
    let scattering_medium = scattering_media.add(ScatteringMedium::earth(
        TRANSMITTANCE_RESOLUTION,
        PHASE_RESOLUTION,
    ));
    commands.spawn(Atmosphere::earth(scattering_medium));
}
