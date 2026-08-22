use std::time::Duration;

use bevy::math::curve::easing::EaseFunction;
use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::On;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ReflectFromReflect;
use bevy::prelude::Res;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use bevy::prelude::warn;

use super::plan::LookAtPlan;
use super::support;
use crate::CameraBasis;
use crate::CameraRequestPreparationError;
use crate::animation::AnimationRejectionReason;
use crate::animation::AnimationSource;
use crate::animation::CameraEvaluationError;
use crate::animation::CameraMove;
use crate::animation::CameraMoveError;
use crate::animation::FreeCamRollTarget;
use crate::fit::camera_pose::FreeCamFitPose;
use crate::fit::constants::DEFAULT_FIT_MARGIN;
use crate::fit::constants::LOOK_AT_AND_ZOOM_TO_FIT_CONTEXT;
use crate::fit::constants::LOOK_AT_AND_ZOOM_TO_FIT_LOOK_FRACTION;
use crate::fit::geometry::FitAnchor;
use crate::fit::geometry::FitSolution;
use crate::fit::triggers::request;
use crate::fit::triggers::request::CameraRequestPart;
use crate::fit::triggers::request::FitRequest;
use crate::fit::triggers::request::HigherCameraRequestController;
use crate::fit::triggers::request::HigherCameraRequestPreparation;
use crate::free_cam::FreeCam;
use crate::operation::Focus;
use crate::operation::OrbitAngles;
use crate::operation::Radius;
use crate::orbit_cam::OrbitCam;

/// Rotates the camera to face a target entity and frames it in view.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct LookAtAndZoomToFit {
    /// The camera entity.
    #[event_target]
    pub camera:   Entity,
    /// The entity to frame.
    pub target:   Entity,
    /// Fraction of screen to leave as margin.
    pub margin:   f32,
    /// Animation duration (`ZERO` for instant).
    pub duration: Duration,
    /// Easing curve for the animation.
    pub easing:   EaseFunction,
}

impl LookAtAndZoomToFit {
    /// Creates a new `LookAtAndZoomToFit` with default parameters.
    #[must_use]
    pub const fn new(camera: Entity, target: Entity) -> Self {
        Self {
            camera,
            target,
            margin: DEFAULT_FIT_MARGIN,
            duration: Duration::ZERO,
            easing: EaseFunction::CubicOut,
        }
    }

    /// Sets the margin.
    #[must_use]
    pub const fn margin(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
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

/// Builds the two moves a timed look-then-fit plays: the turn toward the target,
/// then the close on the fitted pose.
fn look_then_fit_moves(
    plan: LookAtPlan,
    fit: FitSolution,
    free_cam_roll_target: FreeCamRollTarget,
    duration: Duration,
    easing: EaseFunction,
) -> Result<[CameraMove; 2], CameraMoveError> {
    let look_duration = duration.mul_f32(LOOK_AT_AND_ZOOM_TO_FIT_LOOK_FRACTION);
    let fit_duration = duration.saturating_sub(look_duration);
    let look_move = plan.to_look_move(free_cam_roll_target, look_duration, easing)?;
    let fit_move = CameraMove::try_to_orbital_look_at(
        Focus(*fit.focus),
        OrbitAngles {
            yaw:   plan.yaw,
            pitch: plan.pitch,
        },
        Radius(fit.radius),
        free_cam_roll_target,
        fit_duration,
        easing,
    )?;
    Ok([look_move, fit_move])
}

/// Prepares one `LookAtAndZoomToFit` through exactly one selected camera controller.
pub(crate) fn on_look_at_and_zoom_to_fit(
    event: On<LookAtAndZoomToFit>,
    mut commands: Commands,
    cameras: Query<&Camera>,
    projections: Query<&Projection>,
    orbit_cameras: Query<(), With<OrbitCam>>,
    free_cameras: Query<&FreeCam>,
    camera_bases: Query<&CameraBasis>,
    transforms: Query<&Transform>,
    mesh_query: Query<&Mesh3d>,
    children_query: Query<&Children>,
    global_transforms: Query<&GlobalTransform>,
    meshes: Res<Assets<Mesh>>,
) {
    let camera = event.camera;
    let target = event.target;
    let preparation = prepare_look_at_and_zoom_to_fit(
        &event,
        &cameras,
        &projections,
        &orbit_cameras,
        &free_cameras,
        &camera_bases,
        &transforms,
        &mesh_query,
        &children_query,
        &global_transforms,
        &meshes,
    );
    request::finish_higher_camera_request_preparation(
        &mut commands,
        camera,
        AnimationSource::LookAtAndZoomToFit,
        target,
        HigherCameraRequestPreparation::from(preparation),
    );
}

fn prepare_look_at_and_zoom_to_fit(
    event: &LookAtAndZoomToFit,
    cameras: &Query<&Camera>,
    projections: &Query<&Projection>,
    orbit_cameras: &Query<(), With<OrbitCam>>,
    free_cameras: &Query<&FreeCam>,
    camera_bases: &Query<&CameraBasis>,
    transforms: &Query<&Transform>,
    mesh_query: &Query<&Mesh3d>,
    children_query: &Query<&Children>,
    global_transforms: &Query<&GlobalTransform>,
    meshes: &Assets<Mesh>,
) -> Result<crate::PlayAnimation, AnimationRejectionReason> {
    let camera = event.camera;
    let target = event.target;
    let margin = event.margin;
    let duration = event.duration;
    let easing = event.easing;
    let camera_component = cameras.get(camera).map_err(|_| {
        request::request_preparation_rejection(CameraRequestPreparationError::MissingCamera)
    })?;
    let projection = projections.get(camera).map_err(|_| {
        request::request_preparation_rejection(CameraRequestPreparationError::MissingProjection)
    })?;
    let target_global_transform = global_transforms.get(target).map_err(|_| {
        warn!("LookAtAndZoomToFit: target {target:?} has no GlobalTransform");
        request::request_preparation_rejection(
            CameraRequestPreparationError::MissingTargetTransform,
        )
    })?;
    let target_position = target_global_transform.translation();
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
    let fit = request::prepare_fit_for_target(
        &FitRequest {
            context: LOOK_AT_AND_ZOOM_TO_FIT_CONTEXT,
            target,
            yaw: plan.yaw,
            pitch: plan.pitch,
            margin,
            anchor: FitAnchor::Center,
            offset_px: Vec2::ZERO,
            projection,
            camera: camera_component,
        },
        mesh_query,
        children_query,
        global_transforms,
        meshes,
    )
    .map_err(request::request_preparation_rejection)?;
    let camera_moves = look_then_fit_moves(plan, fit, roll_target, duration, easing)
        .map_err(request::invalid_move_rejection)?;
    Ok(support::timed_animation_request(
        camera,
        target,
        AnimationSource::LookAtAndZoomToFit,
        camera_moves,
    ))
}

#[cfg(test)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::Cuboid;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Time;
    use bevy::prelude::Vec3;
    use bevy_kana::Displacement;

    use super::*;
    use crate::CurrentFitTarget;
    use crate::animation;
    use crate::animation::AnimationPlugin;
    use crate::animation::CameraMoveDestination;
    use crate::animation::CameraSequence;
    use crate::fit::FitPlugin;
    use crate::fit::ZoomBegin;
    use crate::fit::ZoomEnd;
    use crate::operation::LookAngles;
    use crate::operation::Position;
    use crate::operation::Roll;

    type TestResult = Result<(), &'static str>;

    const EPSILON: f32 = 0.000_001;
    const TEST_CAMERA_POSITION: Vec3 = Vec3::new(0.0, 1.5, 3.0);
    const TEST_DURATION: Duration = Duration::from_secs(1);
    const TEST_FREE_CAMERA_ROLL: Roll = Roll(0.35);
    const TEST_MARGIN: f32 = 0.15;
    const TEST_TARGET_POSITION: Vec3 = Vec3::new(3.5, 0.5, 0.0);

    #[derive(Resource, Default)]
    struct ZoomEventCounts {
        begin: usize,
        end:   usize,
    }

    fn count_zoom_begin(_: On<ZoomBegin>, mut counts: ResMut<ZoomEventCounts>) {
        counts.begin += 1;
    }

    fn count_zoom_end(_: On<ZoomEnd>, mut counts: ResMut<ZoomEventCounts>) { counts.end += 1; }

    fn assert_f32_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() <= EPSILON);
    }

    fn assert_look_close(actual: LookAngles, expected: LookAngles) {
        assert_f32_close(actual.yaw, expected.yaw);
        assert_f32_close(actual.pitch, expected.pitch);
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Time>()
            .init_resource::<ZoomEventCounts>()
            .add_plugins((AnimationPlugin, FitPlugin))
            .add_observer(count_zoom_begin)
            .add_observer(count_zoom_end);
        app
    }

    fn spawn_target(app: &mut App) -> Entity {
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(1.0, 1.0, 1.0));
        app.world_mut()
            .spawn((
                Mesh3d(mesh),
                Transform::from_translation(TEST_TARGET_POSITION),
                GlobalTransform::from(Transform::from_translation(TEST_TARGET_POSITION)),
            ))
            .id()
    }

    fn spawn_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                OrbitCam::default(),
                Projection::Perspective(PerspectiveProjection::default()),
                Camera::default(),
                Transform::from_translation(TEST_CAMERA_POSITION),
                GlobalTransform::from(Transform::from_translation(TEST_CAMERA_POSITION)),
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
                Projection::Perspective(PerspectiveProjection::default()),
                Camera::default(),
                Transform::from_translation(TEST_CAMERA_POSITION),
                GlobalTransform::from(Transform::from_translation(TEST_CAMERA_POSITION)),
            ))
            .id()
    }

    #[test]
    fn look_at_and_zoom_to_fit_retains_look_then_fit_without_zoom_events() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_camera(&mut app);

        app.world_mut().entity_mut(camera).trigger(|_| {
            LookAtAndZoomToFit::new(camera, target)
                .margin(TEST_MARGIN)
                .duration(TEST_DURATION)
        });
        app.update();

        let Some(sequence) = app.world().get::<CameraSequence>(camera) else {
            return Err("LookAtAndZoomToFit should retain a camera sequence");
        };
        let mut moves = sequence.moves().iter();
        let Some(look_move) = moves.next() else {
            return Err("LookAtAndZoomToFit should retain a look-at move");
        };
        let Some(fit_move) = moves.next() else {
            return Err("LookAtAndZoomToFit should retain a fit move");
        };
        assert!(moves.next().is_none());

        if look_move.destination() != CameraMoveDestination::LookAt {
            return Err("first LookAtAndZoomToFit move should look at the target");
        }
        assert_eq!(look_move.position(), Position(TEST_CAMERA_POSITION));
        assert_eq!(look_move.focus(), Focus(TEST_TARGET_POSITION));
        assert_eq!(
            look_move.duration(),
            TEST_DURATION.mul_f32(LOOK_AT_AND_ZOOM_TO_FIT_LOOK_FRACTION)
        );

        let (expected_yaw, expected_pitch, _) = animation::orbital_parameters_from_offset(
            Displacement(TEST_CAMERA_POSITION - TEST_TARGET_POSITION),
        );
        if fit_move.destination() != CameraMoveDestination::OrbitalLookAt {
            return Err("second LookAtAndZoomToFit move should fit from the look direction");
        }
        let fit_angles = fit_move.orbit_angles();
        assert_f32_close(fit_angles.yaw, expected_yaw);
        assert_f32_close(fit_angles.pitch, expected_pitch);
        assert_eq!(
            fit_move.duration(),
            TEST_DURATION
                .saturating_sub(TEST_DURATION.mul_f32(LOOK_AT_AND_ZOOM_TO_FIT_LOOK_FRACTION),)
        );

        let Some(current_target) = app.world().get::<CurrentFitTarget>(camera) else {
            return Err("LookAtAndZoomToFit should update the current fit target");
        };
        assert_eq!(current_target.0, target);

        let counts = app.world().resource::<ZoomEventCounts>();
        assert_eq!(counts.begin, 0);
        assert_eq!(counts.end, 0);

        Ok(())
    }

    #[test]
    fn free_cam_look_at_and_zoom_to_fit_snaps_fit_preserving_roll() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_free_camera(&mut app);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| LookAtAndZoomToFit::new(camera, target).margin(TEST_MARGIN));
        app.update();

        let Some(free_cam) = app.world().get::<FreeCam>(camera) else {
            return Err("FreeCam should still be present after LookAtAndZoomToFit");
        };
        let expected = LookAtPlan::from_free_camera(
            TEST_CAMERA_POSITION,
            TEST_TARGET_POSITION,
            CameraBasis::Y_UP,
        );
        assert_look_close(free_cam.look.target(), expected.look_angles());
        assert_eq!(free_cam.roll.target(), TEST_FREE_CAMERA_ROLL);
        assert_ne!(free_cam.translate.target().0, TEST_CAMERA_POSITION);
        let Some(current_target) = app.world().get::<CurrentFitTarget>(camera) else {
            return Err("LookAtAndZoomToFit should update the current fit target");
        };
        assert_eq!(current_target.0, target);

        Ok(())
    }

    #[test]
    fn free_cam_look_at_and_zoom_to_fit_retains_look_then_fit() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_free_camera(&mut app);

        app.world_mut().entity_mut(camera).trigger(|_| {
            LookAtAndZoomToFit::new(camera, target)
                .margin(TEST_MARGIN)
                .duration(TEST_DURATION)
        });
        app.update();

        let Some(sequence) = app.world().get::<CameraSequence>(camera) else {
            return Err("timed FreeCam LookAtAndZoomToFit should retain a camera sequence");
        };
        let mut moves = sequence.moves().iter();
        let Some(look_move) = moves.next() else {
            return Err("timed FreeCam LookAtAndZoomToFit should retain a look-at move");
        };
        let Some(fit_move) = moves.next() else {
            return Err("timed FreeCam LookAtAndZoomToFit should retain a fit move");
        };
        assert!(moves.next().is_none());

        if look_move.destination() != CameraMoveDestination::LookAt {
            return Err("first FreeCam LookAtAndZoomToFit move should look at the target");
        }
        assert_eq!(look_move.position(), Position(TEST_CAMERA_POSITION));
        assert_eq!(look_move.focus(), Focus(TEST_TARGET_POSITION));
        assert_eq!(
            look_move.free_cam_roll_target(),
            FreeCamRollTarget::Explicit(TEST_FREE_CAMERA_ROLL)
        );
        assert_eq!(
            look_move.duration(),
            TEST_DURATION.mul_f32(LOOK_AT_AND_ZOOM_TO_FIT_LOOK_FRACTION)
        );

        let expected = LookAtPlan::from_free_camera(
            TEST_CAMERA_POSITION,
            TEST_TARGET_POSITION,
            CameraBasis::Y_UP,
        );
        if fit_move.destination() != CameraMoveDestination::OrbitalLookAt {
            return Err(
                "second FreeCam LookAtAndZoomToFit move should fit from the look direction",
            );
        }
        let fit_angles = fit_move.orbit_angles();
        assert_f32_close(fit_angles.yaw, expected.yaw);
        assert_f32_close(fit_angles.pitch, expected.pitch);
        assert_eq!(
            fit_move.free_cam_roll_target(),
            FreeCamRollTarget::Explicit(TEST_FREE_CAMERA_ROLL)
        );
        assert_eq!(
            fit_move.duration(),
            TEST_DURATION
                .saturating_sub(TEST_DURATION.mul_f32(LOOK_AT_AND_ZOOM_TO_FIT_LOOK_FRACTION),)
        );
        let Some(current_target) = app.world().get::<CurrentFitTarget>(camera) else {
            return Err("LookAtAndZoomToFit should update the current fit target");
        };
        assert_eq!(current_target.0, target);

        Ok(())
    }
}
