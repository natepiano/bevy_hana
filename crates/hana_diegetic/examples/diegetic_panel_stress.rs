//! Panel stress test — add/remove rows to measure `DiegeticPanel` tree-build and
//! `set_tree` churn cost as panel content grows.
//!
//! The companion `diegetic_text_stress` example profiles the other axis — the
//! per-frame `DiegeticTextMut` / `TextContent` write path on standalone labels.
//! This one drives the panel/tree-rebuild path: each row-count change rebuilds
//! the active panel's tree.
//!
//! Press '+' to add rows, '-' to remove (hold for accelerating repeat).
//! Performance stats shown via `DiegeticPanel` overlays.
//!
//! Rows fill columns left-to-right within a panel. When the panel reaches
//! screen width, it pushes backward and a new panel spawns in front.
//!
//! Each row takes a fixed color from a rainbow spread across the panel's
//! rows, set once when the tree is built.

use std::collections::VecDeque;
use std::time::Instant;

use bevy::diagnostic::DiagnosticsStore;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::AlphaMode;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Plane3d;
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
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::prelude::error;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::OrbitCamPose;
use fairy_dust::TitleBar;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::Border;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_diegetic::default_panel_material;
use hana_kana::ToF32;
use hana_kana::ToUsize;
use hana_lagrange::OrbitCamPreset;

// ── Text / layout constants (meters) ─────────────────────────────────────────

const ROW_HEIGHT: f32 = 0.07;
/// Font size for content panel text, in millimeters (matched to `font_unit`).
///
/// Coupled to `ROW_HEIGHT` (a layout-meters value) so glyphs fill most of the
/// row they sit in rather than rendering sub-pixel: `0.07 m` → `70 mm` row,
/// `* 0.6` → a `42 mm` cap height that reads cleanly at the framing distance.
const FONT_SIZE: f32 = ROW_HEIGHT * 1000.0 * 0.6;
const ROW_SPACING: f32 = 0.05;
const COLUMN_GAP: f32 = 0.05;
const HEADER_HEIGHT: f32 = ROW_HEIGHT + 0.01;

/// Column width in meters.
///
/// This width is applied, not approximated: panel width is budgeted from it via
/// `MAX_LAYOUT_WIDTH`, and each column is sized to this width in the layout tree.
const COLUMN_WIDTH: f32 = 1.0;
/// Layout height per panel (meters).
const LAYOUT_HEIGHT: f32 = 2.0;
/// Padding on the outer panel in meters.
const PANEL_PADDING: f32 = 0.06;

// ── Scene constants ──────────────────────────────────────────────────────────

/// How many columns per panel.
const MAX_COLUMNS: usize = 8;
/// Max layout width — exactly fits `MAX_COLUMNS` with gaps and padding (meters).
const MAX_COLUMNS_F32: f32 = 8.0;
const MAX_COLUMN_GAPS_F32: f32 = MAX_COLUMNS_F32 - 1.0;
const MAX_LAYOUT_WIDTH: f32 =
    COLUMN_WIDTH * MAX_COLUMNS_F32 + COLUMN_GAP * MAX_COLUMN_GAPS_F32 + PANEL_PADDING * 2.0;
/// Ground plane size — same as panel width (layout is already in meters).
const GROUND_SIZE: f32 = MAX_LAYOUT_WIDTH;
/// Distance between stacked panels along Z (on the ground plane).
const STACK_DEPTH: f32 = 1.25;

// ── Key repeat ───────────────────────────────────────────────────────────────

/// Rows added per frame during animated column fill.
const ROWS_PER_FRAME: usize = 20;
const FPS_UPDATE_INTERVAL: f32 = 1.0;
const PERF_PEAK_WINDOW_SECS: f32 = 5.0;

// ── Colors ───────────────────────────────────────────────────────────────────

const BORDER_COLOR: Color = Color::srgb(0.39, 0.43, 0.47);
const DIVIDER_COLOR: Color = Color::srgb(0.235, 0.51, 0.706);

// ── Diagnostic overlay (screen-space) constants ───────────────────────────────

/// Font size for the diagnostic readout text (pixels).
const OVERLAY_FONT_SIZE: f32 = 13.0;

/// Value column color (the live numbers).
const STATUS_TEXT_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.9);

/// Label / header column color (dimmer than the values).
const STATUS_LABEL_COLOR: Color = Color::srgba(0.7, 0.78, 0.92, 0.85);

/// Metric-label column width, in pixels.
const LABEL_COLUMN_WIDTH: f32 = 56.0;
/// Numeric value column width, in pixels (right-aligned cells).
const VALUE_COLUMN_WIDTH: f32 = 60.0;
/// Gap between table columns, in pixels.
const TABLE_COL_GAP: f32 = 8.0;
/// Gap between table rows, in pixels.
const TABLE_ROW_GAP: f32 = 2.0;

/// Row labels for the diagnostic table, in display order.
const METRIC_LABELS: [&str; 6] = ["fps", "ms", "upd", "tree", "layout", "text"];

/// Placeholder values shown before the first perf sample arrives.
const INITIAL_METRICS: [&str; 6] = ["--", "--", "--", "--", "--", "--"];

/// Source text for row values.
const SOURCE_TEXT: &str = "bevy diegetic layout engine text rendering msdf atlas glyph quad mesh \
    shader pipeline parley shaping font registry plugin system resource component query transform \
    camera projection orthographic perspective viewport world entity spawn despawn commands insert \
    remove compute propagate sizing grow fit fixed percent padding border direction align children \
    element builder tree result render command rectangle scissor culling material texture sampler \
    uniform binding vertex index normal color alpha blend mask discard fragment screen pixel range \
    distance median clamp smooth antialiasing kerning advance baseline ascent descent bearing \
    rasterize bitmap canonical prepopulate allocator shelf etagere unicode bidi cluster glyph";

// ── Resources / components ───────────────────────────────────────────────────

#[derive(Resource, Default)]
struct StressControls {
    row_count:        usize,
    /// Target row count — the count `row_count` animates toward.
    /// When `target > row_count`, rows are added at [`ROWS_PER_FRAME`].
    /// When `target < row_count`, rows are removed at [`ROWS_PER_FRAME`].
    target_row_count: usize,
}

#[derive(Component)]
struct StressPanel(usize);

#[derive(Component)]
struct GroundPlane;

/// Marker for the combined status overlay panel (FPS + row/panel counts).
#[derive(Component)]
struct StatusPanel;

#[derive(Resource, Default)]
struct StressPerfStats {
    panel_update_ms: f32,
    tree_build_ms:   f32,
    tree_builds:     usize,
    panel_count:     usize,
}

#[derive(Clone, Copy)]
struct PerfSnapshot {
    timestamp: f32,
    fps:       f32,
    frame_ms:  f32,
    update_ms: f32,
    tree_ms:   f32,
    layout_ms: f32,
    text_ms:   f32,
}

/// The last displayed status values, so the overlay tree is rebuilt only when a
/// shown value changes.
#[derive(Resource, Default)]
struct LastDisplayedStatus {
    text: String,
}

// ── App ──────────────────────────────────────────────────────────────────────

fn main() {
    // `hana_diegetic::DiegeticUiPlugin` is registered automatically by
    // `fairy_dust::sprinkle_example`.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_perf_mode()
        .with_save_window_position()
        .with_studio_lighting()
        .with_orbit_cam_preset_pose(
            OrbitCamPose {
                focus:  Vec3::new(0.0, 1.0, GROUND_SIZE * 0.25),
                yaw:    0.0,
                pitch:  0.35,
                radius: 8.0,
            },
            OrbitCamPreset::blender_like(),
        )
        .with_stable_transparency()
        .with_camera_home()
        .yaw(0.0)
        .pitch(0.35)
        .with_title_bar(
            TitleBar::new()
                .with_title("Panel Stress")
                .with_anchor(Anchor::TopLeft)
                .control("+ Add")
                .control("- Remove"),
        )
        .with_camera_control_panel()
        .init_resource::<StressControls>()
        .init_resource::<StressPerfStats>()
        .init_resource::<LastDisplayedStatus>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                handle_input,
                animate_row_count,
                update_status_panel,
                update_panels,
                resize_ground_plane,
            )
                .chain(),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Ground plane. Resized per-frame as panels stack backward (see
    // `resize_ground_plane`), so it is spawned manually rather than via
    // `.with_ground_plane()`.
    commands.spawn((
        GroundPlane,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(GROUND_SIZE, STACK_DEPTH))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.08, 0.08, 0.08, 0.5),
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
    ));

    spawn_status_overlay(&mut commands, &mut materials);
}

/// Spawns the diagnostic readout — a bottom-left screen-space overlay built with
/// the canonical Fairy Dust panel frame. Rebuilt each second by
/// [`update_status_panel`] (top-left is the title bar, bottom-right the camera
/// control panel, so the readout sits bottom-left).
fn spawn_status_overlay(commands: &mut Commands, materials: &mut Assets<StandardMaterial>) {
    let unlit = materials.add(screen_panel_material());
    let built = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_status_overlay_tree(
            &INITIAL_METRICS.map(String::from),
            &INITIAL_METRICS.map(String::from),
            0,
            0,
        ))
        .build();
    match built {
        Ok(built) => {
            commands.spawn((StatusPanel, built, Transform::default()));
        },
        Err(error) => error!("diegetic_panel_stress: failed to build status overlay: {error}"),
    }
}

/// Text style for the diagnostic table's dimmer label / header column.
fn status_label_style() -> TextStyle {
    TextStyle::new(OVERLAY_FONT_SIZE)
        .with_color(STATUS_LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

/// Text style for the diagnostic table's live value columns.
fn status_value_style() -> TextStyle {
    TextStyle::new(OVERLAY_FONT_SIZE)
        .with_color(STATUS_TEXT_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

/// A fixed-width left-aligned cell — used for the metric-label column.
fn label_cell(builder: &mut LayoutBuilder, text: &str) {
    builder.with(
        El::new()
            .width(Sizing::fixed(LABEL_COLUMN_WIDTH))
            .height(Sizing::FIT)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text((text, status_label_style()));
        },
    );
}

/// Whether a value cell renders with the dimmer label color (headers, footer)
/// or the brighter live-value color.
#[derive(Clone, Copy)]
enum CellEmphasis {
    Normal,
    Dim,
}

/// A fixed-width right-aligned cell — used for the numeric value columns, so the
/// digits line up in a proportional font without space-padding.
fn value_cell(builder: &mut LayoutBuilder, text: &str, emphasis: CellEmphasis) {
    let style = match emphasis {
        CellEmphasis::Dim => status_label_style(),
        CellEmphasis::Normal => status_value_style(),
    };
    builder.with(
        El::new()
            .width(Sizing::fixed(VALUE_COLUMN_WIDTH))
            .height(Sizing::FIT)
            .alignment(AlignX::Right, AlignY::Center),
        |builder| {
            builder.text((text, style));
        },
    );
}

/// One table row: a left-aligned label cell and two right-aligned value cells.
fn table_row(
    builder: &mut LayoutBuilder,
    label: &str,
    now: &str,
    max: &str,
    emphasis: CellEmphasis,
) {
    builder.with(
        El::row()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(TABLE_COL_GAP)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            label_cell(builder, label);
            value_cell(builder, now, emphasis);
            value_cell(builder, max, emphasis);
        },
    );
}

/// Builds the diagnostic table inside the canonical panel frame: a `now` column
/// and a `5s max` column of right-aligned numerics, one row per metric, with a
/// panels/rows footer.
fn build_status_overlay_tree(
    now: &[String; 6],
    max: &[String; 6],
    panels: usize,
    rows: usize,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::FIT,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(TABLE_ROW_GAP),
                |builder| {
                    table_row(builder, "", "now", "5s max", CellEmphasis::Dim);
                    for index in 0..METRIC_LABELS.len() {
                        table_row(
                            builder,
                            METRIC_LABELS[index],
                            &now[index],
                            &max[index],
                            CellEmphasis::Normal,
                        );
                    }
                    table_row(
                        builder,
                        "panels",
                        &panels.to_string(),
                        &rows.to_string(),
                        CellEmphasis::Dim,
                    );
                },
            );
        },
    );
    builder.build()
}

// ── Input ────────────────────────────────────────────────────────────────────

/// Sets the target row count. Each key press/hold advances the target by
/// one column's worth of rows. The actual `row_count` animates toward the
/// target at [`ROWS_PER_FRAME`] rows per frame.
fn handle_input(keyboard: Res<ButtonInput<KeyCode>>, mut state: ResMut<StressControls>) {
    let adding = keyboard.pressed(KeyCode::Equal);
    let removing = keyboard.pressed(KeyCode::Minus);

    if !adding && !removing {
        // Stop animation when key is released.
        state.target_row_count = state.row_count;
        return;
    }

    let rpc = rows_per_column(state.target_row_count == 0);

    if adding {
        state.target_row_count += rpc;
    }
    if removing {
        state.target_row_count = state.target_row_count.saturating_sub(rpc);
    }
}

/// Animates `row_count` toward `target_row_count` at [`ROWS_PER_FRAME`].
fn animate_row_count(mut state: ResMut<StressControls>) {
    if state.row_count < state.target_row_count {
        let step = ROWS_PER_FRAME.min(state.target_row_count - state.row_count);
        state.row_count += step;
    } else if state.row_count > state.target_row_count {
        let step = ROWS_PER_FRAME.min(state.row_count - state.target_row_count);
        state.row_count -= step;
    }
}

// ── Status panel ─────────────────────────────────────────────────────────────

fn update_status_panel(
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    state: Res<StressControls>,
    stress_perf: Res<StressPerfStats>,
    diegetic_perf: Res<DiegeticPerfStats>,
    panels: Query<Entity, With<StatusPanel>>,
    mut last_displayed: ResMut<LastDisplayedStatus>,
    mut commands: Commands,
    mut timer: Local<Option<Timer>>,
    mut history: Local<VecDeque<PerfSnapshot>>,
) {
    let timer =
        timer.get_or_insert_with(|| Timer::from_seconds(FPS_UPDATE_INTERVAL, TimerMode::Repeating));
    timer.tick(time.delta());
    if !timer.just_finished() {
        return;
    }
    let frames_per_second = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(bevy::diagnostic::Diagnostic::smoothed);
    let frame_milliseconds = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(bevy::diagnostic::Diagnostic::smoothed);
    let frames_per_second_string =
        frames_per_second.map_or_else(|| "--".to_string(), |value| format!("{value:.0}"));
    let frames_per_second_value = frames_per_second.unwrap_or(0.0).to_f32();
    let frame_milliseconds_value = frame_milliseconds.unwrap_or(0.0).to_f32();
    let frame_milliseconds_string =
        frame_milliseconds.map_or_else(|| "--".to_string(), |value| format!("{value:.1}"));

    history.push_back(PerfSnapshot {
        timestamp: time.elapsed_secs(),
        fps:       frames_per_second_value,
        frame_ms:  frame_milliseconds_value,
        update_ms: stress_perf.panel_update_ms,
        tree_ms:   stress_perf.tree_build_ms,
        layout_ms: diegetic_perf.compute_ms,
        text_ms:   diegetic_perf.panel_text.total_ms,
    });

    let cutoff = time.elapsed_secs() - PERF_PEAK_WINDOW_SECS;
    while history
        .front()
        .is_some_and(|sample| sample.timestamp < cutoff)
    {
        history.pop_front();
    }

    let mut maximum_frames_per_second = 0.0_f32;
    let mut maximum_frame_milliseconds = 0.0_f32;
    let mut maximum_update_milliseconds = 0.0_f32;
    let mut maximum_tree_milliseconds = 0.0_f32;
    let mut maximum_layout_milliseconds = 0.0_f32;
    let mut maximum_text_milliseconds = 0.0_f32;
    for sample in &history {
        maximum_frames_per_second = maximum_frames_per_second.max(sample.fps);
        maximum_frame_milliseconds = maximum_frame_milliseconds.max(sample.frame_ms);
        maximum_update_milliseconds = maximum_update_milliseconds.max(sample.update_ms);
        maximum_tree_milliseconds = maximum_tree_milliseconds.max(sample.tree_ms);
        maximum_layout_milliseconds = maximum_layout_milliseconds.max(sample.layout_ms);
        maximum_text_milliseconds = maximum_text_milliseconds.max(sample.text_ms);
    }

    let now = [
        frames_per_second_string,
        frame_milliseconds_string,
        format!("{:.1}", stress_perf.panel_update_ms),
        format!("{:.1}", stress_perf.tree_build_ms),
        format!("{:.1}", diegetic_perf.compute_ms),
        format!("{:.1}", diegetic_perf.panel_text.total_ms),
    ];
    let max = [
        format!("{maximum_frames_per_second:.0}"),
        format!("{maximum_frame_milliseconds:.1}"),
        format!("{maximum_update_milliseconds:.1}"),
        format!("{maximum_tree_milliseconds:.1}"),
        format!("{maximum_layout_milliseconds:.1}"),
        format!("{maximum_text_milliseconds:.1}"),
    ];
    let panels_count = stress_perf.panel_count;
    let rows = state.row_count;

    // Single-string key so the tree is only rebuilt when a displayed value
    // actually changes.
    let key = format!("{}|{}|{panels_count}|{rows}", now.join(","), max.join(","));
    if key != last_displayed.text {
        last_displayed.text.clone_from(&key);
        for entity in &panels {
            if let Err(error) = commands.set_tree(
                entity,
                build_status_overlay_tree(&now, &max, panels_count, rows),
            ) {
                error!("failed to replace stress status panel tree: {error}");
            }
        }
    }
}

// ── Panel management ─────────────────────────────────────────────────────────

/// Rows that fit in one column.
fn rows_per_column(is_header_column: bool) -> usize {
    let available = if is_header_column {
        LAYOUT_HEIGHT - HEADER_HEIGHT
    } else {
        LAYOUT_HEIGHT
    };
    let count = (available / ROW_HEIGHT).to_usize();
    count.max(1)
}

/// Rows that fit in one full panel (all `MAX_COLUMNS` columns).
fn rows_per_panel() -> usize {
    let first = rows_per_column(true);
    let other = rows_per_column(false);
    first + (MAX_COLUMNS - 1) * other
}

/// Transparent background-quad material for a stress panel, so the scene shows
/// through and only the borders + (PBR-lit) text read against the ground.
fn transparent_panel_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::NONE,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default_panel_material()
    }
}

fn update_panels(
    state: Res<StressControls>,
    existing: Query<(Entity, &StressPanel)>,
    mut panel_transforms: Query<&mut Transform>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut perf: ResMut<StressPerfStats>,
    mut last_panel_count: Local<usize>,
    mut last_row_count: Local<Option<usize>>,
) {
    if last_row_count.as_ref() == Some(&state.row_count) {
        return;
    }
    *last_row_count = Some(state.row_count);
    let update_start = Instant::now();
    let mut tree_build_ms = 0.0_f32;
    let mut tree_builds = 0_usize;

    let rpp = rows_per_panel();
    let words: Vec<&str> = SOURCE_TEXT.split_whitespace().collect();

    let needed = if state.row_count == 0 {
        1
    } else {
        state.row_count.div_ceil(rpp)
    };

    // Despawn excess.
    for (entity, sp) in &existing {
        if sp.0 >= needed {
            commands.entity(entity).despawn();
        }
    }

    let wh = LAYOUT_HEIGHT;

    // Spawn missing.
    for idx in *last_panel_count..needed {
        let tree_start = Instant::now();
        let tree = build_panel_tree(&state, idx, rpp, &words);
        tree_build_ms = tree_start
            .elapsed()
            .as_secs_f32()
            .mul_add(1000.0, tree_build_ms);
        tree_builds += 1;
        commands.spawn((
            StressPanel(idx),
            CameraHomeTarget,
            {
                let panel = DiegeticPanel::world()
                    .size(MAX_LAYOUT_WIDTH, LAYOUT_HEIGHT)
                    .font_unit(Unit::Millimeters)
                    .material(materials.add(transparent_panel_material()))
                    .text_material(materials.add(default_panel_material()))
                    .with_tree(tree)
                    .build();
                let Ok(panel) = panel else {
                    error!("failed to build stress panel dimensions");
                    return;
                };
                panel
            },
            panel_transform(idx, needed, wh),
        ));
    }
    *last_panel_count = needed;

    // Update existing panels.
    let active_panel_idx = needed - 1;
    let panel_count_changed = needed != existing.iter().count();

    for (entity, sp) in &existing {
        if sp.0 < needed
            && let Ok(mut transform) = panel_transforms.get_mut(entity)
        {
            if sp.0 == active_panel_idx {
                // Active panel — content changed, rebuild tree.
                let tree_start = Instant::now();
                if let Err(error) =
                    commands.set_tree(entity, build_panel_tree(&state, sp.0, rpp, &words))
                {
                    error!("failed to replace stress panel tree: {error}");
                }
                tree_build_ms = tree_start
                    .elapsed()
                    .as_secs_f32()
                    .mul_add(1000.0, tree_build_ms);
                tree_builds += 1;
            } else if panel_count_changed {
                // Panel count changed — rebuild frozen panels once so the
                // per-row rainbow respaces against the new row_count.
                if let Err(error) =
                    commands.set_tree(entity, build_panel_tree(&state, sp.0, rpp, &words))
                {
                    error!("failed to replace frozen stress panel tree: {error}");
                }
            }
            *transform = panel_transform(sp.0, needed, wh);
        }
    }

    perf.panel_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;
    perf.tree_build_ms = tree_build_ms;
    perf.tree_builds = tree_builds;
    perf.panel_count = needed;
}

/// Resizes and repositions the ground plane to cover all stacked panels.
fn resize_ground_plane(
    perf: Res<StressPerfStats>,
    mut ground: Query<(&mut Mesh3d, &mut Transform), With<GroundPlane>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut last_panel_count: Local<usize>,
) {
    if perf.panel_count == *last_panel_count {
        return;
    }
    *last_panel_count = perf.panel_count;

    // Shallow plane: just deep enough to reach one STACK_DEPTH behind the
    // last panel so shadows land on the ground.
    let depth = perf.panel_count.max(1).to_f32() * STACK_DEPTH;
    let width = GROUND_SIZE;

    for (mut mesh3d, mut transform) in &mut ground {
        mesh3d.0 = meshes.add(Plane3d::default().mesh().size(width, depth));
        // Center the plane under the panels. Front edge at z = GROUND_SIZE/2,
        // back edge at z = GROUND_SIZE/2 - depth.
        let center_z = GROUND_SIZE.mul_add(0.5, -(depth * 0.5));
        transform.translation.z = center_z;
    }
}

/// Panel position — aligned with the ground plane's X axis.
/// Panel left edge = plane left edge. Older panels pushed backward along Z.
fn panel_transform(panel_idx: usize, total: usize, world_height: f32) -> Transform {
    let depth_from_front = (total - 1 - panel_idx).to_f32();
    // Front panel at z=0 (forward edge of ground plane), older panels push back.
    let z = GROUND_SIZE.mul_add(0.5, -(depth_from_front * STACK_DEPTH));
    // Panel top-left edge aligns with plane left edge.
    let x = -GROUND_SIZE * 0.5;
    // Panel top sits above the ground (TopLeft anchor: y = top edge).
    let y = world_height + 0.3;
    Transform::from_xyz(x, y, z)
}

fn build_panel_tree(
    state: &StressControls,
    panel_idx: usize,
    rpp: usize,
    words: &[&str],
) -> hana_diegetic::LayoutTree {
    let panel_start = panel_idx * rpp;
    let panel_rows = state.row_count.saturating_sub(panel_start).min(rpp);
    let first_col_rows = rows_per_column(panel_idx == 0);
    let other_col_rows = rows_per_column(false);

    let cols = if panel_rows <= first_col_rows {
        1
    } else {
        let remaining = panel_rows - first_col_rows;
        1 + remaining.div_ceil(other_col_rows)
    };

    let mut builder = LayoutBuilder::new(MAX_LAYOUT_WIDTH, LAYOUT_HEIGHT);

    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .gap(COLUMN_GAP)
            .padding(Padding::all(0.03))
            .border(Border::all(0.01, BORDER_COLOR)),
        |b| {
            let mut row_cursor = panel_start;
            for col in 0..cols {
                let is_first = panel_idx == 0 && col == 0;
                let col_rows = if is_first {
                    first_col_rows
                } else {
                    other_col_rows
                };
                let end = (row_cursor + col_rows).min(panel_start + panel_rows);

                b.with(
                    El::column()
                        .width(Sizing::fixed(COLUMN_WIDTH))
                        .height(Sizing::GROW)
                        .gap(0.01)
                        .padding(Padding::all(0.02))
                        .border(Border::all(0.01, BORDER_COLOR)),
                    |b| {
                        if is_first {
                            b.text((
                                "'+' add  '-' remove",
                                TextStyle::new(FONT_SIZE).with_shadow_mode(GlyphShadowMode::None),
                            ));
                            b.with(
                                El::new()
                                    .width(Sizing::GROW)
                                    .height(Sizing::fixed(0.01))
                                    .background(DIVIDER_COLOR),
                                |_| {},
                            );
                        }

                        for i in row_cursor..end {
                            let hue = if state.row_count > 0 {
                                360.0 * (i.to_f32() / state.row_count.to_f32())
                            } else {
                                0.0
                            };
                            let color = Color::hsl(hue, 1.0, 0.7);
                            let label = format!("item {i}:");
                            let value = words[i % words.len()];
                            let config = TextStyle::new(FONT_SIZE)
                                .with_color(color)
                                .with_shadow_mode(GlyphShadowMode::None);
                            b.with(
                                El::row()
                                    .width(Sizing::GROW)
                                    .height(Sizing::FIT)
                                    .gap(ROW_SPACING),
                                |b| {
                                    b.text((&label, config.clone()));
                                    b.with(
                                        El::new().width(Sizing::GROW).height(Sizing::fixed(0.01)),
                                        |_| {},
                                    );
                                    b.text((value, config));
                                },
                            );
                        }
                        row_cursor = end;
                    },
                );
            }
        },
    );

    builder.build()
}
