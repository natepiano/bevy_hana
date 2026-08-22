use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ReflectFromReflect;

use super::CameraInputMetricKind;
use super::ControlSpeed;
use super::InteractionSources;

/// Semantic kind of an `OrbitCam` interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum OrbitCamInteractionKind {
    /// Orbit interaction.
    Orbit,
    /// Pan interaction.
    Pan,
    /// Zoom interaction.
    Zoom,
}

/// Semantic kind of a `FreeCam` interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum FreeCamInteractionKind {
    /// Translation interaction.
    Translate,
    /// Free-look interaction.
    Look,
    /// Roll interaction.
    Roll,
}

/// Emitted when an `OrbitCam` interaction starts.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct OrbitCamInteractionStarted {
    /// Camera entity whose interaction started.
    #[event_target]
    pub camera:  Entity,
    /// Kind of interaction that started.
    pub kind:    OrbitCamInteractionKind,
    /// Sources that contributed to the interaction.
    pub sources: InteractionSources,
}

/// Emitted when a `FreeCam` interaction starts.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct FreeCamInteractionStarted {
    /// Camera entity whose interaction started.
    #[event_target]
    pub camera:  Entity,
    /// Kind of interaction that started.
    pub kind:    FreeCamInteractionKind,
    /// Sources that contributed to the interaction.
    pub sources: InteractionSources,
}

/// Emitted when an `OrbitCam` interaction ends.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct OrbitCamInteractionEnded {
    /// Camera entity whose interaction ended.
    #[event_target]
    pub camera:  Entity,
    /// Kind of interaction that ended.
    pub kind:    OrbitCamInteractionKind,
    /// Sources that contributed before the interaction ended.
    pub sources: InteractionSources,
}

/// Emitted when a `FreeCam` interaction ends.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct FreeCamInteractionEnded {
    /// Camera entity whose interaction ended.
    #[event_target]
    pub camera:  Entity,
    /// Kind of interaction that ended.
    pub kind:    FreeCamInteractionKind,
    /// Sources that contributed before the interaction ended.
    pub sources: InteractionSources,
}

/// Emitted when an active `OrbitCam` interaction's source set changes.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct OrbitCamInteractionSourcesChanged {
    /// Camera entity whose source attribution changed.
    #[event_target]
    pub camera:   Entity,
    /// Kind of interaction whose sources changed.
    pub kind:     OrbitCamInteractionKind,
    /// Sources active before the change.
    pub previous: InteractionSources,
    /// Sources active after the change.
    pub current:  InteractionSources,
}

/// Emitted when an active `FreeCam` interaction's source set changes.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct FreeCamInteractionSourcesChanged {
    /// Camera entity whose source attribution changed.
    #[event_target]
    pub camera:   Entity,
    /// Kind of interaction whose sources changed.
    pub kind:     FreeCamInteractionKind,
    /// Sources active before the change.
    pub previous: InteractionSources,
    /// Sources active after the change.
    pub current:  InteractionSources,
}

impl OrbitCamInteractionSourcesChanged {
    /// Returns sources that joined this interaction.
    #[must_use]
    pub const fn added_sources(&self) -> InteractionSources {
        self.current.difference(self.previous)
    }

    /// Returns sources that left this interaction.
    #[must_use]
    pub const fn removed_sources(&self) -> InteractionSources {
        self.previous.difference(self.current)
    }
}

impl FreeCamInteractionSourcesChanged {
    /// Returns sources that joined this interaction.
    #[must_use]
    pub const fn added_sources(&self) -> InteractionSources {
        self.current.difference(self.previous)
    }

    /// Returns sources that left this interaction.
    #[must_use]
    pub const fn removed_sources(&self) -> InteractionSources {
        self.previous.difference(self.current)
    }
}

/// Emitted when an active `OrbitCam` interaction switches speed variant — for
/// the gamepad preset, when the `rb`/`lb` modifier is pressed or released
/// mid-interaction without changing the source set.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct OrbitCamInteractionSpeedChanged {
    /// Camera entity whose interaction speed changed.
    #[event_target]
    pub camera: Entity,
    /// Kind of interaction whose speed changed.
    pub kind:   OrbitCamInteractionKind,
    /// Speed variant active after the change.
    pub speed:  ControlSpeed,
}

/// Emitted when an active `FreeCam` interaction switches speed variant.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct FreeCamInteractionSpeedChanged {
    /// Camera entity whose interaction speed changed.
    #[event_target]
    pub camera: Entity,
    /// Kind of interaction whose speed changed.
    pub kind:   FreeCamInteractionKind,
    /// Speed variant active after the change.
    pub speed:  ControlSpeed,
}

/// Emitted on the rising edge of a camera home/reset action.
///
/// The event fires when the camera begins easing toward its home pose. Fired
/// for every camera kind that supports home; panels light the home row off this
/// until the eased motion settles.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct CameraHomed {
    /// Camera entity whose home was invoked.
    #[event_target]
    pub camera:  Entity,
    /// Sources that triggered the home action.
    pub sources: InteractionSources,
}

/// Triggers an `OrbitCam` reset to its stored home pose.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct ResetOrbitCamToHome {
    /// Camera entity to reset.
    #[event_target]
    pub camera: Entity,
}

/// Triggers a `FreeCam` reset to its stored home pose.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct ResetFreeCamToHome {
    /// Camera entity to reset.
    #[event_target]
    pub camera: Entity,
}

/// Emitted when logical input metrics are required but unavailable for a camera.
#[derive(Clone, Copy, Debug, EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct CameraInputMetricsMissing {
    /// Camera entity missing metrics.
    #[event_target]
    pub camera: Entity,
    /// Metric that could not be resolved.
    pub metric: CameraInputMetricKind,
}
