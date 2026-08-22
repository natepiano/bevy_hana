use bevy::prelude::Quat;
use bevy::prelude::Vec3;
use bevy_kana::Easing;
use bevy_kana::EasingSample;
use bevy_kana::SequenceEasingSample;
use bevy_kana::SequenceTime;
use bevy_kana::ToF32;

use super::CameraMoveSample;
use super::CameraPlaybackPreparationError;
use super::diagnostics::CameraEvaluationError;
use crate::CameraBasis;
use crate::FreeCam;
use crate::LookAngles;
use crate::OrbitAngles;
use crate::OrbitCam;
use crate::Position;
use crate::Radius;
use crate::Roll;
use crate::animation::queue::CameraMove;
use crate::animation::queue::CameraMoveDestination;
use crate::animation::queue::FreeCamRollTarget;

/// One orbit camera pose whose coordinates have already been captured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OrbitCameraPose {
    pub(in crate::animation::sequence) focus:        crate::Focus,
    pub(in crate::animation::sequence) orbit_angles: OrbitAngles,
    pub(in crate::animation::sequence) radius:       Radius,
}

/// One free-flight camera pose whose coordinates have already been captured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FreeCameraPose {
    pub(in crate::animation::sequence) position: Position,
    pub(in crate::animation::sequence) look:     LookAngles,
    pub(in crate::animation::sequence) roll:     Roll,
}

/// A camera pose in the coordinate system its controller owns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CameraPose {
    Orbit(OrbitCameraPose),
    Free(FreeCameraPose),
}

impl CameraPose {
    pub(in crate::animation::sequence) fn try_orbit(
        focus: crate::Focus,
        orbit_angles: OrbitAngles,
        radius: Radius,
    ) -> Result<Self, CameraEvaluationError> {
        let is_representable = focus.0.is_finite()
            && orbit_angles.yaw.is_finite()
            && orbit_angles.pitch.is_finite()
            && radius.0.is_finite();
        if !is_representable {
            return Err(CameraEvaluationError::UnrepresentablePose);
        }
        Ok(Self::Orbit(OrbitCameraPose {
            focus,
            orbit_angles,
            radius,
        }))
    }

    pub(in crate::animation::sequence) fn try_free(
        position: Position,
        look: LookAngles,
        roll: Roll,
    ) -> Result<Self, CameraEvaluationError> {
        let is_representable = position.0.is_finite()
            && look.yaw.is_finite()
            && look.pitch.is_finite()
            && roll.0.is_finite();
        if !is_representable {
            return Err(CameraEvaluationError::UnrepresentablePose);
        }
        Ok(Self::Free(FreeCameraPose {
            position,
            look,
            roll,
        }))
    }

    pub(in crate::animation::sequence) fn interpolate(
        self,
        endpoint: Self,
        eased: f64,
    ) -> Result<Self, CameraEvaluationError> {
        match (self, endpoint) {
            (Self::Orbit(start), Self::Orbit(endpoint)) => Self::try_orbit(
                crate::Focus(interpolate_vec3(start.focus.0, endpoint.focus.0, eased)?),
                OrbitAngles {
                    yaw:   interpolate_angle(
                        start.orbit_angles.yaw,
                        endpoint.orbit_angles.yaw,
                        eased,
                    )?,
                    pitch: interpolate_angle(
                        start.orbit_angles.pitch,
                        endpoint.orbit_angles.pitch,
                        eased,
                    )?,
                },
                Radius(interpolate_scalar(
                    start.radius.0,
                    endpoint.radius.0,
                    eased,
                )?),
            ),
            (Self::Free(start), Self::Free(endpoint)) => Self::try_free(
                Position(interpolate_vec3(
                    start.position.0,
                    endpoint.position.0,
                    eased,
                )?),
                LookAngles {
                    yaw:   interpolate_angle(start.look.yaw, endpoint.look.yaw, eased)?,
                    pitch: interpolate_angle(start.look.pitch, endpoint.look.pitch, eased)?,
                },
                Roll(interpolate_angle(start.roll.0, endpoint.roll.0, eased)?),
            ),
            (Self::Orbit(_) | Self::Free(_), Self::Orbit(_) | Self::Free(_)) => {
                Err(CameraEvaluationError::UnrepresentablePose)
            },
        }
    }

    pub(in crate::animation::sequence) const fn inherited_free_roll(
        self,
    ) -> Result<Roll, CameraEvaluationError> {
        match self {
            Self::Free(pose) => Ok(pose.roll),
            Self::Orbit(_) => Err(CameraEvaluationError::UnrepresentablePose),
        }
    }
}

/// The live camera pose captured at local sequence position zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::animation::sequence) struct CameraSequenceStartPose(pub(super) CameraPose);

impl CameraSequenceStartPose {
    pub(in crate::animation::sequence) const fn pose(self) -> CameraPose { self.0 }
}

impl TryFrom<&OrbitCam> for CameraSequenceStartPose {
    type Error = CameraPlaybackPreparationError;

    fn try_from(camera: &OrbitCam) -> Result<Self, Self::Error> {
        CameraPose::try_orbit(
            camera.pan.current(),
            camera.orbit.current(),
            camera.zoom.current(),
        )
        .map(Self)
        .map_err(|_| CameraPlaybackPreparationError::UnrepresentablePose)
    }
}

impl TryFrom<&FreeCam> for CameraSequenceStartPose {
    type Error = CameraPlaybackPreparationError;

    fn try_from(camera: &FreeCam) -> Result<Self, Self::Error> {
        CameraPose::try_free(
            camera.translate.current(),
            camera.look.current(),
            camera.roll.current(),
        )
        .map(Self)
        .map_err(|_| CameraPlaybackPreparationError::UnrepresentablePose)
    }
}

/// The last pose the retained evaluator applied to a camera controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::animation::sequence) struct LastAppliedCameraPose(
    pub(in crate::animation::sequence) CameraPose,
);

/// Evaluates one prepared camera move at an already-selected easing path.
///
/// `select_easing` receives the raw interval progress and returns the
/// producer's decision. Authored stage easing is sampled only when that
/// decision says it still applies; replacement output skips it. Resting
/// positions return their captured endpoint exactly, so completion never
/// re-interpolates a value close to the endpoint.
pub(super) fn evaluate_camera_pose(
    sample: CameraMoveSample<'_>,
    select_easing: impl FnOnce(f32) -> SequenceEasingSample,
    sample_authored_easing: impl FnOnce(&Easing, f32) -> EasingSample,
) -> Result<CameraPose, CameraEvaluationError> {
    let (start, endpoint, raw_progress) = match sample {
        CameraMoveSample::Resting { pose } => return Ok(*pose),
        CameraMoveSample::Moving {
            start,
            endpoint,
            raw_progress,
        } => (start, endpoint, raw_progress),
    };

    let eased = match select_easing(raw_progress.normalized()) {
        SequenceEasingSample::AuthoredEasingApplies { progress } => {
            match sample_authored_easing(&endpoint.easing, progress) {
                EasingSample::Eased(eased) if eased.is_finite() => f64::from(eased),
                EasingSample::Eased(_) | EasingSample::NonFinite => {
                    return Err(CameraEvaluationError::NonFiniteEasing);
                },
            }
        },
        SequenceEasingSample::AuthoredEasingSuppressed { eased } if eased.is_finite() => {
            f64::from(eased)
        },
        SequenceEasingSample::AuthoredEasingSuppressed { .. } => {
            return Err(CameraEvaluationError::NonFiniteEasing);
        },
        SequenceEasingSample::CurveRejected(error) => {
            return Err(CameraEvaluationError::ExternalCurveRejected(error));
        },
    };

    start.interpolate(endpoint.pose, eased)
}

/// Resolves one free-flight move endpoint without consulting later controller output.
pub(super) fn resolve_free_camera_endpoint(
    camera_move: &CameraMove,
    previous: CameraPose,
    basis: CameraBasis,
) -> Result<CameraPose, CameraEvaluationError> {
    let roll = match camera_move.free_cam_roll_target() {
        FreeCamRollTarget::Explicit(roll) => roll,
        FreeCamRollTarget::InheritPrevious => previous.inherited_free_roll()?,
    };
    let focus = camera_move.focus().0;
    let orbit_angles = camera_move.orbit_angles();
    match camera_move.destination() {
        CameraMoveDestination::LookAt => {
            let position = camera_move.position();
            CameraPose::try_free(
                position,
                free_camera_look_at(position.0, focus, basis),
                roll,
            )
        },
        CameraMoveDestination::OrbitalLookAt => CameraPose::try_free(
            free_camera_orbit_position(
                focus,
                orbit_angles.yaw,
                orbit_angles.pitch,
                camera_move.radius().0,
                basis,
            ),
            LookAngles {
                yaw:   orbit_angles.yaw,
                pitch: orbit_angles.pitch,
            },
            roll,
        ),
    }
}

/// Resolves the world position of a free-flight orbital endpoint.
pub(super) fn free_camera_orbit_position(
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    radius: f32,
    basis: CameraBasis,
) -> Position {
    let yaw_rotation = Quat::from_rotation_y(yaw);
    let pitch_rotation = Quat::from_rotation_x(-pitch);
    Position(focus + basis.rotation() * yaw_rotation * pitch_rotation * Vec3::new(0.0, 0.0, radius))
}

/// Resolves the free-flight look angles of a world-space look-at endpoint.
pub(super) fn free_camera_look_at(
    translation: Vec3,
    focus: Vec3,
    basis: CameraBasis,
) -> LookAngles {
    let local_offset = basis.rotation().inverse() * (translation - focus);
    let yaw = local_offset.x.atan2(local_offset.z);
    let pitch = local_offset.y.atan2(local_offset.x.hypot(local_offset.z));
    LookAngles { yaw, pitch }
}

/// Returns elapsed sequence time as normalized camera-sequence progress.
pub(super) fn normalized_camera_time(elapsed: SequenceTime, total: SequenceTime) -> f64 {
    if total.is_zero() {
        return 0.0;
    }
    (elapsed.as_seconds_f64() / total.as_seconds_f64()).clamp(0.0, 1.0)
}

/// Interpolates one scalar, rejecting results that do not fit in `f32`.
fn interpolate_scalar(start: f32, endpoint: f32, eased: f64) -> Result<f32, CameraEvaluationError> {
    let value = (f64::from(endpoint) - f64::from(start)).mul_add(eased, f64::from(start));
    let value = value.to_f32();
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CameraEvaluationError::UnrepresentablePose)
    }
}

/// Interpolates one angle along the shortest wrapped path used by retained camera playback.
fn interpolate_angle(start: f32, endpoint: f32, eased: f64) -> Result<f32, CameraEvaluationError> {
    let difference = f64::from(endpoint) - f64::from(start);
    let difference = std::f64::consts::TAU.mul_add(
        -((difference + std::f64::consts::PI) / std::f64::consts::TAU).floor(),
        difference,
    );
    let value = difference.mul_add(eased, f64::from(start));
    let value = value.to_f32();
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CameraEvaluationError::UnrepresentablePose)
    }
}

/// Interpolates a world-space position component-wise.
fn interpolate_vec3(
    start: Vec3,
    endpoint: Vec3,
    eased: f64,
) -> Result<Vec3, CameraEvaluationError> {
    Ok(Vec3::new(
        interpolate_scalar(start.x, endpoint.x, eased)?,
        interpolate_scalar(start.y, endpoint.y, eased)?,
        interpolate_scalar(start.z, endpoint.z, eased)?,
    ))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::Transform;
    use bevy_kana::SequenceEasingError;

    use super::*;
    use crate::animation::sequence::CameraSequence;
    use crate::animation::sequence::playback::*;
    use crate::animation::sequence::support::*;

    #[test]
    fn free_camera_inherited_roll_captures_the_preceding_resolved_endpoint() -> TestResult {
        let sequence = CameraSequence::new(free_move(
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::ZERO,
            FreeCamRollTarget::Explicit(Roll(0.4)),
            Duration::from_secs(1),
        ))
        .then(free_move(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::ZERO,
            FreeCamRollTarget::InheritPrevious,
            Duration::from_secs(1),
        ));
        let mut camera = free_camera(Vec3::ZERO, 0.0, 0.0, 0.1);
        let playback = CameraSequencePlayback::prepare_for_free(
            &sequence,
            &camera,
            CameraBasis::Y_UP,
            FreeFlightControllerInstallation::default(),
        )
        .map_err(|_| "the valid free-camera sequence prepares")?;
        camera.roll.snap_to(Roll(2.0));

        let CameraPose::Free(first_endpoint) = playback.endpoints[0].pose else {
            return Err("free preparation resolves a free-flight endpoint");
        };
        let CameraPose::Free(second_endpoint) = playback.endpoints[1].pose else {
            return Err("free preparation resolves every free-flight endpoint");
        };
        assert_eq!(first_endpoint.roll, Roll(0.4));
        assert_eq!(second_endpoint.roll, Roll(0.4));

        Ok(())
    }

    #[test]
    fn authored_and_suppressed_easing_agree_at_the_same_endpoint() -> TestResult {
        let sequence = CameraSequence::new(orbital_move(
            Vec3::new(8.0, 0.0, 0.0),
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ));
        let playback = prepared_orbit(&sequence);
        let sample = moving_sample(&playback, 0.5)?;
        let authored = evaluate_camera_pose(
            sample,
            |_| SequenceEasingSample::AuthoredEasingApplies { progress: 0.5 },
            |_, progress| EasingSample::Eased(progress),
        )
        .map_err(|_| "authored easing evaluates the prepared camera pose")?;
        let suppressed = evaluate_camera_pose(
            sample,
            |_| SequenceEasingSample::AuthoredEasingSuppressed { eased: 0.5 },
            |_, _| EasingSample::NonFinite,
        )
        .map_err(|_| "suppressed easing evaluates the prepared camera pose")?;

        assert_eq!(authored, suppressed);
        let endpoint = evaluate_camera_pose(
            playback.sample(SequencePosition::END),
            |_| SequenceEasingSample::CurveRejected(SequenceEasingError::NonFiniteOutput),
            |_, _| EasingSample::NonFinite,
        )
        .map_err(|_| "a resting endpoint does not sample easing")?;
        assert_eq!(endpoint, playback.endpoints[0].pose);

        Ok(())
    }

    #[test]
    fn preparation_rejects_a_non_finite_live_camera_pose() {
        let sequence = CameraSequence::new(orbital_move(
            Vec3::ZERO,
            0.0,
            0.0,
            1.0,
            Duration::from_secs(1),
        ));
        let mut camera = free_camera(Vec3::ZERO, 0.0, 0.0, 0.0);
        camera
            .translate
            .set_current(Position(Vec3::splat(f32::NAN)));

        assert!(matches!(
            CameraSequencePlayback::prepare_for_free(
                &sequence,
                &camera,
                CameraBasis::Y_UP,
                FreeFlightControllerInstallation::default(),
            ),
            Err(CameraPlaybackPreparationError::UnrepresentablePose)
        ));
    }

    #[test]
    fn preparation_captures_default_orbit_cam_after_transform_initialization() -> TestResult {
        let mut app = camera_sequence_test_app();
        let camera = app
            .world_mut()
            .spawn((
                OrbitCam::default(),
                Transform::from_translation(Vec3::new(3.0, 4.0, 12.0)),
                CameraSequence::new(orbital_move(
                    Vec3::ZERO,
                    0.0,
                    0.0,
                    1.0,
                    Duration::from_secs(1),
                )),
            ))
            .id();

        app.update();

        let initialized_camera = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("the default orbit camera remains available after initialization")?;
        let initialized_start = CameraSequenceStartPose::try_from(initialized_camera)
            .map_err(|_| "the initialized orbit camera has a finite pose")?;
        let default_start = CameraSequenceStartPose::try_from(&OrbitCam::default())
            .map_err(|_| "the default orbit camera has a finite pose")?;
        let playback = app
            .world()
            .get::<CameraSequencePlayback>(camera)
            .ok_or("the initialized camera sequence prepares inert playback")?;

        assert_eq!(playback.start, initialized_start);
        assert_ne!(playback.start, default_start);

        Ok(())
    }
}
