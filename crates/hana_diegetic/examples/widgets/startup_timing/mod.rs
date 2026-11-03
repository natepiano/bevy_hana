//! Startup measurements shared by the example's main and render worlds.

mod main_world;
mod render_world;

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use bevy::app::App;
use bevy::app::First;
use bevy::app::Last;
use bevy::app::Plugin;
use bevy::app::PostStartup;
use bevy::app::PreStartup;
use bevy::app::Update;
use bevy::ecs::schedule::IntoScheduleConfigs as _;
use bevy::prelude::Resource;
use bevy::prelude::info;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSystems;

use self::render_world::FirstTextBatchPipeline;
use self::render_world::QueuedTextPipelineCandidate;
use self::render_world::TextPipelineObservation;
use self::render_world::observe_text_pipeline;
use self::render_world::probe_text_pipeline;
use crate::spawn_widget_lab;

const STAGES: [StartupStage; 19] = [
    StartupStage::MainEntered,
    StartupStage::DefaultPluginsBuilt,
    StartupStage::DiegeticUiPluginBuilt,
    StartupStage::LagrangePluginBuilt,
    StartupStage::BaselineBuilt,
    StartupStage::ExampleCapabilitiesBuilt,
    StartupStage::AppBuilt,
    StartupStage::StartupRan,
    StartupStage::WidgetLabSpawned,
    StartupStage::EmbeddedFontRegistered,
    StartupStage::FirstFrame,
    StartupStage::WindowVisible,
    StartupStage::FirstLayoutSolved,
    StartupStage::FirstLayoutPlaced,
    StartupStage::FirstTextReified,
    StartupStage::FirstTextShaped,
    StartupStage::FirstTextBatch,
    StartupStage::FirstTextPipelineReady,
    StartupStage::DisplayCompensationReady,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StartupStage {
    MainEntered,
    DefaultPluginsBuilt,
    DiegeticUiPluginBuilt,
    LagrangePluginBuilt,
    BaselineBuilt,
    ExampleCapabilitiesBuilt,
    AppBuilt,
    StartupRan,
    WidgetLabSpawned,
    EmbeddedFontRegistered,
    FirstFrame,
    WindowVisible,
    FirstLayoutSolved,
    FirstLayoutPlaced,
    FirstTextReified,
    FirstTextShaped,
    FirstTextBatch,
    FirstTextPipelineReady,
    DisplayCompensationReady,
}

impl StartupStage {
    const fn index(self) -> usize { self as usize }
}

#[derive(Clone, Copy, Default)]
enum StageTime {
    #[default]
    NotSeen,
    Recorded(Duration),
}

#[derive(Clone, Copy, Default)]
enum WaitingPipelines {
    #[default]
    NotMeasured,
    Count(usize),
}

#[derive(Default)]
struct StartupMeasurements {
    stages:            [StageTime; STAGES.len()],
    text_batch_seen:   bool,
    waiting_pipelines: WaitingPipelines,
    candidate:         QueuedTextPipelineCandidate,
    first_batch:       FirstTextBatchPipeline,
    summary_reported:  bool,
}

/// One clock and one set of first-seen measurements for both Bevy worlds.
#[derive(Clone, Resource)]
pub(super) struct StartupTimeline {
    launched:     Instant,
    measurements: Arc<Mutex<StartupMeasurements>>,
}

impl StartupTimeline {
    fn new(launched: Instant) -> Self {
        let mut measurements = StartupMeasurements::default();
        measurements.stages[StartupStage::MainEntered.index()] =
            StageTime::Recorded(Duration::ZERO);
        Self {
            launched,
            measurements: Arc::new(Mutex::new(measurements)),
        }
    }

    pub(super) fn elapsed_ms(&self) -> f64 { self.launched.elapsed().as_secs_f64() * 1_000.0 }

    /// Returns whether this call captured the stage's first occurrence.
    pub(super) fn record(&self, stage: StartupStage) -> bool {
        self.record_at(stage, Instant::now())
    }

    pub(super) fn record_at(&self, stage: StartupStage, at: Instant) -> bool {
        if let Ok(mut measurements) = self.measurements.lock()
            && matches!(measurements.stages[stage.index()], StageTime::NotSeen)
        {
            measurements.stages[stage.index()] =
                StageTime::Recorded(at.duration_since(self.launched));
            if stage == StartupStage::FirstTextBatch {
                measurements.text_batch_seen = true;
            }
            return true;
        }
        false
    }

    pub(super) fn is_summary_reported(&self) -> bool {
        self.measurements
            .lock()
            .is_ok_and(|measurements| measurements.summary_reported)
    }

    pub(super) fn set_waiting_pipelines(&self, count: usize) {
        if let Ok(mut measurements) = self.measurements.lock() {
            measurements.waiting_pipelines = WaitingPipelines::Count(count);
        }
    }

    fn set_pipeline_states(
        &self,
        candidate: QueuedTextPipelineCandidate,
        first_batch: FirstTextBatchPipeline,
    ) {
        if let Ok(mut measurements) = self.measurements.lock() {
            measurements.candidate = candidate;
            measurements.first_batch = first_batch;
        }
    }

    fn log_frame_before_summary(&self, log_frame: impl FnOnce(&StartupMeasurements)) {
        if let Ok(measurements) = self.measurements.lock() {
            if measurements.summary_reported
                || matches!(
                    measurements.stages[StartupStage::FirstTextPipelineReady.index()],
                    StageTime::Recorded(_)
                )
            {
                return;
            }
            log_frame(&measurements);
        }
    }

    pub(super) fn report_summary(&self) {
        let summary = if let Ok(mut measurements) = self.measurements.lock() {
            if measurements.summary_reported {
                return;
            }
            measurements.summary_reported = true;
            format_summary(&measurements)
        } else {
            return;
        };
        info!("{summary}");
    }

    pub(super) fn report_summary_when_ready(&self) {
        let ready = self.measurements.lock().is_ok_and(|measurements| {
            [
                StartupStage::FirstLayoutPlaced,
                StartupStage::DisplayCompensationReady,
                StartupStage::FirstTextPipelineReady,
            ]
            .into_iter()
            .all(|stage| matches!(measurements.stages[stage.index()], StageTime::Recorded(_)))
        });
        if ready {
            self.report_summary();
        }
    }
}

fn format_summary(measurements: &StartupMeasurements) -> String {
    let mut summary = String::from("widgets startup timeline (ms since main):");
    let mut previous_recorded = StageTime::NotSeen;
    for stage in STAGES {
        let label = format!("{stage:?}");
        match measurements.stages[stage.index()] {
            StageTime::NotSeen => {
                let _ = write!(summary, "\n  {label}: not seen");
            },
            StageTime::Recorded(at) => {
                let elapsed_ms = at.as_secs_f64() * 1_000.0;
                let delta_ms = match previous_recorded {
                    StageTime::NotSeen => 0.0,
                    StageTime::Recorded(previous) => {
                        (at.as_secs_f64() - previous.as_secs_f64()) * 1_000.0
                    },
                };
                let _ = write!(
                    summary,
                    "\n  {label}: {elapsed_ms:.3} ms ({delta_ms:+.3} ms)"
                );
                previous_recorded = StageTime::Recorded(at);
            },
        }
    }
    match measurements.waiting_pipelines {
        WaitingPipelines::NotMeasured => {
            let _ = write!(summary, "\n  waiting pipelines: not measured");
        },
        WaitingPipelines::Count(count) => {
            let _ = write!(summary, "\n  waiting pipelines: {count}");
        },
    }
    let _ = write!(
        summary,
        "\n  warmed candidate: {:?}\n  first visible text batch pipeline: {}",
        measurements.candidate, measurements.first_batch,
    );
    let _ = write!(
        summary,
        "\n  warmed id matches first batch: {}",
        render_world::pipeline_ids_match(measurements.candidate, measurements.first_batch),
    );
    summary
}

// Fairy Dust owns its App privately; this example-local handle lets main stamp
// AppBuilt immediately before the builder's run method is called.
static EXAMPLE_TIMELINE: OnceLock<StartupTimeline> = OnceLock::new();

pub(super) fn record_app_built(app: &App, capabilities_built: Instant) {
    if let Some(timeline) = EXAMPLE_TIMELINE.get() {
        if let Some(baseline) = app
            .world()
            .get_resource::<fairy_dust::BaselineBuildTimings>()
        {
            timeline.record_at(StartupStage::DefaultPluginsBuilt, baseline.default_plugins);
            timeline.record_at(StartupStage::DiegeticUiPluginBuilt, baseline.diegetic_ui);
            timeline.record_at(StartupStage::LagrangePluginBuilt, baseline.lagrange);
            timeline.record_at(StartupStage::BaselineBuilt, baseline.complete);
        }
        timeline.record_at(StartupStage::ExampleCapabilitiesBuilt, capabilities_built);
        timeline.record(StartupStage::AppBuilt);
    }
}

/// Let a first-launch probe start at a narrow size without a resize frame.
pub(super) fn apply_probe_window_size(app: &mut App) {
    let Ok(size) = std::env::var("WIDGETS_STARTUP_WINDOW_SIZE") else {
        return;
    };
    let Some((width, height)) = size.split_once('x') else {
        return;
    };
    let (Ok(width), Ok(height)) = (width.parse::<f32>(), height.parse::<f32>()) else {
        return;
    };
    let world = app.world_mut();
    let mut windows = world.query_filtered::<&mut bevy::prelude::Window, bevy::prelude::With<bevy::window::PrimaryWindow>>();
    if let Ok(mut window) = windows.single_mut(world) {
        window.resolution.set(width, height);
        info!("widgets startup probe window size: {width}x{height}");
    }
}

pub(super) struct StartupTimingPlugin {
    pub launched: Instant,
}

impl Plugin for StartupTimingPlugin {
    fn build(&self, app: &mut App) {
        let timeline = StartupTimeline::new(self.launched);
        let _ = EXAMPLE_TIMELINE.set(timeline.clone());
        app.insert_resource(timeline)
            .add_systems(PreStartup, main_world::record_startup_ran)
            .add_systems(
                PostStartup,
                main_world::record_widget_lab_spawned.after(spawn_widget_lab),
            )
            .add_systems(First, main_world::record_first_frame)
            .add_systems(Update, main_world::record_shader_loads)
            .add_systems(
                Last,
                (
                    main_world::record_window_visible,
                    main_world::record_text_reified,
                    main_world::record_text_batch,
                    main_world::record_layout_placed,
                    main_world::record_display_compensation,
                    main_world::log_startup_frame,
                )
                    .chain(),
            )
            .add_observer(main_world::record_embedded_font)
            .add_observer(main_world::record_layout_solved)
            .add_observer(main_world::record_text_shaped);
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        if let Some(timeline) = EXAMPLE_TIMELINE.get() {
            render_app.insert_resource(timeline.clone());
            render_app
                .init_resource::<TextPipelineObservation>()
                .add_systems(
                    Render,
                    (observe_text_pipeline, probe_text_pipeline)
                        .chain()
                        .in_set(RenderSystems::Cleanup),
                );
        }
    }
}
