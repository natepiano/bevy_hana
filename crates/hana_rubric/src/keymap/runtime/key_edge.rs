use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;

use crate::Keystroke;
use crate::ModifierFamily;
use crate::Modifiers;
use crate::OrdinaryKey;

pub(super) const PHYSICAL_MODIFIER_KEYS: [KeyCode; 8] = [
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::AltLeft,
    KeyCode::AltRight,
    KeyCode::ShiftLeft,
    KeyCode::ShiftRight,
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
];

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum PrimaryTriggerOwnership {
    Unclaimed,
    ModifierFamilies,
    OrdinaryKeys(OrdinaryKeyRoutingState),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum OrdinaryKeyRoutingState {
    Held,
    PressEdgesOnly,
}

impl From<&ButtonInput<KeyCode>> for PrimaryTriggerOwnership {
    fn from(key_input: &ButtonInput<KeyCode>) -> Self {
        let mut primary_trigger_ownership = Self::Unclaimed;

        for key in key_input.get_pressed().copied() {
            match PhysicalKeyRole::from(key) {
                PhysicalKeyRole::OrdinaryKey(_) => {
                    return Self::OrdinaryKeys(OrdinaryKeyRoutingState::Held);
                },
                PhysicalKeyRole::ModifierFamily(_) => {
                    primary_trigger_ownership = Self::ModifierFamilies;
                },
                PhysicalKeyRole::Unroutable => {},
            }
        }

        key_input
            .get_just_pressed()
            .copied()
            .find(|key| matches!(PhysicalKeyRole::from(*key), PhysicalKeyRole::OrdinaryKey(_)))
            .map_or(primary_trigger_ownership, |_| {
                Self::OrdinaryKeys(OrdinaryKeyRoutingState::PressEdgesOnly)
            })
    }
}

#[derive(Clone, Copy)]
pub(super) enum PhysicalKeyRole {
    OrdinaryKey(OrdinaryKey),
    ModifierFamily(ModifierFamily),
    /// A key code no keystroke can name, so no binding can reach it.
    ///
    /// Routing treats it as inert: it claims no primary trigger and suspends no modifier-family
    /// hold, so holding a media key while `shift` is bound to a hold-to-act command leaves that
    /// command running.
    Unroutable,
}

impl From<KeyCode> for PhysicalKeyRole {
    fn from(key: KeyCode) -> Self {
        match key {
            KeyCode::ControlLeft | KeyCode::ControlRight => {
                Self::ModifierFamily(ModifierFamily::Control)
            },
            KeyCode::AltLeft | KeyCode::AltRight => Self::ModifierFamily(ModifierFamily::Alt),
            KeyCode::ShiftLeft | KeyCode::ShiftRight => Self::ModifierFamily(ModifierFamily::Shift),
            KeyCode::SuperLeft | KeyCode::SuperRight => {
                Self::ModifierFamily(ModifierFamily::Platform)
            },
            _ => OrdinaryKey::try_from(key).map_or(Self::Unroutable, Self::OrdinaryKey),
        }
    }
}

pub(super) fn keystroke(pressed: &ButtonInput<KeyCode>, ordinary_key: OrdinaryKey) -> Keystroke {
    Keystroke::from_ordinary_key(Modifiers::from_pressed(pressed), ordinary_key)
}
