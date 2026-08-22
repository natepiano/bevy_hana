//! JSONC keymap decoding with source locations for every authored binding.

#[cfg(test)]
use std::cell::Cell;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fmt;
use std::fmt::Formatter;
use std::ops::Range;

use serde::Deserialize;
use serde::de::MapAccess;
use serde::de::Visitor;
use serde_json_lenient::Error;
use serde_json_lenient::Value;

use crate::CommandId;
use crate::ContextDimensionName;
use crate::ContextSnapshot;
use crate::ContextValueName;
use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;
use crate::KeystrokeSequence;
use crate::KeystrokeSequenceParseError;

const RECOGNIZED_ROOT_MEMBERS: [&str; 2] = ["$schema", "bindings"];

#[cfg(test)]
thread_local! {
    static PREDICATE_MATCH_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_predicate_match_count() { PREDICATE_MATCH_COUNT.with(|count| count.set(0)); }

#[cfg(test)]
pub(crate) fn predicate_match_count() -> usize { PREDICATE_MATCH_COUNT.with(Cell::get) }

#[cfg(test)]
fn record_predicate_match() { PREDICATE_MATCH_COUNT.with(|count| count.set(count.get() + 1)); }

/// A parsed keymap document and the source information needed by later keymap stages.
#[derive(Debug)]
pub(super) struct KeymapDocument {
    #[expect(
        dead_code,
        reason = "the parsed schema reference is intentionally unread after validation"
    )]
    pub(super) schema:            Option<String>,
    pub(super) diagnostic_origin: DiagnosticOrigin,
    pub(super) blocks:            Vec<KeymapBlock>,
}

impl KeymapDocument {
    /// Parses one JSONC keymap document and retains each binding's authored source location.
    pub(super) fn parse(
        diagnostic_origin: &DiagnosticOrigin,
        source: &str,
    ) -> Result<(Self, Vec<Diagnostic>), Vec<Diagnostic>> {
        let wire_document = match serde_json_lenient::from_str::<WireDocument>(source) {
            Ok(wire_document) => wire_document,
            Err(error) => {
                if root_value_starts_with_array(source) {
                    return Err(vec![document_diagnostic(
                        diagnostic_origin,
                        source,
                        root_value_offset(source),
                        DiagnosticKind::Syntax,
                        "Keymap documents require an object envelope with a `bindings` member."
                            .to_owned(),
                    )]);
                }

                return Err(vec![serde_diagnostic(diagnostic_origin, source, error)]);
            },
        };
        let source_index = match SourceIndex::parse(source) {
            Ok(source_index) => source_index,
            Err(error) => {
                return Err(vec![document_diagnostic(
                    diagnostic_origin,
                    source,
                    error.offset,
                    DiagnosticKind::Syntax,
                    error.message,
                )]);
            },
        };

        let SourceIndex {
            blocks: indexed_blocks,
            unrecognized_root_members,
        } = source_index;
        let mut diagnostics = unrecognized_root_members
            .into_iter()
            .map(|member| root_member_diagnostic(diagnostic_origin, member))
            .collect();
        let blocks = parse_blocks(
            diagnostic_origin,
            source,
            wire_document.bindings,
            indexed_blocks,
            &mut diagnostics,
        )?;

        Ok((
            Self {
                schema: wire_document.schema,
                diagnostic_origin: diagnostic_origin.clone(),
                blocks,
            },
            diagnostics,
        ))
    }
}

fn root_member_diagnostic(
    diagnostic_origin: &DiagnosticOrigin,
    member: IndexedRootMember,
) -> Diagnostic {
    let suggestion = RECOGNIZED_ROOT_MEMBERS
        .iter()
        .min_by_key(|recognized_member| {
            levenshtein_distance(member.name.as_str(), recognized_member)
        })
        .copied()
        .unwrap_or("bindings");

    Diagnostic {
        origin:             diagnostic_origin.clone(),
        byte_range:         member.location.byte_range,
        line:               member.location.line,
        column:             member.location.column,
        block_index:        0,
        context:            ContextDiagnosticSubject::Document.text(),
        original_keystroke: String::new(),
        command_id:         String::new(),
        kind:               DiagnosticKind::Syntax,
        severity:           DiagnosticSeverity::Advisory,
        message:            format!(
            "Unrecognized keymap document member `{}`. Did you mean `{suggestion}`?",
            member.name
        ),
        suggestions:        vec![suggestion.to_owned()],
    }
}

/// Builds the advisory diagnostics for unrecognized members of one parsed keymap block.
pub(super) fn unrecognized_block_member_diagnostics(
    document: &KeymapDocument,
    block: &KeymapBlock,
) -> Vec<Diagnostic> {
    block
        .unrecognized_members
        .iter()
        .map(|member| {
            let suggestion = ["context", "bindings"]
                .into_iter()
                .min_by_key(|recognized_member| {
                    levenshtein_distance(member.name.as_str(), recognized_member)
                })
                .unwrap_or("bindings");
            Diagnostic {
                origin:             document.diagnostic_origin.clone(),
                byte_range:         member.byte_range.clone(),
                line:               member.line,
                column:             member.column,
                block_index:        member.block_index,
                context:            ContextDiagnosticSubject::Document.text(),
                original_keystroke: String::new(),
                command_id:         String::new(),
                kind:               DiagnosticKind::Syntax,
                severity:           DiagnosticSeverity::Advisory,
                message:            format!(
                    "Unrecognized keymap block member `{}`. Did you mean `{suggestion}`?",
                    member.name
                ),
                suggestions:        vec![suggestion.to_owned()],
            }
        })
        .collect()
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let mut previous = (0..=right.len()).collect::<Vec<_>>();

    for (left_index, left_character) in left.chars().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        for (right_index, right_character) in right.chars().enumerate() {
            let replace = previous[right_index] + usize::from(left_character != right_character);
            let insert = current[right_index] + 1;
            let delete = previous[right_index + 1] + 1;
            current.push(replace.min(insert).min(delete));
        }
        previous = current;
    }

    previous[right.len()]
}

fn parse_blocks(
    diagnostic_origin: &DiagnosticOrigin,
    source: &str,
    wire_blocks: Vec<WireBlock>,
    indexed_blocks: Vec<IndexedBlock>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<KeymapBlock>, Vec<Diagnostic>> {
    if wire_blocks.len() != indexed_blocks.len() {
        return Err(vec![document_diagnostic(
            diagnostic_origin,
            source,
            0,
            DiagnosticKind::Syntax,
            format!(
                "Internal keymap parsing inconsistency: serde decoded {} block(s), but source indexing recorded {}.",
                wire_blocks.len(),
                indexed_blocks.len(),
            ),
        )]);
    }

    let mut blocks = Vec::with_capacity(wire_blocks.len());

    for (block_index, (wire_block, indexed_block)) in
        wire_blocks.into_iter().zip(indexed_blocks).enumerate()
    {
        blocks.push(parse_block(
            diagnostic_origin,
            source,
            wire_block,
            indexed_block,
            block_index,
            diagnostics,
        )?);
    }

    Ok(blocks)
}

fn parse_block(
    diagnostic_origin: &DiagnosticOrigin,
    source: &str,
    wire_block: WireBlock,
    indexed_block: IndexedBlock,
    block_index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<KeymapBlock, Vec<Diagnostic>> {
    let IndexedBlock {
        context: indexed_context,
        bindings: indexed_bindings,
        unrecognized_members,
    } = indexed_block;
    let scope = parse_context(
        diagnostic_origin,
        source,
        wire_block.context,
        indexed_context,
        block_index,
        diagnostics,
    )?;
    let context_subject = scope.diagnostic_subject();
    let bindings = parse_bindings(
        diagnostic_origin,
        source,
        wire_block.bindings,
        indexed_bindings,
        block_index,
        context_subject,
        diagnostics,
    )?;
    let unrecognized_members = unrecognized_members
        .into_iter()
        .map(|member| UnrecognizedBlockMember {
            name: member.name,
            byte_range: member.location.byte_range,
            line: member.location.line,
            column: member.location.column,
            block_index,
        })
        .collect();

    Ok(KeymapBlock {
        scope,
        bindings,
        unrecognized_members,
    })
}

fn parse_context(
    diagnostic_origin: &DiagnosticOrigin,
    source: &str,
    wire_context: WireContext,
    indexed_context: IndexedContext,
    block_index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<KeymapBlockScope, Vec<Diagnostic>> {
    match (wire_context, indexed_context) {
        (WireContext::Absent, IndexedContext::Absent) => Ok(KeymapBlockScope::Global),
        (WireContext::Object(members), IndexedContext::Object { location, terms }) => {
            Ok(parse_context_object(
                diagnostic_origin,
                ContextSource::new(location, block_index),
                members,
                terms,
                diagnostics,
            ))
        },
        (WireContext::Null, IndexedContext::Value(location)) => {
            let source = ContextSource::new(location, block_index);
            diagnostics.push(context_diagnostic(
                diagnostic_origin,
                &source,
                ContextDiagnosticSubject::InvalidContext,
                DiagnosticKind::Syntax,
                DiagnosticSeverity::Failure,
                "A keymap block `context` value must be an object, not null.".to_owned(),
            ));
            Ok(KeymapBlockScope::Invalid)
        },
        (WireContext::Other, IndexedContext::Value(location)) => {
            let source = ContextSource::new(location, block_index);
            diagnostics.push(context_diagnostic(
                diagnostic_origin,
                &source,
                ContextDiagnosticSubject::InvalidContext,
                DiagnosticKind::Syntax,
                DiagnosticSeverity::Failure,
                "A keymap block `context` value must be an object of state dimensions."
                    .to_owned(),
            ));
            Ok(KeymapBlockScope::Invalid)
        },
        _ => Err(vec![document_diagnostic(
            diagnostic_origin,
            source,
            0,
            DiagnosticKind::Syntax,
            "Internal keymap parsing inconsistency: serde and source indexing disagreed about a block `context` member.".to_owned(),
        )]),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ContextObjectValidity {
    Valid,
    Invalid,
}

fn parse_context_object(
    diagnostic_origin: &DiagnosticOrigin,
    context_source: ContextSource,
    members: Vec<(String, Value)>,
    indexed_terms: Vec<IndexedContextTerm>,
    diagnostics: &mut Vec<Diagnostic>,
) -> KeymapBlockScope {
    if members.len() != indexed_terms.len() {
        diagnostics.push(context_diagnostic(
            diagnostic_origin,
            &context_source,
            ContextDiagnosticSubject::InvalidContext,
            DiagnosticKind::Syntax,
            DiagnosticSeverity::Failure,
            "Internal keymap parsing inconsistency: serde and source indexing recorded different context-member counts.".to_owned(),
        ));
        return KeymapBlockScope::Invalid;
    }
    let mut terms = BTreeMap::new();
    let mut validity = ContextObjectValidity::Valid;
    for ((dimension, value), indexed_term) in members.into_iter().zip(indexed_terms) {
        let term_source = ContextTermSource::new(indexed_term, context_source.block_index);
        let Value::String(value) = value else {
            diagnostics.push(context_diagnostic(
                diagnostic_origin,
                &term_source.value,
                ContextDiagnosticSubject::authored_name_or_invalid_context(dimension),
                DiagnosticKind::Syntax,
                DiagnosticSeverity::Failure,
                "A keymap context value must be a state-value string.".to_owned(),
            ));
            validity = ContextObjectValidity::Invalid;
            continue;
        };
        let dimension_name = ContextDimensionName::new(dimension);
        let value_name = ContextValueName::new(value);
        if terms
            .insert(
                dimension_name.clone(),
                SourceLocatedStateDimensionTerm {
                    value:  value_name.clone(),
                    source: term_source.clone(),
                },
            )
            .is_some()
        {
            diagnostics.push(context_diagnostic(
                diagnostic_origin,
                &term_source.name,
                ContextDiagnosticSubject::PredicateTerm {
                    dimension: dimension_name.as_str().to_owned(),
                    value:     value_name.as_str().to_owned(),
                },
                DiagnosticKind::Context,
                DiagnosticSeverity::Failure,
                format!(
                    "State dimension `{}` with value `{}` appears more than once in this context predicate.",
                    dimension_name.as_str(),
                    value_name.as_str(),
                ),
            ));
            validity = ContextObjectValidity::Invalid;
        }
    }
    if terms.is_empty() {
        diagnostics.push(context_diagnostic(
            diagnostic_origin,
            &context_source,
            ContextDiagnosticSubject::InvalidContext,
            DiagnosticKind::Context,
            DiagnosticSeverity::Failure,
            "A keymap context object must name at least one state dimension.".to_owned(),
        ));
        return KeymapBlockScope::Invalid;
    }
    if validity == ContextObjectValidity::Invalid {
        return KeymapBlockScope::Invalid;
    }
    KeymapBlockScope::Conditional(SourceLocatedStateDimensionPredicate { terms })
}

fn parse_bindings(
    diagnostic_origin: &DiagnosticOrigin,
    source: &str,
    wire_bindings: WireBindings,
    locations: Vec<IndexedBinding>,
    block_index: usize,
    context: ContextDiagnosticSubject,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<Binding>, Vec<Diagnostic>> {
    if wire_bindings.0.len() != locations.len() {
        return Err(vec![document_diagnostic(
            diagnostic_origin,
            source,
            0,
            DiagnosticKind::Syntax,
            format!(
                "Internal keymap parsing inconsistency: serde decoded {} binding(s) in block {block_index}, but source indexing recorded {}.",
                wire_bindings.0.len(),
                locations.len(),
            ),
        )]);
    }

    let mut binding_names = HashSet::new();
    let mut bindings = Vec::with_capacity(wire_bindings.0.len());

    for ((original_keystroke, wire_value), locations) in wire_bindings.0.into_iter().zip(locations)
    {
        let binding_source = BindingSource::new(
            locations.key,
            locations.value,
            block_index,
            context.clone(),
            original_keystroke.clone(),
        );

        if !binding_names.insert(original_keystroke.clone()) {
            diagnostics.push(binding_source.diagnostic(
                diagnostic_origin,
                String::new(),
                DiagnosticKind::Syntax,
                DiagnosticSeverity::Advisory,
                format!(
                    "Keymap block {block_index} declares `{original_keystroke}` more than once."
                ),
            ));
        }

        let keystroke_sequence = match original_keystroke.parse::<KeystrokeSequence>() {
            Ok(keystroke_sequence) => keystroke_sequence,
            Err(error) => {
                diagnostics.push(binding_source.keystroke_diagnostic(
                    diagnostic_origin,
                    source,
                    &error,
                ));
                continue;
            },
        };
        let edit = match parse_binding_edit(diagnostic_origin, &binding_source, wire_value) {
            BindingEditResult::Edit(edit) => edit,
            BindingEditResult::Diagnostic(diagnostic) => {
                diagnostics.push(diagnostic);
                continue;
            },
        };

        bindings.push(Binding {
            keystroke_sequence,
            edit,
            source: binding_source,
        });
    }

    Ok(bindings)
}

/// One ordered block from a [`KeymapDocument`].
#[derive(Clone, Debug)]
pub(super) struct KeymapBlock {
    pub(super) scope:                KeymapBlockScope,
    pub(super) bindings:             Vec<Binding>,
    pub(super) unrecognized_members: Vec<UnrecognizedBlockMember>,
}

/// One unrecognized keymap block member retained for advisory diagnostics.
#[derive(Clone, Debug)]
pub(super) struct UnrecognizedBlockMember {
    pub(super) name:        String,
    pub(super) byte_range:  Range<usize>,
    pub(super) line:        usize,
    pub(super) column:      usize,
    pub(super) block_index: usize,
}

/// One parsed keymap binding and its location in the source document.
#[derive(Clone, Debug)]
pub(super) struct Binding {
    pub(super) keystroke_sequence: KeystrokeSequence,
    pub(super) edit:               BindingEdit,
    pub(super) source:             BindingSource,
}

/// One binding change authored in a [`KeymapDocument`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BindingEdit {
    /// Binds a keystroke sequence to a command.
    Bind(CommandId),
    /// Removes a binding inherited from an earlier block.
    Unbind,
}

/// The source location for one parsed `context` member.
#[derive(Clone, Debug)]
pub(super) struct ContextSource {
    pub(super) byte_range:  Range<usize>,
    pub(super) line:        usize,
    pub(super) column:      usize,
    pub(super) block_index: usize,
}

impl ContextSource {
    const fn new(location: SourceLocation, block_index: usize) -> Self {
        Self {
            byte_range: location.byte_range,
            line: location.line,
            column: location.column,
            block_index,
        }
    }
}

/// The semantic scope for one keymap block.
///
/// Conditional predicates are canonicalized by state-dimension name while the surrounding
/// document retains its authored block order for precedence.
#[derive(Clone, Debug)]
pub(super) enum KeymapBlockScope {
    Global,
    Conditional(SourceLocatedStateDimensionPredicate),
    /// A present but malformed `context` member that cannot become global by accident.
    Invalid,
}

impl KeymapBlockScope {
    fn diagnostic_subject(&self) -> ContextDiagnosticSubject {
        match self {
            Self::Global => ContextDiagnosticSubject::Global,
            Self::Conditional(predicate) => ContextDiagnosticSubject::Predicate(
                predicate
                    .terms()
                    .map(|(dimension, value, _)| {
                        (dimension.as_str().to_owned(), value.as_str().to_owned())
                    })
                    .collect(),
            ),
            Self::Invalid => ContextDiagnosticSubject::InvalidContext,
        }
    }
}

/// A nonempty subject for source diagnostics attached to a keymap block.
#[derive(Clone, Debug)]
enum ContextDiagnosticSubject {
    Document,
    Global,
    InvalidContext,
    AuthoredName(NonemptyContextDiagnosticSubject),
    Predicate(Vec<(String, String)>),
    PredicateTerm {
        dimension: String,
        value:     String,
    },
}

impl ContextDiagnosticSubject {
    fn authored_name_or_invalid_context(name: String) -> Self {
        if name.is_empty() {
            Self::InvalidContext
        } else {
            Self::AuthoredName(NonemptyContextDiagnosticSubject(name))
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Document => "document".to_owned(),
            Self::Global => "global".to_owned(),
            Self::InvalidContext => "context".to_owned(),
            Self::AuthoredName(subject) => subject.0.clone(),
            Self::Predicate(terms) => terms
                .iter()
                .map(|(dimension, value)| format!("{dimension}={value}"))
                .collect::<Vec<_>>()
                .join(", "),
            Self::PredicateTerm { dimension, value } => format!("{dimension}={value}"),
        }
    }
}

/// One authored diagnostic subject that cannot be empty.
#[derive(Clone, Debug)]
struct NonemptyContextDiagnosticSubject(String);

/// One source-located, conjunctive state-dimension predicate.
#[derive(Clone, Debug)]
pub(super) struct SourceLocatedStateDimensionPredicate {
    terms: BTreeMap<ContextDimensionName, SourceLocatedStateDimensionTerm>,
}

impl SourceLocatedStateDimensionPredicate {
    pub(super) fn terms(
        &self,
    ) -> impl Iterator<Item = (&ContextDimensionName, &ContextValueName, &ContextTermSource)> {
        self.terms
            .iter()
            .map(|(dimension, term)| (dimension, &term.value, &term.source))
    }

    pub(super) fn matches(&self, snapshot: &ContextSnapshot) -> bool {
        #[cfg(test)]
        record_predicate_match();
        self.terms.iter().all(|(dimension, term)| {
            snapshot
                .values()
                .find_map(|(snapshot_dimension, snapshot_value)| {
                    (snapshot_dimension == dimension).then_some(snapshot_value)
                })
                == Some(&term.value)
        })
    }
}

#[derive(Clone, Debug)]
struct SourceLocatedStateDimensionTerm {
    value:  ContextValueName,
    source: ContextTermSource,
}

/// Source locations for one state-dimension predicate member.
#[derive(Clone, Debug)]
pub(super) struct ContextTermSource {
    pub(super) name:  ContextSource,
    pub(super) value: ContextSource,
}

impl ContextTermSource {
    const fn new(indexed_term: IndexedContextTerm, block_index: usize) -> Self {
        Self {
            name:  ContextSource::new(indexed_term.name, block_index),
            value: ContextSource::new(indexed_term.value, block_index),
        }
    }
}

/// The retained source locations for one authored binding key and command value.
#[derive(Clone, Debug)]
pub(super) struct BindingSource {
    key:                           SourceLocation,
    value:                         SourceLocation,
    pub(super) block_index:        usize,
    context:                       ContextDiagnosticSubject,
    pub(super) original_keystroke: String,
}

impl BindingSource {
    const fn new(
        key: SourceLocation,
        value: SourceLocation,
        block_index: usize,
        context: ContextDiagnosticSubject,
        original_keystroke: String,
    ) -> Self {
        Self {
            key,
            value,
            block_index,
            context,
            original_keystroke,
        }
    }

    pub(super) fn diagnostic(
        &self,
        diagnostic_origin: &DiagnosticOrigin,
        command_id: String,
        kind: DiagnosticKind,
        severity: DiagnosticSeverity,
        message: String,
    ) -> Diagnostic {
        self.diagnostic_at(
            &self.key,
            diagnostic_origin,
            command_id,
            kind,
            severity,
            message,
        )
    }

    pub(super) fn command_diagnostic(
        &self,
        diagnostic_origin: &DiagnosticOrigin,
        command_id: String,
        kind: DiagnosticKind,
        severity: DiagnosticSeverity,
        message: String,
    ) -> Diagnostic {
        self.diagnostic_at(
            &self.value,
            diagnostic_origin,
            command_id,
            kind,
            severity,
            message,
        )
    }

    fn diagnostic_at(
        &self,
        location: &SourceLocation,
        diagnostic_origin: &DiagnosticOrigin,
        command_id: String,
        kind: DiagnosticKind,
        severity: DiagnosticSeverity,
        message: String,
    ) -> Diagnostic {
        Diagnostic {
            origin: diagnostic_origin.clone(),
            byte_range: location.byte_range.clone(),
            line: location.line,
            column: location.column,
            block_index: self.block_index,
            context: self.context.text(),
            original_keystroke: self.original_keystroke.clone(),
            command_id,
            kind,
            severity,
            message,
            suggestions: Vec::new(),
        }
    }

    fn keystroke_diagnostic(
        &self,
        diagnostic_origin: &DiagnosticOrigin,
        source: &str,
        error: &KeystrokeSequenceParseError,
    ) -> Diagnostic {
        match error {
            KeystrokeSequenceParseError::Empty(_) => self.diagnostic(
                diagnostic_origin,
                String::new(),
                DiagnosticKind::Keystroke,
                DiagnosticSeverity::Failure,
                "A binding key must contain at least one keystroke.".to_owned(),
            ),
            KeystrokeSequenceParseError::Keystroke(error) => {
                let start =
                    (self.key.byte_range.start + error.offset()).min(self.key.byte_range.end);
                let byte_range = start..(start + error.token().len()).min(self.key.byte_range.end);
                let (line, column) = line_and_column(source, start);

                Diagnostic {
                    origin: diagnostic_origin.clone(),
                    byte_range,
                    line,
                    column,
                    block_index: self.block_index,
                    context: self.context.text(),
                    original_keystroke: self.original_keystroke.clone(),
                    command_id: String::new(),
                    kind: DiagnosticKind::Keystroke,
                    severity: DiagnosticSeverity::Failure,
                    message: format!("Unrecognized keystroke token `{}`.", error.token()),
                    suggestions: Vec::new(),
                }
            },
        }
    }
}

enum BindingEditResult {
    Edit(BindingEdit),
    Diagnostic(Diagnostic),
}

fn parse_binding_edit(
    diagnostic_origin: &DiagnosticOrigin,
    binding_source: &BindingSource,
    wire_value: Value,
) -> BindingEditResult {
    match wire_value {
        Value::String(command_id) => match CommandId::try_from(command_id.as_str()) {
            Ok(command_id) => BindingEditResult::Edit(BindingEdit::Bind(command_id)),
            Err(_) => BindingEditResult::Diagnostic(binding_source.command_diagnostic(
                diagnostic_origin,
                command_id.clone(),
                DiagnosticKind::Command,
                DiagnosticSeverity::Failure,
                format!(
                    "Command ID `{command_id}` requires one :: separator and snake-case segments."
                ),
            )),
        },
        Value::Null => BindingEditResult::Edit(BindingEdit::Unbind),
        Value::Array(_) => BindingEditResult::Diagnostic(binding_source.diagnostic(
            diagnostic_origin,
            String::new(),
            DiagnosticKind::Syntax,
            DiagnosticSeverity::Failure,
            "Binding arguments are not yet supported.".to_owned(),
        )),
        _ => BindingEditResult::Diagnostic(binding_source.diagnostic(
            diagnostic_origin,
            String::new(),
            DiagnosticKind::Syntax,
            DiagnosticSeverity::Failure,
            "Binding values must be command ID strings or null.".to_owned(),
        )),
    }
}

fn context_diagnostic(
    diagnostic_origin: &DiagnosticOrigin,
    context_source: &ContextSource,
    context: ContextDiagnosticSubject,
    kind: DiagnosticKind,
    severity: DiagnosticSeverity,
    message: String,
) -> Diagnostic {
    Diagnostic {
        origin: diagnostic_origin.clone(),
        byte_range: context_source.byte_range.clone(),
        line: context_source.line,
        column: context_source.column,
        block_index: context_source.block_index,
        context: context.text(),
        original_keystroke: String::new(),
        command_id: String::new(),
        kind,
        severity,
        message,
        suggestions: Vec::new(),
    }
}

fn serde_diagnostic(
    diagnostic_origin: &DiagnosticOrigin,
    source: &str,
    error: Error,
) -> Diagnostic {
    let byte_offset = byte_offset(source, error.line(), error.column());

    document_diagnostic(
        diagnostic_origin,
        source,
        byte_offset,
        DiagnosticKind::Syntax,
        error.to_string(),
    )
}

fn document_diagnostic(
    diagnostic_origin: &DiagnosticOrigin,
    source: &str,
    byte_offset: usize,
    kind: DiagnosticKind,
    message: String,
) -> Diagnostic {
    let (line, column) = line_and_column(source, byte_offset);

    Diagnostic {
        origin: diagnostic_origin.clone(),
        byte_range: byte_offset..byte_offset,
        line,
        column,
        block_index: 0,
        context: ContextDiagnosticSubject::Document.text(),
        original_keystroke: String::new(),
        command_id: String::new(),
        kind,
        severity: DiagnosticSeverity::Failure,
        message,
        suggestions: Vec::new(),
    }
}

fn root_value_starts_with_array(source: &str) -> bool {
    source
        .get(root_value_offset(source)..)
        .is_some_and(|remaining| remaining.starts_with('['))
}

fn root_value_offset(source: &str) -> usize {
    let mut scanner = JsoncScanner::new(source);
    let _ = scanner.skip_trivia();
    scanner.offset
}

fn byte_offset(source: &str, line: usize, column: usize) -> usize {
    if line == 0 || column == 0 {
        return source.len();
    }

    let mut current_line = 1;
    let mut line_start = 0;

    for (offset, byte) in source.bytes().enumerate() {
        if current_line == line {
            break;
        }
        if byte == b'\n' {
            current_line += 1;
            line_start = offset + 1;
        }
    }

    if current_line != line {
        return source.len();
    }

    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |offset| line_start + offset);
    let mut byte_offset = line_start
        .saturating_add(column - 1)
        .min(line_end)
        .min(source.len());

    while byte_offset > line_start && !source.is_char_boundary(byte_offset) {
        byte_offset -= 1;
    }

    byte_offset
}

fn line_and_column(source: &str, byte_offset: usize) -> (usize, usize) {
    let prefix = &source[..byte_offset.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |offset| offset + 1);
    let column = source[line_start..byte_offset.min(source.len())]
        .chars()
        .count()
        + 1;

    (line, column)
}

#[derive(Deserialize)]
struct WireDocument {
    #[serde(rename = "$schema")]
    schema:   Option<String>,
    bindings: Vec<WireBlock>,
}

#[derive(Deserialize)]
struct WireBlock {
    #[serde(default)]
    context:  WireContext,
    bindings: WireBindings,
}

#[derive(Default)]
enum WireContext {
    #[default]
    Absent,
    Null,
    Object(Vec<(String, Value)>),
    Other,
}

impl<'de> Deserialize<'de> for WireContext {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct WireContextVisitor;

        impl<'de> Visitor<'de> for WireContextVisitor {
            type Value = WireContext;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str("a keymap context object")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(WireContext::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserializer.deserialize_any(self)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(WireContext::Null)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let _ = value;
                Ok(WireContext::Other)
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let _ = value;
                Ok(WireContext::Other)
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(WireContext::Object(entries))
            }

            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(WireContext::Other)
            }

            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(WireContext::Other)
            }

            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(WireContext::Other)
            }

            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(WireContext::Other)
            }

            fn visit_seq<A>(self, _: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                Ok(WireContext::Other)
            }
        }

        deserializer.deserialize_option(WireContextVisitor)
    }
}

struct WireBindings(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for WireBindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct WireBindingsVisitor;

        impl<'de> Visitor<'de> for WireBindingsVisitor {
            type Value = WireBindings;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str("a keymap bindings object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut bindings = Vec::new();

                while let Some(binding) = map.next_entry()? {
                    bindings.push(binding);
                }

                Ok(WireBindings(bindings))
            }
        }

        deserializer.deserialize_map(WireBindingsVisitor)
    }
}

struct SourceIndex {
    blocks:                    Vec<IndexedBlock>,
    unrecognized_root_members: Vec<IndexedRootMember>,
}

impl SourceIndex {
    fn parse(source: &str) -> Result<Self, SourceIndexError> {
        let mut scanner = JsoncScanner::new(source);
        scanner.skip_trivia()?;
        scanner.expect_byte(b'{', "Keymap documents must start with an object.")?;

        let mut blocks = None;
        scanner.skip_trivia()?;

        if scanner.consume_byte(b'}') {
            return Ok(Self {
                blocks:                    Vec::new(),
                unrecognized_root_members: Vec::new(),
            });
        }

        let mut unrecognized_root_members = Vec::new();
        loop {
            let key = scanner.parse_string()?;
            scanner.skip_trivia()?;
            scanner.expect_byte(b':', "Expected `:` after an object key.")?;
            scanner.skip_trivia()?;

            let member_name = scanner.string_value(&key)?;
            if member_name == "bindings" && scanner.peek_byte() == Some(b'[') {
                blocks = Some(scanner.parse_blocks()?);
            } else if RECOGNIZED_ROOT_MEMBERS.contains(&member_name.as_str()) {
                scanner.skip_value()?;
            } else {
                unrecognized_root_members.push(IndexedRootMember {
                    name:     member_name,
                    location: SourceLocation::from_token(scanner.source, key),
                });
                scanner.skip_value()?;
            }

            scanner.skip_trivia()?;
            if scanner.consume_byte(b'}') {
                break;
            }
            scanner.expect_byte(b',', "Expected `,` between object members.")?;
            scanner.skip_trivia()?;
            if scanner.consume_byte(b'}') {
                break;
            }
        }

        scanner.skip_trivia()?;
        if scanner.peek_byte().is_some() {
            return Err(scanner.error("Expected the end of the keymap document."));
        }

        Ok(Self {
            blocks: blocks.unwrap_or_default(),
            unrecognized_root_members,
        })
    }
}

struct IndexedRootMember {
    name:     String,
    location: SourceLocation,
}

struct IndexedBlock {
    context:              IndexedContext,
    bindings:             Vec<IndexedBinding>,
    unrecognized_members: Vec<IndexedBlockMember>,
}

enum IndexedContext {
    Absent,
    Value(SourceLocation),
    Object {
        location: SourceLocation,
        terms:    Vec<IndexedContextTerm>,
    },
}

struct IndexedContextTerm {
    name:  SourceLocation,
    value: SourceLocation,
}

struct IndexedBinding {
    key:   SourceLocation,
    value: SourceLocation,
}

struct IndexedBlockMember {
    name:     String,
    location: SourceLocation,
}

#[derive(Clone, Debug)]
struct SourceLocation {
    byte_range: Range<usize>,
    line:       usize,
    column:     usize,
}

impl SourceLocation {
    fn from_token(source: &str, token: StringToken) -> Self {
        Self::from_range(source, token.content_range)
    }

    fn from_range(source: &str, byte_range: Range<usize>) -> Self {
        let (line, column) = line_and_column(source, byte_range.start);

        Self {
            byte_range,
            line,
            column,
        }
    }
}

struct JsoncScanner<'source> {
    source: &'source str,
    offset: usize,
}

impl<'source> JsoncScanner<'source> {
    const fn new(source: &'source str) -> Self { Self { source, offset: 0 } }

    fn parse_blocks(&mut self) -> Result<Vec<IndexedBlock>, SourceIndexError> {
        self.expect_byte(b'[', "Expected the `bindings` member to be an array.")?;
        self.skip_trivia()?;
        let mut blocks = Vec::new();

        if self.consume_byte(b']') {
            return Ok(blocks);
        }

        loop {
            blocks.push(self.parse_block()?);
            self.skip_trivia()?;
            if self.consume_byte(b']') {
                break;
            }
            self.expect_byte(b',', "Expected `,` between keymap blocks.")?;
            self.skip_trivia()?;
            if self.consume_byte(b']') {
                break;
            }
        }

        Ok(blocks)
    }

    fn parse_block(&mut self) -> Result<IndexedBlock, SourceIndexError> {
        self.expect_byte(b'{', "Expected a keymap block object.")?;
        self.skip_trivia()?;
        let mut context = IndexedContext::Absent;
        let mut bindings = None;
        let mut unrecognized_members = Vec::new();

        if self.consume_byte(b'}') {
            return Ok(IndexedBlock {
                context,
                bindings: Vec::new(),
                unrecognized_members,
            });
        }

        loop {
            let key = self.parse_string()?;
            self.skip_trivia()?;
            self.expect_byte(b':', "Expected `:` after a block member name.")?;
            self.skip_trivia()?;

            let member_name = self.string_value(&key)?;
            match member_name.as_str() {
                "context" => context = self.parse_context_locations()?,
                "bindings" if self.peek_byte() == Some(b'{') => {
                    bindings = Some(self.parse_binding_locations()?);
                },
                _ => {
                    unrecognized_members.push(IndexedBlockMember {
                        name:     member_name,
                        location: SourceLocation::from_token(self.source, key),
                    });
                    self.skip_value()?;
                },
            }

            self.skip_trivia()?;
            if self.consume_byte(b'}') {
                break;
            }
            self.expect_byte(b',', "Expected `,` between block members.")?;
            self.skip_trivia()?;
            if self.consume_byte(b'}') {
                break;
            }
        }

        Ok(IndexedBlock {
            context,
            bindings: bindings.unwrap_or_default(),
            unrecognized_members,
        })
    }

    fn parse_binding_locations(&mut self) -> Result<Vec<IndexedBinding>, SourceIndexError> {
        self.expect_byte(b'{', "Expected a bindings object.")?;
        self.skip_trivia()?;
        let mut bindings = Vec::<IndexedBinding>::new();

        if self.consume_byte(b'}') {
            return Ok(bindings);
        }

        loop {
            let key = self.parse_string()?;
            self.skip_trivia()?;
            self.expect_byte(b':', "Expected `:` after a binding key.")?;
            self.skip_trivia()?;
            let value = if self.peek_byte() == Some(b'"') {
                SourceLocation::from_token(self.source, self.parse_string()?)
            } else {
                let start = self.offset;
                self.skip_value()?;
                SourceLocation::from_range(self.source, start..self.offset)
            };
            bindings.push(IndexedBinding {
                key: SourceLocation::from_token(self.source, key),
                value,
            });
            self.skip_trivia()?;
            if self.consume_byte(b'}') {
                break;
            }
            self.expect_byte(b',', "Expected `,` between bindings.")?;
            self.skip_trivia()?;
            if self.consume_byte(b'}') {
                break;
            }
        }

        Ok(bindings)
    }

    fn parse_context_locations(&mut self) -> Result<IndexedContext, SourceIndexError> {
        if self.peek_byte() == Some(b'\"') {
            return Ok(IndexedContext::Value(SourceLocation::from_token(
                self.source,
                self.parse_string()?,
            )));
        }

        let start = self.offset;
        if self.peek_byte() == Some(b'{') {
            return self.parse_context_object(start);
        }
        self.skip_value()?;
        Ok(IndexedContext::Value(SourceLocation::from_range(
            self.source,
            start..self.offset,
        )))
    }

    fn parse_context_object(&mut self, start: usize) -> Result<IndexedContext, SourceIndexError> {
        self.expect_byte(b'{', "Expected a context object.")?;
        self.skip_trivia()?;
        let mut terms = Vec::new();

        if self.consume_byte(b'}') {
            return Ok(IndexedContext::Object {
                location: SourceLocation::from_range(self.source, start..self.offset),
                terms,
            });
        }

        loop {
            let key = self.parse_string()?;
            self.skip_trivia()?;
            self.expect_byte(b':', "Expected `:` after a context dimension name.")?;
            self.skip_trivia()?;
            let value = if self.peek_byte() == Some(b'\"') {
                SourceLocation::from_token(self.source, self.parse_string()?)
            } else {
                let value_start = self.offset;
                self.skip_value()?;
                SourceLocation::from_range(self.source, value_start..self.offset)
            };
            terms.push(IndexedContextTerm {
                name: SourceLocation::from_token(self.source, key),
                value,
            });
            self.skip_trivia()?;
            if self.consume_byte(b'}') {
                break;
            }
            self.expect_byte(b',', "Expected `,` between context members.")?;
            self.skip_trivia()?;
            if self.consume_byte(b'}') {
                break;
            }
        }

        Ok(IndexedContext::Object {
            location: SourceLocation::from_range(self.source, start..self.offset),
            terms,
        })
    }

    fn skip_value(&mut self) -> Result<(), SourceIndexError> {
        self.skip_trivia()?;

        match self.peek_byte() {
            Some(b'{') => self.skip_object(),
            Some(b'[') => self.skip_array(),
            Some(b'\"') => self.parse_string().map(|_| ()),
            Some(b't') => self.consume_keyword(b"true"),
            Some(b'f') => self.consume_keyword(b"false"),
            Some(b'n') => self.consume_keyword(b"null"),
            Some(b'-' | b'0'..=b'9') => {
                self.skip_number();
                Ok(())
            },
            _ => Err(self.error("Expected a JSON value.")),
        }
    }

    fn skip_object(&mut self) -> Result<(), SourceIndexError> {
        self.expect_byte(b'{', "Expected an object.")?;
        self.skip_trivia()?;

        if self.consume_byte(b'}') {
            return Ok(());
        }

        loop {
            self.parse_string()?;
            self.skip_trivia()?;
            self.expect_byte(b':', "Expected `:` after an object key.")?;
            self.skip_value()?;
            self.skip_trivia()?;
            if self.consume_byte(b'}') {
                return Ok(());
            }
            self.expect_byte(b',', "Expected `,` between object members.")?;
            self.skip_trivia()?;
            if self.consume_byte(b'}') {
                return Ok(());
            }
        }
    }

    fn skip_array(&mut self) -> Result<(), SourceIndexError> {
        self.expect_byte(b'[', "Expected an array.")?;
        self.skip_trivia()?;

        if self.consume_byte(b']') {
            return Ok(());
        }

        loop {
            self.skip_value()?;
            self.skip_trivia()?;
            if self.consume_byte(b']') {
                return Ok(());
            }
            self.expect_byte(b',', "Expected `,` between array values.")?;
            self.skip_trivia()?;
            if self.consume_byte(b']') {
                return Ok(());
            }
        }
    }

    fn skip_number(&mut self) {
        while self.peek_byte().is_some_and(|byte| {
            !byte.is_ascii_whitespace() && !matches!(byte, b',' | b']' | b'}' | b'/')
        }) {
            self.offset += 1;
        }
    }

    fn consume_keyword(&mut self, keyword: &[u8]) -> Result<(), SourceIndexError> {
        if self.source.as_bytes()[self.offset..].starts_with(keyword) {
            self.offset += keyword.len();
            Ok(())
        } else {
            Err(self.error("Expected a JSON keyword."))
        }
    }

    fn parse_string(&mut self) -> Result<StringToken, SourceIndexError> {
        self.expect_byte(b'\"', "Expected a JSON string.")?;
        let start = self.offset - 1;
        let content_start = self.offset;

        while let Some(byte) = self.peek_byte() {
            match byte {
                b'\"' => {
                    let content_end = self.offset;
                    self.offset += 1;
                    return Ok(StringToken {
                        token_range:   start..self.offset,
                        content_range: content_start..content_end,
                    });
                },
                b'\\' => {
                    self.offset += 1;
                    if self.peek_byte().is_none() {
                        return Err(
                            self.error("A JSON string escape must have a following character.")
                        );
                    }
                    self.offset += 1;
                },
                byte if byte.is_ascii_control() => {
                    return Err(self.error("JSON strings cannot contain control characters."));
                },
                _ => self.offset += 1,
            }
        }

        Err(self.error("JSON strings must end with a quote."))
    }

    fn string_value(&self, token: &StringToken) -> Result<String, SourceIndexError> {
        serde_json_lenient::from_str(&self.source[token.token_range.clone()]).map_err(|error| {
            SourceIndexError {
                offset:  token.token_range.start,
                message: error.to_string(),
            }
        })
    }

    fn expect_byte(&mut self, expected: u8, message: &str) -> Result<(), SourceIndexError> {
        self.skip_trivia()?;

        if self.consume_byte(expected) {
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.peek_byte() == Some(expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn skip_trivia(&mut self) -> Result<(), SourceIndexError> {
        loop {
            while self
                .peek_byte()
                .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                self.offset += 1;
            }

            let remaining = &self.source.as_bytes()[self.offset..];
            if remaining.starts_with(b"//") {
                self.offset += 2;
                while self.peek_byte().is_some_and(|byte| byte != b'\n') {
                    self.offset += 1;
                }
                continue;
            }
            if remaining.starts_with(b"/*") {
                self.offset += 2;
                while self.source.as_bytes().get(self.offset..self.offset + 2) != Some(b"*/") {
                    if self.peek_byte().is_none() {
                        return Err(self.error("Block comments must end with `*/`."));
                    }
                    self.offset += 1;
                }
                self.offset += 2;
                continue;
            }

            return Ok(());
        }
    }

    fn peek_byte(&self) -> Option<u8> { self.source.as_bytes().get(self.offset).copied() }

    fn error(&self, message: &str) -> SourceIndexError {
        SourceIndexError {
            offset:  self.offset,
            message: message.to_owned(),
        }
    }
}

struct StringToken {
    token_range:   Range<usize>,
    content_range: Range<usize>,
}

#[derive(Debug)]
struct SourceIndexError {
    offset:  usize,
    message: String,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected keymap parse failures"
)]
mod tests {
    use std::path::PathBuf;

    use super::BindingEdit;
    use super::DiagnosticKind;
    use super::DiagnosticOrigin;
    use super::KeymapDocument;
    use super::SourceIndex;
    use crate::DiagnosticSeverity;

    const SOURCE_PATH: &str = "keymap.jsonc";

    fn keymap_file() -> DiagnosticOrigin {
        DiagnosticOrigin::KeymapFile(PathBuf::from(SOURCE_PATH))
    }

    fn assert_binding_key_slice(source: &str, binding_index: usize, expected_key: &str) {
        let source_index = SourceIndex::parse(source).expect("source index parses binding source");
        let binding_location = &source_index.blocks[0].bindings[binding_index];

        assert_eq!(
            &source[binding_location.key.byte_range.clone()],
            expected_key,
        );
    }

    #[test]
    fn envelope_rejects_string_context_without_losing_block_locations() {
        let source = r#"
            {
                "$schema": "./keymap.schema.json",
                "bindings": [
                    { "bindings": { "space": "transport::toggle_playback" } },
                    {
                        "context": "dimension_lock",
                        "bindings": { "b": "dimension_lock::toggle_coupling" }
                    }
                ]
            }
        "#;

        let (document, diagnostics) =
            KeymapDocument::parse(&keymap_file(), source).expect("valid keymap envelope");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::Syntax
                && diagnostic.severity == DiagnosticSeverity::Failure
                && diagnostic.context == "context"
        }));
        assert_eq!(document.blocks.len(), 2);
        assert!(matches!(
            document.blocks[0].bindings[0].edit,
            BindingEdit::Bind(ref command_id) if command_id.as_str() == "transport::toggle_playback"
        ));
        assert!(matches!(
            &document.blocks[1].scope,
            super::KeymapBlockScope::Invalid
        ));
        assert!(matches!(
            document.blocks[1].bindings[0].edit,
            BindingEdit::Bind(ref command_id) if command_id.as_str() == "dimension_lock::toggle_coupling"
        ));
        assert_eq!(document.blocks[0].bindings[0].source.block_index, 0);
        assert_eq!(document.blocks[1].bindings[0].source.block_index, 1);
    }

    #[test]
    fn unrecognized_root_members_produce_load_time_advisories() {
        let source = r#"{
            "bindingz": [],
            "bindings": []
        }"#;

        let (_, diagnostics) =
            KeymapDocument::parse(&keymap_file(), source).expect("valid JSONC envelope");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("bindingz"))
            .expect("unknown root member must be diagnosed");

        assert_eq!(diagnostic.kind, DiagnosticKind::Syntax);
        assert_eq!(diagnostic.severity, crate::DiagnosticSeverity::Advisory);
        assert_eq!(&source[diagnostic.byte_range.clone()], "bindingz");
    }

    #[test]
    fn bare_root_array_is_rejected_with_envelope_message() {
        let source = r#"[{ "bindings": { "space": "transport::toggle_playback" } }]"#;

        let diagnostics = KeymapDocument::parse(&keymap_file(), source)
            .expect_err("a root array must be rejected");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Syntax);
        assert_eq!(
            diagnostics[0].message,
            "Keymap documents require an object envelope with a `bindings` member."
        );
    }

    #[test]
    fn comments_and_trailing_commas_parse() {
        let source = r#"
            {
                // The schema keeps editor completions available.
                "$schema": "./keymap.schema.json",
                "bindings": [
                    {
                        "bindings": {
                            "space": "transport::toggle_playback",
                        },
                    },
                ],
            }
        "#;

        let (document, diagnostics) =
            KeymapDocument::parse(&keymap_file(), source).expect("JSONC keymap document");

        assert!(diagnostics.is_empty());
        assert_eq!(document.blocks.len(), 1);
        assert_eq!(document.blocks[0].bindings.len(), 1);
    }

    #[test]
    fn null_binding_creates_an_unbind_edit() {
        let source = r#"{ "bindings": [{ "bindings": { "space": null } }] }"#;

        let (document, diagnostics) =
            KeymapDocument::parse(&keymap_file(), source).expect("null binding tombstone");

        assert!(diagnostics.is_empty());
        assert_eq!(document.blocks.len(), 1);
        assert_eq!(document.blocks[0].bindings.len(), 1);
        assert_eq!(document.blocks[0].bindings[0].edit, BindingEdit::Unbind);
    }

    #[test]
    fn binding_arguments_are_rejected_until_they_are_supported() {
        let source = r#"{
            "bindings": [{ "bindings": {
                "space": ["transport::toggle_playback", {}],
                "enter": "editor::copy"
            } }]
        }"#;

        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("binding argument diagnostics retain sibling bindings");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Syntax);
        assert_eq!(
            diagnostics[0].message,
            "Binding arguments are not yet supported."
        );
        assert_eq!(document.blocks[0].bindings.len(), 1);
    }

    #[test]
    fn numeric_binding_arguments_followed_by_comments_are_rejected() {
        let source = r#"{
            "bindings": [{ "bindings": {
                "space": ["transport::toggle_playback", 1/* explanation */],
                "enter": "editor::copy"
            } }]
        }"#;

        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("binding argument diagnostics retain sibling bindings");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Syntax);
        assert_eq!(
            diagnostics[0].message,
            "Binding arguments are not yet supported."
        );
        assert_eq!(document.blocks[0].bindings.len(), 1);
    }

    #[test]
    fn null_context_is_rejected_without_discarding_the_block_bindings() {
        let source = r#"{
            "bindings": [{
                "context": null,
                "bindings": { "space": "transport::toggle_playback" }
            }]
        }"#;

        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("null context diagnostics retain block bindings");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Syntax);
        assert!(matches!(
            &document.blocks[0].scope,
            super::KeymapBlockScope::Invalid
        ));
        assert_eq!(document.blocks[0].bindings.len(), 1);
    }

    #[test]
    fn bad_keystrokes_leave_sibling_bindings_available() {
        let source = r#"{
            "bindings": [{ "bindings": {
                "a": "editor::select_all",
                "unknown": "editor::copy",
                "b": "editor::bold"
            } }]
        }"#;

        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("keystroke diagnostics retain sibling bindings");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Keystroke);
        assert_eq!(diagnostics[0].original_keystroke, "unknown");
        assert_eq!(document.blocks[0].bindings.len(), 2);
    }

    #[test]
    fn every_string_context_is_invalid_authoring_syntax() {
        let source = r#"
            {
                "bindings": [
                    { "context": "editing", "bindings": { "a": "editor::select_all" } },
                    { "context": "editing", "bindings": { "b": "editor::bold" } }
                ]
            }
        "#;

        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("duplicate context diagnostics retain block bindings");
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Failure)
                .count(),
            2
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.context.is_empty())
        );
        assert!(matches!(
            &document.blocks[0].scope,
            super::KeymapBlockScope::Invalid
        ));
        assert!(matches!(
            &document.blocks[1].scope,
            super::KeymapBlockScope::Invalid
        ));
        assert_eq!(document.blocks[0].bindings.len(), 1);
        assert_eq!(document.blocks[1].bindings.len(), 1);
    }

    #[test]
    fn duplicate_binding_keys_within_one_block_are_diagnosed() {
        let source = r#"
            { "bindings": [{ "bindings": {
                "space": "transport::toggle_playback",
                "space": "transport::stop"
            } }] }
        "#;

        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("duplicate binding diagnostics retain both bindings");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.kind == DiagnosticKind::Syntax
                    && diagnostic
                        .message
                        .contains("declares `space` more than once")
            })
            .expect("duplicate binding diagnostic");

        assert_eq!(diagnostic.block_index, 0);
        assert_eq!(diagnostic.original_keystroke, "space");
        let bindings = &document.blocks[0].bindings;
        assert_eq!(bindings.len(), 2);
        assert!(matches!(
            bindings[0].edit,
            BindingEdit::Bind(ref command_id) if command_id.as_str() == "transport::toggle_playback"
        ));
        assert!(matches!(
            bindings[1].edit,
            BindingEdit::Bind(ref command_id) if command_id.as_str() == "transport::stop"
        ));
    }

    #[test]
    fn invalid_bindings_in_repeated_context_blocks_keep_individual_locations() {
        let source = r#"{"bindings":[{"context":"editing","bindings":{"unknown":"editor::select_all","invalid":"editor::bold","a":"editor::copy"}},{"context":"editing","bindings":{"broken":"editor::copy","unmapped":"editor::paste","b":"editor::cut"}}]}"#;
        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("keystroke diagnostics retain valid bindings");

        for (original_keystroke, block_index) in [
            ("unknown", 0),
            ("invalid", 0),
            ("broken", 1),
            ("unmapped", 1),
        ] {
            let diagnostic = diagnostics
                .iter()
                .find(|diagnostic| {
                    diagnostic.kind == DiagnosticKind::Keystroke
                        && diagnostic.original_keystroke == original_keystroke
                })
                .expect("keystroke diagnostic for every invalid binding");
            let byte_start = source
                .find(&format!("\"{original_keystroke}\""))
                .expect("binding key in test source")
                + 1;
            let column = source[..byte_start].chars().count() + 1;

            assert_eq!(diagnostic.byte_range.start, byte_start);
            assert_eq!(diagnostic.line, 1);
            assert_eq!(diagnostic.column, column);
            assert_eq!(diagnostic.block_index, block_index);
        }

        assert_eq!(document.blocks[0].bindings.len(), 1);
        assert_eq!(document.blocks[1].bindings.len(), 1);
    }

    #[test]
    fn escaped_binding_keys_keep_keystroke_diagnostics_inside_their_source_span() {
        let source =
            r#"{ "bindings": [{ "bindings": { "ctrl-\u0078 unknown": "editor::copy" } }] }"#;

        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("keystroke diagnostics retain document data");
        let binding_start = source
            .find(r"ctrl-\u0078 unknown")
            .expect("escaped binding key in source");
        let binding_range = binding_start..binding_start + r"ctrl-\u0078 unknown".len();

        assert!(document.blocks[0].bindings.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].byte_range.start >= binding_range.start);
        assert!(diagnostics[0].byte_range.end <= binding_range.end);
    }

    #[test]
    fn syntax_diagnostics_use_byte_offsets_after_multibyte_text() {
        let source = r#"{ "bindings": [], /* café */ ! }"#;
        let diagnostics = KeymapDocument::parse(&keymap_file(), source)
            .expect_err("invalid JSONC must reject the whole document");
        let error_offset = source.find('!').expect("syntax error marker in source");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].byte_range, error_offset..error_offset);
    }

    #[test]
    fn source_index_skips_nested_binding_object_and_array_values() {
        let source = r#"{
            "bindings": [{ "bindings": {
                "object": { "nested": [true, { "number": 1 }] },
                "array": [{ "item": null }, [1, 2, 3]]
            } }]
        }"#;

        assert_binding_key_slice(source, 0, "object");
        assert_binding_key_slice(source, 1, "array");
    }

    #[test]
    fn source_index_skips_comments_around_binding_punctuation() {
        let source = r#"{
            "bindings": [{ "bindings": {
                "line" // between the key and colon
                : "editor::copy" // between the value and comma
                ,
                "block" /* between the key and colon */ : "editor::paste" /* between the value and comma */,
            } }]
        }"#;

        assert_binding_key_slice(source, 0, "line");
        assert_binding_key_slice(source, 1, "block");
    }

    #[test]
    fn source_index_preserves_escaped_quotes_in_binding_key_ranges() {
        let source = r#"{ "bindings": [{ "bindings": { "button\"quote": "editor::copy" } }] }"#;

        assert_binding_key_slice(source, 0, r#"button\"quote"#);
    }

    #[test]
    fn source_index_finds_bindings_after_unknown_block_members() {
        let source = r#"{
            "bindings": [{
                "before_context": { "nested": [1] },
                "context": "editing",
                "before_bindings": [true, false],
                "bindings": { "named": "editor::copy" }
            }]
        }"#;

        assert_binding_key_slice(source, 0, "named");
    }

    #[test]
    fn object_predicates_preserve_global_and_canonical_dimension_scopes() -> Result<(), String> {
        let source = r#"{
            "bindings": [
                { "bindings": { "a": "editor::copy" } },
                { "context": { "application": "ready" }, "bindings": {} },
                { "context": { "interaction": "editing" }, "bindings": {} },
                { "context": { "interaction": "editing", "application": "ready" }, "bindings": {} }
            ]
        }"#;

        let (document, diagnostics) =
            KeymapDocument::parse(&keymap_file(), source).expect("object predicates parse");

        assert!(diagnostics.is_empty());
        assert!(matches!(
            document.blocks[0].scope,
            super::KeymapBlockScope::Global
        ));
        for (block_index, expected_terms) in [
            (1, vec![("application", "ready")]),
            (2, vec![("interaction", "editing")]),
            (
                3,
                vec![("application", "ready"), ("interaction", "editing")],
            ),
        ] {
            let super::KeymapBlockScope::Conditional(predicate) =
                &document.blocks[block_index].scope
            else {
                return Err(format!(
                    "block {block_index} must be a state-dimension predicate"
                ));
            };
            let terms = predicate
                .terms()
                .map(|(dimension, value, _)| (dimension.as_str(), value.as_str()))
                .collect::<Vec<_>>();
            assert_eq!(terms, expected_terms);
        }
        Ok(())
    }

    #[test]
    fn non_object_context_values_and_empty_objects_become_invalid_semantic_scopes() {
        for context in ["null", "[]", "true", "1", "{}"] {
            let source = format!(
                r#"{{ "bindings": [{{ "context": {context}, "bindings": {{ "a": "editor::copy" }} }}] }}"#
            );
            let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), &source)
                .expect("invalid context retains the parsed document for diagnostics");

            assert!(matches!(
                document.blocks[0].scope,
                super::KeymapBlockScope::Invalid
            ));
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].context, "context");
            assert_eq!(diagnostics[0].severity, crate::DiagnosticSeverity::Failure);
        }
    }

    #[test]
    fn predicate_member_diagnostics_name_and_locate_the_offending_term() {
        let source = r#"{
            "bindings": [{
                "context": {
                    "application": 7,
                    "interaction": "editing",
                    "interaction": "resting"
                },
                "bindings": {}
            }]
        }"#;
        let (document, diagnostics) = KeymapDocument::parse(&keymap_file(), source)
            .expect("invalid predicate members retain document diagnostics");

        assert!(matches!(
            document.blocks[0].scope,
            super::KeymapBlockScope::Invalid
        ));
        let wrong_value = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("state-value string"))
            .expect("wrong value type diagnostic");
        let duplicate = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("appears more than once"))
            .expect("duplicate member diagnostic");
        let wrong_value_start = source.find('7').expect("wrong value in source");
        let duplicate_start = source
            .rfind("interaction")
            .expect("duplicate dimension in source");

        assert_eq!(
            wrong_value.byte_range,
            wrong_value_start..wrong_value_start + 1
        );
        assert_eq!(wrong_value.line, 4);
        assert_eq!(wrong_value.column, 36);
        assert_eq!(wrong_value.context, "application");
        assert_eq!(
            duplicate.byte_range,
            duplicate_start..duplicate_start + "interaction".len()
        );
        assert_eq!(duplicate.line, 6);
        assert_eq!(duplicate.column, 22);
        assert_eq!(duplicate.context, "interaction=resting");
        assert!(duplicate.message.contains("interaction"));
        assert!(duplicate.message.contains("resting"));
    }
}
