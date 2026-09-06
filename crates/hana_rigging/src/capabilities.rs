use std::any::TypeId;
use std::collections::HashMap;
use std::collections::HashSet;

use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::world::EntityRef;
use bevy::ecs::world::EntityWorldMut;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::reflect::PartialReflect;
use bevy::reflect::TypePath;
use bevy::reflect::TypeRegistry;
use thiserror::Error;

use crate::CapabilityProjectionFailure;
use crate::ReporterId;

type CapabilityEquality = fn(&dyn Reflect, &dyn Reflect) -> bool;

/// Reflected component type one endpoint driver requires from a reporter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapabilityRequirement {
    type_id:   TypeId,
    type_path: &'static str,
}

impl CapabilityRequirement {
    /// Name one concrete capability type without constructing a value.
    #[must_use]
    pub(crate) fn of<C>() -> Self
    where
        C: Component + Reflect + TypePath,
    {
        Self {
            type_id:   TypeId::of::<C>(),
            type_path: <C as TypePath>::type_path(),
        }
    }

    /// Return the reflected path of the required capability type.
    #[must_use]
    pub(crate) const fn type_path(&self) -> &'static str { self.type_path }

    pub(crate) const fn type_id(&self) -> TypeId { self.type_id }
}

/// Reporter record that supplied a retained capability declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CapabilitySource {
    pub(crate) reporter:     ReporterId,
    pub(crate) record_index: usize,
}

/// Whether a capability declaration still belongs to a reporter-owned scan or to an accepted set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CapabilitySourceState {
    /// The reporter built this declaration, but the kernel has not accepted its set.
    #[default]
    PendingAcceptance,
    /// The kernel retained this declaration under a registry-issued reporter and record index.
    Retained(CapabilitySource),
}

/// One erased capability value and the typed equality function installed by `Capabilities::add`.
pub(crate) struct CapabilityDeclaration {
    value:  Box<dyn Reflect>,
    equals: CapabilityEquality,
}

/// One retained declaration with the reporter whose accepted set owns it.
#[derive(Clone, Copy)]
pub(crate) struct CapabilityProjectionDeclaration<'a> {
    declaration: &'a CapabilityDeclaration,
    reporter:    ReporterId,
}

/// A projection failure attributed to the reporter outcome that supplied the declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReporterCapabilityProjectionFailure {
    reporter: ReporterId,
    failure:  CapabilityProjectionFailure,
}

/// One projected component type and the reporters whose current declarations contribute it.
#[derive(Clone, PartialEq, Eq)]
struct ProjectedCapabilityType {
    type_path: String,
    reporters: HashSet<ReporterId>,
}

/// Capability component types the kernel currently projects on one device entity.
///
/// The retained type path lets a later complete set retract a type even though the declaration
/// value that originally supplied it is no longer present in any reporter registry.
#[derive(Clone, Component, Default, PartialEq, Eq)]
pub(crate) struct ProjectedCapabilityTypes {
    types: HashMap<TypeId, ProjectedCapabilityType>,
}

impl ProjectedCapabilityTypes {
    pub(crate) fn includes(
        &self,
        reporter: ReporterId,
        requirement: &CapabilityRequirement,
    ) -> bool {
        self.types
            .get(&requirement.type_id())
            .is_some_and(|projected| projected.reporters.contains(&reporter))
    }
}

impl CapabilityDeclaration {
    pub(crate) fn value(&self) -> &dyn Reflect { self.value.as_ref() }

    pub(crate) fn equals(&self, other: &dyn Reflect) -> bool {
        (self.equals)(self.value.as_ref(), other)
    }
}

impl<'a> CapabilityProjectionDeclaration<'a> {
    pub(crate) const fn new(declaration: &'a CapabilityDeclaration, reporter: ReporterId) -> Self {
        Self {
            declaration,
            reporter,
        }
    }

    pub(crate) const fn declaration(self) -> &'a CapabilityDeclaration { self.declaration }

    const fn reporter(self) -> ReporterId { self.reporter }
}

impl ReporterCapabilityProjectionFailure {
    const fn new(reporter: ReporterId, failure: CapabilityProjectionFailure) -> Self {
        Self { reporter, failure }
    }

    pub(crate) fn into_parts(self) -> (ReporterId, CapabilityProjectionFailure) {
        (self.reporter, self.failure)
    }

    pub(crate) const fn reporter(&self) -> ReporterId { self.reporter }

    pub(crate) const fn failure(&self) -> &CapabilityProjectionFailure { &self.failure }
}

/// Erased capability components a provider reports for one [`DeviceRecord`](crate::DeviceRecord).
///
/// Providers can retain private capability types while reporting them to the kernel because
/// [`Capabilities`] stores [`Reflect`] trait objects instead of a kernel-owned device-class enum.
/// Reconciliation later compares overlapping component types and inserts their values on the
/// resolved device entity.
#[derive(Default)]
pub struct Capabilities {
    declarations: Vec<CapabilityDeclaration>,
    source:       CapabilitySourceState,
}

impl Capabilities {
    /// Create an empty declaration for a device that currently exposes no capability components.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Add one reflected capability component to this provider's declaration.
    ///
    /// The value may use a private provider type. Accepted-scan ingestion drops it and reports the
    /// failure in reporter health if its owner did not register reflected-component metadata.
    pub fn add<C>(&mut self, capability: C)
    where
        C: Component + Reflect + PartialEq,
    {
        self.declarations.push(CapabilityDeclaration {
            value:  Box::new(capability),
            equals: typed_capability_equality::<C>,
        });
    }

    /// Add one reflected capability component and return this declaration for builder-style setup.
    #[must_use]
    pub fn with<C>(mut self, capability: C) -> Self
    where
        C: Component + Reflect + PartialEq,
    {
        self.add(capability);
        self
    }

    /// Borrow every declared capability component so reconciliation can compare the declarations
    /// of two reporters that describe the same device.
    ///
    /// References rather than values: `Box<dyn Reflect>` is not clonable in Bevy 0.19, and copying
    /// a declaration out of a reporter's retained set would destroy evidence a reporter that did
    /// not re-scan this frame still needs.
    pub(crate) fn declarations(&self) -> impl Iterator<Item = &CapabilityDeclaration> {
        self.declarations.iter()
    }

    pub(crate) const fn source(&self) -> CapabilitySourceState { self.source }

    /// Retain only declarations whose concrete type has Bevy reflected-component metadata.
    pub(crate) fn retain_projectable(
        &mut self,
        source: CapabilitySource,
        type_registry: &TypeRegistry,
    ) -> Vec<CapabilityProjectionFailure> {
        self.source = CapabilitySourceState::Retained(source);
        let mut failures = Vec::new();
        self.declarations.retain(|declaration| {
            match reflect_component_for(declaration.value().as_partial_reflect(), type_registry) {
                Ok(_) => true,
                Err(error) => {
                    failures.push(CapabilityProjectionFailure::ReflectComponentNotRegistered {
                        type_path: error.into_type_path(),
                    });
                    false
                },
            }
        });
        failures
    }

    /// Drop every declaration when the world has no application type registry.
    pub(crate) fn retain_without_type_registry(
        &mut self,
        source: CapabilitySource,
    ) -> Vec<CapabilityProjectionFailure> {
        self.source = CapabilitySourceState::Retained(source);
        self.declarations
            .drain(..)
            .map(
                |declaration| CapabilityProjectionFailure::ApplicationTypeRegistryUnavailable {
                    affected_type_path: declaration.value().reflect_type_path().to_owned(),
                },
            )
            .collect()
    }
}

fn typed_capability_equality<C>(left: &dyn Reflect, right: &dyn Reflect) -> bool
where
    C: Component + Reflect + PartialEq,
{
    left.downcast_ref::<C>()
        .zip(right.downcast_ref::<C>())
        .is_some_and(|(left, right)| left == right)
}

/// Insert every declaration in one erased group onto `entity`, or insert none of them.
///
/// The declarations it inserts are the union of what several reporters declared, and they stay
/// borrowed from the registries that own them because `Box<dyn Reflect>` cannot be cloned into one
/// merged declaration.
///
/// # Errors
///
/// Returns [`CapabilityAttachError`] when a declaration is not registered as a Bevy reflected
/// component in `type_registry`, leaving `entity` exactly as it was.
#[cfg(test)]
fn attach_declarations<'a>(
    entity: &mut EntityWorldMut,
    type_registry: &TypeRegistry,
    declarations: impl IntoIterator<Item = &'a CapabilityDeclaration>,
) -> Result<(), CapabilityAttachError> {
    // Every declaration is resolved before the first insert, so a group that names one unregistered
    // type leaves the entity exactly as it was rather than half populated with whichever
    // capabilities happened to sort earlier.
    let resolved = declarations
        .into_iter()
        .map(|declaration| {
            reflect_component_for(declaration.value().as_partial_reflect(), type_registry)
                .map(|reflect_component| (declaration, reflect_component))
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (declaration, reflect_component) in resolved {
        // `ReflectComponent::insert` writes unconditionally and Bevy's `Changed<C>` fires on any
        // write, so a reporter rescanning on its own cadence would make every downstream change
        // filter true on every frame if an unchanged declaration were inserted again.
        let already_attached = reflect_component
            .reflect(EntityRef::from(&*entity))
            .and_then(PartialReflect::try_as_reflect)
            .is_some_and(|attached| declaration.equals(attached));
        if already_attached {
            continue;
        }
        reflect_component.insert(
            entity,
            declaration.value().as_partial_reflect(),
            type_registry,
        );
    }

    Ok(())
}

/// Synchronize one device entity's capability components with its current contributing records.
///
/// Types absent from both `agreed` and `disputed` are declarations an accepted complete set
/// removed. They are retracted through their retained type registration. Disputed types are also
/// removed, while agreed types keep the equality guard used by capability attachment. Every
/// registry lookup completes before the first entity write, so a missing registration leaves the
/// capability projection and [`ProjectedCapabilityTypes`] unchanged.
///
/// # Errors
///
/// Returns reporter-attributed [`CapabilityProjectionFailure`] values when any incoming or
/// previously projected type no longer has reflected-component metadata in `type_registry`.
pub(crate) fn project_declarations(
    entity: &mut EntityWorldMut,
    type_registry: &TypeRegistry,
    agreed: &[CapabilityProjectionDeclaration<'_>],
    disputed: &[CapabilityProjectionDeclaration<'_>],
) -> Result<(), Vec<ReporterCapabilityProjectionFailure>> {
    let (agreed_components, mut failures) = resolve_declarations(agreed, type_registry);
    let (disputed_components, disputed_failures) = resolve_declarations(disputed, type_registry);
    failures.extend(disputed_failures);
    let contributed_types: HashSet<TypeId> = agreed
        .iter()
        .chain(disputed)
        .map(|contribution| contribution.declaration().value().as_any().type_id())
        .collect();
    let previously_projected = entity
        .get::<ProjectedCapabilityTypes>()
        .cloned()
        .unwrap_or_default();
    let omitted_types: Vec<_> = previously_projected
        .types
        .iter()
        .filter(|(type_id, _)| !contributed_types.contains(type_id))
        .map(|(type_id, projected_type)| (*type_id, projected_type.clone()))
        .collect();
    let (omitted_components, omitted_failures) =
        resolve_omitted_types(&omitted_types, type_registry);
    failures.extend(omitted_failures);
    if !failures.is_empty() {
        return Err(failures);
    }

    for (_, reflect_component) in &omitted_components {
        remove_if_present(entity, reflect_component);
    }
    for (_, reflect_component) in &disputed_components {
        remove_if_present(entity, reflect_component);
    }
    for (contribution, reflect_component) in &agreed_components {
        insert_if_changed(
            entity,
            type_registry,
            contribution.declaration(),
            reflect_component,
        );
    }

    let mut projected = ProjectedCapabilityTypes::default();
    for contribution in agreed {
        let declaration = contribution.declaration();
        projected
            .types
            .entry(declaration.value().as_any().type_id())
            .or_insert_with(|| ProjectedCapabilityType {
                type_path: declaration.value().reflect_type_path().to_owned(),
                reporters: HashSet::new(),
            })
            .reporters
            .insert(contribution.reporter());
    }
    if entity.get::<ProjectedCapabilityTypes>() != Some(&projected) {
        entity.insert(projected);
    }

    Ok(())
}

fn resolve_declarations<'declaration, 'registry>(
    declarations: &[CapabilityProjectionDeclaration<'declaration>],
    type_registry: &'registry TypeRegistry,
) -> (
    Vec<(
        CapabilityProjectionDeclaration<'declaration>,
        &'registry ReflectComponent,
    )>,
    Vec<ReporterCapabilityProjectionFailure>,
) {
    let mut resolved = Vec::with_capacity(declarations.len());
    let mut failures = Vec::new();
    for contribution in declarations {
        match reflect_component_for(
            contribution.declaration().value().as_partial_reflect(),
            type_registry,
        ) {
            Ok(reflect_component) => resolved.push((*contribution, reflect_component)),
            Err(error) => failures.push(ReporterCapabilityProjectionFailure::new(
                contribution.reporter(),
                error.into_projection_failure(),
            )),
        }
    }
    (resolved, failures)
}

fn resolve_omitted_types<'registry>(
    omitted_types: &[(TypeId, ProjectedCapabilityType)],
    type_registry: &'registry TypeRegistry,
) -> (
    Vec<(TypeId, &'registry ReflectComponent)>,
    Vec<ReporterCapabilityProjectionFailure>,
) {
    let mut resolved = Vec::with_capacity(omitted_types.len());
    let mut failures = Vec::new();
    for (type_id, projected_type) in omitted_types {
        match reflect_component_for_type(*type_id, &projected_type.type_path, type_registry) {
            Ok(reflect_component) => resolved.push((*type_id, reflect_component)),
            Err(error) => {
                let failure = error.into_projection_failure();
                failures.extend(projected_type.reporters.iter().map(|reporter| {
                    ReporterCapabilityProjectionFailure::new(*reporter, failure.clone())
                }));
            },
        }
    }
    (resolved, failures)
}

fn insert_if_changed(
    entity: &mut EntityWorldMut,
    type_registry: &TypeRegistry,
    declaration: &CapabilityDeclaration,
    reflect_component: &ReflectComponent,
) {
    let already_attached = reflect_component
        .reflect(EntityRef::from(&*entity))
        .and_then(PartialReflect::try_as_reflect)
        .is_some_and(|attached| declaration.equals(attached));
    if !already_attached {
        reflect_component.insert(
            entity,
            declaration.value().as_partial_reflect(),
            type_registry,
        );
    }
}

fn remove_if_present(entity: &mut EntityWorldMut, reflect_component: &ReflectComponent) {
    if reflect_component
        .reflect(EntityRef::from(&*entity))
        .is_some()
    {
        reflect_component.remove(entity);
    }
}

/// Look one erased value's reflected component registration up, so every site that projects an
/// erased kernel value onto an entity reports the same contract error for the same reason.
///
/// The last-known-good configuration mirror and [`attach_declarations`] both need this: a driver
/// `Configuration` and a reporter capability reach the world through the same [`ReflectComponent`]
/// path, and a second copy of the lookup would let the two report different errors for the same
/// missing metadata.
pub(crate) fn reflect_component_for<'a>(
    value: &dyn PartialReflect,
    type_registry: &'a TypeRegistry,
) -> Result<&'a ReflectComponent, CapabilityAttachError> {
    let type_path = value.reflect_type_path().to_owned();
    let Some(type_id) = value.try_as_reflect().map(|value| value.as_any().type_id()) else {
        return Err(CapabilityAttachError::NotConcrete { type_path });
    };
    reflect_component_for_type(type_id, &type_path, type_registry)
}

fn reflect_component_for_type<'a>(
    type_id: TypeId,
    type_path: &str,
    type_registry: &'a TypeRegistry,
) -> Result<&'a ReflectComponent, CapabilityAttachError> {
    if !type_registry.contains(type_id) {
        return Err(CapabilityAttachError::Unregistered {
            type_path: type_path.to_owned(),
        });
    }

    type_registry
        .get_type_data::<ReflectComponent>(type_id)
        .ok_or_else(|| CapabilityAttachError::NotAComponent {
            type_path: type_path.to_owned(),
        })
}

/// Failure while turning a provider's erased capability declaration into Bevy components.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(crate) enum CapabilityAttachError {
    /// The capability type was absent from the application's type registry, so Bevy has no
    /// registration for retaining the provider's value on the resolved device entity.
    #[error("capability `{type_path}` is not registered")]
    Unregistered {
        /// Reflected type path used to identify the provider capability that needs registration.
        type_path: String,
    },
    /// The value carries no concrete Rust type behind its reflection, as a dynamic proxy built by
    /// a reflection round trip does, so there is no `TypeId` to look up in the registry.
    #[error("capability `{type_path}` is a dynamic value with no concrete type")]
    NotConcrete {
        /// Reflected type path of the dynamic value that reached the projection.
        type_path: String,
    },
    /// The type registry contains this type but it carries no [`ReflectComponent`] data, so
    /// attaching it would not produce an entity component.
    #[error("capability `{type_path}` is not a reflected component")]
    NotAComponent {
        /// Reflected type path used to identify the provider type that lacks `Component` support.
        type_path: String,
    },
}

impl CapabilityAttachError {
    fn into_type_path(self) -> String {
        match self {
            Self::Unregistered { type_path }
            | Self::NotConcrete { type_path }
            | Self::NotAComponent { type_path } => type_path,
        }
    }

    fn into_projection_failure(self) -> CapabilityProjectionFailure {
        CapabilityProjectionFailure::ReflectComponentNotRegistered {
            type_path: self.into_type_path(),
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::component::Component;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::ecs::world::World;
    use bevy::prelude::Reflect;
    use bevy::reflect::TypeRegistry;

    use super::Capabilities;
    use super::CapabilityAttachError;
    use super::attach_declarations;

    #[derive(Component, Debug, PartialEq, Reflect)]
    #[reflect(Component, PartialEq)]
    struct ChannelCount(u8);

    #[derive(Component, PartialEq, Reflect)]
    #[reflect(Component)]
    struct UnregisteredCapability;

    #[derive(Component, PartialEq, Reflect)]
    struct RegisteredComponentWithoutReflection;

    #[test]
    fn attach_inserts_registered_capability_through_reflection() -> Result<(), CapabilityAttachError>
    {
        let capabilities = Capabilities::new().with(ChannelCount(2));
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<ChannelCount>();
        let mut world = World::new();
        let mut entity = world.spawn_empty();

        attach_declarations(&mut entity, &type_registry, capabilities.declarations())?;

        assert_eq!(entity.get::<ChannelCount>(), Some(&ChannelCount(2)));

        Ok(())
    }

    #[test]
    fn attach_rejects_unregistered_capability() {
        let capabilities = Capabilities::new().with(UnregisteredCapability);
        let type_registry = TypeRegistry::default();
        let mut world = World::new();
        let mut entity = world.spawn_empty();

        assert!(matches!(
            attach_declarations(&mut entity, &type_registry, capabilities.declarations()),
            Err(CapabilityAttachError::Unregistered { .. })
        ));
    }

    #[test]
    fn a_declaration_naming_one_unregistered_type_attaches_none_of_it() {
        let capabilities = Capabilities::new()
            .with(ChannelCount(2))
            .with(UnregisteredCapability);
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<ChannelCount>();
        let mut world = World::new();
        let mut entity = world.spawn_empty();

        assert!(matches!(
            attach_declarations(&mut entity, &type_registry, capabilities.declarations()),
            Err(CapabilityAttachError::Unregistered { .. })
        ));
        assert_eq!(entity.get::<ChannelCount>(), None);
    }

    #[test]
    fn attach_rejects_registered_capability_without_component_reflection() {
        let capabilities = Capabilities::new().with(RegisteredComponentWithoutReflection);
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegisteredComponentWithoutReflection>();
        let mut world = World::new();
        let mut entity = world.spawn_empty();

        assert!(matches!(
            attach_declarations(&mut entity, &type_registry, capabilities.declarations()),
            Err(CapabilityAttachError::NotAComponent { .. })
        ));
    }
}
