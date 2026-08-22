use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::ResMut;

use super::PendingCameraRequest;
use super::PendingCameraRequests;
use super::admission::CameraRequestAdmission;
use super::effects::CameraAdmissionEffectError;
use crate::animation::events::AnimationRejected;
use crate::animation::events::AnimationRejectionReason;
use crate::animation::events::AnimationSource;
use crate::animation::events::CameraRequestPreparationError;
use crate::animation::sequence::controller_installation::CameraControllerAvailability;
use crate::animation::sequence::playback::CameraEvaluationError;

/// Commits pending requests in observer trigger order after driver arbitration.
/// Every rejected candidate leaves all retained state and controller overrides
/// untouched; accepted candidates publish their authoring, stages, prepared
/// endpoints, metadata, and native command request together.
pub(in crate::animation) fn admit_pending_camera_requests(
    mut commands: Commands,
    mut pending: ResMut<PendingCameraRequests>,
    mut admission: CameraRequestAdmission,
) {
    let mut accepted_cameras = Vec::new();
    for request in core::mem::take(&mut pending.0) {
        let camera = request.camera;
        if admission.admit(&mut commands, request, &accepted_cameras) {
            accepted_cameras.push(camera);
        }
    }
}

pub(super) fn trigger_animation_rejected(
    commands: &mut Commands,
    camera: Entity,
    source: AnimationSource,
    target: Option<Entity>,
    reason: AnimationRejectionReason,
) {
    commands.trigger(AnimationRejected {
        camera,
        source,
        target,
        reason,
    });
}

pub(super) fn reject_camera_request(
    commands: &mut Commands,
    request: &PendingCameraRequest,
    reason: AnimationRejectionReason,
) {
    trigger_animation_rejected(
        commands,
        request.camera,
        request.source,
        request.target,
        reason,
    );
}

pub(super) const fn rejection_for_controller_availability(
    availability: CameraControllerAvailability,
) -> Option<AnimationRejectionReason> {
    match availability {
        CameraControllerAvailability::InitializedOrbit
        | CameraControllerAvailability::InitializedFreeFlight => None,
        CameraControllerAvailability::NoController => {
            Some(AnimationRejectionReason::NoCameraController)
        },
        CameraControllerAvailability::FreeFlightWithoutBasis => {
            Some(AnimationRejectionReason::MissingCameraBasis)
        },
        CameraControllerAvailability::ConflictingControllers => {
            Some(AnimationRejectionReason::ConflictingCameraControllers)
        },
    }
}

pub(super) const fn camera_admission_effect_rejection(
    error: CameraAdmissionEffectError,
) -> AnimationRejectionReason {
    match error {
        CameraAdmissionEffectError::MissingProjection => {
            AnimationRejectionReason::RequestPreparationFailed(
                CameraRequestPreparationError::MissingProjection,
            )
        },
        CameraAdmissionEffectError::MissingTargetEntity => {
            AnimationRejectionReason::RequestPreparationFailed(
                CameraRequestPreparationError::MissingTargetGeometry,
            )
        },
        CameraAdmissionEffectError::MissingFitTarget
        | CameraAdmissionEffectError::MissingAuthoredDestination
        | CameraAdmissionEffectError::InvalidOrthographicScale => {
            AnimationRejectionReason::PreparationFailed(CameraEvaluationError::UnrepresentablePose)
        },
    }
}
