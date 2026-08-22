use bevy::ecs::system::ParamSet;
use bevy::prelude::Add;
use bevy::prelude::Changed;
use bevy::prelude::ChildOf;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Mesh3d;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::RelationshipTarget;
use bevy::prelude::Remove;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::world_serialization::WorldInstanceReady;

use super::NoOutline;
use super::Outline;
use super::OutlineBarrier;

/// The descendant `Mesh3d` entities an outline on `source` propagates to.
/// Prunes every [`OutlineBarrier`] subtree below `source` and skips
/// [`NoOutline`] meshes. `source` itself is never a target, so a barrier
/// entity can still source an outline for its own subtree.
fn propagation_targets(
    source: Entity,
    children_query: &Query<&Children>,
    barrier_query: &Query<(), With<OutlineBarrier>>,
    mesh_query: &Query<(), (With<Mesh3d>, Without<NoOutline>)>,
) -> Vec<Entity> {
    let mut targets = Vec::new();
    let mut stack: Vec<Entity> = children_query
        .get(source)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    while let Some(entity) = stack.pop() {
        if barrier_query.contains(entity) {
            continue;
        }
        if mesh_query.contains(entity) {
            targets.push(entity);
        }
        if let Ok(children) = children_query.get(entity) {
            stack.extend(children.iter());
        }
    }
    targets
}

/// When `Outline` is added to an entity, propagate it to all descendant `Mesh3d` entities.
/// Skips entities with `NoOutline` and subtrees behind an [`OutlineBarrier`].
/// Sets `group_source` for `Grouped` overlap mode.
pub(crate) fn propagate_outline_to_descendants(
    added: On<Add, Outline>,
    outline_query: Query<&Outline>,
    mesh_query: Query<(), (With<Mesh3d>, Without<NoOutline>)>,
    barrier_query: Query<(), With<OutlineBarrier>>,
    children_query: Query<&Children>,
    mut commands: Commands,
) {
    let source = added.entity;
    let Ok(outline) = outline_query.get(source) else {
        return;
    };

    // Don't re-propagate from entities that received their outline via propagation
    if outline.group_source.is_some() {
        return;
    }

    let mut propagated = outline.clone();
    propagated.group_source = Some(source);

    for target in propagation_targets(source, &children_query, &barrier_query, &mesh_query) {
        commands.entity(target).insert(propagated.clone());
    }
}

/// When a new child is added to the hierarchy, check if any ancestor has `Outline`
/// and propagate it. Handles glTF scene loading where children spawn after the parent.
pub(crate) fn propagate_outline_on_child_added(
    added: On<Add, ChildOf>,
    child_mesh_query: Query<(), (With<Mesh3d>, Without<NoOutline>)>,
    outline_query: Query<&Outline>,
    barrier_query: Query<(), With<OutlineBarrier>>,
    parent_query: Query<&ChildOf>,
    mut commands: Commands,
) {
    let child = added.entity;
    if !child_mesh_query.contains(child) {
        return;
    }

    // Follow `parent_query` through `ChildOf::parent` until a source `Outline` is
    // found. An `OutlineBarrier` on the child or any ancestor below the outline
    // shields the child from inheriting it.
    let mut current = child;
    loop {
        if barrier_query.contains(current) {
            return;
        }
        let Ok(child_of) = parent_query.get(current) else {
            return;
        };
        let parent = child_of.parent();
        if let Ok(outline) = outline_query.get(parent) {
            // Preserve `Outline::group_source` when the parent outline is already propagated.
            let source = outline.group_source.unwrap_or(parent);
            let mut propagated = outline.clone();
            propagated.group_source = Some(source);
            commands.entity(child).insert(propagated);
            return;
        }
        current = parent;
    }
}

/// When `Mesh3d` is added to an entity, check if any ancestor has `Outline` and propagate it.
/// Handles glTF scene loading where `Mesh3d` may be added after `ChildOf`.
pub(crate) fn propagate_outline_on_mesh_added(
    added: On<Add, Mesh3d>,
    no_outline_query: Query<(), With<NoOutline>>,
    outline_query: Query<&Outline>,
    barrier_query: Query<(), With<OutlineBarrier>>,
    parent_query: Query<&ChildOf>,
    existing_outline: Query<(), With<Outline>>,
    mut commands: Commands,
) {
    let child = added.entity;
    if no_outline_query.contains(child) {
        return;
    }
    if existing_outline.contains(child) {
        return;
    }

    // Follow `parent_query` through `ChildOf::parent` until an `Outline` is
    // found. An `OutlineBarrier` on the child or any ancestor below the outline
    // shields the child from inheriting it.
    let mut current = child;
    loop {
        if barrier_query.contains(current) {
            return;
        }
        let Ok(child_of) = parent_query.get(current) else {
            return;
        };
        let parent = child_of.parent();
        if let Ok(outline) = outline_query.get(parent) {
            let source = outline.group_source.unwrap_or(parent);
            let mut propagated = outline.clone();
            propagated.group_source = Some(source);
            commands.entity(child).insert(propagated);
            return;
        }
        current = parent;
    }
}

/// When a `WorldInstanceReady` fires on an entity with `Outline`, propagate to
/// all descendant meshes. This handles the `WorldAssetRoot` case where the world instance
/// entity may not have a `ChildOf` back to the entity with `Outline`.
pub(crate) fn propagate_outline_on_scene_ready(
    ready: On<WorldInstanceReady>,
    outline_query: Query<&Outline>,
    mesh_query: Query<(), (With<Mesh3d>, Without<NoOutline>)>,
    barrier_query: Query<(), With<OutlineBarrier>>,
    children_query: Query<&Children>,
    mut commands: Commands,
) {
    let source = ready.entity;
    let Ok(outline) = outline_query.get(source) else {
        return;
    };
    if outline.group_source.is_some() {
        return;
    }

    let mut propagated = outline.clone();
    propagated.group_source = Some(source);

    for target in propagation_targets(source, &children_query, &barrier_query, &mesh_query) {
        commands.entity(target).insert(propagated.clone());
    }
}

/// When `Outline` is removed from a source entity, remove it from all descendants.
/// Only acts on source outlines (not propagated copies) to avoid cascading removals.
/// Matching on `group_source` alone (no barrier pruning) also cleans up copies
/// stranded inside a subtree whose `OutlineBarrier` was added after propagation.
pub(crate) fn remove_outline_from_descendants(
    removed: On<Remove, Outline>,
    outline_query: Query<&Outline>,
    mesh_query: Query<(), With<Mesh3d>>,
    children_query: Query<&Children>,
    mut commands: Commands,
) {
    let source = removed.entity;

    // Check if any descendant has a propagated outline from this source.
    // If descendants have outlines with a different source (or no source), leave them alone.
    for descendant in children_query.iter_descendants(source) {
        if !mesh_query.contains(descendant) {
            continue;
        }
        if let Ok(descendant_outline) = outline_query.get(descendant)
            && descendant_outline.group_source == Some(source)
        {
            commands.entity(descendant).try_remove::<Outline>();
        }
    }
}

/// When a source `Outline` changes, update all descendant copies whose
/// `group_source` points at it. The `ParamSet` separates the change-detection
/// read from the descendant writes so mid-tree meshes (which have `Children`
/// themselves) sync too.
pub(crate) fn sync_propagated_outlines(
    mut outlines: ParamSet<(
        Query<(Entity, &Outline), (Changed<Outline>, With<Children>)>,
        Query<&mut Outline>,
    )>,
    mesh_query: Query<(), (With<Mesh3d>, Without<NoOutline>)>,
    barrier_query: Query<(), With<OutlineBarrier>>,
    children_query: Query<&Children>,
) {
    // Only sync outlines that are sources (no group_source means this is the original)
    let sources: Vec<(Entity, Outline)> = outlines
        .p0()
        .iter()
        .filter(|(_, outline)| outline.group_source.is_none())
        .map(|(source, outline)| {
            let mut propagated = outline.clone();
            propagated.group_source = Some(source);
            (source, propagated)
        })
        .collect();

    for (source, propagated) in sources {
        for target in propagation_targets(source, &children_query, &barrier_query, &mesh_query) {
            if let Ok(mut target_outline) = outlines.p1().get_mut(target)
                && target_outline.group_source == Some(source)
            {
                *target_outline = propagated.clone();
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::Handle;
    use bevy::prelude::World;

    use super::*;

    const OUTLINE_WIDTH: f32 = 4.0;

    fn outline() -> Outline { Outline::jump_flood(OUTLINE_WIDTH).build() }

    fn world_with_observers() -> World {
        let mut world = World::new();
        world.add_observer(propagate_outline_to_descendants);
        world.add_observer(propagate_outline_on_child_added);
        world.add_observer(propagate_outline_on_mesh_added);
        world.add_observer(remove_outline_from_descendants);
        world
    }

    #[test]
    fn barrier_blocks_descendant_propagation() {
        let mut world = world_with_observers();
        let root = world.spawn_empty().id();
        let mesh = world.spawn((Mesh3d(Handle::default()), ChildOf(root))).id();
        let barrier = world.spawn((OutlineBarrier, ChildOf(root))).id();
        let barrier_mesh = world
            .spawn((Mesh3d(Handle::default()), ChildOf(barrier)))
            .id();

        world.entity_mut(root).insert(outline());

        assert!(world.entity(mesh).contains::<Outline>());
        assert!(!world.entity(barrier).contains::<Outline>());
        assert!(!world.entity(barrier_mesh).contains::<Outline>());
    }

    #[test]
    fn barrier_entity_sources_its_own_subtree() {
        let mut world = world_with_observers();
        let barrier = world.spawn(OutlineBarrier).id();
        let mesh = world
            .spawn((Mesh3d(Handle::default()), ChildOf(barrier)))
            .id();

        world.entity_mut(barrier).insert(outline());

        let propagated = world
            .entity(mesh)
            .get::<Outline>()
            .expect("barrier source propagates to its own subtree");
        assert_eq!(propagated.group_source, Some(barrier));
    }

    #[test]
    fn barrier_blocks_child_added_inheritance() {
        let mut world = world_with_observers();
        let root = world.spawn(outline()).id();
        let barrier = world.spawn((OutlineBarrier, ChildOf(root))).id();
        let late_mesh = world
            .spawn((Mesh3d(Handle::default()), ChildOf(barrier)))
            .id();

        assert!(!world.entity(late_mesh).contains::<Outline>());
    }

    #[test]
    fn barrier_blocks_mesh_added_inheritance() {
        let mut world = world_with_observers();
        let root = world.spawn(outline()).id();
        let barrier = world.spawn((OutlineBarrier, ChildOf(root))).id();
        let child = world.spawn(ChildOf(barrier)).id();

        world.entity_mut(child).insert(Mesh3d(Handle::default()));

        assert!(!world.entity(child).contains::<Outline>());
    }

    #[test]
    fn late_mesh_under_outlined_root_inherits() {
        let mut world = world_with_observers();
        let root = world.spawn(outline()).id();
        let late_mesh = world.spawn((Mesh3d(Handle::default()), ChildOf(root))).id();

        let propagated = world
            .entity(late_mesh)
            .get::<Outline>()
            .expect("late child mesh inherits the root outline");
        assert_eq!(propagated.group_source, Some(root));
    }

    #[test]
    fn sync_updates_mid_tree_mesh_and_preserves_other_groups() {
        let mut world = world_with_observers();
        let root = world.spawn_empty().id();
        // Mid-tree mesh: has children of its own, so the old `Without<Children>`
        // write query could never reach it.
        let mid_mesh = world.spawn((Mesh3d(Handle::default()), ChildOf(root))).id();
        let leaf_mesh = world
            .spawn((Mesh3d(Handle::default()), ChildOf(mid_mesh)))
            .id();
        let barrier = world.spawn((OutlineBarrier, ChildOf(root))).id();
        let barrier_mesh = world
            .spawn((Mesh3d(Handle::default()), ChildOf(barrier)))
            .id();

        world.entity_mut(root).insert(outline());
        world.entity_mut(barrier).insert(outline());

        let updated_width = OUTLINE_WIDTH * 2.0;
        world
            .entity_mut(root)
            .get_mut::<Outline>()
            .expect("root outline was just inserted")
            .width = updated_width;
        world
            .run_system_once(sync_propagated_outlines)
            .expect("sync system runs");

        let synced_width = |entity: Entity| {
            world
                .entity(entity)
                .get::<Outline>()
                .expect("entity has an outline")
                .width
        };
        assert!((synced_width(mid_mesh) - updated_width).abs() < f32::EPSILON);
        assert!((synced_width(leaf_mesh) - updated_width).abs() < f32::EPSILON);
        // The barrier's own propagated outline keeps its original width.
        assert!((synced_width(barrier_mesh) - OUTLINE_WIDTH).abs() < f32::EPSILON);
    }

    #[test]
    fn removal_cleans_up_propagated_copies_only() {
        let mut world = world_with_observers();
        let root = world.spawn_empty().id();
        let mesh = world.spawn((Mesh3d(Handle::default()), ChildOf(root))).id();
        let barrier = world.spawn((OutlineBarrier, ChildOf(root))).id();
        let barrier_mesh = world
            .spawn((Mesh3d(Handle::default()), ChildOf(barrier)))
            .id();

        world.entity_mut(root).insert(outline());
        world.entity_mut(barrier).insert(outline());
        world.entity_mut(root).remove::<Outline>();

        assert!(!world.entity(mesh).contains::<Outline>());
        // The barrier group's outlines are keyed to the barrier, not the root.
        assert!(world.entity(barrier).contains::<Outline>());
        assert!(world.entity(barrier_mesh).contains::<Outline>());
    }
}
