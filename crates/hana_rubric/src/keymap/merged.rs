//! Effective keymap layering before matcher construction.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::CompiledKeymap;
use super::KeymapGeneration;
use super::document::BindingEdit;
use super::document::BindingSource;
use crate::Capability;
use crate::CommandId;
use crate::CommandLookup;
use crate::CommandRegistry;
use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;
use crate::KeystrokeSequence;

/// The user keymap layered over the embedded defaults, or its absence.
pub(crate) enum UserKeymap {
    Layered {
        origin:   DiagnosticOrigin,
        contents: String,
    },
    DefaultsOnly,
}

/// The keymap source layer that authored a resolved binding.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum BindingSourceLayer {
    ShippedDefault,
    User,
}

#[derive(Clone)]
enum ResolvedEdit {
    Bind(Box<ResolvedBinding>),
    Tombstone,
}

#[derive(Clone)]
struct ResolvedBinding {
    command_id:        CommandId,
    source:            BindingSource,
    source_layer:      BindingSourceLayer,
    diagnostic_origin: DiagnosticOrigin,
}

/// Every source layer's edit at one keystroke identity, newest source first.
///
/// Keeping the shipped edit under a user edit lets held-prefix rejection remove
/// the offending user edit and restore the shipped binding exactly.
#[derive(Clone)]
enum LayeredEdit {
    ShippedDefault(ResolvedEdit),
    User(ResolvedEdit),
    UserOverShippedDefault {
        user:            ResolvedEdit,
        shipped_default: ResolvedEdit,
    },
}

impl LayeredEdit {
    const fn from_layer(source_layer: BindingSourceLayer, resolved_edit: ResolvedEdit) -> Self {
        match source_layer {
            BindingSourceLayer::ShippedDefault => Self::ShippedDefault(resolved_edit),
            BindingSourceLayer::User => Self::User(resolved_edit),
        }
    }

    fn apply(&mut self, source_layer: BindingSourceLayer, resolved_edit: ResolvedEdit) {
        *self = match (source_layer, &*self) {
            (BindingSourceLayer::ShippedDefault, Self::ShippedDefault(_) | Self::User(_)) => {
                Self::ShippedDefault(resolved_edit)
            },
            (BindingSourceLayer::ShippedDefault, Self::UserOverShippedDefault { user, .. }) => {
                Self::UserOverShippedDefault {
                    user:            user.clone(),
                    shipped_default: resolved_edit,
                }
            },
            (BindingSourceLayer::User, Self::User(_)) => Self::User(resolved_edit),
            (
                BindingSourceLayer::User,
                Self::ShippedDefault(shipped_default)
                | Self::UserOverShippedDefault {
                    shipped_default, ..
                },
            ) => Self::UserOverShippedDefault {
                user:            resolved_edit,
                shipped_default: shipped_default.clone(),
            },
        };
    }

    const fn live(&self) -> &ResolvedEdit {
        match self {
            Self::ShippedDefault(edit) | Self::User(edit) => edit,
            Self::UserOverShippedDefault { user, .. } => user,
        }
    }

    fn reject_layer(&mut self, source_layer: BindingSourceLayer) -> BindingRetention {
        match (source_layer, &mut *self) {
            (BindingSourceLayer::ShippedDefault, Self::ShippedDefault(_))
            | (BindingSourceLayer::User, Self::User(_)) => BindingRetention::Unbound,
            (
                BindingSourceLayer::User,
                Self::UserOverShippedDefault {
                    shipped_default, ..
                },
            ) => {
                *self = Self::ShippedDefault(shipped_default.clone());
                BindingRetention::Layered
            },
            (BindingSourceLayer::ShippedDefault, Self::UserOverShippedDefault { user, .. }) => {
                *self = Self::User(user.clone());
                BindingRetention::Layered
            },
            (BindingSourceLayer::ShippedDefault, Self::User(_))
            | (BindingSourceLayer::User, Self::ShippedDefault(_)) => BindingRetention::Layered,
        }
    }
}

enum BindingRetention {
    Layered,
    Unbound,
}

/// One accepted edit in a materialization layer before held-prefix validation.
pub(crate) struct StateMaterializedEdit {
    materialization_layer: usize,
    keystroke_sequence:    KeystrokeSequence,
    edit:                  BindingEdit,
    source:                BindingSource,
    source_layer:          BindingSourceLayer,
    diagnostic_origin:     DiagnosticOrigin,
}

impl StateMaterializedEdit {
    pub(super) const fn new(
        materialization_layer: usize,
        keystroke_sequence: KeystrokeSequence,
        edit: BindingEdit,
        source: BindingSource,
        source_layer: BindingSourceLayer,
        diagnostic_origin: DiagnosticOrigin,
    ) -> Self {
        Self {
            materialization_layer,
            keystroke_sequence,
            edit,
            source,
            source_layer,
            diagnostic_origin,
        }
    }

    fn into_parts(self) -> (usize, KeystrokeSequence, BindingSourceLayer, ResolvedEdit) {
        let resolved_edit = match self.edit {
            BindingEdit::Bind(command_id) => ResolvedEdit::Bind(Box::new(ResolvedBinding {
                command_id,
                source: self.source,
                source_layer: self.source_layer,
                diagnostic_origin: self.diagnostic_origin,
            })),
            BindingEdit::Unbind => ResolvedEdit::Tombstone,
        };
        (
            self.materialization_layer,
            self.keystroke_sequence,
            self.source_layer,
            resolved_edit,
        )
    }
}

/// Every effective block's shipped and user edits before one snapshot becomes a matcher.
#[derive(Default)]
struct MaterializedEdits {
    layers: BTreeMap<usize, HashMap<KeystrokeSequence, LayeredEdit>>,
}

impl MaterializedEdits {
    fn apply(
        &mut self,
        materialization_layer: usize,
        keystroke_sequence: KeystrokeSequence,
        source_layer: BindingSourceLayer,
        resolved_edit: ResolvedEdit,
    ) {
        match self
            .layers
            .entry(materialization_layer)
            .or_default()
            .entry(keystroke_sequence)
        {
            Entry::Occupied(mut occupied) => occupied.get_mut().apply(source_layer, resolved_edit),
            Entry::Vacant(vacant) => {
                vacant.insert(LayeredEdit::from_layer(source_layer, resolved_edit));
            },
        }
    }

    fn reject(&mut self, rejected: &MaterializedRejectedBinding) {
        let Some(layer) = self.layers.get_mut(&rejected.materialization_layer) else {
            return;
        };
        let Entry::Occupied(mut occupied) = layer.entry(rejected.keystroke_sequence.clone()) else {
            return;
        };
        if matches!(
            occupied.get_mut().reject_layer(rejected.source_layer),
            BindingRetention::Unbound
        ) {
            occupied.remove();
        }
    }

    fn held_prefix_bindings(&self) -> Vec<HeldPrefixBinding> {
        let mut effective_bindings = HashMap::new();
        for (materialization_layer, edits) in &self.layers {
            for (keystroke_sequence, layered_edit) in edits {
                match layered_edit.live() {
                    ResolvedEdit::Bind(binding) => {
                        effective_bindings.insert(
                            keystroke_sequence.clone(),
                            (*materialization_layer, *binding.clone()),
                        );
                    },
                    ResolvedEdit::Tombstone => {
                        effective_bindings.remove(keystroke_sequence);
                    },
                }
            }
        }
        effective_bindings
            .into_iter()
            .map(
                |(keystroke_sequence, (materialization_layer, binding))| HeldPrefixBinding {
                    rejection: MaterializedRejectedBinding {
                        materialization_layer,
                        keystroke_sequence: keystroke_sequence.clone(),
                        source_layer: binding.source_layer,
                    },
                    keystroke_sequence,
                    binding,
                },
            )
            .collect()
    }

    fn live_bindings(&self) -> HashMap<KeystrokeSequence, CommandId> {
        let mut bindings = HashMap::new();
        for edits in self.layers.values() {
            for (keystroke_sequence, layered_edit) in edits {
                match layered_edit.live() {
                    ResolvedEdit::Bind(binding) => {
                        bindings.insert(keystroke_sequence.clone(), binding.command_id.clone());
                    },
                    ResolvedEdit::Tombstone => {
                        bindings.remove(keystroke_sequence);
                    },
                }
            }
        }
        bindings
    }
}

#[derive(Clone)]
struct MaterializedRejectedBinding {
    materialization_layer: usize,
    keystroke_sequence:    KeystrokeSequence,
    source_layer:          BindingSourceLayer,
}

#[derive(Clone)]
struct HeldPrefixBinding {
    keystroke_sequence: KeystrokeSequence,
    binding:            ResolvedBinding,
    rejection:          MaterializedRejectedBinding,
}

enum HeldPrefixConflict {
    AcrossSources,
    WithinOneSource,
}

impl HeldPrefixConflict {
    const fn between(held: BindingSourceLayer, other: BindingSourceLayer) -> Self {
        match (held, other) {
            (BindingSourceLayer::ShippedDefault, BindingSourceLayer::User)
            | (BindingSourceLayer::User, BindingSourceLayer::ShippedDefault) => Self::AcrossSources,
            (BindingSourceLayer::ShippedDefault, BindingSourceLayer::ShippedDefault)
            | (BindingSourceLayer::User, BindingSourceLayer::User) => Self::WithinOneSource,
        }
    }
}

/// Live command bindings for one complete materialized state snapshot.
pub(crate) struct MergedKeymap {
    effective_bindings: Vec<(KeystrokeSequence, CommandId)>,
}

impl MergedKeymap {
    const MAX_COMMAND_EDIT_DISTANCE: usize = 3;
    const MAX_COMMAND_SUGGESTIONS: usize = 3;

    /// Resolves one state-dimension materialization for matcher and binding publication.
    pub(crate) fn from_effective_state_bindings(
        state_materialized_edits: Vec<StateMaterializedEdit>,
        command_registry: &CommandRegistry,
    ) -> (Self, Vec<Diagnostic>) {
        let mut materialized_edits = MaterializedEdits::default();
        for materialized_edit in state_materialized_edits {
            let (materialization_layer, keystroke_sequence, source_layer, resolved_edit) =
                materialized_edit.into_parts();
            materialized_edits.apply(
                materialization_layer,
                keystroke_sequence,
                source_layer,
                resolved_edit,
            );
        }
        let mut diagnostics = Vec::new();
        Self::reject_held_prefixes(&mut materialized_edits, command_registry, &mut diagnostics);
        let mut effective_bindings = materialized_edits
            .live_bindings()
            .into_iter()
            .collect::<Vec<_>>();
        effective_bindings.sort_unstable_by(|(left, _), (right, _)| left.structural_cmp(right));
        (Self { effective_bindings }, diagnostics)
    }

    fn reject_held_prefixes(
        materialized_edits: &mut MaterializedEdits,
        command_registry: &CommandRegistry,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let bindings = materialized_edits.held_prefix_bindings();
        let mut rejected_bindings = Vec::new();
        for held_binding in &bindings {
            let is_held = matches!(
                command_registry.lookup(&held_binding.binding.command_id),
                CommandLookup::Found(command) if command.capability == Capability::Held
            );
            if !is_held || held_binding.keystroke_sequence.len() != 1 {
                continue;
            }

            for other_binding in &bindings {
                if other_binding.keystroke_sequence.len() <= 1
                    || other_binding.keystroke_sequence.first()
                        != held_binding.keystroke_sequence.first()
                {
                    continue;
                }

                match HeldPrefixConflict::between(
                    held_binding.binding.source_layer,
                    other_binding.binding.source_layer,
                ) {
                    HeldPrefixConflict::AcrossSources => {
                        diagnostics.push(other_binding.binding.source.diagnostic(
                            &other_binding.binding.diagnostic_origin,
                            other_binding.binding.command_id.to_string(),
                            DiagnosticKind::HeldCommandInSequence,
                            DiagnosticSeverity::Failure,
                            format!(
                                "Multi-stroke binding `{}` in `{}` shares its prefix with hold-to-act binding `{}` in `{}`.",
                                other_binding.binding.command_id,
                                other_binding.binding.diagnostic_origin,
                                held_binding.binding.command_id,
                                held_binding.binding.diagnostic_origin,
                            ),
                        ));
                    },
                    HeldPrefixConflict::WithinOneSource => {
                        diagnostics.push(held_binding.binding.source.diagnostic(
                            &held_binding.binding.diagnostic_origin,
                            held_binding.binding.command_id.to_string(),
                            DiagnosticKind::HeldCommandInSequence,
                            DiagnosticSeverity::Failure,
                            format!(
                                "Hold-to-act command `{}` in `{}` shares its keystroke with multi-stroke binding `{}` in `{}`.",
                                held_binding.binding.command_id,
                                held_binding.binding.diagnostic_origin,
                                other_binding.binding.command_id,
                                other_binding.binding.diagnostic_origin,
                            ),
                        ));
                        rejected_bindings.push(held_binding.rejection.clone());
                    },
                }
                rejected_bindings.push(other_binding.rejection.clone());
            }
        }
        for rejected_binding in &rejected_bindings {
            materialized_edits.reject(rejected_binding);
        }
    }

    /// Constructs the dispatch matcher for one replacement generation.
    #[must_use]
    pub(crate) fn compile(
        &self,
        generation: KeymapGeneration,
        command_registry: &CommandRegistry,
    ) -> CompiledKeymap {
        CompiledKeymap::from_merged(generation, self, command_registry)
    }

    pub(crate) fn closest_command_ids(
        command_id: &CommandId,
        command_registry: &CommandRegistry,
    ) -> Vec<String> {
        let mut candidates = command_registry
            .iter()
            .filter_map(|command_info| {
                bounded_levenshtein(
                    command_id.as_str(),
                    command_info.id.as_str(),
                    Self::MAX_COMMAND_EDIT_DISTANCE,
                )
                .map(|distance| (distance, command_info.id.as_str()))
            })
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        candidates
            .into_iter()
            .take(Self::MAX_COMMAND_SUGGESTIONS)
            .map(|(_, candidate)| candidate.to_owned())
            .collect()
    }

    pub(super) fn effective_bindings(&self) -> &[(KeystrokeSequence, CommandId)] {
        &self.effective_bindings
    }
}

fn bounded_levenshtein(left: &str, right: &str, maximum_distance: usize) -> Option<usize> {
    if left.len().abs_diff(right.len()) > maximum_distance {
        return None;
    }

    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_byte) in left.bytes().enumerate() {
        current[0] = left_index + 1;
        let mut smallest = current[0];
        for (right_index, right_byte) in right.bytes().enumerate() {
            let substitution_cost = usize::from(left_byte != right_byte);
            let distance = (current[right_index] + 1)
                .min(previous[right_index + 1] + 1)
                .min(previous[right_index] + substitution_cost);
            current[right_index + 1] = distance;
            smallest = smallest.min(distance);
        }
        if smallest > maximum_distance {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[right.len()];
    (distance <= maximum_distance).then_some(distance)
}
