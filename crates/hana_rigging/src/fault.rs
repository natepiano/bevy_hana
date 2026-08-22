use bevy::prelude::Reflect;

/// A failure a driver reports after the kernel has begun an operation on a device.
///
/// `DeviceAccessError` keeps the kernel-readable class separate from the provider detail, so
/// retry policy can distinguish a contended camera from a transport failure without discarding
/// the operating-system message needed for diagnostics.
///
/// New hardware can bring failure classifications this crate cannot enumerate in advance, so
/// downstream matches must carry a wildcard arm and adding a variant must not be a breaking change.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
#[non_exhaustive]
pub enum DeviceAccessError {
    /// Another owner retained exclusive access after the driver started, such as a camera open by
    /// a video-conferencing application.
    Contended {
        /// Provider-supplied text that identifies the contending operation or platform response.
        detail: String,
    },
    /// The operating system or device policy refused access after the driver started, such as a
    /// USB accessory denied by a platform permission gate.
    Blocked {
        /// Provider-supplied text that identifies the permission or policy response.
        detail: String,
    },
    /// The device departed after the kernel authorized the operation, such as a display
    /// disconnected while its configuration was being applied.
    Absent {
        /// Provider-supplied text that identifies the departure or missing-device response.
        detail: String,
    },
    /// Communication with the device or its provider failed for a reason outside the other
    /// access classes, such as a HID transport error.
    Transport {
        /// Provider-supplied text that identifies the transport or protocol response.
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;

    use super::DeviceAccessError;

    #[test]
    fn device_access_error_is_a_history_payload_not_a_component() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();
        let type_id = TypeId::of::<DeviceAccessError>();

        assert!(type_registry.contains(type_id));
        assert!(
            type_registry
                .get_type_data::<ReflectComponent>(type_id)
                .is_none()
        );

        drop(type_registry);
    }
}
