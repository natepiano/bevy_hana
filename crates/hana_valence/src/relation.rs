use std::ops::Deref;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Component;
use bevy_ecs::prelude::FromWorld;
use bevy_ecs::prelude::ReflectComponent;
use bevy_ecs::prelude::ReflectFromWorld;
use bevy_ecs::prelude::World;
use bevy_kana::Displacement;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;

use crate::AnchorSite;

/// Relationship source that pins one entity anchor to another entity anchor.
///
/// `AnchoredTo` is immutable, so retargeting is a full component replacement:
/// Bevy runs the relationship remove and insert hooks again, and
/// [`AnchoredHere`] is updated by those hooks. Use [`AnchoredTo::retargeted`]
/// to build the full replacement value instead of mutating only `target`.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[component(immutable)]
#[reflect(PartialEq, Debug, FromWorld, Clone, opaque)]
#[relationship(relationship_target = AnchoredHere)]
pub struct AnchoredTo {
    #[relationship]
    #[entities]
    #[reflect(ignore)]
    target:        Entity,
    source_anchor: AnchorSite,
    target_anchor: AnchorSite,
    offset:        Displacement,
}

impl AnchoredTo {
    /// Creates a relationship from the source entity to `target`.
    #[must_use]
    pub const fn new(target: Entity, source_anchor: AnchorSite, target_anchor: AnchorSite) -> Self {
        Self {
            target,
            source_anchor,
            target_anchor,
            offset: Displacement::new(0.0, 0.0, 0.0),
        }
    }

    /// Returns a replacement relation with an authored target-frame offset.
    ///
    /// The displacement is added after the target frame is resolved and is not
    /// rotated by the source pose. Replacing the relation through this builder
    /// is also the relationship-maintained retarget path.
    #[must_use]
    pub const fn with_offset(mut self, offset: Displacement) -> Self {
        self.offset = offset;
        self
    }

    /// Target entity.
    #[must_use]
    pub const fn target(&self) -> Entity { self.target }

    /// Returns the site on the source entity that is pinned by this relation.
    #[must_use]
    pub const fn source_anchor(&self) -> AnchorSite { self.source_anchor }

    /// Returns the site on the target entity that receives the source site.
    #[must_use]
    pub const fn target_anchor(&self) -> AnchorSite { self.target_anchor }

    /// Returns the authored target-frame displacement.
    #[must_use]
    pub const fn offset(&self) -> Displacement { self.offset }

    /// Returns a copy that points at `target`.
    #[must_use]
    pub const fn retargeted(mut self, target: Entity) -> Self {
        self.target = target;
        self
    }
}

impl FromWorld for AnchoredTo {
    fn from_world(_: &mut World) -> Self {
        Self::new(Entity::PLACEHOLDER, AnchorSite::Center, AnchorSite::Center)
    }
}

/// Reverse relationship target: entities anchored to this entity.
///
/// The stored order is insertion order, which gives resolver and arrangement
/// systems a deterministic iteration order.
#[derive(Component, Debug, Default, Reflect)]
#[reflect(FromWorld, Default)]
#[relationship_target(relationship = AnchoredTo)]
pub struct AnchoredHere(Vec<Entity>);

impl AnchoredHere {
    /// Iterates over source entities.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ { self.0.iter().copied() }

    /// Number of source entities currently pointing here.
    #[must_use]
    pub const fn len(&self) -> usize { self.0.len() }

    /// Whether no source entities point here.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.0.is_empty() }
}

impl Deref for AnchoredHere {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}

/// Resolver-owned per-frame offset override.
///
/// Resolver systems prefer `ResolvedAnchorOffset` over [`AnchoredTo::offset`]
/// when both components are present on the source entity.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Component, Default, opaque)]
pub struct ResolvedAnchorOffset(Displacement);

impl ResolvedAnchorOffset {
    /// Returns the displacement used instead of [`AnchoredTo::offset`] this frame.
    #[must_use]
    pub const fn offset(self) -> Displacement { self.0 }
}

impl From<Displacement> for ResolvedAnchorOffset {
    fn from(offset: Displacement) -> Self { Self(offset) }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy_ecs::entity::Entity;
    use bevy_ecs::prelude::AppTypeRegistry;
    use bevy_ecs::prelude::ReflectComponent;
    use bevy_ecs::prelude::Schedule;
    use bevy_ecs::prelude::World;
    use bevy_kana::Displacement;
    use bevy_math::Vec3;
    use bevy_reflect::TypeRegistry;

    use super::AnchoredHere;
    use super::AnchoredTo;
    use super::ResolvedAnchorOffset;
    use crate::AnchorPose;
    use crate::AnchorSite;
    use crate::Hinge;
    use crate::ResolvedAnchorGeometry;
    use crate::ResolvedAnchorWorld;

    fn reverse_targets(world: &World, target: Entity) -> Vec<Entity> {
        world
            .get::<AnchoredHere>(target)
            .map(AnchoredHere::iter)
            .map(Iterator::collect)
            .unwrap_or_default()
    }

    fn exercise_relationship_hooks(world: &mut World) {
        let target_a = world.spawn_empty().id();
        let target_b = world.spawn_empty().id();
        let source = world.spawn_empty().id();

        world.entity_mut(source).insert(AnchoredTo::new(
            target_a,
            AnchorSite::Vertex(0),
            AnchorSite::Center,
        ));
        assert_eq!(reverse_targets(world, target_a), vec![source]);
        assert!(reverse_targets(world, target_b).is_empty());

        world.entity_mut(source).insert(
            AnchoredTo::new(target_a, AnchorSite::EdgeMidpoint(0), AnchorSite::Center)
                .retargeted(target_b)
                .with_offset(Displacement::from(Vec3::X)),
        );
        assert!(reverse_targets(world, target_a).is_empty());
        assert_eq!(reverse_targets(world, target_b), vec![source]);

        world.entity_mut(source).remove::<AnchoredTo>();
        assert!(reverse_targets(world, target_b).is_empty());
    }

    #[test]
    fn insert_replace_and_remove_update_reverse_index() {
        let mut world = World::new();
        let mut schedule = Schedule::default();
        schedule.add_systems(exercise_relationship_hooks);

        schedule.run(&mut world);
    }

    #[test]
    fn anchor_types_are_registered_with_expected_reflect_component_data() {
        let mut world = World::new();
        world.insert_resource(AppTypeRegistry::default());
        {
            let registry = world.resource::<AppTypeRegistry>();
            let mut registry = registry.write();
            registry.register::<AnchoredTo>();
            registry.register::<AnchoredHere>();
            registry.register::<ResolvedAnchorGeometry>();
            registry.register::<ResolvedAnchorOffset>();
            registry.register::<AnchorPose>();
            registry.register::<ResolvedAnchorWorld>();
            registry.register::<Hinge>();
        }
        let (
            source_registered,
            reverse_registered,
            source_has_reflect_component,
            reverse_has_reflect_component,
            geometry_has_reflect_component,
            offset_has_reflect_component,
            pose_has_reflect_component,
            world_has_reflect_component,
            hinge_has_reflect_component,
        ) = {
            let registry = world.resource::<AppTypeRegistry>().read();
            (
                registry.get(TypeId::of::<AnchoredTo>()).is_some(),
                registry.get(TypeId::of::<AnchoredHere>()).is_some(),
                has_reflect_component::<AnchoredTo>(&registry),
                has_reflect_component::<AnchoredHere>(&registry),
                has_reflect_component::<ResolvedAnchorGeometry>(&registry),
                has_reflect_component::<ResolvedAnchorOffset>(&registry),
                has_reflect_component::<AnchorPose>(&registry),
                has_reflect_component::<ResolvedAnchorWorld>(&registry),
                has_reflect_component::<Hinge>(&registry),
            )
        };

        assert!(source_registered);
        assert!(reverse_registered);
        assert!(!source_has_reflect_component);
        assert!(!reverse_has_reflect_component);
        assert!(geometry_has_reflect_component);
        assert!(offset_has_reflect_component);
        assert!(pose_has_reflect_component);
        assert!(world_has_reflect_component);
        assert!(hinge_has_reflect_component);
    }

    fn has_reflect_component<T: 'static>(registry: &TypeRegistry) -> bool {
        registry
            .get(TypeId::of::<T>())
            .is_some_and(|registration| registration.data::<ReflectComponent>().is_some())
    }
}
