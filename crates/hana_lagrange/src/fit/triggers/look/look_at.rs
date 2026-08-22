use std::time::Duration;

use bevy::math::curve::easing::EaseFunction;
use bevy::prelude::Camera;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::GlobalTransform;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ReflectFromReflect;
use bevy::prelude::Transform;
use bevy::prelude::With;
use bevy::prelude::warn;

use super::plan::LookAtPlan;
use super::support;
use crate::CameraBasis;
use crate::CameraRequestPreparationError;
use crate::animation::AnimationRejectionReason;
use crate::animation::AnimationSource;
use crate::animation::CameraEvaluationError;
use crate::animation::FreeCamRollTarget;
use crate::fit::camera_pose::FreeCamFitPose;
use crate::fit::triggers::request;
use crate::fit::triggers::request::CameraRequestPart;
use crate::fit::triggers::request::HigherCameraRequestController;
use crate::fit::triggers::request::HigherCameraRequestPreparation;
use crate::free_cam::FreeCam;
use crate::orbit_cam::OrbitCam;

/// Rotates the camera in place to face a target entity.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct LookAt {
    /// The camera entity.
    #[event_target]
    pub camera:   Entity,
    /// The entity to look at.
    pub target:   Entity,
    /// Animation duration (`ZERO` for instant).
    pub duration: Duration,
    /// Easing curve for the animation.
    pub easing:   EaseFunction,
}

impl LookAt {
    /// Creates a new `LookAt` with instant duration and cubic-out easing.
    #[must_use]
    pub const fn new(camera: Entity, target: Entity) -> Self {
        Self {
            camera,
            target,
            duration: Duration::ZERO,
            easing: EaseFunction::CubicOut,
        }
    }

    /// Sets the animation duration.
    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Sets the easing function.
    #[must_use]
    pub const fn easing(mut self, easing: EaseFunction) -> Self {
        self.easing = easing;
        self
    }
}

/// Prepares one `LookAt` through exactly one selected camera controller.
pub(crate) fn on_look_at(
    event: On<LookAt>,
    mut commands: Commands,
    cameras: Query<&Camera>,
    orbit_cameras: Query<(), With<OrbitCam>>,
    free_cameras: Query<&FreeCam>,
    camera_bases: Query<&CameraBasis>,
    transforms: Query<&Transform>,
    global_transforms: Query<&GlobalTransform>,
) {
    let camera = event.camera;
    let target = event.target;
    let preparation = prepare_look_at(
        &event,
        &cameras,
        &orbit_cameras,
        &free_cameras,
        &camera_bases,
        &transforms,
        &global_transforms,
    );
    request::finish_higher_camera_request_preparation(
        &mut commands,
        camera,
        AnimationSource::LookAt,
        target,
        HigherCameraRequestPreparation::from(preparation),
    );
}

fn prepare_look_at(
    event: &LookAt,
    cameras: &Query<&Camera>,
    orbit_cameras: &Query<(), With<OrbitCam>>,
    free_cameras: &Query<&FreeCam>,
    camera_bases: &Query<&CameraBasis>,
    transforms: &Query<&Transform>,
    global_transforms: &Query<&GlobalTransform>,
) -> Result<crate::PlayAnimation, AnimationRejectionReason> {
    let camera = event.camera;
    let target = event.target;
    let duration = event.duration;
    let easing = event.easing;
    cameras.get(camera).map_err(|_| {
        request::request_preparation_rejection(CameraRequestPreparationError::MissingCamera)
    })?;
    let target_transform = global_transforms.get(target).map_err(|_| {
        warn!("LookAt: target {target:?} has no GlobalTransform");
        request::request_preparation_rejection(
            CameraRequestPreparationError::MissingTargetTransform,
        )
    })?;
    let target_position = target_transform.translation();
    let controller = request::higher_camera_request_controller(
        CameraRequestPart::from(orbit_cameras.get(camera)),
        CameraRequestPart::from(free_cameras.get(camera)),
        CameraRequestPart::from(camera_bases.get(camera)),
    )?;
    let (plan, roll_target) = match controller {
        HigherCameraRequestController::Orbit => {
            let camera_transform = global_transforms.get(camera).map_err(|_| {
                AnimationRejectionReason::PreparationFailed(
                    CameraEvaluationError::UnrepresentablePose,
                )
            })?;
            (
                LookAtPlan::from_world_positions(camera_transform.translation(), target_position),
                FreeCamRollTarget::InheritPrevious,
            )
        },
        HigherCameraRequestController::FreeFlight => {
            let free = free_cameras
                .get(camera)
                .map_err(|_| AnimationRejectionReason::NoCameraController)?;
            let basis = camera_bases
                .get(camera)
                .map_err(|_| AnimationRejectionReason::MissingCameraBasis)?;
            let transform = transforms.get(camera).map_err(|_| {
                AnimationRejectionReason::PreparationFailed(
                    CameraEvaluationError::UnrepresentablePose,
                )
            })?;
            let start = FreeCamFitPose::from_free_cam_or_transform(free, transform, *basis);
            (
                LookAtPlan::from_free_camera(start.position.0, target_position, *basis),
                FreeCamRollTarget::Explicit(start.roll),
            )
        },
    };
    let look_move = plan
        .to_look_move(roll_target, duration, easing)
        .map_err(request::invalid_move_rejection)?;
    Ok(support::timed_animation_request(
        camera,
        target,
        AnimationSource::LookAt,
        [look_move],
    ))
}

#[cfg(test)]
mod tests {

    use bevy::prelude::App;
    use bevy::prelude::Vec3;

    use super::*;
    use crate::CurrentFitTarget;
    use crate::animation::AnimationPlugin;
    use crate::animation::CameraMoveDestination;
    use crate::animation::CameraSequence;
    use crate::fit::FitPlugin;
    use crate::operation::Focus;
    use crate::operation::LookAngles;
    use crate::operation::Roll;

    type TestResult = Result<(), &'static str>;

    const EPSILON: f32 = 0.000_001;
    const TEST_CAMERA_POSITION: Vec3 = Vec3::new(0.0, 1.5, 3.0);
    const TEST_DURATION: Duration = Duration::from_secs(1);
    const TEST_FREE_CAMERA_ROLL: Roll = Roll(0.35);
    const TEST_TARGET_POSITION: Vec3 = Vec3::new(3.5, 0.5, 0.0);

    fn assert_f32_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() <= EPSILON);
    }

    fn assert_look_close(actual: LookAngles, expected: LookAngles) {
        assert_f32_close(actual.yaw, expected.yaw);
        assert_f32_close(actual.pitch, expected.pitch);
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((AnimationPlugin, FitPlugin));
        app
    }

    fn spawn_target(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                Transform::from_translation(TEST_TARGET_POSITION),
                GlobalTransform::from(Transform::from_translation(TEST_TARGET_POSITION)),
            ))
            .id()
    }

    fn spawn_free_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                FreeCam::from_pose(
                    TEST_CAMERA_POSITION,
                    LookAngles {
                        yaw:   0.25,
                        pitch: -0.1,
                    },
                    TEST_FREE_CAMERA_ROLL,
                ),
                CameraBasis::Y_UP,
                Transform::from_translation(TEST_CAMERA_POSITION),
                GlobalTransform::from(Transform::from_translation(TEST_CAMERA_POSITION)),
            ))
            .id()
    }

    #[test]
    fn free_cam_look_at_snaps_look_preserving_position_and_roll() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_free_camera(&mut app);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| LookAt::new(camera, target));
        app.update();

        let Some(free_cam) = app.world().get::<FreeCam>(camera) else {
            return Err("FreeCam should still be present after LookAt");
        };
        let expected = LookAtPlan::from_free_camera(
            TEST_CAMERA_POSITION,
            TEST_TARGET_POSITION,
            CameraBasis::Y_UP,
        );
        assert_eq!(free_cam.translate.target().0, TEST_CAMERA_POSITION);
        assert_look_close(free_cam.look.target(), expected.look_angles());
        assert_eq!(free_cam.roll.target(), TEST_FREE_CAMERA_ROLL);
        assert!(app.world().get::<CurrentFitTarget>(camera).is_none());

        Ok(())
    }

    #[test]
    fn free_cam_look_at_retains_timed_look_move() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_free_camera(&mut app);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| LookAt::new(camera, target).duration(TEST_DURATION));
        app.update();

        let Some(sequence) = app.world().get::<CameraSequence>(camera) else {
            return Err("timed FreeCam LookAt should retain a camera sequence");
        };
        let mut moves = sequence.moves().iter();
        let Some(look_move) = moves.next() else {
            return Err("timed FreeCam LookAt should retain a look-at move");
        };
        assert!(moves.next().is_none());
        if look_move.destination() != CameraMoveDestination::LookAt {
            return Err("timed FreeCam LookAt should retain a look-at move");
        }
        assert_eq!(
            look_move.position(),
            crate::operation::Position(TEST_CAMERA_POSITION)
        );
        assert_eq!(look_move.focus(), Focus(TEST_TARGET_POSITION));
        assert_eq!(
            look_move.free_cam_roll_target(),
            FreeCamRollTarget::Explicit(TEST_FREE_CAMERA_ROLL)
        );
        assert_eq!(look_move.duration(), TEST_DURATION);

        Ok(())
    }
}
