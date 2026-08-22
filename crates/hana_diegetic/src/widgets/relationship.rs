use std::ops::Deref;

use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FromWorld;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::ReflectFromWorld;
use bevy::prelude::World;

/// Relationship source on each tooltip controller, pointing at its target.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component, PartialEq, Debug, FromWorld, Clone)]
#[relationship(relationship_target = Tooltips)]
pub struct TooltipFor(#[entities] Entity);

impl TooltipFor {
    pub(crate) const fn new(target: Entity) -> Self { Self(target) }

    /// Returns the entity described by this tooltip.
    #[must_use]
    pub const fn target(&self) -> Entity { self.0 }
}

impl FromWorld for TooltipFor {
    fn from_world(_: &mut World) -> Self { Self(Entity::PLACEHOLDER) }
}

/// Reverse membership for tooltip controllers describing one target.
#[derive(Component, Default, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component, FromWorld, Default)]
#[relationship_target(relationship = TooltipFor, linked_spawn)]
pub struct Tooltips(Vec<Entity>);

impl Tooltips {
    /// Iterates over the target's tooltip controller entities.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ { self.0.iter().copied() }
}

impl Deref for Tooltips {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}

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

/// Private lowering from a screen panel attachment to its widget target.
///
/// The matching [`ScreenWidgetAnchoredHere`] component is the authoritative
/// screen-space geometry demand. This relation has no `linked_spawn`; panel
/// role teardown removes the authored attachment while the widget is still
/// queryable.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
#[component(immutable)]
#[relationship(relationship_target = ScreenWidgetAnchoredHere)]
pub(crate) struct ScreenWidgetAnchoredTo(
    #[relationship]
    #[entities]
    Entity,
);

impl ScreenWidgetAnchoredTo {
    pub(crate) const fn new(target: Entity) -> Self { Self(target) }

    pub(crate) const fn target(self) -> Entity { self.0 }
}

impl FromWorld for ScreenWidgetAnchoredTo {
    fn from_world(_: &mut World) -> Self { Self(Entity::PLACEHOLDER) }
}

/// Reverse membership for screen panels attached to one widget.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = ScreenWidgetAnchoredTo)]
pub(crate) struct ScreenWidgetAnchoredHere(Vec<Entity>);

impl ScreenWidgetAnchoredHere {
    pub(crate) fn iter(&self) -> impl Iterator<Item = Entity> + '_ { self.0.iter().copied() }

    pub(crate) const fn is_empty(&self) -> bool { self.0.is_empty() }
}

impl Deref for ScreenWidgetAnchoredHere {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}

/// Marks a widget that contributes a synthetic screen resolver candidate.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub(crate) struct ScreenWidgetAnchorProxy;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_widget_relationship_tracks_retarget_remove_and_despawn() {
        let mut world = World::new();
        let first_target = world.spawn_empty().id();
        let second_target = world.spawn_empty().id();
        let first_source = world.spawn(ScreenWidgetAnchoredTo::new(first_target)).id();
        let second_source = world.spawn(ScreenWidgetAnchoredTo::new(first_target)).id();

        assert_eq!(
            world
                .get::<ScreenWidgetAnchoredHere>(first_target)
                .map(ScreenWidgetAnchoredHere::iter)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            vec![first_source, second_source],
        );

        world
            .entity_mut(first_source)
            .insert(ScreenWidgetAnchoredTo::new(second_target));
        assert_eq!(
            world
                .get::<ScreenWidgetAnchoredHere>(first_target)
                .map(ScreenWidgetAnchoredHere::iter)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            vec![second_source],
        );
        assert_eq!(
            world
                .get::<ScreenWidgetAnchoredHere>(second_target)
                .map(ScreenWidgetAnchoredHere::iter)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            vec![first_source],
        );

        world
            .entity_mut(second_source)
            .remove::<ScreenWidgetAnchoredTo>();
        assert!(
            world
                .get::<ScreenWidgetAnchoredHere>(first_target)
                .is_none()
        );

        world.despawn(first_source);
        assert!(
            world
                .get::<ScreenWidgetAnchoredHere>(second_target)
                .is_none()
        );
    }
}
