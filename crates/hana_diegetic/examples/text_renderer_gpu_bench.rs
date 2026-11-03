//! Runtime render benchmark for the slug text renderer.
//!
//! This is a Bevy render-loop benchmark, not a Criterion CPU benchmark. It is
//! a slug regression/optimization harness: `slug` renders a grid of world
//! text, `empty` renders the same scene with no text as a baseline, and
//! `panels` repeats the Criterion status-panel fixture through the renderer.
//! `rich-panels` and `sibling-panels` compare the same footer scene drawn as
//! one rich hint paragraph or separate hint leaves, with a count column.
//!
//! ```bash
//! cargo run -p hana_diegetic --release --example text_renderer_gpu_bench -- --mode slug
//! cargo run -p hana_diegetic --release --example text_renderer_gpu_bench -- --mode empty
//! cargo run -p hana_diegetic --release --example text_renderer_gpu_bench -- --mode panels --panel-count 16
//! ```
//!
//! Render diagnostics report Bevy's CPU render metrics here. GPU elapsed
//! diagnostics may be unavailable or zero on Metal, so this benchmark treats
//! them as optional report data rather than a reliable timing source.

use std::collections::BTreeMap;
use std::env;
use std::process;

use bevy::app::AppExit;
use bevy::diagnostic::DiagnosticsStore;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::App;
use bevy::prelude::Camera3d;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::MessageWriter;
use bevy::prelude::PluginGroup;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::WindowPlugin;
use bevy::prelude::default;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy::window::MonitorSelection;
use bevy::window::PresentMode;
use bevy::window::WindowPosition;
use bevy::window::WindowResolution;
use bevy::winit::WinitSettings;
use bevy_brp_extras::BrpExtrasPlugin;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::Dimension;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::Padding;
use hana_diegetic::Sizing;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_kana::ToF32;
use hana_kana::ToF64;

const BENCH_TEXT: &str = "Typography";
const CAMERA_DISTANCE: f32 = 9.0;
const DEFAULT_INSTANCES: usize = 720;
const DEFAULT_PANEL_COUNT: usize = 1;
const PANEL_SIZE: f32 = 160.0;
const PANEL_TEXT_SIZE: f32 = 10.0;
const PANEL_STATUS_ROWS: [(&str, &str); 5] = [
    ("fps:", "60"),
    ("frame ms:", "16.7"),
    ("radius:", "0.3"),
    ("entities:", "1024"),
    ("triangles:", "128000"),
];
const PANEL_SPACING: f32 = 1.12;
const DEFAULT_SAMPLE_FRAMES: usize = 240;
const DEFAULT_WARMUP_FRAMES: usize = 180;
const GRID_COLUMNS: usize = 24;
const GRID_SPACING_X: f32 = 0.56;
const GRID_SPACING_Y: f32 = 0.16;
const TEXT_SIZE: f32 = 0.12;
const WINDOW_HEIGHT: u32 = 900;
const WINDOW_WIDTH: u32 = 1600;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BenchMode {
    Empty,
    Panels,
    RichPanels,
    SiblingPanels,
    Slug,
}

impl BenchMode {
    fn parse(value: &str) -> Self {
        match value {
            "empty" => Self::Empty,
            "panels" => Self::Panels,
            "rich-panels" => Self::RichPanels,
            "sibling-panels" => Self::SiblingPanels,
            "slug" => Self::Slug,
            _ => {
                eprintln!(
                    "unsupported --mode '{value}'; use empty, slug, panels, sibling-panels or rich-panels"
                );
                process::exit(2);
            },
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Panels => "panels",
            Self::RichPanels => "rich-panels",
            Self::SiblingPanels => "sibling-panels",
            Self::Slug => "slug",
        }
    }
}

#[derive(Resource)]
struct BenchConfig {
    instances:     usize,
    mode:          BenchMode,
    panel_count:   usize,
    sample_frames: usize,
    warmup_frames: usize,
}

impl BenchConfig {
    fn from_args() -> Self {
        let mut config = Self {
            instances:     DEFAULT_INSTANCES,
            mode:          BenchMode::Slug,
            panel_count:   DEFAULT_PANEL_COUNT,
            sample_frames: DEFAULT_SAMPLE_FRAMES,
            warmup_frames: DEFAULT_WARMUP_FRAMES,
        };
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--mode" => {
                    let value = required_arg(&mut args, "--mode");
                    config.mode = BenchMode::parse(&value);
                },
                "--panel-count" => {
                    config.panel_count =
                        parse_usize(required_arg(&mut args, "--panel-count"), "--panel-count");
                    if config.panel_count == 0 {
                        eprintln!("--panel-count must be greater than zero");
                        process::exit(2);
                    }
                },
                "--instances" => {
                    config.instances =
                        parse_usize(required_arg(&mut args, "--instances"), "--instances");
                },
                "--sample-frames" => {
                    config.sample_frames = parse_usize(
                        required_arg(&mut args, "--sample-frames"),
                        "--sample-frames",
                    );
                },
                "--warmup-frames" => {
                    config.warmup_frames = parse_usize(
                        required_arg(&mut args, "--warmup-frames"),
                        "--warmup-frames",
                    );
                },
                "--help" | "-h" => {
                    print_help();
                    process::exit(0);
                },
                other => {
                    eprintln!("unsupported argument '{other}'");
                    print_help();
                    process::exit(2);
                },
            }
        }
        config
    }
}

#[derive(Default, Resource)]
struct BenchState {
    frame:        usize,
    frame_time:   RunningStats,
    reported:     bool,
    render_cpu:   RunningStats,
    render_gpu:   RunningStats,
    render_paths: BTreeMap<String, RunningStats>,
    panel_paths:  BTreeMap<String, RunningStats>,
}

#[derive(Clone, Debug)]
struct RunningStats {
    max:     f64,
    min:     f64,
    samples: usize,
    sum:     f64,
    sum_sq:  f64,
}

#[derive(Default)]
struct RenderElapsedTotals {
    cpu:     f64,
    gpu:     f64,
    has_cpu: bool,
    has_gpu: bool,
}

impl Default for RunningStats {
    fn default() -> Self {
        Self {
            max:     f64::NEG_INFINITY,
            min:     f64::INFINITY,
            samples: 0,
            sum:     0.0,
            sum_sq:  0.0,
        }
    }
}

impl RunningStats {
    fn push(&mut self, value: f64) {
        self.samples += 1;
        self.sum += value;
        self.sum_sq = value.mul_add(value, self.sum_sq);
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }

    fn mean(&self) -> f64 { self.sum / self.samples.to_f64() }

    fn stddev(&self) -> f64 {
        let mean = self.mean();
        mean.mul_add(-mean, self.sum_sq / self.samples.to_f64())
            .max(0.0)
            .sqrt()
    }
}

fn main() {
    let config = BenchConfig::from_args();
    let mode = config.mode;

    App::new()
        .insert_resource(config)
        .insert_resource(WinitSettings::continuous())
        .init_resource::<BenchState>()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("text_renderer_gpu_bench {}", mode.label()),
                present_mode: PresentMode::AutoNoVsync,
                position: WindowPosition::Centered(MonitorSelection::Primary),
                resolution: WindowResolution::new(WINDOW_WIDTH, WINDOW_HEIGHT),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            RenderDiagnosticsPlugin,
            DiegeticUiPlugin,
            BrpExtrasPlugin::default(),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, collect_samples)
        .run();
}

fn setup(mut commands: Commands, config: Res<BenchConfig>) {
    let camera_distance = if matches!(
        config.mode,
        BenchMode::Panels | BenchMode::RichPanels | BenchMode::SiblingPanels
    ) {
        (panel_columns(config.panel_count).to_f32() * PANEL_SPACING * 1.5).max(CAMERA_DISTANCE)
    } else {
        CAMERA_DISTANCE
    };
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, camera_distance).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    if config.mode == BenchMode::Empty {
        return;
    }
    if matches!(
        config.mode,
        BenchMode::Panels | BenchMode::RichPanels | BenchMode::SiblingPanels
    ) {
        setup_panels(&mut commands, config.panel_count, config.mode);
        return;
    }

    let rows = config.instances.div_ceil(GRID_COLUMNS);
    let origin_x = -GRID_SPACING_X * GRID_COLUMNS.saturating_sub(1).to_f32() * 0.5;
    let origin_y = GRID_SPACING_Y * rows.saturating_sub(1).to_f32() * 0.5;
    for index in 0..config.instances {
        let column = index % GRID_COLUMNS;
        let row = index / GRID_COLUMNS;
        commands.spawn(
            DiegeticText::world(BENCH_TEXT)
                .size(TEXT_SIZE)
                .color(Color::WHITE)
                .shadow_mode(GlyphShadowMode::None)
                .transform(Transform::from_xyz(
                    column.to_f32().mul_add(GRID_SPACING_X, origin_x),
                    row.to_f32().mul_add(-GRID_SPACING_Y, origin_y),
                    0.0,
                ))
                .build(),
        );
    }
}

fn panel_columns(panel_count: usize) -> usize {
    let columns = panel_count.isqrt();
    columns + usize::from(columns * columns < panel_count)
}

fn setup_panels(commands: &mut Commands, panel_count: usize, mode: BenchMode) {
    let columns = panel_columns(panel_count);
    let rows = panel_count.div_ceil(columns);
    let origin_x = -PANEL_SPACING * columns.saturating_sub(1).to_f32() * 0.5;
    let origin_y = PANEL_SPACING * rows.saturating_sub(1).to_f32() * 0.5;
    for index in 0..panel_count {
        commands.spawn((
            match mode {
                BenchMode::RichPanels => build_footer_panel(FooterBenchmarkScene::RichParagraph),
                BenchMode::SiblingPanels => {
                    build_footer_panel(FooterBenchmarkScene::SiblingTextLeaves)
                },
                _ => build_status_panel(),
            },
            Transform::from_xyz(
                (index % columns).to_f32().mul_add(PANEL_SPACING, origin_x),
                (index / columns).to_f32().mul_add(-PANEL_SPACING, origin_y),
                0.0,
            ),
        ));
    }
}

// Mirrors `benches/fixtures/panels.rs` at five rows. Keep this local: including
// feature-gated bench modules in a default-feature example makes mend narrow
// their shared APIs without seeing the benchmark callers.
fn build_status_panel() -> DiegeticPanel {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(8.0))
            .gap(5.0)
            .background(Color::srgb_u8(180, 96, 122)),
    );
    build_status_header(&mut builder);
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(4.0))
            .background(Color::srgb_u8(74, 196, 172)),
        |_| {},
    );
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(Color::srgb_u8(22, 28, 34)),
        |builder| {
            builder.with(
                El::column()
                    .width(Sizing::GROW)
                    .padding(Padding::all(5.0))
                    .gap(2.0),
                |builder| {
                    for (label, value) in PANEL_STATUS_ROWS {
                        builder.with(
                            El::row().width(Sizing::GROW).height(Sizing::FIT),
                            |builder| {
                                builder.text((label, TextStyle::new(PANEL_TEXT_SIZE)));
                                builder.with(
                                    El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                                    |_| {},
                                );
                                builder.text((value, TextStyle::new(PANEL_TEXT_SIZE)));
                            },
                        );
                    }
                },
            );
        },
    );
    let unit = Unit::Custom(1.0 / PANEL_SIZE);
    let dimension = Dimension {
        value: PANEL_SIZE,
        unit:  Some(unit),
    };
    DiegeticPanel::world()
        .size(Sizing::Fixed(dimension), Sizing::Fixed(dimension))
        .font_unit(unit)
        .with_tree(builder.build())
        .build()
        .unwrap_or_else(|error| {
            eprintln!("invalid benchmark status panel: {error}");
            process::exit(1);
        })
}

fn build_status_header(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::grow_range(PANEL_TEXT_SIZE, 20.0))
            .padding(Padding::new(5.0, 5.0, 4.0, 4.0))
            .align_y(AlignY::Center)
            .background(Color::srgb_u8(52, 98, 90)),
        |builder| {
            builder.with(
                El::row().width(Sizing::GROW).height(Sizing::FIT),
                |builder| {
                    builder.with(
                        El::new().width(Sizing::FIT).height(Sizing::GROW),
                        |builder| {
                            builder.text(("STATUS", TextStyle::new(PANEL_TEXT_SIZE)));
                        },
                    );
                    builder.with(
                        El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                        |_| {},
                    );
                    builder.with(
                        El::new()
                            .width(Sizing::FIT)
                            .height(Sizing::GROW)
                            .align_x(AlignX::Right),
                        |builder| {
                            builder.text(("BENCH", TextStyle::new(PANEL_TEXT_SIZE)));
                        },
                    );
                },
            );
        },
    );
}

fn collect_samples(
    config: Res<BenchConfig>,
    diagnostics: Res<DiagnosticsStore>,
    mut exit: MessageWriter<AppExit>,
    mut state: ResMut<BenchState>,
) {
    if state.reported {
        exit.write(AppExit::Success);
        return;
    }

    state.frame += 1;
    if state.frame <= config.warmup_frames {
        return;
    }

    if let Some(frame_ms) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(bevy::diagnostic::Diagnostic::value)
    {
        state.frame_time.push(frame_ms);
    }

    let mut elapsed_totals = RenderElapsedTotals::default();
    for diagnostic in diagnostics.iter() {
        let path = diagnostic.path().as_str();
        if path.starts_with("hana_diegetic/")
            && let Some(value) = diagnostic.value()
        {
            state
                .panel_paths
                .entry(path.to_owned())
                .or_default()
                .push(value);
        }
        if path.starts_with("render/")
            && (path.ends_with("/elapsed_cpu") || path.ends_with("/elapsed_gpu"))
            && let Some(value) = diagnostic.value()
        {
            if path.ends_with("/elapsed_cpu") {
                elapsed_totals.cpu += value;
                elapsed_totals.has_cpu = true;
            } else {
                elapsed_totals.gpu += value;
                elapsed_totals.has_gpu = true;
            }
            state
                .render_paths
                .entry(path.to_owned())
                .or_default()
                .push(value);
        }
    }
    if elapsed_totals.has_cpu {
        state.render_cpu.push(elapsed_totals.cpu);
    }
    if elapsed_totals.has_gpu {
        state.render_gpu.push(elapsed_totals.gpu);
    }

    if state.frame >= config.warmup_frames + config.sample_frames {
        state.reported = true;
        print_report(&config, &state);
        exit.write(AppExit::Success);
    }
}

fn print_report(config: &BenchConfig, state: &BenchState) {
    println!("text_renderer_gpu_bench");
    println!(
        "mode={} instances={} panel_count={} panel_rows={} warmup_frames={} sample_frames={}",
        config.mode.label(),
        config.instances,
        config.panel_count,
        PANEL_STATUS_ROWS.len(),
        config.warmup_frames,
        config.sample_frames
    );
    print_stats("frame_time", &state.frame_time);

    print_stats("render_elapsed_cpu_sum", &state.render_cpu);
    if state.render_gpu.samples == 0 {
        println!("render_elapsed_gpu_sum unavailable");
    } else {
        print_stats("render_elapsed_gpu_sum", &state.render_gpu);
    }

    println!("render_paths");
    for (path, stats) in &state.render_paths {
        print_stats(path, stats);
    }
    println!("diegetic_paths");
    for (path, stats) in &state.panel_paths {
        if path.ends_with("_ms") {
            print_stats(path, stats);
        } else {
            print_counter_stats(path, stats);
        }
    }
}

fn print_counter_stats(label: &str, stats: &RunningStats) {
    println!(
        "{label}: mean={:.4} stddev={:.4} min={:.4} max={:.4} samples={}",
        stats.mean(),
        stats.stddev(),
        stats.min,
        stats.max,
        stats.samples
    );
}

fn print_stats(label: &str, stats: &RunningStats) {
    if stats.samples == 0 {
        println!("{label}: no samples");
        return;
    }
    println!(
        "{label}: mean_ms={:.4} stddev_ms={:.4} min_ms={:.4} max_ms={:.4} samples={}",
        stats.mean(),
        stats.stddev(),
        stats.min,
        stats.max,
        stats.samples
    );
}

fn parse_usize(value: String, name: &str) -> usize {
    value.parse().unwrap_or_else(|_| {
        eprintln!("{name} must be a positive integer, got '{value}'");
        process::exit(2);
    })
}

fn print_help() {
    eprintln!(
        "text_renderer_gpu_bench --mode <empty|slug|panels|sibling-panels|rich-panels> \
        [--instances N] [--panel-count N] [--warmup-frames N] [--sample-frames N]"
    );
}

fn required_arg(args: &mut impl Iterator<Item = String>, name: &str) -> String {
    args.next().unwrap_or_else(|| {
        eprintln!("{name} requires a value");
        process::exit(2);
    })
}

// Local copies preserve the feature-gated Criterion fixture's visibility.
#[derive(Clone, Copy)]
enum FooterBenchmarkScene {
    RichParagraph,
    SiblingTextLeaves,
}

fn build_footer_panel(scene: FooterBenchmarkScene) -> DiegeticPanel {
    const SIZE: f32 = 480.0;
    let dim = Color::srgb(0.6, 0.6, 0.6);
    let base = TextStyle::new(PANEL_TEXT_SIZE).with_color(dim);
    let bright = hana_diegetic::SpanStyle::new().with_color(Color::WHITE);
    let separator =
        hana_diegetic::SpanStyle::new().with_word_spacing((-PANEL_TEXT_SIZE).mul_add(0.6, 4.0));
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .padding(Padding::all(8.0))
            .gap(2.0)
            .background(Color::srgb_u8(22, 28, 34)),
    );
    for row in 0..PANEL_STATUS_ROWS.len() {
        builder.with(
            El::row()
                .width(Sizing::GROW)
                .height(Sizing::FIT)
                .gap(4.0)
                .align_y(AlignY::Center),
            |builder| {
                match scene {
                    FooterBenchmarkScene::RichParagraph => {
                        builder.rich_text(hana_diegetic::RichText::new(base.clone()), |rich| {
                            for (index, (key, action)) in [
                                ("↑↓", "to navigate"),
                                ("⏎", "to use"),
                                ("esc", "to dismiss"),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                if index > 0 {
                                    rich.span((" ", separator)).span("-").span((" ", separator));
                                }
                                rich.span((key, bright)).span((" ", separator)).span(action);
                            }
                        });
                    },
                    FooterBenchmarkScene::SiblingTextLeaves => {
                        for (index, (key, action)) in [
                            ("↑↓", "to navigate"),
                            ("⏎", "to use"),
                            ("esc", "to dismiss"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if index > 0 {
                                builder.text(
                                    hana_diegetic::Text::new("-", base.clone())
                                        .wrap(hana_diegetic::TextWrap::None),
                                );
                            }
                            builder.text(
                                hana_diegetic::Text::new(
                                    key,
                                    base.clone().with_color(Color::WHITE),
                                )
                                .wrap(hana_diegetic::TextWrap::None),
                            );
                            builder.text(
                                hana_diegetic::Text::new(action, base.clone())
                                    .wrap(hana_diegetic::TextWrap::None),
                            );
                        }
                    },
                }
                builder.with(
                    El::new().width(Sizing::GROW).height(Sizing::fixed(1.0)),
                    |_| {},
                );
                builder.text(
                    hana_diegetic::Text::new(
                        format!("{} of {}", row + 1, PANEL_STATUS_ROWS.len()),
                        base.clone(),
                    )
                    .wrap(hana_diegetic::TextWrap::None),
                );
            },
        );
    }
    let unit = Unit::Custom(1.0 / SIZE);
    let dimension = Dimension {
        value: SIZE,
        unit:  Some(unit),
    };
    DiegeticPanel::world()
        .size(Sizing::Fixed(dimension), Sizing::Fixed(dimension))
        .font_unit(unit)
        .with_tree(builder.build())
        .build()
        .unwrap_or_else(|error| {
            eprintln!("invalid benchmark footer panel: {error}");
            process::exit(1);
        })
}
