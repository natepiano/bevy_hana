//! Live source attribution for installed input bindings.

use bevy::input::gamepad::Gamepad;
use bevy::prelude::ButtonInput;
use bevy::prelude::GamepadButton;
use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ModKeys;

use super::action_set::HeldActionBindingEntry;
use super::action_set::ImpulseActionBindingEntry;
use super::held_binding;
use super::held_binding::BindingGate;
use super::held_binding::GateInput;
use super::held_binding::GatePolarity;
use crate::input::CameraSemanticAction;
use crate::input::HeldCameraAction;
use crate::input::InteractionSources;

/// Device state consulted when attributing an input edge to the binding that
/// produced it.
pub struct LiveInputs<'a> {
    pub keyboard: Option<&'a ButtonInput<KeyCode>>,
    pub mouse:    Option<&'a ButtonInput<MouseButton>>,
    /// All connected gamepads; a button counts as active if any pad holds it.
    pub gamepads: &'a [&'a Gamepad],
}

pub trait SourceBinding {
    /// Static device family this binding can produce.
    fn sources(&self) -> InteractionSources;

    /// Whether this binding's physical input is active right now.
    fn is_active(&self, inputs: &LiveInputs<'_>) -> bool;
}

/// Unions the sources of the entries whose physical input is active; falls
/// back to `fallback` when none can be attributed.
pub fn attributed_sources<'a, T: SourceBinding + 'a>(
    entries: impl IntoIterator<Item = &'a T>,
    inputs: &LiveInputs<'_>,
    fallback: InteractionSources,
) -> InteractionSources {
    let sources = entries
        .into_iter()
        .filter(|entry| entry.is_active(inputs))
        .fold(InteractionSources::NONE, |sources, entry| {
            sources.union(entry.sources())
        });
    if sources.is_empty() {
        fallback
    } else {
        sources
    }
}

impl SourceBinding for Binding {
    fn sources(&self) -> InteractionSources { held_binding::sources_for_binding(*self) }

    fn is_active(&self, inputs: &LiveInputs<'_>) -> bool { binding_is_active(*self, inputs) }
}

impl<A: HeldCameraAction> SourceBinding for HeldActionBindingEntry<A> {
    fn sources(&self) -> InteractionSources { Self::sources(self) }

    fn is_active(&self, inputs: &LiveInputs<'_>) -> bool {
        self.engagement_descriptor().enabled_is_active(inputs)
            && self
                .gates()
                .entries()
                .iter()
                .all(|gate| gate_is_satisfied(*gate, inputs))
    }
}

impl<A: CameraSemanticAction> SourceBinding for ImpulseActionBindingEntry<A> {
    fn sources(&self) -> InteractionSources { Self::sources(self) }

    fn is_active(&self, inputs: &LiveInputs<'_>) -> bool {
        self.binding_descriptor().enabled_is_active(inputs)
    }
}

pub(super) fn binding_is_active(binding: Binding, inputs: &LiveInputs<'_>) -> bool {
    match binding {
        Binding::Keyboard { key, mod_keys } => inputs
            .keyboard
            .is_some_and(|keyboard| keyboard.pressed(key) && mod_keys_active(inputs, mod_keys)),
        Binding::MouseButton { button, mod_keys } => inputs
            .mouse
            .is_some_and(|mouse| mouse.pressed(button) && mod_keys_active(inputs, mod_keys)),
        Binding::GamepadButton(button) => gamepad_button_active(inputs.gamepads, button),
        Binding::MouseMotion { .. }
        | Binding::MouseWheel { .. }
        | Binding::GamepadAxis(_)
        | Binding::AnyKey
        | Binding::Custom(_)
        | Binding::None => false,
    }
}

fn gate_is_satisfied(gate: BindingGate, inputs: &LiveInputs<'_>) -> bool {
    let active = match gate.input {
        GateInput::GamepadButton(button) => gamepad_button_active(inputs.gamepads, button),
        GateInput::Key(key) => inputs
            .keyboard
            .is_some_and(|keyboard| keyboard.pressed(key)),
    };
    match gate.polarity {
        GatePolarity::Required => active,
        GatePolarity::Blocked => !active,
    }
}

fn gamepad_button_active(gamepads: &[&Gamepad], button: GamepadButton) -> bool {
    gamepads.iter().any(|gamepad| gamepad.pressed(button))
}

fn mod_keys_active(inputs: &LiveInputs<'_>, mod_keys: ModKeys) -> bool {
    mod_keys.is_empty()
        || inputs
            .keyboard
            .is_some_and(|keyboard| mod_keys_pressed(keyboard, mod_keys))
}

/// Returns `true` when every required modifier key is currently pressed.
pub fn mod_keys_pressed(keyboard: &ButtonInput<KeyCode>, mod_keys: ModKeys) -> bool {
    mod_keys.iter_keys().all(|keys| keyboard.any_pressed(keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyboard_inputs(keyboard: &ButtonInput<KeyCode>) -> LiveInputs<'_> {
        LiveInputs {
            keyboard: Some(keyboard),
            mouse:    None,
            gamepads: &[],
        }
    }

    #[test]
    fn keyboard_binding_reports_active_only_when_key_is_pressed() {
        let binding = Binding::Keyboard {
            key:      KeyCode::KeyH,
            mod_keys: ModKeys::empty(),
        };
        let mut keyboard = ButtonInput::default();

        assert!(!binding.is_active(&keyboard_inputs(&keyboard)));

        keyboard.press(KeyCode::KeyH);

        assert!(binding.is_active(&keyboard_inputs(&keyboard)));
    }

    #[test]
    fn keyboard_binding_respects_mod_keys() {
        let binding = Binding::Keyboard {
            key:      KeyCode::KeyH,
            mod_keys: ModKeys::SHIFT,
        };
        let mut keyboard = ButtonInput::default();
        keyboard.press(KeyCode::KeyH);

        assert!(!binding.is_active(&keyboard_inputs(&keyboard)));

        keyboard.press(KeyCode::ShiftLeft);

        assert!(binding.is_active(&keyboard_inputs(&keyboard)));
    }

    #[test]
    fn gamepad_button_binding_reports_active_when_any_gamepad_holds_button() {
        let binding = Binding::GamepadButton(GamepadButton::Select);
        let idle_gamepad = Gamepad::default();
        let mut active_gamepad = Gamepad::default();
        active_gamepad.digital_mut().press(GamepadButton::Select);

        let gamepads = [&idle_gamepad, &active_gamepad];
        let inputs = LiveInputs {
            keyboard: None,
            mouse:    None,
            gamepads: &gamepads,
        };

        assert!(binding.is_active(&inputs));
    }

    #[test]
    fn attributed_sources_falls_back_when_no_binding_is_active() {
        let bindings = [
            Binding::Keyboard {
                key:      KeyCode::KeyH,
                mod_keys: ModKeys::empty(),
            },
            Binding::GamepadButton(GamepadButton::Select),
        ];
        let keyboard = ButtonInput::default();
        let fallback = InteractionSources::KEYBOARD.union(InteractionSources::GAMEPAD);

        assert_eq!(
            attributed_sources(bindings.iter(), &keyboard_inputs(&keyboard), fallback),
            fallback
        );
    }

    #[test]
    fn attributed_sources_reports_only_active_bindings() {
        let bindings = [
            Binding::Keyboard {
                key:      KeyCode::KeyH,
                mod_keys: ModKeys::empty(),
            },
            Binding::GamepadButton(GamepadButton::Select),
        ];
        let mut keyboard = ButtonInput::default();
        keyboard.press(KeyCode::KeyH);
        let fallback = InteractionSources::KEYBOARD.union(InteractionSources::GAMEPAD);

        assert_eq!(
            attributed_sources(bindings.iter(), &keyboard_inputs(&keyboard), fallback),
            InteractionSources::KEYBOARD
        );
    }
}
