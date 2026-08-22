use bevy::prelude::Reflect;

/// Controls how a [`KeymapCommand`](super::KeymapCommand) can be bound in a keymap.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub enum Capability {
    /// The default capability; the command binds to any keystroke sequence.
    #[default]
    OneShot,
    /// The command binds to one keystroke, including `f`, `shift-f`, or `cmd-shift-f`, but not a
    /// multi-key sequence.
    ///
    /// Keymap validation rejects a `Held` command bound to a sequence. This is a command
    /// property, not a keymap-format limitation.
    Held,
    /// The command is available only through permanently registered application bindings.
    ///
    /// Keymap validation rejects entries that name an `Unremappable` command because it is
    /// reserved for recovery.
    Unremappable,
}

impl Capability {
    /// Returns whether this command can appear in the command palette.
    #[must_use]
    pub const fn is_palette_invocable(self) -> bool {
        matches!(self, Self::OneShot | Self::Unremappable)
    }

    /// Returns whether a user keymap can bind this command.
    #[must_use]
    pub const fn is_user_bindable(self) -> bool { !matches!(self, Self::Unremappable) }
}

/// The semantic transition requested for a held command.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub enum HoldPhase {
    /// Start driving the held command's custom input.
    #[default]
    Begin,
    /// Stop driving the held command's custom input.
    End,
}

#[cfg(test)]
mod tests {
    use super::Capability;

    #[test]
    fn capability_predicates_distinguish_palette_and_keymap_use() {
        assert!(Capability::OneShot.is_palette_invocable());
        assert!(Capability::Unremappable.is_palette_invocable());
        assert!(!Capability::Held.is_palette_invocable());

        assert!(Capability::OneShot.is_user_bindable());
        assert!(Capability::Held.is_user_bindable());
        assert!(!Capability::Unremappable.is_user_bindable());
    }
}
