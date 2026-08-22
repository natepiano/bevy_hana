//! Typed application-state dimensions used by keymap predicates.

use std::any::TypeId;
use std::collections::BTreeMap;
use std::collections::HashSet;

use bevy::app::App;
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::Local;
use bevy::ecs::system::ParamSet;
use bevy::prelude::Message;
use bevy::prelude::MessageWriter;
use bevy::prelude::PreUpdate;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::State;
use bevy::prelude::States;
use bevy::reflect::ReflectDeserialize;
use bevy::reflect::ReflectSerialize;
use serde::Deserialize;
use serde::Serialize;
use strum::EnumMessage;
use strum::IntoEnumIterator;

use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;
use crate::KeymapLoadFailures;
use crate::KeymapSystems;
use crate::keymap_plugin::RegistryValidationFailed;

/// Requirements for a typed, application-owned keymap state dimension.
///
/// This trait is a downstream extension point for application-owned Bevy state enums.
/// Applications derive `strum::EnumIter`, `strum::AsRefStr`, and
/// `strum::EnumMessage` on their Bevy state enum. Each value supplies the
/// authoring name and nonempty reference description Rubric publishes.
pub trait KeymapStateDimension:
    AsRef<str> + Copy + EnumMessage + Eq + IntoEnumIterator + Send + Sync + 'static
{
}

impl<T> KeymapStateDimension for T where
    T: AsRef<str> + Copy + EnumMessage + Eq + IntoEnumIterator + Send + Sync + 'static
{
}

/// The authored name of one total application state dimension.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Reflect, Serialize)]
#[reflect(opaque, Serialize, Deserialize)]
pub struct ContextDimensionName(String);

impl ContextDimensionName {
    pub(crate) fn new(name: impl Into<String>) -> Self { Self(name.into()) }

    /// Borrows the dimension name the application registered.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl AsRef<str> for ContextDimensionName {
    fn as_ref(&self) -> &str { self.as_str() }
}

/// The authored name of one value in a registered state dimension.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Reflect, Serialize)]
#[reflect(opaque, Serialize, Deserialize)]
pub struct ContextValueName(String);

impl ContextValueName {
    pub(crate) fn new(name: impl Into<String>) -> Self { Self(name.into()) }

    /// Borrows the value name declared by the application's typed state.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl AsRef<str> for ContextValueName {
    fn as_ref(&self) -> &str { self.as_str() }
}

/// The complete, deterministic state selection observed from the application.
///
/// Entries are ordered by dimension name rather than plugin registration order.
#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub struct ContextSnapshot {
    values: BTreeMap<ContextDimensionName, ContextValueName>,
}

impl ContextSnapshot {
    const fn from_staged_values(values: BTreeMap<ContextDimensionName, ContextValueName>) -> Self {
        Self { values }
    }

    /// Iterates through dimension/value pairs in ascending dimension-name order.
    pub fn values(&self) -> impl Iterator<Item = (&ContextDimensionName, &ContextValueName)> {
        self.values.iter()
    }
}

#[cfg(test)]
impl FromIterator<(ContextDimensionName, ContextValueName)> for ContextSnapshot {
    fn from_iter<T>(values: T) -> Self
    where
        T: IntoIterator<Item = (ContextDimensionName, ContextValueName)>,
    {
        Self {
            values: values.into_iter().collect(),
        }
    }
}

/// The currently observable state of all keymap state dimensions.
#[derive(Debug, Reflect, Resource)]
#[reflect(Resource)]
pub struct ActiveKeymapContext {
    state: ActiveKeymapContextState,
}

impl Default for ActiveKeymapContext {
    fn default() -> Self {
        Self {
            state: ActiveKeymapContextState::GlobalRouting,
        }
    }
}

impl ActiveKeymapContext {
    /// Returns the durable context state readers may inspect between transitions.
    #[must_use]
    pub const fn state(&self) -> &ActiveKeymapContextState { &self.state }

    fn await_state_dimensions(&mut self) {
        self.state = ActiveKeymapContextState::AwaitingStateDimensions;
    }
}

#[cfg(test)]
impl From<ActiveKeymapContextState> for ActiveKeymapContext {
    fn from(state: ActiveKeymapContextState) -> Self { Self { state } }
}

/// Whether all registered state dimensions currently form a usable snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Reflect)]
pub enum ActiveKeymapContextState {
    /// No state dimensions are registered, so only global bindings may route.
    GlobalRouting,
    /// Registered dimensions have not all reported either a value or their absence yet.
    AwaitingStateDimensions,
    /// One or more application-owned state resources are absent, so no binding may route.
    StateDimensionsUnavailable {
        /// The complete, sorted names of dimensions whose `State<C>` resource is absent.
        missing: Vec<ContextDimensionName>,
    },
    /// Every registered state dimension is available in this complete snapshot.
    Resolved(ContextSnapshot),
}

/// A semantic change to the complete active state-dimension snapshot.
#[derive(Clone, Debug, Message, PartialEq, Eq)]
pub struct ActiveKeymapContextTransition {
    /// The state before this complete-snapshot change.
    pub previous: ActiveKeymapContextState,
    /// The state after this complete-snapshot change.
    pub current:  ActiveKeymapContextState,
}

/// One typed state-dimension registration retained during plugin assembly.
#[derive(Clone)]
pub(crate) struct StateDimensionRegistration {
    name:             String,
    state_type:       TypeId,
    state_type_name:  &'static str,
    values:           Vec<RegisteredStateDimensionValue>,
    install_observer: fn(&mut App),
}

impl StateDimensionRegistration {
    pub(crate) fn new<C: KeymapStateDimension + States>(name: &str) -> Self {
        Self {
            name:             name.to_owned(),
            state_type:       TypeId::of::<C>(),
            state_type_name:  std::any::type_name::<C>(),
            values:           C::iter()
                .map(|value| RegisteredStateDimensionValue {
                    name:        ContextValueName::new(value.as_ref()),
                    description: value.get_message().into(),
                })
                .collect(),
            install_observer: install_state_dimension_observer::<C>,
        }
    }

    fn display(&self) -> String { format!("`{}` ({})", self.name, self.state_type_name) }

    fn install_observer(&self, app: &mut App) { (self.install_observer)(app); }
}

#[derive(Clone)]
struct RegisteredStateDimensionValue {
    name:        ContextValueName,
    description: StateDimensionValueDescription,
}

/// Whether an application declared usable help text for one state value.
#[derive(Clone)]
enum StateDimensionValueDescription {
    DeclaredNonempty(String),
    MissingOrEmpty,
}

impl From<Option<&str>> for StateDimensionValueDescription {
    fn from(description: Option<&str>) -> Self {
        match description {
            Some(description) if !description.is_empty() => {
                Self::DeclaredNonempty(description.to_owned())
            },
            Some(_) | None => Self::MissingOrEmpty,
        }
    }
}

/// Registered state-dimension declarations and their typed value metadata.
#[derive(Default, Resource)]
pub(crate) struct StateDimensionRegistry {
    dimensions: Vec<RegisteredStateDimension>,
}

impl StateDimensionRegistry {
    fn register(
        &mut self,
        registrations: &[StateDimensionRegistration],
    ) -> Result<(), Vec<Diagnostic>> {
        let mut diagnostics = Vec::new();
        let mut accepted = Vec::with_capacity(registrations.len());

        for registration in registrations {
            let candidate = RegisteredStateDimension::from(registration);
            let duplicate_name = self
                .dimensions
                .iter()
                .chain(accepted.iter())
                .find(|dimension| dimension.name.as_str() == registration.name);
            let duplicate_type = self
                .dimensions
                .iter()
                .chain(accepted.iter())
                .find(|dimension| dimension.state_type == registration.state_type);

            if registration.name.is_empty() {
                diagnostics.push(state_dimension_diagnostic(
                    registration,
                    &format!(
                        "A keymap state-dimension name for {} must not be empty.",
                        registration.display(),
                    ),
                ));
            }
            if let Some(previous) = duplicate_name {
                diagnostics.push(state_dimension_diagnostic(
                    registration,
                    &format!(
                        "State-dimension name `{}` is registered by both {} and {}.",
                        registration.name,
                        previous.display(),
                        registration.display(),
                    ),
                ));
            }
            if let Some(previous) = duplicate_type {
                diagnostics.push(state_dimension_diagnostic(
                    registration,
                    &format!(
                        "State type `{}` is registered by both {} and {}.",
                        registration.state_type_name,
                        previous.display(),
                        registration.display(),
                    ),
                ));
            }

            let mut declared_names = HashSet::new();
            for value in &candidate.values {
                if value.name.as_str().is_empty() {
                    diagnostics.push(state_dimension_diagnostic(
                        registration,
                        "A keymap state-dimension value must not be empty.",
                    ));
                }
                if !declared_names.insert(value.name.as_str()) {
                    diagnostics.push(state_dimension_diagnostic(
                        registration,
                        &format!(
                            "State-dimension value `{}` is declared more than once by {}.",
                            value.name.as_str(),
                            registration.display(),
                        ),
                    ));
                }
                if matches!(
                    value.description,
                    StateDimensionValueDescription::MissingOrEmpty
                ) {
                    diagnostics.push(state_dimension_diagnostic(
                        registration,
                        &format!(
                            "State-dimension value `{}` in {} has no description. Add #[strum(message = \"…\")].",
                            value.name.as_str(),
                            registration.display(),
                        ),
                    ));
                }
            }

            accepted.push(candidate);
        }

        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }

        self.dimensions.extend(accepted);
        Ok(())
    }

    fn registration_for_state<C: States>(&self) -> StateDimensionRegistrationLookup<'_> {
        self.dimensions
            .iter()
            .find(|dimension| dimension.state_type == TypeId::of::<C>())
            .map_or(
                StateDimensionRegistrationLookup::NotRegistered,
                |dimension| StateDimensionRegistrationLookup::Registered(&dimension.name),
            )
    }

    fn dimensions(&self) -> impl Iterator<Item = &RegisteredStateDimension> {
        self.dimensions.iter()
    }

    /// Names the runtime state a valid active context must provide for this registry.
    pub(crate) const fn routing_requirement(&self) -> StateDimensionRoutingRequirement {
        if self.dimensions.is_empty() {
            StateDimensionRoutingRequirement::GlobalOnly
        } else {
            StateDimensionRoutingRequirement::CompleteSnapshotRequired
        }
    }

    /// Returns the complete authoring vocabulary in stable dimension-name order.
    pub(crate) fn metadata(&self) -> Vec<StateDimensionMetadata<'_>> {
        let mut dimensions = self
            .dimensions
            .iter()
            .map(StateDimensionMetadata::from)
            .collect::<Vec<_>>();
        dimensions.sort_unstable_by(|left, right| left.name.cmp(right.name));
        dimensions
    }

    pub(crate) fn resolves(
        &self,
        dimension_name: &ContextDimensionName,
        value_name: &ContextValueName,
    ) -> bool {
        self.dimensions.iter().any(|dimension| {
            dimension.name == *dimension_name
                && dimension
                    .values
                    .iter()
                    .any(|value| value.name == *value_name)
        })
    }

    /// Reports whether a reflected snapshot is exactly one registered value for every dimension.
    pub(crate) fn recognizes_complete_snapshot(&self, snapshot: &ContextSnapshot) -> bool {
        snapshot.values.len() == self.dimensions.len()
            && snapshot
                .values()
                .all(|(dimension, value)| self.resolves(dimension, value))
    }
}

/// The active-context requirement imposed by the assembled state-dimension registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StateDimensionRoutingRequirement {
    /// No dimensions are registered, so global routing is the complete application state.
    GlobalOnly,
    /// Registered dimensions require one recognized value each before routing may activate.
    CompleteSnapshotRequired,
}

/// One registered state dimension exposed to document and companion-file generation.
pub(crate) struct StateDimensionMetadata<'registry> {
    pub(crate) name:        &'registry ContextDimensionName,
    pub(crate) description: &'registry str,
    pub(crate) values:      Vec<StateDimensionValueMetadata<'registry>>,
}

impl<'registry> From<&'registry RegisteredStateDimension> for StateDimensionMetadata<'registry> {
    fn from(dimension: &'registry RegisteredStateDimension) -> Self {
        Self {
            name:        &dimension.name,
            description: dimension.state_type_name,
            values:      dimension
                .values
                .iter()
                .map(StateDimensionValueMetadata::from)
                .collect(),
        }
    }
}

/// One typed value in a state-dimension authoring vocabulary.
pub(crate) struct StateDimensionValueMetadata<'registry> {
    pub(crate) name:        &'registry ContextValueName,
    pub(crate) description: &'registry str,
}

impl<'registry> From<&'registry RegisteredStateDimensionValue>
    for StateDimensionValueMetadata<'registry>
{
    fn from(value: &'registry RegisteredStateDimensionValue) -> Self {
        let description = match &value.description {
            StateDimensionValueDescription::DeclaredNonempty(description) => description,
            StateDimensionValueDescription::MissingOrEmpty => "",
        };
        Self {
            name: &value.name,
            description,
        }
    }
}

struct RegisteredStateDimension {
    name:            ContextDimensionName,
    state_type:      TypeId,
    state_type_name: &'static str,
    values:          Vec<RegisteredStateDimensionValue>,
}

impl From<&StateDimensionRegistration> for RegisteredStateDimension {
    fn from(registration: &StateDimensionRegistration) -> Self {
        Self {
            name:            ContextDimensionName::new(registration.name.clone()),
            state_type:      registration.state_type,
            state_type_name: registration.state_type_name,
            values:          registration.values.clone(),
        }
    }
}

impl RegisteredStateDimension {
    fn display(&self) -> String { format!("`{}` ({})", self.name.as_str(), self.state_type_name) }
}

enum StateDimensionRegistrationLookup<'registry> {
    Registered(&'registry ContextDimensionName),
    NotRegistered,
}

#[derive(Default, Resource)]
struct StagedStateDimensions {
    observations: BTreeMap<ContextDimensionName, StateDimensionObservation>,
}

impl StagedStateDimensions {
    fn needs_available(&self, dimension: &ContextDimensionName, value: &str) -> bool {
        !matches!(
            self.observations.get(dimension),
            Some(StateDimensionObservation::Available(current)) if current.as_str() == value
        )
    }

    fn needs_unavailable(&self, dimension: &ContextDimensionName) -> bool {
        !matches!(
            self.observations.get(dimension),
            Some(StateDimensionObservation::Unavailable)
        )
    }

    fn stage_available(&mut self, dimension: &ContextDimensionName, value: &str) {
        let observation = StateDimensionObservation::Available(ContextValueName::new(value));
        self.observations.insert(dimension.clone(), observation);
    }

    fn stage_unavailable(&mut self, dimension: &ContextDimensionName) {
        self.observations
            .insert(dimension.clone(), StateDimensionObservation::Unavailable);
    }

    fn observation(&self, dimension: &ContextDimensionName) -> StateDimensionObservationRef<'_> {
        match self.observations.get(dimension) {
            Some(StateDimensionObservation::Available(value)) => {
                StateDimensionObservationRef::Available(value)
            },
            Some(StateDimensionObservation::Unavailable) => {
                StateDimensionObservationRef::Unavailable
            },
            None => StateDimensionObservationRef::AwaitingObservation,
        }
    }
}

enum StateDimensionObservation {
    Available(ContextValueName),
    Unavailable,
}

enum StateDimensionObservationRef<'value> {
    Available(&'value ContextValueName),
    Unavailable,
    AwaitingObservation,
}

#[derive(Default, Eq, PartialEq)]
enum StateDimensionAvailability {
    #[default]
    AwaitingObservation,
    Resolved,
    Unavailable,
}

/// Whether state-dimension registration can still produce a routable assembled application.
#[derive(Default, Resource)]
enum StateDimensionRegistrationValidity {
    /// Every registration batch accepted so far, so complete typed collection may activate.
    #[default]
    Valid,
    /// At least one registration batch failed, so partial registered state must remain inactive.
    PermanentlyInvalid,
}

fn install_state_dimension_observer<C: KeymapStateDimension + States>(app: &mut App) {
    app.add_systems(
        PreUpdate,
        observe_state_dimension::<C>.in_set(KeymapSystems::ObserveStateDimensions),
    );
}

fn observe_state_dimension<C: KeymapStateDimension + States>(
    state: Option<Res<State<C>>>,
    dimensions: Res<StateDimensionRegistry>,
    mut staged_dimensions: ParamSet<(Res<StagedStateDimensions>, ResMut<StagedStateDimensions>)>,
) {
    let StateDimensionRegistrationLookup::Registered(dimension) =
        dimensions.registration_for_state::<C>()
    else {
        return;
    };
    let changed = match state.as_ref() {
        Some(state) => staged_dimensions
            .p0()
            .needs_available(dimension, state.get().as_ref()),
        None => staged_dimensions.p0().needs_unavailable(dimension),
    };
    if !changed {
        return;
    }

    match state {
        Some(state) => staged_dimensions
            .p1()
            .stage_available(dimension, state.get().as_ref()),
        None => staged_dimensions.p1().stage_unavailable(dimension),
    }
}

pub(crate) fn register_state_dimensions(
    app: &mut App,
    registrations: &[StateDimensionRegistration],
) {
    crate::KeymapPlugin::install_runtime(app);
    app.world_mut()
        .resource_mut::<ActiveKeymapContext>()
        .await_state_dimensions();
    app.init_resource::<StateDimensionRegistry>()
        .init_resource::<StagedStateDimensions>()
        .init_resource::<StateDimensionRegistrationValidity>();

    let registration_result = app
        .world_mut()
        .resource_mut::<StateDimensionRegistry>()
        .register(registrations);
    if let Err(diagnostics) = registration_result {
        *app.world_mut()
            .resource_mut::<StateDimensionRegistrationValidity>() =
            StateDimensionRegistrationValidity::PermanentlyInvalid;
        app.world_mut().insert_resource(RegistryValidationFailed);
        retain_state_dimension_diagnostics(app, &diagnostics);
        return;
    }
    for registration in registrations {
        registration.install_observer(app);
    }
    if !app
        .world()
        .contains_resource::<StateDimensionCollectionInstalled>()
    {
        app.init_resource::<StateDimensionCollectionInstalled>()
            .add_systems(
                PreUpdate,
                collect_state_dimensions.in_set(KeymapSystems::UpdateActiveKeymapContext),
            );
    }
}

#[derive(Default, Resource)]
struct StateDimensionCollectionInstalled;

fn collect_state_dimensions(
    registered_dimensions: Res<StateDimensionRegistry>,
    staged_dimensions: Res<StagedStateDimensions>,
    registration_validity: Res<StateDimensionRegistrationValidity>,
    mut active_context: ResMut<ActiveKeymapContext>,
    mut availability: Local<StateDimensionAvailability>,
    mut transitions: MessageWriter<ActiveKeymapContextTransition>,
) {
    if matches!(
        *registration_validity,
        StateDimensionRegistrationValidity::PermanentlyInvalid
    ) {
        return;
    }
    if !staged_dimensions.is_changed() && !active_context.is_changed() {
        return;
    }

    let current = collect_active_keymap_context_state(&registered_dimensions, &staged_dimensions);
    if active_context.state == current {
        return;
    }

    let unavailable_episode_started = matches!(
        (&*availability, &current),
        (
            StateDimensionAvailability::AwaitingObservation | StateDimensionAvailability::Resolved,
            ActiveKeymapContextState::StateDimensionsUnavailable { .. }
        )
    );
    match current {
        ActiveKeymapContextState::Resolved(_) => {
            *availability = StateDimensionAvailability::Resolved;
        },
        ActiveKeymapContextState::StateDimensionsUnavailable { .. } => {
            *availability = StateDimensionAvailability::Unavailable;
        },
        ActiveKeymapContextState::GlobalRouting
        | ActiveKeymapContextState::AwaitingStateDimensions => {
            *availability = StateDimensionAvailability::AwaitingObservation;
        },
    }

    let previous = std::mem::replace(&mut active_context.state, current);
    if unavailable_episode_started
        && let ActiveKeymapContextState::StateDimensionsUnavailable { missing } =
            &active_context.state
    {
        warn_state_dimensions_unavailable(missing);
    }
    transitions.write(ActiveKeymapContextTransition {
        previous,
        current: active_context.state.clone(),
    });
}

fn collect_active_keymap_context_state(
    registered_dimensions: &StateDimensionRegistry,
    staged_dimensions: &StagedStateDimensions,
) -> ActiveKeymapContextState {
    let mut missing = Vec::new();
    let mut values = BTreeMap::new();
    for dimension in registered_dimensions.dimensions() {
        match staged_dimensions.observation(&dimension.name) {
            StateDimensionObservationRef::Available(value) => {
                values.insert(dimension.name.clone(), value.clone());
            },
            StateDimensionObservationRef::Unavailable => missing.push(dimension.name.clone()),
            StateDimensionObservationRef::AwaitingObservation => {
                return ActiveKeymapContextState::AwaitingStateDimensions;
            },
        }
    }

    if missing.is_empty() {
        ActiveKeymapContextState::Resolved(ContextSnapshot::from_staged_values(values))
    } else {
        missing.sort_unstable();
        ActiveKeymapContextState::StateDimensionsUnavailable { missing }
    }
}

fn warn_state_dimensions_unavailable(missing: &[ContextDimensionName]) {
    let names = missing
        .iter()
        .map(ContextDimensionName::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    bevy::log::warn!(
        "Keymap state dimensions [{names}] are unavailable, so keymap routing is disabled. \
         Initialize and own every required State<C> before adding KeymapPlugin."
    );
}

fn state_dimension_diagnostic(
    registration: &StateDimensionRegistration,
    message: &str,
) -> Diagnostic {
    Diagnostic {
        origin:             DiagnosticOrigin::ContextRegistration,
        byte_range:         0..0,
        line:               0,
        column:             0,
        block_index:        0,
        context:            if registration.name.is_empty() {
            "state dimension".to_owned()
        } else {
            registration.name.clone()
        },
        original_keystroke: String::new(),
        command_id:         String::new(),
        kind:               DiagnosticKind::Context,
        severity:           DiagnosticSeverity::Failure,
        message:            message.to_owned(),
        suggestions:        Vec::new(),
    }
}

fn retain_state_dimension_diagnostics(app: &mut App, diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        bevy::log::error!("{}", diagnostic.message);
    }
    app.world_mut()
        .resource_mut::<KeymapLoadFailures>()
        .retained_diagnostics
        .extend(diagnostics.iter().cloned());
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy::ecs::change_detection::DetectChanges;
    use bevy::input::ButtonInput;
    use bevy::input::keyboard::KeyCode;
    use bevy::prelude::App;
    use bevy::prelude::AppTypeRegistry;
    use bevy::prelude::NextState;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::States;
    use bevy::reflect::FromReflect;
    use bevy::state::app::AppExtStates;
    use bevy::state::app::StatesPlugin;
    use strum::AsRefStr;
    use strum::EnumIter;
    use strum::EnumMessage;

    use super::ActiveKeymapContext;
    use super::ActiveKeymapContextState;
    use super::ContextDimensionName;
    use super::ContextSnapshot;
    use super::ContextValueName;
    use super::StateDimensionRegistrationValidity;
    use crate::CommandRegistry;
    use crate::EffectiveKeymapStatus;
    use crate::KeymapBindings;
    use crate::KeymapPlugin;
    use crate::PaletteBinding;
    use crate::keymap;
    use crate::keymap::CompiledKeymap;
    use crate::keymap::RoutingResetStep;
    use crate::keymap::RoutingResetTrace;
    use crate::query_command_palette;

    #[expect(
        dead_code,
        reason = "`command!` declares an Enhanced Input action, while this test exercises only its reflected command event"
    )]
    mod dimension_dispatch_command {
        use bevy::prelude::Event;
        use bevy::prelude::Reflect;
        use bevy::prelude::ReflectEvent;
        use bevy_enhanced_input::prelude::InputAction;

        use crate::ReflectKeymapCommand;

        crate::command! {
            action:      DimensionDispatchAction,
            event:       DimensionDispatch,
            id:          "dimension::dispatch",
            title:       "Dimension Dispatch",
            description: "Dispatches only while a complete typed snapshot is routable.",
        }
    }

    use dimension_dispatch_command::DimensionDispatch;

    use crate::AuthoredKeymapBindings;

    #[derive(Default, Resource)]
    struct DispatchCount(usize);

    #[derive(
        AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
    )]
    #[strum(serialize_all = "snake_case")]
    enum ApplicationState {
        #[default]
        #[strum(message = "The application is ready for keymap dispatch")]
        Ready,
        #[strum(message = "The application is still starting")]
        Starting,
    }

    #[derive(
        AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
    )]
    #[strum(serialize_all = "snake_case")]
    enum InvalidRegistrationState {
        #[default]
        #[strum(message = "This value is valid even though its dimension registration is not")]
        Present,
    }

    const DIMENSION_DEFAULTS: &str =
        r#"{ "bindings": [{ "bindings": { "g": "dimension::dispatch" } }] }"#;

    #[derive(Clone, Copy, Eq, PartialEq)]
    enum StateResourceAvailability {
        Present,
        Missing,
    }

    fn dimension_app(state_resource: StateResourceAvailability) -> App {
        let mut app = App::new();
        app.world_mut().insert_resource(AppTypeRegistry::default());
        app.world()
            .resource::<AppTypeRegistry>()
            .write()
            .register::<DimensionDispatch>();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<DispatchCount>()
            .init_resource::<RoutingResetTrace>()
            .add_plugins(StatesPlugin);
        if state_resource == StateResourceAvailability::Present {
            app.insert_state(ApplicationState::Ready);
        }
        app.add_plugins(
            KeymapPlugin::new()
                .with_defaults(DIMENSION_DEFAULTS)
                .with_state_dimension::<ApplicationState>("application"),
        );
        app.world_mut().add_observer(
            |_: On<DimensionDispatch>, mut dispatch_count: ResMut<DispatchCount>| {
                dispatch_count.0 += 1;
            },
        );
        app.finish();
        app
    }

    fn assert_palette_context_unavailable(app: &App) {
        let world = app.world();
        let query_result = query_command_palette(
            world.resource::<CommandRegistry>(),
            world.resource::<ActiveKeymapContext>(),
            world.resource::<EffectiveKeymapStatus>(),
            world.resource::<KeymapBindings>(),
            "dimension dispatch",
        );

        assert_eq!(query_result.rows().len(), 1);
        assert_eq!(
            query_result.rows()[0].binding(),
            PaletteBinding::UnmaterializableStateDimensions
        );
    }

    #[test]
    fn awaiting_and_missing_dimensions_never_route_the_global_base() {
        let mut app = dimension_app(StateResourceAvailability::Missing);
        assert_eq!(
            app.world().resource::<ActiveKeymapContext>().state(),
            &ActiveKeymapContextState::AwaitingStateDimensions
        );
        assert!(!app.world().contains_resource::<CompiledKeymap>());

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        keymap::route_input(app.world_mut());
        assert_eq!(app.world().resource::<DispatchCount>().0, 0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::KeyG);

        app.update();
        assert!(matches!(
            app.world().resource::<ActiveKeymapContext>().state(),
            ActiveKeymapContextState::StateDimensionsUnavailable { missing }
                if missing == &[ContextDimensionName::new("application")]
        ));
        assert_eq!(app.world().resource::<DispatchCount>().0, 0);

        app.insert_state(ApplicationState::Ready);
        app.update();
        app.update();
        assert!(matches!(
            app.world().resource::<ActiveKeymapContext>().state(),
            ActiveKeymapContextState::Resolved(_)
        ));
        assert_eq!(app.world().resource::<DispatchCount>().0, 0);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyG);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_released(KeyCode::KeyG);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        app.update();
        assert_eq!(app.world().resource::<DispatchCount>().0, 1);
    }

    #[test]
    fn reflected_unmaterializable_snapshot_is_inactive_without_global_fallback()
    -> Result<(), String> {
        let mut app = dimension_app(StateResourceAvailability::Present);
        app.update();
        let retained_generation = app.world().resource::<CompiledKeymap>().generation();
        let retained_bindings_generation = match app.world().resource::<KeymapBindings>().authored()
        {
            AuthoredKeymapBindings::Loaded(bindings) => bindings.generation(),
            AuthoredKeymapBindings::Unavailable(_) => {
                return Err(String::from("resolved fixture did not publish bindings"));
            },
        };
        app.world_mut()
            .resource_mut::<RoutingResetTrace>()
            .0
            .clear();

        let invalid_context = ActiveKeymapContext {
            state: ActiveKeymapContextState::Resolved(ContextSnapshot::from_staged_values(
                BTreeMap::from([(
                    ContextDimensionName::new("application"),
                    ContextValueName::new("unknown"),
                )]),
            )),
        };
        let reflected_context = ActiveKeymapContext::from_reflect(&invalid_context)
            .ok_or_else(|| String::from("active context did not rebuild through reflection"))?;
        app.world_mut().insert_resource(reflected_context);
        keymap::commit_effective_keymap(app.world_mut());

        assert_eq!(
            app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::UnmaterializableStateDimensions
        );
        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation(),
            retained_generation
        );
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings)
                if bindings.generation() == retained_bindings_generation
        ));
        assert_palette_context_unavailable(&app);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        keymap::route_input(app.world_mut());
        assert_eq!(app.world().resource::<DispatchCount>().0, 0);
        assert_eq!(
            app.world().resource::<RoutingResetTrace>().0,
            vec![
                RoutingResetStep::PendingSequenceCancelled,
                RoutingResetStep::PhysicalSourcesReleased,
                RoutingResetStep::RoutingStateRecorded,
                RoutingResetStep::PressedKeysInhibited,
            ]
        );
        keymap::route_input(app.world_mut());
        assert_eq!(
            app.world().resource::<RoutingResetTrace>().0.len(),
            4,
            "the same inactive snapshot must not reset routing twice"
        );
        Ok(())
    }

    #[test]
    fn reflected_global_state_cannot_bypass_registered_dimensions() -> Result<(), String> {
        let mut app = dimension_app(StateResourceAvailability::Present);
        app.update();
        let retained_generation = app.world().resource::<CompiledKeymap>().generation();
        let retained_bindings = match app.world().resource::<KeymapBindings>().authored() {
            AuthoredKeymapBindings::Loaded(bindings) => bindings.global_bindings_for_test(),
            AuthoredKeymapBindings::Unavailable(_) => {
                return Err(String::from("resolved fixture did not publish bindings"));
            },
        };
        let bindings_tick = app.world().resource_ref::<KeymapBindings>().last_changed();
        app.world_mut()
            .resource_mut::<RoutingResetTrace>()
            .0
            .clear();

        let forged_global = ActiveKeymapContext {
            state: ActiveKeymapContextState::GlobalRouting,
        };
        let reflected_context =
            ActiveKeymapContext::from_reflect(&forged_global).ok_or_else(|| {
                String::from("global active context did not rebuild through reflection")
            })?;
        app.world_mut().insert_resource(reflected_context);
        keymap::commit_effective_keymap(app.world_mut());

        assert_eq!(
            app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::UnmaterializableStateDimensions
        );
        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation(),
            retained_generation
        );
        assert!(matches!(
            app.world().resource::<KeymapBindings>().authored(),
            crate::AuthoredKeymapBindings::Loaded(bindings)
                if bindings.global_bindings_for_test() == retained_bindings
        ));
        assert_palette_context_unavailable(&app);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        keymap::route_input(app.world_mut());
        assert_eq!(app.world().resource::<DispatchCount>().0, 0);
        assert_eq!(
            app.world().resource::<RoutingResetTrace>().0,
            vec![
                RoutingResetStep::PendingSequenceCancelled,
                RoutingResetStep::PhysicalSourcesReleased,
                RoutingResetStep::RoutingStateRecorded,
                RoutingResetStep::PressedKeysInhibited,
            ]
        );
        let compiled_tick = app.world().resource_ref::<CompiledKeymap>().last_changed();

        let status_tick = app
            .world()
            .resource_ref::<EffectiveKeymapStatus>()
            .last_changed();
        let allocations_before = crate::TEST_ALLOCATOR.allocation_count();
        keymap::commit_effective_keymap(app.world_mut());
        let allocations_after = crate::TEST_ALLOCATOR.allocation_count();
        keymap::route_input(app.world_mut());

        assert_eq!(allocations_after - allocations_before, 0);
        assert_eq!(
            app.world()
                .resource_ref::<EffectiveKeymapStatus>()
                .last_changed(),
            status_tick,
            "an unchanged forged global state must not republish its invalid status"
        );
        assert_eq!(
            app.world().resource_ref::<CompiledKeymap>().last_changed(),
            compiled_tick
        );
        assert_eq!(
            app.world().resource_ref::<KeymapBindings>().last_changed(),
            bindings_tick
        );
        assert_eq!(
            app.world().resource::<RoutingResetTrace>().0.len(),
            4,
            "an unchanged forged global state must not reset routing twice"
        );
        Ok(())
    }

    #[test]
    fn failed_later_state_dimension_registration_permanently_disables_collection() {
        let mut app = App::new();
        app.world_mut().insert_resource(AppTypeRegistry::default());
        app.world()
            .resource::<AppTypeRegistry>()
            .write()
            .register::<DimensionDispatch>();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<DispatchCount>()
            .init_resource::<RoutingResetTrace>()
            .add_plugins(StatesPlugin)
            .insert_state(ApplicationState::Ready)
            .insert_state(InvalidRegistrationState::Present)
            .add_plugins(
                KeymapPlugin::new()
                    .with_defaults(DIMENSION_DEFAULTS)
                    .with_state_dimension::<ApplicationState>("application"),
            )
            .add_plugins(
                KeymapPlugin::new()
                    .with_defaults(DIMENSION_DEFAULTS)
                    .with_state_dimension::<InvalidRegistrationState>(""),
            );
        app.world_mut().add_observer(
            |_: On<DimensionDispatch>, mut dispatch_count: ResMut<DispatchCount>| {
                dispatch_count.0 += 1;
            },
        );
        app.finish();

        assert!(matches!(
            app.world().resource::<StateDimensionRegistrationValidity>(),
            StateDimensionRegistrationValidity::PermanentlyInvalid
        ));
        assert_eq!(
            app.world().resource::<ActiveKeymapContext>().state(),
            &ActiveKeymapContextState::AwaitingStateDimensions
        );
        assert_eq!(
            app.world().resource::<EffectiveKeymapStatus>(),
            &EffectiveKeymapStatus::AwaitingStateDimensions
        );
        assert!(
            app.world()
                .resource::<crate::KeymapLoadFailures>()
                .retained_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.origin == crate::DiagnosticOrigin::ContextRegistration)
        );
        assert!(!app.world().contains_resource::<CompiledKeymap>());

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        app.update();
        assert_eq!(app.world().resource::<DispatchCount>().0, 0);
        assert_eq!(
            app.world().resource::<ActiveKeymapContext>().state(),
            &ActiveKeymapContextState::AwaitingStateDimensions
        );
        let status_tick = app
            .world()
            .resource_ref::<EffectiveKeymapStatus>()
            .last_changed();
        let bindings_tick = app.world().resource_ref::<KeymapBindings>().last_changed();
        let reset_steps = app.world().resource::<RoutingResetTrace>().0.clone();
        assert_eq!(reset_steps.len(), 4);

        app.update();

        assert_eq!(app.world().resource::<DispatchCount>().0, 0);
        assert_eq!(
            app.world()
                .resource_ref::<EffectiveKeymapStatus>()
                .last_changed(),
            status_tick
        );
        assert_eq!(
            app.world().resource_ref::<KeymapBindings>().last_changed(),
            bindings_tick
        );
        assert_eq!(app.world().resource::<RoutingResetTrace>().0, reset_steps);
    }

    #[test]
    fn resolved_typed_state_change_publishes_a_new_effective_generation() {
        let mut app = dimension_app(StateResourceAvailability::Present);
        app.update();
        let ready_generation = app.world().resource::<CompiledKeymap>().generation();

        app.world_mut()
            .resource_mut::<NextState<ApplicationState>>()
            .set(ApplicationState::Starting);
        app.update();
        app.update();

        assert!(matches!(
            app.world().resource::<ActiveKeymapContext>().state(),
            ActiveKeymapContextState::Resolved(snapshot)
                if snapshot.values().any(|(dimension, value)| {
                    dimension.as_str() == "application" && value.as_str() == "starting"
                })
        ));
        assert_eq!(
            app.world().resource::<CompiledKeymap>().generation(),
            ready_generation.next()
        );
    }
}
