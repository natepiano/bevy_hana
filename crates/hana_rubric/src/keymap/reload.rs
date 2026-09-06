//! Compiled-keymap replacement from disk-worker source snapshots.

use std::path::PathBuf;
use std::str;
use std::str::Utf8Error;
use std::sync::Arc;

use bevy::ecs::world::World;
use bevy::prelude::Resource;

use super::AcceptedKeymapDocument;
use super::CompiledKeymap;
use super::EffectiveKeymapSnapshot;
use super::EffectiveKeymapStatus;
use super::KeymapBindingUnavailability;
use super::KeymapBindings;
use super::KeymapGeneration;
use super::LoadedKeymapBindings;
use super::MergedKeymap;
use super::merged::UserKeymap;
use crate::CommandRegistry;
use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;
use crate::KeymapLoadFailures;
use crate::Keystroke;
use crate::condition::ActiveKeymapContext;
use crate::condition::ActiveKeymapContextState;
use crate::condition::StateDimensionRegistry;
use crate::condition::StateDimensionRoutingRequirement;
use crate::disk::DiskDelivery;
use crate::disk::DiskWorkerMessage;
use crate::disk::MAX_RETAINED_DIAGNOSTICS;

/// Sources shared by every keymap replacement transaction.
#[derive(Clone, Resource)]
pub(crate) struct ReloadConfiguration {
    defaults_diagnostic_origin: DiagnosticOrigin,
    published_defaults:         String,
    protected_keystrokes:       Vec<Keystroke>,
    allow_default_failures:     bool,
}

impl ReloadConfiguration {
    pub(crate) const fn new(
        defaults_diagnostic_origin: DiagnosticOrigin,
        published_defaults: String,
        protected_keystrokes: Vec<Keystroke>,
        allow_default_failures: bool,
    ) -> Self {
        Self {
            defaults_diagnostic_origin,
            published_defaults,
            protected_keystrokes,
            allow_default_failures,
        }
    }

    #[cfg(test)]
    pub(crate) fn published_defaults(&self) -> &str { &self.published_defaults }
}

/// One source update waiting for the next reload transaction.
#[derive(Default, Resource)]
pub(crate) enum PendingReload {
    #[default]
    Idle,
    Requested(ReloadRequest),
}

impl PendingReload {
    pub(crate) fn replace(&mut self, request: ReloadRequest) { *self = Self::Requested(request); }

    const fn take(&mut self) -> Self { std::mem::replace(self, Self::Idle) }
}

/// Whether the disk worker read bytes for the user keymap file.
///
/// The bytes are shared rather than owned so that a snapshot the coalescing slot
/// supersedes before `take` costs a refcount bump instead of a copy of the whole
/// file.
pub(crate) enum UserKeymapContents {
    Read(Arc<[u8]>),
    Absent,
}

/// Transport-neutral user-keymap input for the reload transaction.
pub(crate) enum ReloadRequest {
    Defaults,
    DiskDiagnostics(Vec<Diagnostic>),
    UserSnapshot {
        source_path: PathBuf,
        contents:    UserKeymapContents,
        diagnostics: Vec<Diagnostic>,
    },
}

/// Whether a commit is establishing the initial defaults or applying a later disk update.
#[derive(Clone, Copy, Eq, PartialEq)]
enum KeymapCommitKind {
    InitialDefaults,
    Reload,
}

impl From<DiskWorkerMessage> for ReloadRequest {
    fn from(disk_worker_message: DiskWorkerMessage) -> Self {
        match disk_worker_message.delivery {
            DiskDelivery::DiagnosticsOnly => Self::DiskDiagnostics(disk_worker_message.diagnostics),
            DiskDelivery::Snapshot(disk_snapshot) => Self::UserSnapshot {
                source_path: disk_snapshot.source_path,
                contents:    disk_snapshot.contents,
                diagnostics: disk_worker_message.diagnostics,
            },
        }
    }
}

/// Commits the startup defaults synchronously before the disk worker starts.
pub(crate) fn commit_defaults(world: &mut World) -> bool {
    matches!(
        commit_request(world, ReloadRequest::Defaults),
        CommitOutcome::Committed
    )
}

/// Replaces the compiled keymap for the next available disk-worker request.
pub(crate) fn commit_reload(world: &mut World) {
    let pending_reload = world
        .get_resource_mut::<PendingReload>()
        .map_or(PendingReload::Idle, |mut pending_reload| {
            pending_reload.take()
        });
    let PendingReload::Requested(request) = pending_reload else {
        return;
    };

    let _ = commit_request(world, request);
}

fn commit_request(world: &mut World, request: ReloadRequest) -> CommitOutcome {
    let (user_keymap, disk_diagnostics, commit_kind) = match request {
        ReloadRequest::Defaults => (
            UserKeymap::DefaultsOnly,
            Vec::new(),
            KeymapCommitKind::InitialDefaults,
        ),
        ReloadRequest::DiskDiagnostics(diagnostics) => {
            record_transport_diagnostics(world, diagnostics);
            return CommitOutcome::NoChange;
        },
        ReloadRequest::UserSnapshot {
            source_path,
            contents,
            diagnostics,
        } => match contents {
            UserKeymapContents::Absent => (
                UserKeymap::DefaultsOnly,
                diagnostics,
                KeymapCommitKind::Reload,
            ),
            UserKeymapContents::Read(contents) => {
                let user_keymap = match str::from_utf8(&contents) {
                    Ok(source) => UserKeymap::Layered {
                        origin:   DiagnosticOrigin::KeymapFile(source_path),
                        contents: source.to_owned(),
                    },
                    Err(error) => {
                        let diagnostic = utf8_diagnostic(source_path, error);
                        record_load_diagnostics(world, vec![diagnostic]);
                        record_transport_diagnostics(world, diagnostics);
                        return CommitOutcome::NoChange;
                    },
                };

                (user_keymap, diagnostics, KeymapCommitKind::Reload)
            },
        },
    };

    let reload_configuration = world.resource::<ReloadConfiguration>().clone();
    commit_state_dimension_request(
        world,
        user_keymap,
        disk_diagnostics,
        commit_kind,
        reload_configuration,
    )
}

fn commit_state_dimension_request(
    world: &mut World,
    user_keymap: UserKeymap,
    disk_diagnostics: Vec<Diagnostic>,
    commit_kind: KeymapCommitKind,
    reload_configuration: ReloadConfiguration,
) -> CommitOutcome {
    if !disk_diagnostics.is_empty() {
        record_transport_diagnostics(world, disk_diagnostics);
    }
    let accepted_document =
        world.resource_scope::<CommandRegistry, _>(|world, command_registry| {
            let state_dimensions = world.resource::<StateDimensionRegistry>();
            AcceptedKeymapDocument::from_sources(
                &reload_configuration.defaults_diagnostic_origin,
                &reload_configuration.published_defaults,
                &user_keymap,
                &command_registry,
                state_dimensions,
                &reload_configuration.protected_keystrokes,
            )
        });
    let (accepted_document, diagnostics) = match accepted_document {
        Ok(result) => result,
        Err(diagnostics) => {
            record_load_diagnostics(world, diagnostics);
            if !world.contains_resource::<AcceptedKeymapDocument>() {
                world.insert_resource(EffectiveKeymapStatus::RejectedInitialDocument);
            }
            mark_invalid_initial_default(world, commit_kind);
            return CommitOutcome::NoChange;
        },
    };
    let defaults_failed = commit_kind == KeymapCommitKind::InitialDefaults
        && diagnostics.iter().any(|diagnostic| {
            diagnostic.origin == reload_configuration.defaults_diagnostic_origin
                && diagnostic.severity == DiagnosticSeverity::Failure
        });
    if defaults_failed && !reload_configuration.allow_default_failures {
        record_load_diagnostics(world, diagnostics);
        if !world.contains_resource::<AcceptedKeymapDocument>() {
            world.insert_resource(EffectiveKeymapStatus::RejectedInitialDocument);
        }
        mark_invalid_initial_default(world, commit_kind);
        return CommitOutcome::NoChange;
    }

    world.insert_resource(accepted_document);
    world.insert_resource(EffectiveKeymapStatus::AwaitingAcceptedDocument);
    record_load_diagnostics(world, diagnostics);
    commit_effective_keymap(world);
    CommitOutcome::Committed
}

/// Rematerializes one state-dimension document only when its effective identity changed.
pub(crate) fn commit_effective_keymap(world: &mut World) {
    let Some(accepted_document) = world.get_resource::<AcceptedKeymapDocument>() else {
        return;
    };
    let active_context = {
        let active_context = world.resource::<ActiveKeymapContext>();
        let current_status = world.resource::<EffectiveKeymapStatus>();
        let state_dimensions = world.resource::<StateDimensionRegistry>();
        if effective_status_matches(active_context.state(), current_status, state_dimensions) {
            return;
        }
        active_context.state().clone()
    };
    let snapshot = match &active_context {
        ActiveKeymapContextState::GlobalRouting => match world
            .resource::<StateDimensionRegistry>()
            .routing_requirement()
        {
            StateDimensionRoutingRequirement::GlobalOnly => EffectiveKeymapSnapshot::Global,
            StateDimensionRoutingRequirement::CompleteSnapshotRequired => {
                world.insert_resource(EffectiveKeymapStatus::UnmaterializableStateDimensions);
                return;
            },
        },
        ActiveKeymapContextState::AwaitingStateDimensions
        | ActiveKeymapContextState::StateDimensionsUnavailable { .. } => {
            world.insert_resource(EffectiveKeymapStatus::inactive(&active_context));
            return;
        },
        ActiveKeymapContextState::Resolved(snapshot) => {
            if !world
                .resource::<StateDimensionRegistry>()
                .recognizes_complete_snapshot(snapshot)
            {
                world.insert_resource(EffectiveKeymapStatus::UnmaterializableStateDimensions);
                return;
            }
            EffectiveKeymapSnapshot::Resolved(snapshot.clone())
        },
    };
    let mut materialization = accepted_document.materialize(snapshot);
    let generation = next_generation(world);
    materialization.publication.generation = generation;
    let (merged_keymap, diagnostics) =
        world.resource_scope::<CommandRegistry, _>(|_, command_registry| {
            MergedKeymap::from_effective_state_bindings(materialization.bindings, &command_registry)
        });
    let compiled_keymap = world.resource_scope::<CommandRegistry, _>(|_, command_registry| {
        merged_keymap.compile(generation, &command_registry)
    });
    let loaded_keymap_bindings = LoadedKeymapBindings::from_state_materialization(
        generation,
        materialization.publication.snapshot.clone(),
        &merged_keymap,
    );
    debug_assert_eq!(
        compiled_keymap.generation,
        loaded_keymap_bindings.generation()
    );
    world.insert_resource(compiled_keymap);
    KeymapBindings::replace_loaded(world, loaded_keymap_bindings);
    world.insert_resource(EffectiveKeymapStatus::Loaded(materialization.publication));
    if !diagnostics.is_empty() {
        record_load_diagnostics(world, diagnostics);
    }
}

fn effective_status_matches(
    active_context: &ActiveKeymapContextState,
    effective_status: &EffectiveKeymapStatus,
    state_dimensions: &StateDimensionRegistry,
) -> bool {
    match (active_context, effective_status) {
        (ActiveKeymapContextState::GlobalRouting, EffectiveKeymapStatus::Loaded(publication)) => {
            state_dimensions.routing_requirement() == StateDimensionRoutingRequirement::GlobalOnly
                && publication.snapshot == EffectiveKeymapSnapshot::Global
        },
        (
            ActiveKeymapContextState::Resolved(snapshot),
            EffectiveKeymapStatus::Loaded(publication),
        ) => {
            matches!(
                &publication.snapshot,
                EffectiveKeymapSnapshot::Resolved(published_snapshot) if published_snapshot == snapshot
            )
        },
        (
            ActiveKeymapContextState::AwaitingStateDimensions,
            EffectiveKeymapStatus::AwaitingStateDimensions,
        ) => true,
        (
            ActiveKeymapContextState::StateDimensionsUnavailable { missing },
            EffectiveKeymapStatus::StateDimensionsUnavailable {
                missing: current_missing,
            },
        ) => missing == current_missing,
        (
            ActiveKeymapContextState::GlobalRouting,
            EffectiveKeymapStatus::UnmaterializableStateDimensions,
        ) => {
            state_dimensions.routing_requirement()
                == StateDimensionRoutingRequirement::CompleteSnapshotRequired
        },
        (
            ActiveKeymapContextState::Resolved(snapshot),
            EffectiveKeymapStatus::UnmaterializableStateDimensions,
        ) => !state_dimensions.recognizes_complete_snapshot(snapshot),
        _ => false,
    }
}

fn mark_invalid_initial_default(world: &mut World, commit_kind: KeymapCommitKind) {
    if commit_kind == KeymapCommitKind::InitialDefaults
        && !world.contains_resource::<CompiledKeymap>()
    {
        KeymapBindings::replace_unavailability(world, KeymapBindingUnavailability::InvalidDefault);
    }
}

fn next_generation(world: &World) -> KeymapGeneration {
    world
        .get_resource::<CompiledKeymap>()
        .map_or(KeymapGeneration::initial(), |compiled_keymap| {
            compiled_keymap.generation.next()
        })
}

fn utf8_diagnostic(source_path: PathBuf, error: Utf8Error) -> Diagnostic {
    Diagnostic {
        origin:             DiagnosticOrigin::KeymapFile(source_path),
        byte_range:         0..0,
        line:               0,
        column:             0,
        block_index:        0,
        context:            String::new(),
        original_keystroke: String::new(),
        command_id:         String::new(),
        kind:               DiagnosticKind::Syntax,
        severity:           DiagnosticSeverity::Failure,
        message:            format!("The user keymap is not valid UTF-8: {error}"),
        suggestions:        Vec::new(),
    }
}

/// Replaces the diagnostics a merge re-derives with the ones this merge
/// produced, leaving the transport diagnostics `record_transport_diagnostics`
/// recorded in place.
///
/// Every [`is_reload_diagnostic`] kind is re-derived from the same two documents
/// on each commit, at every severity, so all of them are purged before the new
/// batch lands — retaining any of them would report one authoring mistake once
/// per commit.
fn record_load_diagnostics(world: &mut World, diagnostics: Vec<Diagnostic>) {
    log_diagnostics(&diagnostics);
    let mut keymap_load_failures = world.resource_mut::<KeymapLoadFailures>();
    keymap_load_failures
        .diagnostics
        .retain(|diagnostic| !is_reload_diagnostic(diagnostic));
    keymap_load_failures.diagnostics.extend(diagnostics);
    retain_recent_diagnostics(&mut keymap_load_failures.diagnostics);
}

fn record_transport_diagnostics(world: &mut World, diagnostics: Vec<Diagnostic>) {
    log_diagnostics(&diagnostics);
    let mut keymap_load_failures = world.resource_mut::<KeymapLoadFailures>();
    keymap_load_failures.diagnostics.extend(diagnostics);
    retain_recent_diagnostics(&mut keymap_load_failures.diagnostics);
}

/// Whether `AcceptedKeymapDocument::from_sources` and
/// `MergedKeymap::from_effective_state_bindings` re-derive this kind from the
/// defaults and user documents on every commit.
///
/// The kinds left out are recorded once and never re-derived: `Disk` and
/// `Companion` come from the disk worker's transport,
/// `MissingDefaultKeymap` and `UnconfiguredKeymapPlugin` from
/// `KeymapPlugin::build`, and `DuplicateCommandId`, `MissingCommandTitle`,
/// `MissingCommandDescription`, `InvalidCommandId` and
/// `CommandEventNotReflected` from `CommandRegistry::build`. Purging those would
/// drop a report no later merge produces again.
const fn is_reload_diagnostic(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic.kind,
        DiagnosticKind::Syntax
            | DiagnosticKind::Keystroke
            | DiagnosticKind::Command
            | DiagnosticKind::Context
            | DiagnosticKind::ReservedKeystroke
            | DiagnosticKind::BareModifierRequiresHeldCommand
            | DiagnosticKind::UnremappableCommand
            | DiagnosticKind::HeldCommandInSequence
    )
}

fn retain_recent_diagnostics(diagnostics: &mut Vec<Diagnostic>) {
    let discarded = diagnostics.len().saturating_sub(MAX_RETAINED_DIAGNOSTICS);
    if discarded > 0 {
        diagnostics.drain(0..discarded);
    }
}

fn log_diagnostics(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        match diagnostic.severity {
            DiagnosticSeverity::Failure => {
                bevy::log::error!("{}", diagnostic.message);
            },
            DiagnosticSeverity::Advisory => {
                bevy::log::warn!("{}", diagnostic.message);
            },
        }
    }
}

enum CommitOutcome {
    Committed,
    NoChange,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::Instant;

    use bevy::input::ButtonInput;
    use bevy::input::keyboard::KeyCode;
    use bevy::prelude::App;
    use bevy::prelude::Event;
    use bevy::prelude::On;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::reflect::TypeRegistry;
    use bevy_enhanced_input::prelude::ActionValue;
    use bevy_enhanced_input::prelude::CustomInput;
    use bevy_enhanced_input::prelude::CustomInputs;

    use super::PendingReload;
    use super::ReloadConfiguration;
    use super::ReloadRequest;
    use super::UserKeymapContents;
    use super::commit_defaults;
    use super::commit_reload;
    use crate::ActiveKeymapContext;
    use crate::AuthoredKeymapBindings;
    use crate::Capability;
    use crate::CommandId;
    use crate::CommandKeystroke;
    use crate::CommandRegistry;
    use crate::DiagnosticKind;
    use crate::DiagnosticOrigin;
    use crate::DiagnosticSeverity;
    use crate::EffectiveKeymapStatus;
    use crate::HeldCommandLookupOutcome;
    use crate::HoldPhase;
    use crate::KeymapBindings;
    use crate::KeymapCommand;
    use crate::KeymapLoadFailures;
    use crate::KeymapPlugin;
    use crate::KeystrokeSequence;
    use crate::MatchOutcome;
    use crate::PaletteBinding;
    use crate::ReflectKeymapCommand;
    use crate::condition::StateDimensionRegistry;
    use crate::keymap::CompiledKeymap;
    use crate::keymap::KeymapGeneration;
    use crate::keymap::runtime;
    use crate::keymap::runtime::KeymapRuntime;
    use crate::query_command_palette;

    const DEFAULTS_PATH: &str = "published-defaults.jsonc";
    const USER_KEYMAP_FIXTURE: &str = "user-keymap.jsonc";
    const MATCH_TIMEOUT: Duration = Duration::from_secs(1);

    #[derive(Default, Event, Reflect)]
    #[reflect(Event, KeymapCommand)]
    struct ReloadFirst;

    impl KeymapCommand for ReloadFirst {
        const ID: &'static str = "reload::first";
        const TITLE: &'static str = "Reload First";
        const DESCRIPTION: &'static str = "First command used by reload transaction tests.";
        const CAPABILITY: Capability = Capability::OneShot;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { None }
    }

    #[derive(Default, Event, Reflect)]
    #[reflect(Event, KeymapCommand)]
    struct ReloadSecond;

    impl KeymapCommand for ReloadSecond {
        const ID: &'static str = "reload::second";
        const TITLE: &'static str = "Reload Second";
        const DESCRIPTION: &'static str = "Second command used by reload transaction tests.";
        const CAPABILITY: Capability = Capability::OneShot;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { None }
    }

    #[derive(Default, Event, Reflect)]
    #[reflect(Event, KeymapCommand)]
    struct ReloadHeld;

    impl KeymapCommand for ReloadHeld {
        const ID: &'static str = "reload::held";
        const TITLE: &'static str = "Reload Held";
        const DESCRIPTION: &'static str = "Held command used by reload transaction tests.";
        const CAPABILITY: Capability = Capability::Held;

        fn build() -> Self { Self }

        fn hold_phase(&self) -> Option<HoldPhase> { Some(HoldPhase::Begin) }
    }

    #[derive(Default, Resource)]
    struct ReloadDispatchCount(usize);

    fn transaction_app(defaults: &str) -> Result<App, String> {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<ReloadFirst>();
        type_registry.register::<ReloadSecond>();
        type_registry.register::<ReloadHeld>();
        let mut custom_inputs = CustomInputs::default();
        let command_registry = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .map_err(|diagnostics| format!("reload registry diagnostics: {diagnostics:?}"))?;
        let mut app = App::new();

        app.insert_resource(command_registry)
            .insert_resource(custom_inputs)
            .insert_resource(StateDimensionRegistry::default())
            .init_resource::<ActiveKeymapContext>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<KeymapLoadFailures>()
            .init_resource::<KeymapBindings>()
            .init_resource::<KeymapRuntime>()
            .init_resource::<PendingReload>()
            .init_resource::<ReloadDispatchCount>()
            .insert_resource(ReloadConfiguration::new(
                DiagnosticOrigin::KeymapFile(PathBuf::from(DEFAULTS_PATH)),
                defaults.to_owned(),
                Vec::new(),
                false,
            ));
        app.world_mut().add_observer(
            |_: On<ReloadFirst>, mut reload_dispatch_count: ResMut<ReloadDispatchCount>| {
                reload_dispatch_count.0 += 1;
            },
        );
        app.world_mut().add_observer(
            |_: On<ReloadSecond>, mut reload_dispatch_count: ResMut<ReloadDispatchCount>| {
                reload_dispatch_count.0 += 1;
            },
        );
        Ok(app)
    }

    fn scheduled_transaction_app(defaults: &'static str) -> Result<App, String> {
        let mut type_registry = TypeRegistry::default();
        type_registry.register::<ReloadFirst>();
        type_registry.register::<ReloadSecond>();
        type_registry.register::<ReloadHeld>();
        let mut custom_inputs = CustomInputs::default();
        let command_registry = CommandRegistry::build(&type_registry, &mut custom_inputs)
            .map_err(|diagnostics| format!("reload registry diagnostics: {diagnostics:?}"))?;
        let mut app = App::new();
        KeymapPlugin::install_runtime(&mut app);

        app.insert_resource(command_registry)
            .insert_resource(custom_inputs)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ReloadDispatchCount>()
            .insert_resource(ReloadConfiguration::new(
                DiagnosticOrigin::KeymapFile(PathBuf::from(DEFAULTS_PATH)),
                defaults.to_owned(),
                Vec::new(),
                false,
            ));
        app.world_mut().add_observer(
            |_: On<ReloadFirst>, mut reload_dispatch_count: ResMut<ReloadDispatchCount>| {
                reload_dispatch_count.0 += 1;
            },
        );
        app.world_mut().add_observer(
            |_: On<ReloadSecond>, mut reload_dispatch_count: ResMut<ReloadDispatchCount>| {
                reload_dispatch_count.0 += 1;
            },
        );
        Ok(app)
    }

    fn queue_defaults(app: &mut App) {
        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::Defaults);
    }

    fn queue_user_snapshot(app: &mut App, contents: &[u8]) {
        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(contents)),
                diagnostics: Vec::new(),
            });
    }

    fn global_matches(app: &mut App, keystroke: &str) -> Result<bool, String> {
        let sequence = keystroke
            .parse::<KeystrokeSequence>()
            .map_err(|error| format!("invalid test keystroke: {error}"))?;
        let match_outcome = app
            .world_mut()
            .resource_mut::<CompiledKeymap>()
            .matcher
            .match_keystroke(sequence.first(), Instant::now(), MATCH_TIMEOUT);

        Ok(matches!(match_outcome, MatchOutcome::Matched(_)))
    }

    fn held_custom_input(app: &App) -> Result<CustomInput, String> {
        let command_id = CommandId::try_from(ReloadHeld::ID)
            .map_err(|error| format!("invalid held command ID: {error}"))?;

        match app
            .world()
            .resource::<CommandRegistry>()
            .held_command_lookup(&command_id)
        {
            HeldCommandLookupOutcome::RegisteredHeldInput(custom_input) => Ok(custom_input),
            HeldCommandLookupOutcome::KnownNonHeld => {
                Err(String::from("reload held command is not held"))
            },
            HeldCommandLookupOutcome::UnknownCommand => {
                Err(String::from("reload held command is not registered"))
            },
        }
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        runtime::route_input(app.world_mut());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(key);
    }

    fn release(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(key);
        runtime::route_input(app.world_mut());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_released(key);
    }

    fn assert_palette_row(
        app: &App,
        query: &str,
        expected_command_id: &CommandId,
        expected_binding: PaletteBinding<'_, '_>,
    ) -> Result<(), String> {
        let world = app.world();
        let query_result = query_command_palette(
            world.resource::<CommandRegistry>(),
            world.resource::<ActiveKeymapContext>(),
            world.resource::<EffectiveKeymapStatus>(),
            world.resource::<KeymapBindings>(),
            query,
        );
        if query_result.rows().len() != 1 {
            return Err(format!(
                "palette query `{query}` returned {} rows instead of one",
                query_result.rows().len()
            ));
        }

        let row = &query_result.rows()[0];
        assert_eq!(row.command().id(), expected_command_id);
        assert_eq!(row.binding(), expected_binding);
        Ok(())
    }

    fn assert_rejected_reload_preserves_palette_and_dispatch(
        app: &mut App,
        rejected_snapshot: &[u8],
        generation: KeymapGeneration,
        command_id: &CommandId,
        representative_binding: &KeystrokeSequence,
        expected_dispatches: usize,
    ) -> Result<(), String> {
        let prior_reload_failure_evidence = app
            .world()
            .resource::<KeymapLoadFailures>()
            .diagnostics
            .clone();
        queue_user_snapshot(app, rejected_snapshot);
        app.update();

        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation,
            generation
        );
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings) if bindings.generation() == generation
        ));
        assert_palette_row(
            app,
            "reload second",
            command_id,
            PaletteBinding::BoundTo(representative_binding),
        )?;
        let current_reload_failure_evidence =
            &app.world().resource::<KeymapLoadFailures>().diagnostics;
        assert_ne!(
            current_reload_failure_evidence,
            &prior_reload_failure_evidence
        );
        assert!(current_reload_failure_evidence.iter().any(|diagnostic| {
            diagnostic.origin == DiagnosticOrigin::KeymapFile(PathBuf::from(USER_KEYMAP_FIXTURE))
        }));
        press(app, KeyCode::KeyH);
        assert_eq!(
            app.world().resource::<ReloadDispatchCount>().0,
            expected_dispatches
        );
        release(app, KeyCode::KeyH);
        press(app, KeyCode::KeyG);
        assert_eq!(
            app.world().resource::<ReloadDispatchCount>().0,
            expected_dispatches
        );
        release(app, KeyCode::KeyG);
        Ok(())
    }

    /// One authoring mistake in the defaults document is one row, however many
    /// times the keymap commits: the defaults are re-merged on every commit, so
    /// a diagnostic retained across commits would be reported once per commit.
    #[test]
    fn an_advisory_from_the_defaults_document_survives_two_commits_as_one_row() -> Result<(), String>
    {
        const DEFAULTS_WITH_UNRECOGNIZED_MEMBER: &str =
            r#"{ "bindings": [{ "contxt": "always", "bindings": { "g": "reload::first" } }] }"#;

        let mut app = transaction_app(DEFAULTS_WITH_UNRECOGNIZED_MEMBER)?;
        assert!(commit_defaults(app.world_mut()));
        let after_first_commit = app
            .world()
            .resource::<KeymapLoadFailures>()
            .diagnostics
            .len();

        queue_user_snapshot(&mut app, br#"{ "bindings": [] }"#);
        commit_reload(app.world_mut());

        assert_eq!(after_first_commit, 1);
        assert_eq!(
            app.world()
                .resource::<KeymapLoadFailures>()
                .diagnostics
                .len(),
            after_first_commit
        );
        Ok(())
    }

    #[test]
    fn malformed_user_document_retains_defaults_diagnostics_and_effective_generation()
    -> Result<(), String> {
        const DEFAULTS_WITH_ROOT_ADVISORY: &str = r#"{
            "bindings": [{ "bindings": { "g": "reload::first" } }],
            "bindngs": []
        }"#;

        let mut app = transaction_app(DEFAULTS_WITH_ROOT_ADVISORY)?;
        assert!(commit_defaults(app.world_mut()));
        let generation = app.world().resource::<CompiledKeymap>().generation;
        let bindings_generation = match app.world().resource::<KeymapBindings>().authored() {
            AuthoredKeymapBindings::Loaded(bindings) => bindings.generation(),
            AuthoredKeymapBindings::Unavailable(_) => {
                return Err(String::from("defaults did not publish bindings"));
            },
        };

        queue_user_snapshot(&mut app, br#"{ "bindings": [}"#);
        commit_reload(app.world_mut());

        let diagnostics = &app.world().resource::<KeymapLoadFailures>().diagnostics;
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics[0].origin,
            DiagnosticOrigin::KeymapFile(PathBuf::from(DEFAULTS_PATH))
        );
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Syntax);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Advisory);
        assert!(diagnostics[0].message.contains("bindngs"));
        assert_eq!(
            diagnostics[1].origin,
            DiagnosticOrigin::KeymapFile(PathBuf::from(USER_KEYMAP_FIXTURE))
        );
        assert_eq!(diagnostics[1].kind, DiagnosticKind::Syntax);
        assert_eq!(diagnostics[1].severity, DiagnosticSeverity::Failure);
        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation,
            generation
        );
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings)
                if bindings.generation() == bindings_generation
        ));
        Ok(())
    }

    /// The held-prefix conflict is re-derived by `reject_held_prefixes` on every
    /// merge, the same way a syntax advisory is, so it is one row across commits
    /// too — the purge covers every kind the merge produces, not just the
    /// parse-time ones.
    #[test]
    fn a_held_prefix_conflict_in_the_defaults_document_survives_two_commits_as_one_row()
    -> Result<(), String> {
        const DEFAULTS_WITH_HELD_PREFIX_CONFLICT: &str = r#"{
            "bindings": [{ "bindings": {
                "a": "reload::held",
                "a b": "reload::first"
            } }]
        }"#;

        let mut app = transaction_app(DEFAULTS_WITH_HELD_PREFIX_CONFLICT)?;
        assert!(commit_defaults(app.world_mut()));
        let after_first_commit = app
            .world()
            .resource::<KeymapLoadFailures>()
            .diagnostics
            .len();

        queue_user_snapshot(&mut app, br#"{ "bindings": [] }"#);
        commit_reload(app.world_mut());

        assert_eq!(after_first_commit, 1);
        assert_eq!(
            app.world()
                .resource::<KeymapLoadFailures>()
                .diagnostics
                .iter()
                .filter(|diagnostic| { diagnostic.kind == DiagnosticKind::HeldCommandInSequence })
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn whole_file_failure_keeps_generation_and_pending_sequence_untouched() -> Result<(), String> {
        let defaults = r#"{
            "bindings": [{ "bindings": {
                "a": "reload::held",
                "g h": "reload::first"
            } }]
        }"#;
        let mut app = transaction_app(defaults)?;

        assert!(commit_defaults(app.world_mut()));
        let custom_input = held_custom_input(&app)?;
        press(&mut app, KeyCode::KeyA);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        let generation = app.world().resource::<CompiledKeymap>().generation;
        let bindings_generation = match app.world().resource::<KeymapBindings>().authored() {
            AuthoredKeymapBindings::Loaded(bindings) => bindings.generation(),
            AuthoredKeymapBindings::Unavailable(_) => {
                return Err(String::from(
                    "successful default load left bindings unavailable",
                ));
            },
        };
        let sequence = "g h"
            .parse::<KeystrokeSequence>()
            .map_err(|error| format!("invalid pending test sequence: {error}"))?;
        let first_match = app
            .world_mut()
            .resource_mut::<CompiledKeymap>()
            .matcher
            .match_keystroke(sequence.first(), Instant::now(), MATCH_TIMEOUT);

        assert!(matches!(first_match, MatchOutcome::Pending));
        queue_user_snapshot(&mut app, br#"{ "bindings": ["#);
        commit_reload(app.world_mut());

        let compiled_keymap = app.world().resource::<CompiledKeymap>();
        assert_eq!(compiled_keymap.generation, generation);
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings)
                if bindings.generation() == bindings_generation
        ));
        assert!(compiled_keymap.matcher.is_pending());
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        assert!(
            app.world()
                .resource::<KeymapLoadFailures>()
                .diagnostics
                .iter()
                .any(|diagnostic| {
                    diagnostic.origin
                        == DiagnosticOrigin::KeymapFile(PathBuf::from(USER_KEYMAP_FIXTURE))
                })
        );
        Ok(())
    }

    #[test]
    fn a_successful_reload_replaces_dispatch_and_bindings_with_one_generation() -> Result<(), String>
    {
        let defaults = r#"{ "bindings": [{ "bindings": { "g": "reload::first" } }] }"#;
        let mut app = transaction_app(defaults)?;
        let first_command = CommandId::try_from(ReloadFirst::ID)
            .map_err(|error| format!("invalid first reload command ID: {error}"))?;
        let second_command = CommandId::try_from(ReloadSecond::ID)
            .map_err(|error| format!("invalid second reload command ID: {error}"))?;
        let first_binding = "g"
            .parse::<KeystrokeSequence>()
            .map_err(|error| format!("invalid first reload binding: {error}"))?;
        let second_binding = "h"
            .parse::<KeystrokeSequence>()
            .map_err(|error| format!("invalid second reload binding: {error}"))?;

        assert!(commit_defaults(app.world_mut()));
        let first_generation = app.world().resource::<CompiledKeymap>().generation;
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&first_command),
            CommandKeystroke::BoundTo(&first_binding)
        );

        queue_user_snapshot(
            &mut app,
            br#"{
                "bindings": [{ "bindings": {
                    "g": null,
                    "h": "reload::second"
                }}]
            }"#,
        );
        commit_reload(app.world_mut());

        let compiled_keymap = app.world().resource::<CompiledKeymap>();
        assert_ne!(compiled_keymap.generation, first_generation);
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings)
                if bindings.generation() == compiled_keymap.generation
        ));
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&first_command),
            CommandKeystroke::Unbound
        );
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&second_command),
            CommandKeystroke::BoundTo(&second_binding)
        );
        Ok(())
    }

    #[test]
    fn reload_while_a_key_is_held_releases_its_previous_physical_source() -> Result<(), String> {
        let defaults = r#"{ "bindings": [{ "bindings": { "a": "reload::held" } }] }"#;
        let mut app = transaction_app(defaults)?;

        assert!(commit_defaults(app.world_mut()));
        let custom_input = held_custom_input(&app)?;
        press(&mut app, KeyCode::KeyA);
        queue_user_snapshot(&mut app, defaults.as_bytes());
        commit_reload(app.world_mut());
        runtime::route_input(app.world_mut());

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn reloading_a_held_binding_moves_its_physical_source() -> Result<(), String> {
        let defaults = r#"{ "bindings": [{ "bindings": { "a": "reload::held" } }] }"#;
        let mut app = transaction_app(defaults)?;

        assert!(commit_defaults(app.world_mut()));
        let custom_input = held_custom_input(&app)?;
        press(&mut app, KeyCode::KeyA);
        queue_user_snapshot(
            &mut app,
            br#"{
                "bindings": [{ "bindings": {
                    "a": null,
                    "b": "reload::held"
                } }]
            }"#,
        );
        commit_reload(app.world_mut());
        runtime::route_input(app.world_mut());
        release(&mut app, KeyCode::KeyA);
        press(&mut app, KeyCode::KeyA);

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        release(&mut app, KeyCode::KeyA);
        press(&mut app, KeyCode::KeyB);
        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(true))
        );
        Ok(())
    }

    #[test]
    fn generation_change_discards_a_pending_sequence() -> Result<(), String> {
        let defaults = r#"{ "bindings": [{ "bindings": { "g h": "reload::first" } }] }"#;
        let mut app = transaction_app(defaults)?;

        assert!(commit_defaults(app.world_mut()));
        let generation = app.world().resource::<CompiledKeymap>().generation;
        press(&mut app, KeyCode::KeyG);
        release(&mut app, KeyCode::KeyG);
        assert!(
            app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );
        queue_user_snapshot(
            &mut app,
            br#"{ "bindings": [{ "bindings": { "j": "reload::second" } }] }"#,
        );
        commit_reload(app.world_mut());
        press(&mut app, KeyCode::KeyH);

        assert_ne!(
            app.world().resource::<CompiledKeymap>().generation,
            generation
        );
        assert!(
            !app.world()
                .resource::<CompiledKeymap>()
                .matcher
                .is_pending()
        );
        assert_eq!(app.world().resource::<ReloadDispatchCount>().0, 0);
        Ok(())
    }

    #[test]
    fn reloading_multiple_held_aliases_releases_the_shared_action() -> Result<(), String> {
        let defaults = r#"{
            "bindings": [{ "bindings": {
                "a": "reload::held",
                "b": "reload::held"
            } }]
        }"#;
        let mut app = transaction_app(defaults)?;

        assert!(commit_defaults(app.world_mut()));
        let custom_input = held_custom_input(&app)?;
        press(&mut app, KeyCode::KeyA);
        press(&mut app, KeyCode::KeyB);
        queue_user_snapshot(&mut app, defaults.as_bytes());
        commit_reload(app.world_mut());
        runtime::route_input(app.world_mut());

        assert_eq!(
            app.world().resource::<CustomInputs>().get(&custom_input),
            Some(&ActionValue::Bool(false))
        );
        Ok(())
    }

    #[test]
    fn partial_failure_commits_defaults_and_every_valid_user_edit() -> Result<(), String> {
        let defaults = r#"{ "bindings": [{ "bindings": { "g": "reload::first" } }] }"#;
        let mut app = transaction_app(defaults)?;

        assert!(commit_defaults(app.world_mut()));
        queue_user_snapshot(
            &mut app,
            br#"{
                "bindings": [{
                    "bindings": {
                        "h": "reload::second",
                        "j": "unknown::command"
                    }
                }]
            }"#,
        );
        commit_reload(app.world_mut());

        assert!(global_matches(&mut app, "g")?);
        assert!(global_matches(&mut app, "h")?);
        assert!(!global_matches(&mut app, "j")?);
        assert!(
            app.world()
                .resource::<KeymapLoadFailures>()
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.command_id == "unknown::command")
        );
        Ok(())
    }

    #[test]
    fn scheduled_reloads_keep_palette_rows_and_keyboard_dispatch_on_one_generation()
    -> Result<(), String> {
        let defaults = r#"{
            "bindings": [{ "bindings": {
                "ctrl-g": "reload::first",
                "g": "reload::first"
            } }]
        }"#;
        let mut app = scheduled_transaction_app(defaults)?;
        let first_command_id = CommandId::try_from(ReloadFirst::ID)
            .map_err(|error| format!("invalid first reload command ID: {error}"))?;
        let second_command_id = CommandId::try_from(ReloadSecond::ID)
            .map_err(|error| format!("invalid second reload command ID: {error}"))?;
        let first_representative_binding = "g"
            .parse::<KeystrokeSequence>()
            .map_err(|error| format!("invalid first representative binding: {error}"))?;
        let second_representative_binding = "h"
            .parse::<KeystrokeSequence>()
            .map_err(|error| format!("invalid second representative binding: {error}"))?;

        assert_palette_row(
            &app,
            "reload first",
            &first_command_id,
            PaletteBinding::KeymapUnavailable(
                crate::KeymapBindingUnavailability::AwaitingInitialLoad,
            ),
        )?;

        queue_defaults(&mut app);
        app.update();

        let first_generation = app.world().resource::<CompiledKeymap>().generation;
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings)
                if bindings.generation() == first_generation
        ));
        assert_palette_row(
            &app,
            "reload first",
            &first_command_id,
            PaletteBinding::BoundTo(&first_representative_binding),
        )?;
        press(&mut app, KeyCode::KeyG);
        assert_eq!(app.world().resource::<ReloadDispatchCount>().0, 1);
        release(&mut app, KeyCode::KeyG);

        queue_user_snapshot(
            &mut app,
            br#"{
                "bindings": [{ "bindings": {
                    "ctrl-g": null,
                    "g": null,
                    "h": "reload::second"
                } }]
            }"#,
        );
        app.update();

        let reloaded_generation = app.world().resource::<CompiledKeymap>().generation;
        assert_ne!(reloaded_generation, first_generation);
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings)
                if bindings.generation() == reloaded_generation
        ));
        assert_palette_row(
            &app,
            "reload second",
            &second_command_id,
            PaletteBinding::BoundTo(&second_representative_binding),
        )?;
        press(&mut app, KeyCode::KeyH);
        assert_eq!(app.world().resource::<ReloadDispatchCount>().0, 2);
        release(&mut app, KeyCode::KeyH);
        press(&mut app, KeyCode::KeyG);
        assert_eq!(app.world().resource::<ReloadDispatchCount>().0, 2);
        release(&mut app, KeyCode::KeyG);

        for (expected_dispatches, rejected_snapshot) in [
            (3, &br#"{ "bindings": ["#[..]),
            (4, &br#"{ "bindings": [{ "#[..]),
        ] {
            assert_rejected_reload_preserves_palette_and_dispatch(
                &mut app,
                rejected_snapshot,
                reloaded_generation,
                &second_command_id,
                &second_representative_binding,
                expected_dispatches,
            )?;
        }
        Ok(())
    }
}
