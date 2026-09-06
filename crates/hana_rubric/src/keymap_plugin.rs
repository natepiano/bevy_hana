//! Keymap plugin configuration and system ordering.

use bevy::app::App;
use bevy::app::Plugin;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::SystemSet;
use bevy::input::InputSystems;
use bevy::prelude::PreUpdate;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::States;
use bevy_enhanced_input::prelude::EnhancedInputSystems;

use crate::ActiveKeymapContext;
use crate::ActiveKeymapContextTransition;
use crate::Capability;
use crate::CommandId;
use crate::CommandLookup;
use crate::CommandRegistry;
use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;
use crate::KeymapLoadFailures;
use crate::KeymapStateDimension;
use crate::Keystroke;
use crate::condition;
use crate::condition::StateDimensionRegistration;
use crate::condition::StateDimensionRegistry;
use crate::disk;
use crate::disk::DiskWorkerChannels;
use crate::disk::KeymapConfigurationDirectory;
use crate::disk::KeymapPathAvailability;
use crate::disk::KeymapPathFailure;
use crate::keymap;
use crate::keymap::KeymapBindingUnavailability;
use crate::keymap::KeymapRuntime;
use crate::keymap::KeystrokeRouting;
use crate::keymap::PendingReload;
use crate::keymap::ProtectedCommandBinding;
use crate::keymap::ReloadRequest;

/// Configures the application's keymap document and state dimensions.
///
/// Add this plugin directly for global-only bindings. Register every application-owned total
/// state through repeated [`Self::with_state_dimension`] calls; Rubric observes those states but
/// never initializes or changes them.
pub struct KeymapPlugin {
    configuration_directory:    KeymapConfigurationDirectory,
    defaults:                   DefaultKeymapSource,
    protected_keystrokes:       Vec<Keystroke>,
    protected_command_bindings: Vec<ProtectedCommandBinding>,
    state_dimensions:           Vec<StateDimensionRegistration>,
}

impl KeymapPlugin {
    /// Starts a keymap plugin configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            configuration_directory:    KeymapConfigurationDirectory::Unconfigured,
            defaults:                   DefaultKeymapSource::NotSupplied,
            protected_keystrokes:       Vec::new(),
            protected_command_bindings: Vec::new(),
            state_dimensions:           Vec::new(),
        }
    }

    /// Sets the application name used to resolve the keymap configuration directory.
    #[must_use]
    pub fn with_app_name(mut self, app_name: &str) -> Self {
        self.configuration_directory =
            KeymapConfigurationDirectory::ForApplication(app_name.to_owned());
        self
    }

    /// Sets the application-embedded default JSONC keymap.
    #[must_use]
    pub const fn with_defaults(mut self, defaults: &'static str) -> Self {
        self.defaults = DefaultKeymapSource::Embedded(defaults);
        self
    }

    /// Reserves a recovery keystroke from user-authored bindings.
    #[must_use]
    pub fn with_protected_keystroke(mut self, keystroke: Keystroke) -> Self {
        self.protected_keystrokes.push(keystroke);
        self
    }

    /// Reserves `keystroke` for the application-owned recovery `command_id`.
    ///
    /// The association remains visible to palette consumers even when no authored keymap can
    /// materialize. It is excluded from Rubric routing: the application detects and
    /// invokes its recovery chord directly. Assembly validates that every associated command is a
    /// registered, palette-invocable [`Capability::Unremappable`] command; a failed association
    /// set records diagnostics and installs none of its associations.
    #[must_use]
    pub fn with_protected_command_binding(
        mut self,
        command_id: CommandId,
        keystroke: Keystroke,
    ) -> Self {
        self.protected_command_bindings
            .push(ProtectedCommandBinding::new(command_id, keystroke));
        self
    }

    /// Registers one total application-owned state dimension for simultaneous observation.
    ///
    /// Call this once for every typed state machine that authored keymap context may reference.
    /// The application must install and own `State<C>`; Rubric neither initializes it nor writes
    /// its `NextState<C>`. Dimension-name and state-type duplicates become retained assembly
    /// diagnostics that identify both registrations.
    #[must_use]
    pub fn with_state_dimension<C>(mut self, name: &str) -> Self
    where
        C: KeymapStateDimension + States,
    {
        self.state_dimensions
            .push(StateDimensionRegistration::new::<C>(name));
        self
    }

    fn install(&self, app: &mut App) {
        Self::install_runtime(app);
        if !self.has_configuration() {
            return;
        }
        if let Some(installed) = app.world().get_resource::<KeymapPluginConfiguration>() {
            assert!(
                *installed == KeymapPluginConfiguration::from(self),
                "hana_rubric: `KeymapPlugin` is already installed with different defaults, an \
                 application name, protected keystrokes, or protected command bindings. Keeping \
                 the first configuration would \
                 run bindings the second caller never asked for, so configure one plugin and \
                 install it once."
            );
            return;
        }

        app.insert_resource(KeymapPluginConfiguration::from(self));
    }

    pub(crate) fn install_runtime(app: &mut App) {
        if app.world().contains_resource::<KeymapRuntimeInstalled>() {
            return;
        }

        app.init_resource::<KeymapRuntimeInstalled>()
            .init_resource::<StateDimensionRegistry>()
            .init_resource::<ActiveKeymapContext>()
            .add_message::<ActiveKeymapContextTransition>()
            .init_resource::<KeymapLoadFailures>()
            .init_resource::<KeystrokeRouting>()
            .init_resource::<keymap::KeymapBindings>()
            .init_resource::<keymap::EffectiveKeymapStatus>()
            .init_resource::<PendingReload>()
            .init_resource::<KeymapRuntime>()
            .configure_sets(
                PreUpdate,
                (
                    KeymapSystems::ObserveStateDimensions,
                    KeymapSystems::UpdateActiveKeymapContext,
                    KeymapSystems::Route,
                )
                    .chain(),
            )
            .add_systems(
                PreUpdate,
                (
                    collect_disk_reload,
                    keymap::commit_reload,
                    keymap::commit_effective_keymap,
                )
                    .chain()
                    .after(KeymapSystems::UpdateActiveKeymapContext)
                    .before(KeymapSystems::Route),
            )
            .add_systems(
                PreUpdate,
                keymap::route_input
                    .in_set(KeymapSystems::Route)
                    .after(InputSystems)
                    .before(EnhancedInputSystems::Update),
            );
    }

    fn finish_assembly(app: &mut App) {
        #[cfg(test)]
        increment_finish_attempts(app);

        if app.world().contains_resource::<KeymapAssemblyFinished>() {
            return;
        }
        app.insert_resource(KeymapAssemblyFinished);
        let command_registry = match CommandRegistry::initialize(app.world_mut()) {
            Ok(command_registry) => command_registry,
            Err(diagnostics) => {
                app.insert_resource(RegistryValidationFailed);
                record_startup_diagnostics(app, &diagnostics);
                CommandRegistry::empty()
            },
        };
        app.world_mut().insert_resource(command_registry);

        let Some(keymap_plugin_configuration) = app
            .world()
            .get_resource::<KeymapPluginConfiguration>()
            .cloned()
        else {
            keymap::KeymapBindings::replace_unavailability(
                app.world_mut(),
                KeymapBindingUnavailability::Unconfigured,
            );
            insert_keymap_path_availability(app, &KeymapConfigurationDirectory::Unconfigured);
            record_unconfigured_keymap_plugin(app);
            return;
        };

        finish_configured_keymap(app, keymap_plugin_configuration);
    }

    const fn has_configuration(&self) -> bool {
        matches!(
            self.configuration_directory,
            KeymapConfigurationDirectory::ForApplication(_)
        ) || matches!(self.defaults, DefaultKeymapSource::Embedded(_))
            || !self.protected_keystrokes.is_empty()
            || !self.protected_command_bindings.is_empty()
    }
}

fn finish_configured_keymap(app: &mut App, keymap_plugin_configuration: KeymapPluginConfiguration) {
    let protected_command_bindings = match validate_protected_command_bindings(
        app.world().resource::<CommandRegistry>(),
        &keymap_plugin_configuration.protected_command_bindings,
    ) {
        Ok(protected_command_bindings) => protected_command_bindings,
        Err(diagnostics) => {
            record_startup_diagnostics(app, &diagnostics);
            Vec::new()
        },
    };
    let mut protected_keystrokes = keymap_plugin_configuration.protected_keystrokes.clone();
    protected_keystrokes.extend(
        protected_command_bindings
            .iter()
            .map(|association| *association.keystroke()),
    );
    keymap::KeymapBindings::replace_protected_command_bindings(
        app.world_mut(),
        protected_command_bindings,
    );
    let keymap_path_availability =
        insert_keymap_path_availability(app, &keymap_plugin_configuration.configuration_directory);
    let DefaultKeymapSource::Embedded(defaults) = keymap_plugin_configuration.defaults else {
        keymap::KeymapBindings::replace_unavailability(
            app.world_mut(),
            KeymapBindingUnavailability::MissingDefault,
        );
        record_missing_default_keymap(app);
        return;
    };
    #[cfg(test)]
    increment_assembly_runs(app);
    let published_defaults = {
        let world = app.world();
        let reference = keymap::state_dimension_reference_default_bytes(
            defaults,
            world.resource::<StateDimensionRegistry>(),
            &protected_keystrokes,
        );
        String::from_utf8_lossy(&reference).into_owned()
    };
    let allow_default_failures = app.world().contains_resource::<RegistryValidationFailed>();
    let reload_configuration = keymap::ReloadConfiguration::new(
        DiagnosticOrigin::EmbeddedDefaults,
        published_defaults.clone(),
        protected_keystrokes,
        allow_default_failures,
    );
    app.world_mut().insert_resource(reload_configuration);

    if !keymap::commit_defaults(app.world_mut()) {
        return;
    }

    start_keymap_disk_worker(app, keymap_path_availability, published_defaults);
}

fn start_keymap_disk_worker(
    app: &mut App,
    keymap_path_availability: KeymapPathAvailability,
    published_defaults: String,
) {
    let paths = match keymap_path_availability.resolved() {
        Ok(paths) => paths,
        Err(keymap_path_failure) => {
            record_unavailable_keymap_paths(app, keymap_path_failure);
            return;
        },
    };
    let schema_result = {
        let world = app.world();
        keymap::state_dimension_schema_bytes(
            world.resource::<CommandRegistry>(),
            world.resource::<StateDimensionRegistry>(),
        )
    };
    let schema = match schema_result {
        Ok(schema) => Some(schema),
        Err(error) => {
            record_startup_diagnostics(
                app,
                &[startup_diagnostic(
                    DiagnosticOrigin::KeymapFile(paths.schema().to_path_buf()),
                    DiagnosticKind::Companion,
                    &format!("Could not generate the keymap schema: {error}"),
                    DiagnosticSeverity::Failure,
                )],
            );
            None
        },
    };
    app.world_mut().insert_resource(KeymapDiskWorker {
        disk_worker_channels: disk::start_disk_worker(
            paths,
            published_defaults.into_bytes(),
            schema,
        ),
    });
}

impl Default for KeymapPlugin {
    fn default() -> Self { Self::new() }
}

impl Plugin for KeymapPlugin {
    fn build(&self, app: &mut App) {
        self.install(app);
        if !self.state_dimensions.is_empty() {
            condition::register_state_dimensions(app, &self.state_dimensions);
        }
    }

    fn finish(&self, app: &mut App) { Self::finish_assembly(app); }

    fn is_unique(&self) -> bool { false }
}

/// Where the keymap plugin takes an application's shipped default bindings from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefaultKeymapSource {
    /// The application compiled a JSONC keymap into its binary.
    Embedded(&'static str),
    /// The application supplied no defaults, so no bindings can be compiled.
    NotSupplied,
}

#[derive(Clone, Eq, PartialEq, Resource)]
struct KeymapPluginConfiguration {
    configuration_directory:    KeymapConfigurationDirectory,
    defaults:                   DefaultKeymapSource,
    protected_keystrokes:       Vec<Keystroke>,
    protected_command_bindings: Vec<ProtectedCommandBinding>,
}

impl From<&KeymapPlugin> for KeymapPluginConfiguration {
    fn from(keymap_plugin: &KeymapPlugin) -> Self {
        Self {
            configuration_directory:    keymap_plugin.configuration_directory.clone(),
            defaults:                   keymap_plugin.defaults,
            protected_keystrokes:       keymap_plugin.protected_keystrokes.clone(),
            protected_command_bindings: keymap_plugin.protected_command_bindings.clone(),
        }
    }
}

fn validate_protected_command_bindings(
    command_registry: &CommandRegistry,
    candidates: &[ProtectedCommandBinding],
) -> Result<Vec<ProtectedCommandBinding>, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut command_ids = Vec::new();
    let mut keystrokes = Vec::new();
    for candidate in candidates {
        if command_ids.contains(candidate.command_id()) {
            diagnostics.push(protected_command_binding_diagnostic(
                candidate,
                format!(
                    "Command `{}` has more than one protected recovery association.",
                    candidate.command_id()
                ),
            ));
        }
        command_ids.push(candidate.command_id().clone());

        if keystrokes.contains(candidate.keystroke()) {
            diagnostics.push(protected_command_binding_diagnostic(
                candidate,
                format!(
                    "Keystroke `{}` has more than one protected recovery association.",
                    candidate.keystroke()
                ),
            ));
        }
        keystrokes.push(*candidate.keystroke());

        match command_registry.lookup(candidate.command_id()) {
            CommandLookup::UnknownCommand => {
                diagnostics.push(protected_command_binding_diagnostic(
                    candidate,
                    format!(
                        "Protected recovery command `{}` is not registered.",
                        candidate.command_id()
                    ),
                ));
            },
            CommandLookup::Found(command) if command.capability != Capability::Unremappable => {
                diagnostics.push(protected_command_binding_diagnostic(
                    candidate,
                    format!(
                        "Protected recovery command `{}` must declare the palette-invocable \
                         `Unremappable` capability.",
                        candidate.command_id()
                    ),
                ));
            },
            CommandLookup::Found(_) => {},
        }
    }
    if diagnostics.is_empty() {
        Ok(candidates.to_vec())
    } else {
        Err(diagnostics)
    }
}

fn protected_command_binding_diagnostic(
    association: &ProtectedCommandBinding,
    message: String,
) -> Diagnostic {
    Diagnostic {
        origin: DiagnosticOrigin::CommandRegistration,
        byte_range: 0..0,
        line: 0,
        column: 0,
        block_index: 0,
        context: String::new(),
        original_keystroke: association.keystroke().to_string(),
        command_id: association.command_id().to_string(),
        kind: DiagnosticKind::ProtectedCommandBinding,
        severity: DiagnosticSeverity::Failure,
        message,
        suggestions: Vec::new(),
    }
}

#[derive(Resource)]
struct KeymapDiskWorker {
    disk_worker_channels: DiskWorkerChannels,
}

#[derive(Resource)]
struct KeymapAssemblyFinished;

#[derive(Resource)]
pub(crate) struct RegistryValidationFailed;

#[derive(Default, Resource)]
struct KeymapRuntimeInstalled;

fn collect_disk_reload(
    keymap_disk_worker: Option<ResMut<KeymapDiskWorker>>,
    mut pending_reload: ResMut<PendingReload>,
) {
    let Some(keymap_disk_worker) = keymap_disk_worker else {
        return;
    };
    let Some(disk_worker_message) =
        disk::take_worker_message(&keymap_disk_worker.disk_worker_channels)
    else {
        return;
    };

    pending_reload.replace(ReloadRequest::from(disk_worker_message));
}

fn record_startup_diagnostics(app: &mut App, diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        match diagnostic.severity {
            DiagnosticSeverity::Failure => bevy::log::error!("{}", diagnostic.message),
            DiagnosticSeverity::Advisory => bevy::log::warn!("{}", diagnostic.message),
        }
    }
    app.world_mut()
        .resource_mut::<KeymapLoadFailures>()
        .retained_diagnostics
        .extend(diagnostics.iter().cloned());
}

/// Inserts [`KeymapPathAvailability`] and hands the same value back to the assembly that needs it.
///
/// Every `finish_assembly` outcome inserts it, including the ones that stop before any binding
/// compiles, so `Res<KeymapPathAvailability>` never distinguishes an application without a keymap
/// directory from an assembly that has not run.
fn insert_keymap_path_availability(
    app: &mut App,
    keymap_configuration_directory: &KeymapConfigurationDirectory,
) -> KeymapPathAvailability {
    let keymap_path_availability = KeymapPathAvailability::from(keymap_configuration_directory);
    app.world_mut()
        .insert_resource(keymap_path_availability.clone());

    keymap_path_availability
}

fn record_unconfigured_keymap_plugin(app: &mut App) {
    record_startup_diagnostics(
        app,
        &[startup_diagnostic(
            DiagnosticOrigin::EmbeddedDefaults,
            DiagnosticKind::UnconfiguredKeymapPlugin,
            "Could not compile any bindings because the keymap plugin was added without an \
             application name, an embedded default keymap, or a protected keystroke.",
            DiagnosticSeverity::Failure,
        )],
    );
}

fn record_missing_default_keymap(app: &mut App) {
    record_startup_diagnostics(
        app,
        &[startup_diagnostic(
            DiagnosticOrigin::EmbeddedDefaults,
            DiagnosticKind::MissingDefaultKeymap,
            "Could not compile any bindings because the keymap plugin was configured without an \
             embedded default keymap.",
            DiagnosticSeverity::Failure,
        )],
    );
}

/// Reports the configuration directory the keymap could not resolve.
///
/// An application that never called [`KeymapPlugin::with_app_name`] has no directory
/// to resolve, so that case is advisory. An application that named one and still has
/// no directory cannot read or write the user's keymap, which is a failure.
fn record_unavailable_keymap_paths(app: &mut App, keymap_path_failure: KeymapPathFailure) {
    let severity = match keymap_path_failure {
        KeymapPathFailure::AppNameNotConfigured => DiagnosticSeverity::Advisory,
        KeymapPathFailure::AppNameNotOnePathComponent
        | KeymapPathFailure::NoPlatformConfigurationDirectory => DiagnosticSeverity::Failure,
    };
    record_startup_diagnostics(
        app,
        &[startup_diagnostic(
            DiagnosticOrigin::PathsUnavailable(keymap_path_failure),
            DiagnosticKind::Disk,
            keymap_path_failure.reason(),
            severity,
        )],
    );
}

fn startup_diagnostic(
    origin: DiagnosticOrigin,
    kind: DiagnosticKind,
    message: &str,
    severity: DiagnosticSeverity,
) -> Diagnostic {
    Diagnostic {
        origin,
        byte_range: 0..0,
        line: 0,
        column: 0,
        block_index: 0,
        context: String::new(),
        original_keystroke: String::new(),
        command_id: String::new(),
        kind,
        severity,
        message: message.to_owned(),
        suggestions: Vec::new(),
    }
}

#[cfg(test)]
#[derive(Default, Resource)]
struct FinishAttempts(usize);

#[cfg(test)]
#[derive(Default, Resource)]
struct AssemblyRuns(usize);

#[cfg(test)]
fn increment_finish_attempts(app: &mut App) {
    app.init_resource::<FinishAttempts>();
    app.world_mut().resource_mut::<FinishAttempts>().0 += 1;
}

#[cfg(test)]
fn increment_assembly_runs(app: &mut App) {
    app.init_resource::<AssemblyRuns>();
    app.world_mut().resource_mut::<AssemblyRuns>().0 += 1;
}

/// Keymap-system ordering points exposed to application context derivation systems.
#[derive(Clone, Debug, Eq, Hash, PartialEq, SystemSet)]
pub enum KeymapSystems {
    /// Observes every application-owned typed state before collecting their complete snapshot.
    ObserveStateDimensions,
    /// Collects staged state-dimension observations into [`crate::ActiveKeymapContext`].
    UpdateActiveKeymapContext,
    /// Routes input through the compiled keymap.
    Route,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests stop when isolated plugin setup cannot continue"
)]
#[allow(
    dead_code,
    reason = "test command declarations are registered through reflection"
)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::Instant;

    use bevy::ecs::change_detection::DetectChanges;
    use bevy::input::ButtonInput;
    use bevy::input::keyboard::KeyCode;
    use bevy::prelude::App;
    use bevy::prelude::AppTypeRegistry;
    use bevy::prelude::Event;
    use bevy::prelude::NextState;
    use bevy::prelude::On;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::States;
    use bevy::state::app::AppExtStates;
    use bevy::state::app::StatesPlugin;
    use bevy_enhanced_input::prelude::ActionValue;
    use bevy_enhanced_input::prelude::CustomInput;
    use bevy_enhanced_input::prelude::CustomInputs;
    use bevy_enhanced_input::prelude::InputAction;
    use strum::AsRefStr;
    use strum::EnumIter;
    use strum::EnumMessage;

    use super::KeymapDiskWorker;
    use super::KeymapPlugin;
    use crate::ActiveKeymapContext;
    use crate::ActiveKeymapContextState;
    use crate::AuthoredKeymapBindings;
    use crate::CommandId;
    use crate::CommandRegistry;
    use crate::DiagnosticKind;
    use crate::DiagnosticOrigin;
    use crate::DiagnosticSeverity;
    use crate::EffectiveKeymapStatus;
    use crate::HeldCommandLookupOutcome;
    use crate::KeymapBindings;
    use crate::KeymapCommand;
    use crate::KeymapGeneration;
    use crate::KeymapLoadFailures;
    use crate::Keystroke;
    use crate::KeystrokeSequence;
    use crate::MatchOutcome;
    use crate::PaletteBinding;
    use crate::ReflectKeymapCommand;
    use crate::disk::ENVIRONMENT_LOCK;
    use crate::disk::KeymapPathAvailability;
    use crate::disk::TestDirectory;
    use crate::disk::XdgConfigHome;
    use crate::keymap;
    use crate::keymap::AcceptedKeymapDocument;
    use crate::keymap::CompiledKeymap;
    use crate::keymap::PendingReload;
    use crate::keymap::ReloadConfiguration;
    use crate::keymap::ReloadRequest;
    use crate::keymap::RoutingResetStep;
    use crate::keymap::RoutingResetTrace;
    use crate::keymap::UserKeymapContents;
    use crate::query_command_palette;

    const DEFAULTS: &str = r#"{ "bindings": [] }"#;
    const TEST_APP_NAME: &str = "hana-rubric-plugin-test";
    const UNCREATABLE_KEYMAP_DIRECTORY: &str = "keymap directory";
    const USER_KEYMAP_FIXTURE: &str = "user-keymap.jsonc";

    crate::command! {
        action:      PluginDispatchAction,
        event:       PluginDispatch,
        id:          "plugin::dispatch",
        title:       "Plugin Dispatch",
        description: "Dispatches through the plugin integration tests.",
    }

    crate::command! {
        held,
        action:      StateDimensionHeldAction,
        event:       StateDimensionHeld,
        id:          "plugin::state_dimension_held",
        title:       "State-Dimension Held",
        description: "Exercises held-prefix validation after state materialization.",
    }

    crate::command! {
        action:      PluginRecoveryAction,
        event:       PluginRecovery,
        id:          "plugin::recovery",
        title:       "Plugin Recovery",
        description: "Application-owned recovery command for association tests.",
        capability:  Unremappable,
    }

    crate::command! {
        action:      PluginAlternateRecoveryAction,
        event:       PluginAlternateRecovery,
        id:          "plugin::alternate_recovery",
        title:       "Plugin Alternate Recovery",
        description: "Second application-owned recovery command for duplicate tests.",
        capability:  Unremappable,
    }

    #[derive(Default, Resource)]
    struct DispatchCount(usize);

    #[derive(
        AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
    )]
    #[strum(serialize_all = "snake_case")]
    enum ApplicationState {
        #[default]
        #[strum(message = "While the application is ready for commands")]
        Ready,
        #[strum(message = "While the application is starting")]
        Starting,
    }

    #[derive(
        AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
    )]
    #[strum(serialize_all = "snake_case")]
    enum InteractionState {
        #[default]
        #[strum(message = "While the interaction is resting")]
        Resting,
        #[strum(message = "While the interaction is editing")]
        Editing,
    }
    fn register_plugin_command(app: &mut App) {
        app.world_mut().insert_resource(AppTypeRegistry::default());
        let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
        app_type_registry.write().register::<PluginDispatch>();
    }

    fn register_state_dimension_commands(app: &mut App) {
        register_plugin_command(app);
        let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
        app_type_registry.write().register::<StateDimensionHeld>();
    }

    fn register_protected_command_association_commands(app: &mut App) {
        register_plugin_command(app);
        let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
        let mut type_registry = app_type_registry.write();
        type_registry.register::<PluginRecovery>();
        type_registry.register::<PluginAlternateRecovery>();
    }

    fn state_dimension_keymap_app(defaults: &'static str) -> App {
        let mut app = App::new();
        register_state_dimension_commands(&mut app);
        app.add_plugins(StatesPlugin)
            .insert_state(ApplicationState::Ready)
            .insert_state(InteractionState::Resting)
            .add_plugins(
                KeymapPlugin::new()
                    .with_defaults(defaults)
                    .with_state_dimension::<ApplicationState>("application")
                    .with_state_dimension::<InteractionState>("interaction"),
            );
        app.finish();
        app
    }

    fn command_id(command_id: &str) -> CommandId {
        CommandId::try_from(command_id).expect("fixture command ID remains valid")
    }

    fn source_line_and_column(source: &str, byte_offset: usize) -> (usize, usize) {
        let prefix = &source[..byte_offset];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let line_start = prefix.rfind('\n').map_or(0, |offset| offset + 1);
        let column = source[line_start..byte_offset].chars().count() + 1;
        (line, column)
    }

    fn assert_isolated_paths(temporary_directory: &TestDirectory) {
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test keymap paths resolve");

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
    }

    fn assert_compiled_global_binding(app: &mut App) {
        let sequence = "g"
            .parse::<KeystrokeSequence>()
            .expect("plugin global test sequence is valid");
        let match_outcome = app
            .world_mut()
            .resource_mut::<CompiledKeymap>()
            .match_effective(sequence.first(), Instant::now(), Duration::from_secs(1));

        assert!(matches!(match_outcome, MatchOutcome::Matched(_)));
    }
    #[test]
    fn chained_protected_keystrokes_reject_each_user_binding() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("protected-keys").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let first = "ctrl-g"
            .parse()
            .map_err(|error| format!("invalid first protected keystroke: {error}"))?;
        let second = "ctrl-h"
            .parse()
            .map_err(|error| format!("invalid second protected keystroke: {error}"))?;
        let mut app = App::new();
        register_plugin_command(&mut app);
        app.add_plugins(
            KeymapPlugin::new()
                .with_app_name(TEST_APP_NAME)
                .with_defaults(DEFAULTS)
                .with_protected_keystroke(first)
                .with_protected_keystroke(second),
        );
        assert_isolated_paths(&temporary_directory);
        app.finish();
        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(
                    *br#"{
                        "bindings": [{
                            "bindings": {
                                "ctrl-g": "plugin::dispatch",
                                "ctrl-h": "plugin::dispatch"
                            }
                        }]
                    }"#,
                )),
                diagnostics: Vec::new(),
            });
        keymap::commit_reload(app.world_mut());

        let reserved = app
            .world()
            .resource::<KeymapLoadFailures>()
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.kind == DiagnosticKind::ReservedKeystroke)
            .count();
        assert_eq!(reserved, 2);

        drop(app);
        drop(xdg_config_home);
        drop(temporary_directory);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn protected_command_association_displays_without_entering_authored_bindings()
    -> Result<(), String> {
        let mut app = App::new();
        register_protected_command_association_commands(&mut app);
        let recovery_id = CommandId::declared::<PluginRecovery>();
        let recovery_keystroke: Keystroke = "ctrl-p"
            .parse()
            .map_err(|error| format!("recovery keystroke remains valid: {error}"))?;
        app.add_plugins(
            KeymapPlugin::new()
                .with_defaults(DEFAULTS)
                .with_protected_command_binding(recovery_id.clone(), recovery_keystroke),
        );
        app.finish();

        let query_result = {
            let world = app.world();
            query_command_palette(
                world.resource::<CommandRegistry>(),
                world.resource::<ActiveKeymapContext>(),
                world.resource::<EffectiveKeymapStatus>(),
                world.resource::<KeymapBindings>(),
                PluginRecovery::TITLE,
            )
        };
        assert_eq!(query_result.rows().len(), 1);
        assert!(matches!(
            query_result.rows()[0].binding(),
            PaletteBinding::ApplicationRecovery(keystroke) if keystroke == &recovery_keystroke
        ));
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&recovery_id),
            crate::CommandKeystroke::Unbound
        );
        Ok(())
    }

    #[test]
    fn invalid_protected_command_associations_are_diagnosed_in_order_without_partial_install()
    -> Result<(), String> {
        let mut app = App::new();
        register_protected_command_association_commands(&mut app);
        let invalid_command = CommandId::try_from("plugin::missing")
            .map_err(|error| format!("invalid command fixture: {error}"))?;
        let protected = |command_id: CommandId, keystroke: &str| {
            let keystroke: Keystroke = keystroke
                .parse()
                .map_err(|error| format!("protected keystroke fixture: {error}"))?;
            Ok::<_, String>((command_id, keystroke))
        };
        let (non_recovery_id, non_recovery_keystroke) =
            protected(CommandId::declared::<PluginDispatch>(), "ctrl-p")?;
        let (missing_id, missing_keystroke) = protected(invalid_command, "ctrl-o")?;
        let (recovery_id, recovery_keystroke) =
            protected(CommandId::declared::<PluginRecovery>(), "ctrl-a")?;
        let (duplicate_recovery_id, duplicate_recovery_keystroke) =
            protected(CommandId::declared::<PluginRecovery>(), "ctrl-b")?;
        let (alternate_recovery_id, duplicate_keystroke) =
            protected(CommandId::declared::<PluginAlternateRecovery>(), "ctrl-a")?;
        app.add_plugins(
            KeymapPlugin::new()
                .with_defaults(DEFAULTS)
                .with_protected_command_binding(non_recovery_id, non_recovery_keystroke)
                .with_protected_command_binding(missing_id, missing_keystroke)
                .with_protected_command_binding(recovery_id.clone(), recovery_keystroke)
                .with_protected_command_binding(duplicate_recovery_id, duplicate_recovery_keystroke)
                .with_protected_command_binding(alternate_recovery_id, duplicate_keystroke),
        );
        app.finish();

        let diagnostics = app
            .world()
            .resource::<KeymapLoadFailures>()
            .retained_diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.kind == DiagnosticKind::ProtectedCommandBinding)
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 4);
        assert_eq!(diagnostics[0].command_id, PluginDispatch::ID);
        assert_eq!(diagnostics[1].command_id, "plugin::missing");
        assert_eq!(diagnostics[2].command_id, PluginRecovery::ID);
        assert_eq!(diagnostics[3].command_id, PluginAlternateRecovery::ID);
        assert!(
            diagnostics[2]
                .message
                .contains("more than one protected recovery association")
        );
        assert!(
            diagnostics[3]
                .message
                .contains("more than one protected recovery association")
        );

        let query_result = {
            let world = app.world();
            query_command_palette(
                world.resource::<CommandRegistry>(),
                world.resource::<ActiveKeymapContext>(),
                world.resource::<EffectiveKeymapStatus>(),
                world.resource::<KeymapBindings>(),
                PluginRecovery::TITLE,
            )
        };
        assert_eq!(query_result.rows()[0].binding(), PaletteBinding::Unbound);
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&recovery_id),
            crate::CommandKeystroke::Unbound
        );
        Ok(())
    }

    #[test]
    fn defaults_without_an_application_name_commit_and_route() {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("defaults-without-app-name").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let mut app = App::new();
        register_plugin_command(&mut app);
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<DispatchCount>()
            .add_plugins(
                KeymapPlugin::new().with_defaults(
                    r#"{ "bindings": [{ "bindings": { "g": "plugin::dispatch" } }] }"#,
                ),
            );
        app.world_mut().add_observer(
            |_: On<PluginDispatch>, mut dispatch_count: ResMut<DispatchCount>| {
                dispatch_count.0 += 1;
            },
        );
        app.finish();

        assert!(app.world().contains_resource::<CompiledKeymap>());
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings)
                if bindings.generation() == app.world().resource::<CompiledKeymap>().generation()
        ));
        assert_compiled_global_binding(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        app.update();
        assert_eq!(app.world().resource::<DispatchCount>().0, 1);

        drop(app);
        drop(xdg_config_home);
        drop(temporary_directory);
        drop(environment_lock);
    }

    #[test]
    fn state_dimension_predicates_materialize_in_document_order() -> Result<(), String> {
        let mut app = App::new();
        register_plugin_command(&mut app);
        app.add_plugins(StatesPlugin)
            .insert_state(ApplicationState::Ready)
            .insert_state(InteractionState::Resting)
            .add_plugins(
                KeymapPlugin::new()
                    .with_defaults(
                        r#"{
                            "bindings": [
                                { "bindings": { "p": "plugin::dispatch", "t": "plugin::dispatch" } },
                                { "context": { "application": "ready" }, "bindings": { "p": null } },
                                { "context": { "interaction": "resting" }, "bindings": { "p": "plugin::dispatch" } },
                                { "context": { "application": "ready", "interaction": "resting" }, "bindings": { "p": null, "t": null } },
                                { "context": { "application": "ready" }, "bindings": { "p": "plugin::dispatch" } }
                            ]
                        }"#,
                    )
                    .with_state_dimension::<ApplicationState>("application")
                    .with_state_dimension::<InteractionState>("interaction"),
            );
        app.finish();

        assert!(matches!(
            app.world().resource::<EffectiveKeymapStatus>(),
            EffectiveKeymapStatus::AwaitingStateDimensions
        ));

        app.update();
        let command_id = CommandId::try_from(PluginDispatch::ID)
            .expect("plugin command ID remains valid in this fixture");
        let loaded = app.world().resource::<KeymapBindings>();
        assert!(matches!(
            loaded.keystroke(&command_id),
            crate::CommandKeystroke::BoundTo(_)
        ));
        assert!(matches!(
            app.world().resource::<EffectiveKeymapStatus>(),
            EffectiveKeymapStatus::Loaded(publication)
                if publication.generation == app.world().resource::<CompiledKeymap>().generation()
                    && publication.matched_layers.iter().map(|layer| layer.document_order).collect::<Vec<_>>()
                        == vec![0, 1, 2, 3]
        ));
        let expected_p: KeystrokeSequence = "p".parse().expect("fixture sequence parses");
        assert_eq!(
            loaded.keystroke(&command_id),
            crate::CommandKeystroke::BoundTo(&expected_p)
        );

        app.world_mut()
            .resource_mut::<NextState<InteractionState>>()
            .set(InteractionState::Editing);
        app.update();
        app.update();
        let loaded = app.world().resource::<KeymapBindings>();
        assert_eq!(
            loaded.keystroke(&command_id),
            crate::CommandKeystroke::BoundTo(&expected_p)
        );
        let active_snapshot = match app.world().resource::<crate::ActiveKeymapContext>().state() {
            ActiveKeymapContextState::Resolved(snapshot) => snapshot.clone(),
            state => {
                return Err(format!(
                    "state dimensions must resolve after the update, found {state:?}"
                ));
            },
        };
        assert_publication_generation(
            app.world().resource::<EffectiveKeymapStatus>(),
            app.world().resource::<CompiledKeymap>().generation(),
        )?;
        assert_eq!(
            app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::Loaded(crate::EffectiveKeymapPublication {
                generation:     app.world().resource::<CompiledKeymap>().generation(),
                snapshot:       crate::EffectiveKeymapSnapshot::Resolved(active_snapshot),
                matched_layers: vec![
                    crate::MatchedPredicateLayer {
                        document_order: 0,
                        predicate:      crate::StateDimensionPredicateIdentity {
                            terms: vec![(
                                crate::ContextDimensionName::new("application"),
                                crate::ContextValueName::new("ready"),
                            )],
                        },
                    },
                    crate::MatchedPredicateLayer {
                        document_order: 3,
                        predicate:      crate::StateDimensionPredicateIdentity {
                            terms: vec![(
                                crate::ContextDimensionName::new("application"),
                                crate::ContextValueName::new("ready"),
                            )],
                        },
                    },
                ],
            })
        );
        assert_eq!(
            loaded.keystroke(&command_id),
            crate::CommandKeystroke::BoundTo(&expected_p)
        );
        Ok(())
    }

    fn assert_publication_generation(
        effective_status: &EffectiveKeymapStatus,
        compiled_generation: KeymapGeneration,
    ) -> Result<(), String> {
        let EffectiveKeymapStatus::Loaded(publication) = effective_status else {
            return Err("resolved snapshot must retain an effective publication".to_owned());
        };
        let publication_generation: crate::prelude::KeymapGeneration = publication.generation;
        let matched_layer_document_order: usize = publication.matched_layers[1].document_order;
        assert_eq!(publication_generation, compiled_generation);
        assert_eq!(publication_generation.to_string(), "1");
        assert_eq!(matched_layer_document_order, 3);
        Ok(())
    }

    #[test]
    fn effective_statuses_report_global_awaiting_sorted_missing_and_resolved_states() {
        let mut global_app = App::new();
        register_plugin_command(&mut global_app);
        global_app.add_plugins(
            KeymapPlugin::new()
                .with_defaults(r#"{ "bindings": [{ "bindings": { "p": "plugin::dispatch" } }] }"#),
        );
        global_app.finish();
        assert_eq!(
            global_app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::Loaded(crate::EffectiveKeymapPublication {
                generation:     KeymapGeneration::initial(),
                snapshot:       crate::EffectiveKeymapSnapshot::Global,
                matched_layers: Vec::new(),
            })
        );

        let mut missing_app = App::new();
        register_state_dimension_commands(&mut missing_app);
        missing_app.add_plugins(StatesPlugin).add_plugins(
            KeymapPlugin::new()
                .with_defaults(r#"{ "bindings": [{ "bindings": { "p": "plugin::dispatch" } }] }"#)
                .with_state_dimension::<InteractionState>("interaction")
                .with_state_dimension::<ApplicationState>("application"),
        );
        missing_app.finish();
        assert_eq!(
            missing_app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::AwaitingStateDimensions
        );
        missing_app.update();
        assert_eq!(
            missing_app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::StateDimensionsUnavailable {
                missing: vec![
                    crate::ContextDimensionName::new("application"),
                    crate::ContextDimensionName::new("interaction"),
                ],
            }
        );

        let mut resolved_app = state_dimension_keymap_app(
            r#"{ "bindings": [{ "bindings": { "p": "plugin::dispatch" } }] }"#,
        );
        resolved_app.update();
        let effective_status = resolved_app.world().resource::<EffectiveKeymapStatus>();
        assert!(
            matches!(effective_status, EffectiveKeymapStatus::Loaded(_)),
            "resolved state dimensions must materialize an effective keymap"
        );
        let EffectiveKeymapStatus::Loaded(publication) = effective_status else {
            return;
        };
        assert_eq!(
            publication.generation,
            resolved_app
                .world()
                .resource::<CompiledKeymap>()
                .generation()
        );
        assert!(matches!(
            publication.snapshot,
            crate::EffectiveKeymapSnapshot::Resolved(_)
        ));
    }

    #[test]
    fn accepted_awaiting_document_starts_its_watcher_and_materializes_after_state_resolution() {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("awaiting-state-handoff").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let mut app = App::new();
        register_state_dimension_commands(&mut app);
        app.add_plugins(StatesPlugin).add_plugins(
            KeymapPlugin::new()
                .with_app_name(TEST_APP_NAME)
                .with_defaults(r#"{ "bindings": [{ "bindings": { "p": "plugin::dispatch" } }] }"#)
                .with_state_dimension::<ApplicationState>("application")
                .with_state_dimension::<InteractionState>("interaction"),
        );
        app.finish();

        assert!(
            app.world()
                .contains_resource::<crate::keymap::AcceptedKeymapDocument>()
        );
        assert!(app.world().contains_resource::<KeymapDiskWorker>());
        assert!(!app.world().contains_resource::<CompiledKeymap>());
        assert_eq!(
            app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::AwaitingStateDimensions
        );
        let accepted_document_tick = app
            .world()
            .resource_ref::<AcceptedKeymapDocument>()
            .last_changed();

        app.world_mut().remove_resource::<KeymapDiskWorker>();
        app.insert_state(ApplicationState::Ready)
            .insert_state(InteractionState::Resting);
        app.update();
        app.update();

        assert!(app.world().contains_resource::<CompiledKeymap>());
        assert!(matches!(
            app.world().resource::<EffectiveKeymapStatus>(),
            EffectiveKeymapStatus::Loaded(_)
        ));
        assert_eq!(
            app.world()
                .resource_ref::<crate::keymap::AcceptedKeymapDocument>()
                .last_changed(),
            accepted_document_tick,
            "state resolution must materialize the retained accepted document instead of accepting it again"
        );

        drop(app);
        drop(xdg_config_home);
        drop(temporary_directory);
        drop(environment_lock);
    }

    #[test]
    fn one_state_materialization_publishes_one_generation_to_palette_and_dispatch_table() {
        let mut app = state_dimension_keymap_app(
            r#"{
                "bindings": [
                    { "context": { "application": "ready" }, "bindings": { "p": "plugin::dispatch" } }
                ]
            }"#,
        );
        app.update();

        let expected_sequence: KeystrokeSequence = "p".parse().expect("fixture sequence parses");
        let effective_status = app.world().resource::<EffectiveKeymapStatus>();
        assert!(
            matches!(effective_status, EffectiveKeymapStatus::Loaded(_)),
            "expected a loaded state materialization, found {effective_status:?}"
        );
        let publication_generation = match effective_status {
            EffectiveKeymapStatus::Loaded(publication) => publication.generation,
            _ => return,
        };
        let compiled_generation = app.world().resource::<CompiledKeymap>().generation();
        let keymap_bindings = app.world().resource::<KeymapBindings>();
        let loaded_keymap_bindings = match keymap_bindings.authored() {
            AuthoredKeymapBindings::Loaded(bindings) => bindings,
            AuthoredKeymapBindings::Unavailable(_) => return,
        };
        let binding_generation = loaded_keymap_bindings.generation();
        assert_eq!(publication_generation, compiled_generation);
        assert_eq!(binding_generation, compiled_generation);
        {
            let world = app.world();
            let query_result = query_command_palette(
                world.resource::<CommandRegistry>(),
                world.resource::<ActiveKeymapContext>(),
                world.resource::<EffectiveKeymapStatus>(),
                world.resource::<KeymapBindings>(),
                "plugin dispatch",
            );
            assert_eq!(query_result.rows().len(), 1);
            assert_eq!(
                query_result.rows()[0].binding(),
                PaletteBinding::BoundTo(&expected_sequence)
            );
        }
        let match_outcome = app
            .world_mut()
            .resource_mut::<CompiledKeymap>()
            .match_effective(
                expected_sequence.first(),
                Instant::now(),
                Duration::from_secs(1),
            );
        assert!(matches!(match_outcome, MatchOutcome::Matched(_)));
    }

    #[test]
    fn state_materialization_changes_generation_once_per_reload_or_snapshot_change() {
        let mut app = state_dimension_keymap_app(
            r#"{ "bindings": [{ "bindings": { "g": "plugin::dispatch" } }] }"#,
        );
        app.update();
        let initial_generation = app.world().resource::<CompiledKeymap>().generation();

        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(
                    *br#"{
                        "bindings": [{
                            "context": { "application": "ready" },
                            "bindings": { "p": "plugin::dispatch" }
                        }]
                    }"#,
                )),
                diagnostics: Vec::new(),
            });
        keymap::commit_reload(app.world_mut());
        let reload_generation = initial_generation.next();
        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation(),
            reload_generation
        );
        app.update();
        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation(),
            reload_generation,
            "the scheduled effective commit must not republish an accepted reload"
        );

        app.world_mut()
            .resource_mut::<NextState<ApplicationState>>()
            .set(ApplicationState::Starting);
        app.update();
        app.update();
        let snapshot_generation = app.world().resource::<CompiledKeymap>().generation();
        assert_eq!(snapshot_generation, reload_generation.next());
        let compiled_tick = app.world().resource_ref::<CompiledKeymap>().last_changed();
        let binding_tick = app.world().resource_ref::<KeymapBindings>().last_changed();
        let status_tick = app
            .world()
            .resource_ref::<EffectiveKeymapStatus>()
            .last_changed();

        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation(),
            snapshot_generation
        );
        assert_eq!(
            app.world().resource_ref::<CompiledKeymap>().last_changed(),
            compiled_tick
        );
        assert_eq!(
            app.world().resource_ref::<KeymapBindings>().last_changed(),
            binding_tick
        );
        assert_eq!(
            app.world()
                .resource_ref::<EffectiveKeymapStatus>()
                .last_changed(),
            status_tick
        );
    }

    #[test]
    fn warmed_unchanged_state_skips_predicate_matching_compilation_and_publication() {
        let mut app = state_dimension_keymap_app(
            r#"{
                "bindings": [
                    { "bindings": { "g": "plugin::dispatch" } },
                    { "context": { "application": "ready" }, "bindings": { "p": "plugin::dispatch" } }
                ]
            }"#,
        );
        app.update();
        app.update();
        let compiled_tick = app.world().resource_ref::<CompiledKeymap>().last_changed();
        let binding_tick = app.world().resource_ref::<KeymapBindings>().last_changed();
        let status_tick = app
            .world()
            .resource_ref::<EffectiveKeymapStatus>()
            .last_changed();

        keymap::reset_predicate_match_count();
        app.update();
        // Allocations are counted around the commit itself rather than the scheduled pass:
        // `bevy/trace`, which `--workspace --all-features` turns on, wraps every system in a
        // span that allocates on this thread and would be charged to the keymap warm path.
        let allocations_before = crate::TEST_ALLOCATOR.allocation_count();
        keymap::commit_effective_keymap(app.world_mut());
        let allocations_after = crate::TEST_ALLOCATOR.allocation_count();

        assert_eq!(allocations_after - allocations_before, 0);
        assert_eq!(keymap::predicate_match_count(), 0);
        assert_eq!(
            app.world().resource_ref::<CompiledKeymap>().last_changed(),
            compiled_tick,
            "an unchanged scheduled pass must not compile another matcher"
        );
        assert_eq!(
            app.world().resource_ref::<KeymapBindings>().last_changed(),
            binding_tick,
            "an unchanged scheduled pass must not write effective palette bindings"
        );
        assert_eq!(
            app.world()
                .resource_ref::<EffectiveKeymapStatus>()
                .last_changed(),
            status_tick,
            "an unchanged scheduled pass must not publish another effective publication"
        );
    }

    #[test]
    fn state_dimension_same_layer_held_prefix_conflict_publishes_neither_binding() {
        let mut app = state_dimension_keymap_app(
            r#"{
                "bindings": [
                    { "bindings": { "p": "plugin::state_dimension_held" } },
                    { "context": { "application": "ready" }, "bindings": { "p q": "plugin::dispatch" } }
                ]
            }"#,
        );

        app.update();
        let held_command_id = command_id(StateDimensionHeld::ID);
        let dispatch_command_id = command_id(PluginDispatch::ID);
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&held_command_id),
            crate::CommandKeystroke::Unbound
        );
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&dispatch_command_id),
            crate::CommandKeystroke::Unbound
        );
        let diagnostics = &app.world().resource::<KeymapLoadFailures>().diagnostics;
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.kind == DiagnosticKind::HeldCommandInSequence)
                .count(),
            1
        );
        assert!(matches!(
            app.world().resource::<EffectiveKeymapStatus>(),
            EffectiveKeymapStatus::Loaded(publication)
                if publication.matched_layers.iter().map(|layer| layer.document_order).collect::<Vec<_>>() == vec![0]
        ));
    }

    #[test]
    fn state_dimension_tombstone_clears_a_same_layer_held_prefix_conflict() {
        let mut app = state_dimension_keymap_app(
            r#"{ "bindings": [{ "bindings": { "p": "plugin::state_dimension_held", "p q": "plugin::dispatch" } }] }"#,
        );

        app.update();
        let held_command_id = command_id(StateDimensionHeld::ID);
        let dispatch_command_id = command_id(PluginDispatch::ID);
        let expected_hold: KeystrokeSequence = "p".parse().expect("fixture sequence parses");
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&held_command_id),
            crate::CommandKeystroke::Unbound,
            "same-layer defaults reject both conflicting bindings before the user tombstone arrives"
        );

        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(
                    *br#"{ "bindings": [{ "context": { "application": "ready" }, "bindings": { "p q": null } }] }"#,
                )),
                diagnostics: Vec::new(),
            });
        keymap::commit_reload(app.world_mut());

        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&held_command_id),
            crate::CommandKeystroke::BoundTo(&expected_hold)
        );
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&dispatch_command_id),
            crate::CommandKeystroke::Unbound
        );
        assert!(
            app.world()
                .resource::<KeymapLoadFailures>()
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.kind != DiagnosticKind::HeldCommandInSequence)
        );
    }

    #[test]
    fn state_dimension_cross_layer_held_prefix_preserves_the_shipped_hold() {
        let mut app = state_dimension_keymap_app(
            r#"{ "bindings": [{ "bindings": { "p": "plugin::state_dimension_held" } }] }"#,
        );
        app.update();
        let held_command_id = command_id(StateDimensionHeld::ID);
        let dispatch_command_id = command_id(PluginDispatch::ID);
        let expected_hold: KeystrokeSequence = "p".parse().expect("fixture sequence parses");

        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(
                    *br#"{ "bindings": [{ "context": { "application": "ready" }, "bindings": { "p q": "plugin::dispatch" } }] }"#,
                )),
                diagnostics: Vec::new(),
            });
        keymap::commit_reload(app.world_mut());

        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&held_command_id),
            crate::CommandKeystroke::BoundTo(&expected_hold)
        );
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&dispatch_command_id),
            crate::CommandKeystroke::Unbound
        );
        let diagnostic = app
            .world()
            .resource::<KeymapLoadFailures>()
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.kind == DiagnosticKind::HeldCommandInSequence)
            .expect("user sequence conflict diagnostic");
        assert_eq!(
            diagnostic.origin,
            DiagnosticOrigin::KeymapFile(PathBuf::from(USER_KEYMAP_FIXTURE))
        );
        assert_eq!(diagnostic.command_id, PluginDispatch::ID);
    }

    #[test]
    fn state_dimension_same_user_layer_conflict_restores_the_shipped_hold() {
        let mut app = state_dimension_keymap_app(
            r#"{ "bindings": [{ "bindings": { "p": "plugin::state_dimension_held" } }] }"#,
        );
        app.update();
        let held_command_id = command_id(StateDimensionHeld::ID);
        let dispatch_command_id = command_id(PluginDispatch::ID);
        let expected_hold: KeystrokeSequence = "p".parse().expect("fixture sequence parses");

        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(
                    *br#"{
                        "bindings": [{
                            "context": { "application": "ready" },
                            "bindings": {
                                "p": "plugin::state_dimension_held",
                                "p q": "plugin::dispatch"
                            }
                        }]
                    }"#,
                )),
                diagnostics: Vec::new(),
            });
        keymap::commit_reload(app.world_mut());

        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&held_command_id),
            crate::CommandKeystroke::BoundTo(&expected_hold)
        );
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&dispatch_command_id),
            crate::CommandKeystroke::Unbound
        );
        let diagnostics = &app.world().resource::<KeymapLoadFailures>().diagnostics;
        let held_prefix_diagnostics = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.kind == DiagnosticKind::HeldCommandInSequence)
            .collect::<Vec<_>>();
        assert_eq!(held_prefix_diagnostics.len(), 1);
        assert_eq!(
            held_prefix_diagnostics[0].origin,
            DiagnosticOrigin::KeymapFile(PathBuf::from(USER_KEYMAP_FIXTURE))
        );
        assert_eq!(
            held_prefix_diagnostics[0].command_id,
            StateDimensionHeld::ID
        );

        keymap::commit_effective_keymap(app.world_mut());
        assert_eq!(
            app.world()
                .resource::<KeymapLoadFailures>()
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.kind == DiagnosticKind::HeldCommandInSequence)
                .count(),
            1,
            "an unchanged effective snapshot must not retain another copy of its conflict diagnostic"
        );
    }

    #[test]
    fn state_dimension_user_hold_rejects_a_shipped_longer_sequence() {
        let mut app = state_dimension_keymap_app(
            r#"{ "bindings": [{ "bindings": { "p q": "plugin::dispatch" } }] }"#,
        );
        app.update();
        let held_command_id = command_id(StateDimensionHeld::ID);
        let dispatch_command_id = command_id(PluginDispatch::ID);
        let expected_hold: KeystrokeSequence = "p".parse().expect("fixture sequence parses");

        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(
                    *br#"{
                        "bindings": [{
                            "context": { "application": "ready" },
                            "bindings": { "p": "plugin::state_dimension_held" }
                        }]
                    }"#,
                )),
                diagnostics: Vec::new(),
            });
        keymap::commit_reload(app.world_mut());

        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&held_command_id),
            crate::CommandKeystroke::BoundTo(&expected_hold)
        );
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&dispatch_command_id),
            crate::CommandKeystroke::Unbound
        );
        let diagnostic = app
            .world()
            .resource::<KeymapLoadFailures>()
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.kind == DiagnosticKind::HeldCommandInSequence)
            .expect("shipped sequence conflict diagnostic");
        assert_eq!(diagnostic.origin, DiagnosticOrigin::EmbeddedDefaults);
        assert_eq!(diagnostic.command_id, PluginDispatch::ID);
    }

    #[test]
    fn state_dimension_predicate_diagnostics_locate_unknown_terms_and_retain_authoring_advisories()
    {
        let defaults = r#"{
            "bindings": [
                { "contxt": true, "bindings": { "a": "plugin::dispach" } },
                {
                    "context": { "application": "not_ready", "unknown": "value" },
                    "bindings": {}
                }
            ]
        }"#;
        let app = state_dimension_keymap_app(defaults);
        let diagnostics = &app.world().resource::<KeymapLoadFailures>().diagnostics;
        let published_defaults = app
            .world()
            .resource::<ReloadConfiguration>()
            .published_defaults();
        let unknown_value_start = published_defaults
            .find("not_ready")
            .expect("unknown value in published defaults");
        let unknown_dimension_start = published_defaults
            .find("unknown")
            .expect("unknown dimension in published defaults");
        let (unknown_value_line, unknown_value_column) =
            source_line_and_column(published_defaults, unknown_value_start);
        let (unknown_dimension_line, unknown_dimension_column) =
            source_line_and_column(published_defaults, unknown_dimension_start);
        let unknown_value = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.context == "application=not_ready")
            .expect("unknown state value diagnostic");
        let unknown_dimension = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.context == "unknown=value")
            .expect("unknown state dimension diagnostic");
        let unrecognized_member = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("contxt"))
            .expect("unrecognized member advisory");
        let unknown_command = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.command_id == "plugin::dispach")
            .expect("unknown command diagnostic");

        assert_eq!(unknown_value.origin, DiagnosticOrigin::EmbeddedDefaults);
        assert_eq!(
            unknown_value.byte_range,
            unknown_value_start..unknown_value_start + "not_ready".len()
        );
        assert_eq!(unknown_value.line, unknown_value_line);
        assert_eq!(unknown_value.column, unknown_value_column);
        assert!(unknown_value.message.contains("application"));
        assert!(unknown_value.message.contains("not_ready"));
        assert_eq!(unknown_dimension.origin, DiagnosticOrigin::EmbeddedDefaults);
        assert_eq!(
            unknown_dimension.byte_range,
            unknown_dimension_start..unknown_dimension_start + "unknown".len()
        );
        assert_eq!(unknown_dimension.line, unknown_dimension_line);
        assert_eq!(unknown_dimension.column, unknown_dimension_column);
        assert!(unknown_dimension.message.contains("unknown"));
        assert!(unknown_dimension.message.contains("value"));
        assert_eq!(unrecognized_member.severity, DiagnosticSeverity::Advisory);
        assert_eq!(
            unrecognized_member.suggestions,
            vec![String::from("context")]
        );
        assert_eq!(
            unknown_command.suggestions,
            vec![String::from(PluginDispatch::ID)]
        );
    }

    #[test]
    fn empty_context_dimension_names_never_publish_empty_diagnostic_subjects() {
        const EMPTY_NAME_CONTEXTS: [(&str, &str); 2] = [
            (
                "a string value",
                r#"{
                    "bindings": [{
                        "context": { "": "ready" },
                        "bindings": { "p": "plugin::dispatch" }
                    }]
                }"#,
            ),
            (
                "a wrong-typed value",
                r#"{
                    "bindings": [{
                        "context": { "": 7 },
                        "bindings": { "p": "plugin::dispatch" }
                    }]
                }"#,
            ),
        ];

        for (value_kind, defaults) in EMPTY_NAME_CONTEXTS {
            let app = state_dimension_keymap_app(defaults);
            let diagnostics = &app.world().resource::<KeymapLoadFailures>().diagnostics;

            assert!(
                !diagnostics.is_empty(),
                "an empty dimension name with {value_kind} must be rejected"
            );
            assert!(
                diagnostics
                    .iter()
                    .all(|diagnostic| !diagnostic.context.is_empty()),
                "an empty dimension name with {value_kind} produced an empty diagnostic context: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn state_dimension_acceptance_rejects_string_contexts_without_creating_an_accepted_document() {
        let app = state_dimension_keymap_app(
            r#"{ "bindings": [{ "context": "ready", "bindings": { "p": "plugin::dispatch" } }] }"#,
        );

        assert_eq!(
            app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::RejectedInitialDocument
        );
        assert!(
            !app.world()
                .contains_resource::<crate::keymap::AcceptedKeymapDocument>()
        );
        assert!(
            app.world()
                .resource::<KeymapLoadFailures>()
                .diagnostics
                .iter()
                .any(|diagnostic| {
                    diagnostic.kind == DiagnosticKind::Syntax
                        && diagnostic.context == "context"
                        && diagnostic.message.contains("object of state dimensions")
                })
        );
    }

    #[test]
    fn rejected_state_dimension_reload_keeps_the_last_accepted_generation_and_publication()
    -> Result<(), String> {
        let mut app = state_dimension_keymap_app(
            r#"{ "bindings": [{ "bindings": { "p": "plugin::dispatch" } }] }"#,
        );
        app.update();
        let command_id = command_id(PluginDispatch::ID);
        let expected_sequence: KeystrokeSequence = "p".parse().expect("fixture sequence parses");
        let generation = app.world().resource::<CompiledKeymap>().generation();
        let accepted_document_tick = app
            .world()
            .resource_ref::<AcceptedKeymapDocument>()
            .last_changed();
        let compiled_tick = app.world().resource_ref::<CompiledKeymap>().last_changed();
        let (binding_generation, binding_table) =
            match app.world().resource::<KeymapBindings>().authored() {
                AuthoredKeymapBindings::Loaded(bindings) => {
                    (bindings.generation(), bindings.global_bindings_for_test())
                },
                AuthoredKeymapBindings::Unavailable(_) => {
                    return Err(String::from(
                        "expected loaded bindings before rejected reload",
                    ));
                },
            };
        let binding_tick = app.world().resource_ref::<KeymapBindings>().last_changed();
        let publication = match app.world().resource::<EffectiveKeymapStatus>() {
            EffectiveKeymapStatus::Loaded(publication) => publication.clone(),
            status => {
                return Err(format!(
                    "expected a loaded state-dimension keymap, found {status:?}"
                ));
            },
        };

        app.world_mut()
            .resource_mut::<PendingReload>()
            .replace(ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(
                    *br#"{ "bindings": [{ "context": { "application": "missing" }, "bindings": { "q": "plugin::dispatch" } }] }"#,
                )),
                diagnostics: Vec::new(),
            });
        keymap::commit_reload(app.world_mut());

        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation(),
            generation
        );
        assert_eq!(
            app.world()
                .resource_ref::<crate::keymap::AcceptedKeymapDocument>()
                .last_changed(),
            accepted_document_tick,
            "a rejected reload must retain the accepted document"
        );
        assert_eq!(
            app.world().resource_ref::<CompiledKeymap>().last_changed(),
            compiled_tick,
            "a rejected reload must retain the compiled matcher resource"
        );
        let (current_binding_generation, current_binding_table) =
            match app.world().resource::<KeymapBindings>().authored() {
                AuthoredKeymapBindings::Loaded(bindings) => {
                    (bindings.generation(), bindings.global_bindings_for_test())
                },
                AuthoredKeymapBindings::Unavailable(_) => {
                    return Err(String::from("a rejected reload replaced loaded bindings"));
                },
            };
        assert_eq!(current_binding_generation, binding_generation);
        assert_eq!(current_binding_table, binding_table);
        assert_eq!(
            app.world().resource_ref::<KeymapBindings>().last_changed(),
            binding_tick,
            "a rejected reload must retain the loaded binding table resource"
        );
        assert_eq!(
            app.world()
                .resource::<KeymapBindings>()
                .keystroke(&command_id),
            crate::CommandKeystroke::BoundTo(&expected_sequence)
        );
        assert_eq!(
            app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::Loaded(publication)
        );
        assert!(
            app.world()
                .resource::<KeymapLoadFailures>()
                .diagnostics
                .iter()
                .any(|diagnostic| {
                    diagnostic.origin
                        == DiagnosticOrigin::KeymapFile(PathBuf::from(USER_KEYMAP_FIXTURE))
                        && diagnostic.context == "application=missing"
                })
        );
        Ok(())
    }

    struct SnapshotReloadCoarrivalFixture {
        app:                App,
        custom_input:       CustomInput,
        initial_generation: KeymapGeneration,
    }

    impl SnapshotReloadCoarrivalFixture {
        fn new() -> Result<Self, String> {
            let mut app = App::new();
            register_state_dimension_commands(&mut app);
            app.init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<DispatchCount>()
                .init_resource::<RoutingResetTrace>()
                .add_plugins(StatesPlugin)
                .insert_state(ApplicationState::Ready)
                .insert_state(InteractionState::Resting)
                .add_plugins(
                    KeymapPlugin::new()
                        .with_defaults(
                            r#"{
                            "bindings": [{ "bindings": {
                                "g": "plugin::dispatch",
                                "g h": "plugin::dispatch",
                                "p": "plugin::state_dimension_held"
                            }}]
                        }"#,
                        )
                        .with_state_dimension::<ApplicationState>("application")
                        .with_state_dimension::<InteractionState>("interaction"),
                );
            app.world_mut().add_observer(
                |_: On<PluginDispatch>, mut dispatch_count: ResMut<DispatchCount>| {
                    dispatch_count.0 += 1;
                },
            );
            app.finish();
            app.update();
            let initial_generation = app.world().resource::<CompiledKeymap>().generation();
            app.world_mut()
                .resource_mut::<RoutingResetTrace>()
                .0
                .clear();

            let held_command_id = command_id(StateDimensionHeld::ID);
            let custom_input = match app
                .world()
                .resource::<CommandRegistry>()
                .held_command_lookup(&held_command_id)
            {
                HeldCommandLookupOutcome::RegisteredHeldInput(input) => input,
                outcome => return Err(format!("held command did not register: {outcome:?}")),
            };
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyP);
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .clear_just_pressed(KeyCode::KeyP);
            assert_eq!(
                app.world().resource::<CustomInputs>().get(&custom_input),
                Some(&ActionValue::Bool(true))
            );
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyG);
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .clear_just_pressed(KeyCode::KeyG);
            assert!(
                app.world()
                    .resource::<CompiledKeymap>()
                    .has_pending_sequence()
            );

            Ok(Self {
                app,
                custom_input,
                initial_generation,
            })
        }

        fn accept_transition(&mut self) -> KeymapGeneration {
            let accepted_reload = ReloadRequest::UserSnapshot {
                source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                contents:    UserKeymapContents::Read(Arc::from(
                    *br#"{
                    "bindings": [
                        { "bindings": { "j": "plugin::dispatch" } },
                        {
                            "context": { "application": "starting" },
                            "bindings": { "k": "plugin::dispatch" }
                        }
                    ]
                }"#,
                )),
                diagnostics: Vec::new(),
            };
            self.app
                .world_mut()
                .resource_mut::<NextState<ApplicationState>>()
                .set(ApplicationState::Starting);
            self.app.update();
            self.app
                .world_mut()
                .resource_mut::<PendingReload>()
                .replace(accepted_reload);
            self.app.update();

            let effective_generation = self.app.world().resource::<CompiledKeymap>().generation();
            assert_eq!(effective_generation, self.initial_generation.next());
            assert!(matches!(
                    self.app.world().resource::<EffectiveKeymapStatus>(),
                EffectiveKeymapStatus::Loaded(publication)
                    if publication.generation == effective_generation
                        && matches!(
                            &publication.snapshot,
                            crate::EffectiveKeymapSnapshot::Resolved(snapshot)
                                if snapshot.values().any(|(dimension, value)| {
                                    dimension.as_str() == "application"
                                        && value.as_str() == "starting"
                                })
                        )
            ));
            assert!(matches!(
                self.app.world().resource::<KeymapBindings>().authored(),
                crate::AuthoredKeymapBindings::Loaded(bindings)
                    if bindings.generation() == effective_generation
            ));
            assert!(
                !self
                    .app
                    .world()
                    .resource::<CompiledKeymap>()
                    .has_pending_sequence()
            );
            assert_eq!(
                self.app
                    .world()
                    .resource::<CustomInputs>()
                    .get(&self.custom_input),
                Some(&ActionValue::Bool(false))
            );
            assert_eq!(
                self.app.world().resource::<RoutingResetTrace>().0,
                vec![
                    RoutingResetStep::PendingSequenceCancelled,
                    RoutingResetStep::PhysicalSourcesReleased,
                    RoutingResetStep::RoutingStateRecorded,
                    RoutingResetStep::PressedKeysInhibited,
                ]
            );

            effective_generation
        }

        fn reject_reload_and_fresh_press(&mut self, effective_generation: KeymapGeneration) {
            self.app
                .world_mut()
                .resource_mut::<PendingReload>()
                .replace(ReloadRequest::UserSnapshot {
                    source_path: PathBuf::from(USER_KEYMAP_FIXTURE),
                    contents:    UserKeymapContents::Read(Arc::from(
                        *br#"{
                        "bindings": [{
                            "context": { "application": "unknown" },
                            "bindings": { "x": "plugin::dispatch" }
                        }]
                    }"#,
                    )),
                    diagnostics: Vec::new(),
                });
            self.app.update();
            assert_eq!(
                self.app.world().resource::<CompiledKeymap>().generation(),
                effective_generation
            );
            assert_eq!(self.app.world().resource::<RoutingResetTrace>().0.len(), 4);

            for key in [KeyCode::KeyG, KeyCode::KeyP] {
                self.app
                    .world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .release(key);
            }
            self.app.update();
            for key in [KeyCode::KeyG, KeyCode::KeyP] {
                self.app
                    .world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .clear_just_released(key);
            }
            self.app
                .world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyK);
            self.app.update();
            assert_eq!(self.app.world().resource::<DispatchCount>().0, 1);
        }
    }

    #[test]
    fn snapshot_and_reload_coarrival_publishes_once_and_resets_routing_once() -> Result<(), String>
    {
        let mut fixture = SnapshotReloadCoarrivalFixture::new()?;
        let effective_generation = fixture.accept_transition();
        fixture.reject_reload_and_fresh_press(effective_generation);
        Ok(())
    }
}
