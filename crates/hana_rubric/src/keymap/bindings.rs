//! Immutable command-to-keystroke tables committed alongside dispatch.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use bevy::ecs::world::World;
use bevy::prelude::Resource;

use super::EffectiveKeymapSnapshot;
use super::KeymapGeneration;
use super::MergedKeymap;
use crate::CommandId;
use crate::Keystroke;
use crate::KeystrokeSequence;

/// Why no effective keymap bindings are available yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeymapBindingUnavailability {
    /// Runtime installation completed before keymap assembly begins.
    AwaitingInitialLoad,
    /// Assembly found no keymap plugin configuration.
    Unconfigured,
    /// Assembly found configuration without an embedded default keymap.
    MissingDefault,
    /// The embedded default keymap was rejected before the first commit.
    InvalidDefault,
}

/// What the effective keymap resolves one declared command's keystroke to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKeystroke<'keymap> {
    /// The committed keymap is unavailable for this startup reason.
    KeymapUnavailable(KeymapBindingUnavailability),
    /// The effective table runs the command from this representative sequence.
    BoundTo(&'keymap KeystrokeSequence),
    /// The loaded effective table has no binding for this command.
    Unbound,
}

/// One application-owned recovery chord associated with its semantic command.
///
/// The association is published beside the effective authored bindings so consumers can display
/// recovery even when no authored generation is available. It never enters Rubric's matcher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProtectedCommandBinding {
    command_id: CommandId,
    keystroke:  Keystroke,
}

impl ProtectedCommandBinding {
    pub(crate) const fn new(command_id: CommandId, keystroke: Keystroke) -> Self {
        Self {
            command_id,
            keystroke,
        }
    }

    pub(crate) const fn command_id(&self) -> &CommandId { &self.command_id }

    pub(crate) const fn keystroke(&self) -> &Keystroke { &self.keystroke }
}

/// Whether a command has an application-owned recovery association outside authored routing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplicationRecoveryAssociation<'keymap> {
    /// The command is associated with this application recovery keystroke.
    Present(&'keymap Keystroke),
    /// The command has no application recovery association.
    Absent,
}

/// The durable availability or effective binding table for the live keymap.
///
/// Protected command associations remain available across every authored-keymap outcome. A
/// recovery chord must remain visible when defaults are invalid or the state snapshot cannot
/// materialize, but it is never included in the authored effective table.
#[derive(Resource)]
pub struct KeymapBindings {
    protected_command_bindings: Vec<ProtectedCommandBinding>,
    state:                      AuthoredKeymapBindingState,
}

/// The authored-binding portion of [`KeymapBindings`].
enum AuthoredKeymapBindingState {
    /// No validated keymap generation is available.
    Unavailable(KeymapBindingUnavailability),
    /// One immutable table for a materialized snapshot generation.
    Loaded(LoadedKeymapBindings),
}

/// The authored-keymap portion of a published [`KeymapBindings`] view.
#[derive(Clone, Copy)]
pub enum AuthoredKeymapBindings<'keymap> {
    /// No validated authored generation is available for this reason.
    Unavailable(KeymapBindingUnavailability),
    /// The committed authored table for one semantic effective generation.
    Loaded(&'keymap LoadedKeymapBindings),
}

impl Default for KeymapBindings {
    fn default() -> Self {
        Self {
            protected_command_bindings: Vec::new(),
            state:                      AuthoredKeymapBindingState::Unavailable(
                KeymapBindingUnavailability::AwaitingInitialLoad,
            ),
        }
    }
}

impl KeymapBindings {
    /// Returns the authored effective-table state while preserving the protected associations.
    #[must_use]
    pub const fn authored(&self) -> AuthoredKeymapBindings<'_> {
        match &self.state {
            AuthoredKeymapBindingState::Unavailable(unavailability) => {
                AuthoredKeymapBindings::Unavailable(*unavailability)
            },
            AuthoredKeymapBindingState::Loaded(bindings) => {
                AuthoredKeymapBindings::Loaded(bindings)
            },
        }
    }

    /// Reports the effective binding for `command_id`.
    #[must_use]
    pub fn keystroke(&self, command_id: &CommandId) -> CommandKeystroke<'_> {
        match &self.state {
            AuthoredKeymapBindingState::Unavailable(unavailability) => {
                CommandKeystroke::KeymapUnavailable(*unavailability)
            },
            AuthoredKeymapBindingState::Loaded(bindings) => {
                bindings.effective_keystroke(command_id)
            },
        }
    }

    /// Reports whether `command_id` has an application recovery route outside authored routing.
    pub(crate) fn application_recovery_association(
        &self,
        command_id: &CommandId,
    ) -> ApplicationRecoveryAssociation<'_> {
        self.protected_command_bindings
            .iter()
            .find(|association| association.command_id() == command_id)
            .map(ProtectedCommandBinding::keystroke)
            .map_or(
                ApplicationRecoveryAssociation::Absent,
                ApplicationRecoveryAssociation::Present,
            )
    }

    #[cfg(test)]
    pub(crate) fn unavailable_with_protected_command_bindings_for_test(
        unavailability: KeymapBindingUnavailability,
        protected_command_bindings: Vec<(CommandId, Keystroke)>,
    ) -> Self {
        Self {
            protected_command_bindings: protected_command_bindings
                .into_iter()
                .map(|(command_id, keystroke)| ProtectedCommandBinding::new(command_id, keystroke))
                .collect(),
            state:                      AuthoredKeymapBindingState::Unavailable(unavailability),
        }
    }

    #[cfg(test)]
    pub(crate) fn loaded_for_test(
        generation: KeymapGeneration,
        snapshot: EffectiveKeymapSnapshot,
        effective_bindings: Vec<(CommandId, KeystrokeSequence)>,
        protected_command_bindings: Vec<(CommandId, Keystroke)>,
    ) -> Self {
        let mut sequences = Vec::new();
        let mut effective = HashMap::new();
        for (command_id, sequence) in effective_bindings {
            let sequence_handle = BindingSequenceHandle(sequences.len());
            sequences.push(sequence);
            effective.insert(command_id, sequence_handle);
        }
        Self {
            protected_command_bindings: protected_command_bindings
                .into_iter()
                .map(|(command_id, keystroke)| ProtectedCommandBinding::new(command_id, keystroke))
                .collect(),
            state:                      AuthoredKeymapBindingState::Loaded(LoadedKeymapBindings {
                generation,
                snapshot,
                sequences,
                effective,
            }),
        }
    }

    pub(crate) fn replace_protected_command_bindings(
        world: &mut World,
        protected_command_bindings: Vec<ProtectedCommandBinding>,
    ) {
        let current = world.resource::<Self>();
        if current.protected_command_bindings != protected_command_bindings {
            world.resource_mut::<Self>().protected_command_bindings = protected_command_bindings;
        }
    }

    pub(crate) fn replace_unavailability(
        world: &mut World,
        unavailability: KeymapBindingUnavailability,
    ) {
        let is_current = matches!(
            world.get_resource::<Self>(),
            Some(bindings)
                if matches!(
                    bindings.authored(),
                    AuthoredKeymapBindings::Unavailable(current) if current == unavailability
                )
        );
        if !is_current {
            world.resource_mut::<Self>().state =
                AuthoredKeymapBindingState::Unavailable(unavailability);
        }
    }

    pub(super) fn replace_loaded(world: &mut World, bindings: LoadedKeymapBindings) {
        world.resource_mut::<Self>().state = AuthoredKeymapBindingState::Loaded(bindings);
    }
}

/// Builds an authored-keymap-unavailable view without any protected associations.
///
/// Applications normally receive this resource from [`crate::KeymapPlugin`]. This conversion is
/// useful to renderer tests that need one terminal keymap outcome in isolation.
impl From<KeymapBindingUnavailability> for KeymapBindings {
    fn from(unavailability: KeymapBindingUnavailability) -> Self {
        Self {
            protected_command_bindings: Vec::new(),
            state:                      AuthoredKeymapBindingState::Unavailable(unavailability),
        }
    }
}

/// One representative sequence per command for a materialized snapshot generation.
pub struct LoadedKeymapBindings {
    generation: KeymapGeneration,
    snapshot:   EffectiveKeymapSnapshot,
    sequences:  Vec<KeystrokeSequence>,
    effective:  HashMap<CommandId, BindingSequenceHandle>,
}

impl LoadedKeymapBindings {
    pub(super) fn from_state_materialization(
        generation: KeymapGeneration,
        snapshot: EffectiveKeymapSnapshot,
        merged_keymap: &MergedKeymap,
    ) -> Self {
        let mut sequences = Vec::new();
        let effective = representative_bindings(merged_keymap.effective_bindings())
            .into_iter()
            .map(|(command_id, keystroke_sequence)| {
                let sequence_handle = BindingSequenceHandle(sequences.len());
                sequences.push(keystroke_sequence.clone());
                (command_id, sequence_handle)
            })
            .collect();
        let bindings = Self {
            generation,
            snapshot,
            sequences,
            effective,
        };
        bindings.assert_matches_merged(merged_keymap);
        bindings
    }

    /// Returns this table's committed keymap replacement identity.
    #[must_use]
    pub(crate) const fn generation(&self) -> KeymapGeneration { self.generation }

    /// Returns the exact active-context identity that produced this table.
    pub(crate) const fn snapshot(&self) -> &EffectiveKeymapSnapshot { &self.snapshot }

    fn effective_keystroke(&self, command_id: &CommandId) -> CommandKeystroke<'_> {
        self.effective
            .get(command_id)
            .map_or(CommandKeystroke::Unbound, |sequence_handle| {
                CommandKeystroke::BoundTo(&self.sequences[sequence_handle.0])
            })
    }

    fn assert_matches_merged(&self, merged_keymap: &MergedKeymap) {
        let expected = representative_bindings(merged_keymap.effective_bindings());
        debug_assert_eq!(expected.len(), self.effective.len());
        debug_assert!(expected.iter().all(|(command_id, sequence)| {
            self.effective_keystroke(command_id) == CommandKeystroke::BoundTo(sequence)
        }));
    }

    #[cfg(test)]
    pub(crate) fn global_bindings_for_test(&self) -> Vec<(CommandId, KeystrokeSequence)> {
        let mut bindings = self
            .effective
            .iter()
            .map(|(command_id, sequence_handle)| {
                (
                    command_id.clone(),
                    self.sequences[sequence_handle.0].clone(),
                )
            })
            .collect::<Vec<_>>();
        bindings.sort_unstable_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
        bindings
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BindingSequenceHandle(usize);

fn representative_bindings(
    effective_bindings: &[(KeystrokeSequence, CommandId)],
) -> HashMap<CommandId, &KeystrokeSequence> {
    let mut representatives: HashMap<CommandId, &KeystrokeSequence> = HashMap::new();
    for (keystroke_sequence, command_id) in effective_bindings {
        match representatives.entry(command_id.clone()) {
            Entry::Occupied(mut occupied) => {
                if keystroke_sequence.structural_cmp(occupied.get()).is_lt() {
                    occupied.insert(keystroke_sequence);
                }
            },
            Entry::Vacant(vacant) => {
                vacant.insert(keystroke_sequence);
            },
        }
    }
    representatives
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::representative_bindings;
    use crate::CommandId;
    use crate::KeystrokeSequence;

    fn command_id() -> Result<CommandId, String> {
        CommandId::from_str("palette::open")
            .map_err(|error| format!("invalid representative command ID: {error}"))
    }

    fn sequence(source: &str) -> Result<KeystrokeSequence, String> {
        KeystrokeSequence::from_str(source)
            .map_err(|error| format!("invalid representative sequence `{source}`: {error}"))
    }

    #[test]
    fn representative_binding_prefers_fewest_strokes_over_source_order() -> Result<(), String> {
        let command_id = command_id()?;
        let longer = sequence("ctrl-p q")?;
        let shorter = sequence("ctrl-p")?;
        for bindings in [
            vec![
                (longer.clone(), command_id.clone()),
                (shorter.clone(), command_id.clone()),
            ],
            vec![
                (shorter.clone(), command_id.clone()),
                (longer, command_id.clone()),
            ],
        ] {
            let representatives = representative_bindings(&bindings);

            assert_eq!(representatives[&command_id], &shorter);
        }
        Ok(())
    }

    #[test]
    fn equal_length_representative_uses_structural_keystroke_order() -> Result<(), String> {
        let command_id = command_id()?;
        let lexically_earlier = sequence("alt-p")?;
        let structurally_earlier = sequence("ctrl-p")?;
        assert!(lexically_earlier.to_string() < structurally_earlier.to_string());
        let bindings = [
            (lexically_earlier, command_id.clone()),
            (structurally_earlier.clone(), command_id.clone()),
        ];

        let representatives = representative_bindings(&bindings);

        assert_eq!(representatives[&command_id], &structurally_earlier);
        Ok(())
    }

    #[test]
    fn platform_and_modifier_representatives_are_host_independent() -> Result<(), String> {
        let command_id = command_id()?;

        for (candidates, expected) in [
            (["ctrl-p", "p"], "p"),
            (["platform-p", "ctrl-p"], "ctrl-p"),
            (["alt-p", "ctrl-p"], "ctrl-p"),
            (["ctrl", "a"], "a"),
            (["z", "a"], "a"),
            (["platform", "ctrl"], "ctrl"),
        ] {
            let bindings = candidates
                .into_iter()
                .map(|candidate| Ok((sequence(candidate)?, command_id.clone())))
                .collect::<Result<Vec<_>, String>>()?;
            let expected = sequence(expected)?;

            let representatives = representative_bindings(&bindings);

            assert_eq!(representatives[&command_id], &expected);
        }
        Ok(())
    }

    #[test]
    fn reversing_binding_construction_order_preserves_representative() -> Result<(), String> {
        let command_id = command_id()?;
        let candidates = [
            sequence("platform-p")?,
            sequence("alt-p")?,
            sequence("ctrl-p")?,
        ];
        let forward = candidates
            .iter()
            .cloned()
            .map(|candidate| (candidate, command_id.clone()))
            .collect::<Vec<_>>();
        let reverse = candidates
            .iter()
            .rev()
            .cloned()
            .map(|candidate| (candidate, command_id.clone()))
            .collect::<Vec<_>>();

        let forward_representatives = representative_bindings(&forward);
        let reverse_representatives = representative_bindings(&reverse);
        let expected = sequence("ctrl-p")?;

        assert_eq!(forward_representatives[&command_id], &expected);
        assert_eq!(reverse_representatives[&command_id], &expected);
        Ok(())
    }
}
