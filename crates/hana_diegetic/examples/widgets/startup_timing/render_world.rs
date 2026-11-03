//! The first visible queued text draw's pipeline state in the render world.

use std::fmt;

use bevy::prelude::*;
use bevy::render::render_resource::CachedPipelineState;
use bevy::render::render_resource::CachedRenderPipelineId;
use bevy::render::render_resource::PipelineCache;
use bevy::shader::ShaderCacheError;
use hana_diegetic::FirstTextBatchPipeline as QueuedFirstTextBatchPipeline;
pub(super) use hana_diegetic::QueuedTextPipelineCandidate;
use hana_diegetic::TextPipelineStartup;

use super::StartupStage;
use super::StartupTimeline;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum FirstTextBatchPipeline {
    #[default]
    NotObserved,
    Waiting {
        view: Entity,
        id:   CachedRenderPipelineId,
    },
    Ready {
        view: Entity,
        id:   CachedRenderPipelineId,
    },
    Failed {
        view: Entity,
        id:   CachedRenderPipelineId,
    },
}

impl fmt::Display for FirstTextBatchPipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObserved => f.write_str("NotObserved"),
            Self::Waiting { view, id } => write!(f, "Waiting({}) view {view:?}", id.id()),
            Self::Ready { view, id } => write!(f, "Ready({}) view {view:?}", id.id()),
            Self::Failed { view, id } => write!(f, "Failed({}) view {view:?}", id.id()),
        }
    }
}

impl FirstTextBatchPipeline {
    fn observe(first_batch: QueuedFirstTextBatchPipeline, cache: &PipelineCache) -> Self {
        let QueuedFirstTextBatchPipeline::Queued { view, id } = first_batch else {
            return Self::NotObserved;
        };
        Self::from_state(view, id, cache.get_render_pipeline_state(id))
    }

    fn from_state(view: Entity, id: CachedRenderPipelineId, state: &CachedPipelineState) -> Self {
        match state {
            CachedPipelineState::Queued | CachedPipelineState::Creating(_) => {
                Self::Waiting { view, id }
            },
            CachedPipelineState::Ok(_) => Self::Ready { view, id },
            CachedPipelineState::Err(error) => {
                info!("first visible text pipeline {id:?} error: {error:?}");
                match error {
                    ShaderCacheError::ShaderNotLoaded(_)
                    | ShaderCacheError::ShaderImportNotYetAvailable => Self::Waiting { view, id },
                    _ => Self::Failed { view, id },
                }
            },
        }
    }
}

fn first_text_pipeline_ready(
    candidate: QueuedTextPipelineCandidate,
    first_batch: FirstTextBatchPipeline,
) -> bool {
    match (candidate, first_batch) {
        (QueuedTextPipelineCandidate::NotQueued, FirstTextBatchPipeline::Ready { .. }) => true,
        (
            QueuedTextPipelineCandidate::Queued {
                view: warm_view,
                id: warm_id,
                ..
            },
            FirstTextBatchPipeline::Ready {
                view: batch_view,
                id: batch_id,
            },
        ) => warm_view == batch_view && warm_id == batch_id,
        _ => false,
    }
}

#[derive(Resource, Default, Clone, Copy)]
pub(super) struct TextPipelineObservation {
    waiting:     usize,
    candidate:   QueuedTextPipelineCandidate,
    first_batch: FirstTextBatchPipeline,
}

pub(super) fn observe_text_pipeline(
    timeline: Res<StartupTimeline>,
    pipeline_cache: Res<PipelineCache>,
    startup: Res<TextPipelineStartup>,
    mut observation: ResMut<TextPipelineObservation>,
) {
    if timeline.is_summary_reported() {
        return;
    }

    observation.waiting = pipeline_cache.waiting_pipelines().count();
    observation.candidate = startup.candidate;
    observation.first_batch = FirstTextBatchPipeline::observe(startup.first_batch, &pipeline_cache);
}

pub(super) fn probe_text_pipeline(
    timeline: Res<StartupTimeline>,
    observation: Res<TextPipelineObservation>,
) {
    if timeline.is_summary_reported() {
        return;
    }
    let TextPipelineObservation {
        waiting,
        candidate,
        first_batch,
    } = *observation;
    timeline.set_waiting_pipelines(waiting);
    timeline.set_pipeline_states(candidate, first_batch);

    if first_text_pipeline_ready(candidate, first_batch) {
        timeline.record(StartupStage::FirstTextPipelineReady);
        timeline.report_summary_when_ready();
        return;
    }
    if should_report_failure(first_batch) {
        timeline.report_summary();
        return;
    }

    let ids_match = pipeline_ids_match(candidate, first_batch);
    info!(
        "startup render frame at {:.2} ms: {waiting} waiting pipelines; warmed candidate {candidate:?}; first visible text batch {first_batch:?}; ids match: {ids_match}",
        timeline.elapsed_ms(),
    );
}

const fn should_report_failure(first_batch: FirstTextBatchPipeline) -> bool {
    matches!(first_batch, FirstTextBatchPipeline::Failed { .. })
}

pub(super) fn pipeline_ids_match(
    candidate: QueuedTextPipelineCandidate,
    first_batch: FirstTextBatchPipeline,
) -> bool {
    match (candidate, first_batch) {
        (
            QueuedTextPipelineCandidate::Queued {
                view: warm_view,
                id: warm_id,
                ..
            },
            FirstTextBatchPipeline::Waiting {
                view: batch_view,
                id: batch_id,
            }
            | FirstTextBatchPipeline::Ready {
                view: batch_view,
                id: batch_id,
            }
            | FirstTextBatchPipeline::Failed {
                view: batch_view,
                id: batch_id,
            },
        ) => warm_view == batch_view && warm_id == batch_id,
        _ => false,
    }
}
