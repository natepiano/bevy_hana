#[cfg(test)]
use std::cell::Cell;
use std::collections::HashMap;
use std::collections::HashSet;

use bevy::prelude::AppTypeRegistry;
use bevy::prelude::On;
use bevy::prelude::ReflectEvent;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::reflect::TypeRegistry;
use bevy_enhanced_input::prelude::ActionValue;
use bevy_enhanced_input::prelude::CustomInput;
use bevy_enhanced_input::prelude::CustomInputs;

use super::Capability;
use super::CommandId;
use super::HoldPhase;
use super::KeymapCommand;
use super::ReflectKeymapCommand;
use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;
use crate::keymap;
use crate::keymap::KeymapRuntime;

/// A read-only table that resolves declared command IDs to their runtime registrations.
#[derive(Resource)]
pub struct CommandRegistry {
    entries:              HashMap<CommandId, CommandEntry>,
    palette_search_index: Vec<CommandPaletteSearchEntry>,
    held_registrations:   Vec<(CustomInput, fn(&mut World, CustomInput))>,
}

/// One command's declared metadata, as read from the registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandInfo<'registry> {
    /// The command's validated identifier.
    pub id:          &'registry CommandId,
    /// Palette title text.
    pub title:       &'registry str,
    /// Palette description text.
    pub description: &'registry str,
    /// Declared keymap binding capability.
    pub capability:  Capability,
}

/// The result of resolving a command ID against the registry's declared metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandLookup<'registry> {
    /// The registry declares this command.
    Found(CommandInfo<'registry>),
    /// No command declaration matches the requested identifier.
    UnknownCommand,
}

/// What [`CommandRegistry::invoke`] did with a requested command ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandInvocationOutcome {
    /// The command's event was triggered.
    Invoked,
    /// No command declaration matches the requested identifier.
    UnknownCommand,
    /// The command is hold-to-act, so its caller must trigger the event with a [`HoldPhase`].
    HeldCommandRequiresPhase,
}

/// The result of looking up a command's held custom input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeldCommandLookupOutcome {
    /// The command is declared held and owns this preallocated custom input.
    RegisteredHeldInput(CustomInput),
    /// The command exists but is not a held command.
    KnownNonHeld,
    /// No command declaration matches the requested identifier.
    UnknownCommand,
}

impl CommandRegistry {
    /// Builds a command registry from the application's reflected type registrations.
    ///
    /// The returned registry is not inserted into the world — the caller owns that. A bare `App`
    /// is enough to call this, since its world already carries the `AppTypeRegistry` this reads.
    ///
    /// # Errors
    ///
    /// Returns registry-validation diagnostics when a command ID is malformed or duplicated, a
    /// command title or description is empty, or a command event lacks `ReflectEvent` type data.
    /// Reflection consumers such as BRP need that registration even though [`Self::invoke`] does
    /// not use reflection to dispatch an event.
    pub fn initialize(world: &mut World) -> Result<Self, Vec<Diagnostic>> {
        world.init_resource::<CustomInputs>();
        let app_type_registry = world.resource::<AppTypeRegistry>().clone();
        let command_registry = {
            let type_registry = app_type_registry.read();
            let mut custom_inputs = world.resource_mut::<CustomInputs>();
            Self::build(&type_registry, &mut custom_inputs)?
        };

        command_registry.register_held_observers(world);
        Ok(command_registry)
    }

    pub(crate) fn empty() -> Self {
        Self {
            entries:              HashMap::new(),
            palette_search_index: Vec::new(),
            held_registrations:   Vec::new(),
        }
    }

    /// Builds a command registry from one reflected type registry.
    pub(crate) fn build(
        type_registry: &TypeRegistry,
        custom_inputs: &mut CustomInputs,
    ) -> Result<Self, Vec<Diagnostic>> {
        let declarations = command_declarations(type_registry);
        let validated_declarations = validate_declarations(declarations)?;
        let mut held_registrations = Vec::new();
        let entries = validated_declarations
            .into_iter()
            .map(|declaration| {
                let capability = declaration.command.capability;
                let invocation = Invocation::from_capability(capability, custom_inputs);
                if let Some(custom_input) = invocation.custom_input() {
                    held_registrations
                        .push((custom_input, declaration.command.register_held_observer));
                }
                let command_entry = CommandEntry {
                    title: declaration.command.title,
                    description: declaration.command.description,
                    capability,
                    dispatch: declaration.command.dispatch,
                    invocation,
                };

                (declaration.command_id, command_entry)
            })
            .collect();
        let palette_search_index = CommandPaletteSearchEntry::from_entries(&entries);

        Ok(Self {
            entries,
            palette_search_index,
            held_registrations,
        })
    }

    /// Iterates over declared command metadata in ascending [`CommandId`] order.
    pub fn iter(&self) -> impl Iterator<Item = CommandInfo<'_>> {
        self.iter_entries().map(|(command_id, entry)| CommandInfo {
            id:          command_id,
            title:       entry.title,
            description: entry.description,
            capability:  entry.capability,
        })
    }

    pub(crate) fn iter_entries(&self) -> impl Iterator<Item = (&CommandId, &CommandEntry)> {
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|(left_id, _), (right_id, _)| left_id.as_str().cmp(right_id.as_str()));
        entries.into_iter()
    }

    /// Walks matching command declarations in immutable authored title/ID order.
    ///
    /// Palette callers normalize a query once before using this iterator. Each index entry retains
    /// normalized command text built with the registry, so the walk never needs to normalize
    /// metadata or reorder declarations.
    pub(crate) fn palette_search(
        &self,
        normalized_query: &str,
    ) -> impl Iterator<Item = PaletteSearchMatchInvocability<'_>> {
        #[cfg(test)]
        record_ordered_palette_search_pass();

        self.palette_search_index
            .iter()
            .filter(move |entry| entry.matches(normalized_query))
            .map(CommandPaletteSearchEntry::palette_invocability)
    }

    /// Resolves `command_id` to the metadata the registry declared for it.
    #[must_use]
    pub fn lookup(&self, command_id: &CommandId) -> CommandLookup<'_> {
        self.entries.get_key_value(command_id).map_or(
            CommandLookup::UnknownCommand,
            |(command_id, entry)| {
                CommandLookup::Found(CommandInfo {
                    id:          command_id,
                    title:       entry.title,
                    description: entry.description,
                    capability:  entry.capability,
                })
            },
        )
    }

    /// Resolves whether `command_id` has a preallocated held custom input.
    #[must_use]
    pub fn held_command_lookup(&self, command_id: &CommandId) -> HeldCommandLookupOutcome {
        self.entries
            .get(command_id)
            .map_or(HeldCommandLookupOutcome::UnknownCommand, |entry| {
                entry.invocation.held_command_lookup_outcome()
            })
    }

    /// Dispatches the event registered for `command_id`.
    ///
    /// This resolves the command ID and calls its monomorphized dispatch function without
    /// reflection type-data lookup or reflected event reconstruction. Reflection registration is
    /// still validated during initialization so BRP and other reflection consumers can find the
    /// command event.
    ///
    /// A hold-to-act command carries a [`HoldPhase`] this call supplies no value for, so it is
    /// rejected as [`CommandInvocationOutcome::HeldCommandRequiresPhase`] before its event is
    /// constructed.
    pub fn invoke(&self, command_id: &CommandId, world: &mut World) -> CommandInvocationOutcome {
        let Some(entry) = self.entries.get(command_id) else {
            return CommandInvocationOutcome::UnknownCommand;
        };
        if matches!(entry.invocation, Invocation::Held(_)) {
            return CommandInvocationOutcome::HeldCommandRequiresPhase;
        }

        (entry.dispatch)(world);
        CommandInvocationOutcome::Invoked
    }

    pub(crate) fn register_held_observers(&self, world: &mut World) {
        for (custom_input, register_held_observer) in &self.held_registrations {
            register_held_observer(world, *custom_input);
        }
    }
}

/// The opaque metadata view for one command the palette may invoke.
///
/// Construction stays inside the command registry so held commands can never become palette rows
/// or palette selections. The view borrows declaration metadata and the validated command ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaletteInvocableCommand<'registry> {
    command_id:  &'registry CommandId,
    title:       &'registry str,
    description: &'registry str,
}

impl<'registry> PaletteInvocableCommand<'registry> {
    /// Returns the validated command identifier for registry-wide invocation.
    #[must_use]
    pub const fn id(&self) -> &'registry CommandId { self.command_id }

    /// Returns the authored palette title.
    #[must_use]
    pub const fn title(&self) -> &'registry str { self.title }

    /// Returns the authored palette description.
    #[must_use]
    pub const fn description(&self) -> &'registry str { self.description }
}

/// One immutable palette-search entry built alongside the registry.
struct CommandPaletteSearchEntry {
    command_id:      CommandId,
    title:           &'static str,
    description:     &'static str,
    capability:      Capability,
    normalized_text: NormalizedCommandText,
}

/// Normalized command metadata retained for palette substring matching.
struct NormalizedCommandText {
    title: String,
    id:    String,
}

impl CommandPaletteSearchEntry {
    fn from_entries(entries: &HashMap<CommandId, CommandEntry>) -> Vec<Self> {
        let mut search_index = entries
            .iter()
            .map(|(command_id, command)| Self {
                command_id:      command_id.clone(),
                title:           command.title,
                description:     command.description,
                capability:      command.capability,
                normalized_text: NormalizedCommandText {
                    title: normalize_palette_search_text(command.title),
                    id:    normalize_palette_search_text(command_id.as_str()),
                },
            })
            .collect::<Vec<_>>();
        search_index.sort_by(|left, right| {
            left.title
                .cmp(right.title)
                .then_with(|| left.command_id.as_str().cmp(right.command_id.as_str()))
        });
        search_index
    }

    fn matches(&self, normalized_query: &str) -> bool {
        self.normalized_text.title.contains(normalized_query)
            || self.normalized_text.id.contains(normalized_query)
    }

    /// States whether this matching declaration is eligible for palette invocation.
    const fn palette_invocability(&self) -> PaletteSearchMatchInvocability<'_> {
        if self.capability.is_palette_invocable() {
            PaletteSearchMatchInvocability::Invocable(PaletteInvocableCommand {
                command_id:  &self.command_id,
                title:       self.title,
                description: self.description,
            })
        } else {
            PaletteSearchMatchInvocability::Held
        }
    }
}

/// Whether a declaration matching a palette query can be invoked from the palette.
pub(crate) enum PaletteSearchMatchInvocability<'registry> {
    /// The matching declaration can produce a palette row and selection.
    Invocable(PaletteInvocableCommand<'registry>),
    /// The matching declaration is held and therefore cannot be palette-invocable.
    Held,
}

/// Normalizes palette title, identifier, and user query text for substring matching.
///
/// Case folds and reduces `::`, `_`, `-`, and whitespace runs to a single space, so an authored
/// identifier such as `camera::reset_home` matches `camera reset home` and `Reset Home`.
pub(crate) fn normalize_palette_search_text(text: &str) -> String {
    #[cfg(test)]
    record_palette_normalization();

    let mut normalized = String::with_capacity(text.len());
    let mut separator = PaletteSearchSeparator::Absent;
    for character in text.chars() {
        if character == ':' || character == '_' || character == '-' || character.is_whitespace() {
            if !normalized.is_empty() {
                separator = PaletteSearchSeparator::Pending;
            }
            continue;
        }
        if matches!(separator, PaletteSearchSeparator::Pending) {
            normalized.push(' ');
            separator = PaletteSearchSeparator::Absent;
        }
        normalized.extend(character.to_lowercase());
    }
    normalized
}

/// Whether normalized palette text needs a separator before the next word.
enum PaletteSearchSeparator {
    Absent,
    Pending,
}

/// Test-only call counts for [`normalize_palette_search_text`] and
/// [`CommandRegistry::palette_search`] after [`CommandRegistry::build`].
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PaletteSearchObservations {
    normalizations:        usize,
    ordered_search_passes: usize,
}

#[cfg(test)]
thread_local! {
    static PALETTE_SEARCH_OBSERVATIONS: Cell<PaletteSearchObservations> = const {
        Cell::new(PaletteSearchObservations {
            normalizations: 0,
            ordered_search_passes: 0,
        })
    };
}

#[cfg(test)]
fn reset_palette_search_observations() {
    PALETTE_SEARCH_OBSERVATIONS.with(|palette_search_observations| {
        palette_search_observations.set(PaletteSearchObservations::default());
    });
}

#[cfg(test)]
fn palette_search_observations() -> PaletteSearchObservations {
    PALETTE_SEARCH_OBSERVATIONS.with(Cell::get)
}

#[cfg(test)]
fn record_palette_normalization() {
    PALETTE_SEARCH_OBSERVATIONS.with(|palette_search_observations| {
        let mut observations = palette_search_observations.get();
        observations.normalizations += 1;
        palette_search_observations.set(observations);
    });
}

#[cfg(test)]
fn record_ordered_palette_search_pass() {
    PALETTE_SEARCH_OBSERVATIONS.with(|palette_search_observations| {
        let mut observations = palette_search_observations.get();
        observations.ordered_search_passes += 1;
        palette_search_observations.set(observations);
    });
}

pub(super) fn register_held_observer<T: KeymapCommand>(
    world: &mut World,
    custom_input: CustomInput,
) {
    world.add_observer(
        move |event: On<T>,
              mut custom_inputs: ResMut<CustomInputs>,
              keymap_runtime: Option<ResMut<KeymapRuntime>>| {
            let Some(hold_phase) = event.event().hold_phase() else {
                return;
            };

            if let Some(mut keymap_runtime) = keymap_runtime {
                keymap::set_event_source(
                    &mut keymap_runtime,
                    custom_input,
                    matches!(hold_phase, HoldPhase::Begin),
                    &mut custom_inputs,
                );
                return;
            }

            let input_is_active = custom_inputs
                .get(&custom_input)
                .is_some_and(|action_value| action_value.as_bool());
            match (hold_phase, input_is_active) {
                (HoldPhase::Begin, false) => {
                    custom_inputs.insert(custom_input, ActionValue::Bool(true));
                },
                (HoldPhase::End, true) => {
                    custom_inputs.insert(custom_input, ActionValue::Bool(false));
                },
                (HoldPhase::Begin, true) | (HoldPhase::End, false) => {},
            }
        },
    );
}

#[derive(Clone)]
pub(crate) struct CommandEntry {
    title:       &'static str,
    description: &'static str,
    capability:  Capability,
    dispatch:    fn(&mut World),
    invocation:  Invocation,
}

impl CommandEntry {
    pub(crate) const fn dispatch(&self) -> fn(&mut World) { self.dispatch }

    pub(crate) const fn invocation(&self) -> Invocation { self.invocation }
}

#[derive(Clone, Copy)]
pub(crate) enum Invocation {
    OneShot,
    Held(CustomInput),
    Unremappable,
}

impl Invocation {
    fn from_capability(capability: Capability, custom_inputs: &mut CustomInputs) -> Self {
        match capability {
            Capability::OneShot => Self::OneShot,
            Capability::Held => Self::Held(custom_inputs.register_input()),
            Capability::Unremappable => Self::Unremappable,
        }
    }

    const fn custom_input(&self) -> Option<CustomInput> {
        match self {
            Self::Held(custom_input) => Some(*custom_input),
            Self::OneShot | Self::Unremappable => None,
        }
    }

    const fn held_command_lookup_outcome(&self) -> HeldCommandLookupOutcome {
        match self {
            Self::Held(custom_input) => {
                HeldCommandLookupOutcome::RegisteredHeldInput(*custom_input)
            },
            Self::OneShot | Self::Unremappable => HeldCommandLookupOutcome::KnownNonHeld,
        }
    }
}

struct CommandDeclaration {
    type_path:               &'static str,
    command:                 ReflectKeymapCommand,
    event_reflection_status: CommandEventReflectionStatus,
}

/// Whether a command event retained the reflection data BRP requires.
///
/// This converts Bevy's optional type data at the registry boundary so command declarations never
/// retain a bare optional reflection value whose absence has to be inferred later.
#[derive(Clone, Copy)]
enum CommandEventReflectionStatus {
    Reflected,
    Missing,
}

struct ValidatedDeclaration {
    command_id: CommandId,
    command:    ReflectKeymapCommand,
}

fn command_declarations(type_registry: &TypeRegistry) -> Vec<CommandDeclaration> {
    let mut declarations = type_registry
        .iter_with_data::<ReflectKeymapCommand>()
        .map(|(registration, command)| CommandDeclaration {
            type_path:               registration.type_info().type_path(),
            command:                 command.clone(),
            event_reflection_status: if registration.data::<ReflectEvent>().is_some() {
                CommandEventReflectionStatus::Reflected
            } else {
                CommandEventReflectionStatus::Missing
            },
        })
        .collect::<Vec<_>>();
    declarations.sort_by_key(|declaration| (declaration.command.id, declaration.type_path));
    declarations
}

fn validate_declarations(
    declarations: Vec<CommandDeclaration>,
) -> Result<Vec<ValidatedDeclaration>, Vec<Diagnostic>> {
    let mut command_ids = HashSet::new();
    let mut diagnostics = Vec::new();
    let mut validated_declarations = Vec::new();

    for declaration in declarations {
        let command_id = if let Ok(command_id) = CommandId::try_from(declaration.command.id) {
            Some(command_id)
        } else {
            diagnostics.push(registry_diagnostic(
                DiagnosticKind::InvalidCommandId,
                declaration.command.id,
                format!(
                    "Command event `{}` declares an invalid command ID.",
                    declaration.type_path
                ),
            ));
            None
        };

        let duplicate_id = command_id
            .as_ref()
            .is_some_and(|command_id| !command_ids.insert(command_id.clone()));
        if duplicate_id {
            diagnostics.push(registry_diagnostic(
                DiagnosticKind::DuplicateCommandId,
                declaration.command.id,
                format!(
                    "Command event `{}` duplicates command ID `{}`.",
                    declaration.type_path, declaration.command.id
                ),
            ));
        }

        let missing_title = declaration.command.title.is_empty();
        if missing_title {
            diagnostics.push(registry_diagnostic(
                DiagnosticKind::MissingCommandTitle,
                declaration.command.id,
                format!(
                    "Command ID `{}` has an empty title.",
                    declaration.command.id
                ),
            ));
        }

        let missing_description = declaration.command.description.is_empty();
        if missing_description {
            diagnostics.push(registry_diagnostic(
                DiagnosticKind::MissingCommandDescription,
                declaration.command.id,
                format!(
                    "Command ID `{}` has an empty description.",
                    declaration.command.id
                ),
            ));
        }

        let event_is_reflected = matches!(
            declaration.event_reflection_status,
            CommandEventReflectionStatus::Reflected
        );
        if !event_is_reflected {
            diagnostics.push(registry_diagnostic(
                DiagnosticKind::CommandEventNotReflected,
                declaration.command.id,
                format!(
                    "Command event `{}` is not reflected as an event.",
                    declaration.type_path
                ),
            ));
        }

        if let Some(command_id) = command_id
            && !duplicate_id
            && !missing_title
            && !missing_description
            && event_is_reflected
        {
            validated_declarations.push(ValidatedDeclaration {
                command_id,
                command: declaration.command,
            });
        }
    }

    if diagnostics.is_empty() {
        Ok(validated_declarations)
    } else {
        Err(diagnostics)
    }
}

fn registry_diagnostic(kind: DiagnosticKind, command_id: &str, message: String) -> Diagnostic {
    Diagnostic {
        origin: DiagnosticOrigin::CommandRegistration,
        byte_range: 0..0,
        line: 0,
        column: 0,
        block_index: 0,
        context: String::new(),
        original_keystroke: String::new(),
        command_id: command_id.to_owned(),
        kind,
        severity: DiagnosticSeverity::Failure,
        message,
        suggestions: Vec::new(),
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "command tests declare input action marker types without constructing them"
)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::any::type_name;

    use bevy::prelude::App;
    use bevy::prelude::AppTypeRegistry;
    use bevy::prelude::Event;
    use bevy::prelude::On;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::reflect::TypeRegistry;
    use bevy_enhanced_input::prelude::ActionValue;
    use bevy_enhanced_input::prelude::CustomInputs;
    use bevy_enhanced_input::prelude::InputAction;

    use super::CommandInvocationOutcome;
    use super::CommandLookup;
    use super::CommandRegistry;
    use super::HeldCommandLookupOutcome;
    use super::Invocation;
    use crate::Capability;
    use crate::CommandId;
    use crate::DiagnosticKind;
    use crate::DiagnosticOrigin;
    use crate::HoldPhase;
    use crate::KeymapCommand;
    use crate::ReflectKeymapCommand;

    crate::command! {
        action:      RegistryOneShotAction,
        event:       RegistryOneShot,
        id:          "registry::one_shot",
        title:       "Registry One Shot",
        description: "Triggers a one-shot command through the registry.",
    }

    crate::command! {
        held,
        action:      RegistryHeldAction,
        event:       RegistryHeld,
        id:          "registry::held",
        title:       "Registry Held",
        description: "Drives a custom input while the command is held.",
    }

    crate::command! {
        action:      RegistryUnremappableAction,
        event:       RegistryUnremappable,
        id:          "registry::unremappable",
        title:       "Registry Unremappable",
        description: "Triggers an unremappable command through the registry.",
        capability:  Unremappable,
    }

    crate::command! {
        action:      RegistryPaletteFirstAction,
        event:       RegistryPaletteFirst,
        id:          "registry::palette_first",
        title:       "Registry Palette Entry",
        description: "Sorts before the matching duplicate palette title.",
    }

    crate::command! {
        action:      RegistryPaletteSecondAction,
        event:       RegistryPaletteSecond,
        id:          "registry::palette_second",
        title:       "Registry Palette Entry",
        description: "Sorts after the matching duplicate palette title.",
    }

    #[derive(Event, Reflect)]
    #[reflect(Event, KeymapCommand)]
    struct DuplicateFirst;

    impl KeymapCommand for DuplicateFirst {
        const ID: &'static str = "registry::duplicate";
        const TITLE: &'static str = "First Duplicate";
        const DESCRIPTION: &'static str = "The first duplicate command declaration.";
        const CAPABILITY: crate::Capability = Capability::OneShot;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { None }
    }

    #[derive(Event, Reflect)]
    #[reflect(Event, KeymapCommand)]
    struct DuplicateSecond;

    impl KeymapCommand for DuplicateSecond {
        const ID: &'static str = "registry::duplicate";
        const TITLE: &'static str = "Second Duplicate";
        const DESCRIPTION: &'static str = "The second duplicate command declaration.";
        const CAPABILITY: crate::Capability = Capability::OneShot;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { None }
    }

    #[derive(Event, Reflect)]
    #[reflect(Event, KeymapCommand)]
    struct EmptyTitle;

    impl KeymapCommand for EmptyTitle {
        const ID: &'static str = "registry::empty_title";
        const TITLE: &'static str = "";
        const DESCRIPTION: &'static str = "A command declaration with an empty title.";
        const CAPABILITY: crate::Capability = Capability::OneShot;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { None }
    }

    #[derive(Event, Reflect)]
    #[reflect(Event, KeymapCommand)]
    struct EmptyDescription;

    impl KeymapCommand for EmptyDescription {
        const ID: &'static str = "registry::empty_description";
        const TITLE: &'static str = "Empty Description";
        const DESCRIPTION: &'static str = "";
        const CAPABILITY: crate::Capability = Capability::OneShot;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { None }
    }

    #[derive(Event, Reflect)]
    #[reflect(Event, KeymapCommand)]
    struct InvalidId;

    impl KeymapCommand for InvalidId {
        const ID: &'static str = "invalid";
        const TITLE: &'static str = "Invalid ID";
        const DESCRIPTION: &'static str = "A command declaration with malformed ID text.";
        const CAPABILITY: crate::Capability = Capability::OneShot;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { None }
    }

    #[derive(Event, Reflect)]
    #[reflect(KeymapCommand)]
    struct EventNotReflected;

    impl KeymapCommand for EventNotReflected {
        const ID: &'static str = "registry::event_not_reflected";
        const TITLE: &'static str = "Event Not Reflected";
        const DESCRIPTION: &'static str = "A command event without ReflectEvent type data.";
        const CAPABILITY: crate::Capability = Capability::OneShot;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { None }
    }

    #[derive(Resource, Default)]
    struct InvocationCount(usize);

    #[test]
    fn registry_rejects_duplicate_ids() {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<DuplicateFirst>();
        type_registry.register::<DuplicateSecond>();
        let mut custom_inputs = CustomInputs::default();

        let diagnostics = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .err()
            .expect("duplicate command IDs must be rejected");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::DuplicateCommandId
                && diagnostic.origin == DiagnosticOrigin::CommandRegistration
                && diagnostic.message.contains("registry::duplicate")
                && diagnostic.message.contains(type_name::<DuplicateSecond>())
        }));
    }

    #[test]
    fn registry_rejects_empty_titles_and_descriptions() {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<EmptyTitle>();
        type_registry.register::<EmptyDescription>();
        let mut custom_inputs = CustomInputs::default();

        let diagnostics = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .err()
            .expect("empty command text must be rejected");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::MissingCommandTitle
                && diagnostic.message.contains(EmptyTitle::ID)
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::MissingCommandDescription
                && diagnostic.message.contains(EmptyDescription::ID)
        }));
    }

    #[test]
    fn registry_rejects_hand_written_invalid_ids_and_unreflected_events() {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<InvalidId>();
        type_registry.register::<EventNotReflected>();
        let mut custom_inputs = CustomInputs::default();

        let diagnostics = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .err()
            .expect("hand-written command errors must be diagnosed");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::InvalidCommandId
                && diagnostic.message.contains(type_name::<InvalidId>())
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == DiagnosticKind::CommandEventNotReflected
                && diagnostic
                    .message
                    .contains(type_name::<EventNotReflected>())
        }));
    }

    #[test]
    fn registry_invokes_one_shot_from_an_opaque_command_id() {
        let mut app = App::new();
        app.world_mut().init_resource::<InvocationCount>();
        app.world_mut().add_observer(
            |_: On<RegistryOneShot>, mut invocation_count: ResMut<InvocationCount>| {
                invocation_count.0 += 1;
            },
        );

        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegistryOneShot>();
        let mut custom_inputs = CustomInputs::default();
        let command_registry = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .expect("a valid command must build a registry");
        let command_id = CommandId::try_from("registry::one_shot")
            .expect("the command macro validates the command ID");

        assert!(matches!(
            command_registry.lookup(&command_id),
            CommandLookup::Found(command_info)
                if command_info.title == RegistryOneShot::TITLE
                    && command_info.description == RegistryOneShot::DESCRIPTION
        ));
        assert_eq!(
            command_registry.invoke(&command_id, app.world_mut()),
            CommandInvocationOutcome::Invoked
        );
        assert_eq!(app.world().resource::<InvocationCount>().0, 1);
    }

    #[test]
    fn registry_invokes_unremappable_from_an_opaque_command_id() {
        let mut app = App::new();
        app.world_mut().init_resource::<InvocationCount>();
        app.world_mut().add_observer(
            |_: On<RegistryUnremappable>, mut invocation_count: ResMut<InvocationCount>| {
                invocation_count.0 += 1;
            },
        );

        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegistryUnremappable>();
        let mut custom_inputs = CustomInputs::default();
        let command_registry = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .expect("a valid command must build a registry");
        let command_id = CommandId::try_from("registry::unremappable")
            .expect("the command macro validates the command ID");

        assert_eq!(
            command_registry.invoke(&command_id, app.world_mut()),
            CommandInvocationOutcome::Invoked
        );
        assert_eq!(app.world().resource::<InvocationCount>().0, 1);
    }

    #[test]
    fn initialize_resolves_registered_commands_and_stable_metadata() {
        let mut app = App::new();
        app.world_mut().insert_resource(AppTypeRegistry::default());
        let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
        {
            let mut type_registry = app_type_registry.write();
            type_registry.register::<RegistryOneShot>();
            type_registry.register::<RegistryHeld>();
        }

        let command_registry = CommandRegistry::initialize(app.world_mut())
            .expect("registered commands must initialize");
        let command_id = CommandId::try_from(RegistryHeld::ID)
            .expect("the command macro validates the command ID");

        assert!(matches!(
            command_registry.lookup(&command_id),
            CommandLookup::Found(command_info)
                if command_info.title == RegistryHeld::TITLE
                    && command_info.capability == Capability::Held
        ));
        assert!(matches!(
            command_registry.held_command_lookup(&command_id),
            HeldCommandLookupOutcome::RegisteredHeldInput(_)
        ));
        assert_eq!(
            command_registry
                .iter()
                .map(|command| command.id.as_str())
                .collect::<Vec<_>>(),
            ["registry::held", "registry::one_shot"]
        );
    }

    #[test]
    fn held_command_registers_one_custom_input_and_updates_it_from_events() -> Result<(), String> {
        let mut app = App::new();
        app.world_mut().init_resource::<CustomInputs>();
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegistryHeld>();
        let held_event = RegistryHeld::build();
        assert_eq!(held_event.phase, HoldPhase::Begin);
        let command_registry = {
            let mut custom_inputs = app.world_mut().resource_mut::<CustomInputs>();
            CommandRegistry::build(&type_registry, &mut custom_inputs)
                .expect("a held command must build a registry")
        };
        command_registry.register_held_observers(app.world_mut());
        let command_id = CommandId::try_from(RegistryHeld::ID)
            .expect("the command macro validates the command ID");
        let custom_input = match command_registry.held_command_lookup(&command_id) {
            HeldCommandLookupOutcome::RegisteredHeldInput(custom_input) => custom_input,
            HeldCommandLookupOutcome::KnownNonHeld => {
                return Err(String::from("held command was reported as non-held"));
            },
            HeldCommandLookupOutcome::UnknownCommand => {
                return Err(String::from("held command was missing from the registry"));
            },
        };

        assert!(matches!(
            command_registry
                .entries
                .get(&command_id)
                .map(|entry| &entry.invocation),
            Some(Invocation::Held(_))
        ));
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            None
        );

        app.world_mut().trigger(RegistryHeld {
            phase: HoldPhase::End,
        });
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            None
        );

        assert_eq!(
            command_registry.invoke(&command_id, app.world_mut()),
            CommandInvocationOutcome::HeldCommandRequiresPhase
        );
        app.world_mut().trigger(RegistryHeld {
            phase: HoldPhase::Begin,
        });
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );

        app.world_mut().trigger(RegistryHeld {
            phase: HoldPhase::End,
        });
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );

        app.world_mut().trigger(RegistryHeld {
            phase: HoldPhase::End,
        });
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn held_command_lookup_distinguishes_registered_known_and_unknown_commands()
    -> Result<(), String> {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegistryHeld>();
        type_registry.register::<RegistryOneShot>();
        let mut custom_inputs = CustomInputs::default();
        let command_registry = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .map_err(|diagnostics| format!("command registry errors: {diagnostics:?}"))?;
        let held_command_id = CommandId::try_from(RegistryHeld::ID)
            .map_err(|error| format!("held command ID is invalid: {error}"))?;
        let one_shot_command_id = CommandId::try_from(RegistryOneShot::ID)
            .map_err(|error| format!("one-shot command ID is invalid: {error}"))?;
        let unknown_command_id = CommandId::try_from("registry::unknown")
            .map_err(|error| format!("unknown command ID is invalid: {error}"))?;

        assert!(matches!(
            command_registry.held_command_lookup(&held_command_id),
            HeldCommandLookupOutcome::RegisteredHeldInput(_)
        ));
        assert_eq!(
            command_registry.held_command_lookup(&one_shot_command_id),
            HeldCommandLookupOutcome::KnownNonHeld
        );
        assert_eq!(
            command_registry.held_command_lookup(&unknown_command_id),
            HeldCommandLookupOutcome::UnknownCommand
        );
        Ok(())
    }

    #[test]
    fn invoke_names_unknown_and_held_commands_without_dispatching_them() {
        let mut app = App::new();
        app.world_mut().init_resource::<InvocationCount>();
        app.world_mut().add_observer(
            |_: On<RegistryHeld>, mut invocation_count: ResMut<InvocationCount>| {
                invocation_count.0 += 1;
            },
        );

        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegistryHeld>();
        let mut custom_inputs = CustomInputs::default();
        let command_registry = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .expect("a valid command must build a registry");
        let held_command_id = CommandId::try_from(RegistryHeld::ID)
            .expect("the command macro validates the command ID");
        let unknown_command_id = CommandId::try_from("registry::absent")
            .expect("the test command ID text is well formed");

        assert_eq!(
            command_registry.invoke(&unknown_command_id, app.world_mut()),
            CommandInvocationOutcome::UnknownCommand
        );
        assert_eq!(
            command_registry.invoke(&held_command_id, app.world_mut()),
            CommandInvocationOutcome::HeldCommandRequiresPhase
        );
        assert_eq!(app.world().resource::<InvocationCount>().0, 0);
        assert_eq!(
            command_registry.lookup(&unknown_command_id),
            CommandLookup::UnknownCommand
        );
        assert!(matches!(
            command_registry.lookup(&held_command_id),
            CommandLookup::Found(command_info)
                if command_info.id == &held_command_id
                    && command_info.title == RegistryHeld::TITLE
                    && command_info.description == RegistryHeld::DESCRIPTION
                    && command_info.capability == Capability::Held
        ));
    }

    #[test]
    fn registry_dispatch_does_not_allocate_after_warmup() {
        let mut app = App::new();
        app.world_mut().init_resource::<InvocationCount>();
        app.world_mut().add_observer(
            |_: On<RegistryOneShot>, mut invocation_count: ResMut<InvocationCount>| {
                invocation_count.0 += 1;
            },
        );

        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegistryOneShot>();
        let mut custom_inputs = CustomInputs::default();
        let command_registry = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .expect("a valid command must build a registry");
        let command_id = CommandId::try_from(RegistryOneShot::ID)
            .expect("the command macro validates the command ID");

        assert_eq!(
            command_registry.invoke(&command_id, app.world_mut()),
            CommandInvocationOutcome::Invoked
        );
        let allocations_before = crate::TEST_ALLOCATOR.allocation_count();
        assert_eq!(
            command_registry.invoke(&command_id, app.world_mut()),
            CommandInvocationOutcome::Invoked
        );
        let allocations_after = crate::TEST_ALLOCATOR.allocation_count();

        assert_eq!(allocations_after - allocations_before, 0);
        assert_eq!(app.world().resource::<InvocationCount>().0, 2);
    }

    #[test]
    fn registry_accepts_commands_with_required_authored_text() {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegistryOneShot>();
        type_registry.register::<RegistryHeld>();
        let mut custom_inputs = CustomInputs::default();

        assert!(CommandRegistry::build(&type_registry, &mut custom_inputs).is_ok());
    }

    #[test]
    fn palette_search_index_normalizes_authored_metadata_and_breaks_title_ties_by_id()
    -> Result<(), String> {
        super::reset_palette_search_observations();
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<RegistryPaletteFirst>();
        type_registry.register::<RegistryPaletteSecond>();
        let mut custom_inputs = CustomInputs::default();
        let command_registry = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .map_err(|diagnostics| format!("registry palette diagnostics: {diagnostics:?}"))?;

        assert_eq!(
            super::palette_search_observations(),
            super::PaletteSearchObservations {
                normalizations:        4,
                ordered_search_passes: 0,
            }
        );

        super::reset_palette_search_observations();
        let normalized_query = super::normalize_palette_search_text("REGISTRY__PALETTE-entry");
        let matching_ids = command_registry
            .palette_search(&normalized_query)
            .filter_map(|matching_command| match matching_command {
                super::PaletteSearchMatchInvocability::Invocable(command) => {
                    Some(command.id().as_str())
                },
                super::PaletteSearchMatchInvocability::Held => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            matching_ids,
            [RegistryPaletteFirst::ID, RegistryPaletteSecond::ID]
        );
        assert_eq!(
            super::palette_search_observations(),
            super::PaletteSearchObservations {
                normalizations:        1,
                ordered_search_passes: 1,
            }
        );
        Ok(())
    }
}
