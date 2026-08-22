//! Matcher construction from one materialized effective keymap.

use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy_enhanced_input::prelude::CustomInput;

use super::MergedKeymap;
use crate::CommandId;
use crate::CommandRegistry;
use crate::KeystrokeSequence;
use crate::ModifierFamily;
use crate::PrimaryTrigger;
use crate::SequenceMatcher;
use crate::command::CommandEntry;
use crate::command::Invocation;

/// A monotonically increasing effective-keymap replacement identity.
///
/// Rubric owns construction and advancement. Consumers can display or compare the identities it
/// publishes without treating a document-layer order as the same value.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct KeymapGeneration(usize);

impl KeymapGeneration {
    pub(crate) const fn initial() -> Self { Self(0) }

    pub(crate) const fn next(self) -> Self { Self(self.0 + 1) }
}

impl Display for KeymapGeneration {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// An opaque index into the compiled command table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandHandle(usize);

#[cfg(test)]
impl CommandHandle {
    pub(super) const fn from_index(index: usize) -> Self { Self(index) }
}

/// One complete matcher and command table for a materialized snapshot generation.
#[derive(Resource)]
pub(crate) struct CompiledKeymap {
    pub(super) generation:  KeymapGeneration,
    pub(super) matcher:     SequenceMatcher<CommandHandle>,
    pub(super) commands:    Vec<(CommandId, CommandEntry)>,
    modifier_held_bindings: ModifierFamilyHeldBindings,
}

impl CompiledKeymap {
    #[must_use]
    pub(crate) fn from_merged(
        generation: KeymapGeneration,
        merged_keymap: &MergedKeymap,
        command_registry: &CommandRegistry,
    ) -> Self {
        let command_entries = command_registry
            .iter_entries()
            .map(|(command_id, command_entry)| (command_id.clone(), command_entry.clone()))
            .collect::<Vec<_>>();
        let command_handles = command_entries
            .iter()
            .enumerate()
            .map(|(index, (command_id, _))| (command_id.clone(), CommandHandle(index)))
            .collect::<HashMap<_, _>>();
        let modifier_held_bindings = ModifierFamilyHeldBindings::from_bindings(
            merged_keymap.effective_bindings(),
            &command_handles,
            &command_entries,
        );
        let matcher = SequenceMatcher::new(merged_keymap.effective_bindings().iter().filter_map(
            |(keystroke_sequence, command_id)| {
                matcher_entry(keystroke_sequence, command_id, &command_handles)
            },
        ));
        Self {
            generation,
            matcher,
            commands: command_entries,
            modifier_held_bindings,
        }
    }

    pub(super) fn command_id(&self, command_handle: CommandHandle) -> Option<&CommandId> {
        self.commands
            .get(command_handle.0)
            .map(|(command_id, _)| command_id)
    }

    pub(super) const fn modifier_family_held_binding(
        &self,
        modifier_family: ModifierFamily,
    ) -> ModifierFamilyHeldBinding {
        self.modifier_held_bindings.for_family(modifier_family)
    }

    pub(crate) fn invocation(&self, command_handle: CommandHandle) -> Option<Invocation> {
        self.commands
            .get(command_handle.0)
            .map(|(_, command_entry)| command_entry.invocation())
    }

    pub(crate) fn dispatch(&self, command_handle: CommandHandle) -> Option<fn(&mut World)> {
        self.commands
            .get(command_handle.0)
            .map(|(_, command_entry)| command_entry.dispatch())
    }

    #[cfg(test)]
    pub(crate) const fn generation(&self) -> KeymapGeneration { self.generation }

    #[cfg(test)]
    pub(crate) const fn has_pending_sequence(&self) -> bool { self.matcher.is_pending() }

    #[cfg(test)]
    pub(crate) fn match_effective(
        &mut self,
        keystroke: crate::Keystroke,
        now: Instant,
        timeout: Duration,
    ) -> crate::MatchOutcome<CommandHandle> {
        self.matcher.match_keystroke(keystroke, now, timeout)
    }
}

fn matcher_entry(
    keystroke_sequence: &KeystrokeSequence,
    command_id: &CommandId,
    command_handles: &HashMap<CommandId, CommandHandle>,
) -> Option<(KeystrokeSequence, CommandHandle)> {
    if !keystroke_sequence
        .iter()
        .all(|keystroke| matches!(keystroke.primary_trigger(), PrimaryTrigger::OrdinaryKey(_)))
    {
        return None;
    }
    command_handles
        .get(command_id)
        .copied()
        .map(|command_handle| (keystroke_sequence.clone(), command_handle))
}

#[derive(Clone, Copy, Default)]
pub(crate) enum ModifierFamilyHeldBinding {
    #[default]
    Unbound,
    Bound(CustomInput),
}

#[derive(Clone, Copy, Default)]
struct ModifierFamilyHeldBindings {
    control:  ModifierFamilyHeldBinding,
    alt:      ModifierFamilyHeldBinding,
    shift:    ModifierFamilyHeldBinding,
    platform: ModifierFamilyHeldBinding,
}

impl ModifierFamilyHeldBindings {
    fn from_bindings(
        bindings: &[(KeystrokeSequence, CommandId)],
        command_handles: &HashMap<CommandId, CommandHandle>,
        command_entries: &[(CommandId, CommandEntry)],
    ) -> Self {
        let mut held_bindings = Self::default();
        for (keystroke_sequence, command_id) in bindings {
            if keystroke_sequence.len() != 1 {
                continue;
            }
            let PrimaryTrigger::ModifierFamily(modifier_family) =
                keystroke_sequence.first().primary_trigger()
            else {
                continue;
            };
            let Some(command_handle) = command_handles.get(command_id).copied() else {
                continue;
            };
            let Some((_, command_entry)) = command_entries.get(command_handle.0) else {
                continue;
            };
            let Invocation::Held(custom_input) = command_entry.invocation() else {
                continue;
            };
            held_bindings.set(
                modifier_family,
                ModifierFamilyHeldBinding::Bound(custom_input),
            );
        }
        held_bindings
    }

    const fn for_family(self, modifier_family: ModifierFamily) -> ModifierFamilyHeldBinding {
        match modifier_family {
            ModifierFamily::Control => self.control,
            ModifierFamily::Alt => self.alt,
            ModifierFamily::Shift => self.shift,
            ModifierFamily::Platform => self.platform,
        }
    }

    const fn set(&mut self, modifier_family: ModifierFamily, binding: ModifierFamilyHeldBinding) {
        match modifier_family {
            ModifierFamily::Control => self.control = binding,
            ModifierFamily::Alt => self.alt = binding,
            ModifierFamily::Shift => self.shift = binding,
            ModifierFamily::Platform => self.platform = binding,
        }
    }
}
