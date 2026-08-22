use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;

/// Retention and reapplication policy stored with a device binding after its provider reports a
/// unit absent.
///
/// `RecoveryPolicy` makes a saved configuration's later treatment explicit, so a missing display,
/// projector, or HID panel cannot acquire automatic output merely because its configuration
/// remains available. `Presence` and `Claim` answer current usability; this policy instead
/// controls whether the binding keeps a configuration and how it may return.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Component, Reflect)]
#[reflect(Component, PartialEq)]
pub enum RecoveryPolicy {
    /// Discard the saved configuration when a provider no longer reports the device, as for a
    /// display whose window arrangement must not return after the display is removed.
    ///
    /// This default enforces the kernel rule that it never silently puts a device back in service;
    /// a later application decision must supply any new configuration and authorization.
    #[default]
    Forget,
    /// Keep the saved configuration but never send it to the device, as for a lighting rig whose
    /// operator must approve every blackout change at the console.
    ///
    /// The kernel holds the value for reports and inspection and offers no path that reapplies
    /// it. `ReapplyOnRequest` is the neighbouring policy that does; choose that one instead when
    /// application code needs to ask the kernel to send the value back.
    Retain,
    /// Keep the saved configuration until application code requests reapplication, as for a
    /// projector whose next presentation determines when its shutter state should return.
    ///
    /// A verified return report does not apply the configuration by itself; the application makes
    /// the request when the restored setting suits its current work.
    ReapplyOnRequest,
    /// Keep the saved configuration and reapply it after reconciliation verifies the returning
    /// physical unit, as when a replugged Stream Deck reports its earlier serial.
    ///
    /// Reapplication is authorized as a restore rather than as in-service use, so a unit whose
    /// key was synthesized still gets its saved configuration back. Driving output stays gated
    /// on a reported identity, which is why returning a projector's shutter state never by
    /// itself puts that projector in service.
    ReapplyOnReturn,
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::reflect::PartialReflect;

    use super::RecoveryPolicy;
    use crate::DeviceKey;
    use crate::IdentityVerdict;

    #[test]
    fn forget_is_the_default_recovery_policy() {
        assert!(matches!(RecoveryPolicy::default(), RecoveryPolicy::Forget));
    }

    #[test]
    fn recovery_policy_copies_reapplication_on_return() {
        fn assert_copy<T: Copy>() {}

        let recovery_policy = RecoveryPolicy::ReapplyOnReturn;
        let copy = recovery_policy;

        assert_copy::<RecoveryPolicy>();
        assert_eq!(copy, recovery_policy);
        assert!(matches!(copy, RecoveryPolicy::ReapplyOnReturn));
    }

    #[test]
    fn reflected_comparison_answers_for_equal_and_unequal_recovery_policies() {
        assert_eq!(
            RecoveryPolicy::Forget.reflect_partial_eq(&RecoveryPolicy::Forget),
            Some(true)
        );
        assert_eq!(
            RecoveryPolicy::Forget.reflect_partial_eq(&RecoveryPolicy::Retain),
            Some(false)
        );
    }

    #[test]
    fn device_components_register_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();

        for type_id in [
            TypeId::of::<RecoveryPolicy>(),
            TypeId::of::<DeviceKey>(),
            TypeId::of::<IdentityVerdict>(),
        ] {
            assert!(type_registry.contains(type_id));
            assert!(
                type_registry
                    .get_type_data::<ReflectComponent>(type_id)
                    .is_some()
            );
        }

        drop(type_registry);
    }
}
