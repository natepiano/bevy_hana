use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy_kana::SequenceStagesRevision;

use super::PendingCameraRequest;
use crate::animation::events::AnimationSource;
use crate::animation::sequence::CameraSequence;
use crate::fit::ZoomContext;

/// `RetainedCameraJourneyOrigin` records the authoring source for one sequence revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::animation) enum RetainedCameraJourneyOrigin {
    /// Originated at the durable public request facade.
    Facade(AnimationSource),
    /// Originated as direct retained `CameraSequence` authoring.
    DirectCameraSequence,
}

impl RetainedCameraJourneyOrigin {
    /// Maps retained request identity onto public lifecycle events.
    pub(in crate::animation::sequence) const fn event_source(self) -> AnimationSource {
        match self {
            Self::Facade(source) => source,
            Self::DirectCameraSequence => AnimationSource::CameraSequence,
        }
    }
}

/// Metadata committed beside one exact retained stage revision.
///
/// A direct `CameraSequence` has source identity but no fabricated fit target
/// or zoom context. Requests preserve all facade information until the next
/// retained definition replaces it.
#[derive(Component, Clone, Debug)]
pub(in crate::animation) struct RetainedCameraJourney {
    pub(in crate::animation::sequence) revision: SequenceStagesRevision,
    pub(in crate::animation::sequence) origin:   RetainedCameraJourneyOrigin,
    pub(in crate::animation::sequence) target:   Option<Entity>,
    pub(in crate::animation::sequence) zoom:     Option<ZoomContext>,
}

impl RetainedCameraJourney {
    pub(super) const fn direct(sequence: &CameraSequence) -> Self {
        Self {
            revision: sequence.sequence_stages().revision(),
            origin:   RetainedCameraJourneyOrigin::DirectCameraSequence,
            target:   None,
            zoom:     None,
        }
    }

    pub(super) fn request(sequence: &CameraSequence, request: &PendingCameraRequest) -> Self {
        Self {
            revision: sequence.sequence_stages().revision(),
            origin:   RetainedCameraJourneyOrigin::Facade(request.source),
            target:   request.target,
            zoom:     request.zoom.clone(),
        }
    }

    pub(super) fn matches_revision(&self, revision: SequenceStagesRevision) -> bool {
        self.revision == revision
    }
}
