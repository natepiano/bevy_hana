use bevy::ecs::observer::On;
use bevy::prelude::Commands;
use bevy::prelude::Discard;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::warn;
use hana_kana::SequenceStages;

use super::super::controller_installation::*;
use super::super::playback::*;
use super::RetainedCameraJourney;
use crate::CameraBasis;
use crate::FreeCam;
use crate::OrbitCam;
use crate::animation::lifecycle::FreeFlightControllerOverrideRestoration;
use crate::animation::lifecycle::OrbitControllerOverrideRestoration;
use crate::animation::sequence::CameraSequence;

/// Publishes direct retained authoring early enough for driver arbitration.
/// A camera whose prepared playback already matches the sequence's current
/// revision is skipped without a write, so the steady path is one check per
/// camera. Rechecking the controller tuple on every pass also lets authoring
/// recover once a compatible controller or camera basis arrives.
pub(in crate::animation) fn prepare_direct_camera_sequences(
    mut commands: Commands,
    mut sequences: Query<(
        Entity,
        &CameraSequence,
        Option<&mut CameraSequencePlayback>,
        Option<&RetainedCameraJourney>,
        Option<&SequenceStages>,
    )>,
    orbit_cameras: Query<&OrbitCam>,
    free_cameras: Query<&FreeCam>,
    coordinate_systems: Query<&CameraBasis>,
    orbit_installations: Query<&OrbitControllerInstallation>,
    free_installations: Query<&FreeFlightControllerInstallation>,
) {
    for (camera, sequence, playback, journey, stages) in &mut sequences {
        let availability = camera_controller_availability(
            orbit_cameras.get(camera).ok(),
            free_cameras.get(camera).ok(),
            coordinate_systems.get(camera).ok(),
        );
        let controller_installation = camera_controller_installation(
            camera,
            availability,
            &coordinate_systems,
            &orbit_installations,
            &free_installations,
        );
        let journey_current = journey
            .is_some_and(|journey| journey.matches_revision(sequence.sequence_stages().revision()));
        if direct_camera_sequence_is_prepared(
            sequence,
            playback.as_deref(),
            journey_current,
            stages,
            &controller_installation,
        ) {
            continue;
        }
        let mut playback = playback;
        match availability {
            CameraControllerAvailability::NoController
            | CameraControllerAvailability::FreeFlightWithoutBasis
            | CameraControllerAvailability::ConflictingControllers => {
                withdraw_unprepared_direct_camera_sequence(
                    &mut commands,
                    camera,
                    sequence,
                    playback.as_deref_mut(),
                    journey,
                    journey_current,
                );
                continue;
            },
            CameraControllerAvailability::InitializedOrbit
            | CameraControllerAvailability::InitializedFreeFlight => {},
        }
        let Ok(controller_installation) = controller_installation else {
            continue;
        };

        if let (Some(retained), Some(journey)) = (playback.as_deref_mut(), journey) {
            emit_cancelled_camera_lifecycle(&mut commands, camera, sequence, retained, journey);
        }

        let (controller_replaced, replacement_lifecycle) =
            direct_replacement_lifecycle(playback.as_deref(), controller_installation);
        if controller_replaced {
            commands.entity(camera).remove::<(
                OrbitControllerOverrideRestoration,
                FreeFlightControllerOverrideRestoration,
            )>();
        }

        let Some(preparation) = prepare_direct_camera_playback(
            camera,
            sequence,
            controller_installation,
            &orbit_cameras,
            &free_cameras,
        ) else {
            continue;
        };

        match preparation {
            Ok(mut prepared) => {
                prepared.lifecycle = replacement_lifecycle;
                commands
                    .entity(camera)
                    .insert((sequence.sequence_stages().clone(), prepared));
                if !journey_current {
                    commands
                        .entity(camera)
                        .insert(RetainedCameraJourney::direct(sequence));
                }
            },
            Err(error) => {
                warn!(camera = ?camera, error = ?error, "camera sequence preparation rejected captured state");
                if !journey_current {
                    commands
                        .entity(camera)
                        .insert(RetainedCameraJourney::direct(sequence));
                }
                commands
                    .entity(camera)
                    .remove::<(CameraSequencePlayback, SequenceStages)>();
            },
        }
    }
}

fn withdraw_unprepared_direct_camera_sequence(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    playback: Option<&mut CameraSequencePlayback>,
    journey: Option<&RetainedCameraJourney>,
    journey_current: bool,
) {
    if let (Some(retained), Some(journey)) = (playback, journey) {
        emit_cancelled_camera_lifecycle(commands, camera, sequence, retained, journey);
        retained.lifecycle = CameraPlaybackLifecycleState::Dormant;
    }
    if !journey_current {
        commands
            .entity(camera)
            .insert(RetainedCameraJourney::direct(sequence));
    }
    commands
        .entity(camera)
        .remove::<(CameraSequencePlayback, SequenceStages)>();
}

fn direct_camera_sequence_is_prepared(
    sequence: &CameraSequence,
    playback: Option<&CameraSequencePlayback>,
    journey_current: bool,
    stages: Option<&SequenceStages>,
    controller_installation: &Result<
        CameraControllerInstallation,
        CameraControllerInstallationError,
    >,
) -> bool {
    let current_revision = sequence.sequence_stages().revision();
    playback.is_some_and(|playback| {
        playback.revision == current_revision
            && controller_installation
                .is_ok_and(|current| playback.controller_installation == current)
    }) && journey_current
        && stages.is_some_and(|stages| stages.revision() == current_revision)
}

fn direct_replacement_lifecycle(
    playback: Option<&CameraSequencePlayback>,
    controller_installation: CameraControllerInstallation,
) -> (bool, CameraPlaybackLifecycleState) {
    let controller_replaced = playback.is_some_and(|retained| {
        !retained
            .controller_installation
            .is_same_installation(controller_installation)
    });
    let lifecycle = playback.map_or(
        CameraPlaybackLifecycleState::Dormant,
        |retained| match retained.lifecycle {
            CameraPlaybackLifecycleState::Dormant => CameraPlaybackLifecycleState::Dormant,
            CameraPlaybackLifecycleState::Effective { .. }
            | CameraPlaybackLifecycleState::ReplacingDefinitionWithRetainedOverride
            | CameraPlaybackLifecycleState::ReplacingControllerInstallation
            | CameraPlaybackLifecycleState::Closing { .. }
                if controller_replaced =>
            {
                CameraPlaybackLifecycleState::ReplacingControllerInstallation
            },
            CameraPlaybackLifecycleState::Effective { .. }
            | CameraPlaybackLifecycleState::ReplacingDefinitionWithRetainedOverride
            | CameraPlaybackLifecycleState::Closing { .. } => {
                CameraPlaybackLifecycleState::ReplacingDefinitionWithRetainedOverride
            },
            CameraPlaybackLifecycleState::ReplacingControllerInstallation => {
                CameraPlaybackLifecycleState::ReplacingControllerInstallation
            },
        },
    );
    (controller_replaced, lifecycle)
}

fn prepare_direct_camera_playback(
    camera: Entity,
    sequence: &CameraSequence,
    controller_installation: CameraControllerInstallation,
    orbit_cameras: &Query<&OrbitCam>,
    free_cameras: &Query<&FreeCam>,
) -> Option<Result<CameraSequencePlayback, CameraPlaybackPreparationError>> {
    match controller_installation {
        CameraControllerInstallation::Orbit(installation) => {
            let orbit = orbit_cameras.get(camera).ok()?;
            Some(CameraSequencePlayback::prepare_for_orbit(
                sequence,
                orbit,
                installation,
            ))
        },
        CameraControllerInstallation::FreeFlight {
            installation,
            basis,
        } => {
            let free = free_cameras.get(camera).ok()?;
            Some(CameraSequencePlayback::prepare_for_free(
                sequence,
                free,
                basis,
                installation,
            ))
        },
    }
}

/// Closes the old exact retained lifecycle before authoring is replaced or
/// removed. `Discard` runs while the old `CameraSequence` is still queryable,
/// so cancellation never borrows move identity from the new revision.
pub(in crate::animation) fn close_discarded_camera_sequence(
    discarded: On<Discard, CameraSequence>,
    mut commands: Commands,
    mut retained: Query<(
        &CameraSequence,
        &mut CameraSequencePlayback,
        &RetainedCameraJourney,
    )>,
) {
    let camera = discarded.entity;
    let Ok((sequence, mut playback, journey)) = retained.get_mut(camera) else {
        return;
    };
    emit_cancelled_camera_lifecycle(&mut commands, camera, sequence, &playback, journey);
    if matches!(
        playback.lifecycle,
        CameraPlaybackLifecycleState::Effective { .. }
            | CameraPlaybackLifecycleState::Closing { .. }
    ) {
        playback.lifecycle = CameraPlaybackLifecycleState::ReplacingDefinitionWithRetainedOverride;
    }
    playback.interruption.rearm();
}

/// Removes private retained state when direct authoring is explicitly removed.
pub(in crate::animation) fn remove_direct_camera_sequence(
    removed: On<Remove, CameraSequence>,
    mut commands: Commands,
) {
    commands.entity(removed.entity).remove::<(
        CameraSequencePlayback,
        SequenceStages,
        RetainedCameraJourney,
        NativeCameraPlayRequest,
    )>();
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::Vec3;

    use super::*;
    use crate::Position;
    use crate::animation::FreeCamRollTarget;
    use crate::animation::sequence::support::*;

    #[test]
    fn direct_removal_closes_and_restores_once_without_boundary_output() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<LifecycleEventOrder>()
            .init_resource::<CameraBoundaryEventOrder>();
        let mut orbit = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        orbit.zoom.set_damping(0.2);
        orbit.pan.set_damping(0.3);
        orbit.orbit.set_damping(0.4);
        let camera = app
            .world_mut()
            .spawn((
                orbit,
                CameraSequence::new(move_lasting(Duration::from_secs(1))),
            ))
            .id();
        record_lifecycle_order(app.world_mut(), camera);
        record_camera_boundary_order(app.world_mut(), camera);
        app.world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera));
        app.update();

        app.world_mut()
            .entity_mut(camera)
            .remove::<CameraSequence>();
        app.update();

        assert_eq!(
            app.world().resource::<LifecycleEventOrder>().0,
            Vec::<&str>::new()
        );
        assert!(
            app.world()
                .resource::<CameraBoundaryEventOrder>()
                .0
                .is_empty()
        );
        let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
        assert_eq!((transitions.captured, transitions.released), (1, 1));
        assert!(
            app.world()
                .get::<OrbitControllerOverrideRestoration>(camera)
                .is_none()
        );
        assert!(app.world().get::<CameraSequencePlayback>(camera).is_none());
        assert!(app.world().get::<SequenceStages>(camera).is_none());
        assert!(app.world().get::<RetainedCameraJourney>(camera).is_none());
        let orbit = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("direct removal retains its orbit controller")?;
        assert_eq!(
            (
                orbit.zoom.damping(),
                orbit.pan.damping(),
                orbit.orbit.damping(),
            ),
            (0.2, 0.3, 0.4)
        );
        Ok(())
    }

    #[test]
    fn controller_loss_consumes_old_stash_before_selected_playback_recovers() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<AnimationClosureCounts>()
            .init_resource::<LifecycleEventOrder>();
        let mut orbit = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        orbit.zoom.set_damping(0.2);
        orbit.pan.set_damping(0.3);
        orbit.orbit.set_damping(0.4);
        let camera = app
            .world_mut()
            .spawn((
                orbit,
                CameraSequence::new(move_lasting(Duration::from_secs(1))),
            ))
            .id();
        count_animation_closures(app.world_mut(), camera);
        record_lifecycle_order(app.world_mut(), camera);
        app.world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera));

        app.update();

        let original_stash = app
            .world()
            .get::<OrbitControllerOverrideRestoration>(camera)
            .ok_or("selected playback stashes the original controller")?;
        assert_eq!(
            (
                original_stash.zoom,
                original_stash.pan,
                original_stash.orbit
            ),
            (0.2, 0.3, 0.4)
        );
        let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
        assert_eq!((transitions.captured, transitions.released), (1, 0));
        assert_eq!(
            app.world().resource::<LifecycleEventOrder>().0,
            Vec::<&str>::new()
        );

        app.world_mut().entity_mut(camera).remove::<OrbitCam>();
        app.update();

        assert!(app.world().get::<CameraSequencePlayback>(camera).is_none());
        assert!(
            app.world()
                .get::<OrbitControllerOverrideRestoration>(camera)
                .is_none()
        );
        assert_eq!(
            app.world().resource::<AnimationClosureCounts>().cancelled,
            0
        );
        let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
        assert_eq!((transitions.captured, transitions.released), (1, 1));
        assert_eq!(
            app.world().resource::<LifecycleEventOrder>().0,
            Vec::<&str>::new()
        );

        let mut replacement = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        replacement.zoom.set_damping(0.6);
        replacement.pan.set_damping(0.7);
        replacement.orbit.set_damping(0.8);
        app.world_mut().entity_mut(camera).insert(replacement);
        app.update();

        assert!(app.world().get::<CameraSequencePlayback>(camera).is_some());
        let replacement_stash = app
            .world()
            .get::<OrbitControllerOverrideRestoration>(camera)
            .ok_or("recovered playback stashes the replacement controller")?;
        assert_eq!(
            (
                replacement_stash.zoom,
                replacement_stash.pan,
                replacement_stash.orbit,
            ),
            (0.6, 0.7, 0.8)
        );
        assert_eq!(
            app.world().resource::<AnimationClosureCounts>().cancelled,
            0
        );
        let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
        assert_eq!((transitions.captured, transitions.released), (2, 1));
        assert_eq!(
            app.world().resource::<LifecycleEventOrder>().0,
            Vec::<&str>::new()
        );
        Ok(())
    }

    #[test]
    fn failed_replacement_removes_the_previous_prepared_playback() -> TestResult {
        let mut app = camera_sequence_test_app();
        let camera = app
            .world_mut()
            .spawn((
                free_camera(Vec3::new(1.0, 2.0, 3.0), 0.25, -0.5, 0.125),
                CameraSequence::new(free_move(
                    Vec3::new(4.0, 5.0, 6.0),
                    Vec3::ZERO,
                    FreeCamRollTarget::InheritPrevious,
                    Duration::from_secs(1),
                )),
            ))
            .id();

        app.update();

        assert!(
            app.world()
                .entity(camera)
                .contains::<CameraSequencePlayback>()
        );

        let replacement = CameraSequence::new(free_move(
            Vec3::new(-4.0, 5.0, 6.0),
            Vec3::ZERO,
            FreeCamRollTarget::InheritPrevious,
            Duration::from_secs(1),
        ));
        let mut entity = app.world_mut().entity_mut(camera);
        entity.insert(replacement);
        let mut free_cam = entity
            .get_mut::<FreeCam>()
            .ok_or("the sequence camera retains its free-flight controller")?;
        free_cam
            .translate
            .set_current(Position(Vec3::splat(f32::NAN)));

        app.update();

        assert!(
            !app.world()
                .entity(camera)
                .contains::<CameraSequencePlayback>()
        );

        Ok(())
    }
}
