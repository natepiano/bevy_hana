//! The panel↔source relationship types for the panel-shape path.
//!
//! These components define the typed traversal index for authored panel-shape
//! sources. A source entity may expand to several analytic path primitives; the
//! primitives remain renderer data, not Bevy entities.

use std::ops::Deref;

use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FromWorld;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::ReflectFromWorld;
use bevy::prelude::World;

use crate::layout::PanelShapeSourceKey;

/// Marker on a panel-shape source entity owned by a diegetic panel.
///
/// The marker identifies entities that participate in the
/// [`PanelShapeOf`] / [`PanelShapes`] traversal index. It does not carry batch
/// data or material-slot ownership.
#[derive(Component, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
#[reflect(Component, PartialEq, Debug, FromWorld, Default, Clone)]
pub(super) struct PanelShape;

/// Resolved source identity stored on a [`PanelShape`] entity.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PanelShapeSource {
    /// Stable source key from the resolved panel command stream.
    pub(super) key:           PanelShapeSourceKey,
    /// Source command index from [`ResolvedPanelShape`](crate::ResolvedPanelShape).
    pub(super) command_index: usize,
}

/// Relationship source on each panel-shape source, pointing at its panel.
///
/// Mirrors [`ChildOf`]: Bevy maintains the matching [`PanelShapes`] set on the
/// panel as sources carrying this component spawn and despawn.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq, Reflect)]
#[reflect(Component, PartialEq, Debug, FromWorld, Clone)]
#[relationship(relationship_target = PanelShapes)]
pub(super) struct PanelShapeOf(#[entities] pub(super) Entity);

impl FromWorld for PanelShapeOf {
    fn from_world(_world: &mut World) -> Self { Self(Entity::PLACEHOLDER) }
}

/// Relationship target on a panel: the set of its panel-shape source entities.
///
/// Bevy maintains this set from [`PanelShapeOf`] components. The target is a
/// traversal index only; the normal hierarchy remains responsible for
/// transform propagation and linked despawn behavior.
#[derive(Component, Default, Debug, Eq, PartialEq, Reflect)]
#[reflect(Component, FromWorld, Default)]
#[relationship_target(relationship = PanelShapeOf)]
pub(super) struct PanelShapes(Vec<Entity>);

impl Deref for PanelShapes {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}
