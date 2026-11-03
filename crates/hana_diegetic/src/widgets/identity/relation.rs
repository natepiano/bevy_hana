use std::ops::Deref;

use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FromWorld;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::ReflectFromWorld;
use bevy::prelude::World;

/// Relationship source on each widget, pointing at its owning panel.
///
/// Bevy maintains the matching [`PanelWidgets`] set as widget entities spawn
/// and despawn. [`ChildOf`](bevy::prelude::ChildOf) remains responsible for hierarchy and despawn.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component, PartialEq, Debug, FromWorld, Clone)]
#[relationship(relationship_target = PanelWidgets)]
pub struct WidgetOf(#[entities] Entity);

impl WidgetOf {
    pub(crate) const fn new(panel: Entity) -> Self { Self(panel) }

    /// Returns the panel entity this widget belongs to.
    #[must_use]
    pub const fn panel(&self) -> Entity { self.0 }
}

impl FromWorld for WidgetOf {
    fn from_world(_world: &mut World) -> Self { Self(Entity::PLACEHOLDER) }
}

/// Relationship target on a panel containing its widget entities.
///
/// This relationship has no `linked_spawn`: widget entities are already
/// descendants through [`ChildOf`](bevy::prelude::ChildOf), which owns their recursive despawn.
#[derive(Component, Default, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component, FromWorld, Default)]
#[relationship_target(relationship = WidgetOf)]
pub struct PanelWidgets(Vec<Entity>);

impl Deref for PanelWidgets {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}
