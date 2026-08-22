mod diagnostics;
mod emission;
mod evaluation;
mod ledger;
mod lifecycle;
mod movement;
mod pose;

use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy_kana::EasingSample;
use bevy_kana::SequenceDirection;
use bevy_kana::SequenceEasingSample;
use bevy_kana::SequencePlayback;
use bevy_kana::SequencePlaybackError;
use bevy_kana::SequencePosition;
use bevy_kana::SequenceStagesRevision;
use diagnostics::CameraEvaluationDiagnosticState;
pub use diagnostics::CameraEvaluationError;
use emission::emit_camera_boundaries;
pub(super) use emission::emit_cancelled_camera_lifecycle;
pub(in crate::animation) use evaluation::evaluate_camera_sequences;
use ledger::CameraBoundaryLedger;
use ledger::CameraMoveInterval;
use ledger::CameraMoveProgress;
use ledger::ResolvedCameraMoveEndpoint;
pub(super) use lifecycle::CameraPlaybackClosureReason;
pub(super) use lifecycle::CameraPlaybackInterruptionState;
use lifecycle::CameraPlaybackInterruptionTransition;
pub(super) use lifecycle::CameraPlaybackLifecycleState;
use lifecycle::DriverEpisode;
use lifecycle::begin_selected_driver_lifecycle;
use lifecycle::camera_journey_matches;
use lifecycle::camera_lifecycle_owner_and_direction;
pub(in crate::animation) use lifecycle::finalize_camera_sequence_transitions;
use lifecycle::transition_camera_lifecycle_owner;
pub(in crate::animation) use movement::apply_camera_sequence_movement;
pub(super) use pose::CameraPose;
use pose::CameraSequenceStartPose;
pub(super) use pose::LastAppliedCameraPose;
#[cfg(test)]
pub(super) use pose::OrbitCameraPose;
use pose::evaluate_camera_pose;
use pose::normalized_camera_time;
use pose::resolve_free_camera_endpoint;
use thiserror::Error;

use super::CameraSequence;
use super::controller_installation::*;
use crate::CameraBasis;
use crate::FreeCam;
use crate::OrbitCam;
use crate::animation::queue::CameraMove;

/// Derived state that was invalid before retained playback could begin.
#[derive(Debug, Error)]
pub(super) enum CameraPlaybackPreparationError {
    #[error("the prepared camera boundary ledger was rejected: {0}")]
    InvalidBoundaryLedger(#[from] SequencePlaybackError),
    #[error("the prepared camera playback changed its boundary ledger")]
    BoundaryLedgerMismatch,
    #[error("the prepared camera playback did not begin monitoring interruption")]
    InvalidInterruptionState,
    #[error("the prepared camera pose is not representable")]
    UnrepresentablePose,
}

/// The immutable start and destination values needed to evaluate one position.
#[derive(Clone, Copy, Debug)]
pub(super) enum CameraMoveSample<'playback> {
    Resting {
        pose: &'playback CameraPose,
    },
    Moving {
        start:        &'playback CameraPose,
        endpoint:     &'playback ResolvedCameraMoveEndpoint,
        raw_progress: CameraMoveProgress,
    },
}

/// Retained local playback and immutable evaluation state for one camera sequence.
///
/// This component is private because applications command the public retained
/// camera surface rather than mutating the local position or captured poses.
/// It owns no lease, frame result, or live-camera reference: preparation reads
/// a controller once, then sampling reads only this value.
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(opaque)]
#[reflect(Component)]
pub(crate) struct CameraSequencePlayback {
    pub(super) playback:                SequencePlayback,
    pub(super) start:                   CameraSequenceStartPose,
    endpoints:                          Vec<ResolvedCameraMoveEndpoint>,
    last_applied:                       LastAppliedCameraPose,
    pub(super) interruption:            CameraPlaybackInterruptionState,
    boundary_ledger:                    CameraBoundaryLedger,
    pub(super) controller_installation: CameraControllerInstallation,
    pub(super) revision:                SequenceStagesRevision,
    pub(super) lifecycle:               CameraPlaybackLifecycleState,
    driver_episode:                     DriverEpisode,
    last_direction:                     SequenceDirection,
    evaluation_diagnostic_state:        CameraEvaluationDiagnosticState,
}

/// Marks a retained sequence that final request admission accepted this frame.
/// The movement stage consumes it through [`CameraCommands`] after the commit
/// has become visible, so native play never bypasses command arbitration.
#[derive(Component)]
pub(in crate::animation) struct NativeCameraPlayRequest;

impl CameraSequencePlayback {
    pub(super) fn prepare_for_orbit(
        sequence: &CameraSequence,
        camera: &OrbitCam,
        installation: OrbitControllerInstallation,
    ) -> Result<Self, CameraPlaybackPreparationError> {
        let start = CameraSequenceStartPose::try_from(camera)?;
        Self::prepare(
            sequence,
            start,
            CameraControllerInstallation::Orbit(installation),
            |camera_move, _| {
                CameraPose::try_orbit(
                    camera_move.focus(),
                    camera_move.orbit_angles(),
                    camera_move.radius(),
                )
            },
        )
    }

    pub(super) fn prepare_for_free(
        sequence: &CameraSequence,
        camera: &FreeCam,
        basis: CameraBasis,
        installation: FreeFlightControllerInstallation,
    ) -> Result<Self, CameraPlaybackPreparationError> {
        let start = CameraSequenceStartPose::try_from(camera)?;
        Self::prepare(
            sequence,
            start,
            CameraControllerInstallation::FreeFlight {
                installation,
                basis,
            },
            |camera_move, previous| resolve_free_camera_endpoint(camera_move, previous, basis),
        )
    }

    fn prepare(
        sequence: &CameraSequence,
        start: CameraSequenceStartPose,
        controller_installation: CameraControllerInstallation,
        resolve_endpoint: impl Fn(&CameraMove, CameraPose) -> Result<CameraPose, CameraEvaluationError>,
    ) -> Result<Self, CameraPlaybackPreparationError> {
        let total = sequence.total();
        let mut previous = start.pose();
        let mut endpoints = Vec::with_capacity(sequence.moves().len());
        for (camera_move, (stage_id, span)) in
            sequence.moves().iter().zip(sequence.stage_ids_with_spans())
        {
            let pose = resolve_endpoint(camera_move, previous)
                .map_err(|_| CameraPlaybackPreparationError::UnrepresentablePose)?;
            endpoints.push(ResolvedCameraMoveEndpoint {
                stage_id,
                span,
                interval: CameraMoveInterval::from_span(span, total),
                pose,
                easing: camera_move.easing().clone(),
            });
            previous = pose;
        }

        let boundary_ledger = CameraBoundaryLedger::from(endpoints.as_slice());
        let playback = SequencePlayback::try_new(boundary_ledger.boundary_positions())?;
        Self {
            playback,
            start,
            endpoints,
            last_applied: LastAppliedCameraPose(start.pose()),
            interruption: CameraPlaybackInterruptionState::Monitoring,
            boundary_ledger,
            controller_installation,
            revision: sequence.sequence_stages().revision(),
            lifecycle: CameraPlaybackLifecycleState::Dormant,
            driver_episode: DriverEpisode::Closed,
            last_direction: SequenceDirection::Forward,
            evaluation_diagnostic_state: CameraEvaluationDiagnosticState::Armed,
        }
        .with_captured_start_applied()
    }

    /// Establishes the exact captured start through the common evaluator.
    ///
    /// This is preparation only: it records the pose retained playback will
    /// compare against later, without writing a controller or advancing
    /// [`SequencePlayback`].
    fn with_captured_start_applied(mut self) -> Result<Self, CameraPlaybackPreparationError> {
        if !self.boundary_ledger_matches_playback() {
            return Err(CameraPlaybackPreparationError::BoundaryLedgerMismatch);
        }
        if !self.interruption.is_monitoring() {
            return Err(CameraPlaybackPreparationError::InvalidInterruptionState);
        }
        let initial_pose = evaluate_camera_pose(
            self.captured_start_sample(),
            |_| SequenceEasingSample::AuthoredEasingSuppressed { eased: 0.0 },
            |_, _| EasingSample::NonFinite,
        )
        .map_err(|_| CameraPlaybackPreparationError::UnrepresentablePose)?;
        self.last_applied = LastAppliedCameraPose(initial_pose);
        Ok(self)
    }

    /// Returns whether the immutable ledger still matches playback ordinals.
    fn boundary_ledger_matches_playback(&self) -> bool {
        self.playback
            .boundary_positions()
            .iter()
            .copied()
            .eq(self.boundary_ledger.boundary_positions())
    }

    /// Samples the captured start before any zero-duration boundary traverses.
    fn captured_start_sample(&self) -> CameraMoveSample<'_> {
        let Some(endpoint) = self.endpoints.first() else {
            return CameraMoveSample::Resting {
                pose: &self.start.0,
            };
        };
        if !endpoint.interval.has_interior() {
            return CameraMoveSample::Resting {
                pose: &self.start.0,
            };
        }
        self.sample(SequencePosition::START)
    }

    pub(super) fn sample(&self, position: SequencePosition) -> CameraMoveSample<'_> {
        let normalized_position = f64::from(position.normalized());
        let mut start = &self.start.0;
        for endpoint in &self.endpoints {
            if normalized_position < endpoint.interval.begin {
                break;
            }
            if endpoint.interval.has_interior() && normalized_position < endpoint.interval.end {
                return CameraMoveSample::Moving {
                    start,
                    endpoint,
                    raw_progress: CameraMoveProgress::from_position(
                        normalized_position,
                        endpoint.interval,
                    ),
                };
            }
            start = &endpoint.pose;
        }
        CameraMoveSample::Resting { pose: start }
    }

    pub(crate) const fn is_native_playing(&self) -> bool { self.playback.is_playing() }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::math::curve::easing::EaseFunction;
    use bevy::prelude::Vec3;
    use bevy_kana::Easing;
    use bevy_kana::SequenceEasingSampler;
    use bevy_kana::SequenceScope;

    use super::*;
    use crate::animation::sequence::support::*;
    use crate::operation::Focus;
    use crate::operation::OrbitAngles;
    use crate::operation::Radius;

    #[test]
    fn prepared_playback_keeps_captured_endpoints_after_the_live_camera_changes() -> TestResult {
        let sequence = CameraSequence::new(orbital_move(
            Vec3::new(4.0, 5.0, 6.0),
            0.75,
            -0.5,
            3.0,
            Duration::from_secs(1),
        ));
        let mut camera = orbit_camera(Vec3::new(-1.0, 2.0, -3.0), 0.25, 0.5, 2.0);
        let playback = CameraSequencePlayback::prepare_for_orbit(
            &sequence,
            &camera,
            OrbitControllerInstallation::default(),
        )
        .map_err(|_| "the valid sequence prepares")?;

        camera.pan.snap_to(Focus(Vec3::splat(99.0)));
        camera.orbit.snap_to(OrbitAngles {
            yaw:   -2.0,
            pitch: 1.0,
        });
        camera.zoom.snap_to(Radius(99.0));

        let CameraMoveSample::Resting { pose } = playback.sample(SequencePosition::END) else {
            return Err("the completed sequence rests on its captured endpoint");
        };
        let CameraPose::Orbit(endpoint) = *pose else {
            return Err("orbit preparation retains an orbit endpoint");
        };
        assert_eq!(endpoint.focus, Focus(Vec3::new(4.0, 5.0, 6.0)));
        assert!((endpoint.orbit_angles.yaw - 0.75).abs() <= f32::EPSILON);
        assert!((endpoint.orbit_angles.pitch + 0.5).abs() <= f32::EPSILON);
        assert_eq!(endpoint.radius, Radius(3.0));
        assert_eq!(playback.last_applied.0, playback.start.pose());
        assert_eq!(
            playback.interruption,
            CameraPlaybackInterruptionState::Monitoring
        );

        Ok(())
    }

    #[test]
    fn replacement_and_composition_leave_the_captured_authored_easing_unchanged() {
        let sequence = CameraSequence::new(orbital_move(
            Vec3::new(2.0, 0.0, 0.0),
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ));
        let playback = prepared_orbit(&sequence);
        let endpoint = &playback.endpoints[0];
        let authored_easing = endpoint.easing.clone();
        let sampler = SequenceEasingSampler;
        let scope = SequenceScope::Stage(endpoint.stage_id);

        assert_eq!(
            sampler.sample(
                scope,
                &bevy_kana::SequenceEasing::ReplacedBy(Easing::Bevy(EaseFunction::QuadraticIn)),
                0.5,
            ),
            SequenceEasingSample::AuthoredEasingSuppressed { eased: 0.25 }
        );
        assert_eq!(
            sampler.sample(
                scope,
                &bevy_kana::SequenceEasing::ComposedWith(Easing::Bevy(EaseFunction::QuadraticIn)),
                0.5,
            ),
            SequenceEasingSample::AuthoredEasingApplies { progress: 0.25 }
        );
        assert_eq!(playback.endpoints[0].easing, authored_easing);
    }

    #[test]
    fn preparation_reuses_retained_stage_identities_and_samples_without_history() -> TestResult {
        let sequence = CameraSequence::new(orbital_move(
            Vec3::new(2.0, 0.0, 0.0),
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ))
        .then(orbital_move(
            Vec3::new(6.0, 0.0, 0.0),
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ));
        let retained_ids: Vec<_> = sequence
            .stage_ids_with_spans()
            .map(|(stage_id, _)| stage_id)
            .collect();
        let playback = prepared_orbit(&sequence);
        assert_eq!(
            playback
                .endpoints
                .iter()
                .map(|endpoint| endpoint.stage_id)
                .collect::<Vec<_>>(),
            retained_ids
        );

        let first = evaluate_camera_pose(
            moving_sample(&playback, 0.75)?,
            |_| SequenceEasingSample::AuthoredEasingSuppressed { eased: 0.5 },
            |_, _| EasingSample::NonFinite,
        )
        .map_err(|_| "the first arbitrary sample evaluates")?;
        evaluate_camera_pose(
            playback.sample(SequencePosition::END),
            |_| SequenceEasingSample::AuthoredEasingSuppressed { eased: 0.5 },
            |_, _| EasingSample::NonFinite,
        )
        .map_err(|_| "the endpoint sample evaluates")?;
        let second = evaluate_camera_pose(
            moving_sample(&playback, 0.75)?,
            |_| SequenceEasingSample::AuthoredEasingSuppressed { eased: 0.5 },
            |_, _| EasingSample::NonFinite,
        )
        .map_err(|_| "the second arbitrary sample evaluates")?;
        assert_eq!(first, second);

        Ok(())
    }
}
