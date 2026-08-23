use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use hana_kana::SequenceDirection;
use hana_kana::SequenceOwner;

use super::CameraSequencePlayback;
use super::emission;
use crate::FreeCam;
use crate::FreeCamInput;
use crate::OrbitCam;
use crate::OrbitCamInput;
use crate::animation::lifecycle::FreeFlightControllerOverrideRestoration;
use crate::animation::lifecycle::OrbitControllerOverrideRestoration;
use crate::animation::queue::CameraInputInterruptBehavior;
use crate::animation::sequence::CameraSequence;
use crate::animation::sequence::RetainedCameraJourney;
use crate::animation::sequence::controller_installation;

/// The one lifecycle owner whose retained movement is currently effective.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::animation::sequence) enum CameraPlaybackLifecycleState {
    #[default]
    Dormant,
    Effective {
        owner: SequenceOwner,
    },
    /// The old definition has closed while its controller override remains
    /// continuously retained until the replacement's final owner is known.
    ReplacingDefinitionWithRetainedOverride,
    /// The old installation has closed and its stale override has been
    /// discarded. A continuing owner must capture the replacement's damping.
    ReplacingControllerInstallation,
    Closing {
        owner:  SequenceOwner,
        reason: CameraPlaybackClosureReason,
    },
}

/// Why an effective retained lifecycle is closing after its final evaluation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::animation::sequence) enum CameraPlaybackClosureReason {
    Completed,
    Cancelled,
}

/// `NativePlaybackActivation` records whether native playback may own controller output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativePlaybackActivation {
    Effective,
    Inactive,
}

/// `PlaybackControllerChange` records whether preparation kept the controller installation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlaybackControllerChange {
    Unchanged,
    Replaced,
}

/// `DriverEpisode` records whether a selected driver opened its current lifecycle episode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum DriverEpisode {
    #[default]
    Closed,
    Open,
}

/// The native interruption action selected by a state transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CameraPlaybackInterruptionTransition {
    Continue,
    Cancel,
    Complete,
}

/// Which native input-interruption transition is currently being applied.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum CameraPlaybackInterruptionState {
    #[default]
    Monitoring,
    Cancelling,
    Completing,
}

impl CameraPlaybackInterruptionState {
    pub(super) const fn is_monitoring(self) -> bool { matches!(self, Self::Monitoring) }

    pub(super) const fn transition(
        &mut self,
        behavior: CameraInputInterruptBehavior,
    ) -> CameraPlaybackInterruptionTransition {
        if !self.is_monitoring() {
            return CameraPlaybackInterruptionTransition::Continue;
        }
        match behavior {
            CameraInputInterruptBehavior::Ignore => CameraPlaybackInterruptionTransition::Continue,
            CameraInputInterruptBehavior::Cancel => {
                *self = Self::Cancelling;
                CameraPlaybackInterruptionTransition::Cancel
            },
            CameraInputInterruptBehavior::Complete => {
                *self = Self::Completing;
                CameraPlaybackInterruptionTransition::Complete
            },
        }
    }

    pub(in crate::animation::sequence) const fn rearm(&mut self) { *self = Self::Monitoring; }
}

pub(super) fn transition_camera_lifecycle_owner(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &mut CameraSequencePlayback,
    journey: Option<&RetainedCameraJourney>,
    owner: SequenceOwner,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    let native_activation = if owner == SequenceOwner::NativePlayback
        && (retained.playback.is_playing() || retained.playback.is_paused())
    {
        NativePlaybackActivation::Effective
    } else {
        NativePlaybackActivation::Inactive
    };
    match retained.lifecycle {
        CameraPlaybackLifecycleState::ReplacingDefinitionWithRetainedOverride => {
            transition_reprepared_camera_lifecycle(
                commands,
                camera,
                sequence,
                retained,
                journey,
                owner,
                native_activation,
                PlaybackControllerChange::Unchanged,
                orbit_cameras,
                free_cameras,
            );
        },
        CameraPlaybackLifecycleState::ReplacingControllerInstallation => {
            transition_reprepared_camera_lifecycle(
                commands,
                camera,
                sequence,
                retained,
                journey,
                owner,
                native_activation,
                PlaybackControllerChange::Replaced,
                orbit_cameras,
                free_cameras,
            );
        },
        CameraPlaybackLifecycleState::Dormant
            if matches!(owner, SequenceOwner::Driver(_))
                || matches!(native_activation, NativePlaybackActivation::Effective) =>
        {
            activate_dormant_camera_lifecycle(
                commands,
                camera,
                sequence,
                retained,
                journey,
                owner,
                native_activation,
                orbit_cameras,
                free_cameras,
            );
        },
        CameraPlaybackLifecycleState::Effective { owner: previous } if previous != owner => {
            transition_effective_camera_lifecycle(
                commands,
                camera,
                sequence,
                retained,
                journey,
                previous,
                owner,
                native_activation,
                orbit_cameras,
                free_cameras,
            );
        },
        CameraPlaybackLifecycleState::Dormant
        | CameraPlaybackLifecycleState::Effective { .. }
        | CameraPlaybackLifecycleState::Closing { .. } => {},
    }
}

fn activate_dormant_camera_lifecycle(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &mut CameraSequencePlayback,
    journey: Option<&RetainedCameraJourney>,
    owner: SequenceOwner,
    native_activation: NativePlaybackActivation,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    controller_installation::stash_camera_controller_override(
        commands,
        camera,
        retained.controller_installation,
        orbit_cameras,
        free_cameras,
    );
    if matches!(native_activation, NativePlaybackActivation::Effective)
        && let Some(journey) = journey
    {
        emission::emit_camera_lifecycle_begin(
            commands,
            camera,
            sequence,
            retained,
            journey,
            owner,
            retained.last_direction,
        );
    }
    retained.lifecycle = CameraPlaybackLifecycleState::Effective { owner };
    retained.driver_episode = DriverEpisode::Closed;
    retained.interruption.rearm();
}

fn transition_effective_camera_lifecycle(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &mut CameraSequencePlayback,
    journey: Option<&RetainedCameraJourney>,
    previous: SequenceOwner,
    owner: SequenceOwner,
    native_activation: NativePlaybackActivation,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    if let Some(journey) = journey
        && (previous == SequenceOwner::NativePlayback
            || matches!(retained.driver_episode, DriverEpisode::Open))
    {
        emission::emit_cancelled_camera_lifecycle(commands, camera, sequence, retained, journey);
    }
    if matches!(owner, SequenceOwner::Driver(_))
        || matches!(native_activation, NativePlaybackActivation::Effective)
    {
        if matches!(native_activation, NativePlaybackActivation::Effective)
            && let Some(journey) = journey
        {
            emission::emit_camera_lifecycle_begin(
                commands,
                camera,
                sequence,
                retained,
                journey,
                owner,
                retained.last_direction,
            );
        }
        retained.lifecycle = CameraPlaybackLifecycleState::Effective { owner };
    } else {
        controller_installation::restore_camera_controller_override(
            commands,
            camera,
            retained.controller_installation,
            orbit_cameras,
            free_cameras,
        );
        retained.lifecycle = CameraPlaybackLifecycleState::Dormant;
    }
    retained.driver_episode = DriverEpisode::Closed;
    retained.interruption.rearm();
}

fn transition_reprepared_camera_lifecycle(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &mut CameraSequencePlayback,
    journey: Option<&RetainedCameraJourney>,
    owner: SequenceOwner,
    native_activation: NativePlaybackActivation,
    controller_change: PlaybackControllerChange,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    if matches!(controller_change, PlaybackControllerChange::Replaced)
        && (matches!(owner, SequenceOwner::Driver(_))
            || matches!(native_activation, NativePlaybackActivation::Effective))
    {
        controller_installation::stash_camera_controller_override(
            commands,
            camera,
            retained.controller_installation,
            orbit_cameras,
            free_cameras,
        );
    }
    if matches!(owner, SequenceOwner::Driver(_)) {
        retained.lifecycle = CameraPlaybackLifecycleState::Effective { owner };
    } else if matches!(native_activation, NativePlaybackActivation::Effective) {
        if let Some(journey) = journey {
            emission::emit_camera_lifecycle_begin(
                commands,
                camera,
                sequence,
                retained,
                journey,
                owner,
                retained.last_direction,
            );
        }
        retained.lifecycle = CameraPlaybackLifecycleState::Effective { owner };
    } else {
        if matches!(controller_change, PlaybackControllerChange::Unchanged) {
            controller_installation::restore_camera_controller_override(
                commands,
                camera,
                retained.controller_installation,
                orbit_cameras,
                free_cameras,
            );
        }
        retained.lifecycle = CameraPlaybackLifecycleState::Dormant;
    }
    retained.driver_episode = DriverEpisode::Closed;
    retained.interruption.rearm();
}

pub(super) fn begin_selected_driver_lifecycle(
    commands: &mut Commands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &mut CameraSequencePlayback,
    journey: &RetainedCameraJourney,
    owner: SequenceOwner,
    direction: SequenceDirection,
) {
    emission::emit_camera_lifecycle_begin(
        commands, camera, sequence, retained, journey, owner, direction,
    );
    retained.lifecycle = CameraPlaybackLifecycleState::Effective { owner };
    retained.interruption.rearm();
}

pub(super) fn camera_journey_matches(
    sequence: &CameraSequence,
    retained: &CameraSequencePlayback,
    journey: &RetainedCameraJourney,
) -> bool {
    let revision = sequence.sequence_stages().revision();
    retained.revision == revision && journey.revision == revision
}

pub(super) const fn camera_lifecycle_owner_and_direction(
    retained: &CameraSequencePlayback,
) -> (SequenceOwner, SequenceDirection) {
    match retained.lifecycle {
        CameraPlaybackLifecycleState::Effective { owner }
        | CameraPlaybackLifecycleState::Closing { owner, .. } => (owner, retained.last_direction),
        CameraPlaybackLifecycleState::Dormant
        | CameraPlaybackLifecycleState::ReplacingDefinitionWithRetainedOverride
        | CameraPlaybackLifecycleState::ReplacingControllerInstallation => {
            (SequenceOwner::NativePlayback, retained.last_direction)
        },
    }
}

/// Emits the final native lifecycle after endpoint evaluation, then restores
/// the controller override once while keeping retained authoring available.
pub(in crate::animation) fn finalize_camera_sequence_transitions(
    mut commands: Commands,
    mut sequences: Query<(
        Entity,
        &CameraSequence,
        &mut CameraSequencePlayback,
        &RetainedCameraJourney,
    )>,
    mut orbit_cameras: Query<(
        &mut OrbitCam,
        &mut OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    mut free_cameras: Query<(
        &mut FreeCam,
        &mut FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    for (camera, sequence, mut retained, journey) in &mut sequences {
        let CameraPlaybackLifecycleState::Closing { owner: _, reason } = retained.lifecycle else {
            continue;
        };
        emission::emit_camera_lifecycle_end(
            &mut commands,
            camera,
            sequence,
            &retained,
            journey,
            reason,
        );
        controller_installation::restore_camera_controller_override(
            &mut commands,
            camera,
            retained.controller_installation,
            &mut orbit_cameras,
            &mut free_cameras,
        );
        retained.lifecycle = CameraPlaybackLifecycleState::Dormant;
        retained.interruption.rearm();
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::Camera;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::Projection;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec3;
    use hana_kana::SequenceDriver;
    use hana_kana::SequenceDriverTakeover;
    use hana_kana::SequenceEasing;
    use hana_kana::SequenceEvaluation;
    use hana_kana::SequenceScope;
    use hana_kana::SequenceSourceState;

    use super::*;
    use crate::CameraBasis;
    use crate::LookAngles;
    use crate::PlayAnimation;
    use crate::Position;
    use crate::Roll;
    use crate::animation::sequence::playback::*;
    use crate::animation::sequence::support::*;
    use crate::operation::Focus;
    use crate::operation::OrbitAngles;
    use crate::operation::Radius;

    #[test]
    fn native_interruption_transitions_close_once_until_rearmed() {
        let mut state = CameraPlaybackInterruptionState::Monitoring;
        assert_eq!(
            state.transition(CameraInputInterruptBehavior::Ignore),
            CameraPlaybackInterruptionTransition::Continue
        );
        assert_eq!(state, CameraPlaybackInterruptionState::Monitoring);
        assert_eq!(
            state.transition(CameraInputInterruptBehavior::Cancel),
            CameraPlaybackInterruptionTransition::Cancel
        );
        assert_eq!(state, CameraPlaybackInterruptionState::Cancelling);
        assert_eq!(
            state.transition(CameraInputInterruptBehavior::Complete),
            CameraPlaybackInterruptionTransition::Continue
        );
        state.rearm();
        assert_eq!(
            state.transition(CameraInputInterruptBehavior::Complete),
            CameraPlaybackInterruptionTransition::Complete
        );
        assert_eq!(state, CameraPlaybackInterruptionState::Completing);
        assert_eq!(
            state.transition(CameraInputInterruptBehavior::Cancel),
            CameraPlaybackInterruptionTransition::Continue
        );
    }

    #[test]
    fn same_batch_last_wins_replacement_cancels_before_beginning_and_stashes_once() {
        let mut app = camera_sequence_test_app();
        app.init_resource::<LifecycleEventOrder>();
        let mut orbit = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        orbit.zoom.set_damping(0.2);
        orbit.pan.set_damping(0.3);
        orbit.orbit.set_damping(0.4);
        let camera = app
            .world_mut()
            .spawn((
                orbit,
                Camera::default(),
                Projection::Perspective(PerspectiveProjection::default()),
                Transform::from_xyz(0.0, 0.0, 10.0),
            ))
            .id();
        record_lifecycle_order(app.world_mut(), camera);
        app.world_mut().trigger(PlayAnimation::new(
            camera,
            [orbital_move(
                Vec3::ZERO,
                0.1,
                0.0,
                2.0,
                Duration::from_secs(1),
            )],
        ));
        app.world_mut().trigger(PlayAnimation::new(
            camera,
            [orbital_move(
                Vec3::ZERO,
                0.2,
                0.0,
                5.0,
                Duration::from_secs(1),
            )],
        ));

        app.update();

        assert_eq!(
            app.world().resource::<LifecycleEventOrder>().0,
            ["begin", "cancel", "begin"]
        );
        assert_eq!(
            app.world()
                .get::<CameraSequence>(camera)
                .expect("the final request is retained")
                .moves()[0]
                .radius(),
            Radius(5.0)
        );
        let stash = app
            .world()
            .get::<OrbitControllerOverrideRestoration>(camera)
            .expect("replacement keeps the original controller override stash");
        assert_eq!((stash.zoom, stash.pan, stash.orbit), (0.2, 0.3, 0.4));
    }

    #[test]
    fn selected_direct_replacement_keeps_override_and_stale_release_closes_once() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<LifecycleEventOrder>()
            .init_resource::<CameraBoundaryEventOrder>()
            .init_resource::<ObservedDriverRestorations>()
            .add_observer(record_driver_restoration);
        let mut orbit = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        orbit.zoom.set_damping(0.2);
        orbit.pan.set_damping(0.3);
        orbit.orbit.set_damping(0.4);
        let old_sequence =
            CameraSequence::new(orbital_move(Vec3::X, 0.1, 0.2, 4.0, Duration::from_secs(1)));
        let old_stage = old_sequence
            .stage_ids_with_spans()
            .next()
            .ok_or("the old direct sequence has one stage")?
            .0;
        let camera = app.world_mut().spawn((orbit, old_sequence)).id();
        record_lifecycle_order(app.world_mut(), camera);
        record_camera_boundary_order(app.world_mut(), camera);
        let displaced = app
            .world_mut()
            .spawn((
                hana_kana::SequenceDriver::new(camera),
                SequenceEvaluation::new(SequenceScope::Stage(old_stage), SequenceEasing::Authored),
            ))
            .id();

        app.update();

        let selected = app
            .world_mut()
            .spawn((
                hana_kana::SequenceDriver::new(camera),
                SequenceDriverTakeover,
            ))
            .id();
        app.update();

        let live_focus = Focus(Vec3::new(3.0, 2.0, 1.0));
        let live_angles = OrbitAngles {
            yaw:   0.7,
            pitch: -0.2,
        };
        let live_radius = Radius(7.0);
        let expected_start = CameraPose::try_orbit(live_focus, live_angles, live_radius)
            .map_err(|_| "the live replacement pose is representable")?;
        {
            let mut orbit = app
                .world_mut()
                .get_mut::<OrbitCam>(camera)
                .ok_or("the selected direct camera retains its orbit controller")?;
            orbit.pan.snap_to(live_focus);
            orbit.orbit.snap_to(live_angles);
            orbit.zoom.snap_to(live_radius);
        }
        app.world_mut()
            .entity_mut(camera)
            .insert(CameraSequence::new(orbital_move(
                Vec3::Y,
                -0.4,
                0.3,
                2.0,
                Duration::from_secs(2),
            )));

        app.update();

        assert_selected_direct_replacement_state(&app, camera, selected, expected_start)?;
        release_stale_selected_driver(&mut app, camera, selected, displaced)
    }

    #[test]
    fn native_effective_direct_replacement_becomes_inert_and_restores_once() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<LifecycleEventOrder>();
        let mut orbit = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        orbit.zoom.set_damping(0.2);
        orbit.pan.set_damping(0.3);
        orbit.orbit.set_damping(0.4);
        let camera = app.world_mut().spawn(orbit).id();
        record_lifecycle_order(app.world_mut(), camera);
        app.world_mut().trigger(PlayAnimation::new(
            camera,
            [move_lasting(Duration::from_secs(10))],
        ));
        app.update();

        app.world_mut()
            .entity_mut(camera)
            .insert(CameraSequence::new(move_lasting(Duration::from_secs(1))));
        app.update();

        assert_eq!(
            app.world().resource::<LifecycleEventOrder>().0,
            ["begin", "cancel"]
        );
        let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
        assert_eq!((transitions.captured, transitions.released), (1, 1));
        assert!(
            app.world()
                .get::<OrbitControllerOverrideRestoration>(camera)
                .is_none()
        );
        let orbit = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("the inert replacement retains its controller")?;
        assert_eq!(
            (
                orbit.zoom.damping(),
                orbit.pan.damping(),
                orbit.orbit.damping(),
            ),
            (0.2, 0.3, 0.4)
        );
        assert!(matches!(
            app.world()
                .get::<CameraSequencePlayback>(camera)
                .ok_or("the inert direct definition remains prepared")?
                .lifecycle,
            CameraPlaybackLifecycleState::Dormant
        ));
        Ok(())
    }

    #[test]
    fn selected_takeover_and_valid_release_pair_lifecycle_without_idle_pose_writes() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<LifecycleEventOrder>()
            .init_resource::<CameraBoundaryEventOrder>()
            .init_resource::<ObservedDriverRestorations>()
            .add_observer(record_driver_restoration);
        let focus = Focus(Vec3::new(1.0, 2.0, 3.0));
        let angles = OrbitAngles {
            yaw:   0.4,
            pitch: -0.1,
        };
        let radius = Radius(8.0);
        let expected_pose = OrbitCameraPose {
            focus,
            orbit_angles: angles,
            radius,
        };
        let mut orbit = OrbitCam::from_pose(focus, angles, radius);
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
        let displaced = app
            .world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera))
            .id();
        app.update();

        let selected = app
            .world_mut()
            .spawn((
                hana_kana::SequenceDriver::new(camera),
                SequenceDriverTakeover,
            ))
            .id();
        app.update();
        app.world_mut()
            .get_mut::<SequenceSourceState>(displaced)
            .ok_or("the displaced driver retains source state")?
            .mark_current();
        app.world_mut()
            .entity_mut(selected)
            .remove::<SequenceDriver>();
        app.update();

        assert_restored_takeover_state(&app, camera, displaced, expected_pose)?;
        release_restored_driver_and_assert_idle(&mut app, camera, displaced)
    }

    #[test]
    fn same_update_orbit_replacement_recaptures_and_restores_its_own_override() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<LifecycleEventOrder>()
            .init_resource::<CameraBoundaryEventOrder>();
        let mut original = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        original.zoom.set_damping(0.2);
        original.pan.set_damping(0.3);
        original.orbit.set_damping(0.4);
        let camera = app
            .world_mut()
            .spawn((
                original,
                CameraSequence::new(move_lasting(Duration::from_secs(1))),
            ))
            .id();
        record_lifecycle_order(app.world_mut(), camera);
        record_camera_boundary_order(app.world_mut(), camera);
        let driver = app
            .world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera))
            .id();
        app.update();

        let focus = Focus(Vec3::new(3.0, 2.0, 1.0));
        let angles = OrbitAngles {
            yaw:   0.7,
            pitch: -0.2,
        };
        let radius = Radius(7.0);
        let expected_start = CameraPose::try_orbit(focus, angles, radius)
            .map_err(|_| "the replacement orbit pose is representable")?;
        let mut replacement = OrbitCam::from_pose(focus, angles, radius);
        replacement.zoom.set_damping(0.6);
        replacement.pan.set_damping(0.7);
        replacement.orbit.set_damping(0.8);
        app.world_mut().entity_mut(camera).insert(replacement);

        app.update();

        assert_orbit_replacement_state(&app, camera, expected_start)?;
        release_orbit_replacement_override(&mut app, camera, driver)
    }

    #[test]
    fn same_update_cross_kind_replacement_captures_only_the_new_override() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<LifecycleEventOrder>()
            .init_resource::<CameraBoundaryEventOrder>();
        let mut original = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        original.zoom.set_damping(0.2);
        original.pan.set_damping(0.3);
        original.orbit.set_damping(0.4);
        let camera = app
            .world_mut()
            .spawn((
                original,
                CameraSequence::new(move_lasting(Duration::from_secs(1))),
            ))
            .id();
        record_lifecycle_order(app.world_mut(), camera);
        record_camera_boundary_order(app.world_mut(), camera);
        let driver = app
            .world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera))
            .id();
        app.update();

        let position = Position(Vec3::new(4.0, 3.0, 2.0));
        let look = LookAngles {
            yaw:   -0.4,
            pitch: 0.2,
        };
        let roll = Roll(0.3);
        let expected_start = CameraPose::try_free(position, look, roll)
            .map_err(|_| "the replacement free-flight pose is representable")?;
        let mut replacement = FreeCam::from_pose(position, look, roll);
        replacement.translate.set_damping(0.6);
        replacement.look.set_damping(0.7);
        replacement.roll.set_damping(0.8);
        app.world_mut().entity_mut(camera).remove::<OrbitCam>();
        app.world_mut()
            .entity_mut(camera)
            .insert((replacement, CameraBasis::Y_UP));

        app.update();

        assert_cross_kind_replacement_state(&app, camera, expected_start)?;
        release_cross_kind_replacement_override(&mut app, camera, driver)
    }

    #[test]
    fn same_update_replacement_without_an_owner_leaves_new_damping_untouched() -> TestResult {
        let mut app = camera_sequence_test_app();
        app.init_resource::<LifecycleEventOrder>()
            .init_resource::<CameraBoundaryEventOrder>();
        let mut original = orbit_camera(Vec3::ZERO, 0.0, 0.0, 10.0);
        original.zoom.set_damping(0.2);
        original.pan.set_damping(0.3);
        original.orbit.set_damping(0.4);
        let camera = app
            .world_mut()
            .spawn((
                original,
                CameraSequence::new(move_lasting(Duration::from_secs(1))),
            ))
            .id();
        record_lifecycle_order(app.world_mut(), camera);
        record_camera_boundary_order(app.world_mut(), camera);
        let driver = app
            .world_mut()
            .spawn(hana_kana::SequenceDriver::new(camera))
            .id();
        app.update();

        let focus = Focus(Vec3::new(6.0, 5.0, 4.0));
        let angles = OrbitAngles {
            yaw:   -0.6,
            pitch: 0.25,
        };
        let radius = Radius(9.0);
        let mut replacement = OrbitCam::from_pose(focus, angles, radius);
        replacement.zoom.set_damping(0.6);
        replacement.pan.set_damping(0.7);
        replacement.orbit.set_damping(0.8);
        app.world_mut()
            .entity_mut(driver)
            .remove::<SequenceDriver>();
        app.world_mut().entity_mut(camera).insert(replacement);

        app.update();

        let retained = app
            .world()
            .get::<CameraSequencePlayback>(camera)
            .ok_or("the ownerless replacement retains dormant preparation")?;
        assert_eq!(
            retained.start.pose(),
            CameraPose::try_orbit(focus, angles, radius)
                .map_err(|_| "the ownerless replacement pose is representable")?
        );
        assert!(matches!(
            retained.lifecycle,
            CameraPlaybackLifecycleState::Dormant
        ));
        assert!(
            app.world()
                .get::<OrbitControllerOverrideRestoration>(camera)
                .is_none()
        );
        assert!(
            app.world()
                .get::<FreeFlightControllerOverrideRestoration>(camera)
                .is_none()
        );
        let orbit = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("the ownerless replacement controller remains installed")?;
        assert_eq!(
            (
                orbit.zoom.damping(),
                orbit.pan.damping(),
                orbit.orbit.damping(),
            ),
            (0.6, 0.7, 0.8)
        );
        let transitions = app.world().resource::<ControllerOverrideTransitionCounts>();
        assert_eq!((transitions.captured, transitions.released), (1, 1));
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
        Ok(())
    }
}
