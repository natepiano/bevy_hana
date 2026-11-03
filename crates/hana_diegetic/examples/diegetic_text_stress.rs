//! Text-write stress test — 100 standalone [`DiegeticText`] labels, each retext
//! every frame through the [`DiegeticTextMut`] write path.
//!
//! This is the canonical subject for the write-path half of the perf gate: a
//! 10×10 grid of world-space labels, every one mutated per frame via
//! [`DiegeticTextMut::for_each_mut`], so a tree-authoritative write and its
//! relayout fire on all 100 labels each frame — the worst-case
//! `O(n_changed)` load. The companion `diegetic_panel_stress` example profiles
//! the other axis (panel tree-build / `set_tree` churn).
//!
//! Controls:
//!   Space — pause / resume per-frame mutation (compare moving vs idle cost);
//!     the title-bar `Pause` segment highlights while paused
//!   A — cycle the text anti-alias mode (Off → Anisotropic → Supersample →
//!     Both); the title bar highlights the active mode
//!   M — toggle the bottom-left GPU pipeline visualization; the title-bar
//!     `Pipeline` segment highlights while it is on
//!   O — toggle stable transparency / OIT; the title-bar `OIT` segment
//!     highlights while it is on
//!
//! A left screen overlay, anchored above the bottom-left GPU pipeline
//! visualization, reports `hana_metrics` frame and render spans alongside
//! the local `layout`, `reify`, `shaping`, and `mesh` spans. Each row has a
//! five-second peak column. The bottom bars illustrate durations; their GPU
//! lane is inferred and is not a GPU timer.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use bevy::camera::primitives::Aabb;
use bevy::diagnostic::Diagnostic;
use bevy::diagnostic::DiagnosticsStore;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::pbr::Shadow;
use bevy::prelude::Add;
use bevy::prelude::Alpha;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::On;
use bevy::prelude::Plugin;
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
use bevy::prelude::error;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSystems;
use bevy::render::render_phase::ViewBinnedRenderPhases;
use diagnostics::DrawCounts;
use diagnostics::StressDiagnosticsPlugin;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::OrbitCamPose;
use fairy_dust::StatsPanelRow;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarSegment;
use fairy_dust::TitleBarStateWriters;
use fairy_dust::diegetic_stats_panel_with_integral_advance;
use fairy_dust::diegetic_stats_tree_with_integral_advance;
use fairy_dust::fps_stats_panel;
use fairy_dust::gpu_meter_panel;
use fairy_dust::screen_panel_frame;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Anchor;
use hana_diegetic::AntiAlias;
use hana_diegetic::ComputedDiegeticPanel;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticTextMeasurer;
use hana_diegetic::DiegeticTextMut;
use hana_diegetic::El;
use hana_diegetic::FontRegistry;
use hana_diegetic::FontUnit;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::MeasureTextFn;
use hana_diegetic::Padding;
use hana_diegetic::PanelAnchorOffset;
use hana_diegetic::PanelAttachment;
use hana_diegetic::PanelEntityReader;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::Text;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_diegetic::TextWrap;
use hana_diegetic::Unit;
use hana_kana::Resolved;
use hana_kana::ToF32;
use hana_kana::ToU32;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamPreset;
use hana_metrics::FrameTiming;
use hana_metrics::FrameTimingPlugin;
use hana_metrics::SpanSample;
use hana_metrics::SpanTiming;

/// The stress graphs keep the old zero baseline until a span first measures.
const fn last_ms_or_zero(span: &SpanTiming) -> f32 {
    match span.sample {
        SpanSample::NeverMeasured => 0.0,
        SpanSample::Measured { last_ms, .. } => last_ms,
    }
}

// ── App — plugin wiring, resources, startup/update systems, shortcuts ────────

fn main() {
    // `fairy_dust::sprinkle_example` registers the diegetic UI plugin.
    // `with_brp_extras` brings in `FrameTimeDiagnosticsPlugin` (the overlay
    // reads its FPS / frame-time diagnostic IDs below); `with_perf_mode` uncaps
    // vsync and the unfocused winit throttle so the reported frame time
    // reflects true per-frame cost.
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_perf_mode()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .size(GROUND_SIZE)
        .with_orbit_cam_preset_pose(
            OrbitCamPose {
                focus:  GRID_FOCUS,
                yaw:    0.0,
                pitch:  0.18,
                radius: CAMERA_INITIAL_RADIUS,
            },
            OrbitCamPreset::blender_like(),
        )
        .with_stable_transparency()
        .with_camera_home()
        .yaw(0.0)
        .pitch(0.18)
        .with_title_bar(text_stress_title_bar())
        .wire_chip_to_state::<Mutating, _>(PAUSE_CHIP, |mutating| chip_activation(!mutating.0))
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
        .wire_chip_to_state::<GpuPipelineShown, _>(PIPELINE_CHIP, |shown| {
            chip_activation(matches!(*shown, GpuPipelineShown::Shown))
        })
        .wire_chip_to_state::<OitState, _>(OIT_CHIP, |oit| chip_activation(oit.0))
        .with_camera_control_panel()
        .add_plugins((FrameTimingPlugin, StressDiagnosticsPlugin))
        .init_resource::<FrameCounter>()
        .init_resource::<Mutating>()
        .init_resource::<OitState>()
        .init_resource::<GpuPipelineShown>()
        .init_resource::<LastDisplayedStatus>()
        .init_resource::<LastDisplayedBatchStats>()
        .add_observer(anchor_status_panel_when_gpu_pipeline_added)
        .add_observer(anchor_status_panel_when_status_panel_added)
        .add_systems(
            Startup,
            (
                spawn_labels,
                spawn_home_target,
                spawn_status_overlay,
                spawn_batch_stats_overlay,
                spawn_gpu_pipeline_overlay,
            ),
        )
        .add_systems(
            Update,
            (
                toggle_mutation.in_set(TitleBarStateWriters),
                advance_frame,
                mutate_labels,
                update_status_panel,
                update_batch_stats_panel,
                update_gpu_pipeline_panel,
            )
                .chain(),
        )
        // Modifier-guarded, so the Ctrl+Shift+A home-gizmo chord doesn't also
        // cycle the AA mode.
        .with_shortcut(KeyCode::KeyA, cycle_anti_alias)
        .with_shortcut(KeyCode::KeyM, toggle_gpu_pipeline)
        .with_shortcut(KeyCode::KeyO, toggle_oit)
        .run();
}

fn text_stress_title_bar() -> TitleBar {
    TitleBar::new()
        .with_title("Text Stress")
        .with_anchor(Anchor::TopLeft)
        .control(TitleBarControl::segmented(
            "Space",
            [TitleBarSegment::new(PAUSE_CHIP, "Pause")],
        ))
        .control(TitleBarControl::segmented(
            "A",
            AA_MODES.map(|(id, label, _)| TitleBarSegment::new(id, label)),
        ))
        .control(TitleBarControl::segmented(
            "M",
            [TitleBarSegment::new(PIPELINE_CHIP, "Pipeline")],
        ))
        .control(TitleBarControl::segmented(
            "O",
            [TitleBarSegment::new(OIT_CHIP, "OIT")],
        ))
}

// ── Text write path — DiegeticText labels retext each frame via DiegeticTextMut ───

// `spawn_labels` runs once at startup, spawning a GRID_SIDE × GRID_SIDE grid of
// standalone `DiegeticText` world labels, each tagged `StressLabel(index)`.
// Each frame `toggle_mutation` reads Space to flip
// `Mutating`, `advance_frame` bumps `FrameCounter`, and `mutate_labels` walks
// every label through `DiegeticTextMut::for_each_mut`, rewriting its string with
// `set_text` — a tree-authoritative write plus relayout on all 100 labels, the
// O(n_changed) worst case the perf gate targets.

/// Labels per side; `GRID_SIDE * GRID_SIDE` is the total label count.
const GRID_SIDE: usize = 10;
/// `GRID_SIDE` as `f32` for const layout math (kept as a literal so const
/// evaluation needs no `usize as f32` cast).
const GRID_SIDE_F32: f32 = 10.0;
const LABEL_COUNT: usize = GRID_SIDE * GRID_SIDE;
/// Horizontal / vertical spacing between label anchors (meters).
const CELL_SPACING: f32 = 0.55;
/// Label cap height (meters).
const LABEL_SIZE: f32 = 0.12;
/// Height of the grid's bottom row above the ground (meters).
const GRID_BASE_Y: f32 = 0.6;

const LABEL_COLOR: Color = Color::srgb(0.92, 0.92, 0.94);

const GRID_FOCUS: Vec3 = Vec3::new(
    0.0,
    GRID_BASE_Y + (GRID_SIDE_F32 - 1.0) * CELL_SPACING * 0.5,
    0.0,
);

/// Marks a stress label and carries its grid index, so `for_each_mut` can write
/// a distinct string per label.
#[derive(Component, Clone, Copy)]
struct StressLabel(usize);

/// Monotonic frame counter driving the per-label text; wrapped to three digits so
/// label widths stay bounded.
#[derive(Resource, Default)]
struct FrameCounter(u64);

/// Whether the per-frame mutation is running. Toggled with Space; the
/// title-bar `Pause` segment highlights while paused.
#[derive(Resource)]
struct Mutating(bool);

impl Default for Mutating {
    fn default() -> Self { Self(true) }
}

/// Spawns the `GRID_SIDE × GRID_SIDE` grid of standalone world labels, each a
/// [`DiegeticText`] carrying a [`StressLabel`] marker holding its grid index.
fn spawn_labels(mut commands: Commands) {
    let half = (GRID_SIDE.to_f32() - 1.0) * 0.5;
    for index in 0..LABEL_COUNT {
        let col = (index % GRID_SIDE).to_f32();
        let row = (index / GRID_SIDE).to_f32();
        let x = (col - half) * CELL_SPACING;
        let y = row.mul_add(CELL_SPACING, GRID_BASE_Y);
        commands.spawn((
            StressLabel(index),
            DiegeticText::world(label_text(index, 0))
                .size(LABEL_SIZE)
                .color(LABEL_COLOR)
                .transform(Transform::from_xyz(x, y, 0.0))
                .build(),
        ));
    }
}

fn toggle_mutation(keyboard: Res<ButtonInput<KeyCode>>, mut mutating: ResMut<Mutating>) {
    if keyboard.just_pressed(KeyCode::Space) {
        mutating.0 = !mutating.0;
    }
}

fn advance_frame(mutating: Res<Mutating>, mut frame: ResMut<FrameCounter>) {
    if mutating.0 {
        frame.0 = frame.0.wrapping_add(1);
    }
}

/// Retexts every label through the [`DiegeticTextMut`] write path.
/// [`DiegeticTextMut::for_each_mut`] yields each label's marker (its grid index)
/// and a `TextEdit` handle, so all 100 strings change in one pass — the
/// `O(n_changed)` worst case the gate targets.
fn mutate_labels(
    mutating: Res<Mutating>,
    frame: Res<FrameCounter>,
    mut labels: DiegeticTextMut<StressLabel>,
) {
    if !mutating.0 {
        return;
    }
    labels.for_each_mut(|label, edit| {
        edit.set_text(label_text(label.0, frame.0));
    });
}

/// The per-label string: a fixed two-digit index and the live three-digit frame
/// counter, so every label's text changes each frame while its width stays
/// stable.
fn label_text(index: usize, frame: u64) -> String { format!("{index:02} {:03}", frame % 1000) }

// ── Camera home & ground — home-frame target and ground-plane size ───────────

/// Keeps the label grid framed 10% farther away in the start and home views.
const CAMERA_DISTANCE_SCALE: f32 = 1.1;
const CAMERA_INITIAL_RADIUS: f32 = 8.5 * CAMERA_DISTANCE_SCALE;

/// World-space half-extents of the static camera-home region, centered on
/// [`GRID_FOCUS`]. Covers the label-anchor span (half the grid each way) plus a
/// few label heights so the glyphs sit inside the framed box, then scales the
/// box to match the farther starting camera radius. The home fit adds its own
/// screen-fraction margin on top.
const HOME_REGION_HALF_EXTENTS: Vec3 = Vec3::new(
    ((GRID_SIDE_F32 - 1.0) * CELL_SPACING * 0.5 + LABEL_SIZE * 3.0) * CAMERA_DISTANCE_SCALE,
    ((GRID_SIDE_F32 - 1.0) * CELL_SPACING * 0.5 + LABEL_SIZE * 1.5) * CAMERA_DISTANCE_SCALE,
    LABEL_SIZE * CAMERA_DISTANCE_SCALE,
);

const GROUND_SIZE: f32 = GRID_SIDE_F32 * CELL_SPACING + 1.0;

/// Spawns the single invisible [`CameraHomeTarget`] that `H` frames: an explicit
/// grid-sized [`Aabb`] at [`GRID_FOCUS`], no mesh, so the home union is one
/// constant box.
fn spawn_home_target(mut commands: Commands) {
    commands.spawn((
        CameraHomeTarget,
        Aabb::from_min_max(
            GRID_FOCUS - HOME_REGION_HALF_EXTENTS,
            GRID_FOCUS + HOME_REGION_HALF_EXTENTS,
        ),
        Transform::default(),
    ));
}

// ── Title-bar controls — pause, anti-alias cycle, pipeline and OIT toggles ───

/// Title-bar segment id for the pause indicator.
const PAUSE_CHIP: &str = "pause";

/// Title-bar segment id for the GPU pipeline indicator.
const PIPELINE_CHIP: &str = "pipeline";

/// Title-bar segment id for the stable-transparency / OIT indicator.
const OIT_CHIP: &str = "oit";

/// The in-shader [`AntiAlias`] modes in `A`-key cycle order: title-bar
/// segment id, visible label, and the mode itself. One source of truth for
/// the chips, the chip wiring, and the cycle step.
const AA_MODES: [(&str, &str, AntiAlias); 4] = [
    ("aa-off", "Off", AntiAlias::Off),
    ("aa-anisotropic", "Anisotropic", AntiAlias::Anisotropic),
    ("aa-supersample", "Supersample", AntiAlias::Supersample),
    ("aa-both", "Both", AntiAlias::Both),
];

/// Whether the main camera uses [`StableTransparency`].
#[derive(Resource)]
struct OitState(bool);

impl Default for OitState {
    fn default() -> Self { Self(true) }
}

/// Maps a boolean to the title-bar activation it represents.
const fn chip_activation(active: bool) -> ControlActivation {
    if active {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}

/// Advances [`AntiAlias`] one step through [`AA_MODES`], wrapping at the
/// end. The change propagates to every text material via the engine's
/// `sync_anti_alias` system, and to the title-bar chips via the
/// per-mode wiring in `main`.
fn cycle_anti_alias(mut anti_alias: ResMut<AntiAlias>) {
    let current = AA_MODES
        .iter()
        .position(|(_, _, mode)| *mode == *anti_alias)
        .unwrap_or(0);
    *anti_alias = AA_MODES[(current + 1) % AA_MODES.len()].2;
}

/// Toggles [`StableTransparency`] / OIT on the main scene camera.
fn toggle_oit(
    mut state: ResMut<OitState>,
    cameras: Query<Entity, With<OrbitCam>>,
    mut commands: Commands,
) {
    state.0 = !state.0;
    for camera in &cameras {
        if state.0 {
            commands.entity(camera).insert(StableTransparency);
        } else {
            commands.entity(camera).remove::<StableTransparency>();
        }
    }
}

// ── Measurement — StressDiagnosticsPlugin: main/render-thread timing & draw counts ──

mod diagnostics {
    use super::*;

    pub(super) struct StressDiagnosticsPlugin;

    impl Plugin for StressDiagnosticsPlugin {
        fn build(&self, app: &mut App) { app.add_plugins(DrawCountPlugin); }
    }

    // ── Measurement — shadow draw counts: per-view shadow phase items ────────────

    /// Latest per-frame phase-item counts, written on the render thread after
    /// `PhaseSort` and read by the overlay on the main thread. Path-agnostic — it
    /// counts whichever toggle state is active — so toggle-off vs toggle-on reads
    /// come from one session.
    #[derive(Default)]
    struct DrawCountBits {
        /// Shadow-phase caster draws summed across shadow views (batchable +
        /// unbatchable bins, plus one per multidrawable batch set).
        shadow:          AtomicU32,
        /// Caster draws per shadow view (one view per cascade per shadow-casting
        /// light), largest first — the components that sum to `shadow`.
        shadow_per_view: Mutex<Vec<u32>>,
    }

    /// Main-world handle to the shared [`DrawCountBits`].
    #[derive(Resource, Clone)]
    pub(super) struct DrawCounts(Arc<DrawCountBits>);

    impl DrawCounts {
        pub(super) fn shadow_items(&self) -> u32 { self.0.shadow.load(Ordering::Relaxed) }

        pub(super) fn shadow_per_view(&self) -> Vec<u32> {
            self.0
                .shadow_per_view
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        }
    }

    /// Counts per-view shadow phase items after `RenderSystems::PhaseSort` into
    /// the shared [`DrawCountBits`] the upper-right overlay reads.
    struct DrawCountPlugin;

    impl Plugin for DrawCountPlugin {
        fn build(&self, app: &mut App) {
            let shared = DrawCounts(Arc::new(DrawCountBits::default()));
            app.insert_resource(shared.clone());
            let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
                return;
            };
            render_app.insert_resource(shared);
            render_app.add_systems(
                Render,
                count_phase_items
                    .after(RenderSystems::PhaseSort)
                    .before(RenderSystems::Render),
            );
        }
    }

    fn count_phase_items(
        shadow_phases: Res<ViewBinnedRenderPhases<Shadow>>,
        counts: Res<DrawCounts>,
    ) {
        let mut shadow_per_view: Vec<u32> = shadow_phases
            .values()
            .map(|phase| {
                (phase
                    .batchable_meshes
                    .values()
                    .map(|bin| bin.entities().len())
                    .sum::<usize>()
                    + phase
                        .unbatchable_meshes
                        .values()
                        .map(|bin| bin.entities.len())
                        .sum::<usize>()
                    + phase.multidrawable_meshes.len())
                .to_u32()
            })
            .collect();
        // Phase-map iteration order is not stable; sort largest-first so the
        // on-screen breakdown stays in a fixed order frame to frame.
        shadow_per_view.sort_unstable_by(|a, b| b.cmp(a));

        let shadow: u32 = shadow_per_view.iter().sum();
        counts.0.shadow.store(shadow, Ordering::Relaxed);
        if let Ok(mut guard) = counts.0.shadow_per_view.lock() {
            *guard = shadow_per_view;
        }
    }
}

// ── Overlay — perf table: status panel and upper-right batch stats ───────────

/// Label/value text size for header-style rows in the left perf panel.
const PERF_HEADER_ROW_FONT_SIZE: f32 = 13.0;
/// Label/value text size for normal rows in the left perf panel.
const PERF_ROW_FONT_SIZE: f32 = 11.0;
const STATUS_TEXT_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.9);
const STATUS_LABEL_COLOR: Color = Color::srgba(0.7, 0.78, 0.92, 0.85);
/// Separator color shared by screen-panel frames, camera-panel dividers, and
/// metric panel section rules.
const PANEL_SEPARATOR_COLOR: Color = Color::srgba(0.1, 0.4, 0.6, 0.3);
/// Separator line thickness in logical pixels.
const PANEL_SEPARATOR_THICKNESS: f32 = 1.0;
/// Wide enough to contain the longest timeline label plus the sub-row indent
/// without leaving a large gap before the numeric columns.
const LABEL_COLUMN_WIDTH: f32 = 144.0;
/// Wide enough to contain the `5s max` header on one line.
const VALUE_COLUMN_WIDTH: f32 = 72.0;
const TABLE_COL_GAP: f32 = 8.0;
const TABLE_ROW_GAP: f32 = 2.0;
/// Full status table width, used by section titles and separators.
const STATUS_TABLE_WIDTH: f32 = LABEL_COLUMN_WIDTH + VALUE_COLUMN_WIDTH * 2.0 + TABLE_COL_GAP * 2.0;
/// Left padding on component-row labels, marking what sums into the block
/// header above.
const SUB_ROW_INDENT: f32 = 12.0;
const FPS_UPDATE_INTERVAL: f32 = 1.0;
const PERF_PEAK_WINDOW_SECS: f32 = 5.0;
/// Larger text for section labels in the left perf panel.
const STATS_HEADER_FONT_SIZE: f32 = 15.0;

/// Label hierarchy for the perf table.
#[derive(Clone, Copy)]
enum RowIndent {
    None,
    Phase,
    Detail,
}

impl RowIndent {
    const fn left_padding(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Phase => SUB_ROW_INDENT,
            Self::Detail => SUB_ROW_INDENT * 2.0,
        }
    }
}

#[derive(Clone, Copy)]
struct MetricRow {
    label:  &'static str,
    indent: RowIndent,
    accent: Option<Color>,
}

impl MetricRow {
    const fn new(label: &'static str, indent: RowIndent) -> Self {
        Self {
            label,
            indent,
            accent: None,
        }
    }

    const fn accented(label: &'static str, indent: RowIndent, accent: Color) -> Self {
        Self {
            label,
            indent,
            accent: Some(accent),
        }
    }
}

/// Diagnostic table rows, in display order. The local diegetic spans are
/// sampled separately from the latest complete `hana_metrics` frame. Render
/// work overlaps main work, so the two groups must not be added together.
const METRIC_ROWS: [MetricRow; 17] = [
    MetricRow::new("fps", RowIndent::None),
    MetricRow::new("ms/frame", RowIndent::None),
    MetricRow::accented("layout", RowIndent::Detail, GPU_PIPELINE_WORK_COLOR),
    MetricRow::accented("reify", RowIndent::Detail, GPU_PIPELINE_WORK_COLOR),
    MetricRow::accented("shaping", RowIndent::Detail, GPU_PIPELINE_WORK_COLOR),
    MetricRow::accented("mesh", RowIndent::Detail, GPU_PIPELINE_WORK_COLOR),
    MetricRow::accented("other", RowIndent::Detail, GPU_PIPELINE_WORK_COLOR),
    MetricRow::accented("wait for render", RowIndent::Phase, GPU_PIPELINE_WAIT_COLOR),
    MetricRow::accented("extract", RowIndent::Phase, GPU_PIPELINE_EXTRACT_COLOR),
    MetricRow::accented(
        "frame slack",
        RowIndent::Phase,
        GPU_PIPELINE_FRAME_SLACK_COLOR,
    ),
    MetricRow::new("render work", RowIndent::None),
    MetricRow::accented("assets", RowIndent::Detail, GPU_PIPELINE_WORK_COLOR),
    MetricRow::accented("prep", RowIndent::Detail, GPU_PIPELINE_WORK_COLOR),
    MetricRow::accented("next image wait", RowIndent::Phase, GPU_PIPELINE_WAIT_COLOR),
    MetricRow::accented(
        "render graph",
        RowIndent::Phase,
        GPU_PIPELINE_RENDER_GRAPH_COLOR,
    ),
    MetricRow::accented("submit", RowIndent::Detail, GPU_PIPELINE_GRAPH_SUBMIT_COLOR),
    MetricRow::accented("cleanup", RowIndent::Phase, GPU_PIPELINE_CLEANUP_COLOR),
];
const METRIC_COUNT: usize = METRIC_ROWS.len();
const FPS_METRIC_INDEX: usize = 0;
const MAIN_WORLD_METRIC_START: usize = 1;
const RENDER_WORLD_METRIC_START: usize = 10;
const INITIAL_METRICS: [&str; METRIC_COUNT] = ["--"; METRIC_COUNT];

/// Marker for the screen-space diagnostic overlay panel.
#[derive(Component)]
struct StatusPanel;

/// Marker for the upper-right glyph-batch stats panel, separate from the GPU
/// pipeline panel so its wide rows don't stretch it.
#[derive(Component)]
struct BatchStatsPanel;

#[derive(Clone, Copy)]
struct PerfSnapshot {
    timestamp:          f32,
    fps:                f32,
    frame_ms:           f32,
    layout_ms:          f32,
    reify_ms:           f32,
    shaping_ms:         f32,
    mesh_ms:            f32,
    other_ms:           f32,
    wait_for_render_ms: f32,
    extract_ms:         f32,
    frame_slack_ms:     f32,
    render_work_ms:     f32,
    assets_ms:          f32,
    prep_ms:            f32,
    wait_for_gpu_ms:    f32,
    render_graph_ms:    f32,
    graph_submit_ms:    f32,
    cleanup_ms:         f32,
}

impl PerfSnapshot {
    const ZERO: Self = Self {
        timestamp:          0.0,
        fps:                0.0,
        frame_ms:           0.0,
        layout_ms:          0.0,
        reify_ms:           0.0,
        shaping_ms:         0.0,
        mesh_ms:            0.0,
        other_ms:           0.0,
        wait_for_render_ms: 0.0,
        extract_ms:         0.0,
        frame_slack_ms:     0.0,
        render_work_ms:     0.0,
        assets_ms:          0.0,
        prep_ms:            0.0,
        wait_for_gpu_ms:    0.0,
        render_graph_ms:    0.0,
        graph_submit_ms:    0.0,
        cleanup_ms:         0.0,
    };
}

/// Last displayed overlay string, so the overlay tree is rebuilt only when a
/// shown value changes.
#[derive(Resource, Default)]
struct LastDisplayedStatus {
    text: String,
}

/// Last displayed batch-stats string, so the batch panel rebuilds only when a
/// shown value changes.
#[derive(Resource, Default)]
struct LastDisplayedBatchStats {
    text: String,
}

fn spawn_status_overlay(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let built = fps_stats_panel(
        build_overlay_tree(
            &INITIAL_METRICS.map(String::from),
            &INITIAL_METRICS.map(String::from),
        ),
        &mut materials,
    );
    match built {
        Ok(built) => {
            commands.spawn((StatusPanel, built, Transform::default()));
        },
        Err(error) => error!("diegetic_text_stress: failed to build status overlay: {error}"),
    }
}

fn spawn_batch_stats_overlay(
    mut commands: Commands,
    fonts: Res<FontRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let built = diegetic_stats_panel_with_integral_advance(
        &batch_stats_rows(&BatchStatsValues::default()),
        &fonts,
        &mut materials,
    );
    match built {
        Ok(built) => {
            commands.spawn((BatchStatsPanel, built, Transform::default()));
        },
        Err(error) => error!("diegetic_text_stress: failed to build batch stats overlay: {error}"),
    }
}

fn status_panel_pipeline_attachment() -> PanelAttachment {
    PanelAttachment::new(Anchor::BottomLeft, Anchor::TopLeft).with_offset(PanelAnchorOffset::new(
        Px(0.0),
        Px(-fairy_dust::SCREEN_PANEL_GAP),
    ))
}

fn anchor_status_panel_when_gpu_pipeline_added(
    trigger: On<Add<GpuPipelinePanel>>,
    status_panels: Query<Entity, With<StatusPanel>>,
    panel_entities: PanelEntityReader,
    mut attachments: Commands,
) {
    let Some(target) = panel_entities.screen(trigger.entity) else {
        return;
    };
    for status_panel in &status_panels {
        let Some(source) = panel_entities.screen(status_panel) else {
            continue;
        };
        attachments.attach_to_panel(source, target, status_panel_pipeline_attachment());
    }
}

fn anchor_status_panel_when_status_panel_added(
    trigger: On<Add<StatusPanel>>,
    gpu_pipeline_panels: Query<Entity, With<GpuPipelinePanel>>,
    panel_entities: PanelEntityReader,
    mut attachments: Commands,
) {
    let Ok(gpu_pipeline_panel) = gpu_pipeline_panels.single() else {
        return;
    };
    let (Some(source), Some(target)) = (
        panel_entities.screen(trigger.entity),
        panel_entities.screen(gpu_pipeline_panel),
    ) else {
        return;
    };
    attachments.attach_to_panel(source, target, status_panel_pipeline_attachment());
}

fn status_label_style() -> TextStyle {
    TextStyle::new(PERF_ROW_FONT_SIZE)
        .with_color(STATUS_LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn status_value_style() -> TextStyle {
    TextStyle::new(PERF_ROW_FONT_SIZE)
        .with_color(STATUS_TEXT_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn status_label_style_with_color(color: Color) -> TextStyle {
    TextStyle::new(PERF_ROW_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn status_value_style_with_color(color: Color) -> TextStyle {
    TextStyle::new(PERF_ROW_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn perf_header_label_style() -> TextStyle {
    TextStyle::new(PERF_HEADER_ROW_FONT_SIZE)
        .with_color(STATUS_LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn perf_header_value_style() -> TextStyle {
    TextStyle::new(PERF_HEADER_ROW_FONT_SIZE)
        .with_color(STATUS_TEXT_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

fn stats_header_label_style() -> TextStyle {
    TextStyle::new(STATS_HEADER_FONT_SIZE)
        .with_color(STATUS_LABEL_COLOR)
        .with_shadow_mode(GlyphShadowMode::None)
}

/// Whether a value cell renders dimmer (headers / footer) or as a live value.
#[derive(Clone, Copy)]
enum CellEmphasis {
    Normal,
    Dim,
    Header,
}

fn label_cell_with_accent(
    builder: &mut LayoutBuilder,
    text: &str,
    indent: RowIndent,
    emphasis: CellEmphasis,
    accent: Option<Color>,
) {
    let style = match (emphasis, accent) {
        (CellEmphasis::Header, _) => perf_header_label_style(),
        (CellEmphasis::Normal, Some(color)) => status_label_style_with_color(color),
        (CellEmphasis::Dim | CellEmphasis::Normal, _) => status_label_style(),
    };
    builder.with(
        El::new()
            .width(Sizing::fixed(LABEL_COLUMN_WIDTH))
            .height(Sizing::FIT)
            .padding(Padding::new(indent.left_padding(), 0.0, 0.0, 0.0))
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text((text, style));
        },
    );
}

fn value_cell_with_accent(
    builder: &mut LayoutBuilder,
    text: &str,
    emphasis: CellEmphasis,
    accent: Option<Color>,
) {
    let style = match (emphasis, accent) {
        (CellEmphasis::Dim, _) => status_label_style(),
        (CellEmphasis::Header, _) => perf_header_value_style(),
        (CellEmphasis::Normal, Some(color)) => status_value_style_with_color(color),
        (CellEmphasis::Normal, _) => status_value_style(),
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

fn table_row(
    builder: &mut LayoutBuilder,
    metric_row: MetricRow,
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
            label_cell_with_accent(
                builder,
                metric_row.label,
                metric_row.indent,
                emphasis,
                metric_row.accent,
            );
            value_cell_with_accent(builder, now, emphasis, metric_row.accent);
            value_cell_with_accent(builder, max, emphasis, metric_row.accent);
        },
    );
}

fn table_subheader(
    builder: &mut LayoutBuilder,
    title: &str,
    indent: RowIndent,
    accent: Option<Color>,
) {
    builder.with(
        El::new()
            .width(Sizing::fixed(STATUS_TABLE_WIDTH))
            .height(Sizing::FIT)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            label_cell_with_accent(builder, title, indent, CellEmphasis::Normal, accent);
        },
    );
}

fn table_section_separator(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::fixed(STATUS_TABLE_WIDTH))
            .height(Sizing::fixed(PANEL_SEPARATOR_THICKNESS))
            .background(PANEL_SEPARATOR_COLOR),
        |_builder| {},
    );
}

fn table_section_title(builder: &mut LayoutBuilder, title: &str) {
    builder.with(
        El::new()
            .width(Sizing::fixed(STATUS_TABLE_WIDTH))
            .height(Sizing::FIT)
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            builder.text((title, stats_header_label_style()));
        },
    );
}

/// Builds the overlay: a `now` and a `5s max` column of right-aligned numerics,
/// with section titles separating the main-world and render-world rows.
fn build_overlay_tree(now: &[String; METRIC_COUNT], max: &[String; METRIC_COUNT]) -> LayoutTree {
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
                    table_row(
                        builder,
                        MetricRow::new("", RowIndent::None),
                        "now",
                        "5s max",
                        CellEmphasis::Dim,
                    );
                    table_row(
                        builder,
                        METRIC_ROWS[FPS_METRIC_INDEX],
                        &now[FPS_METRIC_INDEX],
                        &max[FPS_METRIC_INDEX],
                        CellEmphasis::Header,
                    );
                    table_section_separator(builder);
                    table_section_title(builder, "main world N+1");
                    table_row(
                        builder,
                        METRIC_ROWS[MAIN_WORLD_METRIC_START],
                        &now[MAIN_WORLD_METRIC_START],
                        &max[MAIN_WORLD_METRIC_START],
                        CellEmphasis::Normal,
                    );
                    table_subheader(
                        builder,
                        "work",
                        RowIndent::Phase,
                        Some(GPU_PIPELINE_WORK_COLOR),
                    );
                    for index in (MAIN_WORLD_METRIC_START + 1)..RENDER_WORLD_METRIC_START {
                        table_row(
                            builder,
                            METRIC_ROWS[index],
                            &now[index],
                            &max[index],
                            CellEmphasis::Normal,
                        );
                    }
                    table_section_separator(builder);
                    table_section_title(builder, "render world N");
                    table_row(
                        builder,
                        METRIC_ROWS[RENDER_WORLD_METRIC_START],
                        &now[RENDER_WORLD_METRIC_START],
                        &max[RENDER_WORLD_METRIC_START],
                        CellEmphasis::Normal,
                    );
                    table_subheader(
                        builder,
                        "work",
                        RowIndent::Phase,
                        Some(GPU_PIPELINE_WORK_COLOR),
                    );
                    for index in (RENDER_WORLD_METRIC_START + 1)..METRIC_COUNT {
                        table_row(
                            builder,
                            METRIC_ROWS[index],
                            &now[index],
                            &max[index],
                            CellEmphasis::Normal,
                        );
                    }
                },
            );
        },
    );
    builder.build()
}

fn update_status_panel(
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    diegetic_perf: Res<DiegeticPerfStats>,
    timing: Res<FrameTiming>,
    panels: Query<Entity, With<StatusPanel>>,
    mut last_displayed: ResMut<LastDisplayedStatus>,
    mut commands: Commands,
    mut timer: Local<Option<Timer>>,
    mut history: Local<VecDeque<PerfSnapshot>>,
) {
    let frames_per_second = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(Diagnostic::smoothed);

    // Sample every frame so the smoothed `now` mean and the 5-second peak see
    // every value, not one arbitrary frame per second. The service's latest
    // complete frame can be older than the local diegetic measurements.
    let frame_ms = last_ms_or_zero(&timing.frame_time);
    let layout_ms = diegetic_perf.compute_ms;
    let reify_ms = diegetic_perf.reify_ms;
    let shaping_ms = diegetic_perf.panel_text.shape_ms;
    let mesh_ms = diegetic_perf.panel_text.mesh_build_ms;
    // The local diegetic spans and complete service record can differ by one
    // frame. Clamp the illustrative remainder when that difference matters.
    let main_span_ms = last_ms_or_zero(&timing.first)
        + last_ms_or_zero(&timing.pre_update)
        + last_ms_or_zero(&timing.state_transition)
        + last_ms_or_zero(&timing.fixed_update)
        + last_ms_or_zero(&timing.update)
        + last_ms_or_zero(&timing.spawn_scene)
        + last_ms_or_zero(&timing.post_update)
        + last_ms_or_zero(&timing.last);
    let other_ms = (main_span_ms - layout_ms - reify_ms - shaping_ms - mesh_ms).max(0.0);
    let outside_main_ms = (frame_ms - main_span_ms).max(0.0);
    let wait_for_render_ms =
        last_ms_or_zero(&timing.waiting_for_render).clamp(0.0, outside_main_ms);
    let extract_ms =
        last_ms_or_zero(&timing.extract).clamp(0.0, outside_main_ms - wait_for_render_ms);
    let frame_slack_ms = (outside_main_ms - wait_for_render_ms - extract_ms).max(0.0);
    let assets_ms = last_ms_or_zero(&timing.prepare_assets);
    let prep_ms =
        last_ms_or_zero(&timing.extract_commands) + last_ms_or_zero(&timing.prepare_and_queue);
    let wait_for_gpu_ms = last_ms_or_zero(&timing.waiting_for_next_image);
    let render_graph_ms = last_ms_or_zero(&timing.render_graph);
    let graph_submit_ms = last_ms_or_zero(&timing.submit);
    let cleanup_ms = last_ms_or_zero(&timing.cleanup);
    let render_work_ms =
        assets_ms + prep_ms + wait_for_gpu_ms + render_graph_ms + graph_submit_ms + cleanup_ms;
    history.push_back(PerfSnapshot {
        timestamp: time.elapsed_secs(),
        fps: frames_per_second.unwrap_or(0.0).to_f32(),
        frame_ms,
        layout_ms,
        reify_ms,
        shaping_ms,
        mesh_ms,
        other_ms,
        wait_for_render_ms,
        extract_ms,
        frame_slack_ms,
        render_work_ms,
        assets_ms,
        prep_ms,
        wait_for_gpu_ms,
        render_graph_ms,
        graph_submit_ms,
        cleanup_ms,
    });

    let cutoff = time.elapsed_secs() - PERF_PEAK_WINDOW_SECS;
    while history
        .front()
        .is_some_and(|sample| sample.timestamp < cutoff)
    {
        history.pop_front();
    }

    // Rebuild the overlay tree at most once per second. The overlay is itself a
    // panel, so rebuilding it every frame would add its own shaping / mesh work
    // to the very numbers being measured.
    let timer =
        timer.get_or_insert_with(|| Timer::from_seconds(FPS_UPDATE_INTERVAL, TimerMode::Repeating));
    timer.tick(time.delta());
    if !timer.just_finished() {
        return;
    }

    let mean = window_mean(&history);
    let peak = window_peak(&history);

    let now = format_perf_snapshot(mean);
    let max = format_perf_snapshot(peak);

    let key = format!("{}|{}", now.join(","), max.join(","));
    if key != last_displayed.text {
        last_displayed.text.clone_from(&key);
        for entity in &panels {
            if let Err(error) = commands.set_tree(entity, build_overlay_tree(&now, &max)) {
                error!("failed to replace text stress overlay tree: {error}");
            }
        }
    }
}

fn format_perf_snapshot(snapshot: PerfSnapshot) -> [String; METRIC_COUNT] {
    [
        format!("{:.0}", snapshot.fps),
        format!("{:.1}", snapshot.frame_ms),
        format!("{:.2}", snapshot.layout_ms),
        format!("{:.2}", snapshot.reify_ms),
        format!("{:.2}", snapshot.shaping_ms),
        format!("{:.2}", snapshot.mesh_ms),
        format!("{:.2}", snapshot.other_ms),
        format!("{:.2}", snapshot.wait_for_render_ms),
        format!("{:.2}", snapshot.extract_ms),
        format!("{:.2}", snapshot.frame_slack_ms),
        format!("{:.1}", snapshot.render_work_ms),
        format!("{:.2}", snapshot.assets_ms),
        format!("{:.2}", snapshot.prep_ms),
        format!("{:.2}", snapshot.wait_for_gpu_ms),
        format!("{:.2}", snapshot.render_graph_ms),
        format!("{:.2}", snapshot.graph_submit_ms),
        format!("{:.2}", snapshot.cleanup_ms),
    ]
}

/// The batch, record, upload, and shadow-draw counts shown in the upper-right
/// panel.
#[derive(Default)]
struct BatchStatsValues {
    text_batches:      usize,
    text_runs:         usize,
    text_glyphs:       usize,
    text_instance_up:  usize,
    text_run_table_up: usize,
    line_batches:      usize,
    line_records:      usize,
    line_uploads:      usize,
    sdf_batches:       usize,
    sdf_records:       usize,
    sdf_uploads:       usize,
    material_rows:     usize,
    material_bytes:    usize,
    material_capacity: usize,
    shadow_items:      u32,
    /// Caster draws per shadow view (largest first).
    shadow_per_view:   Vec<u32>,
}

/// Label, value, and detail lines for each batch-stats group. The `batches`
/// details enumerate the batch keys this scene routes to (fixed); the `shadow`
/// details are the live per-view breakdown that produced the number this frame,
/// read from the render phases.
fn batch_stats_rows(values: &BatchStatsValues) -> Vec<StatsPanelRow> {
    vec![
        StatsPanelRow::new(
            "profile",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        ),
        StatsPanelRow::new("world text labels", LABEL_COUNT.to_string()),
        StatsPanelRow::new(
            "batched analytic draws",
            (values.text_batches + values.line_batches).to_string(),
        )
        .details([
            format!(
                "{} text + {} panel-line",
                values.text_batches, values.line_batches
            ),
            "text and panel lines use separate batch stores".to_string(),
        ]),
        StatsPanelRow::new("sdf batches", values.sdf_batches.to_string()).details([
            "backgrounds, borders, separator rectangles".to_string(),
            "visible SDF batch entities".to_string(),
        ]),
        StatsPanelRow::new("sdf records", values.sdf_records.to_string())
            .detail("panel chrome records across all SDF batches"),
        StatsPanelRow::new("material rows", values.material_rows.to_string()).details([
            "MaterialSlotValues rows for SDF, text, and panel lines".to_string(),
            format!("live row bytes: {}", values.material_bytes),
        ]),
        StatsPanelRow::new("material capacity", values.material_capacity.to_string())
            .detail("shared material-table row capacity"),
        StatsPanelRow::new("text batches", values.text_batches.to_string()).details([
            "TextRunBatchStore: compatible text runs share draws".to_string(),
            "labels and panel text route together by key".to_string(),
        ]),
        StatsPanelRow::new("text runs", values.text_runs.to_string())
            .detail("text runs routed across all batches"),
        StatsPanelRow::new("glyphs", values.text_glyphs.to_string())
            .detail("glyph instances across all batches"),
        StatsPanelRow::new("panel-line batches", values.line_batches.to_string())
            .detail("PanelLineBatchStore: compatible line primitives share draws"),
        StatsPanelRow::new("line records", values.line_records.to_string())
            .detail("analytic path instances across line batches"),
        StatsPanelRow::new(
            "buffer uploads",
            (values.text_instance_up
                + values.text_run_table_up
                + values.line_uploads
                + values.sdf_uploads)
                .to_string(),
        )
        .details([
            format!("text instances: {}", values.text_instance_up),
            format!("text run table: {}", values.text_run_table_up),
            format!("panel-line buffers: {}", values.line_uploads),
            format!("sdf buffers: {}", values.sdf_uploads),
        ]),
        StatsPanelRow::new("shadow", values.shadow_items.to_string())
            .details(shadow_details(&values.shadow_per_view)),
    ]
}

/// The live breakdown behind the `shadow` number. This scene has one
/// shadow-casting key light and two casters: the text batch and the ground
/// plane.
fn shadow_details(per_view: &[u32]) -> Vec<String> {
    if per_view.is_empty() {
        return vec![
            "keylight shadow pass".to_string(),
            "casters: text batch & ground plane".to_string(),
            "no shadow cascades yet".to_string(),
        ];
    }
    let draws_per_cascade = per_view.first().copied().unwrap_or_default();
    vec![
        "keylight shadow pass".to_string(),
        "casters: text batch & ground plane".to_string(),
        format!(
            "{} cascades * {} draws (1 for each caster)",
            per_view.len(),
            draws_per_cascade
        ),
    ]
}

/// Refreshes the upper-right batch-stats panel: batch store contents and
/// per-pass phase items.
fn update_batch_stats_panel(
    diegetic_perf: Res<DiegeticPerfStats>,
    draw_counts: Res<DrawCounts>,
    panels: Query<Entity, With<BatchStatsPanel>>,
    mut last_displayed: ResMut<LastDisplayedBatchStats>,
    mut commands: Commands,
    mut timer: Local<Option<Timer>>,
    time: Res<Time>,
    fonts: Res<FontRegistry>,
) {
    let timer =
        timer.get_or_insert_with(|| Timer::from_seconds(FPS_UPDATE_INTERVAL, TimerMode::Repeating));
    timer.tick(time.delta());
    if !timer.just_finished() {
        return;
    }

    let batch = &diegetic_perf.batch;
    let line_batch = diegetic_perf.line_batch;
    let sdf = diegetic_perf.panel_geometry;
    let material_table = diegetic_perf.material_table;
    let values = BatchStatsValues {
        text_batches:      batch.batches.get(),
        text_runs:         batch.runs.get(),
        text_glyphs:       batch.glyph_records.get(),
        text_instance_up:  batch.instance_uploads.get(),
        text_run_table_up: batch.run_table_uploads.get(),
        line_batches:      line_batch.batches.get(),
        line_records:      line_batch.records.get(),
        line_uploads:      line_batch.uploads.get(),
        sdf_batches:       sdf.sdf_batches.get(),
        sdf_records:       sdf.sdf_records.get(),
        sdf_uploads:       sdf.sdf_uploads.get(),
        material_rows:     material_table.rows.get(),
        material_bytes:    material_table.upload_bytes.get(),
        material_capacity: material_table.capacity.get(),
        shadow_items:      draw_counts.shadow_items(),
        shadow_per_view:   draw_counts.shadow_per_view(),
    };
    let rows = batch_stats_rows(&values);
    let mut key = String::new();
    for row in &rows {
        key.push_str(&row.label);
        key.push('=');
        key.push_str(&row.value);
        key.push('|');
        for detail in &row.details {
            key.push_str(detail);
            key.push('|');
        }
    }
    if key != last_displayed.text {
        last_displayed.text.clone_from(&key);
        for entity in &panels {
            if let Err(error) = commands.set_tree(
                entity,
                diegetic_stats_tree_with_integral_advance(&rows, &fonts),
            ) {
                error!("failed to replace diegetic stats tree: {error}");
            }
        }
    }
}

/// Per-frame mean of each metric across the sample window — the smoothed `now`
/// column. Returns [`PerfSnapshot::ZERO`] for an empty window.
fn window_mean(history: &VecDeque<PerfSnapshot>) -> PerfSnapshot {
    let count = history.len().to_f32();
    if count == 0.0 {
        return PerfSnapshot::ZERO;
    }
    let mut sum = PerfSnapshot::ZERO;
    for sample in history {
        sum.fps += sample.fps;
        sum.frame_ms += sample.frame_ms;
        sum.layout_ms += sample.layout_ms;
        sum.reify_ms += sample.reify_ms;
        sum.shaping_ms += sample.shaping_ms;
        sum.mesh_ms += sample.mesh_ms;
        sum.other_ms += sample.other_ms;
        sum.wait_for_render_ms += sample.wait_for_render_ms;
        sum.extract_ms += sample.extract_ms;
        sum.frame_slack_ms += sample.frame_slack_ms;
        sum.render_work_ms += sample.render_work_ms;
        sum.assets_ms += sample.assets_ms;
        sum.prep_ms += sample.prep_ms;
        sum.wait_for_gpu_ms += sample.wait_for_gpu_ms;
        sum.render_graph_ms += sample.render_graph_ms;
        sum.graph_submit_ms += sample.graph_submit_ms;
        sum.cleanup_ms += sample.cleanup_ms;
    }
    PerfSnapshot {
        timestamp:          0.0,
        fps:                sum.fps / count,
        frame_ms:           sum.frame_ms / count,
        layout_ms:          sum.layout_ms / count,
        reify_ms:           sum.reify_ms / count,
        shaping_ms:         sum.shaping_ms / count,
        mesh_ms:            sum.mesh_ms / count,
        other_ms:           sum.other_ms / count,
        wait_for_render_ms: sum.wait_for_render_ms / count,
        extract_ms:         sum.extract_ms / count,
        frame_slack_ms:     sum.frame_slack_ms / count,
        render_work_ms:     sum.render_work_ms / count,
        assets_ms:          sum.assets_ms / count,
        prep_ms:            sum.prep_ms / count,
        wait_for_gpu_ms:    sum.wait_for_gpu_ms / count,
        render_graph_ms:    sum.render_graph_ms / count,
        graph_submit_ms:    sum.graph_submit_ms / count,
        cleanup_ms:         sum.cleanup_ms / count,
    }
}

/// Per-metric peak across the sample window — the `5s max` column.
fn window_peak(history: &VecDeque<PerfSnapshot>) -> PerfSnapshot {
    let mut peak = PerfSnapshot::ZERO;
    for sample in history {
        peak.fps = peak.fps.max(sample.fps);
        peak.frame_ms = peak.frame_ms.max(sample.frame_ms);
        peak.layout_ms = peak.layout_ms.max(sample.layout_ms);
        peak.reify_ms = peak.reify_ms.max(sample.reify_ms);
        peak.shaping_ms = peak.shaping_ms.max(sample.shaping_ms);
        peak.mesh_ms = peak.mesh_ms.max(sample.mesh_ms);
        peak.other_ms = peak.other_ms.max(sample.other_ms);
        peak.wait_for_render_ms = peak.wait_for_render_ms.max(sample.wait_for_render_ms);
        peak.extract_ms = peak.extract_ms.max(sample.extract_ms);
        peak.frame_slack_ms = peak.frame_slack_ms.max(sample.frame_slack_ms);
        peak.render_work_ms = peak.render_work_ms.max(sample.render_work_ms);
        peak.assets_ms = peak.assets_ms.max(sample.assets_ms);
        peak.prep_ms = peak.prep_ms.max(sample.prep_ms);
        peak.wait_for_gpu_ms = peak.wait_for_gpu_ms.max(sample.wait_for_gpu_ms);
        peak.render_graph_ms = peak.render_graph_ms.max(sample.render_graph_ms);
        peak.graph_submit_ms = peak.graph_submit_ms.max(sample.graph_submit_ms);
        peak.cleanup_ms = peak.cleanup_ms.max(sample.cleanup_ms);
    }
    peak
}

// ── Overlay — GPU pipeline: bottom main/render/GPU timeline lanes ───────────

// colors
/// Inferred Bevy GPU-pressure blocks. These may be executing on the hardware or
/// queued behind other GPU work; they are not timestamp-measured execution.
const GPU_PIPELINE_GPU_COLOR: Color = Color::srgb(0.0, 0.92, 1.0);
/// Neutral spans: GPU available gap, frame slack, and render-world extract
/// handoff.
const GPU_PIPELINE_GAP_COLOR: Color = Color::srgb(0.72, 0.78, 0.82);
/// Main and render lane `work` segments.
const GPU_PIPELINE_WORK_COLOR: Color = Color::srgb(0.30, 0.60, 0.95);
/// Render world N `render graph` segment.
const GPU_PIPELINE_RENDER_GRAPH_COLOR: Color = Color::srgb(0.35, 0.85, 0.45);
/// Root render-graph submit set.
const GPU_PIPELINE_GRAPH_SUBMIT_COLOR: Color = Color::srgb(0.70, 0.92, 0.36);
/// Render world N post-graph cleanup work.
const GPU_PIPELINE_CLEANUP_COLOR: Color = Color::srgb(1.0, 0.22, 0.10);
/// Track tint behind each lane — the empty tail past the drawn segments.
const GPU_PIPELINE_TRACK_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.06);
/// Shared between main render wait and render image-acquisition wait.
const GPU_PIPELINE_WAIT_COLOR: Color = Color::srgb(0.95, 0.65, 0.20);
/// Main lane `extract` — the main->render copy and send-back.
const GPU_PIPELINE_EXTRACT_COLOR: Color = Color::srgb(0.72, 0.54, 1.0);
/// Unclassified app-frame slack in the perf table; not measured work.
const GPU_PIPELINE_FRAME_SLACK_COLOR: Color = GPU_PIPELINE_GAP_COLOR;
/// Segment label color for dark backgrounds.
const GPU_PIPELINE_LIGHT_LABEL_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 1.0);
/// Segment label color for light backgrounds.
const GPU_PIPELINE_DARK_LABEL_COLOR: Color = Color::srgba(0.03, 0.04, 0.06, 1.0);
/// WCAG luminance weight for linear red.
const GPU_PIPELINE_LUMINANCE_RED_WEIGHT: f32 = 0.2126;
/// WCAG luminance weight for linear green.
const GPU_PIPELINE_LUMINANCE_GREEN_WEIGHT: f32 = 0.7152;
/// WCAG luminance weight for linear blue.
const GPU_PIPELINE_LUMINANCE_BLUE_WEIGHT: f32 = 0.0722;
/// sRGB breakpoint for converting to linear light.
const GPU_PIPELINE_SRGB_LINEAR_BREAKPOINT: f32 = 0.04045;
/// Linear divisor below [`GPU_PIPELINE_SRGB_LINEAR_BREAKPOINT`].
const GPU_PIPELINE_SRGB_LINEAR_DIVISOR: f32 = 12.92;
/// sRGB offset used by the IEC transfer function.
const GPU_PIPELINE_SRGB_TRANSFER_OFFSET: f32 = 0.055;
/// sRGB transfer-function divisor.
const GPU_PIPELINE_SRGB_TRANSFER_DIVISOR: f32 = 1.055;
/// sRGB transfer-function exponent.
const GPU_PIPELINE_SRGB_TRANSFER_EXPONENT: f32 = 2.4;
/// WCAG contrast-ratio luminance offset.
const GPU_PIPELINE_CONTRAST_LUMINANCE_OFFSET: f32 = 0.05;

// layout
/// Gap between the label column and the bar column in panel pixels.
const GPU_PIPELINE_LABEL_GAP: f32 = 6.0;
/// Vertical gap between lanes in panel pixels.
const GPU_PIPELINE_LANE_GAP: f32 = 4.0;
/// Lane bar thickness in panel pixels.
const GPU_PIPELINE_LANE_HEIGHT: f32 = 14.7;
/// Lane label text size in panel pixels.
const GPU_PIPELINE_LANE_LABEL_FONT_SIZE: f32 = 13.0;
/// Lane row height in panel pixels — the label cell and the bar wrapper share
/// it so the two columns align; tall enough not to clip the label text.
const GPU_PIPELINE_LANE_ROW_HEIGHT: f32 = 21.0;
/// Axis floor (ms) — keeps the bar scale finite before the first frame samples
/// and on a stall.
const GPU_PIPELINE_MIN_AXIS_MS: f32 = 1.0;
/// Timeline spacers thinner than this track fraction are dropped.
const GPU_PIPELINE_MIN_SEGMENT_FRACTION: f32 = 0.001;
/// Segment label font size in panel pixels.
const GPU_PIPELINE_SEGMENT_LABEL_FONT_SIZE: f32 = 9.0;
/// Gap a segment label keeps from each end of its segment. Leading- and
/// trailing-aligned labels pad their own edge by it.
const GPU_PIPELINE_SEGMENT_LABEL_INSET: f32 = 6.0;
const GPU_PIPELINE_LABELS: [&str; 10] = [
    "main work",
    "wait for render",
    "extract",
    "render prep",
    "next image wait",
    "camera graphs",
    "submit",
    "current",
    "available",
    "next",
];

// timing
/// How long each on-screen picture is sampled before it refreshes (seconds).
/// Per-frame values are averaged over this window so the held picture is the
/// second's mean, not one noisy frame.
const GPU_PIPELINE_SAMPLE_PERIOD: f32 = 1.0;
/// How long the bars take to slide from the old picture to the new one at each
/// refresh (seconds). After the morph the picture holds for the rest of the
/// sample period (≈800 ms), giving a still frame to read.
const GPU_PIPELINE_MORPH_DURATION: f32 = 0.2;

/// Marker for the bottom-left GPU pipeline visualization.
#[derive(Component)]
struct GpuPipelinePanel;

/// Whether the GPU pipeline visualization is shown; `M` toggles it and the
/// title-bar `Pipeline` segment highlights while it is on. When hidden, the
/// panel root takes `Visibility::Hidden` and `update_gpu_pipeline_panel` skips
/// the rebuild.
#[derive(Resource, Default)]
enum GpuPipelineShown {
    #[default]
    Shown,
    Hidden,
}

/// Lane values for the GPU pipeline visualization, in milliseconds. The update
/// samples a one-second mean into this, then morphs the on-screen copy toward it
/// over [`GPU_PIPELINE_MORPH_DURATION`]. The lane values are converted into
/// labeled [`TimelineSegment`]s when the tree is rebuilt.
#[derive(Clone, Copy, Default)]
struct GpuPipelineBars {
    /// Shared duration scale in milliseconds, without cross-thread alignment.
    axis:          f32,
    main_work:     f32,
    main_wait:     f32,
    main_extract:  f32,
    prep:          f32,
    gpu_wait:      f32,
    render_graph:  f32,
    /// CPU camera/shadow graph execution before queued command buffers are
    /// submitted to the GPU queue.
    camera_graphs: f32,
    /// Root render-graph submit set, ending after queued command buffers have
    /// been submitted to the GPU queue.
    graph_submit:  f32,
    cleanup:       f32,
}

impl GpuPipelineBars {
    /// Field-wise sum, for accumulating the per-second mean.
    fn add(self, other: Self) -> Self {
        Self {
            axis:          self.axis + other.axis,
            main_work:     self.main_work + other.main_work,
            main_wait:     self.main_wait + other.main_wait,
            main_extract:  self.main_extract + other.main_extract,
            prep:          self.prep + other.prep,
            gpu_wait:      self.gpu_wait + other.gpu_wait,
            render_graph:  self.render_graph + other.render_graph,
            camera_graphs: self.camera_graphs + other.camera_graphs,
            graph_submit:  self.graph_submit + other.graph_submit,
            cleanup:       self.cleanup + other.cleanup,
        }
    }

    /// Field-wise scale, dividing the accumulated sum by the sample count.
    fn scale(self, factor: f32) -> Self {
        Self {
            axis:          self.axis * factor,
            main_work:     self.main_work * factor,
            main_wait:     self.main_wait * factor,
            main_extract:  self.main_extract * factor,
            prep:          self.prep * factor,
            gpu_wait:      self.gpu_wait * factor,
            render_graph:  self.render_graph * factor,
            camera_graphs: self.camera_graphs * factor,
            graph_submit:  self.graph_submit * factor,
            cleanup:       self.cleanup * factor,
        }
    }

    /// Field-wise lerp from `self` toward `to` by fraction `t`, for the morph.
    const fn lerp(self, to: Self, t: f32) -> Self {
        Self {
            axis:          lerp(self.axis, to.axis, t),
            main_work:     lerp(self.main_work, to.main_work, t),
            main_wait:     lerp(self.main_wait, to.main_wait, t),
            main_extract:  lerp(self.main_extract, to.main_extract, t),
            prep:          lerp(self.prep, to.prep, t),
            gpu_wait:      lerp(self.gpu_wait, to.gpu_wait, t),
            render_graph:  lerp(self.render_graph, to.render_graph, t),
            camera_graphs: lerp(self.camera_graphs, to.camera_graphs, t),
            graph_submit:  lerp(self.graph_submit, to.graph_submit, t),
            cleanup:       lerp(self.cleanup, to.cleanup, t),
        }
    }
}

/// Label placement inside a GPU pipeline segment.
#[derive(Clone, Copy)]
enum SegmentLabelAlignment {
    Center,
    Leading,
    Trailing,
}

impl SegmentLabelAlignment {
    const fn align_x(self) -> AlignX {
        match self {
            Self::Center => AlignX::Center,
            Self::Leading => AlignX::Left,
            Self::Trailing => AlignX::Right,
        }
    }

    const fn text_align(self) -> TextAlign {
        match self {
            Self::Center => TextAlign::Center,
            Self::Leading => TextAlign::Left,
            Self::Trailing => TextAlign::Right,
        }
    }

    fn padding(self) -> Padding {
        match self {
            Self::Center => Padding::default(),
            Self::Leading => Padding::new(GPU_PIPELINE_SEGMENT_LABEL_INSET, 0.0, 0.0, 0.0),
            Self::Trailing => Padding::new(0.0, GPU_PIPELINE_SEGMENT_LABEL_INSET, 0.0, 0.0),
        }
    }
}

/// A lane segment on the shared timeline, in milliseconds relative to the
/// render frame's start mark.
#[derive(Clone, Copy)]
struct TimelineSegment {
    start:     f32,
    end:       f32,
    color:     Color,
    label:     &'static str,
    alignment: SegmentLabelAlignment,
}

impl TimelineSegment {
    const fn measured(start: f32, end: f32, color: Color, label: &'static str) -> Self {
        Self {
            start,
            end,
            color,
            label,
            alignment: SegmentLabelAlignment::Center,
        }
    }

    const fn inferred(start: f32, end: f32, color: Color, label: &'static str) -> Self {
        Self {
            start,
            end,
            color,
            label,
            alignment: SegmentLabelAlignment::Center,
        }
    }

    const fn with_label_alignment(mut self, alignment: SegmentLabelAlignment) -> Self {
        self.alignment = alignment;
        self
    }
}

fn spawn_gpu_pipeline_overlay(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let built = gpu_meter_panel(
        build_gpu_pipeline_tree(
            &GpuPipelineBars::default(),
            0.0,
            &GpuPipelineLabelWidths::default(),
        ),
        &mut materials,
    );
    match built {
        Ok(built) => {
            commands.spawn((GpuPipelinePanel, built, Transform::default()));
        },
        Err(error) => error!("diegetic_text_stress: failed to build GPU pipeline overlay: {error}"),
    }
}

#[derive(Default)]
struct GpuPipelineLabelWidths(HashMap<&'static str, f32>);

impl GpuPipelineLabelWidths {
    fn measure(measure_text: &MeasureTextFn, font_scale: f32) -> Self {
        let style = TextStyle::new(GPU_PIPELINE_SEGMENT_LABEL_FONT_SIZE).bold();
        let measure = style.as_measure().scaled(font_scale);
        Self(
            GPU_PIPELINE_LABELS
                .iter()
                .map(|&label| (label, measure_text(label, &measure).width))
                .collect(),
        )
    }

    fn width(&self, label: &str) -> f32 { self.0.get(label).copied().unwrap_or(f32::INFINITY) }
}

#[derive(Default)]
enum GpuPipelineLabelFit {
    #[default]
    AwaitingLayout,
    BuiltAt {
        track_width: f32,
        font_scale:  f32,
        widths:      GpuPipelineLabelWidths,
    },
}

impl GpuPipelineLabelFit {
    const fn needs_rebuild(&self, moving: bool, track_width: f32, font_scale: f32) -> bool {
        match self {
            Self::AwaitingLayout => true,
            Self::BuiltAt {
                track_width: previous_width,
                font_scale: previous_scale,
                ..
            } => {
                moving
                    || previous_width.to_bits() != track_width.to_bits()
                    || previous_scale.to_bits() != font_scale.to_bits()
            },
        }
    }
}

fn gpu_pipeline_track_index(tree: &LayoutTree) -> Result<usize, &'static str> {
    let child = |index| {
        tree.children_of(index)
            .first()
            .copied()
            .ok_or("pipeline tree is incomplete")
    };
    let frame = child(0)?;
    let inner = child(frame)?;
    let columns = child(inner)?;
    let bars = *tree
        .children_of(columns)
        .get(1)
        .ok_or("pipeline bar column is missing")?;
    let lane = child(bars)?;
    child(lane)
}

fn laid_out_gpu_pipeline_track_width(
    tree: &LayoutTree,
    computed: &ComputedDiegeticPanel,
) -> Result<f32, &'static str> {
    let layout = computed
        .layout()
        .solved()
        .ok_or("pipeline layout is unsolved")?;
    let track = gpu_pipeline_track_index(tree)?;
    // LayoutResult stores points; the authored padding and label measurement
    // below use the panel's pixel layout unit.
    Ok(layout
        .computed
        .get(track)
        .ok_or("pipeline track layout is missing")?
        .bounds
        .width
        / Unit::Pixels.to_points())
}

/// Builds the panel: a left label column (`Fit` width, sized to the widest
/// label and right-flushed) beside a bar column, so every bar starts at the
/// same x. Lane rows in both columns share [`GPU_PIPELINE_LANE_ROW_HEIGHT`] to keep
/// them aligned.
fn build_gpu_pipeline_tree(
    bars: &GpuPipelineBars,
    track_width: f32,
    label_widths: &GpuPipelineLabelWidths,
) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::GROW).height(Sizing::FIT));
    screen_panel_frame(
        &mut builder,
        Sizing::GROW,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::row()
                    .width(Sizing::GROW)
                    .height(Sizing::FIT)
                    .gap(GPU_PIPELINE_LABEL_GAP)
                    .alignment(AlignX::Left, AlignY::Top),
                |builder| {
                    builder.with(
                        El::column()
                            .width(Sizing::FIT)
                            .height(Sizing::FIT)
                            .gap(GPU_PIPELINE_LANE_GAP)
                            .alignment(AlignX::Right, AlignY::Center),
                        |builder| {
                            lane_label(builder, "main world");
                            lane_label(builder, "render world");
                            lane_label_with_color(builder, "GPU inferred", GPU_PIPELINE_GPU_COLOR);
                        },
                    );
                    builder.with(
                        El::column()
                            .width(Sizing::GROW)
                            .height(Sizing::FIT)
                            .gap(GPU_PIPELINE_LANE_GAP),
                        |builder| {
                            let span = bars.axis.max(GPU_PIPELINE_MIN_AXIS_MS);
                            lane_bars(
                                builder,
                                span,
                                &main_lane_segments(bars),
                                track_width,
                                label_widths,
                            );
                            lane_bars(
                                builder,
                                span,
                                &render_lane_segments(bars),
                                track_width,
                                label_widths,
                            );
                            lane_bars(
                                builder,
                                span,
                                &gpu_lane_segments(bars),
                                track_width,
                                label_widths,
                            );
                        },
                    );
                },
            );
        },
    );
    builder.build()
}

/// One label cell, sized to its text and vertically centered in a shared lane
/// row. The label column's `Fit` width and `AlignX::Right` flush every label
/// against the bars.
fn lane_label(builder: &mut LayoutBuilder, label: &str) {
    lane_label_with_color(builder, label, STATUS_LABEL_COLOR);
}

fn lane_label_with_color(builder: &mut LayoutBuilder, label: &str, color: Color) {
    builder.with(
        El::new()
            .width(Sizing::FIT)
            .height(Sizing::fixed(GPU_PIPELINE_LANE_ROW_HEIGHT))
            .alignment(AlignX::Right, AlignY::Center),
        |builder| {
            builder.text((label, gpu_pipeline_lane_label_style(color)));
        },
    );
}

fn gpu_pipeline_lane_label_style(color: Color) -> TextStyle {
    TextStyle::new(GPU_PIPELINE_LANE_LABEL_FONT_SIZE)
        .with_color(color)
        .with_shadow_mode(GlyphShadowMode::None)
}

/// One lane's bar track: a grow-width overlay tinted
/// [`GPU_PIPELINE_TRACK_COLOR`] with fractional-offset [`TimelineSegment`]
/// blocks. Empty timeline spans become transparent spacers, so reading straight
/// down at one x compares the same process-monotonic time on every lane.
fn lane_bars(
    builder: &mut LayoutBuilder,
    axis_ms: f32,
    segments: &[TimelineSegment],
    track_width: f32,
    label_widths: &GpuPipelineLabelWidths,
) {
    let axis = axis_ms.max(GPU_PIPELINE_MIN_AXIS_MS);
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::fixed(GPU_PIPELINE_LANE_ROW_HEIGHT))
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            gpu_pipeline_track(builder, axis, segments, track_width, label_widths);
        },
    );
}

fn gpu_pipeline_track(
    builder: &mut LayoutBuilder,
    axis: f32,
    segments: &[TimelineSegment],
    track_width: f32,
    label_widths: &GpuPipelineLabelWidths,
) {
    builder.with(
        El::overlay()
            .width(Sizing::GROW)
            .height(Sizing::fixed(GPU_PIPELINE_LANE_HEIGHT))
            .background(GPU_PIPELINE_TRACK_COLOR),
        |builder| {
            gpu_pipeline_segment_row(builder, axis, segments);
            for segment in segments {
                gpu_pipeline_label_row(builder, axis, *segment, track_width, label_widths);
            }
        },
    );
}

fn gpu_pipeline_segment_row(builder: &mut LayoutBuilder, axis: f32, segments: &[TimelineSegment]) {
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::fixed(GPU_PIPELINE_LANE_HEIGHT)),
        |builder| {
            let mut used = 0.0;
            for segment in segments {
                let start_fraction = (segment.start / axis).clamp(0.0, 1.0);
                let end_fraction = (segment.end / axis).clamp(start_fraction, 1.0);
                add_gpu_pipeline_spacer(builder, start_fraction - used);
                used = used.max(start_fraction);

                let fraction = end_fraction - used;
                add_gpu_pipeline_segment(builder, fraction, *segment);
                used = used.max(end_fraction);
            }
        },
    );
}

fn add_gpu_pipeline_spacer(builder: &mut LayoutBuilder, fraction: f32) {
    if fraction < GPU_PIPELINE_MIN_SEGMENT_FRACTION {
        return;
    }
    builder.with(
        El::new()
            .width(Sizing::percent(fraction))
            .height(Sizing::fixed(GPU_PIPELINE_LANE_HEIGHT)),
        |_builder| {},
    );
}

fn add_gpu_pipeline_segment(builder: &mut LayoutBuilder, fraction: f32, segment: TimelineSegment) {
    let cell = El::new()
        .width(Sizing::percent(fraction))
        .height(Sizing::fixed(GPU_PIPELINE_LANE_HEIGHT))
        .padding(segment.alignment.padding())
        .alignment(segment.alignment.align_x(), AlignY::Center);
    let cell = if segment.color.alpha() > 0.0 {
        cell.background(segment.color)
    } else {
        cell
    };
    builder.with(cell, |_builder| {});
}

fn gpu_pipeline_label_row(
    builder: &mut LayoutBuilder,
    axis: f32,
    segment: TimelineSegment,
    track_width: f32,
    label_widths: &GpuPipelineLabelWidths,
) {
    if segment.label.is_empty() {
        return;
    }
    let start_fraction = (segment.start / axis).clamp(0.0, 1.0);
    let end_fraction = (segment.end / axis).clamp(start_fraction, 1.0);
    let label_fraction = end_fraction - start_fraction;
    let style = gpu_pipeline_segment_label_style(segment);
    let label_width = label_widths.width(segment.label);
    if label_fraction <= 0.0 || !gpu_pipeline_label_fits(label_width, label_fraction * track_width)
    {
        return;
    }
    builder.with(
        El::row()
            .width(Sizing::GROW)
            .height(Sizing::fixed(GPU_PIPELINE_LANE_HEIGHT))
            .alignment(AlignX::Left, AlignY::Center),
        |builder| {
            add_gpu_pipeline_position_spacer(builder, start_fraction);
            builder.with(
                El::new()
                    .width(Sizing::percent(label_fraction))
                    .height(Sizing::fixed(GPU_PIPELINE_LANE_HEIGHT))
                    .padding(segment.alignment.padding())
                    .alignment(segment.alignment.align_x(), AlignY::Center),
                |builder| {
                    builder.text(Text::new(segment.label, style).wrap(TextWrap::None));
                },
            );
        },
    );
}

/// Whether a label of `label_width` fits a segment of `segment_width` with
/// `GPU_PIPELINE_SEGMENT_LABEL_INSET` clear at both ends, whatever its alignment.
fn gpu_pipeline_label_fits(label_width: f32, segment_width: f32) -> bool {
    GPU_PIPELINE_SEGMENT_LABEL_INSET.mul_add(2.0, label_width) <= segment_width
}

fn add_gpu_pipeline_position_spacer(builder: &mut LayoutBuilder, fraction: f32) {
    if fraction <= 0.0 {
        return;
    }
    builder.with(
        El::new()
            .width(Sizing::percent(fraction))
            .height(Sizing::fixed(GPU_PIPELINE_LANE_HEIGHT)),
        |_builder| {},
    );
}

fn gpu_pipeline_segment_label_style(segment: TimelineSegment) -> TextStyle {
    TextStyle::new(GPU_PIPELINE_SEGMENT_LABEL_FONT_SIZE)
        .bold()
        .with_color(gpu_pipeline_segment_label_color(segment))
        .with_align(segment.alignment.text_align())
        .with_shadow_mode(GlyphShadowMode::None)
}

fn gpu_pipeline_segment_label_color(segment: TimelineSegment) -> Color {
    if segment.color.alpha() <= 0.0 {
        return GPU_PIPELINE_LIGHT_LABEL_COLOR;
    }
    let luminance = gpu_pipeline_relative_luminance(segment.color);
    let light_contrast = gpu_pipeline_contrast_ratio(
        luminance,
        gpu_pipeline_relative_luminance(GPU_PIPELINE_LIGHT_LABEL_COLOR),
    );
    let dark_contrast = gpu_pipeline_contrast_ratio(
        luminance,
        gpu_pipeline_relative_luminance(GPU_PIPELINE_DARK_LABEL_COLOR),
    );
    if dark_contrast > light_contrast {
        GPU_PIPELINE_DARK_LABEL_COLOR
    } else {
        GPU_PIPELINE_LIGHT_LABEL_COLOR
    }
}

fn gpu_pipeline_contrast_ratio(background_luminance: f32, label_luminance: f32) -> f32 {
    let lighter = background_luminance.max(label_luminance);
    let darker = background_luminance.min(label_luminance);
    (lighter + GPU_PIPELINE_CONTRAST_LUMINANCE_OFFSET)
        / (darker + GPU_PIPELINE_CONTRAST_LUMINANCE_OFFSET)
}

fn gpu_pipeline_relative_luminance(color: Color) -> f32 {
    let color = color.to_srgba();
    gpu_pipeline_linear_srgb(color.red).mul_add(
        GPU_PIPELINE_LUMINANCE_RED_WEIGHT,
        gpu_pipeline_linear_srgb(color.green).mul_add(
            GPU_PIPELINE_LUMINANCE_GREEN_WEIGHT,
            gpu_pipeline_linear_srgb(color.blue) * GPU_PIPELINE_LUMINANCE_BLUE_WEIGHT,
        ),
    )
}

fn gpu_pipeline_linear_srgb(channel: f32) -> f32 {
    if channel <= GPU_PIPELINE_SRGB_LINEAR_BREAKPOINT {
        channel / GPU_PIPELINE_SRGB_LINEAR_DIVISOR
    } else {
        ((channel + GPU_PIPELINE_SRGB_TRANSFER_OFFSET) / GPU_PIPELINE_SRGB_TRANSFER_DIVISOR)
            .powf(GPU_PIPELINE_SRGB_TRANSFER_EXPONENT)
    }
}

/// Main-lane durations from one complete frame: schedules, render wait, and
/// extraction, placed consecutively on a shared duration scale.
fn main_lane_segments(b: &GpuPipelineBars) -> Vec<TimelineSegment> {
    let period = b.axis.max(GPU_PIPELINE_MIN_AXIS_MS);
    let work_end = b.main_work.clamp(0.0, period);
    let extract_start = (work_end + b.main_wait).clamp(work_end, period);
    let extract_end = (extract_start + b.main_extract).clamp(extract_start, period);
    vec![
        TimelineSegment::measured(0.0, work_end, GPU_PIPELINE_WORK_COLOR, "main work")
            .with_label_alignment(SegmentLabelAlignment::Leading),
        TimelineSegment::measured(
            work_end,
            extract_start,
            GPU_PIPELINE_WAIT_COLOR,
            "wait for render",
        ),
        TimelineSegment::measured(
            extract_start,
            extract_end,
            GPU_PIPELINE_EXTRACT_COLOR,
            "extract",
        )
        .with_label_alignment(SegmentLabelAlignment::Trailing),
    ]
}

/// Render-lane stage durations from the same service record. The image wait is
/// a presentable-image acquire, not a GPU execution timer.
fn render_lane_segments(b: &GpuPipelineBars) -> Vec<TimelineSegment> {
    let period = b.axis.max(GPU_PIPELINE_MIN_AXIS_MS);
    let prep_end = b.prep.clamp(0.0, period);
    let wait_end = (prep_end + b.gpu_wait).clamp(prep_end, period);
    let graph_end = (wait_end + b.render_graph).clamp(wait_end, period);
    let camera_end = (wait_end + b.camera_graphs).clamp(wait_end, graph_end);
    let cleanup_end = (graph_end + b.cleanup).clamp(graph_end, period);
    vec![
        TimelineSegment::measured(0.0, prep_end, GPU_PIPELINE_WORK_COLOR, "render prep")
            .with_label_alignment(SegmentLabelAlignment::Leading),
        TimelineSegment::measured(
            prep_end,
            wait_end,
            GPU_PIPELINE_WAIT_COLOR,
            "next image wait",
        ),
        TimelineSegment::measured(
            wait_end,
            camera_end,
            GPU_PIPELINE_RENDER_GRAPH_COLOR,
            "camera graphs",
        ),
        TimelineSegment::measured(
            camera_end,
            graph_end,
            GPU_PIPELINE_GRAPH_SUBMIT_COLOR,
            "submit",
        ),
        TimelineSegment::measured(graph_end, cleanup_end, GPU_PIPELINE_CLEANUP_COLOR, ""),
    ]
}

/// GPU lane, one frame: `current` pressure until `get_current_texture` releases,
/// `available` while the CPU records camera graphs and completes the submit
/// set, then `next` pressure. `next` starts at the end of the submit set rather
/// than at its start, because the frame marks do not record when the internal
/// `RenderQueue::submit` call ran inside the set. This lane is Bevy queue
/// pressure inferred from CPU-side frame marks, not measured hardware execution.
fn gpu_lane_segments(b: &GpuPipelineBars) -> Vec<TimelineSegment> {
    let period = b.axis.max(GPU_PIPELINE_MIN_AXIS_MS);
    let current_end = (b.prep + b.gpu_wait).clamp(0.0, period);
    let next_start = (current_end + b.camera_graphs + b.graph_submit).clamp(current_end, period);
    vec![
        TimelineSegment::inferred(0.0, current_end, GPU_PIPELINE_GPU_COLOR, "current")
            .with_label_alignment(SegmentLabelAlignment::Leading),
        TimelineSegment::measured(current_end, next_start, GPU_PIPELINE_GAP_COLOR, "available"),
        TimelineSegment::inferred(next_start, period, GPU_PIPELINE_GPU_COLOR, "next")
            .with_label_alignment(SegmentLabelAlignment::Trailing),
    ]
}

/// Per-second timeline animation state, held across frames in a `Local`.
///
/// Each frame's instantaneous lane values are summed into `accum` over one
/// sample period. At the period boundary the mean becomes `target`, the
/// on-screen `displayed` becomes `from`, and `morph` resets to 0. For the next
/// [`GPU_PIPELINE_MORPH_DURATION`] the bars slide `from` → `target`; after that they
/// hold until the following boundary.
#[derive(Default)]
struct GpuPipelineAnim {
    /// On-screen lane values this frame.
    displayed: GpuPipelineBars,
    /// Lane values at the start of the current morph.
    from:      GpuPipelineBars,
    /// The latest one-second mean — the morph destination and held picture.
    target:    GpuPipelineBars,
    /// Running field-wise sum of this period's frames.
    accum:     GpuPipelineBars,
    /// Frames summed into `accum`.
    samples:   u32,
    /// Seconds into the current sample period.
    sampled:   f32,
    /// Seconds since the last sample boundary (drives the morph fraction).
    morph:     f32,
    /// Whether the first sample has primed `displayed`/`from`/`target`.
    primed:    bool,
}

/// Samples the three lanes into a one-second mean and animates the panel toward
/// it: a [`GPU_PIPELINE_MORPH_DURATION`] slide at each boundary, then a still hold
/// for the rest of the second. These are duration bars from one complete
/// service record, with an inferred GPU lane rather than a shared-clock trace.
fn update_gpu_pipeline_panel(
    time: Res<Time>,
    timing: Res<FrameTiming>,
    shown: Res<GpuPipelineShown>,
    panels: Query<
        (
            Entity,
            &DiegeticPanel,
            &ComputedDiegeticPanel,
            &Resolved<FontUnit>,
        ),
        With<GpuPipelinePanel>,
    >,
    measurer: Res<DiegeticTextMeasurer>,
    mut commands: Commands,
    mut anim: Local<GpuPipelineAnim>,
    mut label_fit: Local<GpuPipelineLabelFit>,
) {
    let delta_secs = time.delta_secs();
    let main_span_ms = last_ms_or_zero(&timing.first)
        + last_ms_or_zero(&timing.pre_update)
        + last_ms_or_zero(&timing.state_transition)
        + last_ms_or_zero(&timing.fixed_update)
        + last_ms_or_zero(&timing.update)
        + last_ms_or_zero(&timing.spawn_scene)
        + last_ms_or_zero(&timing.post_update)
        + last_ms_or_zero(&timing.last);
    let prep = last_ms_or_zero(&timing.extract_commands)
        + last_ms_or_zero(&timing.prepare_assets)
        + last_ms_or_zero(&timing.prepare_and_queue);
    let gpu_wait = last_ms_or_zero(&timing.waiting_for_next_image);
    let render_graph = last_ms_or_zero(&timing.render_graph) + last_ms_or_zero(&timing.submit);
    let camera_graphs = last_ms_or_zero(&timing.render_graph);
    let graph_submit = last_ms_or_zero(&timing.submit);
    let cleanup = last_ms_or_zero(&timing.cleanup);
    let render_period = last_ms_or_zero(&timing.frame_time);
    let instant = GpuPipelineBars {
        axis: render_period,
        main_work: main_span_ms,
        main_wait: last_ms_or_zero(&timing.waiting_for_render),
        main_extract: last_ms_or_zero(&timing.extract),
        prep,
        gpu_wait,
        render_graph,
        camera_graphs,
        graph_submit,
        cleanup,
    };

    if !anim.primed {
        anim.displayed = instant;
        anim.from = instant;
        anim.target = instant;
        anim.primed = true;
    }

    anim.accum = anim.accum.add(instant);
    anim.samples += 1;
    anim.sampled += delta_secs;
    anim.morph += delta_secs;

    // Period boundary: average the second, start a new morph toward it.
    if anim.sampled >= GPU_PIPELINE_SAMPLE_PERIOD {
        let mean = anim.accum.scale(1.0 / anim.samples.max(1).to_f32());
        anim.from = anim.displayed;
        anim.target = mean;
        anim.accum = GpuPipelineBars::default();
        anim.samples = 0;
        anim.sampled = 0.0;
        anim.morph = 0.0;
    }

    // Smoothstep the morph fraction so the slide eases in and out.
    let raw = (anim.morph / GPU_PIPELINE_MORPH_DURATION).clamp(0.0, 1.0);
    let eased = raw * raw * 2.0f32.mul_add(-raw, 3.0);
    anim.displayed = anim.from.lerp(anim.target, eased);

    // Rebuild only while the bars are moving (the morph plus one settle frame);
    // during the hold the picture is unchanged, so the tree is left as-is.
    let moving = anim.morph <= GPU_PIPELINE_MORPH_DURATION + delta_secs;
    if matches!(*shown, GpuPipelineShown::Hidden) {
        return;
    }
    let displayed = anim.displayed;
    for (entity, panel, computed, font_unit) in &panels {
        let Ok(track_width) = laid_out_gpu_pipeline_track_width(panel.tree(), computed) else {
            continue;
        };
        let font_scale = panel.font_scale(font_unit.0.0);
        if !label_fit.needs_rebuild(moving, track_width, font_scale) {
            continue;
        }
        let metrics_changed = match &*label_fit {
            GpuPipelineLabelFit::AwaitingLayout => true,
            GpuPipelineLabelFit::BuiltAt {
                font_scale: previous,
                ..
            } => previous.to_bits() != font_scale.to_bits(),
        };
        if metrics_changed {
            *label_fit = GpuPipelineLabelFit::BuiltAt {
                track_width,
                font_scale,
                widths: GpuPipelineLabelWidths::measure(&measurer.measure_fn, font_scale),
            };
        }
        if let GpuPipelineLabelFit::BuiltAt {
            track_width: previous,
            widths,
            ..
        } = &mut *label_fit
        {
            if let Err(error) = commands.set_tree(
                entity,
                build_gpu_pipeline_tree(&displayed, track_width, widths),
            ) {
                error!("failed to replace GPU pipeline panel tree: {error}");
            } else {
                *previous = track_width;
            }
        }
    }
}

/// Toggles the GPU pipeline visualization (`M`): flips [`GpuPipelineShown`] and
/// sets the panel root's `Visibility` to match. Hidden panels also skip the
/// rebuild in [`update_gpu_pipeline_panel`].
fn toggle_gpu_pipeline(
    mut shown: ResMut<GpuPipelineShown>,
    panels: Query<Entity, With<GpuPipelinePanel>>,
    mut commands: Commands,
) {
    *shown = match *shown {
        GpuPipelineShown::Shown => GpuPipelineShown::Hidden,
        GpuPipelineShown::Hidden => GpuPipelineShown::Shown,
    };
    let visibility = match *shown {
        GpuPipelineShown::Shown => Visibility::Inherited,
        GpuPipelineShown::Hidden => Visibility::Hidden,
    };
    for entity in &panels {
        commands.entity(entity).insert(visibility);
    }
}

/// Linear interpolation from `from` to `to` by fraction `t`.
const fn lerp(from: f32, to: f32, t: f32) -> f32 { from + (to - from) * t }
