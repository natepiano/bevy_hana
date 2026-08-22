use std::fmt::Write as _;

use bevy::diagnostic::Diagnostic;
use bevy::diagnostic::DiagnosticsStore;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::Component;
use bevy::prelude::KeyCode;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::Text;
use bevy::prelude::Time;
use bevy::prelude::Timer;
use bevy::prelude::With;
use bevy_kana::ToF64;

use crate::benchmark_state::BenchmarkMode;
use crate::benchmark_state::BenchmarkPhase;
use crate::benchmark_state::BenchmarkState;
use crate::benchmark_state::outline_method_label;
use crate::constants::AUTO_EXIT_DELAY_SECS;
use crate::constants::AUTO_STARTUP_DELAY_SECS;
use crate::constants::BENCHMARK_LABEL;
use crate::constants::BENCHMARK_MODE_AUTO_LABEL;
use crate::constants::BENCHMARK_MODE_INTERACTIVE_LABEL;
use crate::constants::BENCHMARK_PHASE_ANALYZE_LABEL;
use crate::constants::BENCHMARK_PHASE_IDLE_LABEL;
use crate::constants::BENCHMARK_PHASE_SETUP_LABEL;
use crate::constants::HEADS_UP_DISPLAY_CONTROLS;
use crate::constants::HEADS_UP_DISPLAY_FPS_PRECISION;
use crate::constants::HEADS_UP_DISPLAY_FPS_WIDTH;
use crate::constants::HEADS_UP_DISPLAY_MILLISECONDS_PRECISION;
use crate::constants::HEADS_UP_DISPLAY_RESULTS_HEADER;
use crate::constants::HEADS_UP_DISPLAY_SECONDS_PRECISION;
use crate::constants::MEASURE_FRAMES;
use crate::constants::MILLISECONDS_PER_SECOND;
use crate::constants::OUTLINE_PRESENCE_DISABLED_LABEL;
use crate::constants::OUTLINE_PRESENCE_ENABLED_LABEL;
use crate::constants::RESULT_LABEL_PADDING;
use crate::constants::SCENARIOS;
use crate::constants::WARMUP_FRAMES;
use crate::scenarios::ScenarioDefinition;

#[derive(Component)]
pub(super) struct HudText;

#[derive(Resource)]
pub(super) struct HudUpdateTimer(pub(super) Timer);

struct LiveMetrics {
    fps:        f64,
    frame_time: f64,
}

pub(super) fn update_hud(
    state: Res<BenchmarkState>,
    diagnostics: Res<DiagnosticsStore>,
    mut text: Single<&mut Text, With<HudText>>,
    time: Res<Time>,
    mut hud_timer: ResMut<HudUpdateTimer>,
) {
    if !hud_timer.0.tick(time.delta()).just_finished() {
        return;
    }
    text.0 = build_hud_text(&state, &diagnostics);
}

fn build_hud_text(state: &BenchmarkState, diagnostics: &DiagnosticsStore) -> String {
    let scenario = &SCENARIOS[state.current_scenario];
    let benchmark_mode_name = benchmark_mode_label(&state.benchmark_mode);
    let benchmark_phase_info = benchmark_phase_label(state);
    let progress = auto_progress_label(state);
    let col = results_label_width();
    let outline_method_name = outline_method_label(state.outline_method);
    let live_metrics = live_metrics(diagnostics);
    let bench_stats = benchmark_stats_line(state.frame_times.as_slice(), col);

    let mut hud = format!(
        "[{benchmark_mode_name}] {}{progress}  Mode: {outline_method_name}\n{benchmark_phase_info}\n\n{BENCHMARK_LABEL:<col$}FPS: {fps:<fps_width$.fps_precision$}  Frame: {frame_time:.milliseconds_precision$}ms{bench_stats}",
        scenario.name,
        fps = live_metrics.fps,
        fps_precision = HEADS_UP_DISPLAY_FPS_PRECISION,
        fps_width = HEADS_UP_DISPLAY_FPS_WIDTH,
        frame_time = live_metrics.frame_time,
        milliseconds_precision = HEADS_UP_DISPLAY_MILLISECONDS_PRECISION,
    );

    append_results_section(&mut hud, state, col, outline_method_name);
    hud.push_str(HEADS_UP_DISPLAY_CONTROLS);
    hud
}

const fn benchmark_mode_label(mode: &BenchmarkMode) -> &'static str {
    match mode {
        BenchmarkMode::Auto => BENCHMARK_MODE_AUTO_LABEL,
        BenchmarkMode::Interactive => BENCHMARK_MODE_INTERACTIVE_LABEL,
    }
}

fn benchmark_phase_label(state: &BenchmarkState) -> String {
    match state.benchmark_phase {
        BenchmarkPhase::Idle => BENCHMARK_PHASE_IDLE_LABEL.to_string(),
        BenchmarkPhase::StartupDelay => {
            let remaining = AUTO_STARTUP_DELAY_SECS - state.startup_timer.elapsed_secs();
            format!("Starting in {remaining:.HEADS_UP_DISPLAY_SECONDS_PRECISION$}s...")
        },
        BenchmarkPhase::Setup => BENCHMARK_PHASE_SETUP_LABEL.to_string(),
        BenchmarkPhase::Warmup => format!("Warmup {}/{}", state.frame_counter, WARMUP_FRAMES),
        BenchmarkPhase::Measure => format!("Measuring {}/{}", state.frame_counter, MEASURE_FRAMES),
        BenchmarkPhase::Analyze => BENCHMARK_PHASE_ANALYZE_LABEL.to_string(),
        BenchmarkPhase::ExitDelay => {
            let remaining = AUTO_EXIT_DELAY_SECS - state.exit_timer.elapsed_secs();
            format!("Exiting in {remaining:.HEADS_UP_DISPLAY_SECONDS_PRECISION$}s...")
        },
    }
}

fn auto_progress_label(state: &BenchmarkState) -> String {
    if state.benchmark_mode == BenchmarkMode::Auto {
        format!(" ({}/{})", state.current_scenario + 1, SCENARIOS.len())
    } else {
        String::new()
    }
}

fn results_label_width() -> usize {
    let mut max_label_len = BENCHMARK_LABEL.len();
    for scenario in SCENARIOS {
        max_label_len = max_label_len.max(scenario.name.len() + RESULT_LABEL_PADDING);
    }
    max_label_len + 1
}

fn live_metrics(diagnostics: &DiagnosticsStore) -> LiveMetrics {
    LiveMetrics {
        fps:        diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(Diagnostic::smoothed)
            .unwrap_or(0.0),
        frame_time: diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
            .and_then(Diagnostic::smoothed)
            .unwrap_or(0.0),
    }
}

fn benchmark_stats_line(frame_times: &[f64], col: usize) -> String {
    if frame_times.is_empty() {
        return String::new();
    }

    let sum: f64 = frame_times.iter().sum();
    let average = sum / frame_times.len().to_f64();
    let average_frames_per_second = MILLISECONDS_PER_SECOND / average;
    format!(
        "\n{BENCHMARK_LABEL:<col$}FPS: {average_frames_per_second:<HEADS_UP_DISPLAY_FPS_WIDTH$.HEADS_UP_DISPLAY_FPS_PRECISION$}  Frame: {average:.HEADS_UP_DISPLAY_MILLISECONDS_PRECISION$}ms"
    )
}

fn append_results_section(
    hud: &mut String,
    state: &BenchmarkState,
    col: usize,
    outline_method_name: &str,
) {
    hud.push_str(HEADS_UP_DISPLAY_RESULTS_HEADER);
    for scenario in SCENARIOS {
        append_scenario_results(hud, state, scenario, col, outline_method_name);
    }
}

fn append_scenario_results(
    hud: &mut String,
    state: &BenchmarkState,
    scenario: &ScenarioDefinition,
    col: usize,
    outline_method_name: &str,
) {
    let key_char = key_to_char(scenario.key);
    for (index, suffix) in [
        OUTLINE_PRESENCE_DISABLED_LABEL,
        OUTLINE_PRESENCE_ENABLED_LABEL,
    ]
    .iter()
    .enumerate()
    {
        let result_name = format!("{} {suffix} ({outline_method_name})", scenario.name);
        let label = if index == 0 {
            format!("{key_char} {result_name}:")
        } else {
            format!("  {result_name}:")
        };
        append_result_row(hud, state, &result_name, &label, col);
    }
}

fn append_result_row(
    hud: &mut String,
    state: &BenchmarkState,
    result_name: &str,
    label: &str,
    col: usize,
) {
    if let Some(result) = state
        .results
        .iter()
        .find(|result| result.name == result_name)
    {
        let _ = write!(
            hud,
            "\n{label:<col$}FPS: {average_frames_per_second:<fps_width$.fps_precision$}  Frame: {average:.milliseconds_precision$}ms  median: {median:.milliseconds_precision$}ms  95th: {percentile_95:.milliseconds_precision$}ms",
            average_frames_per_second = result.average_frames_per_second(),
            average = result.average,
            median = result.median,
            percentile_95 = result.percentile_95,
            fps_precision = HEADS_UP_DISPLAY_FPS_PRECISION,
            fps_width = HEADS_UP_DISPLAY_FPS_WIDTH,
            milliseconds_precision = HEADS_UP_DISPLAY_MILLISECONDS_PRECISION,
        );
    } else {
        let _ = write!(hud, "\n{label:<col$}---");
    }
}

const fn key_to_char(key: KeyCode) -> char {
    match key {
        KeyCode::Digit0 => '0',
        KeyCode::Digit1 => '1',
        KeyCode::Digit2 => '2',
        KeyCode::Digit3 => '3',
        KeyCode::Digit4 => '4',
        KeyCode::Digit5 => '5',
        KeyCode::Digit6 => '6',
        KeyCode::Digit7 => '7',
        KeyCode::Digit8 => '8',
        KeyCode::Digit9 => '9',
        _ => '?',
    }
}
