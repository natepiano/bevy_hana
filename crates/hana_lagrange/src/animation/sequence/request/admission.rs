use bevy::ecs::system::SystemParam;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::ParamSet;
use bevy::prelude::Projection;
use bevy::prelude::Query;
use hana_kana::SequenceCommands;
use hana_kana::SequenceDirection;
use hana_kana::SequenceOwner;

use super::super::controller_installation::*;
use super::super::playback::*;
use super::CameraSequencePreparationRequested;
use super::PendingCameraRequest;
use super::RetainedCameraJourney;
use super::effects::CameraAdmissionEffectError;
use super::effects::CameraFitTargetEffect;
use super::effects::CameraProjectionEffectCandidate;
use super::effects::PreparedCameraProjectionEffect;
use super::rejection;
use crate::CameraBasis;
use crate::CurrentFitTarget;
use crate::FreeCam;
use crate::OrbitCam;
use crate::animation::events::AnimationBegin;
use crate::animation::events::AnimationRejectionReason;
use crate::animation::events::CameraEventTiming;
use crate::animation::lifecycle::AnimationConflictPolicy;
use crate::animation::lifecycle::FreeFlightControllerOverrideRestoration;
use crate::animation::lifecycle::OrbitControllerOverrideRestoration;
use crate::animation::sequence::CameraSequence;
use crate::fit::ZoomBegin;

/// The queries final facade admission reads and writes.
#[derive(SystemParam)]
pub(in crate::animation) struct CameraRequestAdmission<'w, 's> {
    sequence_commands:   SequenceCommands<'w, 's>,
    conflict_policies:   Query<'w, 's, &'static AnimationConflictPolicy>,
    retained_cameras: Query<
        'w,
        's,
        (
            &'static CameraSequence,
            &'static mut CameraSequencePlayback,
            &'static RetainedCameraJourney,
        ),
    >,
    orbit_cameras: Query<
        'w,
        's,
        (
            &'static mut OrbitCam,
            Option<&'static OrbitControllerOverrideRestoration>,
        ),
    >,
    free_cameras: Query<
        'w,
        's,
        (
            &'static mut FreeCam,
            Option<&'static FreeFlightControllerOverrideRestoration>,
        ),
    >,
    coordinate_systems:  Query<'w, 's, &'static CameraBasis>,
    orbit_installations: Query<'w, 's, &'static OrbitControllerInstallation>,
    free_installations:  Query<'w, 's, &'static FreeFlightControllerInstallation>,
    existing_entities:   Query<'w, 's, ()>,
    projections: ParamSet<
        'w,
        's,
        (
            Query<'w, 's, &'static Projection>,
            Query<'w, 's, &'static mut Projection>,
        ),
    >,
}

struct PreparedCameraAdmission {
    playback:     CameraSequencePlayback,
    projection:   PreparedCameraProjectionEffect,
    installation: CameraControllerInstallation,
}

impl CameraRequestAdmission<'_, '_> {
    fn conflict_rejection(
        &self,
        request: &PendingCameraRequest,
        accepted_cameras: &[Entity],
    ) -> Option<AnimationRejectionReason> {
        let camera = request.camera;
        if let SequenceOwner::Driver(driver) = self.sequence_commands.owner(camera) {
            return Some(AnimationRejectionReason::DriverOwned { driver });
        }
        let policy = self
            .conflict_policies
            .get(camera)
            .copied()
            .unwrap_or_default();
        let current_active = self
            .retained_cameras
            .get(camera)
            .is_ok_and(|(_, playback, _)| playback.playback.is_playing());
        if (current_active || accepted_cameras.contains(&camera))
            && policy == AnimationConflictPolicy::FirstWins
        {
            return Some(AnimationRejectionReason::NativeConflict(
                AnimationConflictPolicy::FirstWins,
            ));
        }
        None
    }

    fn prepare(
        &mut self,
        request: &PendingCameraRequest,
    ) -> Result<PreparedCameraAdmission, AnimationRejectionReason> {
        let camera = request.camera;
        let availability = camera_controller_availability(
            self.orbit_cameras
                .contains(camera)
                .then_some(&OrbitCam::default()),
            self.free_cameras
                .contains(camera)
                .then_some(&FreeCam::default()),
            self.coordinate_systems
                .contains(camera)
                .then_some(&CameraBasis::Y_UP),
        );
        let installation = camera_controller_installation(
            camera,
            availability,
            &self.coordinate_systems,
            &self.orbit_installations,
            &self.free_installations,
        )
        .map_err(|_| {
            rejection::rejection_for_controller_availability(availability)
                .unwrap_or(AnimationRejectionReason::NoCameraController)
        })?;
        let playback = self.prepare_playback(request, installation)?;
        let projection = self
            .prepare_projection(request, availability)
            .map_err(rejection::camera_admission_effect_rejection)?;
        Ok(PreparedCameraAdmission {
            playback,
            projection,
            installation,
        })
    }

    fn prepare_playback(
        &mut self,
        request: &PendingCameraRequest,
        controller_installation: CameraControllerInstallation,
    ) -> Result<CameraSequencePlayback, AnimationRejectionReason> {
        match controller_installation {
            CameraControllerInstallation::Orbit(installation) => {
                let (orbit, _) = self
                    .orbit_cameras
                    .get_mut(request.camera)
                    .map_err(|_| AnimationRejectionReason::NoCameraController)?;
                CameraSequencePlayback::prepare_for_orbit(&request.sequence, &orbit, installation)
            },
            CameraControllerInstallation::FreeFlight {
                installation,
                basis,
            } => {
                let (free, _) = self
                    .free_cameras
                    .get_mut(request.camera)
                    .map_err(|_| AnimationRejectionReason::NoCameraController)?;
                CameraSequencePlayback::prepare_for_free(
                    &request.sequence,
                    &free,
                    basis,
                    installation,
                )
            },
        }
        .map_err(|_| {
            AnimationRejectionReason::PreparationFailed(CameraEvaluationError::UnrepresentablePose)
        })
    }

    fn prepare_projection(
        &mut self,
        request: &PendingCameraRequest,
        availability: CameraControllerAvailability,
    ) -> Result<PreparedCameraProjectionEffect, CameraAdmissionEffectError> {
        if let CameraFitTargetEffect::Set(target) = request.effects.fit_target
            && !self.existing_entities.contains(target)
        {
            return Err(CameraAdmissionEffectError::MissingTargetEntity);
        }
        match (request.effects.projection, availability) {
            (
                CameraProjectionEffectCandidate::FitFreeFlightFromDestination(radius),
                CameraControllerAvailability::InitializedFreeFlight,
            ) => match self
                .projections
                .p0()
                .get(request.camera)
                .map_err(|_| CameraAdmissionEffectError::MissingProjection)?
            {
                Projection::Orthographic(_) if radius.0.is_finite() && radius.0 > 0.0 => Ok(
                    PreparedCameraProjectionEffect::SetOrthographicScale(radius.0),
                ),
                Projection::Orthographic(_) => {
                    Err(CameraAdmissionEffectError::InvalidOrthographicScale)
                },
                Projection::Perspective(_) | Projection::Custom(_) => {
                    Ok(PreparedCameraProjectionEffect::Preserve)
                },
            },
            (
                CameraProjectionEffectCandidate::Preserve
                | CameraProjectionEffectCandidate::FitFreeFlightFromDestination(_),
                _,
            ) => Ok(PreparedCameraProjectionEffect::Preserve),
        }
    }

    pub(super) fn admit(
        &mut self,
        commands: &mut Commands,
        request: PendingCameraRequest,
        accepted_cameras: &[Entity],
    ) -> bool {
        let camera = request.camera;
        commands
            .entity(camera)
            .remove::<CameraSequencePreparationRequested>();
        if let Some(reason) = self.conflict_rejection(&request, accepted_cameras) {
            rejection::reject_camera_request(commands, &request, reason);
            return false;
        }
        let prepared = match self.prepare(&request) {
            Ok(prepared) => prepared,
            Err(reason) => {
                rejection::reject_camera_request(commands, &request, reason);
                return false;
            },
        };
        let accepted_earlier_in_batch = accepted_cameras.contains(&camera);
        self.commit(commands, request, prepared, accepted_earlier_in_batch)
    }

    fn commit(
        &mut self,
        commands: &mut Commands,
        request: PendingCameraRequest,
        prepared: PreparedCameraAdmission,
        accepted_earlier_in_batch: bool,
    ) -> bool {
        let PreparedCameraAdmission {
            mut playback,
            projection,
            installation,
        } = prepared;
        let camera = request.camera;
        let total = request.sequence.total();
        if let Ok((sequence, mut retained, journey)) = self.retained_cameras.get_mut(camera) {
            emit_cancelled_camera_lifecycle(commands, camera, sequence, &retained, journey);
            retained.lifecycle = CameraPlaybackLifecycleState::Dormant;
        }
        if !accepted_earlier_in_batch {
            match installation {
                CameraControllerInstallation::Orbit(installation) => {
                    let Ok((mut orbit, stash)) = self.orbit_cameras.get_mut(camera) else {
                        return false;
                    };
                    stash_orbit_for_retained_playback(
                        commands,
                        camera,
                        &mut orbit,
                        stash,
                        installation,
                    );
                },
                CameraControllerInstallation::FreeFlight { installation, .. } => {
                    let Ok((mut free, stash)) = self.free_cameras.get_mut(camera) else {
                        return false;
                    };
                    stash_free_for_retained_playback(
                        commands,
                        camera,
                        &mut free,
                        stash,
                        installation,
                    );
                },
            }
        }
        if let CameraFitTargetEffect::Set(target) = request.effects.fit_target {
            commands.entity(camera).insert(CurrentFitTarget(target));
        }
        if let PreparedCameraProjectionEffect::SetOrthographicScale(scale) = projection
            && let Ok(mut camera_projection) = self.projections.p1().get_mut(camera)
            && let Projection::Orthographic(orthographic) = &mut *camera_projection
        {
            orthographic.scale = scale;
        }

        playback.lifecycle = CameraPlaybackLifecycleState::Effective {
            owner: SequenceOwner::NativePlayback,
        };
        commands.entity(camera).insert((
            request.sequence.sequence_stages().clone(),
            RetainedCameraJourney::request(&request.sequence, &request),
            request.sequence,
            playback,
            NativeCameraPlayRequest,
        ));
        if let Some(zoom) = request.zoom.as_ref() {
            commands.trigger(ZoomBegin {
                camera,
                target: zoom.target,
                margin: zoom.margin,
                duration: zoom.duration,
                easing: zoom.easing,
            });
        }
        commands.trigger(AnimationBegin {
            camera,
            source: request.source,
            target: request.target,
            owner: SequenceOwner::NativePlayback,
            direction: SequenceDirection::Forward,
            timing: CameraEventTiming::new(hana_kana::SequencePosition::START, total),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::Camera;
    use bevy::prelude::OrthographicProjection;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec3;
    use hana_kana::SequenceStages;

    use super::*;
    use crate::AnimationSource;
    use crate::CameraBasis;
    use crate::CurrentFitTarget;
    use crate::PlayAnimation;
    use crate::animation::sequence::PendingCameraRequests;
    use crate::animation::sequence::support::*;

    #[test]
    fn rejected_fit_effect_candidate_preserves_prior_target_projection_and_playback_state()
    -> TestResult {
        let mut app = camera_sequence_test_app();
        let prior_target = app.world_mut().spawn_empty().id();
        let requested_target = app.world_mut().spawn_empty().id();
        let camera = app
            .world_mut()
            .spawn((
                free_camera(Vec3::new(0.0, 0.0, 10.0), 0.0, 0.0, 0.0),
                CameraBasis::Y_UP,
                Camera::default(),
                Projection::Orthographic(OrthographicProjection {
                    scale: 3.0,
                    ..OrthographicProjection::default_3d()
                }),
                Transform::from_xyz(0.0, 0.0, 10.0),
                CurrentFitTarget(prior_target),
            ))
            .id();
        app.world_mut().trigger(
            PlayAnimation::new(camera, [move_lasting(Duration::from_secs(1))])
                .source(AnimationSource::AnimateToFit)
                .target(requested_target),
        );
        app.world_mut().despawn(requested_target);

        app.update();

        assert_eq!(
            app.world()
                .get::<CurrentFitTarget>(camera)
                .map(|target| target.0),
            Some(prior_target)
        );
        let projection = orthographic_projection(app.world(), camera)?;
        assert_approximately_equal(projection.scale, 3.0);
        assert!(app.world().get::<CameraSequence>(camera).is_none());
        assert!(app.world().get::<SequenceStages>(camera).is_none());
        assert!(app.world().get::<CameraSequencePlayback>(camera).is_none());
        assert!(app.world().resource::<PendingCameraRequests>().0.is_empty());
        Ok(())
    }

    #[test]
    fn accepted_free_fit_effects_commit_for_timed_and_zero_duration_requests() -> TestResult {
        for duration in [Duration::ZERO, Duration::from_secs(1)] {
            let mut app = camera_sequence_test_app();
            let target = app.world_mut().spawn_empty().id();
            let camera = app
                .world_mut()
                .spawn((
                    free_camera(Vec3::new(0.0, 0.0, 10.0), 0.0, 0.0, 0.0),
                    CameraBasis::Y_UP,
                    Camera::default(),
                    Projection::Orthographic(OrthographicProjection {
                        scale: 3.0,
                        ..OrthographicProjection::default_3d()
                    }),
                    Transform::from_xyz(0.0, 0.0, 10.0),
                ))
                .id();
            app.world_mut().trigger(
                PlayAnimation::new(camera, [move_lasting(duration)])
                    .source(AnimationSource::AnimateToFit)
                    .target(target),
            );

            app.update();

            assert_eq!(
                app.world()
                    .get::<CurrentFitTarget>(camera)
                    .map(|target| target.0),
                Some(target)
            );
            let projection = orthographic_projection(app.world(), camera)?;
            assert_approximately_equal(projection.scale, 4.0);
            assert!(app.world().get::<CameraSequencePlayback>(camera).is_some());
        }
        Ok(())
    }

    #[test]
    fn accepted_orbit_fit_target_commits_for_each_duration() {
        for duration in [Duration::ZERO, Duration::from_secs(1)] {
            let mut app = camera_sequence_test_app();
            let target = app.world_mut().spawn_empty().id();
            let camera = app
                .world_mut()
                .spawn((
                    orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0),
                    Camera::default(),
                    Projection::Orthographic(OrthographicProjection {
                        scale: 3.0,
                        ..OrthographicProjection::default_3d()
                    }),
                    Transform::from_xyz(0.0, 0.0, 10.0),
                ))
                .id();
            app.world_mut().trigger(
                PlayAnimation::new(camera, [move_lasting(duration)])
                    .source(AnimationSource::AnimateToFit)
                    .target(target),
            );

            app.update();

            assert_eq!(
                app.world()
                    .get::<CurrentFitTarget>(camera)
                    .map(|target| target.0),
                Some(target)
            );
            assert!(app.world().get::<CameraSequencePlayback>(camera).is_some());
        }
    }
}
