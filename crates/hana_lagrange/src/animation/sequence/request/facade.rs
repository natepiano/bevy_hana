use bevy::ecs::observer::On;
use bevy::prelude::Commands;
use bevy::prelude::Has;
use bevy::prelude::Query;
use bevy::prelude::ResMut;

use super::super::controller_installation::*;
use super::CameraSequencePreparationRequested;
use super::PendingCameraRequest;
use super::PendingCameraRequests;
use super::effects::PendingCameraAdmissionEffects;
use super::rejection;
use crate::CameraBasis;
use crate::FreeCam;
use crate::OrbitCam;
use crate::animation::events::AnimationRejectionReason;
use crate::animation::events::AnimationSource;
use crate::animation::events::CameraRequestPreparationError;
use crate::animation::events::PlayAnimation;
use crate::animation::lifecycle::AnimationConflictPolicy;
use crate::animation::sequence::CameraSequence;
use crate::animation::sequence::playback::CameraSequencePlayback;

/// Captures the public request after cheap rejection checks but before any
/// retained authoring, controller, fit, or ownership mutation.
pub(in crate::animation) fn capture_play_animation(
    start: On<PlayAnimation>,
    mut commands: Commands,
    controller_components: Query<(Has<OrbitCam>, Has<FreeCam>, Has<CameraBasis>)>,
    conflict_policies: Query<&AnimationConflictPolicy>,
    playbacks: Query<&CameraSequencePlayback>,
    mut pending: ResMut<PendingCameraRequests>,
) {
    let camera = start.camera;
    let source = if start.zoom_context.is_some() {
        AnimationSource::ZoomToFit
    } else {
        start.source
    };
    let target = start.target;
    if start.camera_moves.is_empty() {
        rejection::trigger_animation_rejected(
            &mut commands,
            camera,
            source,
            target,
            AnimationRejectionReason::EmptySequence,
        );
        return;
    }
    let availability = controller_components.get(camera).map_or(
        CameraControllerAvailability::NoController,
        |(orbit, free, basis)| {
            camera_controller_availability(
                orbit.then_some(&OrbitCam::default()),
                free.then_some(&FreeCam::default()),
                basis.then_some(&CameraBasis::Y_UP),
            )
        },
    );
    if let Some(reason) = rejection::rejection_for_controller_availability(availability) {
        rejection::trigger_animation_rejected(&mut commands, camera, source, target, reason);
        return;
    }
    if conflict_policies.get(camera).copied().unwrap_or_default()
        == AnimationConflictPolicy::FirstWins
        && playbacks
            .get(camera)
            .is_ok_and(|playback| playback.playback.is_playing())
    {
        rejection::trigger_animation_rejected(
            &mut commands,
            camera,
            source,
            target,
            AnimationRejectionReason::NativeConflict(AnimationConflictPolicy::FirstWins),
        );
        return;
    }

    let Ok(sequence) = CameraSequence::try_from_moves(start.camera_moves.iter().cloned()) else {
        rejection::trigger_animation_rejected(
            &mut commands,
            camera,
            source,
            target,
            AnimationRejectionReason::EmptySequence,
        );
        return;
    };
    let Ok(effects) = PendingCameraAdmissionEffects::derive(source, target, &sequence) else {
        rejection::trigger_animation_rejected(
            &mut commands,
            camera,
            source,
            target,
            AnimationRejectionReason::RequestPreparationFailed(
                CameraRequestPreparationError::MissingTargetGeometry,
            ),
        );
        return;
    };
    pending.0.push(PendingCameraRequest {
        camera,
        sequence,
        source,
        target,
        zoom: start.zoom_context.clone(),
        effects,
    });
    commands
        .entity(camera)
        .insert(CameraSequencePreparationRequested);
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use bevy::math::curve::easing::EaseFunction;
    use bevy::prelude::Camera;
    use bevy::prelude::Cuboid;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::Projection;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy_kana::SequenceStages;

    use super::*;
    use crate::AnimateToFit;
    use crate::CameraMove;
    use crate::CameraRequestPreparationError;
    use crate::FitAnchor;
    use crate::LookAt;
    use crate::LookAtAndZoomToFit;
    use crate::Radius;
    use crate::ZoomEnd;
    use crate::ZoomReason;
    use crate::ZoomToFit;
    use crate::animation::FreeCamRollTarget;
    use crate::animation::sequence::support::*;
    use crate::fit::ZoomBegin;
    use crate::fit::ZoomContext;
    use crate::operation::Focus;
    use crate::operation::OrbitAngles;

    #[test]
    fn zoom_to_fit_reports_missing_camera_once_without_mutation() {
        let mut app = higher_request_test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_orbit_camera(&mut app);
        app.update();
        let original_controller = *app.world().get::<OrbitCam>(camera).unwrap();
        app.world_mut().entity_mut(camera).remove::<Camera>();

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| ZoomToFit::new(camera, target));
        app.update();

        assert_typed_rejection_without_mutation(
            &mut app,
            camera,
            original_controller,
            AnimationSource::ZoomToFit,
            target,
            CameraRequestPreparationError::MissingCamera,
        );
    }

    #[test]
    fn animate_to_fit_reports_missing_projection_once_without_mutation() {
        let mut app = higher_request_test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_orbit_camera(&mut app);
        app.update();
        let original_controller = *app.world().get::<OrbitCam>(camera).unwrap();
        app.world_mut().entity_mut(camera).remove::<Projection>();

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| AnimateToFit::new(camera, target));
        app.update();

        assert_typed_rejection_without_mutation(
            &mut app,
            camera,
            original_controller,
            AnimationSource::AnimateToFit,
            target,
            CameraRequestPreparationError::MissingProjection,
        );
    }

    #[test]
    fn look_at_and_zoom_to_fit_reports_missing_target_geometry_once_without_mutation() {
        let mut app = higher_request_test_app();
        let target = app
            .world_mut()
            .spawn((Transform::default(), GlobalTransform::default()))
            .id();
        let camera = spawn_orbit_camera(&mut app);
        app.update();
        let original_controller = *app.world().get::<OrbitCam>(camera).unwrap();

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| LookAtAndZoomToFit::new(camera, target));
        app.update();

        assert_typed_rejection_without_mutation(
            &mut app,
            camera,
            original_controller,
            AnimationSource::LookAtAndZoomToFit,
            target,
            CameraRequestPreparationError::MissingTargetGeometry,
        );
    }

    #[test]
    fn look_at_reports_missing_target_transform_once_without_mutation() {
        let mut app = higher_request_test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_orbit_camera(&mut app);
        app.update();
        let original_controller = *app.world().get::<OrbitCam>(camera).unwrap();
        app.world_mut()
            .entity_mut(target)
            .remove::<GlobalTransform>();

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| LookAt::new(camera, target));
        app.update();

        assert_typed_rejection_without_mutation(
            &mut app,
            camera,
            original_controller,
            AnimationSource::LookAt,
            target,
            CameraRequestPreparationError::MissingTargetTransform,
        );
    }

    #[test]
    fn zoom_to_fit_reports_unavailable_viewport_once_without_mutation() {
        let mut app = higher_request_test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_orbit_camera(&mut app);
        app.update();
        let original_controller = *app.world().get::<OrbitCam>(camera).unwrap();

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| ZoomToFit::new(camera, target).offset_px(Vec2::ONE));
        app.update();

        assert_typed_rejection_without_mutation(
            &mut app,
            camera,
            original_controller,
            AnimationSource::ZoomToFit,
            target,
            CameraRequestPreparationError::ViewportUnavailable,
        );
    }

    #[test]
    fn animate_to_fit_reports_points_behind_camera_once_without_mutation() {
        let mut app = higher_request_test_app();
        let target = spawn_target_with_mesh(&mut app, Cuboid::new(0.0, 0.0, 0.0).into());
        let camera = spawn_orbit_camera(&mut app);
        app.update();
        let original_controller = *app.world().get::<OrbitCam>(camera).unwrap();

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| AnimateToFit::new(camera, target));
        app.update();

        assert_typed_rejection_without_mutation(
            &mut app,
            camera,
            original_controller,
            AnimationSource::AnimateToFit,
            target,
            CameraRequestPreparationError::PointsBehindCamera,
        );
    }

    #[test]
    fn look_at_and_zoom_to_fit_reports_unsupported_projection_once_without_mutation() {
        let mut app = higher_request_test_app();
        let target = spawn_target(&mut app);
        let camera = spawn_orbit_camera(&mut app);
        app.update();
        app.world_mut()
            .entity_mut(camera)
            .insert(Projection::custom(UnsupportedTestProjection::default()));
        let original_controller = *app.world().get::<OrbitCam>(camera).unwrap();

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| LookAtAndZoomToFit::new(camera, target));
        app.update();

        assert_typed_rejection_without_mutation(
            &mut app,
            camera,
            original_controller,
            AnimationSource::LookAtAndZoomToFit,
            target,
            CameraRequestPreparationError::UnsupportedProjection,
        );
    }

    #[test]
    fn raw_play_animation_does_not_require_higher_request_camera_components() {
        let mut app = higher_request_test_app();
        let camera = spawn_orbit_camera(&mut app);
        app.update();
        app.world_mut()
            .entity_mut(camera)
            .remove::<(Camera, Projection)>();
        let camera_move = CameraMove::try_to_orbital_look_at(
            Focus(Vec3::ZERO),
            OrbitAngles {
                yaw:   0.5,
                pitch: 0.25,
            },
            Radius(4.0),
            FreeCamRollTarget::InheritPrevious,
            Duration::from_secs(1),
            EaseFunction::Linear,
        )
        .expect("raw authored move should be valid");

        app.world_mut()
            .entity_mut(camera)
            .trigger(|_| PlayAnimation::new(camera, [camera_move]));
        app.update();

        assert!(
            app.world()
                .resource::<HigherRequestEventRecord>()
                .rejections
                .is_empty()
        );
        assert!(app.world().get::<CameraSequence>(camera).is_some());
        assert!(app.world().get::<SequenceStages>(camera).is_some());
        assert!(app.world().get::<CameraSequencePlayback>(camera).is_some());
    }

    #[test]
    fn phase22_play_animation_api_remains_source_compatible() {
        let camera = Entity::from_raw_u32(31).expect("31 is a valid test entity index");
        let target = Entity::from_raw_u32(47).expect("47 is a valid test entity index");
        let zoom_context = ZoomContext {
            target,
            margin: 0.2,
            duration: Duration::from_millis(250),
            easing: EaseFunction::SineInOut,
        };
        let literal = PlayAnimation {
            camera,
            camera_moves: VecDeque::from([move_lasting(Duration::from_secs(2))]),
            source: AnimationSource::LookAt,
            target: Some(target),
            zoom_context: Some(zoom_context.clone()),
        };
        assert_eq!(literal.camera, camera);
        assert_eq!(literal.camera_moves.len(), 1);
        assert_eq!(literal.source, AnimationSource::LookAt);
        assert_eq!(literal.target, Some(target));
        let literal_zoom = literal
            .zoom_context
            .as_ref()
            .expect("the public PlayAnimation zoom context is readable");
        assert_eq!(literal_zoom.target, target);
        assert_approximately_equal(literal_zoom.margin, 0.2);
        assert_eq!(literal_zoom.duration, Duration::from_millis(250));
        assert_eq!(literal_zoom.easing, EaseFunction::SineInOut);

        let built = PlayAnimation::new(camera, [move_lasting(Duration::from_millis(1))])
            .source(AnimationSource::AnimateToFit)
            .target(target)
            .zoom_context(zoom_context);
        assert_eq!(built.camera, camera);
        assert_eq!(built.camera_moves.len(), 1);
        assert_eq!(built.source, AnimationSource::ZoomToFit);
        assert_eq!(built.target, Some(target));
        assert_eq!(
            built
                .zoom_context
                .as_ref()
                .expect("PlayAnimation::zoom_context retains its payload")
                .target,
            target
        );
    }

    #[test]
    fn phase22_fit_facade_api_remains_source_compatible() {
        let camera = Entity::from_raw_u32(31).expect("31 is a valid test entity index");
        let target = Entity::from_raw_u32(47).expect("47 is a valid test entity index");
        let zoom_default = ZoomToFit::new(camera, target);
        assert_eq!(zoom_default.camera, camera);
        assert_eq!(zoom_default.target, target);
        assert_eq!(zoom_default.duration, Duration::ZERO);
        assert_eq!(zoom_default.easing, EaseFunction::CubicOut);
        let zoom_built = ZoomToFit::new(camera, target)
            .margin(0.3)
            .anchor(FitAnchor::Center)
            .offset_px(Vec2::new(12.0, -8.0))
            .duration(Duration::from_secs(2))
            .easing(EaseFunction::Linear);
        assert_approximately_equal(zoom_built.margin, 0.3);
        assert_eq!(zoom_built.anchor, FitAnchor::Center);
        assert_eq!(zoom_built.offset_px, Vec2::new(12.0, -8.0));
        assert_eq!(zoom_built.duration, Duration::from_secs(2));
        assert_eq!(zoom_built.easing, EaseFunction::Linear);

        let animate_default = AnimateToFit::new(camera, target);
        assert_eq!(animate_default.camera, camera);
        assert_eq!(animate_default.target, target);
        assert_eq!(animate_default.duration, Duration::ZERO);
        assert_eq!(animate_default.easing, EaseFunction::CubicOut);
        let animate_built = AnimateToFit::new(camera, target)
            .yaw(0.4)
            .pitch(-0.2)
            .margin(0.3)
            .anchor(FitAnchor::Center)
            .offset_px(Vec2::new(-4.0, 6.0))
            .duration(Duration::from_secs(2))
            .easing(EaseFunction::Linear);
        assert_approximately_equal(animate_built.yaw, 0.4);
        assert_approximately_equal(animate_built.pitch, -0.2);
        assert_approximately_equal(animate_built.margin, 0.3);
        assert_eq!(animate_built.anchor, FitAnchor::Center);
        assert_eq!(animate_built.offset_px, Vec2::new(-4.0, 6.0));
        assert_eq!(animate_built.duration, Duration::from_secs(2));
        assert_eq!(animate_built.easing, EaseFunction::Linear);

        let look_default = LookAt::new(camera, target);
        assert_eq!(look_default.camera, camera);
        assert_eq!(look_default.target, target);
        assert_eq!(look_default.duration, Duration::ZERO);
        assert_eq!(look_default.easing, EaseFunction::CubicOut);
        let look_built = LookAt::new(camera, target)
            .duration(Duration::from_secs(2))
            .easing(EaseFunction::Linear);
        assert_eq!(look_built.duration, Duration::from_secs(2));
        assert_eq!(look_built.easing, EaseFunction::Linear);

        let look_and_fit_default = LookAtAndZoomToFit::new(camera, target);
        assert_eq!(look_and_fit_default.camera, camera);
        assert_eq!(look_and_fit_default.target, target);
        assert_eq!(look_and_fit_default.duration, Duration::ZERO);
        assert_eq!(look_and_fit_default.easing, EaseFunction::CubicOut);
        let look_and_fit_built = LookAtAndZoomToFit::new(camera, target)
            .margin(0.3)
            .duration(Duration::from_secs(2))
            .easing(EaseFunction::Linear);
        assert_approximately_equal(look_and_fit_built.margin, 0.3);
        assert_eq!(look_and_fit_built.duration, Duration::from_secs(2));
        assert_eq!(look_and_fit_built.easing, EaseFunction::Linear);
    }

    #[test]
    fn phase22_zoom_event_api_remains_source_compatible() {
        let camera = Entity::from_raw_u32(31).expect("31 is a valid test entity index");
        let target = Entity::from_raw_u32(47).expect("47 is a valid test entity index");
        let zoom_begin = ZoomBegin {
            camera,
            target,
            margin: 0.2,
            duration: Duration::from_millis(250),
            easing: EaseFunction::SineInOut,
        };
        assert_eq!(zoom_begin.camera, camera);
        assert_eq!(zoom_begin.target, target);
        assert_approximately_equal(zoom_begin.margin, 0.2);
        assert_eq!(zoom_begin.duration, Duration::from_millis(250));
        assert_eq!(zoom_begin.easing, EaseFunction::SineInOut);

        let zoom_end_completed = ZoomEnd {
            camera,
            target,
            margin: 0.2,
            duration: Duration::from_millis(250),
            easing: EaseFunction::SineInOut,
            reason: ZoomReason::Completed,
        };
        assert_eq!(zoom_end_completed.camera, camera);
        assert_eq!(zoom_end_completed.target, target);
        assert_approximately_equal(zoom_end_completed.margin, 0.2);
        assert_eq!(zoom_end_completed.duration, Duration::from_millis(250));
        assert_eq!(zoom_end_completed.easing, EaseFunction::SineInOut);
        assert!(matches!(zoom_end_completed.reason, ZoomReason::Completed));
        let zoom_end_cancelled = ZoomEnd {
            reason: ZoomReason::Cancelled,
            ..zoom_end_completed
        };
        assert!(matches!(zoom_end_cancelled.reason, ZoomReason::Cancelled));
    }
}
