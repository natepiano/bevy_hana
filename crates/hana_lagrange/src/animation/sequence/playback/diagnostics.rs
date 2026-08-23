use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::warn;
use hana_kana::SequenceEasingError;
use hana_kana::SequenceOwner;
use hana_kana::SequenceScopeError;
use thiserror::Error;

/// Reasons a prepared camera pose cannot be evaluated.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq, Reflect)]
#[reflect(opaque)]
#[type_path = "hana_lagrange::animation::sequence"]
#[non_exhaustive]
pub enum CameraEvaluationError {
    /// The selected external curve cannot ease its scope.
    #[error("the external camera easing curve was rejected: {0}")]
    ExternalCurveRejected(SequenceEasingError),
    /// An easing path produced a NaN or infinite output.
    #[error("the camera easing curve produced a non-finite output")]
    NonFiniteEasing,
    /// Interpolation produced a pose that camera operations cannot represent.
    #[error("the interpolated camera pose is not representable")]
    UnrepresentablePose,
}

/// The evaluator whose exact result currently controls the camera pose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraEvaluationSource {
    Native,
    Driver(Entity),
}

impl From<SequenceOwner> for CameraEvaluationSource {
    fn from(owner: SequenceOwner) -> Self {
        match owner {
            SequenceOwner::NativePlayback => Self::Native,
            SequenceOwner::Driver(driver) => Self::Driver(driver),
        }
    }
}

/// Every semantic reason a retained evaluation holds its last controller pose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraEvaluationDiagnosticError {
    MissingDriverEvaluation,
    UnresolvedDriverScope(SequenceScopeError),
    Evaluation(CameraEvaluationError),
}

impl From<CameraEvaluationError> for CameraEvaluationDiagnosticError {
    fn from(error: CameraEvaluationError) -> Self { Self::Evaluation(error) }
}

/// Bounded reporting state for one exact evaluator source and failure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum CameraEvaluationDiagnosticState {
    #[default]
    Armed,
    Reported {
        source: CameraEvaluationSource,
        error:  CameraEvaluationDiagnosticError,
    },
}

impl CameraEvaluationDiagnosticState {
    pub(super) fn report(
        &mut self,
        source: CameraEvaluationSource,
        error: CameraEvaluationDiagnosticError,
    ) -> bool {
        if *self == (Self::Reported { source, error }) {
            return false;
        }
        *self = Self::Reported { source, error };
        true
    }

    pub(super) const fn rearm(&mut self) { *self = Self::Armed; }
}

pub(super) fn report_camera_evaluation_failure(
    camera: Entity,
    source: CameraEvaluationSource,
    error: CameraEvaluationDiagnosticError,
) {
    if matches!(
        error,
        CameraEvaluationDiagnosticError::Evaluation(CameraEvaluationError::ExternalCurveRejected(
            SequenceEasingError::MappingNotBoundedMonotonic
        ))
    ) {
        warn!(camera = ?camera, source = ?source, error = ?error, "camera retained evaluation held the prior pose; a single-stage scope accepts overshoot or reversal");
    } else {
        warn!(camera = ?camera, source = ?source, error = ?error, "camera retained evaluation held the prior pose");
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::Update;
    use bevy::prelude::Vec3;
    use hana_kana::EasingSample;
    use hana_kana::RangeCrossings;
    use hana_kana::SequenceDirection;
    use hana_kana::SequenceEasing;
    use hana_kana::SequenceEasingSample;
    use hana_kana::SequenceEvaluation;
    use hana_kana::SequenceMovement;
    use hana_kana::SequencePlaybackSystems;
    use hana_kana::SequencePosition;
    use hana_kana::SequenceScope;

    use super::*;
    use crate::animation::sequence::CameraSequence;
    use crate::animation::sequence::playback::*;
    use crate::animation::sequence::support::*;

    #[test]
    fn evaluation_diagnostics_report_exact_pairs_and_rearm_only_after_success() {
        let driver = Entity::from_raw_u32(17).expect("17 is a valid test entity index");
        let native = CameraEvaluationSource::Native;
        let selected = CameraEvaluationSource::Driver(driver);
        let mapping = CameraEvaluationDiagnosticError::Evaluation(
            CameraEvaluationError::ExternalCurveRejected(
                SequenceEasingError::MappingNotBoundedMonotonic,
            ),
        );
        let non_finite = CameraEvaluationDiagnosticError::Evaluation(
            CameraEvaluationError::ExternalCurveRejected(SequenceEasingError::NonFiniteOutput),
        );
        let mut state = CameraEvaluationDiagnosticState::Armed;

        assert!(state.report(native, mapping));
        assert!(!state.report(native, mapping));
        assert!(state.report(selected, mapping));
        assert!(!state.report(selected, mapping));
        assert!(state.report(selected, non_finite));
        assert_eq!(
            state,
            CameraEvaluationDiagnosticState::Reported {
                source: selected,
                error:  non_finite,
            }
        );
        state.rearm();
        assert_eq!(state, CameraEvaluationDiagnosticState::Armed);
        assert!(state.report(selected, non_finite));
    }

    #[test]
    fn missing_selected_driver_evaluation_holds_pose_after_raw_movement() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<CameraBoundaryEventOrder>();
        let camera = app
            .world_mut()
            .spawn((
                orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0),
                CameraSequence::new(move_lasting(Duration::from_secs(1))),
            ))
            .id();
        record_camera_boundary_order(app.world_mut(), camera);
        let driver = app
            .world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera))
            .id();
        app.add_systems(
            Update,
            despawn_selected_driver_after_movement
                .after(SequencePlaybackSystems::ApplyMovement)
                .before(SequencePlaybackSystems::EvaluateSequences),
        );
        app.update();
        let start_pose = app
            .world()
            .get::<CameraSequencePlayback>(camera)
            .ok_or("selected camera playback was not prepared")?
            .start
            .pose();
        let movement = SequenceMovement::try_new(
            SequencePosition::try_new(0.5).map_err(|_| "0.5 is a valid sequence position")?,
            SequenceDirection::Forward,
            0,
            RangeCrossings::NONE,
        )
        .map_err(|_| "the finite forward movement is valid")?;
        app.world_mut().entity_mut(driver).insert(movement);
        app.world_mut()
            .insert_resource(SelectedDriverDespawn(driver));

        app.update();

        let held = app
            .world()
            .get::<CameraSequencePlayback>(camera)
            .ok_or("selected camera playback remains retained")?;
        assert_eq!(held.playback.position(), movement.position());
        assert_eq!(held.last_applied.0, start_pose);
        assert_eq!(
            held.evaluation_diagnostic_state,
            CameraEvaluationDiagnosticState::Reported {
                source: CameraEvaluationSource::Driver(driver),
                error:  CameraEvaluationDiagnosticError::MissingDriverEvaluation,
            }
        );
        assert_eq!(
            app.world().resource::<CameraBoundaryEventOrder>().0,
            [("begin", Duration::from_secs(1))]
        );
        Ok(())
    }

    #[test]
    fn invalid_selected_driver_evaluation_rearms_at_current_raw_position() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<CameraBoundaryEventOrder>();
        let sequence = CameraSequence::new(move_lasting(Duration::from_secs(1)));
        let description_revision = sequence.sequence_stages().revision();
        let camera = app
            .world_mut()
            .spawn((orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0), sequence))
            .id();
        record_camera_boundary_order(app.world_mut(), camera);
        let driver = app
            .world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera))
            .id();
        app.update();
        let start_pose = app
            .world()
            .get::<CameraSequencePlayback>(camera)
            .ok_or("selected camera playback was not prepared")?
            .start
            .pose();
        let stale_sequence = CameraSequence::new(move_lasting(Duration::from_secs(1)));
        let stale_revision = stale_sequence.sequence_stages().revision();
        let stale_stage = stale_sequence
            .stage_ids_with_spans()
            .next()
            .ok_or("the stale sequence retains its one stage")?
            .0;
        let movement = SequenceMovement::try_new(
            SequencePosition::try_new(0.5).map_err(|_| "0.5 is a valid sequence position")?,
            SequenceDirection::Forward,
            0,
            RangeCrossings::NONE,
        )
        .map_err(|_| "the finite forward movement is valid")?;
        app.world_mut().entity_mut(driver).insert((
            movement,
            SequenceEvaluation::new(SequenceScope::Stage(stale_stage), SequenceEasing::Authored),
        ));

        app.update();

        let held = app
            .world()
            .get::<CameraSequencePlayback>(camera)
            .ok_or("selected camera playback remains retained")?;
        assert_eq!(held.playback.position(), movement.position());
        assert_eq!(held.last_applied.0, start_pose);
        assert_eq!(
            held.evaluation_diagnostic_state,
            CameraEvaluationDiagnosticState::Reported {
                source: CameraEvaluationSource::Driver(driver),
                error:  CameraEvaluationDiagnosticError::UnresolvedDriverScope(
                    SequenceScopeError::StaleRevision {
                        scope:       stale_revision,
                        description: description_revision,
                    },
                ),
            }
        );
        assert_eq!(
            app.world().resource::<CameraBoundaryEventOrder>().0,
            [("begin", Duration::from_secs(1))]
        );

        app.world_mut()
            .entity_mut(driver)
            .insert(SequenceEvaluation::AUTHORED_WHOLE);
        app.update();

        let recovered = app
            .world()
            .get::<CameraSequencePlayback>(camera)
            .ok_or("selected camera playback remains retained after recovery")?;
        assert_eq!(recovered.playback.position(), movement.position());
        assert_ne!(recovered.last_applied.0, start_pose);
        assert_eq!(
            recovered.evaluation_diagnostic_state,
            CameraEvaluationDiagnosticState::Armed
        );
        assert_eq!(
            app.world().resource::<CameraBoundaryEventOrder>().0,
            [("begin", Duration::from_secs(1))]
        );
        Ok(())
    }

    #[test]
    fn camera_evaluation_reports_the_exact_failure_reason() -> TestResult {
        let sequence = CameraSequence::new(orbital_move(
            Vec3::new(1.0, 0.0, 0.0),
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ));
        let playback = prepared_orbit(&sequence);
        let sample = moving_sample(&playback, 0.5)?;
        assert_eq!(
            evaluate_camera_pose(
                sample,
                |_| SequenceEasingSample::CurveRejected(
                    SequenceEasingError::MappingNotBoundedMonotonic,
                ),
                |_, _| EasingSample::NonFinite,
            ),
            Err(CameraEvaluationError::ExternalCurveRejected(
                SequenceEasingError::MappingNotBoundedMonotonic,
            ))
        );
        assert_eq!(
            evaluate_camera_pose(
                sample,
                |_| SequenceEasingSample::AuthoredEasingApplies { progress: 0.5 },
                |_, _| EasingSample::NonFinite,
            ),
            Err(CameraEvaluationError::NonFiniteEasing)
        );

        let sequence = CameraSequence::new(orbital_move(
            Vec3::splat(f32::MAX),
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ));
        let camera = orbit_camera(Vec3::splat(-f32::MAX), 0.0, 0.0, 1.0);
        let playback = CameraSequencePlayback::prepare_for_orbit(
            &sequence,
            &camera,
            OrbitControllerInstallation::default(),
        )
        .map_err(|_| "finite camera endpoints prepare")?;
        assert_eq!(
            evaluate_camera_pose(
                moving_sample(&playback, 0.5)?,
                |_| SequenceEasingSample::AuthoredEasingSuppressed { eased: 2.0 },
                |_, _| EasingSample::NonFinite,
            ),
            Err(CameraEvaluationError::UnrepresentablePose)
        );

        Ok(())
    }
}
