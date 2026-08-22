//! The panel↔run relationship: each text run carries [`TextRunOf`] pointing at
//! its panel; the panel carries [`PanelTextRuns`], the Bevy-maintained set of
//! its run entities.
//!
//! This is an additive typed index over the text-run subset of a panel's
//! children. The runs keep their `ChildOf(panel)` for transform propagation and
//! despawn; `TextRunOf` only narrows traversal to text runs, so a query never
//! has to filter a panel's [`Children`] by [`PanelTextLayout`](super::PanelTextLayout)
//! and the lone run of a one-element [`DiegeticText`](crate::DiegeticText) is
//! found with [`PanelTextRuns::sole`].

use std::ops::Deref;

use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FromWorld;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::ReflectFromWorld;
use bevy::prelude::World;

/// Relationship source on each panel text run, pointing at its owning panel.
///
/// Mirrors [`ChildOf`](bevy::prelude::ChildOf): a public `Entity` field plus a
/// [`panel`](Self::panel) accessor. Bevy maintains the matching [`PanelTextRuns`] set on the panel
/// as runs carrying this component spawn and despawn.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component, PartialEq, Debug, FromWorld, Clone)]
#[relationship(relationship_target = PanelTextRuns)]
pub struct TextRunOf(#[entities] pub Entity);

impl TextRunOf {
    /// The panel entity this run belongs to.
    #[must_use]
    pub const fn panel(&self) -> Entity { self.0 }
}

// `Reflect` deserializes by constructing an instance and patching it, so the
// derive needs a `FromWorld`/`Default`. A relationship source is only ever set
// with a real panel entity, so the placeholder mirrors `ChildOf`'s own
// `FromWorld`.
impl FromWorld for TextRunOf {
    fn from_world(_world: &mut World) -> Self { Self(Entity::PLACEHOLDER) }
}

/// Relationship target on a panel: the set of its text-run entities, maintained
/// by Bevy as runs carrying [`TextRunOf`] spawn and despawn.
///
/// No `linked_spawn`: the runs already sit under `ChildOf(panel)`, whose
/// `linked_spawn` despawns them with the panel, so a second recursive path here
/// would double-despawn. Despawn is `ChildOf`'s job; this is a traversal index.
/// The relationship's on-remove hook still drops a despawned run from the set,
/// so membership stays accurate without `linked_spawn`.
///
/// `iter()` (from the relationship target) yields each run `Entity` by value;
/// the [`Deref`] to `[Entity]` adds `len()` and indexing.
#[derive(Component, Default, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component, FromWorld, Default)]
#[relationship_target(relationship = TextRunOf)]
pub struct PanelTextRuns(Vec<Entity>);

impl PanelTextRuns {
    /// The lone run of a panel whose set holds exactly one entity, else `None`.
    ///
    /// A single-line [`DiegeticText`](crate::DiegeticText) has exactly one run
    /// entity, so this returns it with no id. A wrapped run reifies as one
    /// entity per visual line, so the set holds several and this returns `None`;
    /// the access layer resolves a wrapped run's `line_index == 0` entity
    /// instead (see [`PanelTextReader::sole_text`](super::PanelTextReader)).
    #[must_use]
    pub const fn sole(&self) -> Option<Entity> {
        match self.0.as_slice() {
            [run] => Some(*run),
            _ => None,
        }
    }
}

impl Deref for PanelTextRuns {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}
