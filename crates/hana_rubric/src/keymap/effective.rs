//! Accepted multi-dimensional keymaps and their snapshot-specific materializations.

use bevy::prelude::Resource;

use super::KeymapGeneration;
use super::UserKeymap;
use super::document;
use super::document::Binding;
use super::document::BindingEdit;
use super::document::BindingSource;
use super::document::ContextSource;
use super::document::KeymapBlockScope;
use super::document::KeymapDocument;
use super::document::SourceLocatedStateDimensionPredicate;
use super::merged::BindingSourceLayer;
use super::merged::StateMaterializedEdit;
use crate::ActiveKeymapContextState;
use crate::Capability;
use crate::CommandLookup;
use crate::CommandRegistry;
use crate::ContextDimensionName;
use crate::ContextSnapshot;
use crate::ContextValueName;
use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;
use crate::Keystroke;
use crate::KeystrokeSequence;
use crate::PrimaryTrigger;
use crate::condition::StateDimensionRegistry;

/// A fully parsed, state-vocabulary-validated keymap retained before a runtime snapshot exists.
#[derive(Resource)]
pub(crate) struct AcceptedKeymapDocument {
    global:            Vec<AcceptedBinding>,
    contextual_layers: Vec<AcceptedPredicateLayer>,
}

impl AcceptedKeymapDocument {
    /// Parses and validates both source layers without requiring an active application snapshot.
    pub(crate) fn from_sources(
        defaults_diagnostic_origin: &DiagnosticOrigin,
        defaults_source: &str,
        user_keymap: &UserKeymap,
        command_registry: &CommandRegistry,
        state_dimensions: &StateDimensionRegistry,
        protected_keystrokes: &[Keystroke],
    ) -> Result<(Self, Vec<Diagnostic>), Vec<Diagnostic>> {
        let (defaults, mut diagnostics) =
            KeymapDocument::parse(defaults_diagnostic_origin, defaults_source)?;
        let user = match user_keymap {
            UserKeymap::Layered { origin, contents } => {
                let (document, document_diagnostics) = match KeymapDocument::parse(origin, contents)
                {
                    Ok(parsed) => parsed,
                    Err(mut user_diagnostics) => {
                        diagnostics.append(&mut user_diagnostics);
                        return Err(diagnostics);
                    },
                };
                diagnostics.extend(document_diagnostics);
                Some(document)
            },
            UserKeymap::DefaultsOnly => None,
        };

        let mut global = Vec::new();
        let mut contextual_layers = Vec::new();
        let mut status = if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Failure)
        {
            AcceptanceStatus::Rejected
        } else {
            AcceptanceStatus::Accepted
        };
        let inputs = AcceptanceInputs {
            command_registry,
            state_dimensions,
            protected_keystrokes,
        };
        let mut accepted = AcceptedDocumentParts {
            global:            &mut global,
            contextual_layers: &mut contextual_layers,
            diagnostics:       &mut diagnostics,
            status:            &mut status,
        };
        Self::accept_document(
            &defaults,
            BindingSourceLayer::ShippedDefault,
            &inputs,
            &mut accepted,
        );
        if let Some(user) = user.as_ref() {
            Self::accept_document(user, BindingSourceLayer::User, &inputs, &mut accepted);
        }

        if status == AcceptanceStatus::Rejected {
            return Err(diagnostics);
        }

        Ok((
            Self {
                global,
                contextual_layers,
            },
            diagnostics,
        ))
    }

    fn accept_document(
        document: &KeymapDocument,
        source_layer: BindingSourceLayer,
        inputs: &AcceptanceInputs<'_>,
        accepted: &mut AcceptedDocumentParts<'_>,
    ) {
        for block in &document.blocks {
            accepted
                .diagnostics
                .extend(document::unrecognized_block_member_diagnostics(
                    document, block,
                ));
            let accepted_scope = match &block.scope {
                KeymapBlockScope::Global => AcceptedScope::Global,
                KeymapBlockScope::Invalid => {
                    *accepted.status = AcceptanceStatus::Rejected;
                    continue;
                },
                KeymapBlockScope::Conditional(predicate) => {
                    if validate_predicate(
                        document,
                        predicate,
                        inputs.state_dimensions,
                        accepted.diagnostics,
                    ) == AcceptanceStatus::Rejected
                    {
                        *accepted.status = AcceptanceStatus::Rejected;
                        continue;
                    }
                    AcceptedScope::Conditional(predicate.clone())
                },
            };
            let bindings = block
                .bindings
                .iter()
                .filter_map(|binding| {
                    validate_binding(
                        binding,
                        &document.diagnostic_origin,
                        source_layer,
                        inputs.command_registry,
                        inputs.protected_keystrokes,
                        accepted.diagnostics,
                    )
                })
                .collect::<Vec<_>>();
            match accepted_scope {
                AcceptedScope::Global => accepted.global.extend(bindings),
                AcceptedScope::Conditional(predicate) => {
                    accepted.contextual_layers.push(AcceptedPredicateLayer {
                        predicate,
                        bindings,
                    });
                },
            }
        }
    }

    /// Applies the retained global base then matching contextual layers in authored order.
    pub(crate) fn materialize(
        &self,
        snapshot: EffectiveKeymapSnapshot,
    ) -> EffectiveKeymapMaterialization {
        let mut bindings = Vec::new();
        apply_bindings(&mut bindings, 0, &self.global);
        let mut matched_layers = Vec::new();
        if let EffectiveKeymapSnapshot::Resolved(context_snapshot) = &snapshot {
            for (index, layer) in self.contextual_layers.iter().enumerate() {
                if layer.predicate.matches(context_snapshot) {
                    apply_bindings(&mut bindings, matched_layers.len() + 1, &layer.bindings);
                    matched_layers.push(MatchedPredicateLayer {
                        document_order: index,
                        predicate:      StateDimensionPredicateIdentity::from(&layer.predicate),
                    });
                }
            }
        }
        EffectiveKeymapMaterialization {
            bindings,
            publication: EffectiveKeymapPublication {
                generation: KeymapGeneration::initial(),
                snapshot,
                matched_layers,
            },
        }
    }
}

struct AcceptanceInputs<'inputs> {
    command_registry:     &'inputs CommandRegistry,
    state_dimensions:     &'inputs StateDimensionRegistry,
    protected_keystrokes: &'inputs [Keystroke],
}

struct AcceptedDocumentParts<'parts> {
    global:            &'parts mut Vec<AcceptedBinding>,
    contextual_layers: &'parts mut Vec<AcceptedPredicateLayer>,
    diagnostics:       &'parts mut Vec<Diagnostic>,
    status:            &'parts mut AcceptanceStatus,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum AcceptanceStatus {
    Accepted,
    Rejected,
}

enum AcceptedScope {
    Global,
    Conditional(SourceLocatedStateDimensionPredicate),
}

struct AcceptedPredicateLayer {
    predicate: SourceLocatedStateDimensionPredicate,
    bindings:  Vec<AcceptedBinding>,
}

#[derive(Clone)]
struct AcceptedBinding {
    keystroke_sequence: KeystrokeSequence,
    edit:               BindingEdit,
    source:             BindingSource,
    source_layer:       BindingSourceLayer,
    diagnostic_origin:  DiagnosticOrigin,
}

impl AcceptedBinding {
    fn from_binding(
        binding: &Binding,
        source_layer: BindingSourceLayer,
        diagnostic_origin: &DiagnosticOrigin,
    ) -> Self {
        Self {
            keystroke_sequence: binding.keystroke_sequence.clone(),
            edit: binding.edit.clone(),
            source: binding.source.clone(),
            source_layer,
            diagnostic_origin: diagnostic_origin.clone(),
        }
    }
}

fn apply_bindings(
    effective: &mut Vec<StateMaterializedEdit>,
    materialization_layer: usize,
    bindings: &[AcceptedBinding],
) {
    for binding in bindings {
        effective.push(StateMaterializedEdit::new(
            materialization_layer,
            binding.keystroke_sequence.clone(),
            binding.edit.clone(),
            binding.source.clone(),
            binding.source_layer,
            binding.diagnostic_origin.clone(),
        ));
    }
}

fn validate_predicate(
    document: &KeymapDocument,
    predicate: &SourceLocatedStateDimensionPredicate,
    state_dimensions: &StateDimensionRegistry,
    diagnostics: &mut Vec<Diagnostic>,
) -> AcceptanceStatus {
    let vocabulary = state_dimensions.metadata();
    let mut status = AcceptanceStatus::Accepted;
    for (dimension, value, term_source) in predicate.terms() {
        let Some(registered_dimension) = vocabulary
            .iter()
            .find(|registered_dimension| registered_dimension.name == dimension)
        else {
            let registered_dimensions = vocabulary
                .iter()
                .map(|registered_dimension| registered_dimension.name.as_str())
                .collect::<Vec<_>>();
            let message = if registered_dimensions.is_empty() {
                format!(
                    "State dimension `{}` with value `{}` is unavailable because this application accepts global keymap blocks only.",
                    dimension.as_str(),
                    value.as_str(),
                )
            } else {
                format!(
                    "State dimension `{}` with value `{}` is not registered. Registered dimensions: {}.",
                    dimension.as_str(),
                    value.as_str(),
                    registered_dimensions.join(", ")
                )
            };
            diagnostics.push(predicate_diagnostic(
                document,
                &term_source.name,
                format!("{}={}", dimension.as_str(), value.as_str()),
                message,
                registered_dimensions
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            ));
            status = AcceptanceStatus::Rejected;
            continue;
        };
        if state_dimensions.resolves(dimension, value) {
            continue;
        }
        let values = registered_dimension
            .values
            .iter()
            .map(|registered_value| registered_value.name.as_str())
            .collect::<Vec<_>>();
        diagnostics.push(predicate_diagnostic(
            document,
            &term_source.value,
            format!("{}={}", dimension.as_str(), value.as_str()),
            format!(
                "State value `{}` is not declared by dimension `{}`. Declared values: {}.",
                value.as_str(),
                dimension.as_str(),
                values.join(", ")
            ),
            values.into_iter().map(str::to_owned).collect(),
        ));
        status = AcceptanceStatus::Rejected;
    }
    status
}

fn predicate_diagnostic(
    document: &KeymapDocument,
    source: &ContextSource,
    subject: String,
    message: String,
    suggestions: Vec<String>,
) -> Diagnostic {
    Diagnostic {
        origin: document.diagnostic_origin.clone(),
        byte_range: source.byte_range.clone(),
        line: source.line,
        column: source.column,
        block_index: source.block_index,
        context: subject,
        original_keystroke: String::new(),
        command_id: String::new(),
        kind: DiagnosticKind::Context,
        severity: DiagnosticSeverity::Failure,
        message,
        suggestions,
    }
}

fn validate_binding(
    binding: &Binding,
    diagnostic_origin: &DiagnosticOrigin,
    source_layer: BindingSourceLayer,
    command_registry: &CommandRegistry,
    protected_keystrokes: &[Keystroke],
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<AcceptedBinding> {
    if let Some(protected_keystroke) = protected_keystrokes
        .iter()
        .find(|protected_keystroke| **protected_keystroke == binding.keystroke_sequence.first())
    {
        diagnostics.push(binding.source.diagnostic(
            diagnostic_origin,
            String::new(),
            DiagnosticKind::ReservedKeystroke,
            DiagnosticSeverity::Failure,
            format!(
                "Keystroke 1 `{protected_keystroke}` is reserved for the application's recovery command and cannot start a keymap sequence."
            ),
        ));
        return None;
    }
    let BindingEdit::Bind(command_id) = &binding.edit else {
        return Some(AcceptedBinding::from_binding(
            binding,
            source_layer,
            diagnostic_origin,
        ));
    };
    let CommandLookup::Found(command) = command_registry.lookup(command_id) else {
        let suggestions = super::MergedKeymap::closest_command_ids(command_id, command_registry);
        let message = suggestions.first().map_or_else(
            || format!("Command `{command_id}` is not registered."),
            |suggestion| {
                format!("Command `{command_id}` is not registered. Did you mean `{suggestion}`?")
            },
        );
        let mut diagnostic = binding.source.command_diagnostic(
            diagnostic_origin,
            command_id.to_string(),
            DiagnosticKind::Command,
            DiagnosticSeverity::Failure,
            message,
        );
        diagnostic.suggestions = suggestions;
        diagnostics.push(diagnostic);
        return None;
    };
    if let Some((index, modifier_family)) =
        binding
            .keystroke_sequence
            .iter()
            .enumerate()
            .find_map(|(index, keystroke)| match keystroke.primary_trigger() {
                PrimaryTrigger::ModifierFamily(modifier_family) => Some((index, modifier_family)),
                PrimaryTrigger::OrdinaryKey(_) => None,
            })
        && (command.capability != Capability::Held || binding.keystroke_sequence.len() != 1)
    {
        diagnostics.push(binding.source.diagnostic(
            diagnostic_origin,
            command_id.to_string(),
            DiagnosticKind::BareModifierRequiresHeldCommand,
            DiagnosticSeverity::Failure,
            format!(
                "Bare modifier keystroke {} `{modifier_family}` can only be the sole keystroke bound to a hold-to-act command.",
                index + 1
            ),
        ));
        return None;
    }
    if command.capability == Capability::Held && binding.keystroke_sequence.len() > 1 {
        diagnostics.push(binding.source.diagnostic(
            diagnostic_origin,
            command_id.to_string(),
            DiagnosticKind::HeldCommandInSequence,
            DiagnosticSeverity::Failure,
            format!("Hold-to-act command `{command_id}` must use exactly one keystroke."),
        ));
        return None;
    }
    if command.capability == Capability::Unremappable {
        diagnostics.push(binding.source.diagnostic(
            diagnostic_origin,
            command_id.to_string(),
            DiagnosticKind::UnremappableCommand,
            DiagnosticSeverity::Failure,
            format!("Command `{command_id}` is reserved for recovery."),
        ));
        return None;
    }
    Some(AcceptedBinding::from_binding(
        binding,
        source_layer,
        diagnostic_origin,
    ))
}

/// A materialization-ready snapshot identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectiveKeymapSnapshot {
    /// No state dimensions are registered, so the document's global base is effective.
    Global,
    /// Every registered dimension has a resolved state value.
    Resolved(ContextSnapshot),
}

/// The canonical identity of one matched predicate layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateDimensionPredicateIdentity {
    /// Conjunctive state-dimension/value pairs in dimension-name order.
    pub terms: Vec<(ContextDimensionName, ContextValueName)>,
}

impl From<&SourceLocatedStateDimensionPredicate> for StateDimensionPredicateIdentity {
    fn from(predicate: &SourceLocatedStateDimensionPredicate) -> Self {
        Self {
            terms: predicate
                .terms()
                .map(|(dimension, value, _)| (dimension.clone(), value.clone()))
                .collect(),
        }
    }
}

/// One contextual layer that contributed to an effective keymap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchedPredicateLayer {
    /// Zero-based order among accepted contextual layers.
    pub document_order: usize,
    /// Canonical predicate identity used for this layer match.
    pub predicate:      StateDimensionPredicateIdentity,
}

/// The source snapshot and ordered predicate provenance of an effective keymap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveKeymapPublication {
    /// The dispatch and palette generation materialized from this snapshot.
    pub generation:     KeymapGeneration,
    /// The global-only or complete resolved snapshot that was materialized.
    pub snapshot:       EffectiveKeymapSnapshot,
    /// Every matching contextual layer in its applied document order.
    pub matched_layers: Vec<MatchedPredicateLayer>,
}

/// The state of effective-table availability.
#[derive(Clone, Debug, Default, Eq, PartialEq, Resource)]
pub enum EffectiveKeymapStatus {
    /// No fully accepted document has been retained.
    #[default]
    AwaitingAcceptedDocument,
    /// The initial document was rejected before any accepted document existed.
    RejectedInitialDocument,
    /// A document is accepted but one or more typed states have not reported yet.
    AwaitingStateDimensions,
    /// A document is accepted but these application-owned state resources are unavailable.
    StateDimensionsUnavailable {
        /// Sorted missing state-dimension names.
        missing: Vec<ContextDimensionName>,
    },
    /// A reflected resolved snapshot does not match the registered typed vocabulary.
    UnmaterializableStateDimensions,
    /// Dispatch and palette tables were materialized from this exact publication record.
    Loaded(EffectiveKeymapPublication),
}

impl EffectiveKeymapStatus {
    pub(super) fn inactive(active_context: &ActiveKeymapContextState) -> Self {
        match active_context {
            ActiveKeymapContextState::GlobalRouting | ActiveKeymapContextState::Resolved(_) => {
                Self::AwaitingAcceptedDocument
            },
            ActiveKeymapContextState::AwaitingStateDimensions => Self::AwaitingStateDimensions,
            ActiveKeymapContextState::StateDimensionsUnavailable { missing } => {
                Self::StateDimensionsUnavailable {
                    missing: missing.clone(),
                }
            },
        }
    }
}

pub(crate) struct EffectiveKeymapMaterialization {
    pub(super) bindings:    Vec<StateMaterializedEdit>,
    pub(super) publication: EffectiveKeymapPublication,
}
