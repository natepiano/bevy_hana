//! Main-world startup events and frame diagnostics.

use bevy::asset::AssetEvent;
use bevy::asset::AssetServer;
use bevy::prelude::Added;
use bevy::prelude::Local;
use bevy::prelude::MessageReader;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Shader;
use bevy::prelude::Time;
use bevy::prelude::Window;
use bevy::prelude::info;
use bevy::window::PrimaryWindow;
use fairy_dust::EdgePanelPlacementState;
use fairy_dust::EdgePanelPlacementStatus;
use hana_diegetic::DiegeticPerfStats;
use hana_diegetic::DiegeticTextBatch;
use hana_diegetic::DisplayCompensationReadiness;
use hana_diegetic::FontRegistered;
use hana_diegetic::PanelDimensionsChanged;
use hana_diegetic::PanelTextLayout;
use hana_diegetic::WorldTextReady;
use hana_lagrange::CameraHomePending;

use super::StartupStage;
use super::StartupTimeline;

const PLACEMENT_STABLE_FRAMES: u8 = 2;

pub(super) fn record_startup_ran(timeline: Res<StartupTimeline>) {
    timeline.record(StartupStage::StartupRan);
}

pub(super) fn record_widget_lab_spawned(timeline: Res<StartupTimeline>) {
    timeline.record(StartupStage::WidgetLabSpawned);
}

pub(super) fn record_embedded_font(event: On<FontRegistered>, timeline: Res<StartupTimeline>) {
    if event.source == hana_diegetic::FontSource::Embedded {
        timeline.record(StartupStage::EmbeddedFontRegistered);
    }
}

pub(super) fn record_first_frame(timeline: Res<StartupTimeline>) {
    timeline.record(StartupStage::FirstFrame);
}

pub(super) fn record_window_visible(
    windows: Query<&Window, bevy::prelude::With<PrimaryWindow>>,
    timeline: Res<StartupTimeline>,
) {
    if windows.iter().any(|window| window.visible) {
        timeline.record(StartupStage::WindowVisible);
    }
}

pub(super) fn record_layout_solved(
    _event: On<PanelDimensionsChanged>,
    timeline: Res<StartupTimeline>,
) {
    timeline.record(StartupStage::FirstLayoutSolved);
}

#[derive(Default)]
pub(super) struct EdgePlacementStability {
    revision:      u64,
    stable_frames: u8,
}

const fn layout_placed_after_settling(
    status: &EdgePanelPlacementStatus,
    home_pending: bool,
    observation: &mut EdgePlacementStability,
) -> bool {
    if !matches!(status.state, EdgePanelPlacementState::Placed) || home_pending {
        observation.stable_frames = 0;
        return false;
    }
    if observation.revision != status.revision {
        observation.revision = status.revision;
        observation.stable_frames = 1;
        return false;
    }
    observation.stable_frames = observation.stable_frames.saturating_add(1);
    observation.stable_frames >= PLACEMENT_STABLE_FRAMES
}

pub(super) fn record_layout_placed(
    status: Res<EdgePanelPlacementStatus>,
    home_pending: Query<(), bevy::prelude::With<CameraHomePending>>,
    mut observation: Local<EdgePlacementStability>,
    timeline: Res<StartupTimeline>,
) {
    if layout_placed_after_settling(&status, !home_pending.is_empty(), &mut observation) {
        timeline.record(StartupStage::FirstLayoutPlaced);
        timeline.report_summary_when_ready();
    }
}

const fn compensation_ready(readiness: DisplayCompensationReadiness) -> bool {
    matches!(
        readiness,
        DisplayCompensationReadiness::Identity | DisplayCompensationReadiness::Compensated
    )
}

pub(super) fn record_display_compensation(
    readiness: Res<DisplayCompensationReadiness>,
    timeline: Res<StartupTimeline>,
) {
    if compensation_ready(*readiness) {
        timeline.record(StartupStage::DisplayCompensationReady);
        timeline.report_summary_when_ready();
    }
}

pub(super) fn record_text_reified(
    text_layouts: Query<(), Added<PanelTextLayout>>,
    timeline: Res<StartupTimeline>,
) {
    if !text_layouts.is_empty() {
        timeline.record(StartupStage::FirstTextReified);
    }
}

pub(super) fn record_text_shaped(_event: On<WorldTextReady>, timeline: Res<StartupTimeline>) {
    timeline.record(StartupStage::FirstTextShaped);
}

pub(super) fn record_text_batch(
    text_batches: Query<(), Added<DiegeticTextBatch>>,
    timeline: Res<StartupTimeline>,
) {
    if !text_batches.is_empty() {
        timeline.record(StartupStage::FirstTextBatch);
    }
}

pub(super) fn record_shader_loads(
    mut events: MessageReader<AssetEvent<Shader>>,
    asset_server: Res<AssetServer>,
    timeline: Res<StartupTimeline>,
) {
    for event in events.read() {
        let AssetEvent::LoadedWithDependencies { id } = event else {
            continue;
        };
        if let Some(path) = asset_server.get_path(*id) {
            info!("shader loaded {path} at {:.3} ms", timeline.elapsed_ms());
        } else {
            info!(
                "shader loaded <no path> ({id:?}) at {:.3} ms",
                timeline.elapsed_ms()
            );
        }
    }
}

pub(super) fn log_startup_frame(
    timeline: Res<StartupTimeline>,
    time: Res<Time>,
    perf: Res<DiegeticPerfStats>,
    mut frame_number: bevy::prelude::Local<u32>,
) {
    timeline.log_frame_before_summary(|measurements| {
        *frame_number += 1;
        let text = &perf.panel_text;
        let awaiting_drawable_text = measurements.text_batch_seen
            && matches!(
                measurements.stages[StartupStage::FirstTextPipelineReady.index()],
                super::StageTime::NotSeen
            );
        info!(
            "startup frame {} at {:.3} ms (delta {:.3} ms, awaiting first text pipeline: {}): compute {:.3}, reify {:.3}, text total {:.3}, shape {:.3}, parley {:.3}, mesh build {:.3} ms",
            *frame_number,
            timeline.elapsed_ms(),
            time.delta_secs_f64() * 1_000.0,
            awaiting_drawable_text,
            perf.compute_ms,
            perf.reify_ms,
            text.total_ms,
            text.shape_ms,
            text.parley_ms,
            text.mesh_build_ms,
        );
    });
    if *frame_number >= 600 {
        timeline.report_summary();
    }
}
