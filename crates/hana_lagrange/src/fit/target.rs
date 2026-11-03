use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::On;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ReflectFromReflect;
use bevy::prelude::Vec3;

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

/// Leaves a mesh out of the bounds a fit measures from the target it stands under.
///
/// For a mesh that draws nothing the camera must frame, such as an invisible quad that
/// only takes pointer hits. The mesh's children are still measured.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct OutsideFitBounds;

/// Local-space box drawn by an entity without a main-world mesh.
///
/// Camera fits measure its eight corners alongside visible mesh vertices.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub struct FitBounds {
    /// Centre of the box in the entity's local space.
    pub center:       Vec3,
    /// Positive half extents of the box in the entity's local space.
    pub half_extents: Vec3,
}

/// Chooses the geometry a request frames without changing the target's hierarchy.
#[derive(Clone, Debug, Reflect)]
pub(crate) enum FitBoundsSource {
    MeasureTarget,
    WorldBounds(Vec<Vec3>),
}

/// Geometry published atomically with the admitted fit target, including retargets.
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(Component)]
pub struct AdmittedFitBounds(pub(crate) FitBoundsSource);

/// Inserts [`CurrentFitTarget`] on the camera a [`SetFitTarget`] event names.
pub(super) fn on_set_fit_target(set_target: On<SetFitTarget>, mut commands: Commands) {
    commands.entity(set_target.camera).insert((
        CurrentFitTarget(set_target.target),
        AdmittedFitBounds(FitBoundsSource::MeasureTarget),
    ));
}
