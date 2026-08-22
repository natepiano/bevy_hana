use std::process::exit;

use bevy::ecs::system::SystemParam;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::Camera3d;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Real;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::info;

use crate::benchmark_state::BenchmarkMode;
use crate::benchmark_state::BenchmarkPhase;
use crate::benchmark_state::BenchmarkState;
use crate::benchmark_state::ExitBehavior;
use crate::benchmark_state::OutlinePresence;
use crate::benchmark_state::next_outline_method;
use crate::benchmark_state::outline_method_label;
use crate::constants::AUTO_BENCHMARK_COMPLETE_MESSAGE;
use crate::constants::AUTO_BENCHMARK_START_MESSAGE;
use crate::constants::AUTO_EXIT_DELAY_SECS;
use crate::constants::AUTO_STARTUP_DELAY_SECS;
use crate::constants::BENCHMARK_RESULTS_FRAMES_PER_SECOND_PRECISION;
use crate::constants::BENCHMARK_RESULTS_MILLISECONDS_PRECISION;
use crate::constants::EXITING_MESSAGE;
use crate::constants::MEASURE_FRAMES;
use crate::constants::MILLISECONDS_PER_SECOND;
use crate::constants::OUTLINE_PRESENCE_DISABLED_LABEL;
use crate::constants::OUTLINE_PRESENCE_ENABLED_LABEL;
use crate::constants::SCENARIO_SWITCH_MESSAGE;
use crate::constants::SCENARIOS;
use crate::constants::STARTUP_COMPLETE_MESSAGE;
use crate::constants::WARMUP_FRAMES;
use crate::grid::BenchmarkEntity;
use crate::results::compute_statistics;
use crate::results::write_results;
use crate::scenarios::spawn_scenario;
use crate::viewport::compute_viewport_info;

#[derive(SystemParam)]
pub(super) struct BenchmarkTickParams<'w, 's> {
    commands:     Commands<'w, 's>,
    state:        ResMut<'w, BenchmarkState>,
    meshes:       ResMut<'w, Assets<Mesh>>,
    materials:    ResMut<'w, Assets<StandardMaterial>>,
    time:         Res<'w, Time<Real>>,
    entities:     Query<'w, 's, Entity, With<BenchmarkEntity>>,
    camera_query: Query<'w, 's, (&'static Transform, &'static Projection), With<Camera3d>>,
    windows:      Query<'w, 's, &'static mut Window>,
}

pub(super) fn benchmark_tick(benchmark_tick_params: BenchmarkTickParams<'_, '_>) {
    let BenchmarkTickParams {
        mut commands,
        mut state,
        mut meshes,
        mut materials,
        time,
        entities,
        camera_query,
        mut windows,
    } = benchmark_tick_params;

    match state.benchmark_phase {
        BenchmarkPhase::Idle => {},
        BenchmarkPhase::StartupDelay => {
            handle_startup_delay_phase(&mut state, &mut windows, &time);
        },
        BenchmarkPhase::Setup => {
            handle_setup_phase(
                &mut commands,
                &mut state,
                &mut meshes,
                &mut materials,
                &entities,
                &camera_query,
                &windows,
            );
        },
        BenchmarkPhase::Warmup => advance_warmup_phase(&mut state),
        BenchmarkPhase::Measure => measure_phase(&mut state, &time),
        BenchmarkPhase::Analyze => handle_analyze_phase(&mut state),
        BenchmarkPhase::ExitDelay => handle_exit_delay_phase(&mut state, &time),
    }
}

fn handle_startup_delay_phase(
    state: &mut BenchmarkState,
    windows: &mut Query<&mut Window>,
    time: &Time<Real>,
) {
    if state.startup_timer.elapsed().is_zero()
        && let Ok(mut window) = windows.single_mut()
    {
        window.focused = true;
        info!("Auto mode: focusing window, waiting {AUTO_STARTUP_DELAY_SECS}s before starting");
    }

    state.startup_timer.tick(time.delta());
    if state.startup_timer.just_finished() {
        info!("{}", STARTUP_COMPLETE_MESSAGE);
        state.benchmark_phase = BenchmarkPhase::Setup;
    }
}

fn handle_setup_phase(
    commands: &mut Commands<'_, '_>,
    state: &mut BenchmarkState,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    entities: &Query<Entity, With<BenchmarkEntity>>,
    camera_query: &Query<(&Transform, &Projection), With<Camera3d>>,
    windows: &Query<&mut Window>,
) {
    for entity in entities {
        commands.entity(entity).despawn();
    }

    let result_name = state.result_name();
    state.results.retain(|result| result.name != result_name);

    let scenario = &SCENARIOS[state.current_scenario];
    let outline_label = match state.outline_presence {
        OutlinePresence::Enabled => OUTLINE_PRESENCE_ENABLED_LABEL,
        OutlinePresence::Disabled => OUTLINE_PRESENCE_DISABLED_LABEL,
    };
    info!(
        "Setting up scenario: {} [outline {outline_label}] ({}/{})",
        scenario.name,
        state.current_scenario + 1,
        SCENARIOS.len()
    );

    let Ok((camera_transform, projection)) = camera_query.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let viewport = compute_viewport_info(camera_transform, projection, window);

    spawn_scenario(
        commands,
        meshes,
        materials,
        scenario,
        &viewport,
        state.outline_presence,
        state.outline_method,
    );

    state.frame_counter = 0;
    state.frame_times.clear();
    state.benchmark_phase = BenchmarkPhase::Warmup;
}

const fn advance_warmup_phase(state: &mut BenchmarkState) {
    state.frame_counter += 1;
    if state.frame_counter >= WARMUP_FRAMES {
        state.frame_counter = 0;
        state.benchmark_phase = BenchmarkPhase::Measure;
    }
}

fn measure_phase(state: &mut BenchmarkState, time: &Time<Real>) {
    let frame_time_ms = time.delta_secs_f64() * MILLISECONDS_PER_SECOND;
    state.frame_times.push(frame_time_ms);
    state.frame_counter += 1;

    if state.frame_counter >= MEASURE_FRAMES {
        state.benchmark_phase = BenchmarkPhase::Analyze;
    }
}

fn handle_analyze_phase(state: &mut BenchmarkState) {
    let result_name = state.result_name();
    let result = compute_statistics(&result_name, &mut state.frame_times);
    info!(
        "  {} — average: {:.milliseconds_precision$}ms, median: {:.milliseconds_precision$}ms, 95th: {:.milliseconds_precision$}ms, ~{:.frames_per_second_precision$} FPS",
        result.name,
        result.average,
        result.median,
        result.percentile_95,
        result.average_frames_per_second(),
        frames_per_second_precision = BENCHMARK_RESULTS_FRAMES_PER_SECOND_PRECISION,
        milliseconds_precision = BENCHMARK_RESULTS_MILLISECONDS_PRECISION,
    );

    if let Some(existing) = state
        .results
        .iter_mut()
        .find(|existing| existing.name == result.name)
    {
        *existing = result;
    } else {
        state.results.push(result);
    }

    if state.outline_presence == OutlinePresence::Disabled {
        state.outline_presence = OutlinePresence::Enabled;
        state.benchmark_phase = BenchmarkPhase::Setup;
        return;
    }

    if state.benchmark_mode == BenchmarkMode::Auto && state.current_scenario + 1 < SCENARIOS.len() {
        state.outline_presence = OutlinePresence::Disabled;
        state.current_scenario += 1;
        state.benchmark_phase = BenchmarkPhase::Setup;
        return;
    }

    state.outline_presence = OutlinePresence::Disabled;
    if state.benchmark_mode == BenchmarkMode::Auto {
        write_results(&state.results);
        if state.exit_behavior == ExitBehavior::OnComplete {
            info!("Auto benchmark complete, exiting in {AUTO_EXIT_DELAY_SECS}s");
            state.benchmark_phase = BenchmarkPhase::ExitDelay;
        } else {
            info!("{}", AUTO_BENCHMARK_COMPLETE_MESSAGE);
            state.benchmark_mode = BenchmarkMode::Interactive;
            state.benchmark_phase = BenchmarkPhase::Idle;
        }
    } else {
        state.benchmark_phase = BenchmarkPhase::Idle;
    }
}

fn handle_exit_delay_phase(state: &mut BenchmarkState, time: &Time<Real>) {
    state.exit_timer.tick(time.delta());
    if state.exit_timer.just_finished() {
        info!("{}", EXITING_MESSAGE);
        exit(0);
    }
}

pub(super) fn handle_input(input: Res<ButtonInput<KeyCode>>, mut state: ResMut<BenchmarkState>) {
    if input.just_pressed(KeyCode::KeyL) && !state.results.is_empty() {
        write_results(&state.results);
        return;
    }

    if input.just_pressed(KeyCode::KeyR) {
        info!("{}", AUTO_BENCHMARK_START_MESSAGE);
        state.benchmark_mode = BenchmarkMode::Auto;
        state.current_scenario = 0;
        state.outline_presence = OutlinePresence::Disabled;
        state.results.clear();
        state.benchmark_phase = BenchmarkPhase::Setup;
        return;
    }

    if input.just_pressed(KeyCode::KeyM) {
        let new_outline_method = next_outline_method(state.outline_method);
        info!("Outline mode: {}", outline_method_label(new_outline_method));
        state.outline_method = new_outline_method;
        state.benchmark_mode = BenchmarkMode::Interactive;
        state.outline_presence = OutlinePresence::Disabled;
        state.benchmark_phase = BenchmarkPhase::Setup;
        return;
    }

    for (index, scenario) in SCENARIOS.iter().enumerate() {
        if input.just_pressed(scenario.key) {
            info!(SCENARIO_SWITCH_MESSAGE, scenario.name);
            state.benchmark_mode = BenchmarkMode::Interactive;
            state.current_scenario = index;
            state.outline_presence = OutlinePresence::Disabled;
            state.benchmark_phase = BenchmarkPhase::Setup;
            return;
        }
    }
}
