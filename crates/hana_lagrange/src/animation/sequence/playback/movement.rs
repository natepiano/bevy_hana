use std::time::Duration;

use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Time;
use bevy::prelude::Virtual;
use bevy::prelude::warn;
use bevy_kana::SequenceCommand;
use bevy_kana::SequenceCommands;
use bevy_kana::SequenceDirection;
use bevy_kana::SequenceMovementApplication;
use bevy_kana::SequenceOwner;
use bevy_kana::SequencePosition;
use bevy_kana::SequenceSeekResponse;
use bevy_kana::SequenceUpdate;

use super::CameraPlaybackClosureReason;
use super::CameraPlaybackInterruptionTransition;
use super::CameraPlaybackLifecycleState;
use super::CameraSequencePlayback;
use super::DriverEpisode;
use super::NativeCameraPlayRequest;
use super::begin_selected_driver_lifecycle;
use super::emit_camera_boundaries;
use super::transition_camera_lifecycle_owner;
use crate::FreeCam;
use crate::OrbitCam;
use crate::animation::lifecycle::FreeFlightControllerOverrideRestoration;
use crate::animation::lifecycle::OrbitControllerOverrideRestoration;
use crate::animation::queue::CameraInputInterruptBehavior;
use crate::animation::sequence::CameraSequence;
use crate::animation::sequence::RetainedCameraJourney;
use crate::animation::sequence::controller_installation;

/// Applies native or selected-driver movement once and translates every raw
/// crossed boundary before the evaluator samples the new local position.
pub(in crate::animation) fn apply_camera_sequence_movement(
    mut commands: Commands,
    time: Option<Res<Time<Virtual>>>,
    mut sequence_commands: SequenceCommands,
    interrupt_behaviors: Query<&CameraInputInterruptBehavior>,
    mut sequences: Query<(
        Entity,
        &CameraSequence,
        &mut CameraSequencePlayback,
        Option<&NativeCameraPlayRequest>,
        Option<&RetainedCameraJourney>,
    )>,
    mut orbit_cameras: Query<(
        &mut OrbitCam,
        &mut crate::OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    mut free_cameras: Query<(
        &mut FreeCam,
        &mut crate::FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) {
    for (camera, sequence, mut retained, native_play, journey) in &mut sequences {
        if native_play.is_some() {
            let _ = sequence_commands.apply(
                camera,
                SequenceOwner::NativePlayback,
                &mut retained.playback,
                SequenceCommand::Play,
            );
            commands.entity(camera).remove::<NativeCameraPlayRequest>();
        }
        let owner = sequence_commands.owner(camera);
        transition_camera_lifecycle_owner(
            &mut commands,
            camera,
            sequence,
            &mut retained,
            journey,
            owner,
            &mut orbit_cameras,
            &mut free_cameras,
        );

        let interruption = resolve_native_interruption(
            camera,
            &mut retained,
            &interrupt_behaviors,
            &mut orbit_cameras,
            &mut free_cameras,
        );

        let update = apply_camera_sequence_update(
            time.as_deref(),
            &mut sequence_commands,
            camera,
            sequence,
            &mut retained,
            owner,
            interruption,
        );
        if let (
            SequenceOwner::Driver(_),
            SequenceUpdate::Traversed { direction, .. },
            CameraPlaybackLifecycleState::Effective {
                owner: active_owner,
            },
            Some(journey),
        ) = (owner, update, retained.lifecycle, journey)
            && active_owner == owner
            && matches!(retained.driver_episode, DriverEpisode::Closed)
        {
            begin_selected_driver_lifecycle(
                &mut commands,
                camera,
                sequence,
                &mut retained,
                journey,
                owner,
                direction,
            );
            retained.driver_episode = DriverEpisode::Open;
        }
        if let SequenceUpdate::Traversed { direction, .. } = update {
            retained.last_direction = direction;
        }
        emit_camera_boundaries(
            &mut commands,
            camera,
            sequence,
            &mut retained,
            journey,
            owner,
            update,
        );
        close_completed_native_playback(&mut retained, update);
    }
}

/// Reports whether a moved controller interrupts native playback, and clears the
/// controller's pending input when it does.
fn resolve_native_interruption(
    camera: Entity,
    retained: &mut CameraSequencePlayback,
    interrupt_behaviors: &Query<&CameraInputInterruptBehavior>,
    orbit_cameras: &mut Query<(
        &mut OrbitCam,
        &mut crate::OrbitCamInput,
        Option<&OrbitControllerOverrideRestoration>,
    )>,
    free_cameras: &mut Query<(
        &mut FreeCam,
        &mut crate::FreeCamInput,
        Option<&FreeFlightControllerOverrideRestoration>,
    )>,
) -> CameraPlaybackInterruptionTransition {
    let native_interrupted = matches!(
        retained.lifecycle,
        CameraPlaybackLifecycleState::Effective {
            owner: SequenceOwner::NativePlayback,
        }
    ) && controller_installation::camera_controller_interrupted(
        camera,
        retained.last_applied,
        orbit_cameras,
        free_cameras,
    );
    if !native_interrupted {
        return CameraPlaybackInterruptionTransition::Continue;
    }

    let transition = retained
        .interruption
        .transition(interrupt_behaviors.get(camera).copied().unwrap_or_default());
    controller_installation::clear_camera_input(camera, orbit_cameras, free_cameras);
    transition
}

/// Closes native playback that has run out of movement, so the lifecycle leaves
/// its effective state.
const fn close_completed_native_playback(
    retained: &mut CameraSequencePlayback,
    update: SequenceUpdate,
) {
    if matches!(
        retained.lifecycle,
        CameraPlaybackLifecycleState::Effective {
            owner: SequenceOwner::NativePlayback,
        }
    ) && !retained.playback.is_playing()
        && !retained.playback.is_paused()
        && !matches!(update, SequenceUpdate::NoTraversal)
    {
        retained.lifecycle = CameraPlaybackLifecycleState::Closing {
            owner:  SequenceOwner::NativePlayback,
            reason: CameraPlaybackClosureReason::Completed,
        };
    }
}

pub(super) fn apply_camera_sequence_update(
    time: Option<&Time<Virtual>>,
    sequence_commands: &mut SequenceCommands,
    camera: Entity,
    sequence: &CameraSequence,
    retained: &mut CameraSequencePlayback,
    owner: SequenceOwner,
    interruption: CameraPlaybackInterruptionTransition,
) -> SequenceUpdate {
    match interruption {
        CameraPlaybackInterruptionTransition::Cancel => {
            let _ = sequence_commands.apply(
                camera,
                SequenceOwner::NativePlayback,
                &mut retained.playback,
                SequenceCommand::Cancel,
            );
            retained.lifecycle = CameraPlaybackLifecycleState::Closing {
                owner:  SequenceOwner::NativePlayback,
                reason: CameraPlaybackClosureReason::Cancelled,
            };
            SequenceUpdate::NoTraversal
        },
        CameraPlaybackInterruptionTransition::Complete => {
            let update = sequence_commands.try_seek(
                camera,
                SequenceOwner::NativePlayback,
                &mut retained.playback,
                SequencePosition::END,
                SequenceDirection::Forward,
            );
            retained.lifecycle = CameraPlaybackLifecycleState::Closing {
                owner:  SequenceOwner::NativePlayback,
                reason: CameraPlaybackClosureReason::Completed,
            };
            match update {
                Ok(SequenceSeekResponse::Sought(update)) => update,
                Ok(SequenceSeekResponse::Rejected(owner)) => {
                    warn!(camera = ?camera, owner = ?owner, "native camera completion was rejected after interruption");
                    SequenceUpdate::NoTraversal
                },
                Err(error) => {
                    warn!(camera = ?camera, error = ?error, "native camera completion movement was invalid");
                    SequenceUpdate::NoTraversal
                },
            }
        },
        CameraPlaybackInterruptionTransition::Continue => {
            if matches!(
                retained.lifecycle,
                CameraPlaybackLifecycleState::Effective {
                    owner: SequenceOwner::NativePlayback,
                }
            ) && !retained.playback.is_playing()
                && !retained.playback.is_paused()
            {
                retained.lifecycle = CameraPlaybackLifecycleState::Closing {
                    owner:  SequenceOwner::NativePlayback,
                    reason: CameraPlaybackClosureReason::Cancelled,
                };
                SequenceUpdate::NoTraversal
            } else {
                match sequence_commands.apply_selected_movement(camera, &mut retained.playback) {
                    SequenceMovementApplication::NativePlayback => sequence_commands
                        .advance_native(
                            camera,
                            &mut retained.playback,
                            time.map_or(Duration::ZERO, Time::delta),
                            sequence.total(),
                        ),
                    SequenceMovementApplication::HoldingPriorPosition => {
                        SequenceUpdate::NoTraversal
                    },
                    SequenceMovementApplication::Moved(update) => update,
                    SequenceMovementApplication::Invalid(error) => {
                        warn!(camera = ?camera, owner = ?owner, error = ?error, "camera movement contradicted local playback and was not applied");
                        SequenceUpdate::NoTraversal
                    },
                }
            }
        },
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crate::CameraInputInterruptBehavior;
    use crate::animation::sequence::support::*;

    #[test]
    fn camera_commands_distinguish_absent_native_and_selected_driver_states() -> TestResult {
        let mut app = camera_sequence_test_app();
        assert_absent_camera_command_state(&mut app)?;
        assert_native_camera_command_states()?;
        assert_selected_driver_command_state(&mut app)
    }

    #[test]
    fn orbit_native_input_obeys_ignore_cancel_and_complete() -> TestResult {
        for behavior in [
            CameraInputInterruptBehavior::Ignore,
            CameraInputInterruptBehavior::Cancel,
            CameraInputInterruptBehavior::Complete,
        ] {
            run_orbit_interruption(behavior)?;
        }
        Ok(())
    }

    #[test]
    fn free_native_input_obeys_ignore_cancel_and_complete() -> TestResult {
        for behavior in [
            CameraInputInterruptBehavior::Ignore,
            CameraInputInterruptBehavior::Cancel,
            CameraInputInterruptBehavior::Complete,
        ] {
            run_free_interruption(behavior)?;
        }
        Ok(())
    }

    #[test]
    fn selected_driver_input_suppression_is_interruption_policy_independent() -> TestResult {
        for behavior in [
            CameraInputInterruptBehavior::Ignore,
            CameraInputInterruptBehavior::Cancel,
            CameraInputInterruptBehavior::Complete,
        ] {
            run_selected_orbit_policy(behavior)?;
        }
        Ok(())
    }
}
