use bevy::prelude::Added;
use bevy::prelude::App;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::EntityWorldMut;
use bevy::prelude::Event;
use bevy::prelude::Has;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Message;
use bevy::prelude::MessageReader;
use bevy::prelude::MessageWriter;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy_enhanced_input::prelude::ActionOf;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ContextActivity;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::EnhancedInputSystems;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputContextAppExt;
use bevy_enhanced_input::prelude::InputModKeys;
use bevy_enhanced_input::prelude::ModKeys;
use bevy_enhanced_input::prelude::Start;
use hana_rubric::Keybindings;
use thiserror::Error;

use super::PanelWidget;
use super::PanelWidgets;
use super::WidgetDisabled;
use super::WidgetFocusAuthority;
use super::WidgetFocusable;
use super::WidgetKind;
use super::WidgetOf;
use super::WidgetSystems;
use super::focus;
use super::focus::WidgetTraversal;
use crate::ime::ImeInputBlocker;
use crate::ime::ImeSystemSet;
use crate::panel::ComputedDiegeticPanel;

/// Opt-in enhanced-input adapter for window-scoped widget controls.
pub struct WidgetInputPlugin;

impl Plugin for WidgetInputPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EnhancedInputPlugin>() {
            app.add_plugins(EnhancedInputPlugin);
        }
        app.add_input_context::<WidgetInputContext>()
            .init_resource::<PendingWidgetInputActions>()
            .add_message::<WidgetInput>()
            .configure_sets(
                PreUpdate,
                (
                    WidgetInputSystems::Reconcile,
                    WidgetInputSystems::ActivateContexts,
                )
                    .chain()
                    .before(EnhancedInputSystems::Prepare),
            )
            .configure_sets(
                Update,
                WidgetInputSystems::EmitInput
                    .after(ImeSystemSet::PublishInputBlockers)
                    .before(WidgetSystems::SemanticInput),
            )
            .add_systems(
                PreUpdate,
                (install_default_modes, reconcile_input_installations)
                    .chain()
                    .in_set(WidgetInputSystems::Reconcile),
            )
            .add_systems(
                PreUpdate,
                synchronize_context_activity.in_set(WidgetInputSystems::ActivateContexts),
            )
            .add_systems(
                Update,
                emit_widget_input.in_set(WidgetInputSystems::EmitInput),
            )
            .add_observer(record_action_start::<NextWidgetAction>)
            .add_observer(record_action_start::<PreviousWidgetAction>)
            .add_observer(record_action_start::<FirstWidgetAction>)
            .add_observer(record_action_start::<LastWidgetAction>)
            .add_observer(record_action_start::<ActivateWidgetAction>)
            .add_observer(record_action_start::<CancelWidgetAction>);
    }
}

/// Per-window source of desired widget input configuration.
#[derive(Component, Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Component, Default)]
#[require(WidgetInputModeInitialized)]
pub enum WidgetInputMode {
    /// Installs Tab, Shift+Tab, Enter or Space, and Escape.
    #[default]
    Default,
    /// Installs the supplied bindings for this window.
    Bindings(WidgetInputBindings),
    /// Leaves input context ownership to the application.
    Manual,
}

impl WidgetInputMode {
    /// Returns display labels for the controls selected by this mode.
    #[must_use]
    pub fn control_summary(&self) -> WidgetControlSummary {
        match self {
            Self::Default => WidgetInputBindings::default().control_summary(),
            Self::Bindings(bindings) => bindings.control_summary(),
            Self::Manual => WidgetControlSummary::default(),
        }
    }

    fn bindings(&self) -> Option<WidgetInputBindings> {
        match self {
            Self::Default => Some(WidgetInputBindings::default()),
            Self::Bindings(bindings) => Some(bindings.clone()),
            Self::Manual => None,
        }
    }
}

/// Presence marker that temporarily disables Hana-owned widget input for a window.
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct WidgetInputDisabled;

/// Validated per-action bindings used by [`WidgetInputMode::Bindings`].
#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct WidgetInputBindings {
    next:     Vec<Binding>,
    previous: Vec<Binding>,
    first:    Vec<Binding>,
    last:     Vec<Binding>,
    activate: Vec<Binding>,
    cancel:   Vec<Binding>,
}

impl WidgetInputBindings {
    /// Starts a custom widget-input binding definition.
    #[must_use]
    pub fn builder() -> WidgetInputBindingsBuilder { WidgetInputBindingsBuilder::default() }

    fn control_summary(&self) -> WidgetControlSummary {
        WidgetControlSummary {
            next:     binding_labels(&self.next),
            previous: binding_labels(&self.previous),
            first:    binding_labels(&self.first),
            last:     binding_labels(&self.last),
            activate: binding_labels(&self.activate),
            cancel:   binding_labels(&self.cancel),
        }
    }

    #[cfg(test)]
    fn for_action(&self, action: WidgetInputAction) -> &[Binding] {
        match action {
            WidgetInputAction::Next => &self.next,
            WidgetInputAction::Previous => &self.previous,
            WidgetInputAction::First => &self.first,
            WidgetInputAction::Last => &self.last,
            WidgetInputAction::Activate => &self.activate,
            WidgetInputAction::Cancel => &self.cancel,
        }
    }
}

impl Default for WidgetInputBindings {
    fn default() -> Self {
        Self {
            next:     vec![KeyCode::Tab.into()],
            previous: vec![KeyCode::Tab.with_mod_keys(ModKeys::SHIFT)],
            first:    Vec::new(),
            last:     Vec::new(),
            activate: vec![KeyCode::Enter.into(), KeyCode::Space.into()],
            cancel:   vec![KeyCode::Escape.into()],
        }
    }
}

/// Consuming builder for validated per-window widget bindings.
#[derive(Clone, Debug, Default)]
pub struct WidgetInputBindingsBuilder {
    bindings: WidgetInputBindingsBuilderFields,
}

impl WidgetInputBindingsBuilder {
    /// Adds a binding alternative for focus-next.
    #[must_use]
    pub fn next(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.next.push(binding.into());
        self
    }

    /// Adds a binding alternative for focus-previous.
    #[must_use]
    pub fn previous(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.previous.push(binding.into());
        self
    }

    /// Adds a binding alternative for focus-first.
    #[must_use]
    pub fn first(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.first.push(binding.into());
        self
    }

    /// Adds a binding alternative for focus-last.
    #[must_use]
    pub fn last(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.last.push(binding.into());
        self
    }

    /// Adds a binding alternative for activation.
    #[must_use]
    pub fn activate(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.activate.push(binding.into());
        self
    }

    /// Adds a binding alternative for cancellation.
    #[must_use]
    pub fn cancel(mut self, binding: impl Into<Binding>) -> Self {
        self.bindings.cancel.push(binding.into());
        self
    }

    /// Validates and returns the custom binding collection.
    ///
    /// # Errors
    ///
    /// Returns [`WidgetInputBindingsError::NoneBinding`] for [`Binding::None`]
    /// or [`WidgetInputBindingsError::ConflictingBinding`] when two actions use
    /// the same exact binding.
    pub fn build(self) -> Result<WidgetInputBindings, WidgetInputBindingsError> {
        let mut bindings = WidgetInputBindings {
            next:     self.bindings.next,
            previous: self.bindings.previous,
            first:    self.bindings.first,
            last:     self.bindings.last,
            activate: self.bindings.activate,
            cancel:   self.bindings.cancel,
        };
        let mut assigned = Vec::new();
        validate_action_bindings(&mut bindings.next, &mut assigned)?;
        validate_action_bindings(&mut bindings.previous, &mut assigned)?;
        validate_action_bindings(&mut bindings.first, &mut assigned)?;
        validate_action_bindings(&mut bindings.last, &mut assigned)?;
        validate_action_bindings(&mut bindings.activate, &mut assigned)?;
        validate_action_bindings(&mut bindings.cancel, &mut assigned)?;
        Ok(bindings)
    }
}

#[derive(Clone, Debug, Default)]
struct WidgetInputBindingsBuilderFields {
    next:     Vec<Binding>,
    previous: Vec<Binding>,
    first:    Vec<Binding>,
    last:     Vec<Binding>,
    activate: Vec<Binding>,
    cancel:   Vec<Binding>,
}

/// Validation error from [`WidgetInputBindingsBuilder::build`].
#[derive(Clone, Debug, Error, PartialEq)]
pub enum WidgetInputBindingsError {
    /// A binding was the non-input sentinel.
    #[error("widget input bindings cannot contain Binding::None")]
    NoneBinding,
    /// One exact binding was assigned to more than one semantic action.
    #[error("widget input binding `{0}` is assigned to multiple actions")]
    ConflictingBinding(Binding),
}

/// Point-in-time labels for one window's effective widget controls.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WidgetControlSummary {
    /// Focus-next binding labels.
    pub next:     Vec<String>,
    /// Focus-previous binding labels.
    pub previous: Vec<String>,
    /// Focus-first binding labels.
    pub first:    Vec<String>,
    /// Focus-last binding labels.
    pub last:     Vec<String>,
    /// Activation binding labels.
    pub activate: Vec<String>,
    /// Cancellation binding labels.
    pub cancel:   Vec<String>,
}

#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum WidgetInputSystems {
    Reconcile,
    ActivateContexts,
    EmitInput,
}

#[derive(Component, Clone, Copy, Debug, Default)]
struct WidgetInputModeInitialized;

#[derive(Component, Clone, Copy, Debug, Default)]
struct WidgetInputContext;

#[derive(Component, Clone, Debug, PartialEq)]
struct WidgetInputInstallation {
    bindings: Option<WidgetInputBindings>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WidgetInputAdapterState {
    Enabled,
    Disabled,
}

impl From<bool> for WidgetInputAdapterState {
    fn from(disabled: bool) -> Self {
        if disabled {
            Self::Disabled
        } else {
            Self::Enabled
        }
    }
}

#[derive(Component, Clone, Copy, Debug, Eq, PartialEq)]
enum WidgetInputAction {
    Next,
    Previous,
    First,
    Last,
    Activate,
    Cancel,
}

impl WidgetInputAction {
    #[cfg(test)]
    const ALL: [Self; 6] = [
        Self::Next,
        Self::Previous,
        Self::First,
        Self::Last,
        Self::Activate,
        Self::Cancel,
    ];
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct InstalledWidgetInputBinding(Binding);

#[derive(Default, Resource)]
struct PendingWidgetInputActions {
    actions: Vec<(Entity, WidgetInputAction)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
struct NextWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
struct PreviousWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
struct FirstWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
struct LastWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
struct ActivateWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
struct CancelWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
struct WidgetShiftModifierAction;

fn record_action_start<A: InputAction>(
    action: On<Start<A>>,
    actions: Query<&WidgetInputAction>,
    mut pending: ResMut<PendingWidgetInputActions>,
) {
    let Ok(&widget_action) = actions.get(action.action) else {
        return;
    };
    let pending_action = (action.context, widget_action);
    if !pending.actions.contains(&pending_action) {
        pending.actions.push(pending_action);
    }
}

fn validate_action_bindings(
    bindings: &mut Vec<Binding>,
    assigned: &mut Vec<Binding>,
) -> Result<(), WidgetInputBindingsError> {
    let mut unique = Vec::with_capacity(bindings.len());
    for binding in bindings.drain(..) {
        if binding == Binding::None {
            return Err(WidgetInputBindingsError::NoneBinding);
        }
        if unique.contains(&binding) {
            continue;
        }
        if assigned.contains(&binding) {
            return Err(WidgetInputBindingsError::ConflictingBinding(binding));
        }
        assigned.push(binding);
        unique.push(binding);
    }
    *bindings = unique;
    Ok(())
}

fn binding_labels(bindings: &[Binding]) -> Vec<String> {
    bindings.iter().map(ToString::to_string).collect()
}

fn install_default_modes(
    windows: Query<Entity, (With<Window>, Without<WidgetInputModeInitialized>)>,
    mut commands: Commands,
) {
    for window in windows.iter() {
        commands
            .entity(window)
            .insert((WidgetInputMode::Default, WidgetInputModeInitialized));
    }
}

fn reconcile_input_installations(
    changed: Query<
        (
            Entity,
            &WidgetInputMode,
            Has<WidgetInputDisabled>,
            Option<&WidgetInputInstallation>,
        ),
        (
            With<Window>,
            Or<(
                Changed<WidgetInputMode>,
                Added<WidgetInputDisabled>,
                Without<WidgetInputInstallation>,
            )>,
        ),
    >,
    modes: Query<
        (
            &WidgetInputMode,
            Has<WidgetInputDisabled>,
            Option<&WidgetInputInstallation>,
        ),
        With<Window>,
    >,
    windows: Query<(), With<Window>>,
    mut removed_disabled: RemovedComponents<WidgetInputDisabled>,
    mut removed_modes: RemovedComponents<WidgetInputMode>,
    mut removed_windows: RemovedComponents<Window>,
    mut commands: Commands,
) {
    for (window, mode, disabled, installation) in changed.iter() {
        let state: WidgetInputAdapterState = disabled.into();
        reconcile_window_input(window, mode, state, installation, &mut commands);
    }
    for window in removed_disabled.read() {
        let Ok((mode, disabled, installation)) = modes.get(window) else {
            continue;
        };
        let state: WidgetInputAdapterState = disabled.into();
        reconcile_window_input(window, mode, state, installation, &mut commands);
    }
    for window in removed_modes.read() {
        if modes.get(window).is_err() {
            remove_window_input(window, &mut commands);
        }
    }
    for window in removed_windows.read() {
        if windows.get(window).is_err() {
            remove_window_input(window, &mut commands);
        }
    }
}

fn reconcile_window_input(
    window: Entity,
    mode: &WidgetInputMode,
    state: WidgetInputAdapterState,
    installation: Option<&WidgetInputInstallation>,
    commands: &mut Commands<'_, '_>,
) {
    let desired_bindings = match state {
        WidgetInputAdapterState::Enabled => mode.bindings(),
        WidgetInputAdapterState::Disabled => None,
    };
    if installation.is_some_and(|installed| installed.bindings == desired_bindings) {
        return;
    }

    let Ok(mut window_commands) = commands.get_entity(window) else {
        return;
    };
    window_commands
        .despawn_related::<Actions<WidgetInputContext>>()
        .remove_with_requires::<WidgetInputContext>()
        .insert(WidgetInputInstallation {
            bindings: desired_bindings.clone(),
        });

    let Some(bindings) = desired_bindings else {
        return;
    };
    window_commands.queue(move |mut window_entity: EntityWorldMut<'_>| {
        window_entity.insert(WidgetInputContext);
        window_entity.with_related_entities::<ActionOf<WidgetInputContext>>(|spawner| {
            spawn_widget_input_actions(spawner, bindings);
        });
    });
}

fn remove_window_input(window: Entity, commands: &mut Commands<'_, '_>) {
    let Ok(mut window_commands) = commands.get_entity(window) else {
        return;
    };
    window_commands
        .despawn_related::<Actions<WidgetInputContext>>()
        .remove_with_requires::<WidgetInputContext>()
        .remove::<WidgetInputInstallation>();
}

fn spawn_widget_input_actions(
    spawner: &mut ActionSpawner<WidgetInputContext>,
    bindings: WidgetInputBindings,
) {
    let keybindings = Keybindings::new::<WidgetShiftModifierAction>(
        spawner,
        ActionSettings {
            require_reset: true,
            consume_input: true,
            ..default()
        },
    );
    spawn_widget_input_actions_for::<PreviousWidgetAction>(
        spawner,
        &keybindings,
        WidgetInputAction::Previous,
        bindings.previous,
    );
    spawn_widget_input_actions_for::<NextWidgetAction>(
        spawner,
        &keybindings,
        WidgetInputAction::Next,
        bindings.next,
    );
    spawn_widget_input_actions_for::<FirstWidgetAction>(
        spawner,
        &keybindings,
        WidgetInputAction::First,
        bindings.first,
    );
    spawn_widget_input_actions_for::<LastWidgetAction>(
        spawner,
        &keybindings,
        WidgetInputAction::Last,
        bindings.last,
    );
    spawn_widget_input_actions_for::<ActivateWidgetAction>(
        spawner,
        &keybindings,
        WidgetInputAction::Activate,
        bindings.activate,
    );
    spawn_widget_input_actions_for::<CancelWidgetAction>(
        spawner,
        &keybindings,
        WidgetInputAction::Cancel,
        bindings.cancel,
    );
}

fn spawn_widget_input_actions_for<A: InputAction>(
    spawner: &mut ActionSpawner<WidgetInputContext>,
    keybindings: &Keybindings<WidgetInputContext>,
    action: WidgetInputAction,
    bindings: Vec<Binding>,
) {
    for binding in bindings {
        let action_entity = keybindings.spawn_shortcut::<A>(spawner, binding);
        spawner
            .world_mut()
            .entity_mut(action_entity)
            .insert((action, InstalledWidgetInputBinding(binding)));
    }
}

fn synchronize_context_activity(
    windows: Query<(Entity, &Window)>,
    contexts: Query<(Entity, &ContextActivity<WidgetInputContext>)>,
    mut commands: Commands,
) {
    let mut focused_windows = windows
        .iter()
        .filter_map(|(window, settings)| settings.focused.then_some(window));
    let focused_window = focused_windows
        .next()
        .filter(|_| focused_windows.next().is_none());

    for (window, activity) in contexts.iter() {
        let should_be_active = focused_window == Some(window);
        if **activity != should_be_active {
            commands
                .entity(window)
                .insert(ContextActivity::<WidgetInputContext>::new(should_be_active));
        }
    }
}

fn emit_widget_input(
    mut pending: ResMut<PendingWidgetInputActions>,
    mut widget_input: MessageWriter<WidgetInput>,
) {
    for (window, action) in pending.actions.drain(..) {
        widget_input.write(match action {
            WidgetInputAction::Next => WidgetInput::FocusNext { window },
            WidgetInputAction::Previous => WidgetInput::FocusPrevious { window },
            WidgetInputAction::First => WidgetInput::FocusFirst { window },
            WidgetInputAction::Last => WidgetInput::FocusLast { window },
            WidgetInputAction::Activate => WidgetInput::Activate { window },
            WidgetInputAction::Cancel => WidgetInput::Cancel { window },
        });
    }
}

/// Requests one operation from the widget currently associated with a window.
///
/// `WidgetInput` is buffered so every producer feeds one ordered stream that
/// `route_semantic_input` processes after [`ImeSystemSet::PublishInputBlockers`].
/// Applications may write these requests directly or add [`WidgetInputPlugin`]
/// to translate configured Bevy Enhanced Input bindings into them.
#[derive(Clone, Copy, Debug, Message)]
pub enum WidgetInput {
    /// Focuses the next widget in the window's active panel.
    FocusNext {
        /// Window whose active panel should be traversed.
        window: Entity,
    },
    /// Focuses the previous widget in the window's active panel.
    FocusPrevious {
        /// Window whose active panel should be traversed.
        window: Entity,
    },
    /// Focuses the first widget in the window's active panel.
    FocusFirst {
        /// Window whose active panel should be traversed.
        window: Entity,
    },
    /// Focuses the last widget in the window's active panel.
    FocusLast {
        /// Window whose active panel should be traversed.
        window: Entity,
    },
    /// Activates the focused widget in the window.
    Activate {
        /// Window whose focused widget should receive activation.
        window: Entity,
    },
    /// Cancels the focused widget's active interaction in the window.
    Cancel {
        /// Window whose focused widget should receive cancellation.
        window: Entity,
    },
}

impl WidgetInput {
    /// Returns the window whose widget focus scope receives this request.
    #[must_use]
    pub const fn window(&self) -> Entity {
        match *self {
            Self::FocusNext { window }
            | Self::FocusPrevious { window }
            | Self::FocusFirst { window }
            | Self::FocusLast { window }
            | Self::Activate { window }
            | Self::Cancel { window } => window,
        }
    }
}

#[derive(Clone, Copy, Debug, Event)]
#[event(trigger = bevy::ecs::event::EntityTrigger)]
pub(crate) enum SemanticWidgetIntent {
    Activate { entity: Entity, window: Entity },
    Cancel { entity: Entity },
}

impl EntityEvent for SemanticWidgetIntent {
    fn event_target(&self) -> Entity {
        match *self {
            Self::Activate { entity, .. } | Self::Cancel { entity } => entity,
        }
    }
}

pub(super) fn route_semantic_input(
    mut widget_input: MessageReader<WidgetInput>,
    input_blocker: Option<Res<ImeInputBlocker>>,
    mut authority: ResMut<WidgetFocusAuthority>,
    panels: Query<(&ComputedDiegeticPanel, &PanelWidgets)>,
    focusable_widgets: Query<(&PanelWidget, &WidgetOf), With<WidgetFocusable>>,
    enabled_widgets: Query<&WidgetKind, (With<PanelWidget>, Without<WidgetDisabled>)>,
    mut commands: Commands,
) {
    for input in widget_input.read() {
        match *input {
            WidgetInput::FocusNext { window } => route_traversal(
                window,
                WidgetTraversal::Next,
                input_blocker.as_deref(),
                &mut authority,
                &panels,
                &focusable_widgets,
                &enabled_widgets,
                &mut commands,
            ),
            WidgetInput::FocusPrevious { window } => route_traversal(
                window,
                WidgetTraversal::Previous,
                input_blocker.as_deref(),
                &mut authority,
                &panels,
                &focusable_widgets,
                &enabled_widgets,
                &mut commands,
            ),
            WidgetInput::FocusFirst { window } => route_traversal(
                window,
                WidgetTraversal::First,
                input_blocker.as_deref(),
                &mut authority,
                &panels,
                &focusable_widgets,
                &enabled_widgets,
                &mut commands,
            ),
            WidgetInput::FocusLast { window } => route_traversal(
                window,
                WidgetTraversal::Last,
                input_blocker.as_deref(),
                &mut authority,
                &panels,
                &focusable_widgets,
                &enabled_widgets,
                &mut commands,
            ),
            WidgetInput::Activate { window } => route_activate_intent(
                window,
                input_blocker.as_deref(),
                &authority,
                &enabled_widgets,
                &mut commands,
            ),
            WidgetInput::Cancel { window } => route_cancel_intent(
                window,
                input_blocker.as_deref(),
                &authority,
                &enabled_widgets,
                &mut commands,
            ),
        }
    }
}

fn route_traversal(
    window: Entity,
    traversal: WidgetTraversal,
    input_blocker: Option<&ImeInputBlocker>,
    authority: &mut WidgetFocusAuthority,
    panels: &Query<(&ComputedDiegeticPanel, &PanelWidgets)>,
    focusable_widgets: &Query<(&PanelWidget, &WidgetOf), With<WidgetFocusable>>,
    enabled_widgets: &Query<&WidgetKind, (With<PanelWidget>, Without<WidgetDisabled>)>,
    commands: &mut Commands<'_, '_>,
) {
    if input_blocker.is_some_and(|blocker| blocker.blocks_window(window)) {
        return;
    }
    let previous = authority.focused_widget(window);
    focus::traverse_focus(
        window,
        traversal,
        authority,
        panels,
        focusable_widgets,
        commands,
    );
    let current = authority.focused_widget(window);
    if current == previous {
        return;
    }
    let Some(widget) = current else {
        return;
    };
    if !matches!(enabled_widgets.get(widget), Ok(WidgetKind::EditableField)) {
        return;
    }
    commands.trigger(SemanticWidgetIntent::Activate {
        entity: widget,
        window,
    });
}

fn route_activate_intent(
    window: Entity,
    input_blocker: Option<&ImeInputBlocker>,
    authority: &WidgetFocusAuthority,
    enabled_widgets: &Query<&WidgetKind, (With<PanelWidget>, Without<WidgetDisabled>)>,
    commands: &mut Commands<'_, '_>,
) {
    if input_blocker.is_some_and(|blocker| blocker.blocks_window(window)) {
        return;
    }
    let Some(widget) = authority.focused_widget(window) else {
        return;
    };
    if enabled_widgets.get(widget).is_err() {
        return;
    }
    commands.trigger(SemanticWidgetIntent::Activate {
        entity: widget,
        window,
    });
}

fn route_cancel_intent(
    window: Entity,
    input_blocker: Option<&ImeInputBlocker>,
    authority: &WidgetFocusAuthority,
    enabled_widgets: &Query<&WidgetKind, (With<PanelWidget>, Without<WidgetDisabled>)>,
    commands: &mut Commands<'_, '_>,
) {
    if input_blocker.is_some_and(|blocker| blocker.blocks_window(window)) {
        return;
    }
    let Some(widget) = authority.focused_widget(window) else {
        return;
    };
    if enabled_widgets.get(widget).is_err() {
        return;
    }
    commands.trigger(SemanticWidgetIntent::Cancel { entity: widget });
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::camera::RenderTarget;
    use bevy::diagnostic::FrameCount;
    use bevy::ecs::message::MessageCursor;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::gamepad::RawGamepadButtonChangedEvent;
    use bevy::input::gamepad::RawGamepadEvent;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Camera;
    use bevy::prelude::Commands;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::Event;
    use bevy::prelude::Gamepad;
    use bevy::prelude::GamepadButton;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MessageWriter;
    use bevy::prelude::Messages;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::Press;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy::prelude::RelationshipTarget;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::SpawnRelated;
    use bevy::prelude::SpawnWith;
    use bevy::prelude::Startup;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::prelude::default;
    use bevy::window::Ime;
    use bevy::window::WindowClosed;
    use bevy::window::WindowFocused;
    use bevy::window::WindowRef;
    use bevy_enhanced_input::prelude::ActionSettings;
    use bevy_enhanced_input::prelude::ActionSpawner;
    use bevy_enhanced_input::prelude::Actions;
    use bevy_enhanced_input::prelude::Binding;
    use bevy_enhanced_input::prelude::ContextActivity;
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use bevy_enhanced_input::prelude::InputAction;
    use bevy_enhanced_input::prelude::InputContextAppExt;
    use bevy_enhanced_input::prelude::InputModKeys;
    use bevy_enhanced_input::prelude::ModKeys;
    use hana_rubric::Keybindings;

    use super::InstalledWidgetInputBinding;
    use super::SemanticWidgetIntent;
    use super::WidgetInput;
    use super::WidgetInputAction;
    use super::WidgetInputBindings;
    use super::WidgetInputBindingsError;
    use super::WidgetInputContext;
    use super::WidgetInputDisabled;
    use super::WidgetInputMode;
    use super::WidgetInputPlugin;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeBufferSnapshot;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeCursorState;
    use crate::ImeEditableFieldSpec;
    use crate::ImeInputBlocker;
    use crate::ImeOpenSession;
    use crate::ImeSystemSet;
    use crate::ImeTarget;
    use crate::ImeTextChanged;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::PanelWidgetWriter;
    use crate::RequestPanelFocus;
    use crate::RequestWidgetFocus;
    use crate::WidgetDisabled;
    use crate::WidgetFocused;
    use crate::WidgetInteractivity;
    use crate::ime::ImePlugin;
    use crate::layout::LayoutTreeChange;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetFocusAuthority;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::focus::WidgetFocusVisible;

    const PANEL_HEIGHT: f32 = 50.0;
    const PANEL_WIDTH: f32 = 100.0;

    #[derive(Clone, Copy)]
    enum TestIme {
        Absent,
        Present,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedIntentAction {
        Activate,
        Cancel,
    }

    #[derive(Default, Resource)]
    struct RecordedIntents(Vec<(Entity, RecordedIntentAction)>);

    #[derive(Default, Resource)]
    struct ImeLeaseAfterInput(Option<Entity>);

    #[derive(Default, Resource)]
    struct RecordedImeTextChanges(Vec<ImeBufferSnapshot>);

    #[derive(Component)]
    struct AppOwnedInputContext;

    #[derive(Resource)]
    struct AppOwnedWindow(Entity);

    hana_rubric::action!(
        /// Test action owned by the application rather than `WidgetInputPlugin`.
        AppOwnedNextAction
    );

    hana_rubric::action!(
        /// Modifier action for the test application's keybindings.
        AppOwnedShiftAction
    );

    hana_rubric::event!(
        /// Test event that sends the core focus-next message.
        AppOwnedNextEvent
    );

    fn record_intent(intent: On<SemanticWidgetIntent>, mut recorded: ResMut<RecordedIntents>) {
        let entry = match *intent.event() {
            SemanticWidgetIntent::Activate { entity, .. } => {
                (entity, RecordedIntentAction::Activate)
            },
            SemanticWidgetIntent::Cancel { entity } => (entity, RecordedIntentAction::Cancel),
        };
        recorded.0.push(entry);
    }

    fn record_ime_lease_after_input(
        blocker: Res<ImeInputBlocker>,
        mut recorded: ResMut<ImeLeaseAfterInput>,
    ) {
        recorded.0 = blocker.window();
    }

    fn record_ime_text_change(
        change: On<ImeTextChanged>,
        mut changes: ResMut<RecordedImeTextChanges>,
    ) {
        changes.0.push(change.snapshot.clone());
    }

    fn test_app(test_ime: TestIme) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<RecordedIntents>()
            .init_resource::<RecordedImeTextChanges>()
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .add_observer(record_intent)
            .add_observer(record_ime_text_change);
        if matches!(test_ime, TestIme::Present) {
            app.add_message::<Ime>()
                .add_message::<KeyboardInput>()
                .add_message::<WindowClosed>()
                .add_message::<WindowFocused>()
                .init_resource::<ButtonInput<KeyCode>>()
                .add_plugins(ImePlugin);
        }
        app
    }

    fn spawn_panel(app: &mut App, ids: &[&str]) -> Entity {
        let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
        for id in ids {
            builder.with(El::new().button(*id), |_| {});
        }
        let result = DiegeticPanel::world()
            .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
            .with_tree(builder.build())
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        app.world_mut().spawn(panel).id()
    }

    fn spawn_editable_panel(app: &mut App) -> Entity {
        let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
        builder.with(
            El::new().editable_field(
                "editable",
                ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test")),
            ),
            |builder| {
                builder.text("Initial text");
            },
        );
        builder.with(El::new().button("next"), |_| {});
        let result = DiegeticPanel::world()
            .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
            .with_tree(builder.build())
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        app.world_mut().spawn(panel).id()
    }

    fn spawn_builtin_editable_panel(app: &mut App) -> (Entity, LayoutTree) {
        let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
        builder.with(
            El::new().editable_field(
                "editable",
                ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text)),
            ),
            |builder| {
                builder.text("Initial text");
            },
        );
        builder.with(El::new().button("next"), |_| {});
        let tree = builder.build();
        let result = DiegeticPanel::world()
            .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
            .with_tree(tree.clone())
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return (Entity::PLACEHOLDER, tree);
        };
        (app.world_mut().spawn(panel).id(), tree)
    }

    fn widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let Ok(widget) = result else {
            return Entity::PLACEHOLDER;
        };
        assert!(widget.is_some());
        widget.unwrap_or(Entity::PLACEHOLDER)
    }

    fn request_focus(app: &mut App, window: Entity, widget: Entity) {
        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
    }

    fn adapter_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, WidgetInputPlugin));
        app.finish();
        app
    }

    fn installed_actions(app: &App, window: Entity) -> Vec<Entity> {
        app.world()
            .get::<Actions<WidgetInputContext>>(window)
            .map_or_else(Vec::new, |actions| {
                actions
                    .iter()
                    .filter(|action| app.world().get::<WidgetInputAction>(*action).is_some())
                    .collect()
            })
    }

    fn context_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut contexts = world.query_filtered::<Entity, With<WidgetInputContext>>();
        contexts.iter(world).count()
    }

    fn action_count(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut actions = world.query_filtered::<Entity, With<WidgetInputAction>>();
        actions.iter(world).count()
    }

    fn binding_count(bindings: &WidgetInputBindings) -> usize {
        WidgetInputAction::ALL
            .into_iter()
            .map(|action| bindings.for_action(action).len())
            .sum()
    }

    fn default_action_count() -> usize { binding_count(&WidgetInputBindings::default()) }

    fn context_is_active(app: &App, window: Entity) -> bool {
        app.world()
            .get::<ContextActivity<WidgetInputContext>>(window)
            .is_some_and(|activity| **activity)
    }

    fn widget_input(app: &App) -> Vec<WidgetInput> {
        let mut cursor = MessageCursor::<WidgetInput>::default();
        cursor
            .read(app.world().resource::<Messages<WidgetInput>>())
            .copied()
            .collect()
    }

    fn next_requests(app: &App) -> Vec<WidgetInput> {
        widget_input(app)
            .into_iter()
            .filter(|input| matches!(input, WidgetInput::FocusNext { .. }))
            .collect()
    }

    fn previous_requests(app: &App) -> Vec<WidgetInput> {
        widget_input(app)
            .into_iter()
            .filter(|input| matches!(input, WidgetInput::FocusPrevious { .. }))
            .collect()
    }

    fn activate_requests(app: &App) -> Vec<WidgetInput> {
        widget_input(app)
            .into_iter()
            .filter(|input| matches!(input, WidgetInput::Activate { .. }))
            .collect()
    }

    fn clear_widget_input(app: &mut App) {
        app.world_mut()
            .resource_mut::<Messages<WidgetInput>>()
            .clear();
    }

    fn press_key(app: &mut App, window: Entity, key_code: KeyCode) {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
    }

    fn release_key(app: &mut App, window: Entity, key_code: KeyCode) {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Released,
            text: None,
            repeat: false,
            window,
        });
    }

    fn spawn_app_owned_input(mut commands: Commands) {
        commands.spawn((
            AppOwnedInputContext,
            Actions::<AppOwnedInputContext>::spawn(SpawnWith(spawn_app_owned_actions)),
        ));
    }

    fn spawn_app_owned_actions(spawner: &mut ActionSpawner<AppOwnedInputContext>) {
        let keybindings =
            Keybindings::new::<AppOwnedShiftAction>(spawner, ActionSettings::default());
        keybindings.spawn_key::<AppOwnedNextAction>(spawner, KeyCode::KeyN);
    }

    fn send_app_owned_next(
        window: Res<AppOwnedWindow>,
        mut widget_input: MessageWriter<WidgetInput>,
    ) {
        widget_input.write(WidgetInput::FocusNext { window: window.0 });
    }

    #[test]
    fn binding_builder_adds_alternatives_deduplicates_and_permits_omissions() {
        let result = WidgetInputBindings::builder()
            .next(KeyCode::Tab)
            .activate(KeyCode::Enter)
            .activate(KeyCode::Enter)
            .activate(KeyCode::Space)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };

        assert_eq!(bindings.next, vec![Binding::from(KeyCode::Tab)]);
        assert!(bindings.previous.is_empty());
        assert!(bindings.first.is_empty());
        assert!(bindings.last.is_empty());
        assert_eq!(
            bindings.activate,
            vec![Binding::from(KeyCode::Enter), Binding::from(KeyCode::Space),]
        );
        assert!(bindings.cancel.is_empty());
    }

    #[test]
    fn binding_builder_rejects_none_and_cross_action_conflicts() {
        assert_eq!(
            WidgetInputBindings::builder().next(Binding::None).build(),
            Err(WidgetInputBindingsError::NoneBinding)
        );

        let tab = Binding::from(KeyCode::Tab);
        assert_eq!(
            WidgetInputBindings::builder()
                .next(tab)
                .activate(tab)
                .build(),
            Err(WidgetInputBindingsError::ConflictingBinding(tab))
        );
    }

    #[test]
    fn binding_error_messages_are_stable() {
        assert_eq!(
            WidgetInputBindingsError::NoneBinding.to_string(),
            "widget input bindings cannot contain Binding::None"
        );
        assert_eq!(
            WidgetInputBindingsError::ConflictingBinding(KeyCode::Tab.into()).to_string(),
            "widget input binding `Tab` is assigned to multiple actions"
        );
    }

    #[test]
    fn control_summary_describes_defaults_custom_bindings_and_manual_mode() {
        let defaults = WidgetInputMode::Default.control_summary();
        assert_eq!(defaults.next, vec![Binding::from(KeyCode::Tab).to_string()]);
        assert_eq!(
            defaults.previous,
            vec![KeyCode::Tab.with_mod_keys(ModKeys::SHIFT).to_string()]
        );
        assert!(defaults.first.is_empty());
        assert!(defaults.last.is_empty());
        assert_eq!(
            defaults.activate,
            vec![
                Binding::from(KeyCode::Enter).to_string(),
                Binding::from(KeyCode::Space).to_string(),
            ]
        );

        let result = WidgetInputBindings::builder()
            .next(KeyCode::ArrowDown)
            .cancel(KeyCode::Backspace)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let custom = WidgetInputMode::Bindings(bindings).control_summary();
        assert_eq!(
            custom.next,
            vec![Binding::from(KeyCode::ArrowDown).to_string()]
        );
        assert_eq!(
            custom.cancel,
            vec![Binding::from(KeyCode::Backspace).to_string()]
        );
        assert!(custom.previous.is_empty());
        assert_eq!(
            WidgetInputMode::Manual.control_summary(),
            super::WidgetControlSummary::default()
        );
    }

    #[test]
    fn default_mode_installs_one_context_and_one_action_per_binding_for_each_window() {
        let mut app = adapter_app();
        let first = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let second = app
            .world_mut()
            .spawn(Window {
                focused: false,
                ..default()
            })
            .id();
        app.update();

        assert_eq!(context_count(&mut app), 2);
        assert_eq!(action_count(&mut app), default_action_count() * 2);
        assert!(matches!(
            app.world().get::<WidgetInputMode>(first),
            Some(WidgetInputMode::Default)
        ));
        assert!(matches!(
            app.world().get::<WidgetInputMode>(second),
            Some(WidgetInputMode::Default)
        ));
    }

    #[test]
    fn modes_reconcile_only_the_changed_window() {
        let mut app = adapter_app();
        let default_window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Default,
            ))
            .id();
        let result = WidgetInputBindings::builder()
            .next(KeyCode::ArrowDown)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let bound_action_count = binding_count(&bindings);
        let bound_window = app
            .world_mut()
            .spawn((Window::default(), WidgetInputMode::Bindings(bindings)))
            .id();
        let manual_window = app
            .world_mut()
            .spawn((Window::default(), WidgetInputMode::Manual))
            .id();
        app.update();

        let default_actions = installed_actions(&app, default_window);
        assert_eq!(default_actions.len(), default_action_count());
        assert_eq!(
            installed_actions(&app, bound_window).len(),
            bound_action_count
        );
        assert!(installed_actions(&app, manual_window).is_empty());

        app.world_mut()
            .entity_mut(bound_window)
            .insert(WidgetInputMode::Manual);
        app.update();

        assert_eq!(installed_actions(&app, default_window), default_actions);
        assert!(installed_actions(&app, bound_window).is_empty());
        assert!(installed_actions(&app, manual_window).is_empty());
    }

    #[test]
    fn equal_rebind_is_a_no_op_and_changed_rebind_has_no_duplicates() {
        let mut app = adapter_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), WidgetInputMode::Default))
            .id();
        app.update();
        let original_actions = installed_actions(&app, window);

        app.world_mut()
            .entity_mut(window)
            .insert(WidgetInputMode::Default);
        app.update();
        assert_eq!(installed_actions(&app, window), original_actions);

        let result = WidgetInputBindings::builder()
            .next(KeyCode::ArrowDown)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let rebound_action_count = binding_count(&bindings);
        app.world_mut()
            .entity_mut(window)
            .insert(WidgetInputMode::Bindings(bindings));
        app.update();

        let rebound_actions = installed_actions(&app, window);
        assert_eq!(rebound_actions.len(), rebound_action_count);
        assert_ne!(rebound_actions, original_actions);
        assert_eq!(action_count(&mut app), rebound_action_count);
    }

    #[test]
    fn disabled_and_removed_modes_remove_only_adapter_owned_input() {
        let mut app = adapter_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), WidgetInputMode::Default))
            .id();
        app.update();
        assert_eq!(
            installed_actions(&app, window).len(),
            default_action_count()
        );

        app.world_mut()
            .entity_mut(window)
            .insert(WidgetInputDisabled);
        app.update();
        assert!(installed_actions(&app, window).is_empty());
        assert!(matches!(
            app.world().get::<WidgetInputMode>(window),
            Some(WidgetInputMode::Default)
        ));

        app.world_mut()
            .entity_mut(window)
            .remove::<WidgetInputDisabled>();
        app.update();
        assert_eq!(
            installed_actions(&app, window).len(),
            default_action_count()
        );
        assert_eq!(action_count(&mut app), default_action_count());

        app.world_mut()
            .entity_mut(window)
            .remove::<WidgetInputMode>();
        app.update();
        assert!(installed_actions(&app, window).is_empty());
        assert!(app.world().get::<WidgetInputMode>(window).is_none());
        assert_eq!(action_count(&mut app), 0);

        app.world_mut()
            .entity_mut(window)
            .insert((Window::default(), WidgetInputMode::Default));
        app.update();
        assert_eq!(
            installed_actions(&app, window).len(),
            default_action_count()
        );
        app.world_mut().entity_mut(window).remove::<Window>();
        app.update();
        assert!(installed_actions(&app, window).is_empty());
        assert_eq!(action_count(&mut app), 0);
    }

    #[test]
    fn shift_tab_emits_previous_only_for_the_unique_focused_window() {
        let mut app = adapter_app();
        let focused_window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        app.world_mut().spawn(Window {
            focused: false,
            ..default()
        });
        app.update();
        assert!(
            app.world()
                .get::<ContextActivity<WidgetInputContext>>(focused_window)
                .is_some_and(|activity| **activity)
        );

        press_key(&mut app, focused_window, KeyCode::ShiftLeft);
        press_key(&mut app, focused_window, KeyCode::Tab);
        app.update();
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::Tab)
        );

        assert!(next_requests(&app).is_empty());
        assert_eq!(
            previous_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![focused_window]
        );

        clear_widget_input(&mut app);
        release_key(&mut app, focused_window, KeyCode::ShiftLeft);
        app.update();
        assert!(next_requests(&app).is_empty());
        assert!(previous_requests(&app).is_empty());
    }

    #[test]
    fn held_modifier_is_available_after_window_context_activation() {
        let mut app = adapter_app();
        let first_window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let second_window = app
            .world_mut()
            .spawn(Window {
                focused: false,
                ..default()
            })
            .id();
        app.update();

        press_key(&mut app, first_window, KeyCode::ShiftLeft);
        app.update();
        clear_widget_input(&mut app);

        if let Some(mut window) = app.world_mut().get_mut::<Window>(first_window) {
            window.focused = false;
        }
        if let Some(mut window) = app.world_mut().get_mut::<Window>(second_window) {
            window.focused = true;
        }
        app.update();
        assert!(next_requests(&app).is_empty());
        assert!(previous_requests(&app).is_empty());

        press_key(&mut app, second_window, KeyCode::Tab);
        app.update();
        assert!(next_requests(&app).is_empty());
        assert_eq!(
            previous_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![second_window]
        );
    }

    #[test]
    fn held_main_input_requires_fresh_press_after_window_context_activation() {
        let mut app = adapter_app();
        let first_window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let second_window = app
            .world_mut()
            .spawn(Window {
                focused: false,
                ..default()
            })
            .id();
        app.update();

        press_key(&mut app, first_window, KeyCode::ShiftLeft);
        press_key(&mut app, first_window, KeyCode::Tab);
        app.update();
        assert_eq!(
            previous_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![first_window]
        );
        clear_widget_input(&mut app);

        if let Some(mut window) = app.world_mut().get_mut::<Window>(first_window) {
            window.focused = false;
        }
        if let Some(mut window) = app.world_mut().get_mut::<Window>(second_window) {
            window.focused = true;
        }
        app.update();
        app.update();
        assert!(next_requests(&app).is_empty());
        assert!(previous_requests(&app).is_empty());

        release_key(&mut app, second_window, KeyCode::Tab);
        app.update();
        assert!(next_requests(&app).is_empty());
        assert!(previous_requests(&app).is_empty());

        press_key(&mut app, second_window, KeyCode::Tab);
        app.update();
        assert!(next_requests(&app).is_empty());
        assert_eq!(
            previous_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![second_window]
        );
    }

    #[test]
    fn modifier_priority_uses_each_physical_binding() {
        let mut app = adapter_app();
        let result = WidgetInputBindings::builder()
            .next(KeyCode::Enter)
            .next(KeyCode::Space.with_mod_keys(ModKeys::SHIFT))
            .activate(KeyCode::Enter.with_mod_keys(ModKeys::SHIFT))
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Bindings(bindings),
            ))
            .id();
        app.update();

        let installed_bindings = installed_actions(&app, window)
            .into_iter()
            .filter_map(|action| app.world().get::<InstalledWidgetInputBinding>(action))
            .map(|installed| installed.0)
            .collect::<Vec<_>>();
        assert!(installed_bindings.contains(&Binding::from(KeyCode::Enter)));
        assert!(installed_bindings.contains(&KeyCode::Space.with_mod_keys(ModKeys::SHIFT)));
        assert!(installed_bindings.contains(&KeyCode::Enter.with_mod_keys(ModKeys::SHIFT)));

        press_key(&mut app, window, KeyCode::ShiftLeft);
        press_key(&mut app, window, KeyCode::Enter);
        app.update();

        assert!(next_requests(&app).is_empty());
        assert_eq!(
            activate_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );
    }

    #[test]
    fn modifier_release_keeps_an_unrelated_alternative_available() {
        let mut app = adapter_app();
        let result = WidgetInputBindings::builder()
            .next(KeyCode::Enter.with_mod_keys(ModKeys::SHIFT))
            .activate(KeyCode::Enter)
            .activate(KeyCode::KeyN)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Bindings(bindings),
            ))
            .id();
        app.update();

        press_key(&mut app, window, KeyCode::ShiftLeft);
        press_key(&mut app, window, KeyCode::Enter);
        app.update();
        assert_eq!(
            next_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );
        assert!(activate_requests(&app).is_empty());

        clear_widget_input(&mut app);
        release_key(&mut app, window, KeyCode::ShiftLeft);
        press_key(&mut app, window, KeyCode::KeyN);
        app.update();

        assert!(next_requests(&app).is_empty());
        assert_eq!(
            activate_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );
    }

    #[test]
    fn simultaneous_alternatives_emit_one_semantic_message() {
        let mut app = adapter_app();
        let result = WidgetInputBindings::builder()
            .next(KeyCode::KeyN)
            .next(KeyCode::KeyM)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Bindings(bindings),
            ))
            .id();
        app.update();

        press_key(&mut app, window, KeyCode::KeyN);
        press_key(&mut app, window, KeyCode::KeyM);
        app.update();

        assert_eq!(
            next_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );
    }

    #[test]
    fn modifier_release_does_not_start_a_custom_unmodified_action() {
        let mut app = adapter_app();
        let result = WidgetInputBindings::builder()
            .next(KeyCode::Enter.with_mod_keys(ModKeys::SHIFT))
            .activate(KeyCode::Enter)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Bindings(bindings),
            ))
            .id();
        app.update();

        press_key(&mut app, window, KeyCode::ShiftLeft);
        press_key(&mut app, window, KeyCode::Enter);
        app.update();

        assert_eq!(
            next_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );
        assert!(activate_requests(&app).is_empty());

        release_key(&mut app, window, KeyCode::ShiftLeft);
        app.update();
        assert!(activate_requests(&app).is_empty());

        release_key(&mut app, window, KeyCode::Enter);
        app.update();
        press_key(&mut app, window, KeyCode::Enter);
        app.update();
        assert_eq!(
            activate_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );
    }

    #[test]
    fn modified_and_bare_alternatives_emit_once_without_release_handoff() {
        let mut app = adapter_app();
        let result = WidgetInputBindings::builder()
            .next(KeyCode::Enter.with_mod_keys(ModKeys::SHIFT))
            .next(KeyCode::Enter)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Bindings(bindings),
            ))
            .id();
        app.update();

        press_key(&mut app, window, KeyCode::ShiftLeft);
        press_key(&mut app, window, KeyCode::Enter);
        app.update();
        assert_eq!(
            next_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );

        clear_widget_input(&mut app);
        release_key(&mut app, window, KeyCode::ShiftLeft);
        app.update();
        assert!(next_requests(&app).is_empty());
    }

    #[test]
    fn no_unique_focused_window_emits_no_adapter_action() {
        let mut app = adapter_app();
        let first = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        app.world_mut().spawn(Window {
            focused: true,
            ..default()
        });
        app.update();

        press_key(&mut app, first, KeyCode::Tab);
        app.update();

        assert!(next_requests(&app).is_empty());
    }

    #[test]
    fn first_tab_focuses_first_widget_in_application_selected_panel() {
        let mut app = test_app(TestIme::Absent);
        app.add_plugins((InputPlugin, WidgetInputPlugin));
        app.finish();
        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let panel = spawn_panel(&mut app, &["first", "second"]);
        let other_panel = spawn_panel(&mut app, &["other"]);
        app.update();
        let first = widget(&mut app, panel, "first");
        let other = widget(&mut app, other_panel, "other");

        app.world_mut().trigger(RequestPanelFocus { window, panel });
        press_key(&mut app, window, KeyCode::Tab);
        app.update();

        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            Some(first)
        );
        assert!(app.world().get::<WidgetFocused>(first).is_some());
        assert!(app.world().get::<WidgetFocusVisible>(first).is_some());
        assert!(app.world().get::<WidgetFocused>(other).is_none());
    }

    #[test]
    fn gamepad_action_targets_the_unique_focused_window() {
        let mut app = adapter_app();
        let result = WidgetInputBindings::builder()
            .activate(GamepadButton::South)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Bindings(bindings),
            ))
            .id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        app.update();

        app.world_mut()
            .write_message(RawGamepadEvent::Button(RawGamepadButtonChangedEvent::new(
                gamepad,
                GamepadButton::South,
                1.0,
            )));
        app.update();

        assert_eq!(
            activate_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );
    }

    #[test]
    fn gamepad_action_emits_nothing_without_a_unique_focused_window() {
        let mut app = adapter_app();
        let result = WidgetInputBindings::builder()
            .activate(GamepadButton::South)
            .build();
        assert!(result.is_ok());
        let Ok(bindings) = result else {
            return;
        };
        let first_window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Bindings(bindings.clone()),
            ))
            .id();
        let second_window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                WidgetInputMode::Bindings(bindings),
            ))
            .id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        app.update();

        assert!(!context_is_active(&app, first_window));
        assert!(!context_is_active(&app, second_window));
        app.world_mut()
            .write_message(RawGamepadEvent::Button(RawGamepadButtonChangedEvent::new(
                gamepad,
                GamepadButton::South,
                1.0,
            )));
        app.update();

        assert!(activate_requests(&app).is_empty());
    }

    #[test]
    fn app_owned_kana_action_sends_core_message_without_adapter_entities() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, EnhancedInputPlugin))
            .add_input_context::<AppOwnedInputContext>()
            .add_message::<WidgetInput>()
            .add_systems(Startup, spawn_app_owned_input);
        hana_rubric::bind_action_system!(
            app,
            AppOwnedNextAction,
            AppOwnedNextEvent,
            send_app_owned_next
        );
        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        app.insert_resource(AppOwnedWindow(window));
        app.finish();
        app.update();

        press_key(&mut app, window, KeyCode::KeyN);
        app.update();

        assert_eq!(
            next_requests(&app)
                .iter()
                .map(WidgetInput::window)
                .collect::<Vec<_>>(),
            vec![window]
        );
        assert_eq!(context_count(&mut app), 0);
        assert_eq!(action_count(&mut app), 0);
    }

    #[test]
    fn adapter_disable_and_mode_removal_preserve_widget_focus() {
        let mut app = test_app(TestIme::Absent);
        app.add_plugins((InputPlugin, WidgetInputPlugin));
        app.finish();
        let first_window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let second_window = app
            .world_mut()
            .spawn(Window {
                focused: false,
                ..default()
            })
            .id();
        let panel = spawn_panel(&mut app, &["first", "second"]);
        app.update();
        let first = widget(&mut app, panel, "first");
        let second = widget(&mut app, panel, "second");
        request_focus(&mut app, first_window, first);
        request_focus(&mut app, second_window, second);

        app.world_mut()
            .entity_mut(first_window)
            .insert(WidgetInputDisabled);
        app.world_mut()
            .entity_mut(second_window)
            .remove::<WidgetInputMode>();
        app.update();

        assert!(app.world().get::<WidgetFocused>(first).is_some());
        assert!(app.world().get::<WidgetFocused>(second).is_some());
        assert!(matches!(
            app.world().get::<WidgetInputMode>(first_window),
            Some(WidgetInputMode::Default)
        ));
        assert!(app.world().get::<WidgetInputMode>(second_window).is_none());
    }

    #[test]
    fn window_focus_add_and_remove_reconcile_contexts_without_clearing_widget_focus() {
        let mut app = test_app(TestIme::Absent);
        app.add_plugins((InputPlugin, WidgetInputPlugin));
        app.finish();
        let first_window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let second_window = app
            .world_mut()
            .spawn(Window {
                focused: false,
                ..default()
            })
            .id();
        let panel = spawn_panel(&mut app, &["first", "second"]);
        app.update();
        let first = widget(&mut app, panel, "first");
        let second = widget(&mut app, panel, "second");
        request_focus(&mut app, first_window, first);
        request_focus(&mut app, second_window, second);

        assert!(context_is_active(&app, first_window));
        assert!(!context_is_active(&app, second_window));

        if let Some(mut window) = app.world_mut().get_mut::<Window>(first_window) {
            window.focused = false;
        }
        if let Some(mut window) = app.world_mut().get_mut::<Window>(second_window) {
            window.focused = true;
        }
        app.update();

        assert!(!context_is_active(&app, first_window));
        assert!(context_is_active(&app, second_window));
        assert!(app.world().get::<WidgetFocused>(first).is_some());
        assert!(app.world().get::<WidgetFocused>(second).is_some());

        let added_window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        app.update();

        assert!(!context_is_active(&app, first_window));
        assert!(!context_is_active(&app, second_window));
        assert!(!context_is_active(&app, added_window));

        app.world_mut().entity_mut(added_window).remove::<Window>();
        app.update();

        assert!(!context_is_active(&app, first_window));
        assert!(context_is_active(&app, second_window));
        assert!(
            app.world()
                .get::<WidgetInputContext>(added_window)
                .is_none()
        );
        assert!(app.world().get::<WidgetFocused>(first).is_some());
        assert!(app.world().get::<WidgetFocused>(second).is_some());
    }

    #[test]
    fn ime_blocks_an_adapter_action_in_its_leased_window() {
        let mut app = test_app(TestIme::Present);
        app.add_plugins((InputPlugin, WidgetInputPlugin));
        app.finish();
        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let panel = spawn_panel(&mut app, &["target"]);
        app.update();
        let target = widget(&mut app, panel, "target");
        request_focus(&mut app, window, target);
        let owner = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::AppOwned {
                owner,
                field_id: PanelElementId::named("ime-owner"),
            },
            window,
            initial_text: String::new(),
            field_spec: ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test")),
            anchor: None,
        });
        press_key(&mut app, window, KeyCode::Enter);
        app.update();

        assert!(app.world().resource::<RecordedIntents>().0.is_empty());
        assert!(app.world().get::<WidgetFocused>(target).is_some());
    }

    #[test]
    fn pointer_focus_routes_each_semantic_intent_once() {
        let mut app = test_app(TestIme::Absent);
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_panel(&mut app, &["target"]);
        app.update();
        let target = widget(&mut app, panel, "target");
        let Some(window_ref) = WindowRef::Entity(window).normalize(None) else {
            return;
        };
        app.world_mut().trigger(Pointer::new(
            PointerId::Mouse,
            Location {
                target:   NormalizedRenderTarget::Window(window_ref),
                position: Vec2::ZERO,
            },
            Press {
                button: PointerButton::Primary,
                hit:    HitData::new(camera, 0.0, None, None),
                count:  1,
            },
            target,
        ));
        app.world_mut()
            .write_message(WidgetInput::Activate { window });
        app.world_mut()
            .write_message(WidgetInput::Cancel { window });
        app.update();

        assert_eq!(
            app.world().resource::<RecordedIntents>().0,
            vec![
                (target, RecordedIntentAction::Activate),
                (target, RecordedIntentAction::Cancel),
            ]
        );
    }

    #[test]
    fn same_frame_widget_input_preserves_write_order() {
        let mut app = test_app(TestIme::Absent);
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["first", "second"]);
        app.update();
        let first = widget(&mut app, panel, "first");
        let second = widget(&mut app, panel, "second");
        request_focus(&mut app, window, first);

        app.world_mut()
            .write_message(WidgetInput::Activate { window });
        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();

        assert_eq!(
            app.world().resource::<RecordedIntents>().0,
            vec![(first, RecordedIntentAction::Activate)]
        );
        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            Some(second)
        );

        app.world_mut().resource_mut::<RecordedIntents>().0.clear();
        request_focus(&mut app, window, first);
        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.world_mut()
            .write_message(WidgetInput::Activate { window });
        app.update();

        assert_eq!(
            app.world().resource::<RecordedIntents>().0,
            vec![(second, RecordedIntentAction::Activate)]
        );
    }

    #[test]
    fn same_frame_disabled_resolution_suppresses_semantic_intent_without_clearing_focus() {
        let mut app = test_app(TestIme::Absent);
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["target"]);
        app.update();
        let target = widget(&mut app, panel, "target");
        request_focus(&mut app, window, target);

        let result = app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.override_interactivity(target, WidgetInteractivity::Disabled)
            });
        assert!(matches!(result, Ok(true)));
        app.world_mut()
            .write_message(WidgetInput::Activate { window });
        app.update();

        assert!(app.world().get::<WidgetDisabled>(target).is_some());
        assert!(app.world().get::<WidgetFocused>(target).is_some());
        assert!(app.world().resource::<RecordedIntents>().0.is_empty());
    }

    #[test]
    fn ime_blocks_only_the_leased_windows_semantic_input() {
        let mut app = test_app(TestIme::Present);
        let blocked_window = app.world_mut().spawn(Window::default()).id();
        let available_window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["blocked", "available"]);
        app.update();
        let blocked = widget(&mut app, panel, "blocked");
        let available = widget(&mut app, panel, "available");
        request_focus(&mut app, blocked_window, blocked);
        request_focus(&mut app, available_window, available);

        let owner = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(ImeOpenSession {
            target:       ImeTarget::AppOwned {
                owner,
                field_id: PanelElementId::named("ime-owner"),
            },
            window:       blocked_window,
            initial_text: String::new(),
            field_spec:   ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test")),
            anchor:       None,
        });
        app.world_mut().write_message(WidgetInput::Activate {
            window: blocked_window,
        });
        app.world_mut().write_message(WidgetInput::Activate {
            window: available_window,
        });
        app.update();

        assert_eq!(
            app.world().resource::<RecordedIntents>().0,
            vec![(available, RecordedIntentAction::Activate)]
        );
    }

    #[test]
    fn focused_editable_field_activates_ime_and_reserves_traversal() {
        let mut app = test_app(TestIme::Present);
        let window = app.world_mut().spawn(Window::default()).id();
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::IDENTITY,
            RenderTarget::Window(WindowRef::Entity(window)),
        ));
        let panel = spawn_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "editable");
        let next = widget(&mut app, panel, "next");
        request_focus(&mut app, window, editable);
        app.world_mut().flush();
        app.world_mut()
            .write_message(WidgetInput::Activate { window });
        let routed = app.world_mut().run_system_once(super::route_semantic_input);
        assert!(routed.is_ok());
        app.world_mut().flush();

        assert!(
            app.world()
                .resource::<ImeInputBlocker>()
                .blocks_window(window)
        );
        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            Some(editable)
        );

        app.world_mut()
            .resource_mut::<Messages<WidgetInput>>()
            .clear();
        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        let routed = app.world_mut().run_system_once(super::route_semantic_input);
        assert!(routed.is_ok());
        app.world_mut().flush();
        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            Some(editable)
        );
        assert_ne!(editable, next);
    }

    #[test]
    fn traversal_into_editable_field_starts_editing_with_all_text_selected() {
        let mut app = test_app(TestIme::Present);
        let window = app.world_mut().spawn(Window::default()).id();
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::IDENTITY,
            RenderTarget::Window(WindowRef::Entity(window)),
        ));
        let panel = spawn_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "editable");

        app.world_mut().trigger(RequestPanelFocus { window, panel });
        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        let routed = app.world_mut().run_system_once(super::route_semantic_input);
        assert!(routed.is_ok());
        app.world_mut().flush();

        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            Some(editable)
        );
        assert!(
            app.world()
                .resource::<ImeInputBlocker>()
                .blocks_window(window)
        );
        let changes = &app.world().resource::<RecordedImeTextChanges>().0;
        let Some(snapshot) = changes.last() else {
            panic!("traversal should start an IME edit session");
        };
        assert_eq!(snapshot.committed_text, "Initial text");
        let ImeCursorState::Selection(selection) = &snapshot.cursor else {
            panic!("traversal should select the editable field's text");
        };
        assert_eq!(selection.anchor.as_usize(), 0);
        assert_eq!(selection.focus.as_usize(), "Initial text".len());
    }

    #[test]
    fn tab_commits_an_inline_edit_and_traverses_to_the_next_widget() {
        let mut app = test_app(TestIme::Present);
        let window = app.world_mut().spawn(Window::default()).id();
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::IDENTITY,
            RenderTarget::Window(WindowRef::Entity(window)),
        ));
        let (panel, _) = spawn_builtin_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "editable");
        let next = widget(&mut app, panel, "next");

        app.world_mut().trigger(RequestPanelFocus { window, panel });
        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        let routed = app.world_mut().run_system_once(super::route_semantic_input);
        assert!(routed.is_ok());
        app.world_mut().flush();
        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            Some(editable)
        );

        let next_frame = app.world().resource::<FrameCount>().0.wrapping_add(1);
        app.world_mut().resource_mut::<FrameCount>().0 = next_frame;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Tab);
        press_key(&mut app, window, KeyCode::Tab);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Tab);
        release_key(&mut app, window, KeyCode::Tab);
        app.update();

        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            Some(next)
        );
        assert!(
            !app.world()
                .resource::<ImeInputBlocker>()
                .blocks_window(window)
        );
        assert!(
            app.world().get::<WidgetFocusVisible>(editable).is_none(),
            "the old field must stop qualifying for a focus-triggered tooltip"
        );
    }

    #[test]
    fn activating_editable_field_with_enter_does_not_close_new_session() {
        let mut app = test_app(TestIme::Present);
        app.init_resource::<ImeLeaseAfterInput>().add_systems(
            Update,
            record_ime_lease_after_input
                .after(ImeSystemSet::Input)
                .before(ImeSystemSet::UpdateEditorGeometry),
        );
        let window = app.world_mut().spawn(Window::default()).id();
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::IDENTITY,
            RenderTarget::Window(WindowRef::Entity(window)),
        ));
        let (panel, _) = spawn_builtin_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "editable");
        request_focus(&mut app, window, editable);
        app.world_mut().flush();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.world_mut()
            .write_message(WidgetInput::Activate { window });
        app.update();

        assert_eq!(
            app.world().resource::<ImeLeaseAfterInput>().0,
            Some(window),
            "the Enter edge that opens the editor must not close it",
        );
    }

    #[test]
    fn enter_commits_inline_edit_and_removes_selection_without_reactivation() {
        let mut app = test_app(TestIme::Present);
        let window = app.world_mut().spawn(Window::default()).id();
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::IDENTITY,
            RenderTarget::Window(WindowRef::Entity(window)),
        ));
        let (panel, authored_tree) = spawn_builtin_editable_panel(&mut app);
        app.update();
        let editable = widget(&mut app, panel, "editable");
        request_focus(&mut app, window, editable);
        app.world_mut().flush();
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::WorldPanelField {
                panel,
                field_id: PanelElementId::named("editable"),
            },
            window,
            initial_text: "Initial text".to_owned(),
            field_spec: ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Text,
            )),
            anchor: None,
        });
        app.world_mut().flush();
        assert_eq!(
            app.world().resource::<ImeInputBlocker>().window(),
            Some(window),
            "the inline session should be active before Enter",
        );
        let next_frame = app.world().resource::<FrameCount>().0.wrapping_add(1);
        app.world_mut().resource_mut::<FrameCount>().0 = next_frame;

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.world_mut()
            .write_message(WidgetInput::Activate { window });
        app.update();

        assert_eq!(
            app.world().resource::<ImeInputBlocker>().window(),
            None,
            "Enter should complete the active edit instead of reopening it",
        );
        let committed_tree = app
            .world()
            .get::<DiegeticPanel>(panel)
            .map(DiegeticPanel::tree);
        assert!(committed_tree.is_some(), "the editable panel should remain");
        let Some(committed_tree) = committed_tree else {
            return;
        };
        assert_eq!(
            authored_tree.classify_change(committed_tree),
            LayoutTreeChange::Identical,
            "committing should remove the transient inline selection",
        );
    }
}
