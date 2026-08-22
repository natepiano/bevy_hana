use bevy::prelude::Entity;
use thiserror::Error;

use crate::Radius;
use crate::animation::events::AnimationSource;
use crate::animation::sequence::CameraSequence;

/// Fit-overlay mutation derived from one high-level request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraFitTargetEffect {
    Preserve,
    Set(Entity),
}

/// Projection intent derived from the authored retained destination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum CameraProjectionEffectCandidate {
    Preserve,
    FitFreeFlightFromDestination(Radius),
}

/// Domain effects carried beside a request until final admission.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PendingCameraAdmissionEffects {
    pub(super) fit_target: CameraFitTargetEffect,
    pub(super) projection: CameraProjectionEffectCandidate,
}

impl PendingCameraAdmissionEffects {
    pub(super) fn derive(
        source: AnimationSource,
        target: Option<Entity>,
        sequence: &CameraSequence,
    ) -> Result<Self, CameraAdmissionEffectError> {
        let fits_target = matches!(
            source,
            AnimationSource::AnimateToFit
                | AnimationSource::ZoomToFit
                | AnimationSource::LookAtAndZoomToFit
        );
        if !fits_target {
            return Ok(Self {
                fit_target: CameraFitTargetEffect::Preserve,
                projection: CameraProjectionEffectCandidate::Preserve,
            });
        }
        let Some(target) = target else {
            return Err(CameraAdmissionEffectError::MissingFitTarget);
        };
        let destination = sequence
            .moves()
            .last()
            .ok_or(CameraAdmissionEffectError::MissingAuthoredDestination)?;
        Ok(Self {
            fit_target: CameraFitTargetEffect::Set(target),
            projection: CameraProjectionEffectCandidate::FitFreeFlightFromDestination(
                destination.radius(),
            ),
        })
    }
}

/// A projection mutation already checked against the live camera tuple.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PreparedCameraProjectionEffect {
    Preserve,
    SetOrthographicScale(f32),
}

/// Why one request's semantic fit/projection transaction could not be admitted.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(super) enum CameraAdmissionEffectError {
    #[error("a fit request did not name the target it fits")]
    MissingFitTarget,
    #[error("a fit request had no authored camera destination")]
    MissingAuthoredDestination,
    #[error("the fit target no longer exists")]
    MissingTargetEntity,
    #[error("a free-flight fit request requires a projection component")]
    MissingProjection,
    #[error("the authored destination cannot be used as an orthographic scale")]
    InvalidOrthographicScale,
}
