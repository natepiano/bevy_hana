//! Unit system demo — real physical sizes.
//!
//! Two panels at their true physical scale in a world where 1 unit = 1 meter:
//!
//! - **Left**: A4 page (210 × 297 mm) with metric rulers (cm/mm ticks).
//! - **Right**: US business card (3½ × 2 inches) with imperial rulers (⅛″ ticks).
//!
//! Font sizes are specified in typographic points. The unit system converts
//! them to each panel's layout unit automatically.
//!
//! Press **D** to toggle debug outlines. Press **R** to toggle rulers.
//! Press **F** to toggle hairline fade; hold **↑** / **↓** to vary the fade
//! exponent.

use std::time::Duration;

use bevy::prelude::Alpha;
use bevy::prelude::Assets;
use bevy::prelude::Click;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::On;
use bevy::prelude::OrthographicProjection;
use bevy::prelude::PerspectiveProjection;
use bevy::prelude::Pointer;
use bevy::prelude::PointerButton;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec4;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy::prelude::error;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::AntiAlias;
use hana_diegetic::Border;
use hana_diegetic::CascadeEntityCommandsExt;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::DiegeticText;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::HairlineFade;
use hana_diegetic::HairlineWidth;
use hana_diegetic::In;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelDraw;
use hana_diegetic::PanelLine;
use hana_diegetic::PanelPoint;
use hana_diegetic::PanelShapeBatchPerfStats;
use hana_diegetic::PaperSize;
use hana_diegetic::Pt;
use hana_diegetic::ShadowCasting;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Unit;
use hana_kana::ToF32;
use hana_kana::ToI32;
use hana_lagrange::AnimationBegin;
use hana_lagrange::AnimationEnd;
use hana_lagrange::AnimationSource;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::Radius;
use hana_lagrange::ZoomToFit;

// ── A4 dimensions ────────────────────────────────────────────────────
const A4: PaperSize = PaperSize::A4;
const A4_WIDTH: Mm = Mm(A4.width_mm());
const A4_HEIGHT: Mm = Mm(A4.height_mm());

// ── US business card dimensions ──────────────────────────────────────
const CARD_WIDTH: In = In(3.5);
const CARD_HEIGHT: In = In(2.0);
const CARD_NAME_SIZE: Pt = Pt(15.0);
const CARD_TITLE_SIZE: Pt = Pt(13.0);
const CARD_DETAIL_SIZE: Pt = Pt(11.0);
const CARD_FOOTER_SIZE: Pt = Pt(6.5);

// ── Index card ──────────────────────────────────────────────────────
const INDEX_WIDTH: In = In(5.0);
const INDEX_HEIGHT: In = In(7.0);
const INDEX_BG: Color = Color::srgba(0.08, 0.10, 0.14, 1.0);
const INDEX_HEADING_COLOR: Color = Color::srgb(0.4, 0.85, 0.75);
const INDEX_LABEL_COLOR: Color = Color::srgba(0.7, 0.75, 0.85, 0.9);
const INDEX_CODE_COLOR: Color = Color::srgb(0.95, 0.9, 0.75);
const INDEX_HEADING_SIZE: Pt = Pt(14.0);
const INDEX_SUBHEADING_SIZE: Pt = Pt(11.0);
const INDEX_LABEL_SIZE: Pt = Pt(10.0);
const INDEX_CODE_SIZE: Pt = Pt(10.0);
const INDEX_FOOTER_SIZE: Pt = Pt(10.0);

// ── Screen panel styling ─────────────────────────────────────────────
/// Inner-background alpha applied to both the title bar and the camera
/// control panel. Higher than `fairy_dust`'s default (0.50) so the example's
/// HUD reads as a more opaque surface against the 3D scene.
const PANEL_BACKGROUND_ALPHA: f32 = 0.90;
const BATCH_PANEL_TITLE_SIZE: Pt = Pt(8.0);
const BATCH_PANEL_VALUE_SIZE: Pt = Pt(10.0);
const BATCH_PANEL_LABEL_SIZE: Pt = Pt(7.0);
const BATCH_PANEL_TITLE_COLOR: Color = Color::srgb(0.70, 0.78, 0.90);
const BATCH_PANEL_VALUE_COLOR: Color = Color::srgb(0.92, 1.00, 0.82);
const BATCH_PANEL_LABEL_COLOR: Color = Color::srgba(0.78, 0.84, 0.92, 0.86);

// ── Scene layout ─────────────────────────────────────────────────────
const GAP: Mm = Mm(15.0);
const LIFT: Mm = Mm(55.0);
const TITLE_GAP: Mm = Mm(8.0);
/// Square edge length of the ground plane in meters. Sized to comfortably
/// extend beyond the three-panel layout (~0.35m × 0.30m) without dominating
/// the scene.
const GROUND_PLANE_SIZE: f32 = 1.0;

// ── Ruler ────────────────────────────────────────────────────────────
const RULER_GAP: Mm = Mm(3.0);
const EDGE_LABEL_EXTRA: In = In(0.5);

// ── Panel ruler — metric (mm units) ─────────────────────────────────
const PANEL_RULER_CM_LINE: Mm = Mm(0.3);
const PANEL_RULER_CM_TICK: Mm = Mm(5.0);
const PANEL_RULER_MM1_LINE: Mm = Mm(0.1);
const PANEL_RULER_MM1_TICK: Mm = Mm(2.0);
const PANEL_RULER_MM5_LINE: Mm = Mm(0.1);
const PANEL_RULER_MM5_TICK: Mm = Mm(3.5);
const PANEL_RULER_SPINE: Mm = Mm(0.2);
const PANEL_RULER_WIDTH: Mm = Mm(10.0);
const PANEL_RULER_MM_LABEL_GAP: Mm = Mm(0.8);

// ── Panel ruler — imperial (inch units) ─────────────────────────────
const PANEL_RULER_16TH_LINE: In = In(0.003);
const PANEL_RULER_16TH_TICK: In = In(0.05);
const PANEL_RULER_8TH_LINE: In = In(0.004);
const PANEL_RULER_8TH_TICK: In = In(0.08);
const PANEL_RULER_HALF_LINE: In = In(0.006);
const PANEL_RULER_HALF_TICK: In = In(0.16);
const PANEL_RULER_INCH_LINE: In = In(0.012);
const PANEL_RULER_INCH_SPINE: In = In(0.008);
const PANEL_RULER_INCH_TICK: In = In(0.2);
const PANEL_RULER_INCH_WIDTH: In = In(0.45);
const PANEL_RULER_QTR_LINE: In = In(0.004);
const PANEL_RULER_QTR_TICK: In = In(0.12);
const PANEL_RULER_IN_LABEL_GAP: In = In(0.0315);

// ── Home / zoom ─────────────────────────────────────────────────────
const HOME_PITCH: f32 = 0.1;
const HOME_YAW: f32 = 0.0;
const HOME_MARGIN: f32 = 0.25;
const ZOOM_DURATION_MS: u64 = 1000;
const ZOOM_MARGIN: f32 = 0.08;

// ── Hairline fade ────────────────────────────────────────────────────
const FADE_EXPONENT_DEFAULT: f32 = 1.0;
const FADE_EXPONENT_MIN: f32 = 0.25;
const FADE_EXPONENT_MAX: f32 = 6.0;
/// Exponent change per second while an arrow key is held.
const FADE_EXPONENT_RATE: f32 = 1.0;

// ── Colors ───────────────────────────────────────────────────────────
const A4_DIM_COLOR: Color = Color::srgba(0.0, 0.0, 0.1, 1.0);
const A4_TEXT_COLOR: Color = Color::BLACK;
const CARD_DIM_COLOR: Color = Color::WHITE;
const CARD_TEXT_COLOR: Color = Color::srgb(1.0, 1.0, 0.85);
/// Debug outline thickness shown as a physical 0.3mm line on every panel.
const DEBUG_OUTLINE: Pt = Pt(1.0);

// ── Marker components ────────────────────────────────────────────────

#[derive(Component)]
struct A4Panel;

#[derive(Component)]
struct CardPanel;

#[derive(Component)]
struct IndexPanel;

#[derive(Component)]
struct PanelRuler;

#[derive(Component)]
struct BatchCountPanel;

#[derive(Component, Clone, Copy, Default, PartialEq)]
struct BatchCountDisplay {
    stats: PanelShapeBatchPerfStats,
    fade:  HairlineFade,
}

/// Last `Fade` exponent, restored when `F` re-enables fade.
#[derive(Resource, Clone, Copy)]
struct FadeExponentMemory(f32);

impl Default for FadeExponentMemory {
    fn default() -> Self { Self(FADE_EXPONENT_DEFAULT) }
}

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq)]
enum DebugOutlines {
    On,
    #[default]
    Off,
}

impl DebugOutlines {
    const fn is_on(self) -> bool { matches!(self, Self::On) }

    const fn toggle(&mut self) {
        *self = match *self {
            Self::On => Self::Off,
            Self::Off => Self::On,
        };
    }
}

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq)]
enum Rulers {
    #[default]
    Visible,
    Hidden,
}

impl Rulers {
    const fn toggle(&mut self) {
        *self = match *self {
            Self::Visible => Self::Hidden,
            Self::Hidden => Self::Visible,
        };
    }
}

#[derive(Resource, Clone, Copy, Default, PartialEq, Eq)]
enum CameraProjection {
    #[default]
    Perspective,
    Orthographic,
}

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

fn build_panel_or_log(
    panel: Result<DiegeticPanel, hana_diegetic::PanelBuildError>,
    label: &str,
) -> Option<DiegeticPanel> {
    match panel {
        Ok(panel) => Some(panel),
        Err(error) => {
            error!("failed to build {label}: {error}");
            None
        },
    }
}

fn spawn_batch_count_panel(commands: &mut Commands, materials: &mut Assets<StandardMaterial>) {
    let display = BatchCountDisplay::default();
    let unlit = materials.add(screen_panel_material());
    let panel = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_batch_count_panel_tree(display))
        .build();

    let Some(panel) = build_panel_or_log(panel, "batch count panel") else {
        return;
    };
    commands.spawn((BatchCountPanel, display, panel, Transform::default()));
}

fn update_batch_count_panel(
    mut commands: Commands,
    perf: Res<DiegeticPerfStats>,
    hairline: Res<HairlineWidth>,
    mut panels: Query<(Entity, &mut BatchCountDisplay), With<BatchCountPanel>>,
) {
    let next = BatchCountDisplay {
        stats: perf.line_batch,
        fade:  hairline.fade,
    };
    for (entity, mut display) in &mut panels {
        if *display == next {
            continue;
        }
        *display = next;
        if let Err(error) = commands.set_tree(entity, build_batch_count_panel_tree(next)) {
            error!("failed to replace batch-count panel tree: {error}");
        }
    }
}

fn fade_line(fade: HairlineFade) -> String {
    match fade {
        HairlineFade::Full => "fade off".to_string(),
        HairlineFade::Fade { exponent } => format!("fade exp {exponent:.2}"),
    }
}

fn build_batch_count_panel_tree(display: BatchCountDisplay) -> LayoutTree {
    let stats = display.stats;
    let title = TextStyle::new(BATCH_PANEL_TITLE_SIZE)
        .with_color(BATCH_PANEL_TITLE_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);
    let value = TextStyle::new(BATCH_PANEL_VALUE_SIZE)
        .with_color(BATCH_PANEL_VALUE_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);
    let label = TextStyle::new(BATCH_PANEL_LABEL_SIZE)
        .with_color(BATCH_PANEL_LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None);
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::FIT,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND.with_alpha(PANEL_BACKGROUND_ALPHA),
        |builder| {
            builder.with(
                El::column().width(Sizing::FIT).height(Sizing::FIT).gap(2.0),
                |builder| {
                    builder.text(("Line Batches", title));
                    builder.text((format!("{}", stats.batches), value));
                    builder.text((format!("records {}", stats.records), label.clone()));
                    builder.text((format!("uploads {}", stats.uploads), label.clone()));
                    builder.text((fade_line(display.fade), label));
                },
            );
        },
    );
    builder.build()
}

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .size(GROUND_PLANE_SIZE)
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .unclamped()
        .with_stable_transparency()
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_background_color(DEFAULT_PANEL_BACKGROUND.with_alpha(PANEL_BACKGROUND_ALPHA))
                .control("D Outlines")
                .control("R Rulers")
                .control("F Fade")
                .control("↑↓ Exponent")
                .control(TitleBarControl::segmented(
                    "A",
                    AA_MODES.map(|(id, label, _)| TitleBarSegment::new(id, label)),
                ))
                .control("P Perspective")
                .control("O Orthographic")
                .control("Click to Zoom"),
        )
        .wire_chip_to_state::<DebugOutlines, _>("D Outlines", |state| match state {
            DebugOutlines::On => ControlActivation::Active,
            DebugOutlines::Off => ControlActivation::Inactive,
        })
        .wire_chip_to_state::<Rulers, _>("R Rulers", |state| match state {
            Rulers::Visible => ControlActivation::Active,
            Rulers::Hidden => ControlActivation::Inactive,
        })
        .wire_chip_to_state::<HairlineWidth, _>("F Fade", |hairline| match hairline.fade {
            HairlineFade::Fade { .. } => ControlActivation::Active,
            HairlineFade::Full => ControlActivation::Inactive,
        })
        // The arrows only act while fade is on, so the chip lights with it.
        .wire_chip_to_state::<HairlineWidth, _>("↑↓ Exponent", |hairline| match hairline.fade {
            HairlineFade::Fade { .. } => ControlActivation::Active,
            HairlineFade::Full => ControlActivation::Inactive,
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
        .wire_chip_to_state::<CameraProjection, _>("P Perspective", |state| match state {
            CameraProjection::Perspective => ControlActivation::Active,
            CameraProjection::Orthographic => ControlActivation::Inactive,
        })
        .wire_chip_to_state::<CameraProjection, _>("O Orthographic", |state| match state {
            CameraProjection::Orthographic => ControlActivation::Active,
            CameraProjection::Perspective => ControlActivation::Inactive,
        })
        .wire_chip_to_events_filtered::<AnimationBegin, AnimationEnd, _, _>(
            "Click to Zoom",
            |event| event.source == AnimationSource::ZoomToFit,
            |event| event.source == AnimationSource::ZoomToFit,
        )
        .with_camera_control_panel()
        .with_camera_control_panel_background_color(
            DEFAULT_PANEL_BACKGROUND.with_alpha(PANEL_BACKGROUND_ALPHA),
        )
        .init_resource::<DebugOutlines>()
        .init_resource::<Rulers>()
        .init_resource::<CameraProjection>()
        .init_resource::<FadeExponentMemory>()
        .insert_resource(HairlineWidth {
            fade: HairlineFade::Fade {
                exponent: FADE_EXPONENT_DEFAULT,
            },
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, update_batch_count_panel)
        // D / R toggles and the P / O projection switch all run through Fairy
        // Dust's shortcut binding, which fires each only when no modifier is held.
        .with_shortcut(KeyCode::KeyD, toggle_debug_outlines)
        .with_shortcut(KeyCode::KeyR, toggle_rulers)
        .with_shortcut(KeyCode::KeyA, cycle_anti_alias)
        .with_shortcut(KeyCode::KeyP, to_perspective_projection)
        .with_shortcut(KeyCode::KeyO, to_orthographic_projection)
        .with_shortcut(KeyCode::KeyF, toggle_hairline_fade)
        .with_held_shortcut(KeyCode::ArrowUp, increase_fade_exponent)
        .with_held_shortcut(KeyCode::ArrowDown, decrease_fade_exponent)
        .run();
}

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    spawn_batch_count_panel(&mut commands, &mut materials);

    let a4_width_meters = f32::from(A4_WIDTH);
    let a4_height_meters = f32::from(A4_HEIGHT);
    let card_width_m = f32::from(CARD_WIDTH);
    let card_height_m = f32::from(CARD_HEIGHT);
    let gap = f32::from(GAP);
    let lift = f32::from(LIFT);
    let title_gap = f32::from(TITLE_GAP);

    let total_width = a4_width_meters + gap + card_width_m;
    let group_left = -total_width / 2.0;

    let a4_page_x = group_left + a4_width_meters / 2.0;
    let a4_page_y = a4_height_meters / 2.0 + lift;

    let a4_page_top = a4_page_y + a4_height_meters / 2.0;
    let card_x = group_left + a4_width_meters + gap + card_width_m / 2.0;
    let card_y = a4_page_top - card_height_m / 2.0;

    let ruler_color = Color::WHITE;

    spawn_a4_with_titles(
        &mut commands,
        a4_page_x,
        a4_page_y,
        a4_page_top,
        card_x,
        title_gap,
    );

    spawn_rulers(
        &mut commands,
        ruler_color,
        a4_page_x,
        a4_page_y,
        a4_width_meters,
        a4_height_meters,
        card_x,
        card_y,
        card_width_m,
        card_height_m,
    );

    spawn_card_panel(&mut commands, card_x, card_y);

    let index_width_m = f32::from(INDEX_WIDTH);
    let index_height_m = f32::from(INDEX_HEIGHT);
    let card_left = card_x - card_width_m / 2.0;
    let index_x = card_left + index_width_m / 2.0;
    let a4_page_bottom = a4_page_y - a4_height_meters / 2.0;
    let index_y = a4_page_bottom + index_height_m / 2.0;

    spawn_photo_panel_with_title(&mut commands, index_x, index_y, index_height_m, title_gap);
    spawn_index_card_rulers(
        &mut commands,
        index_x,
        index_y,
        index_width_m,
        index_height_m,
        ruler_color,
    );
}

fn spawn_a4_with_titles(
    commands: &mut Commands,
    a4_page_x: f32,
    a4_page_y: f32,
    a4_page_top: f32,
    card_x: f32,
    title_gap: f32,
) {
    let Some(a4_panel) = build_panel_or_log(
        DiegeticPanel::world()
            .paper(PaperSize::A4)
            .anchor(Anchor::Center)
            .shadow_casting(ShadowCasting::On)
            .layout(|b| build_a4_content(b, false))
            .build(),
        "A4 dimensions",
    ) else {
        return;
    };

    commands
        .spawn((
            A4Panel,
            CameraHomeTarget,
            a4_panel,
            Transform::from_xyz(a4_page_x, a4_page_y, 0.0),
        ))
        .observe(on_panel_clicked);

    commands
        .spawn(
            DiegeticText::world("A4 Paper — 210 × 297 mm")
                .size(18.0)
                .color(Color::WHITE)
                .anchor(Anchor::BottomCenter)
                .transform(Transform::from_xyz(a4_page_x, a4_page_top + title_gap, 0.0))
                .build(),
        )
        .override_font_unit(Unit::Points);
    commands
        .spawn(
            DiegeticText::world("US Business Card — 3½ × 2 in")
                .size(18.0)
                .color(Color::WHITE)
                .anchor(Anchor::BottomCenter)
                .transform(Transform::from_xyz(card_x, a4_page_top + title_gap, 0.0))
                .build(),
        )
        .override_font_unit(Unit::Points);
}

fn spawn_card_panel(commands: &mut Commands, card_x: f32, card_y: f32) {
    let Some(card_panel) = build_panel_or_log(
        DiegeticPanel::world()
            .paper(PaperSize::BusinessCard)
            .anchor(Anchor::Center)
            .shadow_casting(ShadowCasting::On)
            .layout(|b| build_card_content(b, false))
            .build(),
        "card dimensions",
    ) else {
        return;
    };

    commands
        .spawn((
            CardPanel,
            CameraHomeTarget,
            card_panel,
            Transform::from_xyz(card_x, card_y, 0.0),
        ))
        .observe(on_panel_clicked);
}

fn spawn_photo_panel_with_title(
    commands: &mut Commands,
    index_x: f32,
    index_y: f32,
    index_height_m: f32,
    title_gap: f32,
) {
    let Some(index_panel) = build_panel_or_log(
        DiegeticPanel::world()
            .paper(PaperSize::Photo5x7)
            .anchor(Anchor::Center)
            .shadow_casting(ShadowCasting::On)
            .layout(|b| build_index_content(b, false))
            .build(),
        "index card dimensions",
    ) else {
        return;
    };

    commands
        .spawn((
            IndexPanel,
            CameraHomeTarget,
            index_panel,
            Transform::from_xyz(index_x, index_y, 0.0),
        ))
        .observe(on_panel_clicked);
    commands
        .spawn(
            DiegeticText::world("Photo — 5 × 7 in")
                .size(18.0)
                .color(Color::WHITE)
                .anchor(Anchor::BottomCenter)
                .transform(Transform::from_xyz(
                    index_x,
                    index_y + index_height_m / 2.0 + title_gap,
                    0.0,
                ))
                .build(),
        )
        .override_font_unit(Unit::Points);
}

fn spawn_index_card_rulers(
    commands: &mut Commands,
    index_x: f32,
    index_y: f32,
    index_width_m: f32,
    index_height_m: f32,
    ruler_color: Color,
) {
    // Vertical ruler (right side).
    let index_ruler_x = index_x + index_width_m / 2.0 + f32::from(RULER_GAP);
    let index_sixteenths = (INDEX_HEIGHT.0 * 16.0).round().to_i32();
    let index_ruler_height = In(INDEX_HEIGHT.0 + EDGE_LABEL_EXTRA.0);
    let index_ruler_top = index_y + index_height_m / 2.0 + f32::from(EDGE_LABEL_EXTRA);
    let Some(index_vertical_ruler) = build_panel_or_log(
        DiegeticPanel::world()
            .size(PANEL_RULER_INCH_WIDTH, index_ruler_height)
            .anchor(Anchor::TopLeft)
            .with_tree(build_imperial_panel_ruler(
                index_sixteenths,
                index_ruler_height,
                ruler_color,
            ))
            .build(),
        "index vertical ruler dimensions",
    ) else {
        return;
    };

    commands.spawn((
        PanelRuler,
        index_vertical_ruler,
        Transform::from_xyz(index_ruler_x, index_ruler_top, 0.0),
    ));

    // Horizontal ruler (bottom).
    let index_bottom_ruler_x = index_x - index_width_m / 2.0;
    let index_bottom_ruler_y = index_y - index_height_m / 2.0 - f32::from(RULER_GAP);
    let index_width_sixteenths = (INDEX_WIDTH.0 * 16.0).round().to_i32();
    let Some(index_horizontal_ruler) = build_panel_or_log(
        DiegeticPanel::world()
            .size(INDEX_WIDTH, PANEL_RULER_INCH_WIDTH)
            .anchor(Anchor::TopLeft)
            .with_tree(build_imperial_horizontal_ruler(
                index_width_sixteenths,
                ruler_color,
            ))
            .build(),
        "index horizontal ruler dimensions",
    ) else {
        return;
    };

    commands.spawn((
        PanelRuler,
        index_horizontal_ruler,
        Transform::from_xyz(index_bottom_ruler_x, index_bottom_ruler_y, 0.0),
    ));
}

fn spawn_rulers(
    commands: &mut Commands,
    ruler_color: Color,
    a4_page_x: f32,
    a4_page_y: f32,
    a4_width_meters: f32,
    a4_height_meters: f32,
    card_x: f32,
    card_y: f32,
    card_width_m: f32,
    card_height_m: f32,
) {
    // A4 vertical ruler (left side).
    let a4_ruler_x = a4_page_x - a4_width_meters / 2.0 - f32::from(RULER_GAP);
    let a4_ruler_top = a4_page_y + a4_height_meters / 2.0;
    let Some(a4_vertical_ruler) = build_panel_or_log(
        DiegeticPanel::world()
            .size(PANEL_RULER_WIDTH, A4_HEIGHT)
            .anchor(Anchor::TopRight)
            .with_tree(build_metric_panel_ruler(A4_HEIGHT.0.to_i32(), ruler_color))
            .build(),
        "A4 vertical ruler dimensions",
    ) else {
        return;
    };

    commands.spawn((
        PanelRuler,
        a4_vertical_ruler,
        Transform::from_xyz(a4_ruler_x, a4_ruler_top, 0.0),
    ));

    // A4 horizontal ruler (bottom).
    let a4_bottom_ruler_x = a4_page_x - a4_width_meters / 2.0;
    let a4_bottom_ruler_y = a4_page_y - a4_height_meters / 2.0 - f32::from(RULER_GAP);
    let Some(a4_horizontal_ruler) = build_panel_or_log(
        DiegeticPanel::world()
            .size(A4_WIDTH, PANEL_RULER_WIDTH)
            .anchor(Anchor::TopLeft)
            .with_tree(build_metric_horizontal_ruler(
                A4_WIDTH.0.to_i32(),
                ruler_color,
            ))
            .build(),
        "A4 horizontal ruler dimensions",
    ) else {
        return;
    };

    commands.spawn((
        PanelRuler,
        a4_horizontal_ruler,
        Transform::from_xyz(a4_bottom_ruler_x, a4_bottom_ruler_y, 0.0),
    ));

    // Card vertical ruler (right side).
    let card_ruler_x = card_x + card_width_m / 2.0 + f32::from(RULER_GAP);
    let card_sixteenths = (CARD_HEIGHT.0 * 16.0).round().to_i32();
    let card_ruler_height = In(CARD_HEIGHT.0 + EDGE_LABEL_EXTRA.0);
    let card_ruler_top = card_y + card_height_m / 2.0 + f32::from(EDGE_LABEL_EXTRA);
    let Some(card_vertical_ruler) = build_panel_or_log(
        DiegeticPanel::world()
            .size(PANEL_RULER_INCH_WIDTH, card_ruler_height)
            .anchor(Anchor::TopLeft)
            .with_tree(build_imperial_panel_ruler(
                card_sixteenths,
                card_ruler_height,
                ruler_color,
            ))
            .build(),
        "card vertical ruler dimensions",
    ) else {
        return;
    };

    commands.spawn((
        PanelRuler,
        card_vertical_ruler,
        Transform::from_xyz(card_ruler_x, card_ruler_top, 0.0),
    ));

    // Card horizontal ruler (bottom).
    let card_bottom_ruler_x = card_x - card_width_m / 2.0;
    let card_bottom_ruler_y = card_y - card_height_m / 2.0 - f32::from(RULER_GAP);
    let card_w_sixteenths = (CARD_WIDTH.0 * 16.0).round().to_i32();
    let Some(card_horizontal_ruler) = build_panel_or_log(
        DiegeticPanel::world()
            .size(CARD_WIDTH, PANEL_RULER_INCH_WIDTH)
            .anchor(Anchor::TopLeft)
            .with_tree(build_imperial_horizontal_ruler(
                card_w_sixteenths,
                ruler_color,
            ))
            .build(),
        "card horizontal ruler dimensions",
    ) else {
        return;
    };

    commands.spawn((
        PanelRuler,
        card_horizontal_ruler,
        Transform::from_xyz(card_bottom_ruler_x, card_bottom_ruler_y, 0.0),
    ));
}

const PERSPECTIVE_FOV: f32 = std::f32::consts::FRAC_PI_4;

fn perspective_to_orthographic_radius(r: f32) -> f32 { r * (PERSPECTIVE_FOV / 2.0).tan() * 2.0 }

fn orthographic_to_perspective_radius(r: f32) -> f32 { r / ((PERSPECTIVE_FOV / 2.0).tan() * 2.0) }

/// `P` switches to perspective, `O` to orthographic, through Fairy Dust's
/// shortcut binding. Each fires only when no modifier is held.
fn to_perspective_projection(
    cameras: Query<(&mut Projection, &mut OrbitCam)>,
    projection_state: ResMut<CameraProjection>,
) {
    switch_projection(CameraProjection::Perspective, cameras, projection_state);
}

fn to_orthographic_projection(
    cameras: Query<(&mut Projection, &mut OrbitCam)>,
    projection_state: ResMut<CameraProjection>,
) {
    switch_projection(CameraProjection::Orthographic, cameras, projection_state);
}

/// Switches every camera to `next` (perspective or orthographic), converting
/// orbit radius so the framed scene keeps its apparent size.
fn switch_projection(
    next: CameraProjection,
    mut cameras: Query<(&mut Projection, &mut OrbitCam)>,
    mut projection_state: ResMut<CameraProjection>,
) {
    let to_ortho = next == CameraProjection::Orthographic;
    let to_perspective = next == CameraProjection::Perspective;
    for (mut proj, mut poc) in &mut cameras {
        if to_ortho && matches!(&*proj, Projection::Perspective(_)) {
            let r = poc.zoom.current().0;
            let ortho_r = perspective_to_orthographic_radius(r);
            poc.zoom.snap_to(Radius(ortho_r));
            *proj = Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: 1.0,
                },
                far: 40.0,
                ..OrthographicProjection::default_3d()
            });
            poc.force_update();
            *projection_state = CameraProjection::Orthographic;
        } else if to_perspective && matches!(&*proj, Projection::Orthographic(_)) {
            let r = poc.zoom.current().0;
            let persp_r = orthographic_to_perspective_radius(r);
            poc.zoom.snap_to(Radius(persp_r));
            *proj = Projection::Perspective(PerspectiveProjection {
                near: 0.001,
                near_clip_plane: Vec4::new(0.0, 0.0, -1.0, -0.001),
                fov: PERSPECTIVE_FOV,
                ..default()
            });
            poc.force_update();
            *projection_state = CameraProjection::Perspective;
        }
    }
}

// ── Toggle systems ───────────────────────────────────────────────────

fn toggle_debug_outlines(
    mut debug: ResMut<DebugOutlines>,
    a4_panels: Query<Entity, With<A4Panel>>,
    card_panels: Query<Entity, (With<CardPanel>, Without<A4Panel>, Without<IndexPanel>)>,
    index_panels: Query<Entity, (With<IndexPanel>, Without<A4Panel>, Without<CardPanel>)>,
    mut commands: Commands,
) {
    debug.toggle();
    let on = debug.is_on();
    bevy::log::info!("debug outlines: {on}");

    for entity in &a4_panels {
        if let Err(error) = commands.set_tree(entity, build_a4_page(on)) {
            error!("failed to replace A4 panel tree: {error}");
        }
    }
    for entity in &card_panels {
        if let Err(error) = commands.set_tree(entity, build_card(on)) {
            error!("failed to replace card panel tree: {error}");
        }
    }
    for entity in &index_panels {
        if let Err(error) = commands.set_tree(entity, build_index_page(on)) {
            error!("failed to replace index panel tree: {error}");
        }
    }
}

fn toggle_rulers(
    mut rulers_state: ResMut<Rulers>,
    mut rulers: Query<&mut Visibility, With<PanelRuler>>,
) {
    rulers_state.toggle();
    let vis = match *rulers_state {
        Rulers::Visible => Visibility::Inherited,
        Rulers::Hidden => Visibility::Hidden,
    };
    for mut visibility in &mut rulers {
        *visibility = vis;
    }
}

fn cycle_anti_alias(mut anti_alias: ResMut<AntiAlias>) {
    let current = AA_MODES
        .iter()
        .position(|(_, _, mode)| *mode == *anti_alias)
        .unwrap_or(0);
    *anti_alias = AA_MODES[(current + 1) % AA_MODES.len()].2;
}

/// `F` — toggles hairline fade, restoring the last arrow-tuned exponent.
fn toggle_hairline_fade(
    mut hairline: ResMut<HairlineWidth>,
    mut memory: ResMut<FadeExponentMemory>,
) {
    hairline.fade = match hairline.fade {
        HairlineFade::Full => HairlineFade::Fade { exponent: memory.0 },
        HairlineFade::Fade { exponent } => {
            memory.0 = exponent;
            HairlineFade::Full
        },
    };
}

/// Held `↑` — raises the fade exponent (fades sooner).
fn increase_fade_exponent(hairline: ResMut<HairlineWidth>, time: Res<Time>) {
    adjust_fade_exponent(hairline, time.delta_secs() * FADE_EXPONENT_RATE);
}

/// Held `↓` — lowers the fade exponent (fades later).
fn decrease_fade_exponent(hairline: ResMut<HairlineWidth>, time: Res<Time>) {
    adjust_fade_exponent(hairline, -time.delta_secs() * FADE_EXPONENT_RATE);
}

fn adjust_fade_exponent(mut hairline: ResMut<HairlineWidth>, delta: f32) {
    let HairlineFade::Fade { exponent } = hairline.fade else {
        return;
    };
    let next = (exponent + delta).clamp(FADE_EXPONENT_MIN, FADE_EXPONENT_MAX);
    if next.to_bits() != exponent.to_bits() {
        hairline.fade = HairlineFade::Fade { exponent: next };
    }
}

// ── Panel rulers ────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum TickAlignment {
    Left,
    Right,
}

/// Pins major ticks (one line per mark, in mark order) to
/// [`HairlineFade::Full`] per line so only minor ticks fade with distance.
/// All lines stay in one element: the renderer merges them into a single
/// analytic path, fading each coverage evaluation by its winning curve's
/// exponent, so the exempt and fading lines abut without an anti-aliasing
/// junction.
fn exempt_major_ticks(ticks: Vec<PanelLine>, is_major: impl Fn(i32) -> bool) -> Vec<PanelLine> {
    (0_i32..)
        .zip(ticks)
        .map(|(mark, line)| {
            if is_major(mark) {
                line.hairline_fade(HairlineFade::Full)
            } else {
                line
            }
        })
        .collect()
}

const fn is_metric_major(mm: i32) -> bool { mm % 10 == 0 }

const fn is_imperial_major(sixteenth: i32) -> bool { sixteenth % 16 == 0 }

/// Ruler track element: ticks (majors pinned to `HairlineFade::Full`, minors
/// inheriting the global fade) and the never-fading spine in one draw.
fn ruler_track(b: &mut LayoutBuilder, width: Sizing, height: Sizing, lines: Vec<PanelLine>) {
    b.with(
        El::new()
            .width(width)
            .height(height)
            .draw(PanelDraw::lines(lines)),
        |_| {},
    );
}

fn metric_vertical_tick_lines(height_millimeters: i32, color: Color) -> Vec<PanelLine> {
    vertical_tick_lines(
        height_millimeters,
        1.0,
        PANEL_RULER_CM_TICK.0,
        0.0,
        TickAlignment::Right,
        color,
        millimeter_tick_size,
    )
}

fn metric_vertical_ruler_lines(height_millimeters: i32, color: Color) -> Vec<PanelLine> {
    let ticks = metric_vertical_tick_lines(height_millimeters, color);
    let mut lines = exempt_major_ticks(ticks, is_metric_major);
    lines.push(
        vertical_spine_line(
            height_millimeters.to_f32(),
            PANEL_RULER_CM_TICK.0,
            PANEL_RULER_SPINE.0,
            color,
        )
        .hairline_fade(HairlineFade::Full),
    );
    lines
}

#[cfg(test)]
fn metric_horizontal_tick_lines(width_millimeters: i32, color: Color) -> Vec<PanelLine> {
    horizontal_tick_lines(width_millimeters, 1.0, 0.0, color, millimeter_tick_size)
}

fn metric_horizontal_ruler_lines(width_millimeters: i32, color: Color) -> Vec<PanelLine> {
    let ticks = horizontal_tick_lines(
        width_millimeters,
        1.0,
        PANEL_RULER_SPINE.0,
        color,
        millimeter_tick_size,
    );
    let mut lines = exempt_major_ticks(ticks, is_metric_major);
    lines.push(
        horizontal_spine_line(width_millimeters.to_f32(), 0.0, PANEL_RULER_SPINE.0, color)
            .hairline_fade(HairlineFade::Full),
    );
    lines
}

#[cfg(test)]
fn imperial_vertical_tick_lines(height_sixteenths: i32, color: Color) -> Vec<PanelLine> {
    vertical_tick_lines(
        height_sixteenths,
        1.0 / 16.0,
        PANEL_RULER_INCH_TICK.0,
        0.0,
        TickAlignment::Left,
        color,
        sixteenth_tick_size,
    )
}

fn imperial_vertical_ruler_lines(height_sixteenths: i32, color: Color) -> Vec<PanelLine> {
    let height_inches = height_sixteenths.to_f32() / 16.0;
    let ticks = vertical_tick_lines(
        height_sixteenths,
        1.0 / 16.0,
        PANEL_RULER_INCH_TICK.0,
        PANEL_RULER_INCH_SPINE.0,
        TickAlignment::Left,
        color,
        sixteenth_tick_size,
    );
    let mut lines = exempt_major_ticks(ticks, is_imperial_major);
    lines.push(
        vertical_spine_line(height_inches, 0.0, PANEL_RULER_INCH_SPINE.0, color)
            .hairline_fade(HairlineFade::Full),
    );
    lines
}

#[cfg(test)]
fn imperial_horizontal_tick_lines(width_sixteenths: i32, color: Color) -> Vec<PanelLine> {
    horizontal_tick_lines(
        width_sixteenths,
        1.0 / 16.0,
        0.0,
        color,
        sixteenth_tick_size,
    )
}

fn imperial_horizontal_ruler_lines(width_sixteenths: i32, color: Color) -> Vec<PanelLine> {
    let width_inches = width_sixteenths.to_f32() / 16.0;
    let ticks = horizontal_tick_lines(
        width_sixteenths,
        1.0 / 16.0,
        PANEL_RULER_INCH_SPINE.0,
        color,
        sixteenth_tick_size,
    );
    let mut lines = exempt_major_ticks(ticks, is_imperial_major);
    lines.push(
        horizontal_spine_line(width_inches, 0.0, PANEL_RULER_INCH_SPINE.0, color)
            .hairline_fade(HairlineFade::Full),
    );
    lines
}

fn vertical_tick_lines(
    count: i32,
    slot_height: f32,
    track_width: f32,
    track_x: f32,
    alignment: TickAlignment,
    color: Color,
    tick_size_fn: fn(i32) -> (f32, f32),
) -> Vec<PanelLine> {
    let height = count.to_f32() * slot_height;
    (0..=count)
        .map(|mark| {
            let (tick_length, stroke_width) = tick_size_fn(mark);
            let edge_y = (count - mark).to_f32() * slot_height;
            let y = vertical_tick_center(edge_y, stroke_width);
            let (start_x, end_x) = match alignment {
                TickAlignment::Left => (track_x, track_x + tick_length),
                TickAlignment::Right => {
                    (track_x + track_width - tick_length, track_x + track_width)
                },
            };
            PanelLine::new(
                PanelPoint::new(start_x, y.min(height)),
                PanelPoint::new(end_x, y.min(height)),
            )
            .width(stroke_width)
            .color(color)
        })
        .collect()
}

fn horizontal_tick_lines(
    count: i32,
    slot_width: f32,
    track_y: f32,
    color: Color,
    tick_size_fn: fn(i32) -> (f32, f32),
) -> Vec<PanelLine> {
    let width = count.to_f32() * slot_width;
    (0..=count)
        .map(|mark| {
            let (tick_length, stroke_width) = tick_size_fn(mark);
            let edge_x = mark.to_f32() * slot_width;
            let x = horizontal_tick_center(edge_x, width, stroke_width);
            PanelLine::new(
                PanelPoint::new(x, track_y),
                PanelPoint::new(x, track_y + tick_length),
            )
            .width(stroke_width)
            .color(color)
        })
        .collect()
}

fn vertical_spine_line(height: f32, x: f32, width: f32, color: Color) -> PanelLine {
    let center_x = width.mul_add(0.5, x);
    PanelLine::new(
        PanelPoint::new(center_x, 0.0),
        PanelPoint::new(center_x, height),
    )
    .width(width)
    .color(color)
}

fn horizontal_spine_line(width: f32, y: f32, line_width: f32, color: Color) -> PanelLine {
    let center_y = line_width.mul_add(0.5, y);
    PanelLine::new(
        PanelPoint::new(0.0, center_y),
        PanelPoint::new(width, center_y),
    )
    .width(line_width)
    .color(color)
}

fn vertical_tick_center(edge_y: f32, stroke_width: f32) -> f32 {
    if edge_y <= f32::EPSILON {
        stroke_width * 0.5
    } else {
        stroke_width.mul_add(-0.5, edge_y)
    }
}

fn horizontal_tick_center(edge_x: f32, width: f32, stroke_width: f32) -> f32 {
    if (width - edge_x).abs() <= f32::EPSILON {
        stroke_width.mul_add(-0.5, width)
    } else {
        stroke_width.mul_add(0.5, edge_x)
    }
}

fn build_metric_panel_ruler(height_millimeters: i32, ruler_color: Color) -> LayoutTree {
    let mut builder = LayoutBuilder::new(PANEL_RULER_WIDTH, Mm(height_millimeters.to_f32()));
    let label_style = TextStyle::new(Pt(8.0)).with_color(ruler_color);
    let last_centimeter_mark = height_millimeters / 10;
    // Top spacer: distance from top of ruler to center of topmost cm block.
    let top_spacer = last_centimeter_mark
        .to_f32()
        .mul_add(-10.0, height_millimeters.to_f32())
        - 5.0;

    builder.with(El::row().width(Sizing::GROW).height(Sizing::GROW), |b| {
        // ── Left column: labels ─────────────────────────────
        b.with(
            El::column()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .align_x(AlignX::Right)
                .padding(Padding::new(
                    Mm(0.0),
                    PANEL_RULER_MM_LABEL_GAP,
                    Mm(0.0),
                    Mm(0.0),
                )),
            |b| {
                // Top spacer.
                if top_spacer > 0.0 {
                    b.with(
                        El::new()
                            .height(Sizing::fixed(Mm(top_spacer)))
                            .width(Sizing::GROW),
                        |_| {},
                    );
                }
                // One 10mm block per cm, with text centered.
                for centimeter in (1..=last_centimeter_mark).rev() {
                    b.with(
                        El::new()
                            .height(Sizing::fixed(Mm(10.0)))
                            .width(Sizing::GROW)
                            .align_x(AlignX::Right)
                            .align_y(AlignY::Center),
                        |b| {
                            b.text(
                                Text::new(format!("{centimeter}"), label_style.clone())
                                    .wrap(TextWrap::None),
                            );
                        },
                    );
                }
                // Bottom spacer (5mm below cm 1).
                b.with(
                    El::new().height(Sizing::fixed(Mm(5.0))).width(Sizing::GROW),
                    |_| {},
                );
            },
        );

        // ── Right column: ticks + spine ─────────────────────
        // All lines share one draw and merge into a single analytic
        // path, so tick/spine junctions render without an anti-aliasing
        // line; majors and the spine pin `HairlineFade::Full` per line
        // while minors inherit the global fade.
        ruler_track(
            b,
            Sizing::fixed(Mm(PANEL_RULER_CM_TICK.0 + PANEL_RULER_SPINE.0)),
            Sizing::fixed(Mm(height_millimeters.to_f32())),
            metric_vertical_ruler_lines(height_millimeters, ruler_color),
        );
    });

    builder.build()
}

const fn millimeter_tick_size(mm: i32) -> (f32, f32) {
    if mm % 10 == 0 {
        (PANEL_RULER_CM_TICK.0, PANEL_RULER_CM_LINE.0)
    } else if mm % 5 == 0 {
        (PANEL_RULER_MM5_TICK.0, PANEL_RULER_MM5_LINE.0)
    } else {
        (PANEL_RULER_MM1_TICK.0, PANEL_RULER_MM1_LINE.0)
    }
}

/// Imperial panel ruler — spine on the LEFT, ticks extending RIGHT, labels on RIGHT.
/// `panel_height` may be taller than the measurement range to fit edge labels.
fn build_imperial_panel_ruler(
    height_sixteenths: i32,
    panel_height: In,
    ruler_color: Color,
) -> LayoutTree {
    let height_inches = height_sixteenths.to_f32() / 16.0;
    let last_label_inch = height_sixteenths / 16;
    let top_spacer = panel_height.0 - last_label_inch.to_f32() - 0.5;
    let mut builder = LayoutBuilder::new(PANEL_RULER_INCH_WIDTH, panel_height);
    let label_style = TextStyle::new(Pt(8.0)).with_color(ruler_color);

    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .align_y(AlignY::Bottom),
        |b| {
            // ── Left column: spine + ticks ──────────────────────
            ruler_track(
                b,
                Sizing::fixed(In(PANEL_RULER_INCH_TICK.0 + PANEL_RULER_INCH_SPINE.0)),
                Sizing::fixed(In(height_inches)),
                imperial_vertical_ruler_lines(height_sixteenths, ruler_color),
            );

            // ── Right column: labels ────────────────────────────
            b.with(
                El::column()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .align_x(AlignX::Left)
                    .padding(Padding::new(
                        PANEL_RULER_IN_LABEL_GAP,
                        In(0.0),
                        In(0.0),
                        In(0.0),
                    )),
                |b| {
                    if top_spacer > 0.0 {
                        b.with(
                            El::new()
                                .height(Sizing::fixed(In(top_spacer)))
                                .width(Sizing::GROW),
                            |_| {},
                        );
                    }
                    for inch in (1..=last_label_inch).rev() {
                        b.with(
                            El::new()
                                .height(Sizing::fixed(In(1.0)))
                                .width(Sizing::GROW)
                                .align_x(AlignX::Left)
                                .align_y(AlignY::Center),
                            |b| {
                                b.text(
                                    Text::new(format!("{inch}"), label_style.clone())
                                        .wrap(TextWrap::None),
                                );
                            },
                        );
                    }
                    b.with(
                        El::new().height(Sizing::fixed(In(0.5))).width(Sizing::GROW),
                        |_| {},
                    );
                },
            );
        },
    );

    builder.build()
}

const fn sixteenth_tick_size(sixteenth: i32) -> (f32, f32) {
    if sixteenth % 16 == 0 {
        (PANEL_RULER_INCH_TICK.0, PANEL_RULER_INCH_LINE.0)
    } else if sixteenth % 8 == 0 {
        (PANEL_RULER_HALF_TICK.0, PANEL_RULER_HALF_LINE.0)
    } else if sixteenth % 4 == 0 {
        (PANEL_RULER_QTR_TICK.0, PANEL_RULER_QTR_LINE.0)
    } else if sixteenth % 2 == 0 {
        (PANEL_RULER_8TH_TICK.0, PANEL_RULER_8TH_LINE.0)
    } else {
        (PANEL_RULER_16TH_TICK.0, PANEL_RULER_16TH_LINE.0)
    }
}

/// Horizontal metric ruler — spine on TOP, ticks extending DOWN, labels below.
fn build_metric_horizontal_ruler(width_millimeters: i32, ruler_color: Color) -> LayoutTree {
    let mut builder = LayoutBuilder::new(Mm(width_millimeters.to_f32()), PANEL_RULER_WIDTH);
    let label_style = TextStyle::new(Pt(8.0)).with_color(ruler_color);
    // Labels go at centimeter 1 through the last label slot, each centered in
    // a 10mm block.
    // Skip the cm at the exact edge (it's just a tick, no room for a label).
    let last_label_centimeter = (width_millimeters - 5) / 10;
    let right_spacer = last_label_centimeter
        .to_f32()
        .mul_add(-10.0, width_millimeters.to_f32() - 5.0);

    builder.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        // ── Top row: spine + ticks ──────────────────────────
        ruler_track(
            b,
            Sizing::fixed(Mm(width_millimeters.to_f32())),
            Sizing::fixed(Mm(PANEL_RULER_CM_TICK.0 + PANEL_RULER_SPINE.0)),
            metric_horizontal_ruler_lines(width_millimeters, ruler_color),
        );

        // ── Bottom row: labels ──────────────────────────────
        b.with(
            El::row()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .align_y(AlignY::Top)
                .padding(Padding::new(
                    Mm(0.0),
                    Mm(0.0),
                    PANEL_RULER_MM_LABEL_GAP,
                    Mm(0.0),
                )),
            |b| {
                // Left spacer (5mm to center of first cm block).
                b.with(
                    El::new().width(Sizing::fixed(Mm(5.0))).height(Sizing::GROW),
                    |_| {},
                );
                for centimeter in 1..=last_label_centimeter {
                    b.with(
                        El::new()
                            .width(Sizing::fixed(Mm(10.0)))
                            .height(Sizing::GROW),
                        |b| {
                            b.with(
                                El::column()
                                    .width(Sizing::GROW)
                                    .height(Sizing::GROW)
                                    .align_x(AlignX::Center),
                                |b| {
                                    b.text(
                                        Text::new(format!("{centimeter}"), label_style.clone())
                                            .wrap(TextWrap::None),
                                    );
                                },
                            );
                        },
                    );
                }
                if right_spacer > 0.0 {
                    b.with(
                        El::new()
                            .width(Sizing::fixed(Mm(right_spacer)))
                            .height(Sizing::GROW),
                        |_| {},
                    );
                }
            },
        );
    });

    builder.build()
}

/// Horizontal imperial ruler — spine on TOP, ticks extending DOWN, labels below.
fn build_imperial_horizontal_ruler(width_sixteenths: i32, ruler_color: Color) -> LayoutTree {
    let width_inches = width_sixteenths.to_f32() / 16.0;
    let mut builder = LayoutBuilder::new(In(width_inches), PANEL_RULER_INCH_WIDTH);
    let label_style = TextStyle::new(Pt(8.0)).with_color(ruler_color);
    let last_label_inch = width_sixteenths / 16;
    let right_spacer = width_inches - 0.5 - last_label_inch.to_f32();

    builder.with(El::column().width(Sizing::GROW).height(Sizing::GROW), |b| {
        // ── Top row: spine + ticks ──────────────────────────
        ruler_track(
            b,
            Sizing::fixed(In(width_inches)),
            Sizing::fixed(In(PANEL_RULER_INCH_TICK.0 + PANEL_RULER_INCH_SPINE.0)),
            imperial_horizontal_ruler_lines(width_sixteenths, ruler_color),
        );

        // ── Bottom row: labels ──────────────────────────────
        b.with(
            El::row()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .align_y(AlignY::Top)
                .padding(Padding::new(
                    In(0.0),
                    In(0.0),
                    PANEL_RULER_IN_LABEL_GAP,
                    In(0.0),
                )),
            |b| {
                b.with(
                    El::new().width(Sizing::fixed(In(0.5))).height(Sizing::GROW),
                    |_| {},
                );
                for inch in 1..=last_label_inch {
                    b.with(
                        El::new().width(Sizing::fixed(In(1.0))).height(Sizing::GROW),
                        |b| {
                            b.with(
                                El::column()
                                    .width(Sizing::GROW)
                                    .height(Sizing::GROW)
                                    .align_x(AlignX::Center),
                                |b| {
                                    b.text(
                                        Text::new(format!("{inch}"), label_style.clone())
                                            .wrap(TextWrap::None),
                                    );
                                },
                            );
                        },
                    );
                }
                if right_spacer > 0.0 {
                    b.with(
                        El::new()
                            .width(Sizing::fixed(In(right_spacer)))
                            .height(Sizing::GROW),
                        |_| {},
                    );
                }
            },
        );
    });

    builder.build()
}

// ── Panel content ────────────────────────────────────────────────────

const DEBUG_BORDER_COLOR: Color = Color::srgba(1.0, 0.2, 0.2, 0.8);

fn debug_border(debug: bool, width: impl Into<hana_diegetic::Dimension>) -> Border {
    let color = if debug {
        DEBUG_BORDER_COLOR
    } else {
        Color::NONE
    };
    Border::all(width, color)
}

fn debug_text(b: &mut hana_diegetic::LayoutBuilder, text: &str, style: TextStyle, db: Border) {
    b.with(El::new().width(Sizing::GROW).border(db), |b| {
        b.text((text, style));
    });
}

/// Builds an A4 page layout tree (used by `toggle_debug_outlines` for runtime rebuild).
fn build_a4_page(debug: bool) -> hana_diegetic::LayoutTree {
    let mut builder = LayoutBuilder::new(A4_WIDTH, A4_HEIGHT);
    build_a4_content(&mut builder, debug);
    builder.build()
}

/// Populates an A4 page layout into the given builder.
fn build_a4_content(builder: &mut LayoutBuilder, debug: bool) {
    let db = debug_border(debug, DEBUG_OUTLINE);

    let heading = TextStyle::new(Pt(18.0)).with_color(A4_TEXT_COLOR);
    let body = TextStyle::new(Pt(12.0)).with_color(A4_TEXT_COLOR);

    builder.with(
        El::column()
            .size(A4_WIDTH, A4_HEIGHT)
            .padding(Padding::all(Mm(15.0)))
            .gap(Mm(4.0))
            .background(Color::WHITE),
        |b| {
            build_font_samples_row(b, db);

            // ── Divider ─────────────────────────────────────────
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(Mm(0.3)))
                    .background(A4_DIM_COLOR),
                |_| {},
            );

            build_two_column_article(b, &heading, &body, db);

            // ── Divider ─────────────────────────────────────────
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(Mm(0.3)))
                    .background(A4_DIM_COLOR),
                |_| {},
            );

            // ── Footer ──────────────────────────────────────────
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .align_x(AlignX::Center)
                    .border(db),
                |b| {
                    b.with(El::new().border(db), |b| {
                        b.text((
                            "PaperSize::A4  |  layout: Millimeters  |  font: Points",
                            TextStyle::new(Pt(14.0)).with_color(A4_DIM_COLOR),
                        ));
                    });
                },
            );
        },
    );
}

fn build_font_samples_row(b: &mut LayoutBuilder, db: Border) {
    b.with(
        El::row()
            .width(Sizing::GROW)
            .gap(Mm(6.0))
            .align_y(AlignY::Bottom)
            .border(db),
        |b| {
            for (label, size) in [
                ("72pt", Pt(72.0)),
                ("36pt", Pt(36.0)),
                ("24pt", Pt(24.0)),
                ("18pt", Pt(18.0)),
                ("12pt", Pt(12.0)),
                ("9pt", Pt(9.0)),
            ] {
                debug_text(b, label, TextStyle::new(size).with_color(A4_TEXT_COLOR), db);
            }
        },
    );
}

fn build_two_column_article(
    b: &mut LayoutBuilder,
    heading: &TextStyle,
    body: &TextStyle,
    db: Border,
) {
    b.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(Mm(8.0))
            .border(db),
        |b| {
            b.with(
                El::column().width(Sizing::GROW).gap(Mm(4.0)).border(db),
                |b| {
                    debug_text(b, "Real Units, Real Sizes", heading.clone(), db);
                    debug_text(
                        b,
                        "This page is 210 by 297 millimeters \u{2014} an A4 sheet \
                         at true physical scale, in a game world where one unit equals \
                         one meter. The business card beside it is 3.5 by 2 inches. \
                         Neither requires manual conversion. Each panel declares its \
                         own layout unit, and the system handles the rest.",
                        body.clone(),
                        db,
                    );
                    debug_text(
                        b,
                        "Font sizes are in typographic points. Padding and spacing \
                         follow the panel\u{2019}s layout unit. Mix freely: Mm(6.0) \
                         for a margin, Pt(18.0) for a heading, In(0.5) for a \
                         gutter. The convenience types carry their unit through the \
                         layout engine \u{2014} no math required.",
                        body.clone(),
                        db,
                    );
                    debug_text(
                        b,
                        "By default, a panel\u{2019}s dimensions map directly to \
                         meters. This 210mm-wide A4 page occupies 0.21 meters in \
                         the scene \u{2014} physically correct at 1:1 world scale.",
                        body.clone(),
                        db,
                    );
                },
            );

            b.with(
                El::new()
                    .width(Sizing::fixed(Mm(0.3)))
                    .height(Sizing::GROW)
                    .background(A4_DIM_COLOR),
                |_| {},
            );

            b.with(
                El::column().width(Sizing::GROW).gap(Mm(4.0)).border(db),
                |b| {
                    debug_text(b, "Any Scale You Need", heading.clone(), db);
                    debug_text(
                        b,
                        "Set world_height on any panel and it scales uniformly to \
                         fit. A museum placard at 0.3 meters. A highway billboard \
                         at 12 meters. A planetary-scale announcement at 50,000 \
                         meters \u{2014} zoom out far enough and there it is. Text \
                         stays sharp at any distance by rendering glyphs using SDFs \
                         (signed distance fields), not rasterized bitmaps. \u{2014} \
                         The GPU evaluates the distance function per fragment - \
                         edges remain crisp regardless of scale.",
                        body.clone(),
                        db,
                    );
                    debug_text(
                        b,
                        "PanelDefaults sets panel construction defaults: \
                         layout in meters, fonts in points. \
                         Override per-panel with layout_unit and font_unit, or \
                         per-element with types like Mm(10.0) and Pt(24.0) \
                         inline. The system converts at layout time so the \
                         engine always works in a consistent coordinate space \
                         internally.",
                        body.clone(),
                        db,
                    );
                    debug_text(
                        b,
                        "Custom(0.01) defines a centimeter. Custom(9.461e15) \
                         defines a light-year. The unit system does not care about \
                         scale \u{2014} it only needs to know the ratio to meters.",
                        body.clone(),
                        db,
                    );
                },
            );
        },
    );
}

/// Builds a business card layout tree (used by `toggle_debug_outlines` for runtime rebuild).
fn build_card(debug: bool) -> hana_diegetic::LayoutTree {
    let mut builder = LayoutBuilder::new(CARD_WIDTH, CARD_HEIGHT);
    build_card_content(&mut builder, debug);
    builder.build()
}

/// Populates a business card layout into the given builder.
fn build_card_content(builder: &mut LayoutBuilder, debug: bool) {
    let db = debug_border(debug, DEBUG_OUTLINE);

    builder.with(
        El::column()
            .size(CARD_WIDTH, CARD_HEIGHT)
            .padding(Padding::all(In(0.15)))
            .gap(In(0.04))
            .background(Color::srgb(0.392, 0.584, 0.929)),
        |b| {
            debug_text(
                b,
                "MARY JANE LOGICIELEUR",
                TextStyle::new(CARD_NAME_SIZE).with_color(CARD_TEXT_COLOR),
                db,
            );
            debug_text(
                b,
                "Software Engineer",
                TextStyle::new(CARD_TITLE_SIZE).with_color(CARD_DIM_COLOR),
                db,
            );
            debug_text(
                b,
                "mary-jane@example.com",
                TextStyle::new(CARD_DETAIL_SIZE).with_color(CARD_TEXT_COLOR),
                db,
            );
            debug_text(
                b,
                "+1 (555) 012-3456",
                TextStyle::new(CARD_DETAIL_SIZE).with_color(CARD_TEXT_COLOR),
                db,
            );

            // Spacer
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});

            // Footer
            debug_text(
                b,
                "PaperSize::BusinessCard  |  layout: Inches  |  font: Points",
                TextStyle::new(CARD_FOOTER_SIZE).with_color(CARD_DIM_COLOR),
                db,
            );
        },
    );
}

/// Builds a 5×7 index page layout tree (used by `toggle_debug_outlines` for runtime rebuild).
fn build_index_page(debug: bool) -> hana_diegetic::LayoutTree {
    let mut builder = LayoutBuilder::new(INDEX_WIDTH, INDEX_HEIGHT);
    build_index_content(&mut builder, debug);
    builder.build()
}

/// Populates the 5×7 card with a units API reference table.
fn build_index_content(builder: &mut LayoutBuilder, debug: bool) {
    let db = debug_border(debug, DEBUG_OUTLINE);
    let heading = TextStyle::new(INDEX_HEADING_SIZE).with_color(INDEX_HEADING_COLOR);
    let subheading = TextStyle::new(INDEX_SUBHEADING_SIZE).with_color(INDEX_HEADING_COLOR);
    let label = TextStyle::new(INDEX_LABEL_SIZE).with_color(INDEX_LABEL_COLOR);
    let code = TextStyle::new(INDEX_CODE_SIZE).with_color(INDEX_CODE_COLOR);
    let footer = TextStyle::new(INDEX_FOOTER_SIZE).with_color(INDEX_LABEL_COLOR);

    builder.with(
        El::column()
            .size(INDEX_WIDTH, INDEX_HEIGHT)
            .padding(Padding::all(In(0.2)))
            .gap(In(0.08))
            .background(INDEX_BG),
        |b| {
            debug_text(b, "Units API", heading.clone(), db);

            // ── Panel sizing ────────────────────────────────────
            debug_text(b, "Panel Sizing", subheading.clone(), db);
            index_row(b, "f32", ".size(0.127, 0.178)", &label, &code, db);
            index_row(b, "In", ".size(In(5.0), In(7.0))", &label, &code, db);
            index_row(b, "Mm", ".size(Mm(127.0), Mm(177.8))", &label, &code, db);
            index_row(b, "Pt", ".size(Pt(360.0), Pt(504.0))", &label, &code, db);
            index_row(
                b,
                "PaperSize",
                ".paper(PaperSize::Photo5x7)",
                &label,
                &code,
                db,
            );
            index_row(
                b,
                "Pixels",
                ".size(Px(800.0), Px(600.0))",
                &label,
                &code,
                db,
            );

            // ── Divider ─────────────────────────────────────────
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(In(0.01)))
                    .background(INDEX_HEADING_COLOR),
                |_| {},
            );

            // ── Element sizing ──────────────────────────────────
            debug_text(b, "Element Sizing", subheading.clone(), db);
            index_row(
                b,
                "Fixed",
                "El::new().size(Mm(50.0), Mm(30.0))",
                &label,
                &code,
                db,
            );
            index_row(
                b,
                "Grow",
                "El::new().width(Sizing::GROW)",
                &label,
                &code,
                db,
            );
            index_row(b, "Fit", "El::new().width(Sizing::FIT)", &label, &code, db);
            index_row(
                b,
                "Percent",
                "El::new().width(Sizing::percent(0.5))",
                &label,
                &code,
                db,
            );

            // ── Divider ─────────────────────────────────────────
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::fixed(In(0.01)))
                    .background(INDEX_HEADING_COLOR),
                |_| {},
            );

            // ── World scaling ───────────────────────────────────
            debug_text(b, "World Scaling", subheading.clone(), db);
            index_row(b, "Height", ".world_height(0.5)", &label, &code, db);
            index_row(b, "Width", ".world_width(1.0)", &label, &code, db);

            // Spacer
            b.with(El::new().width(Sizing::GROW).height(Sizing::GROW), |_| {});

            // Footer
            b.with(
                El::new()
                    .width(Sizing::GROW)
                    .align_x(AlignX::Center)
                    .border(db),
                |b| {
                    b.with(El::new().border(db), |b| {
                        b.text((
                            "PaperSize::Photo5x7  |  layout: Inches  |  font: Points",
                            footer,
                        ));
                    });
                },
            );
        },
    );
}

/// A single row in the index card table: label on the left, code on the right.
fn index_row(
    b: &mut LayoutBuilder,
    label_text: &str,
    code_text: &str,
    label: &TextStyle,
    code: &TextStyle,
    db: Border,
) {
    b.with(
        El::row().width(Sizing::GROW).gap(In(0.12)).border(db),
        |b| {
            b.with(El::column().width(Sizing::fixed(In(1.0))).border(db), |b| {
                b.text((label_text, label.clone()));
            });
            b.with(El::column().width(Sizing::GROW).border(db), |b| {
                b.text((code_text, code.clone()));
            });
        },
    );
}

// ── Click handlers ───────────────────────────────────────────────────

fn on_panel_clicked(mut click: On<Pointer<Click>>, mut commands: Commands) {
    if click.button != PointerButton::Primary {
        return;
    }
    click.propagate(false);
    let camera = click.hit.camera;
    commands.trigger(
        ZoomToFit::new(camera, click.entity)
            .margin(ZOOM_MARGIN)
            .duration(Duration::from_millis(ZOOM_DURATION_MS)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.0001;

    #[test]
    fn metric_vertical_ticks_are_inclusive_and_right_aligned() {
        let lines = metric_vertical_tick_lines(10, Color::WHITE);

        assert_eq!(lines.len(), 11);
        assert_close(start_y(&lines[10]), PANEL_RULER_CM_LINE.0 * 0.5);
        assert_close(end_y(&lines[10]), PANEL_RULER_CM_LINE.0 * 0.5);
        assert_close(start_x(&lines[10]), 0.0);
        assert_close(end_x(&lines[10]), PANEL_RULER_CM_TICK.0);
        assert_close(start_y(&lines[0]), 10.0 - PANEL_RULER_CM_LINE.0 * 0.5);
        assert_close(
            start_x(&lines[5]),
            PANEL_RULER_CM_TICK.0 - PANEL_RULER_MM5_TICK.0,
        );
        assert_close(end_x(&lines[5]), PANEL_RULER_CM_TICK.0);
        assert_close_value(line_width(&lines[5]), PANEL_RULER_MM5_LINE.0);
    }

    #[test]
    fn metric_vertical_ruler_lines_include_right_spine() {
        let lines = metric_vertical_ruler_lines(10, Color::WHITE);
        let spine = lines.last().expect("ruler lines should include a spine");

        // 11 ticks plus the spine; cm marks 0 and 10 and the spine pin
        // `HairlineFade::Full`, the rest inherit.
        assert_eq!(lines.len(), 12);
        assert_fade_exempt_marks(&lines, &[0, 10]);
        assert_close(
            start_x(spine),
            PANEL_RULER_CM_TICK.0 + PANEL_RULER_SPINE.0 * 0.5,
        );
        assert_close(
            end_x(spine),
            PANEL_RULER_CM_TICK.0 + PANEL_RULER_SPINE.0 * 0.5,
        );
        assert_close(start_y(spine), 0.0);
        assert_close(end_y(spine), 10.0);
        assert_close_value(line_width(spine), PANEL_RULER_SPINE.0);
    }

    #[test]
    fn metric_horizontal_ticks_are_inclusive_and_endpoint_inset() {
        let lines = metric_horizontal_tick_lines(10, Color::WHITE);

        assert_eq!(lines.len(), 11);
        assert_close(start_x(&lines[0]), PANEL_RULER_CM_LINE.0 * 0.5);
        assert_close(start_y(&lines[0]), 0.0);
        assert_close(end_y(&lines[0]), PANEL_RULER_CM_TICK.0);
        assert_close(start_x(&lines[10]), 10.0 - PANEL_RULER_CM_LINE.0 * 0.5);
        assert_close(end_y(&lines[5]), PANEL_RULER_MM5_TICK.0);
        assert_close_value(line_width(&lines[5]), PANEL_RULER_MM5_LINE.0);
    }

    #[test]
    fn metric_horizontal_ruler_lines_offset_ticks_below_spine() {
        let lines = metric_horizontal_ruler_lines(10, Color::WHITE);
        let first_tick = lines.first().expect("ruler lines should include ticks");
        let spine = lines.last().expect("ruler lines should include a spine");

        assert_eq!(lines.len(), 12);
        assert_fade_exempt_marks(&lines, &[0, 10]);
        assert_close(start_y(first_tick), PANEL_RULER_SPINE.0);
        assert_close(
            end_y(first_tick),
            PANEL_RULER_SPINE.0 + PANEL_RULER_CM_TICK.0,
        );
        assert_close(start_y(spine), PANEL_RULER_SPINE.0 * 0.5);
        assert_close(end_y(spine), PANEL_RULER_SPINE.0 * 0.5);
        assert_close_value(line_width(spine), PANEL_RULER_SPINE.0);
    }

    #[test]
    fn imperial_vertical_ticks_use_measured_track_height() {
        let height_sixteenths = 32;
        let lines = imperial_vertical_tick_lines(height_sixteenths, Color::WHITE);
        let measured_height = height_sixteenths.to_f32() / 16.0;
        let taller_label_panel = measured_height + EDGE_LABEL_EXTRA.0;

        assert_eq!(lines.len(), 33);
        assert_close(
            start_y(&lines[0]),
            measured_height - PANEL_RULER_INCH_LINE.0 * 0.5,
        );
        assert_close(start_y(&lines[32]), PANEL_RULER_INCH_LINE.0 * 0.5);
        assert_close(end_x(&lines[0]), PANEL_RULER_INCH_TICK.0);
        assert!(start_y(&lines[0]).is_some_and(|y| y < taller_label_panel));
    }

    #[test]
    fn imperial_vertical_ruler_lines_offset_ticks_after_spine() {
        let lines = imperial_vertical_ruler_lines(16, Color::WHITE);
        let first_tick = lines.first().expect("ruler lines should include ticks");
        let spine = lines.last().expect("ruler lines should include a spine");

        // 17 ticks plus the spine; inch marks 0 and 16 and the spine pin
        // `HairlineFade::Full`, the rest inherit.
        assert_eq!(lines.len(), 18);
        assert_fade_exempt_marks(&lines, &[0, 16]);
        assert_close(start_x(first_tick), PANEL_RULER_INCH_SPINE.0);
        assert_close(
            end_x(first_tick),
            PANEL_RULER_INCH_SPINE.0 + PANEL_RULER_INCH_TICK.0,
        );
        assert_close(start_x(spine), PANEL_RULER_INCH_SPINE.0 * 0.5);
        assert_close(end_x(spine), PANEL_RULER_INCH_SPINE.0 * 0.5);
        assert_close_value(line_width(spine), PANEL_RULER_INCH_SPINE.0);
    }

    #[test]
    fn imperial_horizontal_ticks_preserve_major_minor_lengths() {
        let width_sixteenths = 16;
        let lines = imperial_horizontal_tick_lines(width_sixteenths, Color::WHITE);

        assert_eq!(lines.len(), 17);
        assert_close(start_x(&lines[0]), PANEL_RULER_INCH_LINE.0 * 0.5);
        assert_close(start_x(&lines[16]), 1.0 - PANEL_RULER_INCH_LINE.0 * 0.5);
        assert_close(end_y(&lines[8]), PANEL_RULER_HALF_TICK.0);
        assert_close_value(line_width(&lines[8]), PANEL_RULER_HALF_LINE.0);
        assert_close(end_y(&lines[1]), PANEL_RULER_16TH_TICK.0);
        assert_close_value(line_width(&lines[1]), PANEL_RULER_16TH_LINE.0);
    }

    #[test]
    fn imperial_horizontal_ruler_lines_offset_ticks_below_spine() {
        let lines = imperial_horizontal_ruler_lines(16, Color::WHITE);
        let first_tick = lines.first().expect("ruler lines should include ticks");
        let spine = lines.last().expect("ruler lines should include a spine");

        assert_eq!(lines.len(), 18);
        assert_fade_exempt_marks(&lines, &[0, 16]);
        assert_close(start_y(first_tick), PANEL_RULER_INCH_SPINE.0);
        assert_close(
            end_y(first_tick),
            PANEL_RULER_INCH_SPINE.0 + PANEL_RULER_INCH_TICK.0,
        );
        assert_close(start_y(spine), PANEL_RULER_INCH_SPINE.0 * 0.5);
        assert_close(end_y(spine), PANEL_RULER_INCH_SPINE.0 * 0.5);
        assert_close_value(line_width(spine), PANEL_RULER_INCH_SPINE.0);
    }

    fn start_x(line: &PanelLine) -> Option<f32> { point_x(line.start()) }

    fn start_y(line: &PanelLine) -> Option<f32> { point_y(line.start()) }

    fn end_x(line: &PanelLine) -> Option<f32> { point_x(line.end()) }

    fn end_y(line: &PanelLine) -> Option<f32> { point_y(line.end()) }

    fn point_x(point: &PanelPoint) -> Option<f32> {
        point.x().start_dimension().map(|dimension| dimension.value)
    }

    fn point_y(point: &PanelPoint) -> Option<f32> {
        point.y().start_dimension().map(|dimension| dimension.value)
    }

    fn line_width(line: &PanelLine) -> f32 { line.line_style().width_dimension().value }

    /// Ruler lines = ticks in mark order then the spine last; the listed
    /// major marks and the spine must pin `HairlineFade::Full`, every other
    /// tick must inherit (no override).
    fn assert_fade_exempt_marks(lines: &[PanelLine], major_marks: &[i32]) {
        let (spine, ticks) = lines.split_last().expect("ruler lines should not be empty");
        assert_eq!(
            spine.line_style().hairline_fade_override(),
            Some(HairlineFade::Full),
            "spine must pin HairlineFade::Full"
        );
        for (mark, tick) in (0_i32..).zip(ticks) {
            let expected = major_marks.contains(&mark).then_some(HairlineFade::Full);
            assert_eq!(
                tick.line_style().hairline_fade_override(),
                expected,
                "tick at mark {mark} has the wrong fade override"
            );
        }
    }

    fn assert_close(actual: Option<f32>, expected: f32) {
        assert!(
            actual.is_some_and(|actual| (actual - expected).abs() <= EPSILON),
            "expected {expected}, got {actual:?}",
        );
    }

    fn assert_close_value(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= EPSILON,
            "expected {expected}, got {actual}",
        );
    }
}
