//! Same-frame hidden resolution for draws that follow another entity's visibility.
//!
//! Batched text, image, fill, and shape records draw from batch entities that
//! sit outside the panel hierarchy, so Bevy's visibility propagation never
//! reaches them; a cable likewise draws apart from the jacks it joins. Such
//! systems can run before `VisibilitySystems::VisibilityPropagate`, where
//! `InheritedVisibility` still holds the previous frame. [`AncestorVisibility`]
//! reads `Visibility` up the `ChildOf` chain instead, which is current the frame
//! it changes.

use bevy::ecs::change_detection::Tick;
use bevy::ecs::entity::EntityHashMap;
use bevy::ecs::system::SystemChangeTick;
use bevy::ecs::system::SystemParam;
use bevy::prelude::ChildOf;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::Query;
use bevy::prelude::Visibility;

/// Resolves whether an entity is hidden by its own `Visibility` or by any
/// ancestor it inherits from.
///
/// The rules match Bevy's propagation: `Hidden` hides, `Visible` shows
/// whatever its ancestors say, and `Inherited` defers to the parent. An entity
/// with no `Visibility` defers the same way, and the root of the chain is
/// visible.
///
/// Answers are kept for the rest of the system run, so the many records one
/// panel owns walk its ancestors once.
#[derive(SystemParam)]
pub struct AncestorVisibility<'w, 's> {
    nodes:    Query<'w, 's, (Option<&'static Visibility>, Option<&'static ChildOf>)>,
    resolved: Local<'s, ResolvedThisRun>,
    ticks:    SystemChangeTick,
}

/// Hidden answers from one system run, dropped when the next run starts.
#[derive(Default)]
struct ResolvedThisRun {
    run:    Option<Tick>,
    hidden: EntityHashMap<bool>,
}

impl AncestorVisibility<'_, '_> {
    /// Whether `entity` resolves hidden this frame.
    pub fn is_hidden(&mut self, entity: Entity) -> bool {
        let run = self.ticks.this_run();
        if self.resolved.run != Some(run) {
            self.resolved.run = Some(run);
            self.resolved.hidden.clear();
        }
        if let Some(&hidden) = self.resolved.hidden.get(&entity) {
            return hidden;
        }
        let hidden = self.walk(entity);
        self.resolved.hidden.insert(entity, hidden);
        hidden
    }

    fn walk(&self, entity: Entity) -> bool {
        let mut current = entity;
        loop {
            let Ok((visibility, child_of)) = self.nodes.get(current) else {
                return false;
            };
            match visibility {
                Some(Visibility::Hidden) => return true,
                Some(Visibility::Visible) => return false,
                Some(Visibility::Inherited) | None => {
                    let Some(child_of) = child_of else {
                        return false;
                    };
                    current = child_of.parent();
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::SystemParamValidationError;
    use bevy::ecs::system::SystemState;
    use bevy::prelude::World;

    use super::*;

    fn resolves_hidden(
        world: &mut World,
        entity: Entity,
    ) -> Result<bool, SystemParamValidationError> {
        let mut state = SystemState::<AncestorVisibility>::new(world);
        state
            .get_mut(world)
            .map(|mut visibility| visibility.is_hidden(entity))
    }

    #[test]
    fn a_hidden_grandparent_hides_every_inheriting_descendant() {
        let mut world = World::new();
        let root = world.spawn(Visibility::Hidden).id();
        let middle = world.spawn((Visibility::Inherited, ChildOf(root))).id();
        let bare = world.spawn(ChildOf(middle)).id();
        let leaf = world.spawn((Visibility::Inherited, ChildOf(bare))).id();

        for entity in [root, middle, bare, leaf] {
            assert!(
                matches!(resolves_hidden(&mut world, entity), Ok(true)),
                "{entity} inherits the root's Hidden"
            );
        }
    }

    #[test]
    fn visible_overrides_a_hidden_ancestor_and_a_visible_chain_draws() {
        let mut world = World::new();
        let hidden_root = world.spawn(Visibility::Hidden).id();
        let shown = world
            .spawn((Visibility::Visible, ChildOf(hidden_root)))
            .id();
        let under_shown = world.spawn((Visibility::Inherited, ChildOf(shown))).id();
        let visible_root = world.spawn(Visibility::Inherited).id();
        let visible_leaf = world
            .spawn((Visibility::Inherited, ChildOf(visible_root)))
            .id();

        for entity in [shown, under_shown, visible_root, visible_leaf] {
            assert!(
                matches!(resolves_hidden(&mut world, entity), Ok(false)),
                "{entity} resolves visible"
            );
        }
    }
}
