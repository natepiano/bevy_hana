mod admission;
mod direct;
mod effects;
mod facade;
mod journey;
mod rejection;

use bevy::ecs::schedule::SystemSet;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Resource;
pub(in crate::animation) use direct::close_discarded_camera_sequence;
pub(in crate::animation) use direct::prepare_direct_camera_sequences;
pub(in crate::animation) use direct::remove_direct_camera_sequence;
use effects::PendingCameraAdmissionEffects;
pub(in crate::animation) use facade::capture_play_animation;
pub(super) use journey::RetainedCameraJourney;
#[cfg(test)]
pub(in crate::animation::sequence) use journey::RetainedCameraJourneyOrigin;
pub(in crate::animation) use rejection::admit_pending_camera_requests;

use super::CameraSequence;
use crate::animation::events::AnimationSource;
use crate::fit::ZoomContext;

/// Private camera-domain schedule boundaries inside the shared playback path.
#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(in crate::animation) enum CameraSequenceSystems {
    /// Initializes controller pose and home state before preparation.
    InitializeControllers,
    /// Publishes direct authoring before shared driver arbitration.
    PublishDirectAuthoring,
    /// Admits observed facade requests after arbitration has flushed.
    AdmitRequests,
}

/// A request observed from the public facade before final ownership admission.
#[derive(Clone, Debug)]
pub(super) struct PendingCameraRequest {
    camera:   Entity,
    sequence: CameraSequence,
    source:   AnimationSource,
    target:   Option<Entity>,
    zoom:     Option<ZoomContext>,
    effects:  PendingCameraAdmissionEffects,
}

/// Private short-lived request inbox. It is drained once per update and is not
/// retained playback.
#[derive(Resource, Default)]
pub(in crate::animation) struct PendingCameraRequests(pub(super) Vec<PendingCameraRequest>);

/// Marks a facade request that needs its controller pose initialized before
/// the ordered admission boundary. It exists for one update only and is not
/// retained playback state.
#[derive(Component)]
pub(in crate::animation) struct CameraSequencePreparationRequested;
