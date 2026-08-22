use bevy::prelude::Event;
use bevy::prelude::Reflect;
use bevy::prelude::World;
use bevy::reflect::FromType;
use bevy_enhanced_input::prelude::CustomInput;

use super::Capability;
use super::HoldPhase;
use super::registry;

/// Metadata declaration for a semantic event that can be invoked from a keymap.
///
/// `command!` implements `KeymapCommand` for generated event types. Downstream crates can also
/// implement this trait for their own reflected semantic events.
pub trait KeymapCommand: Event + Reflect {
    /// Command identifier text that satisfies [`CommandId::is_valid`](super::CommandId::is_valid).
    const ID: &'static str;
    /// Palette title text for this command.
    const TITLE: &'static str;
    /// Palette description text for this command.
    const DESCRIPTION: &'static str;
    /// Keymap binding capability declared for this command.
    const CAPABILITY: Capability;

    /// Constructs this event when the command registry dispatches it.
    #[must_use]
    fn build() -> Self;

    /// Returns the requested transition for a held semantic event.
    ///
    /// Every command implements this method. Non-held commands return `None`.
    /// `command! { held, ... }` returns its `phase` field, and hand-written held commands return
    /// the transition that drives their [`CustomInput`](bevy_enhanced_input::prelude::CustomInput).
    #[must_use]
    fn hold_phase(&self) -> Option<HoldPhase>;
}

/// Reflection type data retained for each [`KeymapCommand`] event registration.
#[derive(Clone, Debug)]
pub struct ReflectKeymapCommand {
    /// Validated command identifier text.
    pub id:                            &'static str,
    /// Palette title text.
    pub title:                         &'static str,
    /// Palette description text.
    pub description:                   &'static str,
    /// Keymap binding capability.
    pub capability:                    Capability,
    pub(super) dispatch:               fn(&mut World),
    pub(super) register_held_observer: fn(&mut World, CustomInput),
}

impl<T> FromType<T> for ReflectKeymapCommand
where
    T: KeymapCommand,
    for<'a> T::Trigger<'a>: Default,
{
    fn from_type() -> Self {
        Self {
            id:                     T::ID,
            title:                  T::TITLE,
            description:            T::DESCRIPTION,
            capability:             T::CAPABILITY,
            dispatch:               dispatch_command::<T>,
            register_held_observer: registry::register_held_observer::<T>,
        }
    }
}

fn dispatch_command<T>(world: &mut World)
where
    T: KeymapCommand,
    for<'a> T::Trigger<'a>: Default,
{
    world.trigger(T::build());
}

/// Declares an input action, semantic event, and reflected keymap-command metadata.
///
/// The invocation module must import `InputAction`, `Event`, `Reflect`, `ReflectEvent`, and
/// `ReflectKeymapCommand` unqualified. The wrapped `action!` and `event!` macros emit bare derive
/// names, and the `Reflect` derive resolves `ReflectEvent` and `ReflectKeymapCommand` at the
/// invocation site.
///
/// ```ignore
/// use bevy::prelude::Event;
/// use bevy::prelude::Reflect;
/// use bevy::prelude::ReflectEvent;
/// use bevy_enhanced_input::prelude::InputAction;
/// use hana_rubric::command;
/// use hana_rubric::ReflectKeymapCommand;
///
/// command! {
///     action:      CameraHome,
///     event:       CameraHomeEvent,
///     id:          "camera::home",
///     title:       "Reset Camera to Home",
///     description: "Return the camera to its default framing.",
/// }
/// ```
///
/// Held commands use the leading `held,` marker and generate an event payload with a
/// [`HoldPhase`](super::HoldPhase):
///
/// ```ignore
/// command! {
///     held,
///     action:      FreezeTrackingAction,
///     event:       FreezeTracking,
///     id:          "tracking::freeze",
///     title:       "Freeze Tracking",
///     description: "Freezes tracking while the binding remains held.",
/// }
/// ```
///
/// The expansion reaches the local `$crate::action!` and `$crate::event!` macros.
#[macro_export]
macro_rules! command {
    (
        held,
        action: $action:ident,
        event: $event:ident,
        id: $id:literal,
        title: $title:literal,
        description: $description:literal $(,)?
    ) => {
        $crate::command!(
            @declare
            held,
            $action,
            $event,
            $id,
            $title,
            $description,
            $crate::Capability::Held
        );
    };
    (
        action: $action:ident,
        event: $event:ident,
        id: $id:literal,
        title: $title:literal,
        description: $description:literal,
        capability: Held $(,)?
    ) => {
        compile_error!(
            "declare a hold-to-act command with `command! { held, ... }`; \
             `capability: Held` expands to an event with no hold phase"
        );
    };
    (
        action: $action:ident,
        event: $event:ident,
        id: $id:literal,
        title: $title:literal,
        description: $description:literal,
        capability: $capability:ident $(,)?
    ) => {
        $crate::command!(
            @declare
            unit,
            $action,
            $event,
            $id,
            $title,
            $description,
            $crate::Capability::$capability
        );
    };
    (
        action: $action:ident,
        event: $event:ident,
        id: $id:literal,
        title: $title:literal,
        description: $description:literal $(,)?
    ) => {
        $crate::command!(
            @declare
            unit,
            $action,
            $event,
            $id,
            $title,
            $description,
            $crate::Capability::OneShot
        );
    };
    (@declare $event_kind:ident, $action:ident, $event:ident, $id:literal, $title:literal, $description:literal, $capability:path) => {
        $crate::action!($action);
        $crate::command!(@event $event_kind, $event);

        const _: () = assert!($crate::CommandId::is_valid($id));

        impl $crate::KeymapCommand for $event {
            const ID:          &'static str = $id;
            const TITLE:       &'static str = $title;
            const DESCRIPTION: &'static str = $description;
            const CAPABILITY:  $crate::Capability = $capability;
            $crate::command!(@build $event_kind);
            $crate::command!(@hold_phase $event_kind);
        }
    };
    (@build unit) => {
        fn build() -> Self { Self }
    };
    (@build held) => {
        fn build() -> Self {
            Self {
                phase: $crate::HoldPhase::Begin,
            }
        }
    };
    (@hold_phase unit) => {
        fn hold_phase(&self) -> Option<$crate::HoldPhase> { None }
    };
    (@hold_phase held) => {
        fn hold_phase(&self) -> Option<$crate::HoldPhase> { Some(self.phase) }
    };
    (@event unit, $event:ident) => {
        $crate::event!($event, reflect: KeymapCommand);
    };
    (@event held, $event:ident) => {
        $crate::event!($event { phase: $crate::HoldPhase }, reflect: KeymapCommand);
    };
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "command tests declare input action marker types without constructing them"
)]
mod tests {
    use std::any::TypeId;

    use bevy::prelude::Event;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy::reflect::GetTypeRegistration;
    use bevy::reflect::TypeRegistry;
    use bevy_enhanced_input::prelude::InputAction;

    use super::Capability;
    use super::HoldPhase;
    use super::ReflectKeymapCommand;

    crate::command! {
        action:      CameraHome,
        event:       CameraHomeEvent,
        id:          "camera::home",
        title:       "Reset Camera to Home",
        description: "Return the camera to its default framing.",
    }

    crate::command! {
        held,
        action:      CameraFocusHeld,
        event:       CameraFocusHeldEvent,
        id:          "camera::focus_held",
        title:       "Focus Camera",
        description: "Focus the camera while the key is held.",
    }

    crate::command! {
        action:      RecoveryPalette,
        event:       RecoveryPaletteEvent,
        id:          "recovery::palette",
        title:       "Open Recovery Palette",
        description: "Open the permanently registered recovery palette.",
        capability: Unremappable,
    }

    #[test]
    fn command_macro_declares_input_action_and_event() {
        assert_input_action::<CameraHome>();
        assert_event::<CameraHomeEvent>();
    }

    #[test]
    fn command_metadata_registers_declared_text_and_default_capability() {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<CameraHomeEvent>();

        let command =
            type_registry.get_type_data::<ReflectKeymapCommand>(TypeId::of::<CameraHomeEvent>());

        assert_eq!(command.map(|command| command.id), Some("camera::home"));
        assert_eq!(
            command.map(|command| command.title),
            Some("Reset Camera to Home")
        );
        assert_eq!(
            command.map(|command| command.description),
            Some("Return the camera to its default framing.")
        );
        assert_eq!(
            command.map(|command| command.capability),
            Some(Capability::OneShot)
        );
    }

    #[test]
    fn held_capability_registers_with_the_event() {
        assert_eq!(
            registered_capability::<CameraFocusHeldEvent>(),
            Some(Capability::Held)
        );
    }

    #[test]
    fn held_command_event_carries_the_requested_phase() {
        let event = CameraFocusHeldEvent {
            phase: HoldPhase::Begin,
        };

        assert_eq!(event.phase, HoldPhase::Begin);
    }

    #[test]
    fn unremappable_capability_registers_with_the_event() {
        assert_eq!(
            registered_capability::<RecoveryPaletteEvent>(),
            Some(Capability::Unremappable)
        );
    }

    fn assert_input_action<T: InputAction>() {}

    fn assert_event<T: Event>() {}

    fn registered_capability<T: GetTypeRegistration>() -> Option<Capability> {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<T>();
        type_registry
            .get_type_data::<ReflectKeymapCommand>(TypeId::of::<T>())
            .map(|command| command.capability)
    }
}
