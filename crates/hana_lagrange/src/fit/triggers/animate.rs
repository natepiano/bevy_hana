use std::collections::VecDeque;
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
use bevy::prelude::Vec2;
use bevy::prelude::With;

use super::request;
use super::request::CameraRequestPart;
use super::request::FitRequest;
use super::request::HigherCameraRequestController;
use super::request::HigherCameraRequestPreparation;
use crate::CameraBasis;
use crate::CameraRequestPreparationError;
use crate::animation::AnimationRejectionReason;
use crate::animation::AnimationSource;
use crate::animation::CameraMove;
use crate::animation::FreeCamRollTarget;
use crate::animation::PlayAnimation;
use crate::fit::constants::ANIMATE_TO_FIT_CONTEXT;
use crate::fit::constants::DEFAULT_ANIMATE_TO_FIT_PITCH;
use crate::fit::constants::DEFAULT_ANIMATE_TO_FIT_YAW;
use crate::fit::constants::DEFAULT_FIT_MARGIN;
use crate::fit::geometry::FitAnchor;
use crate::free_cam::FreeCam;
use crate::operation::Focus;
use crate::operation::OrbitAngles;
use crate::operation::Radius;
use crate::operation::Roll;
use crate::orbit_cam::OrbitCam;

/// Animates the camera to a caller-specified orientation while framing a target entity.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct AnimateToFit {
    /// The camera entity.
    #[event_target]
    pub(crate) camera:    Entity,
    /// The entity to frame.
    pub(crate) target:    Entity,
    /// Final yaw in radians.
    pub(crate) yaw:       f32,
    /// Final pitch in radians.
    pub(crate) pitch:     f32,
    /// Fraction of screen to leave as margin.
    pub(crate) margin:    f32,
    /// Screen-space anchor used after the target has been fitted.
    pub(crate) anchor:    FitAnchor,
    /// Pixel offset from the selected anchor, using positive x right and positive y down.
    pub(crate) offset_px: Vec2,
    /// Animation duration (`ZERO` for instant).
    pub(crate) duration:  Duration,
    /// Easing curve for the animation.
    pub(crate) easing:    EaseFunction,
}

impl AnimateToFit {
    /// Creates a new `AnimateToFit` with default parameters.
    #[must_use]
    pub const fn new(camera: Entity, target: Entity) -> Self {
        Self {
            camera,
            target,
            yaw: DEFAULT_ANIMATE_TO_FIT_YAW,
            pitch: DEFAULT_ANIMATE_TO_FIT_PITCH,
            margin: DEFAULT_FIT_MARGIN,
            anchor: FitAnchor::Center,
            offset_px: Vec2::ZERO,
            duration: Duration::ZERO,
            easing: EaseFunction::CubicOut,
        }
    }

    /// Sets the target yaw.
    #[must_use]
    pub const fn yaw(mut self, yaw: f32) -> Self {
        self.yaw = yaw;
        self
    }

    /// Sets the target pitch.
    #[must_use]
    pub const fn pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }

    /// Sets the margin.
    #[must_use]
    pub const fn margin(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
    }

    /// Sets which fitted bounds point should land on the matching viewport point.
    #[must_use]
    pub const fn anchor(mut self, anchor: FitAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Sets a pixel offset from the selected anchor.
    ///
    /// Positive x moves the fitted bounds right. Positive y moves them down,
    /// matching Bevy's screen-space coordinate convention.
    #[must_use]
    pub const fn offset_px(mut self, offset_px: Vec2) -> Self {
        self.offset_px = offset_px;
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

/// Prepares one `AnimateToFit` through exactly one selected camera controller.
pub(crate) fn on_animate_to_fit(
    event: On<AnimateToFit>,
    mut commands: Commands,
    cameras: Query<&Camera>,
    projections: Query<&Projection>,
    orbit_cameras: Query<(), With<OrbitCam>>,
    free_cameras: Query<(), With<FreeCam>>,
    camera_bases: Query<(), With<CameraBasis>>,
    mesh_query: Query<&Mesh3d>,
    children_query: Query<&Children>,
    global_transform_query: Query<&GlobalTransform>,
    meshes: Res<Assets<Mesh>>,
) {
    let camera = event.camera;
    let target = event.target;
    let preparation = prepare_animate_to_fit(
        &event,
        &cameras,
        &projections,
        &orbit_cameras,
        &free_cameras,
        &camera_bases,
        &mesh_query,
        &children_query,
        &global_transform_query,
        &meshes,
    );
    request::finish_higher_camera_request_preparation(
        &mut commands,
        camera,
        AnimationSource::AnimateToFit,
        target,
        HigherCameraRequestPreparation::from(preparation),
    );
}

fn prepare_animate_to_fit(
    event: &AnimateToFit,
    cameras: &Query<&Camera>,
    projections: &Query<&Projection>,
    orbit_cameras: &Query<(), With<OrbitCam>>,
    free_cameras: &Query<(), With<FreeCam>>,
    camera_bases: &Query<(), With<CameraBasis>>,
    mesh_query: &Query<&Mesh3d>,
    children_query: &Query<&Children>,
    global_transform_query: &Query<&GlobalTransform>,
    meshes: &Assets<Mesh>,
) -> Result<PlayAnimation, AnimationRejectionReason> {
    let camera = event.camera;
    let target = event.target;
    let yaw = event.yaw;
    let pitch = event.pitch;
    let margin = event.margin;
    let duration = event.duration;
    let easing = event.easing;
    let anchor = event.anchor;
    let offset_px = event.offset_px;
    let camera_component = cameras.get(camera).map_err(|_| {
        request::request_preparation_rejection(CameraRequestPreparationError::MissingCamera)
    })?;
    let projection = projections.get(camera).map_err(|_| {
        request::request_preparation_rejection(CameraRequestPreparationError::MissingProjection)
    })?;
    let controller = request::higher_camera_request_controller(
        CameraRequestPart::from(orbit_cameras.get(camera)),
        CameraRequestPart::from(free_cameras.get(camera)),
        CameraRequestPart::from(camera_bases.get(camera)),
    )?;
    let roll_target = match controller {
        HigherCameraRequestController::Orbit => FreeCamRollTarget::InheritPrevious,
        HigherCameraRequestController::FreeFlight => FreeCamRollTarget::Explicit(Roll::default()),
    };
    let fit = request::prepare_fit_for_target(
        &FitRequest {
            context: ANIMATE_TO_FIT_CONTEXT,
            target,
            yaw,
            pitch,
            margin,
            anchor,
            offset_px,
            projection,
            camera: camera_component,
        },
        mesh_query,
        children_query,
        global_transform_query,
        meshes,
    )
    .map_err(request::request_preparation_rejection)?;
    let fit_move = CameraMove::try_to_orbital_look_at(
        Focus(*fit.focus),
        OrbitAngles { yaw, pitch },
        Radius(fit.radius),
        roll_target,
        duration,
        easing,
    )
    .map_err(request::invalid_move_rejection)?;
    Ok(PlayAnimation::new(camera, VecDeque::from([fit_move]))
        .source(AnimationSource::AnimateToFit)
        .target(target))
}

#[cfg(test)]
mod tests {

    use bevy::prelude::App;
    use bevy::prelude::Cuboid;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::OrthographicProjection;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec3;

    use super::*;
    use crate::AnimationBegin;
    use crate::AnimationEnd;
    use crate::CurrentFitTarget;
    use crate::Initialization;
    use crate::LookAngles;
    use crate::Position;
    use crate::animation::AnimationPlugin;
    use crate::animation::AnimationRejected;
    use crate::animation::CameraMoveDestination;
    use crate::animation::CameraSequence;
    use crate::fit::FitPlugin;

    const CAMERA_POSITION: Vec3 = Vec3::new(0.0, 0.0, 8.0);
    const FIT_PITCH: f32 = 0.25;
    const FIT_YAW: f32 = 0.5;
    const TEST_DURATION: Duration = Duration::from_millis(800);
    const TARGET_POSITION: Vec3 = Vec3::ZERO;
    const ORTHOGRAPHIC_SCALE: f32 = 3.0;
    const EPSILON: f32 = 0.000_001;

    type TestResult = Result<(), &'static str>;

    #[derive(Resource, Default)]
    struct AnimationEventCounts {
        begin: usize,
        end:   usize,
    }

    #[derive(Resource, Default)]
    struct AnimationRejectedCount(usize);

    fn count_animation_begin(_: On<AnimationBegin>, mut counts: ResMut<AnimationEventCounts>) {
        counts.begin += 1;
    }

    fn count_animation_end(_: On<AnimationEnd>, mut counts: ResMut<AnimationEventCounts>) {
        counts.end += 1;
    }

    fn count_animation_rejected(
        _: On<AnimationRejected>,
        mut count: ResMut<AnimationRejectedCount>,
    ) {
        count.0 += 1;
    }

    fn assert_f32_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() <= EPSILON);
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<AnimationEventCounts>()
            .init_resource::<AnimationRejectedCount>()
            .add_plugins((AnimationPlugin, FitPlugin))
            .add_observer(count_animation_begin)
            .add_observer(count_animation_end)
            .add_observer(count_animation_rejected);
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
                Transform::from_translation(TARGET_POSITION),
                GlobalTransform::from(Transform::from_translation(TARGET_POSITION)),
            ))
            .id()
    }

    /// Spawns an orbit camera under an orthographic projection, which projects
    /// every point regardless of depth, so a fit still solves for the
    /// non-finite yaw the rejected move is authored from.
    fn spawn_orbit_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                OrbitCam::from_pose(
                    Focus(TARGET_POSITION),
                    OrbitAngles {
                        yaw:   FIT_YAW,
                        pitch: FIT_PITCH,
                    },
                    Radius(CAMERA_POSITION.z),
                ),
                Projection::Orthographic(OrthographicProjection {
                    scale: ORTHOGRAPHIC_SCALE,
                    ..OrthographicProjection::default_3d()
                }),
                Camera::default(),
                Transform::from_translation(CAMERA_POSITION),
            ))
            .id()
    }

    fn spawn_free_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                FreeCam::from_pose(
                    Position(CAMERA_POSITION),
                    LookAngles::default(),
                    Roll::default(),
                ),
                Projection::Perspective(PerspectiveProjection::default()),
                Camera::default(),
                CameraBasis::Y_UP,
                Transform::from_translation(CAMERA_POSITION),
            ))
            .id()
    }

    #[test]
    fn instant_free_cam_animate_to_fit_writes_free_pose() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_free_camera(&mut app);

        app.world_mut().entity_mut(camera).trigger(|_| {
            AnimateToFit::new(camera, target)
                .yaw(FIT_YAW)
                .pitch(FIT_PITCH)
        });
        app.update();

        let Some(free_cam) = app.world().get::<FreeCam>(camera) else {
            return Err("FreeCam should still be present after AnimateToFit");
        };
        assert_f32_close(free_cam.look.current().yaw, FIT_YAW);
        assert_f32_close(free_cam.look.current().pitch, FIT_PITCH);
        assert_f32_close(free_cam.roll.current().0, 0.0);
        assert_ne!(free_cam.translate.current(), Position(CAMERA_POSITION));
        assert_eq!(free_cam.initialization, Initialization::Active);

        let Some(current_target) = app.world().get::<CurrentFitTarget>(camera) else {
            return Err("AnimateToFit should update CurrentFitTarget for FreeCam");
        };
        assert_eq!(current_target.0, target);

        let counts = app.world().resource::<AnimationEventCounts>();
        assert_eq!(counts.begin, 1);
        assert_eq!(counts.end, 1);

        Ok(())
    }

    #[test]
    fn timed_free_cam_animate_to_fit_starts_free_camera_animation() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_free_camera(&mut app);

        app.world_mut().entity_mut(camera).trigger(|_| {
            AnimateToFit::new(camera, target)
                .yaw(FIT_YAW)
                .pitch(FIT_PITCH)
                .duration(TEST_DURATION)
        });
        app.update();

        let Some(sequence) = app.world().get::<CameraSequence>(camera) else {
            return Err("timed FreeCam AnimateToFit should retain a camera sequence");
        };
        let Some(fit_move) = sequence.moves().first() else {
            return Err("timed FreeCam AnimateToFit should retain an orbital look-at move");
        };
        if fit_move.destination() != CameraMoveDestination::OrbitalLookAt {
            return Err("timed FreeCam AnimateToFit should retain an orbital look-at move");
        }
        let fit_angles = fit_move.orbit_angles();
        assert_f32_close(fit_angles.yaw, FIT_YAW);
        assert_f32_close(fit_angles.pitch, FIT_PITCH);
        assert_eq!(
            fit_move.free_cam_roll_target(),
            FreeCamRollTarget::Explicit(Roll::default())
        );

        let counts = app.world().resource::<AnimationEventCounts>();
        assert_eq!(counts.begin, 1);
        assert_eq!(counts.end, 0);

        Ok(())
    }

    #[test]
    fn a_rejected_orbit_cam_animate_to_fit_leaves_the_camera_untouched() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_orbit_camera(&mut app);

        app.world_mut().entity_mut(camera).trigger(|_| {
            AnimateToFit::new(camera, target)
                .yaw(f32::NAN)
                .pitch(FIT_PITCH)
                .duration(TEST_DURATION)
        });
        app.update();

        assert_eq!(app.world().resource::<AnimationRejectedCount>().0, 1);
        assert!(app.world().get::<CurrentFitTarget>(camera).is_none());
        let Some(Projection::Orthographic(projection)) = app.world().get::<Projection>(camera)
        else {
            return Err("a rejected move leaves the orthographic projection in place");
        };
        assert_f32_close(projection.scale, ORTHOGRAPHIC_SCALE);

        Ok(())
    }
}
