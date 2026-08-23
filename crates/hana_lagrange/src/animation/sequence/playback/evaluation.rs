use bevy::prelude::Entity;
use bevy::prelude::Query;
use hana_kana::EasingSampler;
use hana_kana::SequenceCommands;
use hana_kana::SequenceEasing;
use hana_kana::SequenceEasingSample;
use hana_kana::SequenceEasingSampler;
use hana_kana::SequenceEvaluation;
use hana_kana::SequenceOwner;
use hana_kana::SequencePosition;
use hana_kana::SequenceRange;
use hana_kana::SequenceScope;

use super::CameraMoveSample;
use super::CameraPlaybackLifecycleState;
use super::CameraSequencePlayback;
use super::diagnostics;
use super::diagnostics::CameraEvaluationDiagnosticError;
use super::diagnostics::CameraEvaluationError;
use super::diagnostics::CameraEvaluationSource;
use super::pose;
use super::pose::CameraPose;
use super::pose::LastAppliedCameraPose;
use crate::FreeCam;
use crate::OrbitCam;
use crate::animation::sequence::CameraSequence;
use crate::animation::sequence::controller_installation;

/// Evaluates the retained endpoint interval at the single current local
/// position and writes controller current and target values together.
pub(in crate::animation) fn evaluate_camera_sequences(
    sequence_commands: SequenceCommands,
    evaluations: Query<&SequenceEvaluation>,
    mut sequences: Query<(Entity, &CameraSequence, &mut CameraSequencePlayback)>,
    mut orbit_cameras: Query<&mut OrbitCam>,
    mut free_cameras: Query<&mut FreeCam>,
) {
    let easing_sampler = EasingSampler;
    let sequence_easing_sampler = SequenceEasingSampler;
    for (camera, sequence, mut retained) in &mut sequences {
        let owner = sequence_commands.owner(camera);
        let source = CameraEvaluationSource::from(owner);
        let evaluated = evaluate_retained_camera_pose(
            sequence,
            &retained,
            owner,
            &evaluations,
            sequence_easing_sampler,
            easing_sampler,
        );
        let pose = match evaluated {
            Ok(pose) => {
                retained.evaluation_diagnostic_state.rearm();
                pose
            },
            Err(error) => {
                if retained.evaluation_diagnostic_state.report(source, error) {
                    diagnostics::report_camera_evaluation_failure(camera, source, error);
                }
                if camera_lifecycle_enforces_output(retained.lifecycle) {
                    write_camera_controller_pose(
                        camera,
                        retained.last_applied.0,
                        &mut orbit_cameras,
                        &mut free_cameras,
                    );
                }
                continue;
            },
        };
        if !camera_lifecycle_enforces_output(retained.lifecycle) {
            continue;
        }
        let controller_already_holds_pose = orbit_cameras
            .get(camera)
            .is_ok_and(|orbit| controller_installation::orbit_target_matches_pose(orbit, pose))
            || free_cameras
                .get(camera)
                .is_ok_and(|free| controller_installation::free_target_matches_pose(free, pose));
        if retained.last_applied.0 == pose && controller_already_holds_pose {
            continue;
        }
        if write_camera_controller_pose(camera, pose, &mut orbit_cameras, &mut free_cameras) {
            retained.last_applied = LastAppliedCameraPose(pose);
        }
    }
}

#[derive(Clone, Copy)]
enum CameraSampleEasing<'evaluation> {
    Authored,
    Suppressed,
    External {
        scope:  SequenceScope,
        easing: &'evaluation SequenceEasing,
    },
}

fn evaluate_retained_camera_pose(
    sequence: &CameraSequence,
    retained: &CameraSequencePlayback,
    owner: SequenceOwner,
    evaluations: &Query<&SequenceEvaluation>,
    sequence_easing_sampler: SequenceEasingSampler,
    easing_sampler: EasingSampler,
) -> Result<CameraPose, CameraEvaluationDiagnosticError> {
    let raw_position = retained.playback.position();
    let SequenceOwner::Driver(driver) = owner else {
        return evaluate_camera_sample(
            retained.sample(raw_position),
            CameraSampleEasing::Authored,
            sequence_easing_sampler,
            easing_sampler,
        )
        .map_err(CameraEvaluationDiagnosticError::Evaluation);
    };
    let evaluation = evaluations
        .get(driver)
        .map_err(|_| CameraEvaluationDiagnosticError::MissingDriverEvaluation)?;
    let scope = evaluation.scope();
    let range = sequence
        .sequence_stages()
        .resolve(scope)
        .map_err(CameraEvaluationDiagnosticError::UnresolvedDriverScope)?;
    match (evaluation.easing(), scope) {
        (SequenceEasing::Authored, _) => evaluate_camera_sample(
            retained.sample(raw_position),
            CameraSampleEasing::Authored,
            sequence_easing_sampler,
            easing_sampler,
        ),
        (easing, SequenceScope::Stage(stage_id)) => {
            let sample = retained.sample(raw_position);
            let applies = matches!(
                sample,
                CameraMoveSample::Moving { endpoint, .. } if endpoint.stage_id == stage_id
            );
            evaluate_camera_sample(
                sample,
                if applies {
                    CameraSampleEasing::External { scope, easing }
                } else {
                    CameraSampleEasing::Authored
                },
                sequence_easing_sampler,
                easing_sampler,
            )
        },
        (easing, SequenceScope::WholeSequence | SequenceScope::StageRange { .. }) => {
            let progress = range.progress(raw_position);
            match sequence_easing_sampler.sample(scope, easing, progress) {
                SequenceEasingSample::AuthoredEasingApplies { progress } => evaluate_camera_sample(
                    retained.sample(remapped_camera_position(range, progress)?),
                    CameraSampleEasing::Authored,
                    sequence_easing_sampler,
                    easing_sampler,
                ),
                SequenceEasingSample::AuthoredEasingSuppressed { eased } => evaluate_camera_sample(
                    retained.sample(remapped_camera_position(range, eased)?),
                    CameraSampleEasing::Suppressed,
                    sequence_easing_sampler,
                    easing_sampler,
                ),
                SequenceEasingSample::CurveRejected(error) => {
                    Err(CameraEvaluationError::ExternalCurveRejected(error))
                },
            }
        },
    }
    .map_err(CameraEvaluationDiagnosticError::Evaluation)
}

fn evaluate_camera_sample(
    sample: CameraMoveSample<'_>,
    easing: CameraSampleEasing<'_>,
    sequence_easing_sampler: SequenceEasingSampler,
    easing_sampler: EasingSampler,
) -> Result<CameraPose, CameraEvaluationError> {
    pose::evaluate_camera_pose(
        sample,
        |raw_progress| match easing {
            CameraSampleEasing::Authored => SequenceEasingSample::AuthoredEasingApplies {
                progress: raw_progress,
            },
            CameraSampleEasing::Suppressed => SequenceEasingSample::AuthoredEasingSuppressed {
                eased: raw_progress,
            },
            CameraSampleEasing::External { scope, easing } => {
                sequence_easing_sampler.sample(scope, easing, raw_progress)
            },
        },
        |authored, progress| easing_sampler.sample(authored, progress),
    )
}

fn remapped_camera_position(
    range: SequenceRange,
    progress: f32,
) -> Result<SequencePosition, CameraEvaluationError> {
    let start = range.start().normalized();
    let end = range.end().normalized();
    SequencePosition::try_new((end - start).mul_add(progress, start))
        .map_err(|_| CameraEvaluationError::UnrepresentablePose)
}

const fn camera_lifecycle_enforces_output(lifecycle: CameraPlaybackLifecycleState) -> bool {
    matches!(
        lifecycle,
        CameraPlaybackLifecycleState::Effective { .. }
            | CameraPlaybackLifecycleState::Closing { .. }
    )
}

fn write_camera_controller_pose(
    camera: Entity,
    pose: CameraPose,
    orbit_cameras: &mut Query<&mut OrbitCam>,
    free_cameras: &mut Query<&mut FreeCam>,
) -> bool {
    match pose {
        CameraPose::Orbit(pose) => {
            let Ok(mut orbit) = orbit_cameras.get_mut(camera) else {
                return false;
            };
            orbit.pan.snap_to(pose.focus);
            orbit.orbit.snap_to(pose.orbit_angles);
            orbit.zoom.snap_to(pose.radius);
            orbit.force_update();
        },
        CameraPose::Free(pose) => {
            let Ok(mut free) = free_cameras.get_mut(camera) else {
                return false;
            };
            free.translate.snap_to(pose.position);
            free.look.snap_to(pose.look);
            free.roll.snap_to(pose.roll);
            free.force_update();
        },
    }
    true
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::Vec3;

    use super::*;
    use crate::animation::sequence::playback::*;
    use crate::animation::sequence::support::*;
    use crate::operation::Focus;

    #[test]
    fn stage_overshoot_and_reversal_keep_raw_time_ordered() -> TestResult {
        let sequence = CameraSequence::new(orbital_move(
            Vec3::new(10.0, 0.0, 0.0),
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ));
        let playback = prepared_orbit(&sequence);
        let early_sample = moving_sample(&playback, 0.25)?;
        let late_sample = moving_sample(&playback, 0.75)?;
        let CameraMoveSample::Moving {
            raw_progress: early_progress,
            ..
        } = early_sample
        else {
            return Err("the early sample moves");
        };
        let CameraMoveSample::Moving {
            raw_progress: late_progress,
            ..
        } = late_sample
        else {
            return Err("the late sample moves");
        };
        assert!(early_progress.normalized() < late_progress.normalized());

        let CameraPose::Orbit(early_pose) = pose::evaluate_camera_pose(
            early_sample,
            |_| SequenceEasingSample::AuthoredEasingSuppressed { eased: 1.25 },
            |_, _| EasingSample::NonFinite,
        )
        .map_err(|_| "finite overshoot evaluates the prepared camera pose")?
        else {
            return Err("orbit playback evaluates an orbit pose");
        };
        let CameraPose::Orbit(late_pose) = pose::evaluate_camera_pose(
            late_sample,
            |_| SequenceEasingSample::AuthoredEasingSuppressed { eased: -0.25 },
            |_, _| EasingSample::NonFinite,
        )
        .map_err(|_| "finite reversal evaluates the prepared camera pose")?
        else {
            return Err("orbit playback evaluates an orbit pose");
        };
        assert_eq!(early_pose.focus, Focus(Vec3::new(12.5, 0.0, 0.0)));
        assert_eq!(late_pose.focus, Focus(Vec3::new(-2.5, 0.0, 0.0)));

        Ok(())
    }
}
