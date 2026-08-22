use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::world::EntityRef;
use bevy::ecs::world::EntityWorldMut;
use bevy::prelude::Reflect;
use bevy::reflect::PartialReflect;
use bevy::reflect::TypeRegistry;
use thiserror::Error;

/// Erased capability components a provider reports for one `crate::DeviceRecord`.
///
/// Providers can retain private capability types while reporting them to the kernel because
/// `Capabilities` stores `Reflect` trait objects instead of a kernel-owned device-class enum.
/// Reconciliation later compares overlapping component types and inserts their values on the
/// resolved device entity.
#[derive(Default)]
pub struct Capabilities(Vec<Box<dyn Reflect>>);

impl Capabilities {
    /// Create an empty declaration for a device that currently exposes no capability components.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Add one reflected capability component to this provider's declaration.
    ///
    /// The value may use a private provider type. Reconciliation reports an error if its owner did
    /// not register it as a Bevy component before the completed scan reaches the device entity.
    pub fn add(&mut self, capability: impl Reflect) { self.0.push(Box::new(capability)); }

    /// Add one reflected capability component and return this declaration for builder-style setup.
    #[must_use]
    pub fn with(mut self, capability: impl Reflect) -> Self {
        self.add(capability);
        self
    }

    /// Borrow every declared capability component so reconciliation can compare the declarations
    /// of two reporters that describe the same device.
    ///
    /// References rather than values: `Box<dyn Reflect>` is not clonable in Bevy 0.19, and copying
    /// a declaration out of a reporter's retained set would destroy evidence a reporter that did
    /// not re-scan this frame still needs.
    pub(crate) fn declarations(&self) -> impl Iterator<Item = &dyn Reflect> {
        self.0.iter().map(AsRef::as_ref)
    }
}

/// Insert every declaration in one erased group onto `entity`, or insert none of them.
///
/// The declarations it inserts are the union of what several reporters declared, and they stay
/// borrowed from the registries that own them because `Box<dyn Reflect>` cannot be cloned into one
/// merged declaration.
///
/// # Errors
///
/// Returns `CapabilityAttachError` when a declaration is not registered as a Bevy reflected
/// component in `type_registry`, leaving `entity` exactly as it was.
pub(crate) fn attach_declarations<'a>(
    entity: &mut EntityWorldMut,
    type_registry: &TypeRegistry,
    declarations: impl IntoIterator<Item = &'a dyn Reflect>,
) -> Result<(), CapabilityAttachError> {
    // Every declaration is resolved before the first insert, so a group that names one unregistered
    // type leaves the entity exactly as it was rather than half populated with whichever
    // capabilities happened to sort earlier.
    let resolved = declarations
        .into_iter()
        .map(|capability| {
            reflect_component_for(capability.as_partial_reflect(), type_registry)
                .map(|reflect_component| (capability, reflect_component))
        })
        .collect::<Result<Vec<_>, _>>()?;

    for (capability, reflect_component) in resolved {
        // `ReflectComponent::insert` writes unconditionally and Bevy's `Changed<C>` fires on any
        // write, so a reporter rescanning on its own cadence would make every downstream change
        // filter true on every frame if an unchanged declaration were inserted again.
        let already_attached = reflect_component
            .reflect(EntityRef::from(&*entity))
            .is_some_and(|attached| {
                attached.reflect_partial_eq(capability.as_partial_reflect()) == Some(true)
            });
        if already_attached {
            continue;
        }
        reflect_component.insert(entity, capability.as_partial_reflect(), type_registry);
    }

    Ok(())
}

/// Remove every declaration in one erased group from `entity`, leaving a component the entity does
/// not carry untouched.
///
/// The device-entity projection detaches the capability types its contributors disagree about:
/// `crate::CapabilitiesDisputed` announces a disagreement and the kernel never adjudicates one, so
/// neither reporter's value may sit on the entity as an established fact. The failure is the same
/// missing-`ReflectComponent` contract error `attach_declarations` reports, because it is the same
/// registry lookup.
///
/// # Errors
///
/// Returns `CapabilityAttachError` when a declaration is not registered as a Bevy reflected
/// component in `type_registry`, leaving `entity` exactly as it was.
pub(crate) fn detach_declarations<'a>(
    entity: &mut EntityWorldMut,
    type_registry: &TypeRegistry,
    declarations: impl IntoIterator<Item = &'a dyn Reflect>,
) -> Result<(), CapabilityAttachError> {
    let resolved = declarations
        .into_iter()
        .map(|capability| reflect_component_for(capability.as_partial_reflect(), type_registry))
        .collect::<Result<Vec<_>, _>>()?;

    for reflect_component in resolved {
        // Removing a component the entity never carried still moves it between archetypes, which
        // is the per-frame churn the projection's equality guards exist to stop.
        if reflect_component
            .reflect(EntityRef::from(&*entity))
            .is_some()
        {
            reflect_component.remove(entity);
        }
    }

    Ok(())
}

/// Look one erased value's reflected component registration up, so every site that projects an
/// erased kernel value onto an entity reports the same contract error for the same reason.
///
/// The last-known-good configuration mirror and `attach_declarations` both need this: a driver
/// `Configuration` and a reporter capability reach the world through the same `ReflectComponent`
/// path, and a second copy of the lookup would let the two disagree about what missing metadata
/// means.
pub(crate) fn reflect_component_for<'a>(
    value: &dyn PartialReflect,
    type_registry: &'a TypeRegistry,
) -> Result<&'a ReflectComponent, CapabilityAttachError> {
    let type_path = value.reflect_type_path().to_owned();
    let Some(type_id) = value.try_as_reflect().map(|value| value.as_any().type_id()) else {
        return Err(CapabilityAttachError::NotConcrete { type_path });
    };
    if !type_registry.contains(type_id) {
        return Err(CapabilityAttachError::Unregistered { type_path });
    }

    type_registry
        .get_type_data::<ReflectComponent>(type_id)
        .ok_or(CapabilityAttachError::NotAComponent { type_path })
}

/// Failure while turning a provider's erased capability declaration into Bevy components.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(crate) enum CapabilityAttachError {
    /// The capability type was absent from the application's type registry, so Bevy cannot know
    /// how to retain the provider's value on the resolved device entity.
    #[error("capability `{type_path}` is not registered")]
    Unregistered {
        /// Reflected type path used to identify the provider capability that needs registration.
        type_path: String,
    },
    /// The value carries no concrete Rust type behind its reflection, as a dynamic proxy built by
    /// a reflection round trip does, so there is no type for the registry to be asked about.
    #[error("capability `{type_path}` is a dynamic value with no concrete type")]
    NotConcrete {
        /// Reflected type path of the dynamic value that reached the projection.
        type_path: String,
    },
    /// The type registry knows this type but it is not a reflected Bevy component, so attaching it
    /// would not produce an entity component.
    #[error("capability `{type_path}` is not a reflected component")]
    NotAComponent {
        /// Reflected type path used to identify the provider type that lacks `Component` support.
        type_path: String,
    },
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

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct UnregisteredCapability;

    #[derive(Reflect)]
    struct RegisteredNonComponent;

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
        let capabilities = Capabilities::new().with(RegisteredNonComponent);
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegisteredNonComponent>();
        let mut world = World::new();
        let mut entity = world.spawn_empty();

        assert!(matches!(
            attach_declarations(&mut entity, &type_registry, capabilities.declarations()),
            Err(CapabilityAttachError::NotAComponent { .. })
        ));
    }
}
