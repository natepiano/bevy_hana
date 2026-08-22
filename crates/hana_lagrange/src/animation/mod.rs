//! Retained camera animation over ordered [`CameraMove`] authoring.
//!
//! [`PlayAnimation`] is the durable request facade. Accepted requests and direct
//! [`CameraSequence`] authoring share one retained evaluator, shared sequence
//! commands, and shared driver ownership.

mod constants;
mod events;
mod lifecycle;
mod queue;
mod sequence;

use bevy::ecs::schedule::ApplyDeferred;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::App;
use bevy::prelude::Plugin;
use bevy::prelude::Update;
use bevy_kana::SequencePlaybackPlugin;
use bevy_kana::SequencePlaybackSystems;
pub use events::AnimationBegin;
pub use events::AnimationEnd;
pub use events::AnimationReason;
pub use events::AnimationRejected;
pub use events::AnimationRejectionReason;
pub use events::AnimationSource;
pub use events::CameraEventTiming;
pub use events::CameraMoveBegin;
pub use events::CameraMoveEnd;
pub use events::CameraRequestPreparationError;
pub use events::PlayAnimation;
pub use lifecycle::AnimationConflictPolicy;
pub use queue::CameraInputInterruptBehavior;
pub use queue::CameraMove;
pub use queue::CameraMoveDestination;
pub use queue::CameraMoveError;
pub use queue::FreeCamRollTarget;
pub(crate) use queue::orbital_parameters_from_offset;
pub(crate) use queue::warn_rejected_camera_move;
pub use sequence::CameraCommands;
use sequence::CameraControllerInstallationIdentityAllocator;
pub use sequence::CameraEvaluationError;
pub use sequence::CameraPlaybackObservation;
pub use sequence::CameraSequence;
pub use sequence::CameraSequenceError;
pub(crate) use sequence::CameraSequencePlayback;
use sequence::CameraSequenceSystems;
use sequence::PendingCameraRequests;

/// Registers retained camera playback and its public request facade.
pub(crate) struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SequencePlaybackPlugin)
            .init_resource::<PendingCameraRequests>()
            .init_resource::<CameraControllerInstallationIdentityAllocator>()
            .add_observer(sequence::identify_orbit_controller_installation)
            .add_observer(sequence::identify_free_flight_controller_installation)
            .add_observer(sequence::capture_play_animation)
            .add_observer(sequence::close_discarded_camera_sequence)
            .add_observer(sequence::remove_direct_camera_sequence)
            .add_observer(lifecycle::restore_retained_camera_state)
            .configure_sets(
                Update,
                (
                    CameraSequenceSystems::InitializeControllers,
                    CameraSequenceSystems::PublishDirectAuthoring,
                )
                    .chain()
                    .before(SequencePlaybackSystems::ArbitrateDrivers),
            )
            .configure_sets(
                Update,
                CameraSequenceSystems::AdmitRequests
                    .after(SequencePlaybackSystems::ArbitrateDrivers)
                    .before(SequencePlaybackSystems::ProduceMovement),
            )
            .add_systems(
                Update,
                (
                    ApplyDeferred,
                    sequence::initialize_camera_controllers
                        .in_set(CameraSequenceSystems::InitializeControllers),
                    ApplyDeferred,
                    sequence::prepare_direct_camera_sequences
                        .in_set(CameraSequenceSystems::PublishDirectAuthoring),
                    ApplyDeferred,
                )
                    .chain()
                    .before(SequencePlaybackSystems::ArbitrateDrivers),
            )
            .add_systems(
                Update,
                (
                    ApplyDeferred,
                    sequence::admit_pending_camera_requests
                        .in_set(CameraSequenceSystems::AdmitRequests),
                    ApplyDeferred,
                )
                    .chain()
                    .after(SequencePlaybackSystems::ArbitrateDrivers)
                    .before(SequencePlaybackSystems::ProduceMovement),
            )
            .add_systems(
                Update,
                sequence::apply_camera_sequence_movement
                    .in_set(SequencePlaybackSystems::ApplyMovement),
            )
            .add_systems(
                Update,
                (
                    sequence::clear_selected_driver_input,
                    sequence::evaluate_camera_sequences,
                    sequence::finalize_camera_sequence_transitions,
                )
                    .chain()
                    .in_set(SequencePlaybackSystems::EvaluateSequences),
            );
    }
}
