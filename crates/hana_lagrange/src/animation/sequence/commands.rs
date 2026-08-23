use bevy::ecs::system::SystemParam;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use hana_kana::SequenceCommand;
use hana_kana::SequenceCommandResponse;
use hana_kana::SequenceCommands;
use hana_kana::SequenceOwner;
use hana_kana::SequenceOwnership;
use hana_kana::SequencePosition;

use super::playback::CameraSequencePlayback;

/// Exact public observation of a camera domain's retained local playback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraPlaybackObservation {
    /// The camera has no prepared retained playback.
    NoRetainedSequence,
    /// The retained sequence has this local position and current owner.
    Retained {
        /// Current local normalized position.
        position: SequencePosition,
        /// Native or selected-driver owner.
        owner:    SequenceOwner,
    },
}

/// Camera-domain commands over private retained playback.
///
/// Absence is reported separately from native ownership, and a selected driver
/// receives the same shared command rejection event as fold playback.
#[derive(SystemParam)]
pub struct CameraCommands<'w, 's> {
    sequence_commands: SequenceCommands<'w, 's>,
    playbacks:         Query<'w, 's, &'static mut CameraSequencePlayback>,
}

impl CameraCommands<'_, '_> {
    /// Returns whether `camera` has retained playback and, if so, who owns it.
    #[must_use]
    pub fn owner(&self, camera: Entity) -> SequenceOwnership {
        if self.playbacks.get(camera).is_err() {
            return SequenceOwnership::NoRetainedSequence;
        }
        SequenceOwnership::Retained(self.sequence_commands.owner(camera))
    }

    /// Reads the camera's exact local playback state without exposing storage.
    #[must_use]
    pub fn observe(&self, camera: Entity) -> CameraPlaybackObservation {
        let Ok(playback) = self.playbacks.get(camera) else {
            return CameraPlaybackObservation::NoRetainedSequence;
        };
        CameraPlaybackObservation::Retained {
            position: playback.playback.position(),
            owner:    self.sequence_commands.owner(camera),
        }
    }

    /// Applies a shared targeted command to a retained camera sequence.
    pub fn apply(
        &mut self,
        camera: Entity,
        issuer: SequenceOwner,
        command: SequenceCommand,
    ) -> SequenceCommandResponse {
        let Ok(mut playback) = self.playbacks.get_mut(camera) else {
            return SequenceCommandResponse::NoRetainedSequence;
        };
        self.sequence_commands
            .apply(camera, issuer, &mut playback.playback, command)
    }
}
