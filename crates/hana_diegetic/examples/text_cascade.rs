//! `text_cascade` — interactive tour of the `hana_diegetic` text cascade.
//!
//! The cascade rule is: **my own override, else my parent's override, else the
//! global default**. This example shows that rule from both sides:
//!
//! - authoring values with the per-attribute root resources (`TextAlpha`, `FontUnit`),
//!   `override_*`, and `inherit_*`;
//! - reading values back with `resolved_text_alpha` and `resolved_font_unit`.
//!
//! Hotkeys:
//! - `G` — toggle the global text-alpha default: `Blend` / `AlphaToCoverage`.
//! - `P` — toggle the parent panel alpha override: `Add` / inherit.
//! - `L` — toggle one panel label's own alpha override: `Premultiplied` / inherit.
//! - `S` — toggle the standalone alpha override: `Add` / inherit.
//! - `U` — toggle the global font-unit default: `Millimeters` / `Points`.
//! - `F` — toggle the standalone font-unit override: `Points` / inherit.
//! - `H` — home the camera.
//!
//! `G` and `U` affect the world content only. Screen panels author stable panel
//! values so rendering-focused global defaults cannot make example controls hard
//! to read.

use bevy::prelude::AlphaMode;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::World;
use bevy::prelude::error;
use bevy::prelude::warn;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::LABEL_SIZE;
use fairy_dust::TitleBar;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::CascadeEntityCommandsExt;
use hana_diegetic::CascadeSet;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticTextMut;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::FontUnit;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Mm;
use hana_diegetic::Padding;
use hana_diegetic::PanelBuildError;
use hana_diegetic::PanelElementId;
use hana_diegetic::PanelSystems;
use hana_diegetic::PanelText;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::Text;
use hana_diegetic::TextAlpha;
use hana_diegetic::TextContent;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_diegetic::default_panel_material;
use hana_diegetic::resolved_font_unit;
use hana_diegetic::resolved_text_alpha;
use hana_lagrange::OrbitCamPreset;

const EXAMPLE_TITLE: &str = "Text Cascade";
const STANDALONE_TITLE_TEXT: &str = "Standalone World Text";
const DEFAULT_ALPHA_PREFIX: &str = "global alpha";
const STANDALONE_ALPHA_PREFIX: &str = "standalone alpha";
const STANDALONE_UNIT_PREFIX: &str = "standalone unit";
const PANEL_ALPHA_PREFIX: &str = "panel alpha";
const PANEL_LOCAL_PREFIX: &str = "local alpha";
const PANEL_UNIT_PREFIX: &str = "panel unit";
const SCREEN_OVERRIDE_NOTE: &str =
    "This screen legend keeps Blend + Points; G/U affect the world examples above.";
const PANEL_ALPHA_VALUE_ID: &str = "panel_alpha_value";
const PANEL_LOCAL_VALUE_ID: &str = "panel_local_value";
const PANEL_UNIT_VALUE_ID: &str = "panel_unit_value";
const HUD_GLOBAL_ALPHA_VALUE_ID: &str = "hud_global_alpha_value";
const HUD_GLOBAL_ALPHA_AUTHORING_ID: &str = "hud_global_alpha_authoring";
const HUD_GLOBAL_UNIT_VALUE_ID: &str = "hud_global_unit_value";
const HUD_GLOBAL_UNIT_AUTHORING_ID: &str = "hud_global_unit_authoring";
const HUD_STANDALONE_ALPHA_VALUE_ID: &str = "hud_standalone_alpha_value";
const HUD_STANDALONE_ALPHA_AUTHORING_ID: &str = "hud_standalone_alpha_authoring";
const HUD_STANDALONE_UNIT_VALUE_ID: &str = "hud_standalone_unit_value";
const HUD_STANDALONE_UNIT_AUTHORING_ID: &str = "hud_standalone_unit_authoring";
const HUD_PANEL_ALPHA_VALUE_ID: &str = "hud_panel_alpha_value";
const HUD_PANEL_ALPHA_AUTHORING_ID: &str = "hud_panel_alpha_authoring";
const HUD_PANEL_LOCAL_VALUE_ID: &str = "hud_panel_local_value";
const HUD_PANEL_LOCAL_AUTHORING_ID: &str = "hud_panel_local_authoring";
const WORLD_TEXT_LINE_COUNT: usize = 3;

const SCENE_Z: f32 = 2.35;
const SCENE_FRAME_Y: f32 = 0.62;
const SCENE_FRAME_Z_OFFSET: f32 = -0.01;
const HOME_PITCH: f32 = 0.055;
const HOME_YAW: f32 = 0.0;
const HOME_MARGIN: f32 = 0.35;
const HOME_OFFSET_PX: Vec2 = Vec2::new(-140.0, -100.0);

const GLOBAL_ALPHA_A: AlphaMode = AlphaMode::Blend;
const GLOBAL_ALPHA_B: AlphaMode = AlphaMode::AlphaToCoverage;
const PANEL_ALPHA: AlphaMode = AlphaMode::Add;
const LABEL_ALPHA: AlphaMode = AlphaMode::Premultiplied;
const STANDALONE_ALPHA: AlphaMode = AlphaMode::Add;

const GLOBAL_UNIT_A: Unit = Unit::Millimeters;
const GLOBAL_UNIT_B: Unit = Unit::Points;
const STANDALONE_UNIT: Unit = Unit::Points;

const GROUND_PLANE_SIZE: f32 = 3.2;
const GROUND_PLANE_WIDTH_SCALE: f32 = 1.18;
const GROUND_PLANE_DEPTH_SCALE: f32 = 0.42;
const GROUND_PLANE_Z_OFFSET: f32 = -0.35;
const GROUND_PLANE_Z: f32 = SCENE_Z + GROUND_PLANE_Z_OFFSET;

const SCENE_FRAME_WIDTH: Mm = Mm(2460.0);
const SCENE_FRAME_HEIGHT: Mm = Mm(620.0);
const SCENE_FRAME_LEFT_WIDTH: Mm = Mm(1300.0);

const WORLD_TEXT_X: f32 = -1.08;
const WORLD_TEXT_TITLE_OFFSET_FROM_FRAME: f32 = 0.17;
const WORLD_TEXT_TITLE_BODY_GAP: f32 = 0.10;
const WORLD_TEXT_ROW_GAP: f32 = 0.105;
const WORLD_TEXT_TITLE_Y: f32 = SCENE_FRAME_Y + WORLD_TEXT_TITLE_OFFSET_FROM_FRAME;
const WORLD_TEXT_DEFAULT_Y: f32 = WORLD_TEXT_TITLE_Y - WORLD_TEXT_TITLE_BODY_GAP;
const WORLD_TEXT_ALPHA_Y: f32 = WORLD_TEXT_DEFAULT_Y - WORLD_TEXT_ROW_GAP;
const WORLD_TEXT_UNIT_Y: f32 = WORLD_TEXT_ALPHA_Y - WORLD_TEXT_ROW_GAP;

const WORLD_TITLE_SIZE: f32 = 0.052;
const WORLD_TEXT_SIZE: f32 = 0.036;
const WORLD_UNIT_TEXT_SIZE: f32 = 36.0;
const PANEL_TEXT_SIZE: f32 = 36.0;
const PANEL_TITLE_SIZE: f32 = 52.0;

const DEFAULT_COLOR: Color = Color::srgb(0.55, 0.75, 1.0);
const INHERITED_COLOR: Color = Color::srgb(0.35, 1.0, 0.7);
const OVERRIDE_COLOR: Color = Color::srgb(1.0, 0.8, 0.3);
const HUD_HEADER_COLOR: Color = Color::srgb(0.55, 0.78, 0.95);
const HUD_CONTROL_COLOR: Color = Color::srgb(0.78, 0.84, 0.92);
const HUD_KEY_COLOR: Color = Color::srgb(0.96, 0.76, 0.36);
const PANEL_BORDER: Color = Color::srgba(0.15, 0.7, 0.9, 0.5);
const PANEL_BORDER_ACCENT: Color = Color::srgba(0.15, 0.7, 0.9, 0.85);
const PANEL_BORDER_DIM: Color = Color::srgba(0.1, 0.4, 0.6, 0.3);
const INVISIBLE_FRAME_BACKGROUND: Color = Color::srgba(0.0, 0.0, 0.0, 0.0);
const PANEL_FRAME_BACKGROUND: Color = Color::srgba(0.0, 0.02, 0.04, 0.35);

const PANEL_FRAME_PAD: Mm = Mm(18.0);
const PANEL_INNER_PAD: Mm = Mm(28.0);
const PANEL_ROW_GAP: Mm = Mm(40.0);
const PANEL_BORDER_WIDTH: Mm = Mm(6.0);
const PANEL_RADIUS: Mm = Mm(54.0);
const PANEL_INNER_RADIUS: Mm = Mm(34.0);
const PANEL_INNER_BORDER_WIDTH: Mm = Mm(3.0);

const HUD_PADDING: Px = Px(10.0);
const HUD_RADIUS: Px = Px(8.0);
const HUD_BORDER_WIDTH: Px = Px(1.0);
const HUD_CONTROL_CELL_GAP: Px = Px(6.0);
const HUD_ROW_GAP: Px = Px(4.0);
const HUD_SECTION_GAP: Px = Px(8.0);
const HUD_CONTROL_KEY_WIDTH: Px = Px(28.0);
const HUD_CONTROL_LABEL_WIDTH: Px = Px(130.0);
const HUD_CONTROL_AUTHORING_WIDTH: Px = Px(150.0);
const HUD_CONTROL_VALUE_WIDTH: Px = Px(150.0);

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .size(GROUND_PLANE_SIZE)
        .transform(
            Transform::from_xyz(0.0, 0.0, GROUND_PLANE_Z).with_scale(Vec3::new(
                GROUND_PLANE_WIDTH_SCALE,
                1.0,
                GROUND_PLANE_DEPTH_SCALE,
            )),
        )
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_stable_transparency()
        .with_camera_home()
        .pitch(HOME_PITCH)
        .yaw(HOME_YAW)
        .margin(HOME_MARGIN)
        .offset_px(HOME_OFFSET_PX)
        .with_title_bar(TitleBar::new().with_title(EXAMPLE_TITLE))
        .with_camera_control_panel()
        .insert_resource(TextAlpha(GLOBAL_ALPHA_A))
        .insert_resource(FontUnit(GLOBAL_UNIT_A))
        .init_resource::<CascadeDemoState>()
        .init_resource::<HudSnapshotCache>()
        .add_systems(Startup, setup)
        // Each cascade toggle runs through Fairy Dust's shortcut binding, which
        // fires only when no modifier is held — so bare `L` does not also fire
        // on the `Ctrl+Shift+L` screen-panel chord.
        .with_shortcut(KeyCode::KeyG, cycle_global_alpha)
        .with_shortcut(KeyCode::KeyU, cycle_global_unit)
        .with_shortcut(KeyCode::KeyP, toggle_panel_alpha)
        .with_shortcut(KeyCode::KeyS, toggle_standalone_alpha)
        .with_shortcut(KeyCode::KeyF, toggle_standalone_unit)
        .with_shortcut(KeyCode::KeyL, toggle_label_alpha)
        .add_systems(
            Update,
            (refresh_hud, refresh_world_text, refresh_panel_text)
                .chain()
                .after(CascadeSet::Propagate)
                .before(PanelSystems::ComputeLayout),
        )
        .run();
}

#[derive(Resource, Clone, Copy)]
struct CascadeDemoEntities {
    default_alpha:    Entity,
    standalone_alpha: Entity,
    standalone_unit:  Entity,
    panel:            Entity,
    hud:              Entity,
}

#[derive(Component)]
enum WorldTextLine {
    GlobalAlpha,
    StandaloneAlpha,
    StandaloneUnit,
}

#[derive(Resource, Clone, Copy, PartialEq)]
struct CascadeDemoState {
    global_alpha:     AlphaMode,
    global_unit:      Unit,
    panel_alpha:      OverrideMode,
    label_alpha:      OverrideMode,
    standalone_alpha: OverrideMode,
    standalone_unit:  OverrideMode,
}

#[derive(Clone, Copy, PartialEq)]
enum OverrideMode {
    Inherit,
    Override,
}

impl OverrideMode {
    const fn toggled(self) -> Self {
        match self {
            Self::Inherit => Self::Override,
            Self::Override => Self::Inherit,
        }
    }

    const fn is_override(self) -> bool { matches!(self, Self::Override) }
}

#[derive(Clone, Copy)]
enum CascadeAuthorship {
    GlobalDefault,
    Override,
    InheritedGlobal,
    InheritedPanel,
}

impl CascadeAuthorship {
    const fn label(self) -> &'static str {
        match self {
            Self::GlobalDefault => "global default",
            Self::Override => "override",
            Self::InheritedGlobal => "inherited: global",
            Self::InheritedPanel => "inherited: panel",
        }
    }
}

impl Default for CascadeDemoState {
    fn default() -> Self {
        Self {
            global_alpha:     GLOBAL_ALPHA_A,
            global_unit:      GLOBAL_UNIT_A,
            panel_alpha:      OverrideMode::Override,
            label_alpha:      OverrideMode::Override,
            standalone_alpha: OverrideMode::Override,
            standalone_unit:  OverrideMode::Inherit,
        }
    }
}

#[derive(Resource, Default)]
struct HudSnapshotCache(Option<HudSnapshot>);

#[derive(Clone, PartialEq)]
struct HudSnapshot {
    state:                 CascadeDemoState,
    default_alpha:         AlphaMode,
    standalone_alpha:      AlphaMode,
    standalone_unit:       Unit,
    panel_unit:            Unit,
    panel_inherited_alpha: Option<AlphaMode>,
    panel_local_alpha:     Option<AlphaMode>,
}

impl Default for HudSnapshot {
    fn default() -> Self {
        Self {
            state:                 CascadeDemoState::default(),
            default_alpha:         GLOBAL_ALPHA_A,
            standalone_alpha:      STANDALONE_ALPHA,
            standalone_unit:       GLOBAL_UNIT_A,
            panel_unit:            GLOBAL_UNIT_A,
            panel_inherited_alpha: Some(PANEL_ALPHA),
            panel_local_alpha:     Some(LABEL_ALPHA),
        }
    }
}

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let initial = HudSnapshot::default();
    let scene_frame = build_scene_frame_panel(&mut materials, &initial).map(|panel| {
        commands
            .spawn((
                CameraHomeTarget,
                panel,
                Transform::from_xyz(0.0, SCENE_FRAME_Y, SCENE_Z + SCENE_FRAME_Z_OFFSET),
            ))
            .inherit_font_unit()
            .id()
    });

    spawn_standalone_title(&mut commands);

    let default_alpha = commands
        .spawn((
            WorldTextLine::GlobalAlpha,
            DiegeticText::world(alpha_line(
                DEFAULT_ALPHA_PREFIX,
                GLOBAL_ALPHA_A,
                CascadeAuthorship::GlobalDefault,
            ))
            .size(WORLD_TEXT_SIZE)
            .color(DEFAULT_COLOR)
            .anchor(Anchor::TopLeft)
            .unlit()
            .transform(Transform::from_xyz(
                WORLD_TEXT_X,
                WORLD_TEXT_DEFAULT_Y,
                SCENE_Z,
            ))
            .build(),
        ))
        .id();

    let standalone_alpha = commands
        .spawn((
            WorldTextLine::StandaloneAlpha,
            DiegeticText::world(alpha_line(
                STANDALONE_ALPHA_PREFIX,
                STANDALONE_ALPHA,
                CascadeAuthorship::Override,
            ))
            .size(WORLD_TEXT_SIZE)
            .color(OVERRIDE_COLOR)
            .anchor(Anchor::TopLeft)
            .unlit()
            .transform(Transform::from_xyz(
                WORLD_TEXT_X,
                WORLD_TEXT_ALPHA_Y,
                SCENE_Z,
            ))
            .build(),
        ))
        .override_text_alpha(STANDALONE_ALPHA)
        .id();

    let standalone_unit = commands
        .spawn((
            WorldTextLine::StandaloneUnit,
            DiegeticText::world(unit_line(
                STANDALONE_UNIT_PREFIX,
                GLOBAL_UNIT_A,
                CascadeAuthorship::InheritedGlobal,
            ))
            .size(WORLD_UNIT_TEXT_SIZE)
            .color(DEFAULT_COLOR)
            .anchor(Anchor::TopLeft)
            .unlit()
            .transform(Transform::from_xyz(
                WORLD_TEXT_X,
                WORLD_TEXT_UNIT_Y,
                SCENE_Z,
            ))
            .build(),
        ))
        .inherit_font_unit()
        .id();

    let hud = build_hud_panel(&mut materials, &initial)
        .map(|panel| commands.spawn((HudPanel, panel, Transform::default())).id());

    match (scene_frame, hud) {
        (Ok(scene_frame), Ok(hud)) => {
            commands.insert_resource(CascadeDemoEntities {
                default_alpha,
                standalone_alpha,
                standalone_unit,
                panel: scene_frame,
                hud,
            });
        },
        (Err(error), _) => {
            error!("cascade: failed to build scene frame: {error}");
        },
        (_, Err(error)) => {
            error!("cascade: failed to build HUD panel: {error}");
        },
    }
}

fn spawn_standalone_title(commands: &mut Commands) {
    commands.spawn(
        DiegeticText::world(STANDALONE_TITLE_TEXT)
            .size(WORLD_TITLE_SIZE)
            .color(HUD_HEADER_COLOR)
            .anchor(Anchor::TopLeft)
            .unlit()
            .transform(Transform::from_xyz(
                WORLD_TEXT_X,
                WORLD_TEXT_TITLE_Y,
                SCENE_Z,
            ))
            .build(),
    );
}

fn build_scene_frame_panel(
    materials: &mut Assets<StandardMaterial>,
    snapshot: &HudSnapshot,
) -> Result<DiegeticPanel, PanelBuildError> {
    let unlit = StandardMaterial {
        unlit: true,
        ..default_panel_material()
    };
    let unlit = materials.add(unlit);
    DiegeticPanel::world()
        .size(SCENE_FRAME_WIDTH, SCENE_FRAME_HEIGHT)
        .anchor(Anchor::Center)
        .material(unlit.clone())
        .text_material(unlit)
        .text_alpha_mode(PANEL_ALPHA)
        .with_tree(build_scene_frame_tree(snapshot))
        .build()
}

fn build_scene_frame_tree(snapshot: &HudSnapshot) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(INVISIBLE_FRAME_BACKGROUND),
    );
    builder.with(
        El::row().width(Sizing::GROW).height(Sizing::GROW),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(SCENE_FRAME_LEFT_WIDTH))
                    .height(Sizing::GROW),
                |_| {},
            );
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .alignment(AlignX::Center, AlignY::Center),
                |builder| build_panel_layout(builder, snapshot),
            );
        },
    );
    builder.build()
}

fn build_hud_panel(
    materials: &mut Assets<StandardMaterial>,
    snapshot: &HudSnapshot,
) -> Result<DiegeticPanel, PanelBuildError> {
    let unlit = StandardMaterial {
        unlit: true,
        ..default_panel_material()
    };
    let unlit = materials.add(unlit);
    DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_hud_tree(snapshot))
        .build()
}

fn build_panel_layout(builder: &mut LayoutBuilder, snapshot: &HudSnapshot) {
    let title = TextStyle::new(PANEL_TITLE_SIZE).with_color(HUD_HEADER_COLOR);
    let panel_alpha_style = panel_alpha_style(snapshot.state.panel_alpha);
    let panel_local_style = panel_local_style(snapshot.state.label_alpha);
    let unit = TextStyle::new(PANEL_TEXT_SIZE).with_color(DEFAULT_COLOR);
    let panel_alpha_text = panel_alpha_text(snapshot);
    let local_text = panel_local_text(snapshot);
    let unit_text = panel_unit_text(snapshot);

    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(PANEL_FRAME_PAD))
            .alignment(AlignX::Center, AlignY::Center)
            .corner_radius(CornerRadius::all(PANEL_RADIUS))
            .background(PANEL_FRAME_BACKGROUND)
            .border(Border::all(PANEL_BORDER_WIDTH, PANEL_BORDER_ACCENT)),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .padding(Padding::all(PANEL_INNER_PAD))
                    .gap(PANEL_ROW_GAP)
                    .corner_radius(CornerRadius::all(PANEL_INNER_RADIUS))
                    .background(DEFAULT_PANEL_BACKGROUND)
                    .border(Border::all(PANEL_INNER_BORDER_WIDTH, PANEL_BORDER_DIM)),
                |builder| {
                    builder.text(("Panel Text", title));
                    builder.text(
                        Text::new(panel_alpha_text, panel_alpha_style)
                            .id(PanelElementId::named(PANEL_ALPHA_VALUE_ID))
                            .measure_as(panel_alpha_measure_text()),
                    );
                    builder.text(
                        Text::new(local_text, panel_local_style)
                            .id(PanelElementId::named(PANEL_LOCAL_VALUE_ID))
                            .measure_as(panel_local_measure_text()),
                    );
                    builder.text(
                        Text::new(unit_text, unit)
                            .id(PanelElementId::named(PANEL_UNIT_VALUE_ID))
                            .measure_as(panel_unit_measure_text()),
                    );
                },
            );
        },
    );
}

/// `G` — cycles the global `TextAlpha` default every entity inherits.
fn cycle_global_alpha(mut state: ResMut<CascadeDemoState>, mut alpha_default: ResMut<TextAlpha>) {
    state.global_alpha = next_alpha_default(state.global_alpha);
    alpha_default.0 = state.global_alpha;
}

/// `U` — cycles the global `FontUnit` default every entity inherits.
fn cycle_global_unit(mut state: ResMut<CascadeDemoState>, mut unit_default: ResMut<FontUnit>) {
    state.global_unit = next_unit_default(state.global_unit);
    unit_default.0 = state.global_unit;
}

/// `P` — toggles whether the panel overrides or inherits text alpha.
fn toggle_panel_alpha(
    entities: Option<Res<CascadeDemoEntities>>,
    mut state: ResMut<CascadeDemoState>,
    mut commands: Commands,
) {
    let Some(entities) = entities else {
        return;
    };
    state.panel_alpha = state.panel_alpha.toggled();
    let mut panel = commands.entity(entities.panel);
    if state.panel_alpha.is_override() {
        panel.override_text_alpha(PANEL_ALPHA);
    } else {
        panel.inherit_text_alpha();
    }
}

/// `S` — toggles whether the standalone text overrides or inherits text alpha.
fn toggle_standalone_alpha(
    entities: Option<Res<CascadeDemoEntities>>,
    mut state: ResMut<CascadeDemoState>,
    mut commands: Commands,
) {
    let Some(entities) = entities else {
        return;
    };
    state.standalone_alpha = state.standalone_alpha.toggled();
    let mut standalone = commands.entity(entities.standalone_alpha);
    if state.standalone_alpha.is_override() {
        standalone.override_text_alpha(STANDALONE_ALPHA);
    } else {
        standalone.inherit_text_alpha();
    }
}

/// `F` — toggles whether the standalone text overrides or inherits font unit.
fn toggle_standalone_unit(
    entities: Option<Res<CascadeDemoEntities>>,
    mut state: ResMut<CascadeDemoState>,
    mut commands: Commands,
) {
    let Some(entities) = entities else {
        return;
    };
    state.standalone_unit = state.standalone_unit.toggled();
    let mut standalone = commands.entity(entities.standalone_unit);
    if state.standalone_unit.is_override() {
        standalone.override_font_unit(STANDALONE_UNIT);
    } else {
        standalone.inherit_font_unit();
    }
}

/// `L` — toggles whether the panel's local label overrides or inherits alpha.
fn toggle_label_alpha(
    entities: Option<Res<CascadeDemoEntities>>,
    mut state: ResMut<CascadeDemoState>,
    mut panel_text: PanelText,
) {
    let Some(entities) = entities else {
        return;
    };
    let next = state.label_alpha.toggled();
    if !panel_text.set_style(
        entities.panel,
        &PanelElementId::named(PANEL_LOCAL_VALUE_ID),
        panel_local_style(next),
    ) {
        warn!("cascade: panel label is not spawned yet");
        return;
    }
    state.label_alpha = next;
}

fn next_alpha_default(current: AlphaMode) -> AlphaMode {
    if current == GLOBAL_ALPHA_A {
        GLOBAL_ALPHA_B
    } else {
        GLOBAL_ALPHA_A
    }
}

fn next_unit_default(current: Unit) -> Unit {
    if current == GLOBAL_UNIT_A {
        GLOBAL_UNIT_B
    } else {
        GLOBAL_UNIT_A
    }
}

fn refresh_hud(world: &mut World) {
    let Some(entities) = world.get_resource::<CascadeDemoEntities>().copied() else {
        return;
    };
    let state = *world.resource::<CascadeDemoState>();
    let snapshot = HudSnapshot {
        state,
        default_alpha: resolved_text_alpha(world, entities.default_alpha),
        standalone_alpha: resolved_text_alpha(world, entities.standalone_alpha),
        standalone_unit: resolved_font_unit(world, entities.standalone_unit),
        panel_unit: resolved_font_unit(world, entities.panel),
        panel_inherited_alpha: Some(resolved_text_alpha(world, entities.panel)),
        panel_local_alpha: find_text_entity_by_prefix(world, PANEL_LOCAL_PREFIX)
            .map(|entity| resolved_text_alpha(world, entity)),
    };

    let mut cache = world.resource_mut::<HudSnapshotCache>();
    if cache.0.as_ref() == Some(&snapshot) {
        return;
    }
    cache.0 = Some(snapshot);
}

fn refresh_world_text(
    cache: Res<HudSnapshotCache>,
    mut applied: Local<Option<HudSnapshot>>,
    mut lines: DiegeticTextMut<WorldTextLine>,
) {
    let Some(snapshot) = cache.0.as_ref() else {
        return;
    };
    if applied.as_ref() == Some(snapshot) {
        return;
    }
    let visited = lines.for_each_mut(|line, text_edit| {
        let text = match line {
            WorldTextLine::GlobalAlpha => alpha_line(
                DEFAULT_ALPHA_PREFIX,
                snapshot.default_alpha,
                CascadeAuthorship::GlobalDefault,
            ),
            WorldTextLine::StandaloneAlpha => standalone_alpha_text(snapshot),
            WorldTextLine::StandaloneUnit => standalone_unit_text(snapshot),
        };
        text_edit.set_text(text);
    });
    if visited == WORLD_TEXT_LINE_COUNT {
        *applied = Some(snapshot.clone());
    }
}

fn refresh_panel_text(
    cache: Res<HudSnapshotCache>,
    entities: Option<Res<CascadeDemoEntities>>,
    mut applied: Local<Option<HudSnapshot>>,
    mut panel_text: PanelText,
) {
    let (Some(snapshot), Some(entities)) = (cache.0.as_ref(), entities) else {
        return;
    };
    if applied.as_ref() == Some(snapshot) {
        return;
    }

    let updates = [
        panel_text.set_style(
            entities.panel,
            &PanelElementId::named(PANEL_ALPHA_VALUE_ID),
            panel_alpha_style(snapshot.state.panel_alpha),
        ),
        panel_text.set_style(
            entities.panel,
            &PanelElementId::named(PANEL_LOCAL_VALUE_ID),
            panel_local_style(snapshot.state.label_alpha),
        ),
        panel_text.set_text(
            entities.panel,
            &PanelElementId::named(PANEL_ALPHA_VALUE_ID),
            panel_alpha_text(snapshot),
        ),
        panel_text.set_text(
            entities.panel,
            &PanelElementId::named(PANEL_LOCAL_VALUE_ID),
            panel_local_text(snapshot),
        ),
        panel_text.set_text(
            entities.panel,
            &PanelElementId::named(PANEL_UNIT_VALUE_ID),
            panel_unit_text(snapshot),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_GLOBAL_ALPHA_AUTHORING_ID),
            CascadeAuthorship::GlobalDefault.label(),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_GLOBAL_ALPHA_VALUE_ID),
            alpha_label(snapshot.state.global_alpha),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_GLOBAL_UNIT_AUTHORING_ID),
            CascadeAuthorship::GlobalDefault.label(),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_GLOBAL_UNIT_VALUE_ID),
            unit_label(snapshot.state.global_unit),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_STANDALONE_ALPHA_AUTHORING_ID),
            standalone_alpha_authorship(snapshot).label(),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_STANDALONE_ALPHA_VALUE_ID),
            alpha_label(snapshot.standalone_alpha),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_STANDALONE_UNIT_AUTHORING_ID),
            standalone_unit_authorship(snapshot).label(),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_STANDALONE_UNIT_VALUE_ID),
            unit_label(snapshot.standalone_unit),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_PANEL_ALPHA_AUTHORING_ID),
            panel_alpha_authorship(snapshot).label(),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_PANEL_ALPHA_VALUE_ID),
            alpha_label(resolved_panel_alpha(snapshot)),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_PANEL_LOCAL_AUTHORING_ID),
            panel_local_authorship(snapshot).label(),
        ),
        panel_text.set_text(
            entities.hud,
            &PanelElementId::named(HUD_PANEL_LOCAL_VALUE_ID),
            alpha_label(resolved_panel_local_alpha(snapshot)),
        ),
    ];
    if updates.into_iter().all(|updated| updated) {
        *applied = Some(snapshot.clone());
    }
}

fn find_text_entity_by_prefix(world: &mut World, prefix: &str) -> Option<Entity> {
    let mut labels = world.query::<(Entity, &TextContent)>();
    labels
        .iter(world)
        .find_map(|(entity, text)| text.text().starts_with(prefix).then_some(entity))
}

#[derive(Component)]
struct HudPanel;

fn build_hud_tree(snapshot: &HudSnapshot) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    let header = TextStyle::new(LABEL_SIZE).with_color(HUD_HEADER_COLOR);
    let control_key = TextStyle::new(LABEL_SIZE).with_color(HUD_KEY_COLOR);
    let control_text = TextStyle::new(LABEL_SIZE).with_color(HUD_CONTROL_COLOR);

    builder.with(
        El::row().width(Sizing::FIT).height(Sizing::FIT),
        |builder| {
            build_hud_card(builder, |builder| {
                build_controls_table(builder, snapshot, &header, &control_key, &control_text);
            });
        },
    );
    builder.build()
}

fn build_hud_card(builder: &mut LayoutBuilder, build: impl FnOnce(&mut LayoutBuilder)) {
    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .padding(Padding::all(HUD_PADDING))
            .gap(HUD_SECTION_GAP)
            .corner_radius(CornerRadius::all(HUD_RADIUS))
            .background(DEFAULT_PANEL_BACKGROUND)
            .border(Border::all(HUD_BORDER_WIDTH, PANEL_BORDER)),
        build,
    );
}

fn build_controls_table(
    builder: &mut LayoutBuilder,
    snapshot: &HudSnapshot,
    header: &TextStyle,
    control_key: &TextStyle,
    control_text: &TextStyle,
) {
    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(HUD_SECTION_GAP),
        |builder| {
            builder.text(("World cascade controls", header.clone()));
            controls_header_row(builder, control_text);
            controls_section(builder, "Global defaults", header, |builder| {
                controls_row(
                    builder,
                    "G",
                    "text alpha",
                    CascadeAuthorship::GlobalDefault,
                    alpha_label(snapshot.state.global_alpha),
                    HUD_GLOBAL_ALPHA_AUTHORING_ID,
                    HUD_GLOBAL_ALPHA_VALUE_ID,
                    control_key,
                    control_text,
                );
                controls_row(
                    builder,
                    "U",
                    "font unit",
                    CascadeAuthorship::GlobalDefault,
                    unit_label(snapshot.state.global_unit),
                    HUD_GLOBAL_UNIT_AUTHORING_ID,
                    HUD_GLOBAL_UNIT_VALUE_ID,
                    control_key,
                    control_text,
                );
            });
            controls_section(builder, STANDALONE_TITLE_TEXT, header, |builder| {
                controls_row(
                    builder,
                    "S",
                    "text alpha",
                    standalone_alpha_authorship(snapshot),
                    alpha_label(snapshot.standalone_alpha),
                    HUD_STANDALONE_ALPHA_AUTHORING_ID,
                    HUD_STANDALONE_ALPHA_VALUE_ID,
                    control_key,
                    control_text,
                );
                controls_row(
                    builder,
                    "F",
                    "font unit",
                    standalone_unit_authorship(snapshot),
                    unit_label(snapshot.standalone_unit),
                    HUD_STANDALONE_UNIT_AUTHORING_ID,
                    HUD_STANDALONE_UNIT_VALUE_ID,
                    control_key,
                    control_text,
                );
            });
            controls_section(builder, "Panel Text", header, |builder| {
                controls_row(
                    builder,
                    "P",
                    "panel alpha",
                    panel_alpha_authorship(snapshot),
                    alpha_label(resolved_panel_alpha(snapshot)),
                    HUD_PANEL_ALPHA_AUTHORING_ID,
                    HUD_PANEL_ALPHA_VALUE_ID,
                    control_key,
                    control_text,
                );
                controls_row(
                    builder,
                    "L",
                    "label alpha",
                    panel_local_authorship(snapshot),
                    alpha_label(resolved_panel_local_alpha(snapshot)),
                    HUD_PANEL_LOCAL_AUTHORING_ID,
                    HUD_PANEL_LOCAL_VALUE_ID,
                    control_key,
                    control_text,
                );
            });
            builder.text((SCREEN_OVERRIDE_NOTE, control_text.clone()));
        },
    );
}

fn controls_header_row(builder: &mut LayoutBuilder, style: &TextStyle) {
    builder.with(controls_row_layout(), |builder| {
        controls_cell(builder, HUD_CONTROL_KEY_WIDTH, "Key", style);
        controls_cell(builder, HUD_CONTROL_LABEL_WIDTH, "Property", style);
        controls_cell(builder, HUD_CONTROL_AUTHORING_WIDTH, "Authoring", style);
        controls_cell(builder, HUD_CONTROL_VALUE_WIDTH, "Resolved", style);
    });
}

fn controls_section(
    builder: &mut LayoutBuilder,
    title: &str,
    header: &TextStyle,
    build: impl FnOnce(&mut LayoutBuilder),
) {
    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(HUD_ROW_GAP),
        |builder| {
            builder.text((title, header.clone()));
            build(builder);
        },
    );
}

fn controls_row(
    builder: &mut LayoutBuilder,
    key: &str,
    label: &str,
    cascade_authorship: CascadeAuthorship,
    value: &str,
    authorship_id: &str,
    value_id: &str,
    key_style: &TextStyle,
    text_style: &TextStyle,
) {
    builder.with(controls_row_layout(), |builder| {
        controls_cell(builder, HUD_CONTROL_KEY_WIDTH, key, key_style);
        controls_cell(builder, HUD_CONTROL_LABEL_WIDTH, label, text_style);
        controls_named_cell(
            builder,
            HUD_CONTROL_AUTHORING_WIDTH,
            cascade_authorship.label(),
            authorship_id,
            text_style,
        );
        controls_named_cell(
            builder,
            HUD_CONTROL_VALUE_WIDTH,
            value,
            value_id,
            text_style,
        );
    });
}

fn controls_row_layout() -> El {
    El::row()
        .width(Sizing::FIT)
        .height(Sizing::FIT)
        .gap(HUD_CONTROL_CELL_GAP)
}

fn controls_cell(builder: &mut LayoutBuilder, width: Px, text: &str, style: &TextStyle) {
    builder.with(
        El::new().width(Sizing::fixed(width)).height(Sizing::FIT),
        |builder| {
            builder.text((text, style.clone()));
        },
    );
}

fn controls_named_cell(
    builder: &mut LayoutBuilder,
    width: Px,
    text: &str,
    id: &str,
    style: &TextStyle,
) {
    builder.with(
        El::new().width(Sizing::fixed(width)).height(Sizing::FIT),
        |builder| {
            builder.text(Text::new(text, style.clone()).id(PanelElementId::named(id)));
        },
    );
}

fn panel_local_style(mode: OverrideMode) -> TextStyle {
    let color = if mode.is_override() {
        OVERRIDE_COLOR
    } else {
        INHERITED_COLOR
    };
    let style = TextStyle::new(PANEL_TEXT_SIZE).with_color(color);
    if mode.is_override() {
        style.with_alpha_mode(LABEL_ALPHA)
    } else {
        style
    }
}

fn panel_alpha_style(mode: OverrideMode) -> TextStyle {
    let color = if mode.is_override() {
        OVERRIDE_COLOR
    } else {
        INHERITED_COLOR
    };
    TextStyle::new(PANEL_TEXT_SIZE).with_color(color)
}

fn standalone_alpha_text(snapshot: &HudSnapshot) -> String {
    format!(
        "{STANDALONE_ALPHA_PREFIX} = {}",
        standalone_alpha_value(snapshot)
    )
}

fn standalone_alpha_value(snapshot: &HudSnapshot) -> String {
    alpha_value(
        snapshot.standalone_alpha,
        standalone_alpha_authorship(snapshot),
    )
}

const fn standalone_alpha_authorship(snapshot: &HudSnapshot) -> CascadeAuthorship {
    if snapshot.state.standalone_alpha.is_override() {
        CascadeAuthorship::Override
    } else {
        CascadeAuthorship::InheritedGlobal
    }
}

fn standalone_unit_text(snapshot: &HudSnapshot) -> String {
    format!(
        "{STANDALONE_UNIT_PREFIX} = {}",
        standalone_unit_value(snapshot)
    )
}

fn standalone_unit_value(snapshot: &HudSnapshot) -> String {
    unit_value(
        snapshot.standalone_unit,
        standalone_unit_authorship(snapshot),
    )
}

const fn standalone_unit_authorship(snapshot: &HudSnapshot) -> CascadeAuthorship {
    if snapshot.state.standalone_unit.is_override() {
        CascadeAuthorship::Override
    } else {
        CascadeAuthorship::InheritedGlobal
    }
}

fn panel_alpha_text(snapshot: &HudSnapshot) -> String {
    format!("{PANEL_ALPHA_PREFIX} = {}", panel_alpha_value(snapshot))
}

fn panel_alpha_value(snapshot: &HudSnapshot) -> String {
    alpha_value(
        resolved_panel_alpha(snapshot),
        panel_alpha_authorship(snapshot),
    )
}

const fn panel_alpha_authorship(snapshot: &HudSnapshot) -> CascadeAuthorship {
    if snapshot.state.panel_alpha.is_override() {
        CascadeAuthorship::Override
    } else {
        CascadeAuthorship::InheritedGlobal
    }
}

fn resolved_panel_alpha(snapshot: &HudSnapshot) -> AlphaMode {
    snapshot
        .panel_inherited_alpha
        .unwrap_or(snapshot.state.global_alpha)
}

fn panel_local_text(snapshot: &HudSnapshot) -> String {
    format!("{PANEL_LOCAL_PREFIX} = {}", panel_local_value(snapshot))
}

fn panel_local_value(snapshot: &HudSnapshot) -> String {
    alpha_value(
        resolved_panel_local_alpha(snapshot),
        panel_local_authorship(snapshot),
    )
}

const fn panel_local_authorship(snapshot: &HudSnapshot) -> CascadeAuthorship {
    match (snapshot.state.label_alpha, snapshot.state.panel_alpha) {
        (OverrideMode::Override, _) => CascadeAuthorship::Override,
        (OverrideMode::Inherit, OverrideMode::Override) => CascadeAuthorship::InheritedPanel,
        (OverrideMode::Inherit, OverrideMode::Inherit) => CascadeAuthorship::InheritedGlobal,
    }
}

fn resolved_panel_local_alpha(snapshot: &HudSnapshot) -> AlphaMode {
    snapshot
        .panel_local_alpha
        .unwrap_or(snapshot.state.global_alpha)
}

fn panel_unit_text(snapshot: &HudSnapshot) -> String {
    unit_line(
        PANEL_UNIT_PREFIX,
        snapshot.panel_unit,
        CascadeAuthorship::InheritedGlobal,
    )
}

fn panel_alpha_measure_text() -> String {
    alpha_line(
        PANEL_ALPHA_PREFIX,
        GLOBAL_ALPHA_B,
        CascadeAuthorship::InheritedGlobal,
    )
}

fn panel_local_measure_text() -> String {
    alpha_line(
        PANEL_LOCAL_PREFIX,
        GLOBAL_ALPHA_B,
        CascadeAuthorship::InheritedGlobal,
    )
}

fn panel_unit_measure_text() -> String {
    unit_line(
        PANEL_UNIT_PREFIX,
        GLOBAL_UNIT_A,
        CascadeAuthorship::InheritedGlobal,
    )
}

fn alpha_line(label: &str, alpha: AlphaMode, cascade_authorship: CascadeAuthorship) -> String {
    format!("{label} = {}", alpha_value(alpha, cascade_authorship))
}

fn unit_line(label: &str, unit: Unit, cascade_authorship: CascadeAuthorship) -> String {
    format!("{label} = {}", unit_value(unit, cascade_authorship))
}

fn alpha_value(alpha: AlphaMode, cascade_authorship: CascadeAuthorship) -> String {
    format!("{} ({})", alpha_label(alpha), cascade_authorship.label())
}

fn unit_value(unit: Unit, cascade_authorship: CascadeAuthorship) -> String {
    format!("{} ({})", unit_label(unit), cascade_authorship.label())
}

const fn alpha_label(alpha: AlphaMode) -> &'static str {
    match alpha {
        AlphaMode::Opaque => "Opaque",
        AlphaMode::Mask(_) => "Mask",
        AlphaMode::Blend => "Blend",
        AlphaMode::Premultiplied => "Premultiplied",
        AlphaMode::Add => "Add",
        AlphaMode::Multiply => "Multiply",
        AlphaMode::AlphaToCoverage => "AlphaToCoverage",
    }
}

const fn unit_label(unit: Unit) -> &'static str {
    match unit {
        Unit::Meters => "Meters",
        Unit::Millimeters => "Millimeters",
        Unit::Points => "Points",
        Unit::Pixels => "Pixels",
        Unit::Inches => "Inches",
        Unit::Custom(_) => "Custom",
    }
}
