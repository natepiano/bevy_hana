//! Authored cascade values and relationship-backed ECS propagation.

use std::marker::PhantomData;
use std::ops::Deref;

use bevy::ecs::system::EntityCommands;
use bevy::log::warn;
use bevy::platform::collections::HashSet;
use bevy::prelude::App;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::FromReflect;
use bevy::prelude::FromWorld;
use bevy::prelude::Insert;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::On;
use bevy::prelude::Plugin;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::ReflectFromWorld;
use bevy::prelude::ReflectResource;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use bevy::prelude::SystemSet;
use bevy::prelude::TypePath;
use bevy::prelude::Update;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::reflect::GetTypeRegistration;
use bevy::reflect::Typed;

/// Maximum number of entities inspected while resolving one cascade value.
pub const CASCADE_DEPTH_LIMIT: usize = 64;

/// An authored value that either inherits from a lower-precedence scope or
/// overrides it.
///
/// `Cascade<T>` is storage-independent when used in ordinary structs. When
/// inserted as an ECS component, [`CascadePlugin<T>`] maintains the matching
/// [`Resolved<T>`] cache.
#[derive(Clone, Component, Copy, Debug, Default, Eq, Hash, PartialEq, Reflect)]
#[reflect(Component)]
pub enum Cascade<T> {
    /// Inherit from the next lower-precedence scope.
    #[default]
    Inherit,
    /// Use this value instead of every lower-precedence scope.
    Override(T),
}

impl<T> Cascade<T> {
    /// Returns `true` when this value inherits from the cascade.
    #[must_use]
    pub const fn is_inherit(&self) -> bool { matches!(self, Self::Inherit) }

    /// Returns `true` when this value authors an override.
    #[must_use]
    pub const fn is_override(&self) -> bool { matches!(self, Self::Override(_)) }

    /// Borrows the override value while preserving the authored state.
    #[must_use]
    pub const fn as_ref(&self) -> Cascade<&T> {
        match self {
            Self::Inherit => Cascade::Inherit,
            Self::Override(value) => Cascade::Override(value),
        }
    }

    /// Mutably borrows the override value while preserving the authored state.
    #[must_use]
    pub const fn as_mut(&mut self) -> Cascade<&mut T> {
        match self {
            Self::Inherit => Cascade::Inherit,
            Self::Override(value) => Cascade::Override(value),
        }
    }

    /// Returns the override value by reference when one is authored.
    #[must_use]
    pub const fn as_override(&self) -> Option<&T> {
        match self {
            Self::Inherit => None,
            Self::Override(value) => Some(value),
        }
    }

    /// Applies `map_override` to an authored override or returns `inherited`
    /// when this value inherits.
    #[must_use]
    pub fn map_or<U>(self, inherited: U, map_override: impl FnOnce(T) -> U) -> U {
        match self {
            Self::Inherit => inherited,
            Self::Override(value) => map_override(value),
        }
    }

    /// Applies `map_override` to an authored override or calls `inherited`
    /// when this value inherits.
    #[must_use]
    pub fn map_or_else<U>(
        self,
        inherited: impl FnOnce() -> U,
        map_override: impl FnOnce(T) -> U,
    ) -> U {
        match self {
            Self::Inherit => inherited(),
            Self::Override(value) => map_override(value),
        }
    }

    /// Transforms an authored override while preserving inheritance.
    #[must_use]
    pub fn map<U>(self, map_override: impl FnOnce(T) -> U) -> Cascade<U> {
        match self {
            Self::Inherit => Cascade::Inherit,
            Self::Override(value) => Cascade::Override(map_override(value)),
        }
    }

    /// Resolves this authored state against one inherited value.
    #[must_use]
    pub fn resolve(self, inherited: T) -> T {
        match self {
            Self::Inherit => inherited,
            Self::Override(value) => value,
        }
    }

    /// Resolves this authored state by reference without cloning either value.
    #[must_use]
    pub const fn resolve_ref<'a>(&'a self, inherited: &'a T) -> &'a T {
        match self {
            Self::Inherit => inherited,
            Self::Override(value) => value,
        }
    }
}

impl<T: Copy> Cascade<&T> {
    /// Copies the borrowed override while preserving inheritance.
    #[must_use]
    pub const fn copied(self) -> Cascade<T> {
        match self {
            Self::Inherit => Cascade::Inherit,
            Self::Override(value) => Cascade::Override(*value),
        }
    }
}

impl<T: Clone> Cascade<&T> {
    /// Clones the borrowed override while preserving inheritance.
    #[must_use]
    pub fn cloned(self) -> Cascade<T> {
        match self {
            Self::Inherit => Cascade::Inherit,
            Self::Override(value) => Cascade::Override(value.clone()),
        }
    }
}

/// Resolves owned cascade layers from highest to lowest precedence.
///
/// The first [`Cascade::Override`] wins. `root` is returned when every layer
/// inherits.
#[must_use]
pub fn resolve_cascade<T>(layers: impl IntoIterator<Item = Cascade<T>>, root: T) -> T {
    layers
        .into_iter()
        .find_map(|layer| match layer {
            Cascade::Inherit => None,
            Cascade::Override(value) => Some(value),
        })
        .unwrap_or(root)
}

/// Resolves borrowed cascade layers from highest to lowest precedence.
///
/// The first [`Cascade::Override`] wins. `root` is returned when every layer
/// inherits.
#[must_use]
pub fn resolve_cascade_ref<'a, T>(
    layers: impl IntoIterator<Item = &'a Cascade<T>>,
    root: &'a T,
) -> &'a T {
    layers
        .into_iter()
        .find_map(Cascade::as_override)
        .unwrap_or(root)
}

/// Value contract for an ECS cascade attribute.
///
/// Downstream crates opt in by deriving [`Reflect`] on a cloneable value type.
pub trait CascadeAttribute:
    Clone + PartialEq + Send + Sync + FromReflect + TypePath + Typed + GetTypeRegistration + 'static
{
}

impl<A> CascadeAttribute for A where
    A: Clone
        + PartialEq
        + Send
        + Sync
        + FromReflect
        + TypePath
        + Typed
        + GetTypeRegistration
        + 'static
{
}

/// This entity obtains inherited cascade values from `target`.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq, Reflect)]
#[component(immutable)]
#[reflect(Component, PartialEq, Debug, FromWorld, Clone)]
#[relationship(relationship_target = CascadeChildren, allow_self_referential)]
pub struct CascadeFrom {
    #[relationship]
    #[entities]
    #[reflect(ignore, default = "placeholder_entity")]
    target: Entity,
}

impl CascadeFrom {
    /// Connects this entity's inherited cascade values to `target`.
    #[must_use]
    pub const fn new(target: Entity) -> Self { Self { target } }

    /// Entity consulted after this entity's authored value.
    #[must_use]
    pub const fn target(&self) -> Entity { self.target }
}

impl FromWorld for CascadeFrom {
    fn from_world(_: &mut World) -> Self { Self::new(Entity::PLACEHOLDER) }
}

const fn placeholder_entity() -> Entity { Entity::PLACEHOLDER }

/// Entities whose [`CascadeFrom`] relationship targets this entity.
///
/// Bevy maintains this collection. It does not use linked despawn.
#[derive(Component, Debug, Default, Eq, PartialEq, Reflect)]
#[reflect(Component, FromWorld, Default)]
#[relationship_target(relationship = CascadeFrom)]
pub struct CascadeChildren(Vec<Entity>);

impl Deref for CascadeChildren {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}

/// Root value used when a cascade walk finds no authored override.
#[derive(Clone, Debug, Reflect, Resource)]
#[reflect(Resource)]
pub struct CascadeDefault<A: CascadeAttribute>(pub A);

#[derive(Resource)]
struct CascadeCombine<A: CascadeAttribute>(fn(A, &A) -> A);

/// Resource holding attribute `A`'s root value.
///
/// [`CascadeDefault<A>`] implements this for every attribute. A crate with its own
/// root resource type implements this for that type and passes it to
/// [`CascadePlugin`].
pub trait CascadeRootResource<A: CascadeAttribute>: Resource + GetTypeRegistration {
    /// Returns the root value.
    fn root(&self) -> A;

    /// Builds this resource from a root value.
    fn from_root(root: A) -> Self;
}

impl<A: CascadeAttribute> CascadeRootResource<A> for CascadeDefault<A> {
    fn root(&self) -> A { self.0.clone() }

    fn from_root(root: A) -> Self { Self(root) }
}

/// Cached effective value for a participating cascade attribute.
#[derive(Clone, Component, Debug, Reflect)]
#[reflect(Component)]
pub struct Resolved<A: CascadeAttribute>(pub A);

impl<A: CascadeAttribute> Resolved<A> {
    /// Borrows the effective value.
    #[must_use]
    pub const fn value(&self) -> &A { &self.0 }
}

/// System ordering for shared cascade propagation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum CascadeSet {
    /// Updates [`Resolved<A>`] after authored values and relationships change.
    Propagate,
}

/// Installs propagation for one cascade attribute.
///
/// `R` holds the attribute's root value. [`CascadePlugin::new`] uses
/// [`CascadeDefault<A>`]; [`CascadePlugin::with_root_resource`] swaps in a
/// caller-owned resource type.
pub struct CascadePlugin<A: CascadeAttribute, R: CascadeRootResource<A> = CascadeDefault<A>> {
    root:          A,
    combine:       fn(A, &A) -> A,
    root_resource: PhantomData<R>,
}

impl<A: CascadeAttribute> CascadePlugin<A, CascadeDefault<A>> {
    /// Creates an attribute plugin with `root` as its default value.
    #[must_use]
    pub const fn new(root: A) -> Self {
        Self {
            root,
            combine: replace_cascade_value::<A>,
            root_resource: PhantomData,
        }
    }
}

impl<A: CascadeAttribute, R: CascadeRootResource<A>> CascadePlugin<A, R> {
    /// Sets the function that combines a lower authored value with the value
    /// inherited from the next scope.
    #[must_use]
    pub const fn with_combine(mut self, combine: fn(A, &A) -> A) -> Self {
        self.combine = combine;
        self
    }

    /// Moves the same root value into a caller-supplied root resource type.
    #[must_use]
    pub fn with_root_resource<T: CascadeRootResource<A>>(self) -> CascadePlugin<A, T> {
        CascadePlugin {
            root:          self.root,
            combine:       self.combine,
            root_resource: PhantomData,
        }
    }
}

impl<A: CascadeAttribute, R: CascadeRootResource<A>> Default for CascadePlugin<A, R>
where
    CascadeDefault<A>: Default,
{
    fn default() -> Self {
        Self {
            root:          CascadeDefault::<A>::default().0,
            combine:       replace_cascade_value::<A>,
            root_resource: PhantomData,
        }
    }
}

impl<A: CascadeAttribute, R: CascadeRootResource<A>> Plugin for CascadePlugin<A, R> {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<R>() {
            app.insert_resource(R::from_root(self.root.clone()));
        }
        if !app.world().contains_resource::<CascadeCombine<A>>() {
            app.insert_resource(CascadeCombine(self.combine));
        }
        app.register_type::<Cascade<A>>()
            .register_type::<Resolved<A>>()
            .register_type::<R>()
            .add_observer(resolve_inserted_cascade::<A, R>)
            .add_systems(
                Update,
                propagate_cascade::<A, R>.in_set(CascadeSet::Propagate),
            );
    }
}

/// Generic authored-value commands for cascade participants.
pub trait CascadeEntityCommandsExt {
    /// Starts or updates participation in attribute `A` with `authored`.
    fn set_cascade<A: CascadeAttribute>(&mut self, authored: Cascade<A>) -> &mut Self;

    /// Starts or updates participation in attribute `A` with a local override.
    fn override_cascade<A: CascadeAttribute>(&mut self, value: A) -> &mut Self;

    /// Starts or keeps participation in attribute `A` while inheriting.
    fn inherit_cascade<A: CascadeAttribute>(&mut self) -> &mut Self;

    /// Stops participation in attribute `A` and removes its resolved cache.
    fn remove_cascade<A: CascadeAttribute>(&mut self) -> &mut Self;
}

impl CascadeEntityCommandsExt for EntityCommands<'_> {
    fn set_cascade<A: CascadeAttribute>(&mut self, authored: Cascade<A>) -> &mut Self {
        self.insert(authored)
    }

    fn override_cascade<A: CascadeAttribute>(&mut self, value: A) -> &mut Self {
        self.set_cascade(Cascade::Override(value))
    }

    fn inherit_cascade<A: CascadeAttribute>(&mut self) -> &mut Self {
        self.set_cascade(Cascade::<A>::Inherit)
    }

    fn remove_cascade<A: CascadeAttribute>(&mut self) -> &mut Self {
        self.remove::<(Cascade<A>, Resolved<A>)>()
    }
}

/// Reads the cached value for a participating entity.
#[must_use]
pub fn resolved_cascade<A: CascadeAttribute>(world: &World, entity: Entity) -> Option<&A> {
    world.get::<Resolved<A>>(entity).map(Resolved::value)
}

/// Resolves an entity directly from current authored components and resources.
///
/// Returns `None` when [`CascadePlugin`] has not installed the `R` resource.
#[must_use]
pub fn resolve_entity_cascade<A: CascadeAttribute, R: CascadeRootResource<A>>(
    world: &World,
    entity: Entity,
) -> Option<A> {
    let root = world.get_resource::<R>()?.root();
    let combine = world
        .get_resource::<CascadeCombine<A>>()
        .map_or(replace_cascade_value::<A> as fn(A, &A) -> A, |resource| {
            resource.0
        });
    Some(resolve_from_world::<A>(world, entity, root, combine))
}

/// Seeds or refreshes one participant after all commands queued alongside the
/// authored insert have applied.
fn resolve_inserted_cascade<A: CascadeAttribute, R: CascadeRootResource<A>>(
    inserted: On<Insert, Cascade<A>>,
    mut commands: Commands,
) {
    let entity = inserted.event_target();
    commands.queue(move |world: &mut World| {
        if world.get::<Cascade<A>>(entity).is_none() {
            return;
        }
        let Some(value) = resolve_entity_cascade::<A, R>(world, entity) else {
            return;
        };
        if world
            .get::<Resolved<A>>(entity)
            .is_some_and(|current| current.0 == value)
        {
            return;
        }
        world.entity_mut(entity).insert(Resolved(value));
    });
}

fn propagate_cascade<A: CascadeAttribute, R: CascadeRootResource<A>>(
    default: Res<R>,
    combine: Res<CascadeCombine<A>>,
    authored: Query<&Cascade<A>>,
    relationships: Query<&CascadeFrom>,
    children: Query<&CascadeChildren>,
    participants: Query<Entity, With<Cascade<A>>>,
    resolved: Query<&Resolved<A>>,
    changed_authored: Query<Entity, Changed<Cascade<A>>>,
    changed_relationships: Query<Entity, Changed<CascadeFrom>>,
    mut removed_authored: RemovedComponents<Cascade<A>>,
    mut removed_relationships: RemovedComponents<CascadeFrom>,
    mut commands: Commands,
) {
    let removed_authored: Vec<Entity> = removed_authored.read().collect();
    let removed_relationships: Vec<Entity> = removed_relationships.read().collect();
    let mut dirty = HashSet::new();

    if default.is_changed() {
        dirty.extend(participants.iter());
    }

    for root in changed_authored
        .iter()
        .chain(changed_relationships.iter())
        .chain(removed_authored.iter().copied())
        .chain(removed_relationships.iter().copied())
    {
        collect_subtree(root, &children, &mut dirty);
    }

    for entity in dirty {
        if authored.get(entity).is_err() {
            if resolved.get(entity).is_ok() {
                commands.entity(entity).remove::<Resolved<A>>();
            }
            continue;
        }

        let value =
            resolve_from_queries(entity, &authored, &relationships, default.root(), combine.0);
        if resolved.get(entity).is_ok_and(|current| current.0 == value) {
            continue;
        }
        commands.entity(entity).insert(Resolved(value));
    }
}

fn collect_subtree(root: Entity, children: &Query<&CascadeChildren>, dirty: &mut HashSet<Entity>) {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if !dirty.insert(entity) {
            continue;
        }
        if let Ok(child_list) = children.get(entity) {
            stack.extend(child_list.iter());
        }
    }
}

fn resolve_from_queries<A: CascadeAttribute>(
    entity: Entity,
    authored: &Query<&Cascade<A>>,
    relationships: &Query<&CascadeFrom>,
    root: A,
    combine: fn(A, &A) -> A,
) -> A {
    let mut visited = HashSet::new();
    let mut current = entity;
    let mut accumulated = None;

    for _ in 0..CASCADE_DEPTH_LIMIT {
        if !visited.insert(current) {
            warn!(
                "cascade relationship cycle reached from {entity:?}; resolving accumulated overrides with the root default"
            );
            return resolve_accumulated(accumulated, root, combine);
        }
        if let Ok(Cascade::Override(value)) = authored.get(current) {
            accumulated =
                Some(accumulated.map_or_else(|| value.clone(), |lower| combine(lower, value)));
        }
        let Ok(relationship) = relationships.get(current) else {
            return resolve_accumulated(accumulated, root, combine);
        };
        current = relationship.target();
    }

    warn!(
        "cascade depth limit reached from {entity:?}; resolving accumulated overrides with the root default"
    );
    resolve_accumulated(accumulated, root, combine)
}

fn resolve_from_world<A: CascadeAttribute>(
    world: &World,
    entity: Entity,
    root: A,
    combine: fn(A, &A) -> A,
) -> A {
    let mut visited = HashSet::new();
    let mut current = entity;
    let mut accumulated = None;

    for _ in 0..CASCADE_DEPTH_LIMIT {
        if !visited.insert(current) {
            warn!(
                "cascade relationship cycle reached from {entity:?}; resolving accumulated overrides with the root default"
            );
            return resolve_accumulated(accumulated, root, combine);
        }
        if let Some(Cascade::Override(value)) = world.get::<Cascade<A>>(current) {
            accumulated =
                Some(accumulated.map_or_else(|| value.clone(), |lower| combine(lower, value)));
        }
        let Some(relationship) = world.get::<CascadeFrom>(current) else {
            return resolve_accumulated(accumulated, root, combine);
        };
        current = relationship.target();
    }

    warn!(
        "cascade depth limit reached from {entity:?}; resolving accumulated overrides with the root default"
    );
    resolve_accumulated(accumulated, root, combine)
}

fn resolve_accumulated<A: CascadeAttribute>(
    accumulated: Option<A>,
    root: A,
    combine: fn(A, &A) -> A,
) -> A {
    match accumulated {
        Some(lower) => combine(lower, &root),
        None => root,
    }
}

const fn replace_cascade_value<A>(lower: A, _: &A) -> A { lower }

#[cfg(test)]
mod tests {
    use bevy::prelude::ChildOf;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::ResMut;

    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq, Reflect)]
    struct TestValue(u32);

    #[derive(Default, Resource)]
    struct ResolvedWrites(usize);

    fn count_changes(
        changed: Query<(), Changed<Resolved<TestValue>>>,
        mut writes: ResMut<ResolvedWrites>,
    ) {
        writes.0 += changed.iter().count();
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(CascadePlugin::new(TestValue(0)))
            .init_resource::<ResolvedWrites>()
            .add_systems(Update, count_changes.after(CascadeSet::Propagate));
        app
    }

    fn read(app: &App, entity: Entity) -> Option<TestValue> {
        resolved_cascade(app.world(), entity).cloned()
    }

    fn add_test_values(lower: TestValue, higher: &TestValue) -> TestValue {
        TestValue(lower.0 + higher.0)
    }

    #[test]
    fn pure_cascade_resolution_preserves_existing_api() {
        let layers = [Cascade::Inherit, Cascade::Override(2), Cascade::Override(1)];

        assert_eq!(resolve_cascade(layers, 0), 2);
    }

    #[test]
    fn added_participant_seeds_root_default() {
        let mut app = test_app();
        let entity = app.world_mut().spawn(Cascade::<TestValue>::Inherit).id();

        app.update();

        assert_eq!(read(&app, entity), Some(TestValue(0)));
    }

    #[test]
    fn inserted_participant_seeds_before_update() {
        let mut app = test_app();
        let entity = app.world_mut().spawn(Cascade::Override(TestValue(4))).id();

        assert_eq!(read(&app, entity), Some(TestValue(4)));
    }

    #[test]
    fn child_of_alone_does_not_inherit() {
        let mut app = test_app();
        let parent = app.world_mut().spawn(Cascade::Override(TestValue(1))).id();
        let child = app
            .world_mut()
            .spawn((Cascade::<TestValue>::Inherit, ChildOf(parent)))
            .id();

        app.update();

        assert_eq!(read(&app, child), Some(TestValue(0)));
    }

    #[test]
    fn explicit_relationship_inherits_through_transparent_entity() {
        let mut app = test_app();
        let root = app.world_mut().spawn(Cascade::Override(TestValue(1))).id();
        let transparent = app.world_mut().spawn(CascadeFrom::new(root)).id();
        let child = app
            .world_mut()
            .spawn((Cascade::<TestValue>::Inherit, CascadeFrom::new(transparent)))
            .id();

        app.update();

        assert_eq!(read(&app, child), Some(TestValue(1)));
    }

    #[test]
    fn local_override_wins_and_updates_descendants() {
        let mut app = test_app();
        let parent = app.world_mut().spawn(Cascade::Override(TestValue(1))).id();
        let child = app
            .world_mut()
            .spawn((Cascade::<TestValue>::Inherit, CascadeFrom::new(parent)))
            .id();
        app.update();

        app.world_mut()
            .entity_mut(parent)
            .insert(Cascade::Override(TestValue(2)));
        app.update();

        assert_eq!(read(&app, parent), Some(TestValue(2)));
        assert_eq!(read(&app, child), Some(TestValue(2)));
    }

    #[test]
    fn default_combine_keeps_the_first_override_across_widget_panel_and_root() {
        let mut app = test_app();
        let panel = app.world_mut().spawn(Cascade::Override(TestValue(1))).id();
        let widget = app
            .world_mut()
            .spawn((Cascade::Override(TestValue(2)), CascadeFrom::new(panel)))
            .id();
        let root_only = app.world_mut().spawn(Cascade::<TestValue>::Inherit).id();

        app.update();

        assert_eq!(read(&app, widget), Some(TestValue(2)));
        assert_eq!(read(&app, root_only), Some(TestValue(0)));
    }

    #[test]
    fn configured_combine_folds_widget_panel_and_root() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(CascadePlugin::new(TestValue(1)).with_combine(add_test_values));
        let panel = app.world_mut().spawn(Cascade::Override(TestValue(2))).id();
        let widget = app
            .world_mut()
            .spawn((Cascade::Override(TestValue(4)), CascadeFrom::new(panel)))
            .id();

        app.update();

        assert_eq!(read(&app, widget), Some(TestValue(7)));
    }

    #[test]
    fn root_default_change_updates_inheriting_participants() {
        let mut app = test_app();
        let entity = app.world_mut().spawn(Cascade::<TestValue>::Inherit).id();
        app.update();

        app.world_mut()
            .resource_mut::<CascadeDefault<TestValue>>()
            .0 = TestValue(3);
        app.update();

        assert_eq!(read(&app, entity), Some(TestValue(3)));
    }

    #[test]
    fn relationship_retarget_and_removal_reresolve_subtree() {
        let mut app = test_app();
        let first = app.world_mut().spawn(Cascade::Override(TestValue(1))).id();
        let second = app.world_mut().spawn(Cascade::Override(TestValue(2))).id();
        let child = app
            .world_mut()
            .spawn((Cascade::<TestValue>::Inherit, CascadeFrom::new(first)))
            .id();
        app.update();

        app.world_mut()
            .entity_mut(child)
            .insert(CascadeFrom::new(second));
        app.update();
        assert_eq!(read(&app, child), Some(TestValue(2)));

        app.world_mut().entity_mut(child).remove::<CascadeFrom>();
        app.update();
        assert_eq!(read(&app, child), Some(TestValue(0)));
    }

    #[test]
    fn authored_removal_stops_participation_and_updates_descendants() {
        let mut app = test_app();
        let parent = app.world_mut().spawn(Cascade::Override(TestValue(1))).id();
        let child = app
            .world_mut()
            .spawn((Cascade::<TestValue>::Inherit, CascadeFrom::new(parent)))
            .id();
        app.update();

        app.world_mut()
            .entity_mut(parent)
            .remove::<Cascade<TestValue>>();
        app.update();

        assert_eq!(read(&app, parent), None);
        assert_eq!(read(&app, child), Some(TestValue(0)));
    }

    #[test]
    fn unchanged_values_do_not_mark_resolved_as_changed() {
        let mut app = test_app();
        let entity = app.world_mut().spawn(Cascade::Override(TestValue(1))).id();
        app.update();
        app.world_mut().resource_mut::<ResolvedWrites>().0 = 0;

        app.world_mut()
            .entity_mut(entity)
            .insert(Cascade::Override(TestValue(1)));
        app.update();

        assert_eq!(app.world().resource::<ResolvedWrites>().0, 0);
    }

    #[test]
    fn cycle_uses_root_default() {
        let mut app = test_app();
        let first = app.world_mut().spawn(Cascade::<TestValue>::Inherit).id();
        let second = app.world_mut().spawn(Cascade::<TestValue>::Inherit).id();
        app.world_mut()
            .entity_mut(first)
            .insert(CascadeFrom::new(second));
        app.world_mut()
            .entity_mut(second)
            .insert(CascadeFrom::new(first));

        app.update();

        assert_eq!(read(&app, first), Some(TestValue(0)));
        assert_eq!(read(&app, second), Some(TestValue(0)));
    }

    #[test]
    fn excessive_depth_uses_root_default() {
        let mut app = test_app();
        let override_root = app.world_mut().spawn(Cascade::Override(TestValue(1))).id();
        let mut target = override_root;
        for _ in 0..CASCADE_DEPTH_LIMIT {
            target = app.world_mut().spawn(CascadeFrom::new(target)).id();
        }
        let participant = app
            .world_mut()
            .spawn((Cascade::<TestValue>::Inherit, CascadeFrom::new(target)))
            .id();

        app.update();

        assert_eq!(read(&app, participant), Some(TestValue(0)));
    }
}
