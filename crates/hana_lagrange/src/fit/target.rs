use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::On;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ReflectFromReflect;

/// Sets the debug overlay target without triggering a zoom.
#[derive(EntityEvent, Reflect)]
#[reflect(Event, FromReflect)]
pub struct SetFitTarget {
    /// The camera entity.
    #[event_target]
    pub camera: Entity,
    /// The entity whose bounds to visualize.
    pub target: Entity,
}

impl SetFitTarget {
    /// Creates a new `SetFitTarget` event.
    #[must_use]
    pub const fn new(camera: Entity, target: Entity) -> Self { Self { camera, target } }
}

/// Marks the entity that the camera is currently fitted to.
///
/// Stays on the camera after the fit completes, so the debug overlay keeps
/// drawing the same target.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
pub struct CurrentFitTarget(
    /// The entity being fitted.
    pub Entity,
);

/// Inserts [`CurrentFitTarget`] on the camera a [`SetFitTarget`] event names.
pub(super) fn on_set_fit_target(set_target: On<SetFitTarget>, mut commands: Commands) {
    commands
        .entity(set_target.camera)
        .insert(CurrentFitTarget(set_target.target));
}
