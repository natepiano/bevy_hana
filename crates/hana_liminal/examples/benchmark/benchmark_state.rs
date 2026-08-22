use std::env::var;

use bevy::prelude::Resource;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy_kana::ToUsize;
use hana_liminal::OutlineMethod;

use crate::constants::AUTO_EXIT_DELAY_SECS;
use crate::constants::AUTO_MODE_ENV_VAR;
use crate::constants::AUTO_MODE_ENV_VAR_ENABLED_VALUE;
use crate::constants::AUTO_STARTUP_DELAY_SECS;
use crate::constants::MEASURE_FRAMES;
use crate::constants::OUTLINE_METHOD_JUMP_FLOOD_LABEL;
use crate::constants::OUTLINE_METHOD_SCREEN_HULL_LABEL;
use crate::constants::OUTLINE_METHOD_WORLD_HULL_LABEL;
use crate::constants::OUTLINE_PRESENCE_DISABLED_LABEL;
use crate::constants::OUTLINE_PRESENCE_ENABLED_LABEL;
use crate::constants::RESULTS_PER_SCENARIO;
use crate::constants::SCENARIOS;
use crate::results::ScenarioResult;

#[derive(PartialEq, Eq)]
pub(super) enum BenchmarkMode {
    Auto,
    Interactive,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OutlinePresence {
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ExitBehavior {
    KeepRunning,
    OnComplete,
}

pub(super) enum BenchmarkPhase {
    Idle,
    StartupDelay,
    Setup,
    Warmup,
    Measure,
    Analyze,
    ExitDelay,
}

#[derive(Resource)]
pub(super) struct BenchmarkState {
    pub(super) benchmark_mode:   BenchmarkMode,
    pub(super) current_scenario: usize,
    pub(super) outline_presence: OutlinePresence,
    pub(super) outline_method:   OutlineMethod,
    pub(super) benchmark_phase:  BenchmarkPhase,
    pub(super) frame_counter:    u32,
    pub(super) frame_times:      Vec<f64>,
    pub(super) results:          Vec<ScenarioResult>,
    pub(super) startup_timer:    Timer,
    pub(super) exit_timer:       Timer,
    pub(super) exit_behavior:    ExitBehavior,
}

impl BenchmarkState {
    pub(super) fn new() -> Self {
        let exit_behavior =
            if var(AUTO_MODE_ENV_VAR).is_ok_and(|value| value == AUTO_MODE_ENV_VAR_ENABLED_VALUE) {
                ExitBehavior::OnComplete
            } else {
                ExitBehavior::KeepRunning
            };
        let (benchmark_mode, benchmark_phase) = if exit_behavior == ExitBehavior::OnComplete {
            (BenchmarkMode::Auto, BenchmarkPhase::StartupDelay)
        } else {
            (BenchmarkMode::Interactive, BenchmarkPhase::Idle)
        };

        Self {
            benchmark_mode,
            current_scenario: 0,
            outline_presence: OutlinePresence::Disabled,
            outline_method: OutlineMethod::default(),
            benchmark_phase,
            frame_counter: 0,
            frame_times: Vec::with_capacity(MEASURE_FRAMES.to_usize()),
            results: Vec::with_capacity(SCENARIOS.len() * RESULTS_PER_SCENARIO),
            startup_timer: Timer::from_seconds(AUTO_STARTUP_DELAY_SECS, TimerMode::Once),
            exit_timer: Timer::from_seconds(AUTO_EXIT_DELAY_SECS, TimerMode::Once),
            exit_behavior,
        }
    }

    pub(super) fn result_name(&self) -> String {
        let scenario = &SCENARIOS[self.current_scenario];
        let suffix = match self.outline_presence {
            OutlinePresence::Enabled => OUTLINE_PRESENCE_ENABLED_LABEL,
            OutlinePresence::Disabled => OUTLINE_PRESENCE_DISABLED_LABEL,
        };
        let mode_label = outline_method_label(self.outline_method);
        format!("{} {suffix} ({mode_label})", scenario.name)
    }
}

pub(super) const fn outline_method_label(outline_method: OutlineMethod) -> &'static str {
    match outline_method {
        OutlineMethod::JumpFlood => OUTLINE_METHOD_JUMP_FLOOD_LABEL,
        OutlineMethod::WorldHull => OUTLINE_METHOD_WORLD_HULL_LABEL,
        OutlineMethod::ScreenHull => OUTLINE_METHOD_SCREEN_HULL_LABEL,
    }
}

pub(super) const fn next_outline_method(outline_method: OutlineMethod) -> OutlineMethod {
    match outline_method {
        OutlineMethod::JumpFlood => OutlineMethod::WorldHull,
        OutlineMethod::WorldHull => OutlineMethod::ScreenHull,
        OutlineMethod::ScreenHull => OutlineMethod::JumpFlood,
    }
}
