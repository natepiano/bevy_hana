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
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::debug;

use super::request;
use super::request::CameraRequestPart;
use super::request::FitRequest;
use super::request::HigherCameraRequestController;
use super::request::HigherCameraRequestPreparation;
use crate::CameraBasis;
use crate::CameraRequestPreparationError;
use crate::animation::AnimationRejectionReason;
use crate::animation::AnimationSource;
use crate::animation::CameraEvaluationError;
use crate::animation::CameraMove;
use crate::animation::FreeCamRollTarget;
use crate::animation::PlayAnimation;
use crate::constants::MILLIS_PER_SECOND;
use crate::fit::camera_pose::FreeCamFitPose;
use crate::fit::constants::DEFAULT_FIT_MARGIN;
use crate::fit::constants::ZOOM_TO_FIT_CONTEXT;
use crate::fit::geometry::FitAnchor;
use crate::free_cam::FreeCam;
use crate::operation::Focus;
use crate::operation::OrbitAngles;
use crate::operation::Radius;
use crate::orbit_cam::OrbitCam;

/// Context for a zoom-to-fit operation routed through `PlayAnimation`.
#[derive(Clone, Debug, Reflect)]
pub struct ZoomContext {
    /// The entity being framed.
    pub target:   Entity,
    /// The margin from the triggering `ZoomToFit`.
    pub margin:   f32,
    /// The duration from the triggering `ZoomToFit`.
    pub duration: Duration,
    /// The easing curve from the triggering `ZoomToFit`.
    pub easing:   EaseFunction,
}

/// Frames a target entity in the camera view while preserving the current viewing angle.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct ZoomToFit {
    /// The camera entity to zoom.
    #[event_target]
    pub(crate) camera:    Entity,
    /// The entity to frame.
    pub(crate) target:    Entity,
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

impl ZoomToFit {
    /// Creates a new `ZoomToFit` event with default margin, instant duration, and cubic-out easing.
    #[must_use]
    pub const fn new(camera: Entity, target: Entity) -> Self {
        Self {
            camera,
            target,
            margin: DEFAULT_FIT_MARGIN,
            anchor: FitAnchor::Center,
            offset_px: Vec2::ZERO,
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

/// Emitted when a `ZoomToFit` operation begins.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct ZoomBegin {
    /// The camera that is zooming.
    #[event_target]
    pub camera:   Entity,
    /// The entity being framed.
    pub target:   Entity,
    /// The margin from the triggering `ZoomToFit`.
    pub margin:   f32,
    /// The duration from the triggering `ZoomToFit`.
    pub duration: Duration,
    /// The easing curve from the triggering `ZoomToFit`.
    pub easing:   EaseFunction,
}

/// Emitted when a `ZoomToFit` operation stops, either by completing naturally
/// or by being cancelled. Inspect [`ZoomEnd::reason`] to distinguish.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct ZoomEnd {
    /// The camera that stopped zooming.
    #[event_target]
    pub camera:   Entity,
    /// The entity that was being framed.
    pub target:   Entity,
    /// The margin from the triggering `ZoomToFit`.
    pub margin:   f32,
    /// The duration from the triggering `ZoomToFit`.
    pub duration: Duration,
    /// The easing curve from the triggering `ZoomToFit`.
    pub easing:   EaseFunction,
    /// Why the zoom stopped: completed naturally, or cancelled.
    pub reason:   ZoomReason,
}

/// Why a [`ZoomEnd`] fired.
#[derive(Clone, Copy, Debug, Reflect)]
pub enum ZoomReason {
    /// The zoom-to-fit animation ran to completion.
    Completed,
    /// The zoom-to-fit was interrupted before it could complete.
    Cancelled,
}

/// Prepares one `ZoomToFit` through exactly one selected camera controller.
pub(crate) fn on_zoom_to_fit(
    zoom: On<ZoomToFit>,
    mut commands: Commands,
    cameras: Query<&Camera>,
    projections: Query<&Projection>,
    orbit_cameras: Query<&OrbitCam>,
    free_cameras: Query<&FreeCam>,
    camera_bases: Query<&CameraBasis>,
    transforms: Query<&Transform>,
    mesh_query: Query<&Mesh3d>,
    children_query: Query<&Children>,
    global_transform_query: Query<&GlobalTransform>,
    meshes: Res<Assets<Mesh>>,
) {
    let camera = zoom.camera;
    let target = zoom.target;
    let preparation = prepare_zoom_to_fit(
        &zoom,
        &cameras,
        &projections,
        &orbit_cameras,
        &free_cameras,
        &camera_bases,
        &transforms,
        &mesh_query,
        &children_query,
        &global_transform_query,
        &meshes,
    );
    request::finish_higher_camera_request_preparation(
        &mut commands,
        camera,
        AnimationSource::ZoomToFit,
        target,
        HigherCameraRequestPreparation::from(preparation),
    );
}

struct ZoomToFitOrientation {
    yaw:         f32,
    pitch:       f32,
    roll_target: FreeCamRollTarget,
}

fn zoom_to_fit_orientation(
    camera: Entity,
    duration: Duration,
    controller: HigherCameraRequestController,
    orbit_cameras: &Query<&OrbitCam>,
    free_cameras: &Query<&FreeCam>,
    camera_bases: &Query<&CameraBasis>,
    transforms: &Query<&Transform>,
) -> Result<ZoomToFitOrientation, AnimationRejectionReason> {
    match controller {
        HigherCameraRequestController::Orbit => {
            let orbit = orbit_cameras
                .get(camera)
                .map_err(|_| AnimationRejectionReason::NoCameraController)?;
            debug!(
                "ZoomToFit: yaw={:.3} pitch={:.3} current_focus={:.1?} current_radius={:.1} duration_ms={:.0}",
                orbit.orbit.target().yaw,
                orbit.orbit.target().pitch,
                orbit.pan.target().0,
                orbit.zoom.target().0,
                duration.as_secs_f32() * MILLIS_PER_SECOND,
            );
            Ok(ZoomToFitOrientation {
                yaw:         orbit.orbit.target().yaw,
                pitch:       orbit.orbit.target().pitch,
                roll_target: FreeCamRollTarget::InheritPrevious,
            })
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
            debug!(
                "FreeCam ZoomToFit: yaw={:.3} pitch={:.3} roll={:.3} position={:.1?} duration_ms={:.0}",
                start.look.yaw,
                start.look.pitch,
                start.roll.0,
                start.position.0,
                duration.as_secs_f32() * MILLIS_PER_SECOND,
            );
            Ok(ZoomToFitOrientation {
                yaw:         start.look.yaw,
                pitch:       start.look.pitch,
                roll_target: FreeCamRollTarget::Explicit(start.roll),
            })
        },
    }
}

fn prepare_zoom_to_fit(
    zoom: &ZoomToFit,
    cameras: &Query<&Camera>,
    projections: &Query<&Projection>,
    orbit_cameras: &Query<&OrbitCam>,
    free_cameras: &Query<&FreeCam>,
    camera_bases: &Query<&CameraBasis>,
    transforms: &Query<&Transform>,
    mesh_query: &Query<&Mesh3d>,
    children_query: &Query<&Children>,
    global_transform_query: &Query<&GlobalTransform>,
    meshes: &Assets<Mesh>,
) -> Result<PlayAnimation, AnimationRejectionReason> {
    let camera = zoom.camera;
    let target = zoom.target;
    let margin = zoom.margin;
    let duration = zoom.duration;
    let easing = zoom.easing;
    let anchor = zoom.anchor;
    let offset_px = zoom.offset_px;
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
    let orientation = zoom_to_fit_orientation(
        camera,
        duration,
        controller,
        orbit_cameras,
        free_cameras,
        camera_bases,
        transforms,
    )?;
    let yaw = orientation.yaw;
    let pitch = orientation.pitch;
    let fit = request::prepare_fit_for_target(
        &FitRequest {
            context: ZOOM_TO_FIT_CONTEXT,
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
        orientation.roll_target,
        duration,
        easing,
    )
    .map_err(request::invalid_move_rejection)?;
    let zoom_context = ZoomContext {
        target,
        margin,
        duration,
        easing,
    };
    Ok(PlayAnimation::new(camera, VecDeque::from([fit_move]))
        .zoom_context(zoom_context)
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
    use bevy::prelude::Vec3;

    use super::*;
    use crate::CurrentFitTarget;
    use crate::animation::AnimationBegin;
    use crate::animation::AnimationEnd;
    use crate::animation::AnimationPlugin;
    use crate::animation::AnimationRejected;
    use crate::animation::CameraMoveDestination;
    use crate::animation::CameraSequence;
    use crate::fit::FitPlugin;
    use crate::operation::LookAngles;
    use crate::operation::Position;
    use crate::operation::Roll;

    const CAMERA_POSITION: Vec3 = Vec3::new(0.0, 0.0, 8.0);
    const CAMERA_LOOK: LookAngles = LookAngles {
        yaw:   0.5,
        pitch: -0.25,
    };
    const CAMERA_ROLL: Roll = Roll(0.35);
    /// A roll no move can carry, so the fit solves and only the move is rejected.
    const REJECTED_CAMERA_ROLL: Roll = Roll(f32::NAN);
    const ORTHOGRAPHIC_SCALE: f32 = 3.0;
    const TARGET_POSITION: Vec3 = Vec3::ZERO;
    const TEST_DURATION: Duration = Duration::from_millis(800);
    const EPSILON: f32 = 0.000_001;

    type TestResult = Result<(), &'static str>;

    #[derive(Resource, Default)]
    struct ZoomEventCounts {
        begin: usize,
        end:   usize,
    }

    #[derive(Resource, Default)]
    struct AnimationEventCounts {
        begin: usize,
        end:   usize,
    }

    #[derive(Resource, Default)]
    struct AnimationRejectedCount(usize);

    fn count_zoom_begin(_: On<ZoomBegin>, mut counts: ResMut<ZoomEventCounts>) {
        counts.begin += 1;
    }

    fn count_zoom_end(_: On<ZoomEnd>, mut counts: ResMut<ZoomEventCounts>) { counts.end += 1; }

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
            .init_resource::<ZoomEventCounts>()
            .init_resource::<AnimationEventCounts>()
            .init_resource::<AnimationRejectedCount>()
            .add_observer(count_zoom_begin)
            .add_observer(count_zoom_end)
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

    fn spawn_free_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                FreeCam::from_pose(CAMERA_POSITION, CAMERA_LOOK, CAMERA_ROLL),
                Projection::Perspective(PerspectiveProjection::default()),
                Camera::default(),
                CameraBasis::Y_UP,
                Transform::from_translation(CAMERA_POSITION),
            ))
            .id()
    }

    /// Spawns a free camera whose roll no move can carry, under an orthographic
    /// projection so a resized projection would show up as a changed scale.
    fn spawn_rejecting_free_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                FreeCam::from_pose(CAMERA_POSITION, CAMERA_LOOK, REJECTED_CAMERA_ROLL),
                Projection::Orthographic(OrthographicProjection {
                    scale: ORTHOGRAPHIC_SCALE,
                    ..OrthographicProjection::default_3d()
                }),
                Camera::default(),
                CameraBasis::Y_UP,
                Transform::from_translation(CAMERA_POSITION),
            ))
            .id()
    }

    #[test]
    fn a_rejected_free_cam_zoom_to_fit_leaves_the_camera_untouched() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_rejecting_free_camera(&mut app);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| ZoomToFit::new(camera, target).duration(TEST_DURATION));
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

    #[test]
    fn instant_free_cam_zoom_to_fit_preserves_look_and_roll() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_free_camera(&mut app);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| ZoomToFit::new(camera, target));
        app.update();

        let Some(free_cam) = app.world().get::<FreeCam>(camera) else {
            return Err("FreeCam should still be present after ZoomToFit");
        };
        assert_f32_close(free_cam.look.current().yaw, CAMERA_LOOK.yaw);
        assert_f32_close(free_cam.look.current().pitch, CAMERA_LOOK.pitch);
        assert_f32_close(free_cam.roll.current().0, CAMERA_ROLL.0);
        assert_ne!(free_cam.translate.current(), Position(CAMERA_POSITION));

        let Some(current_target) = app.world().get::<CurrentFitTarget>(camera) else {
            return Err("ZoomToFit should update CurrentFitTarget for FreeCam");
        };
        assert_eq!(current_target.0, target);

        let zoom_counts = app.world().resource::<ZoomEventCounts>();
        assert_eq!(zoom_counts.begin, 1);
        assert_eq!(zoom_counts.end, 1);
        let animation_counts = app.world().resource::<AnimationEventCounts>();
        assert_eq!(animation_counts.begin, 1);
        assert_eq!(animation_counts.end, 1);

        Ok(())
    }

    #[test]
    fn timed_free_cam_zoom_to_fit_starts_free_camera_animation() -> TestResult {
        let mut app = test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_free_camera(&mut app);

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| ZoomToFit::new(camera, target).duration(TEST_DURATION));
        app.update();

        let Some(sequence) = app.world().get::<CameraSequence>(camera) else {
            return Err("timed FreeCam ZoomToFit should retain a camera sequence");
        };
        let Some(fit_move) = sequence.moves().first() else {
            return Err("timed FreeCam ZoomToFit should retain an orbital look-at move");
        };
        if fit_move.destination() != CameraMoveDestination::OrbitalLookAt {
            return Err("timed FreeCam ZoomToFit should retain an orbital look-at move");
        }
        let fit_angles = fit_move.orbit_angles();
        assert_f32_close(fit_angles.yaw, CAMERA_LOOK.yaw);
        assert_f32_close(fit_angles.pitch, CAMERA_LOOK.pitch);
        assert_eq!(
            fit_move.free_cam_roll_target(),
            FreeCamRollTarget::Explicit(CAMERA_ROLL)
        );

        let current_target = app.world().get::<CurrentFitTarget>(camera);
        assert!(current_target.is_some());
        assert_eq!(current_target.map(|target| target.0), Some(target));

        let zoom_counts = app.world().resource::<ZoomEventCounts>();
        assert_eq!(zoom_counts.begin, 1);
        assert_eq!(zoom_counts.end, 0);
        let animation_counts = app.world().resource::<AnimationEventCounts>();
        assert_eq!(animation_counts.begin, 1);
        assert_eq!(animation_counts.end, 0);

        Ok(())
    }
}
