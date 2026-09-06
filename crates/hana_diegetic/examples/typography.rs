//! Typography overlay demo — visualizes font-level metric lines and
//! per-glyph bounding boxes on a `WorldText` entity using the library's
//! built-in `TypographyOverlay` debug component.
//!
//! Requires the `typography_overlay` feature:
//! ```sh
//! cargo run --example typography --features typography_overlay
//! ```

use std::time::Duration;

use bevy::light::NotShadowReceiver;
use bevy::prelude::Add;
use bevy::prelude::Alpha;
use bevy::prelude::AssetServer;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::KeyCode;
use bevy::prelude::On;
use bevy::prelude::ParamSet;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy::prelude::info;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::AntiAlias;
use hana_diegetic::Border;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticTextMut;
use hana_diegetic::El;
use hana_diegetic::Font;
use hana_diegetic::FontId;
use hana_diegetic::FontRegistered;
use hana_diegetic::FontRegistry;
use hana_diegetic::GlyphMetricVisibility;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::OverlayBoundingBox;
use hana_diegetic::Padding;
use hana_diegetic::Pt;
use hana_diegetic::Px;
use hana_diegetic::ShadowCasting;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::TypographyOverlay;
use hana_lagrange::OrbitCamPreset;

const DISPLAY_SIZE: f32 = 0.48;
const DISPLAY_Y: f32 = 0.5;
const DISPLAY_Z: f32 = 2.0;
const COMMENT_SIZE: f32 = 0.15;
const COMMENT_GROUND_LIFT: f32 = 0.005;
/// Front edge of the ground plane (closest to camera).
const GROUND_FRONT_Z: f32 = GROUND_CENTER_Z + GROUND_SIZE * GROUND_DEPTH_SCALE * 0.5;
/// Place the comment halfway between the word and the front of the ground.
const COMMENT_Z: f32 = f32::midpoint(DISPLAY_Z, GROUND_FRONT_Z);
/// Mirrors `hana_diegetic::debug::constants::BBOX_COLOR`, which is `pub(super)`.
const COMMENT_COLOR: Color = Color::srgba(1.0, 1.0, 0.6, 0.7);
const ZOOM_TO_FIT_MARGIN: f32 = 0.05;
const GROUND_SIZE: f32 = 5.4;
const GROUND_DEPTH_SCALE: f32 = 0.7;
const GROUND_CENTER_Z: f32 = GROUND_SIZE * 0.5 * (1.0 - GROUND_DEPTH_SCALE);
const GROUND_COLOR: Color = Color::srgb(0.08, 0.08, 0.08);

const HOME_YAW: f32 = 0.0;
const HOME_PITCH: f32 = 0.055;

const LIGHT_AIM: Vec3 = Vec3::new(0.0, DISPLAY_Y, DISPLAY_Z);
const KEY_LIGHT_POS: Vec3 = Vec3::new(0.0, 5.0, DISPLAY_Z + 12.0);

const HUD_BACKGROUND: Color = Color::srgba(0.02, 0.03, 0.07, 0.80);
const HUD_FRAME_BACKGROUND: Color = Color::srgba(0.01, 0.01, 0.03, 0.95);
const HUD_BORDER_ACCENT: Color = Color::srgba(0.15, 0.7, 0.9, 0.5);
const HUD_BORDER_DIM: Color = Color::srgba(0.1, 0.4, 0.6, 0.3);
const HUD_TITLE_COLOR: Color = Color::srgb(0.9, 0.95, 1.0);
const HUD_ACTIVE_COLOR: Color = Color::srgb(0.3, 1.0, 0.8);
const HUD_INACTIVE_COLOR: Color = Color::srgba(0.6, 0.65, 0.8, 0.85);

const CAM_HELP_WIDTH: Px = Px(280.0);
const CAM_HELP_RADIUS: Px = Px(15.0);
const CAM_HELP_FRAME_PAD: Px = Px(2.0);
const CAM_HELP_BORDER: Px = Px(2.0);
const CAM_HELP_INSET: Px = Px(CAM_HELP_FRAME_PAD.0 + CAM_HELP_BORDER.0);
const CAM_HELP_INNER_RADIUS: Px = Px(CAM_HELP_RADIUS.0 - CAM_HELP_INSET.0);

const CYCLE_HIGHLIGHT_MIN: Duration = Duration::from_millis(500);

const FONTS_PANEL_WIDTH: Px = CAM_HELP_WIDTH;
const FONTS_PANEL_HEIGHT: Px = Px(232.0);
const FONTS_PANEL_GAP: Px = Px(10.0);
const FONTS_PANEL_ROW_HEIGHT: Px = Px(24.0);
const FONTS_PANEL_KEY_WIDTH: Px = Px(18.0);

/// Font key bindings: (digit key label, font family name, `KeyCode`).
/// `JetBrains` Mono is always available; the rest are loaded at runtime.
const FONT_KEYS: &[(&str, &str, KeyCode)] = &[
    ("1", "JetBrains Mono", KeyCode::Digit1),
    ("2", "Noto Sans", KeyCode::Digit2),
    ("3", "EB Garamond", KeyCode::Digit3),
    ("4", "Crimson Text", KeyCode::Digit4),
    ("5", "Liberation Sans", KeyCode::Digit5),
    ("6", "Liberation Serif", KeyCode::Digit6),
    ("7", "Noto Sans CJK SC", KeyCode::Digit7),
];

/// Anti-alias cycle: (chip id, segment label, mode). `A` steps through them.
const AA_MODES: [(&str, &str, AntiAlias); 4] = [
    ("aa-off", "Off", AntiAlias::Off),
    ("aa-anisotropic", "Anisotropic", AntiAlias::Anisotropic),
    ("aa-supersample", "Supersample", AntiAlias::Supersample),
    ("aa-both", "Both", AntiAlias::Both),
];

const fn chip_activation(active: bool) -> ControlActivation {
    if active {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}
const CRIMSON_TEXT_REGULAR_FONT_ASSET_PATH: &str = "fonts/CrimsonText-Regular.ttf";
const EB_GARAMOND_REGULAR_FONT_ASSET_PATH: &str = "fonts/EBGaramond-Regular.ttf";
const LIBERATION_SANS_REGULAR_FONT_ASSET_PATH: &str = "fonts/LiberationSans-Regular.ttf";
const LIBERATION_SERIF_REGULAR_FONT_ASSET_PATH: &str = "fonts/LiberationSerif-Regular.ttf";
const NOTO_SANS_CJK_SC_REGULAR_FONT_ASSET_PATH: &str = "fonts/NotoSansCJKsc-Regular.otf";
const NOTO_SANS_REGULAR_FONT_ASSET_PATH: &str = "fonts/NotoSans-Regular.ttf";

const DISPLAY_WORDS: &[(&str, &str)] = &[
    ("Typography", "accented cap above ascent"),
    ("V", "EB Garamond Test"),
    ("Ångström", "ring accent, umlaut"),
    ("fjord", "f-j ligature candidate, j descender"),
    ("Qüixy", "Q descender, umlaut, y descender"),
    ("Éblouir", "accented É above ascent"),
    ("glyph", "g + y descenders, x-height"),
    ("WAVEFORM", "all caps, wide W/M, kerning (AV)"),
    ("Bézier", "accented é, mixed case"),
    ("Señal", "tilde above lowercase ñ"),
    ("Ïjssel", "diaeresis on cap I, IJ digraph"),
    ("Übergrößen", "Ü above cap, ß eszett"),
    ("Sphinx", "ascender curve, x terminal"),
    ("Jäger", "J descender, ä umlaut, g descender"),
    ("Côté", "circumflex + acute above cap"),
    ("pqbd", "mirror descender/ascender letters"),
    ("Ål", "Å ring accent, l ascender, narrow"),
    ("Grüße", "ü umlaut, ß eszett"),
    ("Twiggy", "T overhang, double g + y descender"),
    ("ÀÇÉÎÕÜ", "six accented caps"),
    ("fly", "f ascender, l ascender, y descender"),
    ("difficult", "ffi ligature (liga)"),
    ("::=>!=", "calt sequences (contextual alternates)"),
    ("Thirsty", "Th + st discretionary ligatures (dlig)"),
    ("AVOW Type", "kerning pairs AV, OW, Ty (kern)"),
    ("漢字", "Chinese Han glyphs"),
    ("かなカナ", "Japanese hiragana + katakana"),
    ("한글", "Korean Hangul syllables"),
];

#[derive(Component)]
struct FontsPanel;

#[derive(Component)]
struct CommentText;

#[derive(Resource, Clone, Copy, PartialEq, Eq)]
struct TypographyFeatureState {
    flags: u8,
}

impl TypographyFeatureState {
    const OVERLAY: u8 = 1 << 0;
    const GLYPH_METRICS: u8 = 1 << 1;
    const FONT_METRICS: u8 = 1 << 2;
    const LABELS: u8 = 1 << 3;
    const SHADOW: u8 = 1 << 4;
    const WORD_VISIBLE: u8 = 1 << 5;
    const GROUND_VISIBLE: u8 = 1 << 6;

    const fn has(self, flag: u8) -> bool { (self.flags & flag) != 0 }

    const fn set(&mut self, flag: u8, value: bool) {
        self.flags = if value {
            self.flags | flag
        } else {
            self.flags & !flag
        };
    }

    const fn toggle(&mut self, flag: u8) { self.flags ^= flag; }

    const fn overlay(self) -> bool { self.has(Self::OVERLAY) }

    const fn glyph_metrics(self) -> bool { self.has(Self::GLYPH_METRICS) }

    const fn font_metrics(self) -> bool { self.has(Self::FONT_METRICS) }

    const fn labels(self) -> bool { self.has(Self::LABELS) }

    const fn shadow(self) -> bool { self.has(Self::SHADOW) }

    const fn word_visible(self) -> bool { self.has(Self::WORD_VISIBLE) }

    const fn ground_visible(self) -> bool { self.has(Self::GROUND_VISIBLE) }
}

impl Default for TypographyFeatureState {
    fn default() -> Self {
        Self {
            flags: Self::OVERLAY
                | Self::GLYPH_METRICS
                | Self::FONT_METRICS
                | Self::LABELS
                | Self::SHADOW
                | Self::WORD_VISIBLE
                | Self::GROUND_VISIBLE,
        }
    }
}

#[derive(Resource, Default, Clone, Copy, PartialEq)]
enum CycleState {
    #[default]
    Idle,
    Cycling {
        started_at:    Duration,
        overlay_ready: bool,
    },
}

/// Marker for the main display text that the overlay toggle targets.
#[derive(Component)]
struct DisplayText;

/// Marker on the translucent ground-plane entity so `G` can toggle its
/// visibility (diagnostic: does the ground's OIT layer feed the grazing
/// breakup).
#[derive(Component)]
struct GroundPlaneToggle;

/// Authored display-word color. `W` toggles the word between this and the same
/// color at alpha 0; alpha-0 glyphs are discarded before `oit_draw`, so they
/// leave the OIT fragment pool entirely — isolating whether the co-rendered
/// word glyphs drive the grazing breakup. Color is excluded from the layout
/// hash, so the overlay (built from glyph extents) does not rebuild.
const WORD_COLOR: Color = Color::srgb(0.9, 0.9, 0.9);

/// Tracks which word in `DISPLAY_WORDS` is currently shown,
/// with a repeat timer for hold-to-cycle.
#[derive(Resource)]
struct WordCycle {
    index: usize,
    timer: Timer,
}

/// Holds the loaded font handles so the font assets are not dropped.
#[derive(Resource, Default)]
struct FontHandles(Vec<Handle<Font>>);

#[derive(Resource)]
struct SelectedFont(usize);

/// A font index a digit shortcut asked for, consumed by `switch_font`.
#[derive(Resource, Default)]
struct RequestedFont(Option<usize>);

#[derive(Resource, Default)]
struct HomeOverlayTarget(Option<Entity>);

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
        .insert(GroundPlaneToggle)
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .unclamped()
        .with_stable_transparency()
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(ZOOM_TO_FIT_MARGIN)
        .with_restore_camera_on_restart()
        .with_title_bar(title_bar())
        .wire_chip_to_state::<TypographyFeatureState, _>("T Overlay", |state| {
            if state.overlay() {
                ControlActivation::Active
            } else {
                ControlActivation::Inactive
            }
        })
        .wire_chip_to_state::<TypographyFeatureState, _>("W Word", |state| {
            chip_activation(state.word_visible())
        })
        .wire_chip_to_state::<TypographyFeatureState, _>("G Ground", |state| {
            chip_activation(state.ground_visible())
        })
        .wire_chip_to_state::<TypographyFeatureState, _>("B Boxes", |state| {
            chip_activation(state.glyph_metrics())
        })
        .wire_chip_to_state::<TypographyFeatureState, _>("M Metrics", |state| {
            chip_activation(state.font_metrics())
        })
        .wire_chip_to_state::<TypographyFeatureState, _>("L Labels", |state| {
            chip_activation(state.labels())
        })
        .wire_chip_to_state::<TypographyFeatureState, _>("S Shadow", |state| {
            chip_activation(state.shadow())
        })
        .wire_chip_to_state::<CycleState, _>("←/→ Cycle Word", |state| match state {
            CycleState::Cycling { .. } => ControlActivation::Active,
            CycleState::Idle => ControlActivation::Inactive,
        })
        .wire_chip_to_state::<AntiAlias, _>(AA_MODES[0].0, |anti_alias| {
            chip_activation(*anti_alias == AA_MODES[0].2)
        })
        .wire_chip_to_state::<AntiAlias, _>(AA_MODES[1].0, |anti_alias| {
            chip_activation(*anti_alias == AA_MODES[1].2)
        })
        .wire_chip_to_state::<AntiAlias, _>(AA_MODES[2].0, |anti_alias| {
            chip_activation(*anti_alias == AA_MODES[2].2)
        })
        .wire_chip_to_state::<AntiAlias, _>(AA_MODES[3].0, |anti_alias| {
            chip_activation(*anti_alias == AA_MODES[3].2)
        })
        .with_camera_control_panel()
        .insert_resource(WordCycle {
            index: 0,
            timer: Timer::from_seconds(0.15, TimerMode::Repeating),
        })
        .insert_resource(SelectedFont(0))
        .init_resource::<FontHandles>()
        .init_resource::<HomeOverlayTarget>()
        .init_resource::<TypographyFeatureState>()
        .init_resource::<CycleState>()
        .init_resource::<RequestedFont>()
        .add_systems(Startup, setup)
        // `switch_font` consumes the digit request; `cycle_word` is a held
        // arrow-key word scrubber (press-to-advance, hold-to-repeat) that the
        // shortcut binding can't express, so it stays a raw per-frame reader.
        .add_systems(Update, (switch_font, cycle_word, tick_cycle_state))
        // T and the font digits run through Fairy Dust's shortcut binding,
        // which fires each only when no modifier is held.
        .with_shortcut(KeyCode::KeyT, toggle_overlay)
        .with_shortcut(KeyCode::KeyA, cycle_anti_alias)
        .with_shortcut(KeyCode::KeyW, toggle_word_text)
        .with_shortcut(KeyCode::KeyG, toggle_ground_plane)
        .with_shortcut(KeyCode::KeyB, toggle_glyph_metrics)
        .with_shortcut(KeyCode::KeyM, toggle_font_metrics)
        .with_shortcut(KeyCode::KeyL, toggle_labels)
        .with_shortcut(KeyCode::KeyS, toggle_overlay_shadow)
        .with_shortcut(KeyCode::Digit1, request_font_1)
        .with_shortcut(KeyCode::Digit2, request_font_2)
        .with_shortcut(KeyCode::Digit3, request_font_3)
        .with_shortcut(KeyCode::Digit4, request_font_4)
        .with_shortcut(KeyCode::Digit5, request_font_5)
        .with_shortcut(KeyCode::Digit6, request_font_6)
        .with_shortcut(KeyCode::Digit7, request_font_7)
        .add_observer(on_font_registered)
        .add_observer(on_overlay_bounds_added)
        .run();
}

fn title_bar() -> TitleBar {
    TitleBar::new()
        .control("T Overlay")
        .control("W Word")
        .control("G Ground")
        .control("B Boxes")
        .control("M Metrics")
        .control("L Labels")
        .control("S Shadow")
        .control("←/→ Cycle Word")
        .control(TitleBarControl::segmented(
            "A",
            AA_MODES.map(|(id, label, _)| TitleBarSegment::new(id, label)),
        ))
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut font_handles: ResMut<FontHandles>,
    font_registry: Res<FontRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    load_fonts(&asset_server, &mut font_handles);

    let (initial_word, initial_comment) = DISPLAY_WORDS[0];

    // Display word with typography overlay. The camera home tracks the hidden
    // overlay bounds entity that is spawned for the rebuilt overlay.
    commands.spawn((
        DisplayText,
        DiegeticText::world(initial_word)
            .size(DISPLAY_SIZE)
            .color(WORD_COLOR)
            .transform(Transform::from_xyz(0.0, DISPLAY_Y, DISPLAY_Z))
            .build(),
        TypographyOverlay::default().with_shadow(ShadowCasting::On),
    ));

    // Comment text — lies flat in the ground plane, in front of the word,
    // reading toward the camera so the overlay never overlaps it.
    commands.spawn((
        CommentText,
        DiegeticText::world(initial_comment)
            .size(COMMENT_SIZE)
            .color(COMMENT_COLOR)
            .transform(Transform {
                translation: Vec3::new(0.0, COMMENT_GROUND_LIFT, COMMENT_Z),
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                ..default()
            })
            .build(),
    ));

    spawn_hud_panels(&mut commands, &font_registry, &mut materials);
}

fn spawn_hud_panels(
    commands: &mut Commands,
    font_registry: &FontRegistry,
    materials: &mut Assets<StandardMaterial>,
) {
    let unlit_material = hana_diegetic::default_panel_material();
    let unlit = StandardMaterial {
        unlit: true,
        ..unlit_material
    };
    let unlit = materials.add(unlit);

    let fonts_panel = DiegeticPanel::screen()
        .size(
            Sizing::fixed(FONTS_PANEL_WIDTH),
            Sizing::fixed(FONTS_PANEL_HEIGHT),
        )
        .anchor(hana_diegetic::Anchor::TopRight)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_fonts_panel(font_registry, 0))
        .build();
    let Ok(fonts_panel) = fonts_panel else {
        error!("failed to build fonts HUD dimensions");
        return;
    };

    commands.spawn((FontsPanel, fonts_panel, Transform::default()));
}

fn load_fonts(asset_server: &AssetServer, font_handles: &mut FontHandles) {
    for path in [
        NOTO_SANS_REGULAR_FONT_ASSET_PATH,
        EB_GARAMOND_REGULAR_FONT_ASSET_PATH,
        CRIMSON_TEXT_REGULAR_FONT_ASSET_PATH,
        LIBERATION_SANS_REGULAR_FONT_ASSET_PATH,
        LIBERATION_SERIF_REGULAR_FONT_ASSET_PATH,
        NOTO_SANS_CJK_SC_REGULAR_FONT_ASSET_PATH,
    ] {
        font_handles.0.push(asset_server.load(path));
    }
}

fn on_overlay_bounds_added(
    trigger: On<Add, OverlayBoundingBox>,
    mut cycle_state: ResMut<CycleState>,
    mut home_target: ResMut<HomeOverlayTarget>,
    marked_home_targets: Query<(), With<CameraHomeTarget>>,
    mut commands: Commands,
) {
    let target = trigger.entity;
    info!("OverlayBoundingBox added: {target:?}");
    if let CycleState::Cycling {
        started_at,
        overlay_ready: false,
    } = *cycle_state
    {
        *cycle_state = CycleState::Cycling {
            started_at,
            overlay_ready: true,
        };
    }
    // The bounds entity is rebuilt on every word/font change, so move the home
    // marker to the newest one each cycle.
    if let Some(previous) = home_target.0.replace(target)
        && previous != target
        && marked_home_targets.contains(previous)
    {
        commands.entity(previous).remove::<CameraHomeTarget>();
    }
    commands.entity(target).insert(CameraHomeTarget);
}

fn tick_cycle_state(time: Res<Time>, mut cycle_state: ResMut<CycleState>) {
    if let CycleState::Cycling {
        started_at,
        overlay_ready: true,
    } = *cycle_state
        && time.elapsed().saturating_sub(started_at) >= CYCLE_HIGHLIGHT_MIN
    {
        *cycle_state = CycleState::Idle;
    }
}

const fn row_color(active: bool) -> Color {
    if active {
        HUD_ACTIVE_COLOR
    } else {
        HUD_INACTIVE_COLOR
    }
}

fn build_font_key_cells(selected_font: usize) -> Vec<ColumnCell<'static>> {
    FONT_KEYS
        .iter()
        .enumerate()
        .map(|(idx, (label, _, _))| {
            ColumnCell::Text(
                label,
                TextStyle::new(Pt(12.0)).with_color(row_color(idx == selected_font)),
            )
        })
        .collect()
}

fn build_font_name_cells(
    font_registry: &FontRegistry,
    selected_font: usize,
) -> Vec<ColumnCell<'static>> {
    FONT_KEYS
        .iter()
        .enumerate()
        .map(|(idx, (_, name, _))| {
            let font_id = font_registry
                .font_id_by_name(name)
                .unwrap_or(FontId::MONOSPACE)
                .0;
            ColumnCell::Text(
                name,
                TextStyle::new(Pt(12.0))
                    .with_font(font_id)
                    .with_color(row_color(idx == selected_font)),
            )
        })
        .collect()
}

fn build_fonts_panel(
    font_registry: &FontRegistry,
    selected_font: usize,
) -> hana_diegetic::LayoutTree {
    let row_height = Sizing::fixed(FONTS_PANEL_ROW_HEIGHT);
    let key_cells = build_font_key_cells(selected_font);
    let name_cells = build_font_name_cells(font_registry, selected_font);

    let mut builder = LayoutBuilder::new(FONTS_PANEL_WIDTH, FONTS_PANEL_HEIGHT);
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .align_x(AlignX::Right),
        |b| {
            b.with(
                El::new()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .padding(Padding::all(CAM_HELP_FRAME_PAD))
                    .corner_radius(CornerRadius::new(
                        CAM_HELP_RADIUS,
                        CAM_HELP_RADIUS,
                        CAM_HELP_RADIUS,
                        CAM_HELP_RADIUS,
                    ))
                    .background(HUD_FRAME_BACKGROUND)
                    .border(Border::all(CAM_HELP_BORDER, HUD_BORDER_ACCENT)),
                |b| {
                    b.with(
                        El::column()
                            .width(Sizing::FIT)
                            .height(Sizing::FIT)
                            .padding(Padding::all(Px(10.0)))
                            .gap(Px(6.0))
                            .corner_radius(CornerRadius::new(
                                CAM_HELP_INNER_RADIUS,
                                CAM_HELP_INNER_RADIUS,
                                CAM_HELP_INNER_RADIUS,
                                CAM_HELP_INNER_RADIUS,
                            ))
                            .background(HUD_BACKGROUND)
                            .border(Border::all(Px(1.0), HUD_BORDER_DIM)),
                        |b| {
                            b.text((
                                "FONTS",
                                TextStyle::new(fairy_dust::TITLE_SIZE).with_color(HUD_TITLE_COLOR),
                            ));
                            b.with(
                                El::row()
                                    .width(Sizing::FIT)
                                    .height(Sizing::FIT)
                                    .gap(FONTS_PANEL_GAP),
                                |b| {
                                    b.with(
                                        El::column()
                                            .width(Sizing::fixed(FONTS_PANEL_KEY_WIDTH))
                                            .height(Sizing::FIT)
                                            .align_x(AlignX::Center),
                                        |b| {
                                            for cell in &key_cells {
                                                let ColumnCell::Text(text, config) = cell;
                                                b.with(
                                                    El::new()
                                                        .width(Sizing::GROW)
                                                        .height(row_height)
                                                        .align_x(AlignX::Center)
                                                        .align_y(AlignY::Center),
                                                    |b| {
                                                        b.text((*text, config.clone()));
                                                    },
                                                );
                                            }
                                        },
                                    );
                                    column(b, AlignX::Left, row_height, &name_cells);
                                },
                            );
                        },
                    );
                },
            );
        },
    );
    builder.build()
}

/// Cell content for a column.
enum ColumnCell<'a> {
    Text(&'a str, TextStyle),
}

/// Builds a column of fixed-height rows.
fn column(b: &mut LayoutBuilder, align: AlignX, row_height: Sizing, cells: &[ColumnCell<'_>]) {
    b.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .align_x(align),
        |b| {
            for cell in cells {
                let ColumnCell::Text(text, config) = cell;
                b.with(
                    El::new()
                        .width(Sizing::FIT)
                        .height(row_height)
                        .align_y(AlignY::Center),
                    |b| {
                        b.text((*text, config.clone()));
                    },
                );
            }
        },
    );
}

fn on_font_registered(
    trigger: On<FontRegistered>,
    panels: Query<Entity, With<FontsPanel>>,
    font_registry: Res<FontRegistry>,
    selected_font: Res<SelectedFont>,
    mut commands: Commands,
) {
    info!(
        "FontRegistered: {} (id: {}, {:?})",
        trigger.name, trigger.id.0, trigger.source
    );
    for entity in &panels {
        info!("Rebuilding fonts panel");
        if let Err(error) =
            commands.set_tree(entity, build_fonts_panel(&font_registry, selected_font.0))
        {
            error!("failed to replace typography panel tree: {error}");
        }
    }
}

fn overlay_from_state(state: TypographyFeatureState) -> TypographyOverlay {
    let glyph_metrics = if state.glyph_metrics() {
        GlyphMetricVisibility::Shown
    } else {
        GlyphMetricVisibility::Hidden
    };
    let font_metrics = if state.font_metrics() {
        GlyphMetricVisibility::Shown
    } else {
        GlyphMetricVisibility::Hidden
    };
    let labels = if state.labels() {
        GlyphMetricVisibility::Shown
    } else {
        GlyphMetricVisibility::Hidden
    };
    let shadow_casting = if state.shadow() {
        ShadowCasting::On
    } else {
        ShadowCasting::Off
    };

    TypographyOverlay {
        glyph_metrics,
        font_metrics,
        labels,
        shadow_casting,
        ..TypographyOverlay::default()
    }
}

/// `B` — toggles the per-glyph bounding boxes and origin dots.
fn toggle_glyph_metrics(
    mut overlays: Query<&mut TypographyOverlay, With<DisplayText>>,
    mut state: ResMut<TypographyFeatureState>,
) {
    state.toggle(TypographyFeatureState::GLYPH_METRICS);
    let visibility = if state.glyph_metrics() {
        GlyphMetricVisibility::Shown
    } else {
        GlyphMetricVisibility::Hidden
    };
    for mut overlay in &mut overlays {
        overlay.glyph_metrics = visibility;
    }
}

/// `M` — toggles the font metric guide lines (ascent/cap/x-height/baseline/...).
fn toggle_font_metrics(
    mut overlays: Query<&mut TypographyOverlay, With<DisplayText>>,
    mut state: ResMut<TypographyFeatureState>,
) {
    state.toggle(TypographyFeatureState::FONT_METRICS);
    let visibility = if state.font_metrics() {
        GlyphMetricVisibility::Shown
    } else {
        GlyphMetricVisibility::Hidden
    };
    for mut overlay in &mut overlays {
        overlay.font_metrics = visibility;
    }
}

/// `L` — toggles the overlay labels.
fn toggle_labels(
    mut overlays: Query<&mut TypographyOverlay, With<DisplayText>>,
    mut state: ResMut<TypographyFeatureState>,
) {
    state.toggle(TypographyFeatureState::LABELS);
    let visibility = if state.labels() {
        GlyphMetricVisibility::Shown
    } else {
        GlyphMetricVisibility::Hidden
    };
    for mut overlay in &mut overlays {
        overlay.labels = visibility;
    }
}

/// `S` — toggles shadows on the ground plane. Switching the ground to a
/// `NotShadowReceiver` clears every shadow cast onto it — the word's and the
/// overlay's — in one control; the overlay's own cast flag is kept in sync so
/// it stops feeding the shadow map while off.
fn toggle_overlay_shadow(
    mut commands: Commands,
    mut overlays: Query<&mut TypographyOverlay, With<DisplayText>>,
    grounds: Query<Entity, With<GroundPlaneToggle>>,
    mut state: ResMut<TypographyFeatureState>,
) {
    state.toggle(TypographyFeatureState::SHADOW);
    let shadow_casting = if state.shadow() {
        ShadowCasting::On
    } else {
        ShadowCasting::Off
    };
    for mut overlay in &mut overlays {
        overlay.shadow_casting = shadow_casting;
    }
    for ground in &grounds {
        if state.shadow() {
            commands.entity(ground).remove::<NotShadowReceiver>();
        } else {
            commands.entity(ground).insert(NotShadowReceiver);
        }
    }
}

/// `W` — toggles the display word between visible and alpha 0, and ties its
/// cast shadow to that visibility (an alpha-0 word still fills the shadow map,
/// so the shadow has to be turned off with the glyphs). The overlay is built
/// from glyph extents, not color, so it is unaffected.
fn toggle_word_text(
    mut display: DiegeticTextMut<DisplayText>,
    mut state: ResMut<TypographyFeatureState>,
) {
    state.toggle(TypographyFeatureState::WORD_VISIBLE);
    let color = if state.word_visible() {
        WORD_COLOR
    } else {
        WORD_COLOR.with_alpha(0.0)
    };
    let shadow_mode = if state.word_visible() {
        GlyphShadowMode::Cast
    } else {
        GlyphShadowMode::None
    };
    display.for_each_style_mut(|style| {
        style.set_color(color);
        style.set_shadow_mode(shadow_mode);
    });
}

/// `G` — toggles the translucent ground plane's visibility.
fn toggle_ground_plane(
    mut grounds: Query<&mut Visibility, With<GroundPlaneToggle>>,
    mut state: ResMut<TypographyFeatureState>,
) {
    state.toggle(TypographyFeatureState::GROUND_VISIBLE);
    let target_visibility = if state.ground_visible() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut grounds {
        *visibility = target_visibility;
    }
}

fn toggle_overlay(
    mut commands: Commands,
    mut state: ResMut<TypographyFeatureState>,
    with_overlay: Query<Entity, (With<DisplayText>, With<TypographyOverlay>)>,
    without_overlay: Query<Entity, (With<DisplayText>, Without<TypographyOverlay>)>,
) {
    if with_overlay.is_empty() {
        for entity in &without_overlay {
            commands.entity(entity).insert(overlay_from_state(*state));
        }
        state.set(TypographyFeatureState::OVERLAY, true);
    } else {
        for entity in &with_overlay {
            commands.entity(entity).remove::<TypographyOverlay>();
        }
        state.set(TypographyFeatureState::OVERLAY, false);
    }
}

/// `A` — steps the global anti-alias mode through [`AA_MODES`].
fn cycle_anti_alias(mut anti_alias: ResMut<AntiAlias>) {
    let current = AA_MODES
        .iter()
        .position(|(_, _, mode)| *mode == *anti_alias)
        .unwrap_or(0);
    *anti_alias = AA_MODES[(current + 1) % AA_MODES.len()].2;
}

fn cycle_word(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut cycle: ResMut<WordCycle>,
    mut cycle_state: ResMut<CycleState>,
    mut labels: ParamSet<(DiegeticTextMut<DisplayText>, DiegeticTextMut<CommentText>)>,
) {
    let forward = keyboard.pressed(KeyCode::ArrowRight);
    let backward = keyboard.pressed(KeyCode::ArrowLeft);
    if !forward && !backward {
        cycle.timer.reset();
        return;
    }

    // Advance immediately on first press, then on timer ticks while held.
    let just =
        keyboard.just_pressed(KeyCode::ArrowRight) || keyboard.just_pressed(KeyCode::ArrowLeft);
    let should_advance = just || cycle.timer.tick(time.delta()).just_finished();
    if !should_advance {
        return;
    }

    let len = DISPLAY_WORDS.len();
    if forward {
        cycle.index = (cycle.index + 1) % len;
    } else {
        cycle.index = (cycle.index + len - 1) % len;
    }
    let (word, comment) = DISPLAY_WORDS[cycle.index];
    labels.p0().set(word);
    labels.p1().set(comment);
    *cycle_state = CycleState::Cycling {
        started_at:    time.elapsed(),
        overlay_ready: false,
    };
}

/// 1..7 request a font through Fairy Dust's shortcut binding; `switch_font`
/// applies the request. Each fires only when no modifier is held.
fn request_font_1(mut requested: ResMut<RequestedFont>) { requested.0 = Some(0); }

fn request_font_2(mut requested: ResMut<RequestedFont>) { requested.0 = Some(1); }

fn request_font_3(mut requested: ResMut<RequestedFont>) { requested.0 = Some(2); }

fn request_font_4(mut requested: ResMut<RequestedFont>) { requested.0 = Some(3); }

fn request_font_5(mut requested: ResMut<RequestedFont>) { requested.0 = Some(4); }

fn request_font_6(mut requested: ResMut<RequestedFont>) { requested.0 = Some(5); }

fn request_font_7(mut requested: ResMut<RequestedFont>) { requested.0 = Some(6); }

fn switch_font(
    mut requested: ResMut<RequestedFont>,
    font_registry: Res<FontRegistry>,
    mut selected_font: ResMut<SelectedFont>,
    panels: Query<Entity, With<FontsPanel>>,
    mut display_text: DiegeticTextMut<DisplayText>,
    mut commands: Commands,
) {
    let Some(idx) = requested.0.take() else {
        return;
    };
    selected_font.0 = idx;
    let name = FONT_KEYS[idx].1;
    let font_id = font_registry
        .font_id_by_name(name)
        .unwrap_or(FontId::MONOSPACE)
        .0;
    display_text.for_each_style_mut(|style| style.set_font_id(font_id));
    for entity in &panels {
        if let Err(error) =
            commands.set_tree(entity, build_fonts_panel(&font_registry, selected_font.0))
        {
            error!("failed to replace typography panel tree: {error}");
        }
    }
}
