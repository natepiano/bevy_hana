//! Borrowed, renderer-independent command-palette queries.

use std::marker::PhantomData;

use crate::ActiveKeymapContext;
use crate::ActiveKeymapContextState;
use crate::AuthoredKeymapBindings;
use crate::CommandRegistry;
use crate::ContextDimensionName;
use crate::EffectiveKeymapStatus;
use crate::KeymapBindingUnavailability;
use crate::KeymapBindings;
use crate::Keystroke;
use crate::KeystrokeSequence;
use crate::command;
use crate::command::PaletteInvocableCommand;
use crate::command::PaletteSearchMatchInvocability;
use crate::keymap::ApplicationRecoveryAssociation;
use crate::keymap::CommandKeystroke;
use crate::keymap::EffectiveKeymapSnapshot;

/// Queries the live command palette without storing a renderer-specific cache.
///
/// The returned model retains immutable borrows of the registry, keymap bindings, and active
/// condition. It therefore cannot outlive—or coexist with a mutable update to—any queried input.
#[must_use]
pub fn query_command_palette<'registry, 'keymap, 'context, 'status>(
    command_registry: &'registry CommandRegistry,
    active_context: &'context ActiveKeymapContext,
    effective_keymap_status: &'status EffectiveKeymapStatus,
    keymap_bindings: &'keymap KeymapBindings,
    query: &str,
) -> CommandPaletteQueryResult<'registry, 'keymap, 'context, 'status> {
    let normalized_query = command::normalize_palette_search_text(query);
    let query_is_empty = normalized_query.is_empty();
    let mut rows = Vec::new();
    let mut query_match = PaletteQueryMatch::None;

    for matching_command in command_registry.palette_search(&normalized_query) {
        let command = match matching_command {
            PaletteSearchMatchInvocability::Invocable(command) => command,
            PaletteSearchMatchInvocability::Held => {
                if matches!(query_match, PaletteQueryMatch::None) {
                    query_match = PaletteQueryMatch::Held;
                }
                continue;
            },
        };

        if !matches!(query_match, PaletteQueryMatch::Invocable(_)) {
            query_match = PaletteQueryMatch::Invocable(command);
        }
        rows.push(CommandPaletteRow {
            command,
            binding: palette_binding(
                keymap_bindings,
                active_context,
                effective_keymap_status,
                command.id(),
            ),
        });
    }

    let selection = if query_is_empty {
        PaletteSelectionOutcome::EmptyQuery
    } else {
        match query_match {
            PaletteQueryMatch::None => PaletteSelectionOutcome::NoMatch,
            PaletteQueryMatch::Held => PaletteSelectionOutcome::NotPaletteInvocable,
            PaletteQueryMatch::Invocable(command) => PaletteSelectionOutcome::Selected(command),
        }
    };

    CommandPaletteQueryResult {
        rows,
        selection,
        active_context,
        source_borrows: PhantomData,
    }
}

/// The complete borrowed result of one command-palette query.
///
/// Rows retain command declaration, effective-binding, and active-context borrows. The explicit
/// input borrows keep the result synchronized with all inputs even when a particular result has
/// no rows or no bound sequence.
pub struct CommandPaletteQueryResult<'registry, 'keymap, 'context, 'status> {
    rows:           Vec<CommandPaletteRow<'registry, 'keymap, 'context>>,
    selection:      PaletteSelectionOutcome<'registry>,
    active_context: &'context ActiveKeymapContext,
    source_borrows: PhantomData<(
        &'registry CommandRegistry,
        &'keymap KeymapBindings,
        &'context ActiveKeymapContext,
        &'status EffectiveKeymapStatus,
    )>,
}

impl<'registry, 'keymap, 'context> CommandPaletteQueryResult<'registry, 'keymap, 'context, '_> {
    /// Returns the palette-invocable rows in authored title/identifier order.
    #[must_use]
    pub fn rows(&self) -> &[CommandPaletteRow<'registry, 'keymap, 'context>] { &self.rows }

    /// Returns the command selected by this query or the semantic reason that it selects none.
    #[must_use]
    pub const fn selection(&self) -> PaletteSelectionOutcome<'registry> { self.selection }

    /// Returns the complete active state snapshot that all rows observed.
    #[must_use]
    pub const fn active_context(&self) -> &'context ActiveKeymapContextState {
        self.active_context.state()
    }
}

/// One renderer-ready palette row with borrowed command metadata and effective binding state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandPaletteRow<'registry, 'keymap, 'context> {
    command: PaletteInvocableCommand<'registry>,
    binding: PaletteBinding<'keymap, 'context>,
}

impl<'registry, 'keymap, 'context> CommandPaletteRow<'registry, 'keymap, 'context> {
    /// Returns the palette-invocable command metadata this row renders.
    #[must_use]
    pub const fn command(&self) -> PaletteInvocableCommand<'registry> { self.command }

    /// Returns this command's exhaustive binding outcome in the queried context.
    #[must_use]
    pub const fn binding(&self) -> PaletteBinding<'keymap, 'context> { self.binding }
}

/// One command's palette-visible keyboard binding state.
///
/// These variants are mutually exclusive and ordered by their reader-visible precedence. A
/// protected application recovery chord wins over every authored-keymap or active-state outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaletteBinding<'keymap, 'context> {
    /// An application-owned recovery chord invokes this command outside authored routing.
    ApplicationRecovery(&'keymap Keystroke),
    /// The effective active table binds the command to this representative sequence.
    BoundTo(&'keymap KeystrokeSequence),
    /// The effective active table contains no binding for the command.
    Unbound,
    /// No validated keymap generation is available for this startup reason.
    KeymapUnavailable(KeymapBindingUnavailability),
    /// Registered state dimensions have not all reported their initial values yet.
    AwaitingStateDimensions,
    /// These sorted application-owned state dimensions are unavailable.
    StateDimensionsUnavailable(&'context [ContextDimensionName]),
    /// A reflected snapshot could not materialize against the registered typed vocabulary.
    UnmaterializableStateDimensions,
}

/// What the queried text selects for direct registry-wide invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaletteSelectionOutcome<'registry> {
    /// The first palette-invocable command matching a nonempty query.
    Selected(PaletteInvocableCommand<'registry>),
    /// The normalized query is empty, so rows list every palette-invocable command without one
    /// selected command.
    EmptyQuery,
    /// No declared command matched the normalized query.
    NoMatch,
    /// Declared commands matched, but every match is held rather than palette-invocable.
    NotPaletteInvocable,
}

/// The palette-invocation eligibility found while walking matching declarations.
enum PaletteQueryMatch<'registry> {
    /// No declaration matched the normalized query.
    None,
    /// Matching declarations seen so far are all held commands.
    Held,
    /// The first palette-invocable matching declaration.
    Invocable(PaletteInvocableCommand<'registry>),
}

/// Whether the published effective status and binding table describe the active snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublishedSnapshotStatus {
    /// Every generation and snapshot identity agrees.
    Coherent,
    /// At least one published identity disagrees or has no loaded value.
    Incoherent,
}

fn palette_binding<'keymap, 'context>(
    keymap_bindings: &'keymap KeymapBindings,
    active_context: &'context ActiveKeymapContext,
    effective_keymap_status: &EffectiveKeymapStatus,
    command_id: &crate::CommandId,
) -> PaletteBinding<'keymap, 'context> {
    match keymap_bindings.application_recovery_association(command_id) {
        ApplicationRecoveryAssociation::Present(keystroke) => {
            return PaletteBinding::ApplicationRecovery(keystroke);
        },
        ApplicationRecoveryAssociation::Absent => {},
    }
    let loaded_keymap_bindings = match keymap_bindings.authored() {
        AuthoredKeymapBindings::Unavailable(unavailability) => {
            return PaletteBinding::KeymapUnavailable(unavailability);
        },
        AuthoredKeymapBindings::Loaded(bindings) => bindings,
    };

    match active_context.state() {
        ActiveKeymapContextState::AwaitingStateDimensions => {
            PaletteBinding::AwaitingStateDimensions
        },
        ActiveKeymapContextState::StateDimensionsUnavailable { missing } => {
            PaletteBinding::StateDimensionsUnavailable(missing)
        },
        ActiveKeymapContextState::GlobalRouting | ActiveKeymapContextState::Resolved(_) => {
            match published_snapshot_status(
                active_context.state(),
                effective_keymap_status,
                loaded_keymap_bindings.snapshot(),
                loaded_keymap_bindings.generation(),
            ) {
                PublishedSnapshotStatus::Coherent => {
                    palette_binding_from_keymap(keymap_bindings.keystroke(command_id))
                },
                PublishedSnapshotStatus::Incoherent => {
                    PaletteBinding::UnmaterializableStateDimensions
                },
            }
        },
    }
}

fn published_snapshot_status(
    active_context: &ActiveKeymapContextState,
    effective_keymap_status: &EffectiveKeymapStatus,
    committed_snapshot: &EffectiveKeymapSnapshot,
    committed_generation: crate::KeymapGeneration,
) -> PublishedSnapshotStatus {
    let EffectiveKeymapStatus::Loaded(publication) = effective_keymap_status else {
        return PublishedSnapshotStatus::Incoherent;
    };
    if publication.generation == committed_generation
        && &publication.snapshot == committed_snapshot
        && match (active_context, committed_snapshot) {
            (ActiveKeymapContextState::GlobalRouting, EffectiveKeymapSnapshot::Global) => true,
            (
                ActiveKeymapContextState::Resolved(active_snapshot),
                EffectiveKeymapSnapshot::Resolved(committed_snapshot),
            ) => active_snapshot == committed_snapshot,
            _ => false,
        }
    {
        PublishedSnapshotStatus::Coherent
    } else {
        PublishedSnapshotStatus::Incoherent
    }
}

const fn palette_binding_from_keymap<'context>(
    command_keystroke: CommandKeystroke<'_>,
) -> PaletteBinding<'_, 'context> {
    match command_keystroke {
        CommandKeystroke::BoundTo(sequence) => PaletteBinding::BoundTo(sequence),
        CommandKeystroke::Unbound => PaletteBinding::Unbound,
        CommandKeystroke::KeymapUnavailable(unavailability) => {
            PaletteBinding::KeymapUnavailable(unavailability)
        },
    }
}

#[cfg(test)]
mod tests {
    use std::ptr;
    use std::str::FromStr;

    use super::PaletteBinding;
    use super::palette_binding;
    use super::query_command_palette;
    use crate::ActiveKeymapContext;
    use crate::ActiveKeymapContextState;
    use crate::CommandId;
    use crate::CommandRegistry;
    use crate::ContextDimensionName;
    use crate::ContextSnapshot;
    use crate::ContextValueName;
    use crate::EffectiveKeymapPublication;
    use crate::EffectiveKeymapSnapshot;
    use crate::EffectiveKeymapStatus;
    use crate::KeymapBindingUnavailability;
    use crate::KeymapBindings;
    use crate::KeymapGeneration;
    use crate::Keystroke;
    use crate::KeystrokeSequence;

    const BOUND_COMMAND_ID: &str = "palette_test::bound";
    const UNBOUND_COMMAND_ID: &str = "palette_test::unbound";
    const RECOVERY_COMMAND_ID: &str = "palette_test::recovery";

    fn command_id(text: &str) -> Result<CommandId, String> {
        CommandId::from_str(text).map_err(|error| format!("invalid command id `{text}`: {error}"))
    }

    fn sequence(text: &str) -> Result<KeystrokeSequence, String> {
        KeystrokeSequence::from_str(text)
            .map_err(|error| format!("invalid keystroke sequence `{text}`: {error}"))
    }

    fn keystroke(text: &str) -> Result<Keystroke, String> {
        Keystroke::from_str(text).map_err(|error| format!("invalid keystroke `{text}`: {error}"))
    }

    fn context_snapshot(values: &[(&str, &str)]) -> ContextSnapshot {
        values
            .iter()
            .map(|(dimension, value)| {
                (
                    ContextDimensionName::new(*dimension),
                    ContextValueName::new(*value),
                )
            })
            .collect()
    }

    fn loaded_bindings(snapshot: EffectiveKeymapSnapshot) -> Result<KeymapBindings, String> {
        Ok(KeymapBindings::loaded_for_test(
            KeymapGeneration::initial(),
            snapshot,
            vec![(command_id(BOUND_COMMAND_ID)?, sequence("ctrl-b")?)],
            Vec::new(),
        ))
    }

    fn loaded_status(
        generation: KeymapGeneration,
        snapshot: EffectiveKeymapSnapshot,
    ) -> EffectiveKeymapStatus {
        EffectiveKeymapStatus::Loaded(EffectiveKeymapPublication {
            generation,
            snapshot,
            matched_layers: Vec::new(),
        })
    }

    #[test]
    fn application_recovery_association_precedes_keymap_and_state_failures() -> Result<(), String> {
        let recovery_command_id = command_id(RECOVERY_COMMAND_ID)?;
        let recovery_keystroke = keystroke("ctrl-p")?;
        let keymap_bindings = KeymapBindings::unavailable_with_protected_command_bindings_for_test(
            KeymapBindingUnavailability::InvalidDefault,
            vec![(recovery_command_id.clone(), recovery_keystroke)],
        );
        let active_context =
            ActiveKeymapContext::from(ActiveKeymapContextState::StateDimensionsUnavailable {
                missing: vec![ContextDimensionName::new("application")],
            });

        assert!(matches!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &EffectiveKeymapStatus::UnmaterializableStateDimensions,
                &recovery_command_id,
            ),
            PaletteBinding::ApplicationRecovery(keystroke) if keystroke.to_string() == "ctrl-p"
        ));
        Ok(())
    }

    #[test]
    fn keymap_unavailability_precedes_state_and_retains_the_missing_context_borrow()
    -> Result<(), String> {
        let command_id = command_id(BOUND_COMMAND_ID)?;
        let keymap_bindings = KeymapBindings::from(KeymapBindingUnavailability::MissingDefault);
        let active_context =
            ActiveKeymapContext::from(ActiveKeymapContextState::StateDimensionsUnavailable {
                missing: vec![
                    ContextDimensionName::new("alpha"),
                    ContextDimensionName::new("zebra"),
                ],
            });
        let registry = CommandRegistry::empty();
        let query_result = query_command_palette(
            &registry,
            &active_context,
            &EffectiveKeymapStatus::AwaitingAcceptedDocument,
            &keymap_bindings,
            "",
        );
        let ActiveKeymapContextState::StateDimensionsUnavailable {
            missing: source_missing,
        } = active_context.state()
        else {
            return Err(String::from("missing-context fixture changed state"));
        };
        let ActiveKeymapContextState::StateDimensionsUnavailable {
            missing: query_missing,
        } = query_result.active_context()
        else {
            return Err(String::from(
                "query did not retain the missing context state",
            ));
        };

        assert!(matches!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &EffectiveKeymapStatus::AwaitingAcceptedDocument,
                &command_id,
            ),
            PaletteBinding::KeymapUnavailable(KeymapBindingUnavailability::MissingDefault)
        ));
        assert_eq!(
            query_missing
                .iter()
                .map(ContextDimensionName::as_str)
                .collect::<Vec<_>>(),
            ["alpha", "zebra"]
        );
        assert!(ptr::eq(source_missing, query_missing));
        Ok(())
    }

    #[test]
    fn state_outcomes_precede_global_effective_table_checks() -> Result<(), String> {
        let keymap_bindings = loaded_bindings(EffectiveKeymapSnapshot::Global)?;
        let bound_command_id = command_id(BOUND_COMMAND_ID)?;
        let effective_status = loaded_status(
            KeymapGeneration::initial().next(),
            EffectiveKeymapSnapshot::Global,
        );
        let awaiting_context =
            ActiveKeymapContext::from(ActiveKeymapContextState::AwaitingStateDimensions);
        let missing_context =
            ActiveKeymapContext::from(ActiveKeymapContextState::StateDimensionsUnavailable {
                missing: vec![
                    ContextDimensionName::new("alpha"),
                    ContextDimensionName::new("zebra"),
                ],
            });

        assert_eq!(
            palette_binding(
                &keymap_bindings,
                &awaiting_context,
                &effective_status,
                &bound_command_id,
            ),
            PaletteBinding::AwaitingStateDimensions
        );
        assert!(matches!(
            palette_binding(
                &keymap_bindings,
                &missing_context,
                &effective_status,
                &bound_command_id,
            ),
            PaletteBinding::StateDimensionsUnavailable(missing)
                if missing.iter().map(ContextDimensionName::as_str).eq(["alpha", "zebra"])
        ));
        Ok(())
    }

    #[test]
    fn global_generation_and_snapshot_matches_control_bound_and_unbound_outcomes()
    -> Result<(), String> {
        let keymap_bindings = loaded_bindings(EffectiveKeymapSnapshot::Global)?;
        let active_context = ActiveKeymapContext::default();
        let bound_command_id = command_id(BOUND_COMMAND_ID)?;
        let unbound_command_id = command_id(UNBOUND_COMMAND_ID)?;
        let coherent_status =
            loaded_status(KeymapGeneration::initial(), EffectiveKeymapSnapshot::Global);
        let mismatched_generation = loaded_status(
            KeymapGeneration::initial().next(),
            EffectiveKeymapSnapshot::Global,
        );
        let mismatched_snapshot = loaded_status(
            KeymapGeneration::initial(),
            EffectiveKeymapSnapshot::Resolved(context_snapshot(&[("application", "ready")])),
        );

        assert!(matches!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &coherent_status,
                &bound_command_id,
            ),
            PaletteBinding::BoundTo(sequence) if sequence.to_string() == "ctrl-b"
        ));
        assert_eq!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &coherent_status,
                &unbound_command_id,
            ),
            PaletteBinding::Unbound
        );
        assert_eq!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &mismatched_generation,
                &bound_command_id,
            ),
            PaletteBinding::UnmaterializableStateDimensions
        );
        assert_eq!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &mismatched_snapshot,
                &bound_command_id,
            ),
            PaletteBinding::UnmaterializableStateDimensions
        );
        Ok(())
    }

    #[test]
    fn resolved_generation_and_snapshot_matches_control_bound_outcomes() -> Result<(), String> {
        let active_snapshot =
            context_snapshot(&[("application", "ready"), ("interaction", "editing")]);
        let keymap_bindings =
            loaded_bindings(EffectiveKeymapSnapshot::Resolved(active_snapshot.clone()))?;
        let active_context =
            ActiveKeymapContext::from(ActiveKeymapContextState::Resolved(active_snapshot.clone()));
        let bound_command_id = command_id(BOUND_COMMAND_ID)?;
        let coherent_status = loaded_status(
            KeymapGeneration::initial(),
            EffectiveKeymapSnapshot::Resolved(active_snapshot),
        );
        let mismatched_generation = loaded_status(
            KeymapGeneration::initial().next(),
            EffectiveKeymapSnapshot::Resolved(context_snapshot(&[
                ("application", "ready"),
                ("interaction", "editing"),
            ])),
        );
        let mismatched_snapshot = loaded_status(
            KeymapGeneration::initial(),
            EffectiveKeymapSnapshot::Resolved(context_snapshot(&[
                ("application", "ready"),
                ("interaction", "resting"),
            ])),
        );

        assert!(matches!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &coherent_status,
                &bound_command_id,
            ),
            PaletteBinding::BoundTo(sequence) if sequence.to_string() == "ctrl-b"
        ));
        assert_eq!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &mismatched_generation,
                &bound_command_id,
            ),
            PaletteBinding::UnmaterializableStateDimensions
        );
        assert_eq!(
            palette_binding(
                &keymap_bindings,
                &active_context,
                &mismatched_snapshot,
                &bound_command_id,
            ),
            PaletteBinding::UnmaterializableStateDimensions
        );
        Ok(())
    }
}
