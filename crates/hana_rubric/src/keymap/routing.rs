//! Whether keystrokes route to keymap commands or to a text field that owns the keyboard.

use std::any::TypeId;
use std::sync::Arc;

use bevy::prelude::Resource;

use crate::CommandId;

/// The party a text-entry handover belongs to.
///
/// Built from a type the taker already owns — its panel component, its plugin,
/// its marker type — so that two parties that both write [`KeystrokeRouting`]
/// cannot hand the keyboard back on each other's behalf.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct KeyboardOwner(TypeId);

impl KeyboardOwner {
    /// Names the owner by a type it controls.
    #[must_use]
    pub const fn of<T: 'static>() -> Self { Self(TypeId::of::<T>()) }
}

/// What [`KeystrokeRouting::take_for_text_entry`] did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardClaim {
    /// The caller now owns the keyboard, with the exempt list it supplied.
    Granted,
    /// Another party already owns the keyboard; nothing changed.
    HeldByAnother,
}

/// What [`KeystrokeRouting::release`] did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardRelease {
    /// The caller owned the keyboard and the keymap has it back.
    Released,
    /// Another party owns the keyboard; nothing changed.
    HeldByAnother,
    /// No text field owned the keyboard; nothing changed.
    AlreadyWithKeymap,
}

/// Whether the keys the reader presses are commands or the text of a field.
///
/// An application that opens a text field calls
/// [`Self::take_for_text_entry`] so the characters typed into the field are not
/// also matched against the compiled keymap, and [`Self::release`] when the
/// field closes. Routing keeps running while the field owns the keyboard — it
/// still consumes every release edge — but dispatches nothing except the
/// commands the text-entry state exempts, which is how the command that closes
/// the field keeps working from its own keystroke.
///
/// The [`KeyboardOwner`] the taker presents is what makes the two calls a pair:
/// a party that never took the keyboard cannot release it, so a palette and an
/// application's own text field cannot cancel each other's handover.
///
/// Every keyboard handover releases the held physical sources and re-inhibits
/// the keys still down, the same reset a keymap reload performs, so a key held
/// across the transition cannot stay logically pressed after it.
///
/// Both variants are `#[non_exhaustive]` so the [`KeyboardOwner`] check cannot be
/// bypassed from outside this crate: a consumer reaches either state through
/// [`Self::take_for_text_entry`] or [`Self::release`], both of which return
/// `HeldByAnother` when the caller is not the party holding the keyboard, and
/// never by assigning a variant directly.
#[derive(Clone, Debug, Eq, PartialEq, Resource)]
pub enum KeystrokeRouting {
    /// Every compiled binding routes.
    #[non_exhaustive]
    EveryBinding,
    /// A text field owns the keyboard, and only `exempt_commands` still reach
    /// their observers.
    ///
    /// The exemption covers keystrokes the compiled keymap matches. It does not
    /// reach the bare-modifier held bindings
    /// `activate_modifier_family_held_bindings` drives, which it never
    /// activates while the field owns the keyboard — a held modifier is never
    /// the command that closes a field.
    #[non_exhaustive]
    TextEntry {
        /// The party that took the keyboard, and the only one that can hand it
        /// back.
        owner:           KeyboardOwner,
        /// The commands a keystroke still runs while the field owns the
        /// keyboard, normally the one that closes the field.
        ///
        /// Shared rather than owned so that `route_input` reading the resource
        /// each frame costs a refcount bump instead of a heap allocation on the
        /// input path.
        exempt_commands: Arc<[CommandId]>,
    },
}

impl KeystrokeRouting {
    /// Suspends command routing for a text field `owner` opened, leaving
    /// `exempt_commands` runnable from the keyboard.
    ///
    /// Build the ids with [`CommandId::declared`], which takes them from the
    /// command declaration rather than from re-parsed text.
    pub(super) fn text_entry(
        owner: KeyboardOwner,
        exempt_commands: impl IntoIterator<Item = CommandId>,
    ) -> Self {
        Self::TextEntry {
            owner,
            exempt_commands: exempt_commands.into_iter().collect(),
        }
    }

    /// Hands the keyboard to `owner`'s text field.
    ///
    /// A party that already owns the keyboard may take it again, which replaces
    /// its exempt list. A party that does not gets [`KeyboardClaim::HeldByAnother`]
    /// and the state is left alone.
    #[must_use = "a refused claim leaves another party holding the keyboard"]
    pub fn take_for_text_entry(
        &mut self,
        owner: KeyboardOwner,
        exempt_commands: impl IntoIterator<Item = CommandId>,
    ) -> KeyboardClaim {
        match self {
            Self::TextEntry { owner: holder, .. } if *holder != owner => {
                KeyboardClaim::HeldByAnother
            },
            Self::EveryBinding | Self::TextEntry { .. } => {
                *self = Self::text_entry(owner, exempt_commands);
                KeyboardClaim::Granted
            },
        }
    }

    /// Hands the keyboard back to the keymap, if `owner` is the party holding
    /// it.
    #[must_use = "a refused release leaves another party holding the keyboard"]
    pub fn release(&mut self, owner: KeyboardOwner) -> KeyboardRelease {
        match self {
            Self::EveryBinding => KeyboardRelease::AlreadyWithKeymap,
            Self::TextEntry { owner: holder, .. } if *holder == owner => {
                *self = Self::EveryBinding;
                KeyboardRelease::Released
            },
            Self::TextEntry { .. } => KeyboardRelease::HeldByAnother,
        }
    }

    /// Whether a keystroke matching `command_id` still runs it.
    pub(super) fn routes(&self, command_id: &CommandId) -> bool {
        match self {
            Self::EveryBinding => true,
            Self::TextEntry {
                exempt_commands, ..
            } => exempt_commands.contains(command_id),
        }
    }
}

/// Written by hand rather than derived because `#[non_exhaustive]` on
/// [`KeystrokeRouting::EveryBinding`] rules out `#[derive(Default)]`'s
/// `#[default]` marker.
impl Default for KeystrokeRouting {
    fn default() -> Self { Self::EveryBinding }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "the command declarations generate action marker types the registry reaches by \
              reflection"
)]
mod tests {
    use bevy::prelude::Event;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy_enhanced_input::prelude::InputAction;

    use super::KeyboardClaim;
    use super::KeyboardOwner;
    use super::KeyboardRelease;
    use super::KeystrokeRouting;
    use crate::CommandId;
    use crate::ReflectKeymapCommand;

    struct QueryField;
    struct RenameField;

    crate::command! {
        action:      RoutingPaletteOpenAction,
        event:       PaletteOpen,
        id:          "routing::palette_open",
        title:       "Routing Palette Open",
        description: "Stays runnable while a text field owns the keyboard.",
    }

    crate::command! {
        action:      RoutingPaletteRunAction,
        event:       PaletteRun,
        id:          "routing::palette_run",
        title:       "Routing Palette Run",
        description: "Stands down while a text field owns the keyboard.",
    }

    #[test]
    fn every_binding_routes_each_declared_command() {
        let keystroke_routing = KeystrokeRouting::EveryBinding;

        assert!(keystroke_routing.routes(&CommandId::declared::<PaletteOpen>()));
        assert!(keystroke_routing.routes(&CommandId::declared::<PaletteRun>()));
    }

    #[test]
    fn text_entry_routes_only_the_commands_it_exempts() {
        let keystroke_routing = KeystrokeRouting::text_entry(
            KeyboardOwner::of::<QueryField>(),
            [CommandId::declared::<PaletteOpen>()],
        );

        assert!(keystroke_routing.routes(&CommandId::declared::<PaletteOpen>()));
        assert!(!keystroke_routing.routes(&CommandId::declared::<PaletteRun>()));
    }

    #[test]
    fn a_second_text_field_cannot_take_the_keyboard_from_the_field_holding_it() {
        let mut keystroke_routing = KeystrokeRouting::EveryBinding;
        assert_eq!(
            keystroke_routing.take_for_text_entry(
                KeyboardOwner::of::<QueryField>(),
                [CommandId::declared::<PaletteOpen>()],
            ),
            KeyboardClaim::Granted
        );

        let keyboard_claim = keystroke_routing.take_for_text_entry(
            KeyboardOwner::of::<RenameField>(),
            [CommandId::declared::<PaletteRun>()],
        );

        assert_eq!(keyboard_claim, KeyboardClaim::HeldByAnother);
        assert!(keystroke_routing.routes(&CommandId::declared::<PaletteOpen>()));
        assert!(!keystroke_routing.routes(&CommandId::declared::<PaletteRun>()));
    }

    #[test]
    fn a_field_that_never_took_the_keyboard_cannot_hand_it_back() {
        let mut keystroke_routing = KeystrokeRouting::text_entry(
            KeyboardOwner::of::<QueryField>(),
            [CommandId::declared::<PaletteOpen>()],
        );

        let keyboard_release = keystroke_routing.release(KeyboardOwner::of::<RenameField>());

        assert_eq!(keyboard_release, KeyboardRelease::HeldByAnother);
        assert!(!keystroke_routing.routes(&CommandId::declared::<PaletteRun>()));
    }

    #[test]
    fn the_field_holding_the_keyboard_hands_it_back_to_every_binding() {
        let mut keystroke_routing = KeystrokeRouting::text_entry(
            KeyboardOwner::of::<QueryField>(),
            [CommandId::declared::<PaletteOpen>()],
        );

        let keyboard_release = keystroke_routing.release(KeyboardOwner::of::<QueryField>());

        assert_eq!(keyboard_release, KeyboardRelease::Released);
        assert_eq!(keystroke_routing, KeystrokeRouting::EveryBinding);
        assert_eq!(
            keystroke_routing.release(KeyboardOwner::of::<QueryField>()),
            KeyboardRelease::AlreadyWithKeymap
        );
    }

    #[test]
    fn retaking_the_keyboard_replaces_the_holders_exempt_list() {
        let mut keystroke_routing = KeystrokeRouting::text_entry(
            KeyboardOwner::of::<QueryField>(),
            [CommandId::declared::<PaletteOpen>()],
        );

        let keyboard_claim = keystroke_routing.take_for_text_entry(
            KeyboardOwner::of::<QueryField>(),
            [CommandId::declared::<PaletteRun>()],
        );

        assert_eq!(keyboard_claim, KeyboardClaim::Granted);
        assert!(!keystroke_routing.routes(&CommandId::declared::<PaletteOpen>()));
        assert!(keystroke_routing.routes(&CommandId::declared::<PaletteRun>()));
    }
}
