//! View-preserving handover between orbit and free-flight camera controllers.

use bevy::ecs::system::EntityCommands;
use bevy::ecs::world::EntityWorldMut;
use bevy::prelude::Component;
use bevy::prelude::Projection;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;

use crate::CameraBasis;
use crate::CameraHomePending;
use crate::FreeCam;
use crate::FreeCamHomePose;
use crate::FreeCamInput;
use crate::FreeCamInputContext;
use crate::FreeCamInputMode;
use crate::Initialization;
use crate::OrbitCam;
use crate::OrbitCamHomePose;
use crate::OrbitCamInput;
use crate::OrbitCamInputContext;
use crate::OrbitCamInputMode;

/// The orbit controller retained while a camera is in free flight.
///
/// Its limits and behavior policy are restored when the camera returns to
/// orbit mode. The held controller itself is intentionally opaque; use
/// [`orbit_pose_for_view`] to read the orbit pose matching the current view.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub struct OrbitCamHeldForReturn(OrbitCam);

impl OrbitCamHeldForReturn {
    /// Retains an explicit orbit configuration for a free-flight camera that
    /// did not enter free flight through [`CameraControllerHandover`].
    #[must_use]
    pub const fn preserve_for_return(orbit_cam: OrbitCam) -> Self { Self(orbit_cam) }

    /// Applies a settings change that must still be present when the camera
    /// returns to orbit mode.
    pub fn update_for_return(&mut self, update: impl FnOnce(&mut OrbitCam)) { update(&mut self.0); }
}

/// Returns the retained orbit controller aligned to the camera's current view.
///
/// The retained zoom radius and all controller limits are preserved. Roll is
/// intentionally omitted because an orbit camera represents only yaw and
/// pitch.
#[must_use]
pub fn orbit_pose_for_view(
    held: &OrbitCamHeldForReturn,
    transform: &Transform,
    projection: &Projection,
    basis: CameraBasis,
) -> OrbitCam {
    let mut orbit_cam = held.0;
    align_orbit_cam_with_view(&mut orbit_cam, transform, projection, basis);
    orbit_cam
}

/// View-preserving camera-controller handover commands.
///
/// A handover cancels retained camera movement at the camera's current pose.
pub trait CameraControllerHandover {
    /// Replaces an orbit controller with a free-flight controller while
    /// retaining the orbit configuration for a later return.
    fn switch_orbit_cam_to_free_cam(&mut self, input_mode: FreeCamInputMode) -> &mut Self;

    /// Replaces a free-flight controller with the retained orbit controller,
    /// aligned to the current camera view.
    fn switch_free_cam_to_orbit_cam(&mut self, input_mode: OrbitCamInputMode) -> &mut Self;
}

impl CameraControllerHandover for EntityCommands<'_> {
    fn switch_orbit_cam_to_free_cam(&mut self, input_mode: FreeCamInputMode) -> &mut Self {
        self.queue(move |mut entity: EntityWorldMut| {
            if entity.contains::<FreeCam>() {
                return;
            }
            let Some(orbit_cam) = entity.get::<OrbitCam>().copied() else {
                return;
            };
            let basis = entity
                .get::<CameraBasis>()
                .copied()
                .unwrap_or(CameraBasis::Y_UP);
            let free_home = entity
                .get::<OrbitCamHomePose>()
                .copied()
                .zip(entity.get::<Projection>())
                .map(|(home, projection)| {
                    FreeCamHomePose::from_orbit_home(home, basis, projection)
                });
            // A controller switch interrupts retained movement at the camera's current pose.
            entity.remove::<crate::CameraSequence>();
            entity
                .remove::<(
                    OrbitCam,
                    OrbitCamInput,
                    OrbitCamInputContext,
                    OrbitCamInputMode,
                    CameraHomePending,
                )>()
                .insert((
                    OrbitCamHeldForReturn(orbit_cam),
                    FreeCam::default(),
                    input_mode,
                ));
            if let Some(home) = free_home {
                entity.insert(home);
            }
        })
    }

    fn switch_free_cam_to_orbit_cam(&mut self, input_mode: OrbitCamInputMode) -> &mut Self {
        self.queue(move |mut entity: EntityWorldMut| {
            if entity.contains::<OrbitCam>() || !entity.contains::<FreeCam>() {
                return;
            }
            let (Some(transform), Some(projection)) = (
                entity.get::<Transform>().copied(),
                entity.get::<Projection>().cloned(),
            ) else {
                return;
            };
            let held = entity
                .get::<OrbitCamHeldForReturn>()
                .copied()
                .unwrap_or_else(|| OrbitCamHeldForReturn(OrbitCam::default()));
            let basis = entity
                .get::<CameraBasis>()
                .copied()
                .unwrap_or(CameraBasis::Y_UP);
            let orbit_cam = orbit_pose_for_view(&held, &transform, &projection, basis);
            // A controller switch interrupts retained movement at the camera's current pose.
            entity.remove::<crate::CameraSequence>();
            entity
                .remove::<(
                    FreeCam,
                    FreeCamInput,
                    FreeCamInputContext,
                    FreeCamInputMode,
                    CameraHomePending,
                    OrbitCamHeldForReturn,
                )>()
                .insert((orbit_cam, input_mode));
        })
    }
}

fn align_orbit_cam_with_view(
    orbit_cam: &mut OrbitCam,
    transform: &Transform,
    projection: &Projection,
    basis: CameraBasis,
) {
    let back = transform.rotation * Vec3::Z;
    let radius = orbit_cam.zoom.current().0;
    let distance = match projection {
        Projection::Orthographic(orthographic) => {
            f32::midpoint(orthographic.near, orthographic.far)
        },
        Projection::Perspective(_) | Projection::Custom(_) => radius,
    };
    orbit_cam
        .pan
        .snap_to(transform.translation - back * distance);
    orbit_cam.orbit.snap_to((
        back.dot(basis.right).atan2(back.dot(basis.back)),
        back.dot(basis.up).clamp(-1.0, 1.0).asin(),
    ));
    orbit_cam.zoom.snap_to(radius);
    orbit_cam.initialization = Initialization::Active;
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::curve::easing::EaseFunction;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Camera;
    use bevy::prelude::Commands;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::Quat;
    use bevy::prelude::Vec3;

    use super::*;
    use crate::AnglePairLimit;
    use crate::CameraMove;
    use crate::CameraSequence;
    use crate::Focus;
    use crate::FreeCamPreset;
    use crate::FreeCamRollTarget;
    use crate::LagrangePlugin;
    use crate::OrbitAngles;
    use crate::OrbitCamPreset;
    use crate::PlayAnimation;
    use crate::Radius;
    use crate::ScalarLimit;
    use crate::UpsideDownPolicy;

    const TRANSFORM_TOLERANCE: f32 = 1.0e-4;

    fn test_app() -> App {
        let mut app = crate::test_app();
        app.add_plugins((MinimalPlugins, LagrangePlugin));
        app.finish();
        app
    }

    fn spawn_orbit_camera(app: &mut App, orbit_cam: OrbitCam) -> Entity {
        app.world_mut()
            .spawn((
                orbit_cam,
                Camera::default(),
                Projection::Perspective(PerspectiveProjection::default()),
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
            ))
            .id()
    }

    fn switch_to_free(app: &mut App, camera: Entity) {
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.entity(camera).switch_orbit_cam_to_free_cam(
                    FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse()),
                );
            })
            .expect("handover system should run");
    }

    fn switch_to_orbit(app: &mut App, camera: Entity) {
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.entity(camera).switch_free_cam_to_orbit_cam(
                    OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                );
            })
            .expect("handover system should run");
    }

    fn assert_transform_close(actual: &Transform, expected: &Transform) {
        assert!(
            actual.translation.distance(expected.translation) < TRANSFORM_TOLERANCE,
            "translation changed: actual={:?}, expected={:?}",
            actual.translation,
            expected.translation
        );
        assert!(
            actual.rotation.dot(expected.rotation).abs() > 1.0 - TRANSFORM_TOLERANCE,
            "rotation changed: actual={:?}, expected={:?}",
            actual.rotation,
            expected.rotation
        );
    }

    #[test]
    fn orbit_free_orbit_round_trip_preserves_view_and_orbit_policy() {
        let mut app = test_app();
        let mut initial_orbit = OrbitCam::from_pose(Vec3::new(0.5, 0.25, -0.5), (0.2, -0.1), 4.0);
        *initial_orbit.orbit.limit_mut() = AnglePairLimit {
            yaw:   ScalarLimit::None,
            pitch: ScalarLimit::Clamp {
                min: -0.75,
                max: 0.625,
            },
        };
        *initial_orbit.zoom.limit_mut() = ScalarLimit::Clamp {
            min: 0.125,
            max: 80.0,
        };
        initial_orbit.upside_down_policy = UpsideDownPolicy::Allow;
        let camera = spawn_orbit_camera(&mut app, initial_orbit);
        app.update();
        let initial_transform = *app.world().get::<Transform>(camera).unwrap();
        let initialized_orbit = *app.world().get::<OrbitCam>(camera).unwrap();

        switch_to_free(&mut app, camera);
        assert!(app.world().get::<OrbitCam>(camera).is_none());
        assert!(app.world().get::<FreeCam>(camera).is_some());
        assert_transform_close(
            app.world().get::<Transform>(camera).unwrap(),
            &initial_transform,
        );

        switch_to_orbit(&mut app, camera);
        let restored = app.world().get::<OrbitCam>(camera).unwrap();
        assert!(app.world().get::<FreeCam>(camera).is_none());
        assert_transform_close(
            app.world().get::<Transform>(camera).unwrap(),
            &initial_transform,
        );
        assert_eq!(restored.orbit.limit(), initialized_orbit.orbit.limit());
        assert_eq!(restored.zoom.limit(), initialized_orbit.zoom.limit());
        assert_eq!(
            restored.upside_down_policy,
            initialized_orbit.upside_down_policy
        );
    }

    #[test]
    fn repeated_handover_to_current_mode_is_a_no_op() {
        let mut app = test_app();
        let camera = spawn_orbit_camera(&mut app, OrbitCam::default());
        app.update();

        switch_to_free(&mut app, camera);
        let first_free = *app.world().get::<FreeCam>(camera).unwrap();
        let first_held = *app.world().get::<OrbitCamHeldForReturn>(camera).unwrap();
        let first_mode = app.world().get::<FreeCamInputMode>(camera).unwrap().clone();
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands
                    .entity(camera)
                    .switch_orbit_cam_to_free_cam(FreeCamInputMode::Manual);
            })
            .expect("second free handover system should run");
        assert_eq!(app.world().get::<FreeCam>(camera), Some(&first_free));
        assert_eq!(
            app.world().get::<OrbitCamHeldForReturn>(camera),
            Some(&first_held)
        );
        assert_eq!(
            app.world().get::<FreeCamInputMode>(camera),
            Some(&first_mode)
        );

        switch_to_orbit(&mut app, camera);
        let first_orbit = *app.world().get::<OrbitCam>(camera).unwrap();
        let first_mode = app
            .world()
            .get::<OrbitCamInputMode>(camera)
            .unwrap()
            .clone();
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands
                    .entity(camera)
                    .switch_free_cam_to_orbit_cam(OrbitCamInputMode::Manual);
            })
            .expect("second orbit handover system should run");
        assert_eq!(app.world().get::<OrbitCam>(camera), Some(&first_orbit));
        assert_eq!(
            app.world().get::<OrbitCamInputMode>(camera),
            Some(&first_mode)
        );
    }

    #[test]
    fn handover_to_orbit_without_free_camera_is_a_no_op() {
        let mut app = test_app();
        let held = OrbitCamHeldForReturn::preserve_for_return(OrbitCam::default());
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                Projection::Perspective(PerspectiveProjection::default()),
                held,
            ))
            .id();

        switch_to_orbit(&mut app, camera);

        assert!(app.world().get::<OrbitCam>(camera).is_none());
        assert_eq!(
            app.world().get::<OrbitCamHeldForReturn>(camera),
            Some(&held)
        );
    }

    #[test]
    fn animation_after_handover_starts_from_free_camera_view() {
        let mut app = test_app();
        let camera = spawn_orbit_camera(&mut app, OrbitCam::from_pose(Vec3::ZERO, (0.0, 0.0), 6.0));
        app.update();
        switch_to_free(&mut app, camera);

        let free_view = Transform {
            translation: Vec3::new(7.0, 3.0, 5.0),
            rotation: Quat::from_rotation_y(0.65) * Quat::from_rotation_x(-0.2),
            ..Transform::default()
        };
        *app.world_mut().get_mut::<Transform>(camera).unwrap() = free_view;
        let expected_start = orbit_pose_for_view(
            app.world().get::<OrbitCamHeldForReturn>(camera).unwrap(),
            &free_view,
            app.world().get::<Projection>(camera).unwrap(),
            CameraBasis::Y_UP,
        );
        let movement = CameraMove::try_to_orbital_look_at(
            Focus(Vec3::new(-2.0, 1.0, 4.0)),
            OrbitAngles {
                yaw:   -0.4,
                pitch: 0.3,
            },
            Radius(3.0),
            FreeCamRollTarget::InheritPrevious,
            Duration::from_hours(24),
            EaseFunction::Linear,
        )
        .expect("animation movement should be valid");

        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                commands.entity(camera).switch_free_cam_to_orbit_cam(
                    OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                );
                commands.trigger(PlayAnimation::new(camera, [movement.clone()]));
            })
            .expect("handover and animation system should run");
        app.update();

        assert!(app.world().get::<CameraSequence>(camera).is_some());
        let animated = app.world().get::<OrbitCam>(camera).unwrap();
        assert!(
            animated
                .pan
                .current()
                .0
                .distance(expected_start.pan.current().0)
                < 1.0e-4
        );
        assert!((animated.orbit.current().yaw - expected_start.orbit.current().yaw).abs() < 1.0e-4);
        assert!(
            (animated.orbit.current().pitch - expected_start.orbit.current().pitch).abs() < 1.0e-4
        );
        assert!((animated.zoom.current().0 - expected_start.zoom.current().0).abs() < 1.0e-4);
        assert_transform_close(app.world().get::<Transform>(camera).unwrap(), &free_view);
    }
}
