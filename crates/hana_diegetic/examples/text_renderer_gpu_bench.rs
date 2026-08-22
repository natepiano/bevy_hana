//! Runtime render benchmark for the slug text renderer.
//!
//! This is a Bevy render-loop benchmark, not a Criterion CPU benchmark. It is
//! a slug regression/optimization harness: `slug` renders a grid of world
//! text, `empty` renders the same scene with no text as a baseline.
//!
//! ```bash
//! cargo run -p hana_diegetic --release --example text_renderer_gpu_bench -- --mode slug
//! cargo run -p hana_diegetic --release --example text_renderer_gpu_bench -- --mode empty
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
use bevy_kana::ToF32;
use bevy_kana::ToF64;
use hana_diegetic::DiegeticText;
use hana_diegetic::DiegeticUiPlugin;
use hana_diegetic::GlyphShadowMode;

const BENCH_TEXT: &str = "Typography";
const CAMERA_DISTANCE: f32 = 9.0;
const DEFAULT_INSTANCES: usize = 720;
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
    Slug,
}

impl BenchMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "empty" => Some(Self::Empty),
            "slug" => Some(Self::Slug),
            _ => None,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Slug => "slug",
        }
    }
}

#[derive(Resource)]
struct BenchConfig {
    instances:     usize,
    mode:          BenchMode,
    sample_frames: usize,
    warmup_frames: usize,
}

impl BenchConfig {
    fn from_args() -> Self {
        let mut config = Self {
            instances:     DEFAULT_INSTANCES,
            mode:          BenchMode::Slug,
            sample_frames: DEFAULT_SAMPLE_FRAMES,
            warmup_frames: DEFAULT_WARMUP_FRAMES,
        };
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--mode" => {
                    let value = required_arg(&mut args, "--mode");
                    config.mode = BenchMode::parse(&value).unwrap_or_else(|| {
                        eprintln!("unsupported --mode '{value}'; use empty or slug");
                        process::exit(2);
                    });
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
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, collect_samples)
        .run();
}

fn setup(mut commands: Commands, config: Res<BenchConfig>) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, CAMERA_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    if config.mode == BenchMode::Empty {
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
        "mode={} instances={} warmup_frames={} sample_frames={}",
        config.mode.label(),
        config.instances,
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
        "text_renderer_gpu_bench --mode <empty|slug> \
        [--instances N] [--warmup-frames N] [--sample-frames N]"
    );
}

fn required_arg(args: &mut impl Iterator<Item = String>, name: &str) -> String {
    args.next().unwrap_or_else(|| {
        eprintln!("{name} requires a value");
        process::exit(2);
    })
}
