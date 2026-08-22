//! Shadow-receiver repro for the analytic-path `world_position.w` corruption.
//!
//! The batched analytic-path vertex shader stores each quad's record index in
//! `world_position.w` (freeing the UV varyings for material box UVs). Bevy's
//! directional/spot shadow sampling multiplies the *full* `world_position`
//! vec4 by the light's `clip_from_world`, so a non-1.0 `.w` displaces the
//! shadow-map lookup. Because every glyph quad carries a different record
//! index, a directional shadow cast across a text run is scrambled per glyph
//! instead of landing as a coherent band.
//!
//! Scene (viewed top-down): a long flat text line lies on the ground; a thin
//! tall wall stands just behind it so Fairy Dust's key light throws a shadow
//! band across the middle of the text line. The wall is offset in z from the
//! text so it does not occlude the glyphs from the top-down camera.
//!
//! - Bug present: no coherent band — glyphs are individually and randomly darkened/lit where their
//!   displaced lookups land.
//! - Bug fixed: a clean directional shadow band crosses the text line, with lit glyphs on one side
//!   and shadowed glyphs on the other.

use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Cuboid;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Quat;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::default;
use fairy_dust::CameraHomeTarget;
use fairy_dust::TitleBar;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticText;
use hana_lagrange::OrbitCamPreset;

// Camera home pose: near top-down so the flat text and the shadow band read
// clearly.
const HOME_YAW: f32 = 0.0;
const HOME_PITCH: f32 = 1.45;

// Thin tall wall caster, standing on the ground behind the text line (+z) so
// the key light (at (-3.5, 7, 4.8) aiming at the origin) throws its shadow
// toward -z across the glyphs. Offset in z keeps it off the text in screen
// space under the top-down camera.
const WALL_SIZE: Vec3 = Vec3::new(4.0, 1.2, 0.14);
const WALL_TRANSLATION: Vec3 = Vec3::new(1.9, 0.6, 0.95);
const WALL_COLOR: Color = Color::srgb(0.62, 0.62, 0.66);

// Long flat text line on the ground.
const TEXT_SIZE: f32 = 0.5;
const TEXT_TRANSLATION: Vec3 = Vec3::new(-3.4, 0.02, 0.5);
const TEXT_COLOR: Color = Color::srgb(0.93, 0.93, 0.93);

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_stable_transparency()
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(0.4)
        .with_title_bar(
            TitleBar::new()
                .with_title("Shadow Receiver Repro")
                .with_anchor(Anchor::TopLeft)
                .control("flat text receives the wall's directional shadow"),
        )
        .with_camera_control_panel()
        .add_systems(Startup, spawn_scene)
        .run();
}

/// Spawns the caster wall and the long flat text line that receives the key
/// light's directional shadow. Many glyphs means many distinct record indices,
/// so the `world_position.w` corruption scatters the shadow lookup glyph by
/// glyph instead of producing one coherent band.
fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        CameraHomeTarget,
        Mesh3d(meshes.add(Cuboid::new(WALL_SIZE.x, WALL_SIZE.y, WALL_SIZE.z))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: WALL_COLOR,
            ..default()
        })),
        Transform::from_translation(WALL_TRANSLATION),
    ));

    commands.spawn((
        CameraHomeTarget,
        DiegeticText::world("SHADOW RECEIVED ACROSS THIS LINE")
            .size(TEXT_SIZE)
            .color(TEXT_COLOR)
            .anchor(Anchor::CenterLeft)
            .transform(
                Transform::from_translation(TEXT_TRANSLATION)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            )
            .build(),
    ));
}
