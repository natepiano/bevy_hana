use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::reflect::ReflectDeserialize;
use bevy::reflect::ReflectSerialize;
use serde::Deserialize;
use serde::Serialize;

use super::scheme::AuthoredId;
use super::scheme::Digest;
use super::scheme::ReportedId;
use super::scheme::SchemeName;

/// Opaque runtime handle issued once for a newly observed `DeviceKey`.
///
/// `DeviceId` is process-local, `Copy`, and never persisted. Reconciliation assigns its private
/// integer from a monotonic counter so a removed device leaves a dangling handle instead of a
/// handle that silently denotes a later device.
///
/// Reflection sees the handle opaquely, which registers `DeviceId` as a component while denying
/// construction from a dynamic tuple struct: without that, an inspector could mint a handle the
/// device registry never issued and route an apply to another unit. Opacity also withholds the
/// field, so reflection-driven tooling reads the issued value through `DeviceId::get` rather than
/// through the type registry.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Component, Reflect)]
#[reflect(opaque)]
#[reflect(Component, PartialEq)]
pub struct DeviceId(u64);

impl DeviceId {
    /// Wrap the device registry's next counter value.
    ///
    /// Private to the crate because only `crate::Devices` issues handles; a reporter or an
    /// application that could mint one would be asserting an identity it never established.
    pub(crate) const fn new(value: u64) -> Self { Self(value) }

    /// Report the issued counter value for diagnostics and stable ordering in reports.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

/// Durable designation for a device that can cross process and storage boundaries.
///
/// `DeviceKey::id` retains whether its value is proof or a hint. Storing that distinction in the
/// enum variant prevents callers from copying a string while losing the rule that controls output.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Component, Serialize, Deserialize, Reflect)]
#[reflect(Component, PartialEq, Serialize, Deserialize)]
pub struct DeviceKey {
    /// Physical role that keeps a display, audio interface, DMX universe, or HID panel separate
    /// even when two providers use similar identifier values.
    pub kind: DeviceKind,
    /// Identity source whose variant records whether this designation may ever authorize output.
    pub id:   DeviceIdSource,
}

/// Trust classification for the value used in a `DeviceKey`.
///
/// `DeviceIdSource` prevents a synthesized location hint from being treated like a value reported
/// by the unit itself. Only `Reported` and explicitly authored inventory identity can later
/// authorize output.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Serialize, Deserialize)]
pub enum DeviceIdSource {
    /// A unit reported this value, such as its USB serial or a monitor serial in EDID.
    ///
    /// This is evidence of the physical unit and is the only source that can later authorize
    /// output to that unit.
    Reported {
        /// Registered identity space that gives the reported value its provider-defined meaning.
        scheme: SchemeName,
        /// Provider-defined value reported by the physical unit within `scheme`.
        value:  ReportedId,
    },
    /// Hana derived this value from descriptors because the unit exposes no unique identity.
    ///
    /// A port-derived camera value and a serial-less display UUID can restore saved configuration,
    /// but neither can authorize output because a later scan can assign the hint to another unit.
    Synthesized {
        /// Fixed-width FNV-1a result derived from the provider's descriptors.
        digest: Digest,
    },
    /// Application-authored durable identity for hardware retained in `HardwareInventory`.
    ///
    /// An authored display entry can authorize its own declared endpoint after reporter evidence
    /// identifies the live unit under this key. Unlike `Synthesized`, this is explicit operator
    /// intent persisted by the application instead of a reusable location hint.
    Authored {
        /// Checked persisted value the application assigned to this durable inventory identity.
        value: AuthoredId,
    },
}

/// Physical role used to keep identifier spaces for unrelated hardware separate.
///
/// This enum is non-exhaustive because future providers can report device classes that existing
/// applications do not control; downstream matches must retain a wildcard for those classes.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Serialize, Deserialize)]
pub enum DeviceKind {
    /// A display panel, typically identified by an EDID serial before window placement is restored.
    Display,
    /// A camera whose reported serial or derived port location distinguishes capture sources.
    Camera,
    /// An audio interface, such as a USB unit whose provider reports a `CoreAudio` identifier.
    AudioInterface,
    /// A DMX universe that a lighting provider addresses through a patch or network endpoint.
    DmxUniverse,
    /// A HID control panel, such as a USB Stream Deck or a network-attached dock child.
    HidPanel,
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::error::Error;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use bevy::reflect::FromReflect;
    use bevy::reflect::ReflectSerialize;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;
    use bevy::world_serialization::DynamicWorldBuilder;
    use ron::Options;
    use ron::extensions::Extensions;
    use ron::ser::PrettyConfig;
    use serde::Serialize;

    use super::DeviceId;
    use super::DeviceIdSource;
    use super::DeviceKey;
    use super::DeviceKind;
    use crate::AuthoredId;
    use crate::Digest;
    use crate::ReportedId;
    use crate::SchemeName;

    #[derive(Component, Reflect, Serialize)]
    #[reflect(Component, Serialize)]
    struct PersistedDevice(DeviceKey);

    #[test]
    fn device_key_round_trips_from_each_ron_form() -> Result<(), Box<dyn Error>> {
        let options = Options::default().with_default_extension(Extensions::UNWRAP_NEWTYPES);
        let pretty = PrettyConfig::new().struct_names(true).compact_structs(true);
        let display = reported_key("edid-serial", "DELL-U2723QE-9J4K2H3", DeviceKind::Display)?;
        let audio_interface = reported_key(
            "coreaudio-uid",
            "Scarlett18i20:D4E5",
            DeviceKind::AudioInterface,
        )?;
        let dmx_universe = reported_key("patch", "artnet/10.0.0.7/u1", DeviceKind::DmxUniverse)?;
        let hid_panel = reported_key("usb-serial", "CL15K1A00080", DeviceKind::HidPanel)?;
        let dock_child = reported_key("net-dock-node", "dock:AB12/child/2", DeviceKind::HidPanel)?;
        let camera = DeviceKey {
            kind: DeviceKind::Camera,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(14_695_981_039_346_656_037),
            },
        };
        let authored_display = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("studio-display-a")?,
            },
        };

        for (ron, expected) in [
            (
                r#"DeviceKey(kind: Display, id: Reported(scheme: "edid-serial", value: "DELL-U2723QE-9J4K2H3"))"#,
                display,
            ),
            (
                r#"DeviceKey(kind: AudioInterface, id: Reported(scheme: "coreaudio-uid", value: "Scarlett18i20:D4E5"))"#,
                audio_interface,
            ),
            (
                r#"DeviceKey(kind: DmxUniverse, id: Reported(scheme: "patch", value: "artnet/10.0.0.7/u1"))"#,
                dmx_universe,
            ),
            (
                r#"DeviceKey(kind: HidPanel, id: Reported(scheme: "usb-serial", value: "CL15K1A00080"))"#,
                hid_panel,
            ),
            (
                r#"DeviceKey(kind: HidPanel, id: Reported(scheme: "net-dock-node", value: "dock:AB12/child/2"))"#,
                dock_child,
            ),
            (
                r"DeviceKey(kind: Camera, id: Synthesized(digest: 14695981039346656037))",
                camera,
            ),
            (
                r#"DeviceKey(kind: Display, id: Authored(value: "studio-display-a"))"#,
                authored_display,
            ),
        ] {
            let parsed = options.from_str::<DeviceKey>(ron)?;
            assert_eq!(parsed, expected);

            let serialized = options.to_string_pretty(&expected, pretty.clone())?;
            assert_eq!(serialized, ron);
        }

        Ok(())
    }

    #[test]
    fn device_id_is_excluded_from_persisted_device_entities() -> Result<(), Box<dyn Error>> {
        let key = reported_key("edid-serial", "DELL-U2723QE-9J4K2H3", DeviceKind::Display)?;
        let mut app = App::new();
        let entity = app
            .world_mut()
            .spawn((DeviceId::new(7), PersistedDevice(key)))
            .id();
        let serialized = {
            let world = app.world();
            let app_type_registry = world.resource::<AppTypeRegistry>().clone();
            let type_registry = app_type_registry.read();
            let dynamic_world = DynamicWorldBuilder::from_world(world, &type_registry)
                .deny_component::<DeviceId>()
                .extract_entity(entity)
                .build();
            let serialized = dynamic_world.serialize(&type_registry)?;
            drop(type_registry);
            serialized
        };

        assert!(serialized.contains("PersistedDevice"));
        assert!(!serialized.contains("DeviceId"));

        Ok(())
    }

    #[test]
    fn reflection_cannot_construct_device_id() {
        let mut dynamic_device_id = DynamicTupleStruct::default();
        dynamic_device_id.insert(0_u64);

        assert!(DeviceId::from_reflect(&dynamic_device_id).is_none());
    }

    #[test]
    fn device_id_registers_component_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();

        assert!(
            type_registry
                .get_type_data::<ReflectComponent>(TypeId::of::<DeviceId>())
                .is_some()
        );

        drop(type_registry);
    }

    fn reported_key(
        scheme: &str,
        value: &str,
        kind: DeviceKind,
    ) -> Result<DeviceKey, Box<dyn Error>> {
        Ok(DeviceKey {
            kind,
            id: DeviceIdSource::Reported {
                scheme: SchemeName::new(scheme)?,
                value:  ReportedId::new(value)?,
            },
        })
    }
}
