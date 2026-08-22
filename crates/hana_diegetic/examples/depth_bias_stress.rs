//! Visual probe for inter-panel hardware depth-bias regressions.
//!
//! The rear blue panel owns many filler draw commands before its final blue
//! probe rectangle. The front red panel is physically closer to the camera. If
//! the blue probe draws over the red panel at small positive offsets, the
//! hardware `StandardMaterial::depth_bias` path is overpowering real panel
//! depth.
//!
//! This protects the `DrawZIndexRank` depth-bias model: the blue probe's
//! `DrawOrderIndex` grows with the filler count, but its material `depth_bias`
//! should only be its dense z-index rank. The red panel should stay in front
//! for every filler count when its physical z offset is positive.
//!
//! This does not stress OIT. The probe uses opaque panels so it isolates Bevy's
//! hardware `StandardMaterial::depth_bias` path from the separate OIT
//! `OitDepthOffset` path.

use bevy::prelude::Alpha;
use bevy::prelude::AlphaMode;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::KeyCode;
use bevy::prelude::Name;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::prelude::info;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::DescriptionPanel;
use fairy_dust::LABEL_SIZE;
use fairy_dust::OrbitCamPose;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::PanelBuildError;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_lagrange::OrbitCamPreset;

const HOME_FOCUS: Vec3 = Vec3::new(0.0, PANEL_CENTER_Y, 0.0);
const HOME_MARGIN: f32 = 0.20;
const HOME_PITCH: f32 = 0.0;
const HOME_RADIUS: f32 = 3.0;
const HOME_YAW: f32 = 0.0;

const REAR_PANEL_WIDTH_M: f32 = 2.4;
const REAR_PANEL_HEIGHT_M: f32 = 1.45;
const FRONT_PANEL_WIDTH_M: f32 = 0.92;
const FRONT_PANEL_HEIGHT_M: f32 = 0.64;
const FRONT_PANEL_X: f32 = 0.0;
const FRONT_PANEL_Y: f32 = PANEL_CENTER_Y;
const GROUND_CLEARANCE_M: f32 = 0.08;
const PANEL_CENTER_Y: f32 = GROUND_CLEARANCE_M + REAR_PANEL_HEIGHT_M * 0.5;
const REAR_PANEL_Z: f32 = 0.0;

/// Rear-panel filler command counts.
///
/// These grow the blue probe's `DrawOrderIndex` without adding more authored
/// `DrawZIndex` values, so the blue probe's `DrawZIndexRank` stays stable.
const FILLER_COUNTS: [usize; 9] = [0, 32, 64, 128, 256, 512, 1024, 2048, 4096];
/// Physical z offsets for the red panel, in meters.
const FRONT_OFFSETS: [f32; 5] = [0.0, 0.000_01, 0.000_1, 0.001, 0.01];
/// Starting `FILLER_COUNTS` index: 64 filler commands.
const DEFAULT_FILLER_COUNT_INDEX: usize = 2;
/// Starting `FRONT_OFFSETS` index: a 1e-4m front-panel offset.
const DEFAULT_FRONT_OFFSET_INDEX: usize = 2;

const FILLER_COLOR: Color = Color::srgba(0.08, 0.11, 0.16, 0.035);
const FILLER_SIZE_M: f32 = 0.006;
const FRONT_COLOR: Color = Color::srgb(0.95, 0.08, 0.05);
const REAR_PROBE_COLOR: Color = Color::srgb(0.05, 0.32, 1.0);

const PANEL_RADIUS_M: f32 = 0.025;
const STATUS_FONT_SIZE: f32 = 13.0;
const STATUS_LABEL_COLOR: Color = Color::srgba(0.68, 0.75, 0.88, 0.92);
const STATUS_VALUE_COLOR: Color = Color::srgba(0.93, 0.98, 1.0, 0.98);
const STATUS_WARNING_COLOR: Color = Color::srgb(1.0, 0.45, 0.34);
const STATUS_LABEL_WIDTH: f32 = 118.0;
const STATUS_VALUE_WIDTH: f32 = 92.0;
const STATUS_COL_GAP: f32 = 10.0;
const STATUS_ROW_GAP: f32 = 3.0;
const DESCRIPTION_HEADING: &str = "Depth Bias Stress";
const DESCRIPTION_LINES: [&str; 5] = [
    "Red should stay in front.",
    "1-9: rear command count.",
    "[ ]: front z offset.",
    "OIT is off.",
    "Blue over red = bug.",
];

const COUNT_0_CONTROL: &str = "count-0";
const COUNT_32_CONTROL: &str = "count-32";
const COUNT_64_CONTROL: &str = "count-64";
const COUNT_128_CONTROL: &str = "count-128";
const COUNT_256_CONTROL: &str = "count-256";
const COUNT_512_CONTROL: &str = "count-512";
const COUNT_1024_CONTROL: &str = "count-1024";
const COUNT_2048_CONTROL: &str = "count-2048";
const COUNT_4096_CONTROL: &str = "count-4096";
const OFFSET_DOWN_CONTROL: &str = "offset-down";
const OFFSET_UP_CONTROL: &str = "offset-up";

#[derive(Component)]
struct RearPanel;

#[derive(Component)]
struct FrontPanel;

#[derive(Component)]
struct StatusPanel;

#[derive(Resource, Clone, Copy, PartialEq, Eq)]
struct StressState {
    filler_count_index: usize,
    front_offset_index: usize,
}

impl Default for StressState {
    fn default() -> Self {
        Self {
            filler_count_index: DEFAULT_FILLER_COUNT_INDEX,
            front_offset_index: DEFAULT_FRONT_OFFSET_INDEX,
        }
    }
}

impl StressState {
    const fn filler_count(self) -> usize { FILLER_COUNTS[self.filler_count_index] }

    const fn front_offset(self) -> f32 { FRONT_OFFSETS[self.front_offset_index] }

    /// `DrawOrderIndex` of the final blue probe rectangle.
    const fn rear_probe_index(self) -> usize { self.filler_count() }

    /// Dense rank of the blue probe's authored z-index inside the rear panel.
    fn rear_probe_z_index_rank(self) -> usize { usize::from(self.filler_count() != 0) }

    /// Expected material `depth_bias` for the blue probe.
    ///
    /// This mirrors `DrawZIndexRank::screen_depth_bias()` with
    /// `LAYER_DEPTH_BIAS = 1.0`.
    fn rear_probe_depth_bias(self) -> usize { self.rear_probe_z_index_rank() }

    fn set_filler_count_index(&mut self, index: usize) {
        self.filler_count_index = index.min(FILLER_COUNTS.len() - 1);
    }

    fn nudge_offset(&mut self, delta: isize) {
        self.front_offset_index = self
            .front_offset_index
            .saturating_add_signed(delta)
            .min(FRONT_OFFSETS.len() - 1);
    }
}

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_orbit_cam_preset_pose(
            OrbitCamPose {
                focus:  HOME_FOCUS,
                yaw:    HOME_YAW,
                pitch:  HOME_PITCH,
                radius: HOME_RADIUS,
            },
            OrbitCamPreset::blender_like(),
        )
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(title_bar())
        .wire_chip_to_state::<StressState, _>(COUNT_0_CONTROL, |state| {
            count_chip_activation(*state, 0)
        })
        .wire_chip_to_state::<StressState, _>(COUNT_32_CONTROL, |state| {
            count_chip_activation(*state, 1)
        })
        .wire_chip_to_state::<StressState, _>(COUNT_64_CONTROL, |state| {
            count_chip_activation(*state, 2)
        })
        .wire_chip_to_state::<StressState, _>(COUNT_128_CONTROL, |state| {
            count_chip_activation(*state, 3)
        })
        .wire_chip_to_state::<StressState, _>(COUNT_256_CONTROL, |state| {
            count_chip_activation(*state, 4)
        })
        .wire_chip_to_state::<StressState, _>(COUNT_512_CONTROL, |state| {
            count_chip_activation(*state, 5)
        })
        .wire_chip_to_state::<StressState, _>(COUNT_1024_CONTROL, |state| {
            count_chip_activation(*state, 6)
        })
        .wire_chip_to_state::<StressState, _>(COUNT_2048_CONTROL, |state| {
            count_chip_activation(*state, 7)
        })
        .wire_chip_to_state::<StressState, _>(COUNT_4096_CONTROL, |state| {
            count_chip_activation(*state, 8)
        })
        .with_description_panel(description_panel())
        .with_camera_control_panel()
        .init_resource::<StressState>()
        .add_systems(Startup, setup)
        .add_systems(Update, refresh_scene)
        .with_shortcut(KeyCode::Digit1, select_filler_count::<0>)
        .with_shortcut(KeyCode::Digit2, select_filler_count::<1>)
        .with_shortcut(KeyCode::Digit3, select_filler_count::<2>)
        .with_shortcut(KeyCode::Digit4, select_filler_count::<3>)
        .with_shortcut(KeyCode::Digit5, select_filler_count::<4>)
        .with_shortcut(KeyCode::Digit6, select_filler_count::<5>)
        .with_shortcut(KeyCode::Digit7, select_filler_count::<6>)
        .with_shortcut(KeyCode::Digit8, select_filler_count::<7>)
        .with_shortcut(KeyCode::Digit9, select_filler_count::<8>)
        .with_shortcut(KeyCode::BracketLeft, decrease_front_offset)
        .with_shortcut(KeyCode::BracketRight, increase_front_offset)
        .run();
}

fn title_bar() -> TitleBar {
    TitleBar::new()
        .with_title("Depth Bias Stress")
        .with_background_color(DEFAULT_PANEL_BACKGROUND.with_alpha(0.88))
        .control(TitleBarControl::segmented(
            "Filler",
            [
                TitleBarSegment::new(COUNT_0_CONTROL, "1 0"),
                TitleBarSegment::new(COUNT_32_CONTROL, "2 32"),
                TitleBarSegment::new(COUNT_64_CONTROL, "3 64"),
                TitleBarSegment::new(COUNT_128_CONTROL, "4 128"),
                TitleBarSegment::new(COUNT_256_CONTROL, "5 256"),
            ],
        ))
        .control(TitleBarControl::segmented(
            "Filler",
            [
                TitleBarSegment::new(COUNT_512_CONTROL, "6 512"),
                TitleBarSegment::new(COUNT_1024_CONTROL, "7 1024"),
                TitleBarSegment::new(COUNT_2048_CONTROL, "8 2048"),
                TitleBarSegment::new(COUNT_4096_CONTROL, "9 4096"),
            ],
        ))
        .control(TitleBarControl::segmented(
            "Front z",
            [
                TitleBarSegment::new(OFFSET_DOWN_CONTROL, "[ Less"),
                TitleBarSegment::new(OFFSET_UP_CONTROL, "] More"),
            ],
        ))
}

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_HEADING)
        .with_fit_width()
        .with_body_size(LABEL_SIZE.0)
        .lines(DESCRIPTION_LINES)
}

fn setup(
    mut commands: Commands,
    state: Res<StressState>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let material = materials.add(depth_probe_material());

    let Ok(rear_panel) = rear_panel(*state, material.clone()) else {
        error!("depth_bias_stress: failed to build rear panel");
        return;
    };
    commands.spawn((
        Name::new("Rear depth-bias stress panel"),
        CameraHomeTarget,
        RearPanel,
        rear_panel,
        Transform::from_translation(Vec3::new(0.0, PANEL_CENTER_Y, REAR_PANEL_Z)),
    ));

    let Ok(front_panel) = front_panel(material) else {
        error!("depth_bias_stress: failed to build front panel");
        return;
    };
    commands.spawn((
        Name::new("Front physical-depth reference panel"),
        CameraHomeTarget,
        FrontPanel,
        front_panel,
        Transform::from_translation(front_panel_translation(*state)),
    ));

    spawn_status_panel(&mut commands, &mut materials, *state);
    log_state("initial", *state);
}

fn refresh_scene(
    mut commands: Commands,
    state: Res<StressState>,
    rear_panel: Single<Entity, With<RearPanel>>,
    front_panel: Single<Entity, With<FrontPanel>>,
    status_panel: Single<Entity, With<StatusPanel>>,
) {
    if !state.is_changed() {
        return;
    }

    if let Err(error) = commands.set_tree(*rear_panel, rear_tree(state.filler_count())) {
        error!("failed to replace rear panel tree: {error}");
    }
    commands
        .entity(*front_panel)
        .insert(Transform::from_translation(front_panel_translation(*state)));
    if let Err(error) = commands.set_tree(*status_panel, status_tree(*state)) {
        error!("failed to replace status panel tree: {error}");
    }
    log_state("updated", *state);
}

fn spawn_status_panel(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    state: StressState,
) {
    let material = materials.add(screen_panel_material());
    let built = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::TopRight)
        .material(material.clone())
        .text_material(material)
        .with_tree(status_tree(state))
        .build();
    match built {
        Ok(panel) => {
            commands.spawn((
                Name::new("Depth bias stress live values"),
                StatusPanel,
                panel,
                Transform::default(),
            ));
        },
        Err(error) => error!("depth_bias_stress: failed to build status panel: {error}"),
    }
}

fn status_tree(state: StressState) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::FIT,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND.with_alpha(0.88),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(STATUS_ROW_GAP),
                |builder| {
                    builder.text(("live values", status_label_style()));
                    status_row(builder, "rear filler", &state.filler_count().to_string());
                    status_row(
                        builder,
                        "probe index",
                        &state.rear_probe_index().to_string(),
                    );
                    status_row(
                        builder,
                        "z-index rank",
                        &state.rear_probe_z_index_rank().to_string(),
                    );
                    status_row(
                        builder,
                        "depth bias",
                        &format!("{}.0", state.rear_probe_depth_bias()),
                    );
                    status_row(
                        builder,
                        "front z m",
                        &format!("{:.5}", state.front_offset()),
                    );
                    builder.text(("red should cover blue", status_warning_style()));
                },
            );
        },
    );
    builder.build()
}

fn status_row(builder: &mut LayoutBuilder, label: &str, value: &str) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(STATUS_COL_GAP)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.with(
                El::new()
                    .width(Sizing::fixed(STATUS_LABEL_WIDTH))
                    .height(Sizing::FIT)
                    .alignment(AlignX::Left, AlignY::Center),
                |builder| {
                    builder.text((label, status_label_style()));
                },
            );
            builder.with(
                El::new()
                    .width(Sizing::fixed(STATUS_VALUE_WIDTH))
                    .height(Sizing::FIT)
                    .alignment(AlignX::Right, AlignY::Center),
                |builder| {
                    builder.text((value, status_value_style()));
                },
            );
        },
    );
}

fn status_label_style() -> TextStyle {
    TextStyle::new(STATUS_FONT_SIZE)
        .with_color(STATUS_LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn status_value_style() -> TextStyle {
    TextStyle::new(STATUS_FONT_SIZE)
        .with_color(STATUS_VALUE_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn status_warning_style() -> TextStyle {
    TextStyle::new(STATUS_FONT_SIZE)
        .with_color(STATUS_WARNING_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn depth_probe_material() -> StandardMaterial {
    StandardMaterial {
        alpha_mode: AlphaMode::Opaque,
        unlit: true,
        ..screen_panel_material()
    }
}

fn rear_panel(
    state: StressState,
    material: Handle<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    DiegeticPanel::world()
        .size(REAR_PANEL_WIDTH_M, REAR_PANEL_HEIGHT_M)
        .anchor(Anchor::Center)
        .material(material)
        .with_tree(rear_tree(state.filler_count()))
        .build()
}

fn front_panel(material: Handle<StandardMaterial>) -> Result<DiegeticPanel, PanelBuildError> {
    DiegeticPanel::world()
        .size(FRONT_PANEL_WIDTH_M, FRONT_PANEL_HEIGHT_M)
        .anchor(Anchor::Center)
        .material(material)
        .with_tree(front_tree())
        .build()
}

fn rear_tree(filler_count: usize) -> LayoutTree {
    let mut builder =
        LayoutBuilder::with_root(El::overlay().size(REAR_PANEL_WIDTH_M, REAR_PANEL_HEIGHT_M));

    // These tiny rectangles exist only to increase the probe's
    // `DrawOrderIndex`. They share `DrawZIndex(0)` and should not increase the
    // later probe's material `depth_bias`.
    for _ in 0..filler_count {
        builder.with(
            El::new()
                .width(Sizing::fixed(FILLER_SIZE_M))
                .height(Sizing::fixed(FILLER_SIZE_M))
                .background(FILLER_COLOR)
                .z_index(0),
            |_| {},
        );
    }

    // The probe is the only `DrawZIndex(1)` command. With any filler present,
    // it should have `DrawZIndexRank(1)` no matter how large `filler_count` is.
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .corner_radius(CornerRadius::all(PANEL_RADIUS_M))
            .background(REAR_PROBE_COLOR)
            .z_index(1),
        |_| {},
    );

    builder.build()
}

fn front_tree() -> LayoutTree {
    LayoutBuilder::with_root(
        El::new()
            .size(FRONT_PANEL_WIDTH_M, FRONT_PANEL_HEIGHT_M)
            .corner_radius(CornerRadius::all(PANEL_RADIUS_M))
            .background(FRONT_COLOR),
    )
    .build()
}

const fn front_panel_translation(state: StressState) -> Vec3 {
    Vec3::new(FRONT_PANEL_X, FRONT_PANEL_Y, state.front_offset())
}

fn select_filler_count<const INDEX: usize>(mut state: ResMut<StressState>) {
    state.set_filler_count_index(INDEX);
}

fn decrease_front_offset(mut state: ResMut<StressState>) { state.nudge_offset(-1); }

fn increase_front_offset(mut state: ResMut<StressState>) { state.nudge_offset(1); }

const fn count_chip_activation(state: StressState, index: usize) -> ControlActivation {
    if state.filler_count_index == index {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}

fn log_state(label: &str, state: StressState) {
    info!(
        "{label}: filler_commands={} front_offset_m={:.6}",
        state.filler_count(),
        state.front_offset()
    );
}
