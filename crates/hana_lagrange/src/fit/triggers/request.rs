use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Vec2;
use bevy::prelude::warn;

use crate::animation;
use crate::animation::AnimationRejected;
use crate::animation::AnimationRejectionReason;
use crate::animation::AnimationSource;
use crate::animation::CameraMoveError;
use crate::animation::CameraRequestPreparationError;
use crate::animation::PlayAnimation;
use crate::fit::geometry;
use crate::fit::geometry::FitAnchor;
use crate::fit::geometry::FitError;
use crate::fit::geometry::FitSolution;

/// `HigherCameraRequestController` selects the camera family for facade preparation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HigherCameraRequestController {
    Orbit,
    FreeFlight,
}

/// `CameraRequestPart` records whether one component needed by preparation is available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraRequestPart {
    Available,
    Missing,
}

impl<T, E> From<Result<T, E>> for CameraRequestPart {
    fn from(result: Result<T, E>) -> Self { result.map_or(Self::Missing, |_| Self::Available) }
}

/// `HigherCameraRequestPreparation` is the authoritative result of one facade preparation.
pub(super) enum HigherCameraRequestPreparation {
    /// Preparation produced the raw facade request consumed by retained admission.
    Prepared(PlayAnimation),
    /// Preparation rejected the request before retained state could be mutated.
    Rejected(AnimationRejectionReason),
}

impl From<Result<PlayAnimation, AnimationRejectionReason>> for HigherCameraRequestPreparation {
    fn from(result: Result<PlayAnimation, AnimationRejectionReason>) -> Self {
        match result {
            Ok(request) => Self::Prepared(request),
            Err(reason) => Self::Rejected(reason),
        }
    }
}

/// Selects exactly one camera family before controller-specific preparation.
pub(super) const fn higher_camera_request_controller(
    orbit_controller: CameraRequestPart,
    free_flight_controller: CameraRequestPart,
    camera_basis: CameraRequestPart,
) -> Result<HigherCameraRequestController, AnimationRejectionReason> {
    match (orbit_controller, free_flight_controller, camera_basis) {
        (CameraRequestPart::Available, CameraRequestPart::Missing, _) => {
            Ok(HigherCameraRequestController::Orbit)
        },
        (
            CameraRequestPart::Missing,
            CameraRequestPart::Available,
            CameraRequestPart::Available,
        ) => Ok(HigherCameraRequestController::FreeFlight),
        (CameraRequestPart::Missing, CameraRequestPart::Available, CameraRequestPart::Missing) => {
            Err(AnimationRejectionReason::MissingCameraBasis)
        },
        (CameraRequestPart::Missing, CameraRequestPart::Missing, _) => {
            Err(AnimationRejectionReason::NoCameraController)
        },
        (CameraRequestPart::Available, CameraRequestPart::Available, _) => {
            Err(AnimationRejectionReason::ConflictingCameraControllers)
        },
    }
}

pub(super) const fn request_preparation_rejection(
    error: CameraRequestPreparationError,
) -> AnimationRejectionReason {
    AnimationRejectionReason::RequestPreparationFailed(error)
}

pub(super) fn invalid_move_rejection(error: CameraMoveError) -> AnimationRejectionReason {
    animation::warn_rejected_camera_move(&error);
    AnimationRejectionReason::InvalidMove(error)
}

/// Commits exactly one outcome for one higher-level facade event.
pub(super) fn finish_higher_camera_request_preparation(
    commands: &mut Commands,
    camera: Entity,
    source: AnimationSource,
    target: Entity,
    preparation: HigherCameraRequestPreparation,
) {
    match preparation {
        HigherCameraRequestPreparation::Prepared(request) => commands.trigger(request),
        HigherCameraRequestPreparation::Rejected(reason) => {
            commands.trigger(AnimationRejected {
                camera,
                source,
                target: Some(target),
                reason,
            });
        },
    }
}

/// Parameters for a fit calculation request.
pub(super) struct FitRequest<'a> {
    pub(super) context:    &'a str,
    pub(super) target:     Entity,
    pub(super) yaw:        f32,
    pub(super) pitch:      f32,
    pub(super) margin:     f32,
    pub(super) anchor:     FitAnchor,
    pub(super) offset_px:  Vec2,
    pub(super) projection: &'a Projection,
    pub(super) camera:     &'a Camera,
}

/// Shared fit preparation used by fit-family facade observers.
/// Extracts target mesh vertices and computes the fit solution for the requested
/// camera orientation.
pub(super) fn prepare_fit_for_target(
    request: &FitRequest,
    mesh_query: &Query<&Mesh3d>,
    children_query: &Query<&Children>,
    global_transform_query: &Query<&GlobalTransform>,
    meshes: &Assets<Mesh>,
) -> Result<FitSolution, CameraRequestPreparationError> {
    let context = request.context;
    let target = request.target;
    global_transform_query.get(target).map_err(|_| {
        warn!("{context}: target {target:?} has no GlobalTransform");
        CameraRequestPreparationError::MissingTargetTransform
    })?;
    let (vertices, geometric_center) = geometry::extract_mesh_vertices(
        target,
        children_query,
        mesh_query,
        global_transform_query,
        meshes,
    )
    .ok_or_else(|| {
        warn!("{context}: Failed to extract mesh vertices for entity {target:?}");
        CameraRequestPreparationError::MissingTargetGeometry
    })?;

    geometry::calculate_fit(
        &vertices,
        geometric_center,
        request.yaw,
        request.pitch,
        request.margin,
        request.anchor,
        request.offset_px,
        request.projection,
        request.camera,
    )
    .map_err(|error| {
        warn!("{context}: Failed to calculate fit for entity {target:?}: {error}");
        match error {
            FitError::NoViewport => CameraRequestPreparationError::ViewportUnavailable,
            FitError::PointsBehindCamera => CameraRequestPreparationError::PointsBehindCamera,
            FitError::UnsupportedProjection => CameraRequestPreparationError::UnsupportedProjection,
        }
    })
}
