//! Slug text rendering example.
//!
//! Renders Latin and CJK world text through the slug glyph backend — hana's
//! sole text renderer. A large headline and a small line show slug coverage
//! across sizes; a CJK row resolved from a runtime-loaded font shows that
//! slug builds quadratic curve bands for CFF cubic outlines too. A
//! [`GlyphRenderMode::PunchOut`] row shows the inverted-coverage fill: each
//! glyph quad is filled everywhere except the letter.

use bevy::anti_alias::smaa::Smaa;
use bevy::prelude::Add;
use bevy::prelude::AssetServer;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::KeyCode;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::TitleBar;
use fairy_dust::TitleChipActivation;
use hana_diegetic::DiegeticText;
use hana_diegetic::Font;
use hana_diegetic::FontRegistered;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphRenderMode;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamPreset;

const HEADLINE_TEXT: &str = "Typography";
const HEADLINE_SIZE: f32 = 0.48;
const HEADLINE_Y: f32 = 0.85;
const SMALL_TEXT: &str = "slug renders glyphs as quadratic Bézier contours";
const SMALL_SIZE: f32 = 0.07;
const SMALL_Y: f32 = 0.18;
const CJK_TEXT: &str = "漢字 かな 한글";
const CJK_SIZE: f32 = 0.34;
const CJK_Y: f32 = 0.52;
const PUNCH_OUT_TEXT: &str = "PunchOut";
const PUNCH_OUT_SIZE: f32 = 0.13;
const PUNCH_OUT_Y: f32 = 0.30;
const PUNCH_OUT_COLOR: Color = Color::srgba(0.45, 0.62, 1.0, 1.0);
const DISPLAY_Z: f32 = 2.0;
const CJK_FONT_ASSET_PATH: &str = "fonts/NotoSansCJKsc-Regular.otf";
const CJK_FONT_FAMILY: &str = "Noto Sans CJK SC";
const LATIN_COLOR: Color = Color::srgba(1.0, 0.38, 0.20, 1.0);
const CJK_COLOR: Color = Color::srgb(0.9, 0.9, 0.9);
const GROUND_SIZE: f32 = 5.4;
const GROUND_DEPTH_SCALE: f32 = 0.7;
const GROUND_CENTER_Z: f32 = GROUND_SIZE * 0.5 * (1.0 - GROUND_DEPTH_SCALE);
const GROUND_COLOR: Color = Color::srgb(0.08, 0.08, 0.08);
const HOME_PITCH: f32 = 0.055;
const HOME_YAW: f32 = 0.0;
const LIGHT_AIM: Vec3 = Vec3::new(0.0, HEADLINE_Y, DISPLAY_Z);
const KEY_LIGHT_POS: Vec3 = Vec3::new(0.0, 5.0, DISPLAY_Z + 12.0);

/// Title-bar control label for the SMAA toggle.
const SMAA_CONTROL: &str = "S SMAA";

/// Keeps the runtime-loaded CJK font handle alive so it stays registered.
#[derive(Resource, Default)]
struct FontHandles(Vec<Handle<Font>>);

/// Source of truth for the post-process SMAA toggle.
#[derive(Resource, Clone, Copy, Default, PartialEq, Eq)]
enum SmaaState {
    /// SMAA on: post-process AA smooths the mesh edges that `Msaa::Off` (forced
    /// by OIT) leaves jagged.
    #[default]
    On,
    /// SMAA off.
    Off,
}

impl TitleChipActivation for SmaaState {
    fn activation(&self) -> ControlActivation {
        match self {
            Self::On => ControlActivation::Active,
            Self::Off => ControlActivation::Inactive,
        }
    }
}

/// Seed SMAA on the orbit camera when it spawns so the example opens with edge
/// anti-aliasing on (matching [`SmaaState`]'s default).
fn seed_smaa(trigger: On<Add, OrbitCam>, mut commands: Commands) {
    commands.entity(trigger.entity).insert(Smaa::default());
}

/// On `S`, toggle [`SmaaState`] and add or remove [`Smaa`] on the scene camera.
fn toggle_smaa(
    mut state: ResMut<SmaaState>,
    cameras: Query<Entity, With<OrbitCam>>,
    mut commands: Commands,
) {
    *state = match *state {
        SmaaState::On => SmaaState::Off,
        SmaaState::Off => SmaaState::On,
    };
    for camera in &cameras {
        match *state {
            SmaaState::On => {
                commands.entity(camera).insert(Smaa::default());
            },
            SmaaState::Off => {
                commands.entity(camera).remove::<Smaa>();
            },
        }
    }
}

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .aim_at(LIGHT_AIM)
        .key_light_pos(KEY_LIGHT_POS)
        .with_ground_plane()
        .size(GROUND_SIZE)
        .transform(
            Transform::from_xyz(0.0, 0.0, GROUND_CENTER_Z).with_scale(Vec3::new(
                1.0,
                1.0,
                GROUND_DEPTH_SCALE,
            )),
        )
        .color(GROUND_COLOR)
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_stable_transparency()
        .with_camera_home()
        .pitch(HOME_PITCH)
        .yaw(HOME_YAW)
        .with_title_bar(TitleBar::new().control(SMAA_CONTROL))
        .wire_chip_to_activation::<SmaaState>(SMAA_CONTROL)
        .with_camera_control_panel()
        .init_resource::<FontHandles>()
        .init_resource::<SmaaState>()
        .add_systems(Startup, setup)
        .add_observer(on_font_registered)
        .add_observer(seed_smaa)
        // `S` toggles SMAA through Fairy Dust's shortcut binding, which fires it
        // only when no modifier is held.
        .with_shortcut(KeyCode::KeyS, toggle_smaa)
        .run();
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut font_handles: ResMut<FontHandles>,
) {
    // The CJK row needs a font with Han/Kana/Hangul coverage; load it at
    // runtime and spawn the row once it registers (see `on_font_registered`).
    font_handles.0.push(asset_server.load(CJK_FONT_ASSET_PATH));

    commands.spawn((
        Name::new("Headline"),
        CameraHomeTarget,
        DiegeticText::world(HEADLINE_TEXT)
            .size(HEADLINE_SIZE)
            .color(LATIN_COLOR)
            .transform(Transform::from_xyz(0.0, HEADLINE_Y, DISPLAY_Z))
            .build(),
    ));
    commands.spawn((
        Name::new("Small line"),
        CameraHomeTarget,
        DiegeticText::world(SMALL_TEXT)
            .size(SMALL_SIZE)
            .color(LATIN_COLOR)
            .transform(Transform::from_xyz(0.0, SMALL_Y, DISPLAY_Z))
            .build(),
    ));
    // PunchOut fill: each glyph quad is painted everywhere except the letter,
    // so the word reads as solid blocks with the letters knocked out.
    commands.spawn((
        Name::new("PunchOut row"),
        CameraHomeTarget,
        DiegeticText::world(PUNCH_OUT_TEXT)
            .size(PUNCH_OUT_SIZE)
            .color(PUNCH_OUT_COLOR)
            .render_mode(GlyphRenderMode::PunchOut)
            .transform(Transform::from_xyz(0.0, PUNCH_OUT_Y, DISPLAY_Z))
            .build(),
    ));
}

/// Spawns the CJK row once its font registers, using the resolved font id so
/// the Han/Kana/Hangul codepoints resolve against a face that has them.
fn on_font_registered(
    trigger: On<FontRegistered>,
    font_registry: Res<FontRegistry>,
    mut commands: Commands,
) {
    if trigger.name != CJK_FONT_FAMILY {
        return;
    }
    let Some(font_id) = font_registry.font_id_by_name(CJK_FONT_FAMILY) else {
        return;
    };
    commands.spawn((
        Name::new("CJK row"),
        CameraHomeTarget,
        DiegeticText::world(CJK_TEXT)
            .size(CJK_SIZE)
            .font(font_id.0)
            .color(CJK_COLOR)
            .transform(Transform::from_xyz(0.0, CJK_Y, DISPLAY_Z))
            .build(),
    ));
}
