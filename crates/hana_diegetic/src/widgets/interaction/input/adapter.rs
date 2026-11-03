use std::collections::HashMap;
use std::collections::HashSet;

use bevy::prelude::Added;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityWorldMut;
use bevy::prelude::Has;
use bevy::prelude::MessageWriter;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::RemovedComponents;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::ActionOf;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Actions;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::Cancel;
use bevy_enhanced_input::prelude::Complete;
use bevy_enhanced_input::prelude::ContextActivity;
use bevy_enhanced_input::prelude::ContextPriority;
use bevy_enhanced_input::prelude::Fire;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::Pulse;
use bevy_enhanced_input::prelude::Start;
use hana_rubric::Keybindings;

use super::bindings::WidgetInputAction;
use super::bindings::WidgetInputBindings;
use super::bindings::WidgetInputMode;
use super::bindings::WidgetInputModeInitialized;
use super::bindings::WidgetStepBinding;
use super::gate::StepBindingFocus;
use super::gate::WidgetBindingGate;
use super::routing::WidgetInput;
use super::routing::WidgetStepDirection;

/// Presence marker that temporarily disables Hana-owned widget input for a window.
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct WidgetInputDisabled;

#[derive(Component, Clone, Copy, Debug, Default)]
pub(super) struct WidgetInputContext;

#[derive(Component, Clone, Debug, PartialEq)]
pub(super) enum WidgetInputInstallation {
    /// `WidgetInputMode::Manual`: the application owns the window's input context.
    ApplicationOwned,
    /// `WidgetInputDisabled` is on the window; it wins over every mode.
    Disabled,
    /// The widget actions are spawned from these bindings.
    Installed(WidgetInputBindings),
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

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct InstalledWidgetInputBinding(Binding);

#[derive(Default, Resource)]
pub(super) struct PendingWidgetInputActions {
    inputs: Vec<PendingWidgetInput>,
}

enum PendingWidgetInput {
    Action {
        window: Entity,
        action: WidgetInputAction,
    },
    ActivationEdge(WidgetInput),
}

/// Activate action entities still held for each window.
#[derive(Default, Resource)]
pub(super) struct HeldWidgetActivations {
    windows: HashMap<Entity, HashSet<Entity>>,
}

impl HeldWidgetActivations {
    fn start(&mut self, window: Entity, action: Entity) -> bool {
        let actions = self.windows.entry(window).or_default();
        let first = actions.is_empty();
        actions.insert(action) && first
    }

    fn end(&mut self, window: Entity, action: Entity) -> bool {
        let Some(actions) = self.windows.get_mut(&window) else {
            return false;
        };
        if !actions.remove(&action) || !actions.is_empty() {
            return false;
        }
        self.windows.remove(&window);
        true
    }

    fn interrupt(&mut self, window: Entity, pending: &mut PendingWidgetInputActions) {
        if self
            .windows
            .remove(&window)
            .is_some_and(|actions| !actions.is_empty())
        {
            pending.inputs.push(PendingWidgetInput::ActivationEdge(
                WidgetInput::ActivateInterrupted { window },
            ));
        }
    }
}

#[derive(Default, Resource)]
pub(super) struct PendingActivateStarts(Vec<(Entity, Entity)>);

#[derive(Default, Resource)]
pub(super) struct PendingActivateEndings(Vec<(Entity, Entity)>);

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct NextWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct PreviousWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct FirstWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct LastWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct ActivateWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct CancelWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct StepLeftWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct StepRightWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct StepUpWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
pub(super) struct StepDownWidgetAction;

#[derive(Clone, Copy, Debug, Default, Eq, InputAction, PartialEq)]
#[action_output(bool)]
struct WidgetShiftModifierAction;

pub(super) fn record_action_start<A: InputAction>(
    action: On<Start<A>>,
    actions: Query<&WidgetInputAction>,
    mut pending: ResMut<PendingWidgetInputActions>,
) {
    let Ok(&widget_action) = actions.get(action.action) else {
        return;
    };
    let window = action.context;
    if !pending.inputs.iter().any(|input| {
        matches!(input, PendingWidgetInput::Action { window: queued_window, action: queued_action }
            if *queued_window == window && *queued_action == widget_action)
    }) {
        pending.inputs.push(PendingWidgetInput::Action {
            window,
            action: widget_action,
        });
    }
}

pub(super) fn record_step_fire<A: InputAction>(
    action: On<Fire<A>>,
    actions: Query<&WidgetInputAction>,
    mut pending: ResMut<PendingWidgetInputActions>,
) {
    let Ok(&widget_action) = actions.get(action.action) else {
        return;
    };
    if !matches!(widget_action, WidgetInputAction::Step(_)) {
        return;
    }
    pending.inputs.push(PendingWidgetInput::Action {
        window: action.context,
        action: widget_action,
    });
}

pub(super) fn record_activate_start(
    action: On<Start<ActivateWidgetAction>>,
    mut starts: ResMut<PendingActivateStarts>,
) {
    starts.0.push((action.context, action.action));
}

pub(super) fn queue_activate_complete(
    action: On<Complete<ActivateWidgetAction>>,
    mut endings: ResMut<PendingActivateEndings>,
) {
    endings.0.push((action.context, action.action));
}

pub(super) fn queue_activate_cancel(
    action: On<Cancel<ActivateWidgetAction>>,
    mut endings: ResMut<PendingActivateEndings>,
) {
    endings.0.push((action.context, action.action));
}

pub(super) fn classify_activate_endings(
    mut held: ResMut<HeldWidgetActivations>,
    mut starts: ResMut<PendingActivateStarts>,
    mut endings: ResMut<PendingActivateEndings>,
    mut pending: ResMut<PendingWidgetInputActions>,
    actions: Query<&WidgetBindingGate, With<Action<ActivateWidgetAction>>>,
    windows: Query<&Window>,
    contexts: Query<&ContextActivity<WidgetInputContext>>,
) {
    for (window, action) in endings.0.drain(..) {
        if held.end(window, action) {
            let released = actions.get(action).is_ok_and(|gate| gate.open)
                && windows.get(window).is_ok_and(|window| window.focused)
                && contexts.get(window).is_ok_and(|activity| **activity);
            pending
                .inputs
                .push(PendingWidgetInput::ActivationEdge(if released {
                    WidgetInput::ActivateRelease { window }
                } else {
                    WidgetInput::ActivateInterrupted { window }
                }));
        }
    }
    for (&window, actions_held) in &mut held.windows {
        actions_held.retain(|&action| actions.get(action).is_ok());
        if actions_held.is_empty() {
            pending.inputs.push(PendingWidgetInput::ActivationEdge(
                WidgetInput::ActivateInterrupted { window },
            ));
        }
    }
    held.windows.retain(|_, actions| !actions.is_empty());
    for (window, action) in starts.0.drain(..) {
        if actions.get(action).is_ok() && held.start(window, action) {
            pending.inputs.push(PendingWidgetInput::Action {
                window,
                action: WidgetInputAction::Activate,
            });
        }
    }
}

pub(super) fn install_default_modes(
    windows: Query<Entity, (With<Window>, Without<WidgetInputModeInitialized>)>,
    mut commands: Commands,
) {
    for window in windows.iter() {
        commands
            .entity(window)
            .insert((WidgetInputMode::Default, WidgetInputModeInitialized));
    }
}

pub(super) fn reconcile_input_installations(
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
    mut held: ResMut<HeldWidgetActivations>,
    mut pending: ResMut<PendingWidgetInputActions>,
    mut commands: Commands,
) {
    for (window, mode, disabled, installation) in changed.iter() {
        let state: WidgetInputAdapterState = disabled.into();
        reconcile_window_input(
            window,
            mode,
            state,
            installation,
            &mut held,
            &mut pending,
            &mut commands,
        );
    }
    for window in removed_disabled.read() {
        let Ok((mode, disabled, installation)) = modes.get(window) else {
            continue;
        };
        let state: WidgetInputAdapterState = disabled.into();
        reconcile_window_input(
            window,
            mode,
            state,
            installation,
            &mut held,
            &mut pending,
            &mut commands,
        );
    }
    for window in removed_modes.read() {
        if modes.get(window).is_err() {
            remove_window_input(window, &mut held, &mut pending, &mut commands);
        }
    }
    for window in removed_windows.read() {
        if windows.get(window).is_err() {
            remove_window_input(window, &mut held, &mut pending, &mut commands);
        }
    }
}

fn reconcile_window_input(
    window: Entity,
    mode: &WidgetInputMode,
    state: WidgetInputAdapterState,
    installation: Option<&WidgetInputInstallation>,
    held: &mut HeldWidgetActivations,
    pending: &mut PendingWidgetInputActions,
    commands: &mut Commands<'_, '_>,
) {
    let desired = match (state, mode) {
        (WidgetInputAdapterState::Disabled, _) => WidgetInputInstallation::Disabled,
        (WidgetInputAdapterState::Enabled, WidgetInputMode::Default) => {
            WidgetInputInstallation::Installed(WidgetInputBindings::default())
        },
        (WidgetInputAdapterState::Enabled, WidgetInputMode::Bindings(bindings)) => {
            WidgetInputInstallation::Installed(bindings.clone())
        },
        (WidgetInputAdapterState::Enabled, WidgetInputMode::Manual) => {
            WidgetInputInstallation::ApplicationOwned
        },
    };
    if installation == Some(&desired) {
        return;
    }

    held.interrupt(window, pending);

    let Ok(mut window_commands) = commands.get_entity(window) else {
        return;
    };
    window_commands
        .despawn_related::<Actions<WidgetInputContext>>()
        .remove_with_requires::<WidgetInputContext>()
        .insert(desired.clone());

    let WidgetInputInstallation::Installed(bindings) = desired else {
        return;
    };
    window_commands.queue(move |mut window_entity: EntityWorldMut<'_>| {
        window_entity.insert((
            WidgetInputContext,
            ContextPriority::<WidgetInputContext>::new(1),
        ));
        window_entity.with_related_entities::<ActionOf<WidgetInputContext>>(|spawner| {
            spawn_widget_input_actions(spawner, bindings);
        });
    });
}

fn remove_window_input(
    window: Entity,
    held: &mut HeldWidgetActivations,
    pending: &mut PendingWidgetInputActions,
    commands: &mut Commands<'_, '_>,
) {
    held.interrupt(window, pending);
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
    // `require_reset` hides a key held across installation or removal from every
    // context until it is released, even while `WidgetBindingGate` is closed:
    // the reset check consumes the input before any condition runs. Without it,
    // a key held while widget input is removed reaches the application as a
    // fresh press, so one press would act twice.
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
    for binding in bindings.activate {
        let shortcut = keybindings.spawn_held_shortcut::<ActivateWidgetAction>(spawner, binding);
        spawner.world_mut().entity_mut(shortcut.press).insert((
            WidgetInputAction::Activate,
            InstalledWidgetInputBinding(binding),
            WidgetBindingGate { open: false },
        ));
        spawner.world_mut().entity_mut(shortcut.hold).insert((
            WidgetInputAction::Activate,
            InstalledWidgetInputBinding(binding),
            WidgetBindingGate { open: false },
            ActionSettings {
                consume_input: true,
                require_reset: true,
                ..default()
            },
        ));
    }
    spawn_widget_input_actions_for::<CancelWidgetAction>(
        spawner,
        &keybindings,
        WidgetInputAction::Cancel,
        bindings.cancel,
    );
    spawn_widget_step_actions_for::<StepLeftWidgetAction>(
        spawner,
        &keybindings,
        WidgetStepDirection::Left,
        &bindings.steps,
    );
    spawn_widget_step_actions_for::<StepRightWidgetAction>(
        spawner,
        &keybindings,
        WidgetStepDirection::Right,
        &bindings.steps,
    );
    spawn_widget_step_actions_for::<StepUpWidgetAction>(
        spawner,
        &keybindings,
        WidgetStepDirection::Up,
        &bindings.steps,
    );
    spawn_widget_step_actions_for::<StepDownWidgetAction>(
        spawner,
        &keybindings,
        WidgetStepDirection::Down,
        &bindings.steps,
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
        spawner.world_mut().entity_mut(action_entity).insert((
            action,
            InstalledWidgetInputBinding(binding),
            WidgetBindingGate { open: false },
        ));
    }
}

fn spawn_widget_step_actions_for<A: InputAction>(
    spawner: &mut ActionSpawner<WidgetInputContext>,
    keybindings: &Keybindings<WidgetInputContext>,
    direction: WidgetStepDirection,
    steps: &[WidgetStepBinding],
) {
    for step in steps.iter().filter(|step| step.direction == direction) {
        let binding = step.binding;
        let shortcut = keybindings.spawn_held_shortcut::<A>(spawner, binding);
        spawner.world_mut().entity_mut(shortcut.press).insert((
            WidgetInputAction::Step(direction),
            InstalledWidgetInputBinding(binding),
            WidgetBindingGate { open: false },
            StepBindingFocus::AwaitingFirstOpen,
        ));
        spawner.world_mut().entity_mut(shortcut.hold).insert((
            WidgetInputAction::Step(direction),
            InstalledWidgetInputBinding(binding),
            WidgetBindingGate { open: false },
            StepBindingFocus::AwaitingFirstOpen,
            Pulse::new(0.15).with_initial_delay(0.4),
        ));
    }
}

pub(super) fn synchronize_context_activity(
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

pub(super) fn emit_widget_input(
    mut pending: ResMut<PendingWidgetInputActions>,
    mut widget_input: MessageWriter<WidgetInput>,
) {
    for input in pending.inputs.drain(..) {
        widget_input.write(match input {
            PendingWidgetInput::Action { window, action } => match action {
                WidgetInputAction::Next => WidgetInput::FocusNext { window },
                WidgetInputAction::Previous => WidgetInput::FocusPrevious { window },
                WidgetInputAction::First => WidgetInput::FocusFirst { window },
                WidgetInputAction::Last => WidgetInput::FocusLast { window },
                WidgetInputAction::Activate => WidgetInput::ActivatePress { window },
                WidgetInputAction::Cancel => WidgetInput::Cancel { window },
                WidgetInputAction::Step(direction) => WidgetInput::Step { window, direction },
            },
            PendingWidgetInput::ActivationEdge(input) => input,
        });
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy::ecs::message::MessageCursor;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::gamepad::RawGamepadButtonChangedEvent;
    use bevy::input::gamepad::RawGamepadEvent;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Commands;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::Event;
    use bevy::prelude::Gamepad;
    use bevy::prelude::GamepadButton;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MessageWriter;
    use bevy::prelude::Messages;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::PreUpdate;
    use bevy::prelude::Query;
    use bevy::prelude::Reflect;
    use bevy::prelude::ReflectEvent;
    use bevy::prelude::RelationshipTarget;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::SpawnRelated;
    use bevy::prelude::SpawnWith;
    use bevy::prelude::Startup;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::prelude::World;
    use bevy::prelude::default;
    use bevy::window::Ime;
    use bevy::window::WindowClosed;
    use bevy::window::WindowFocused;
    use bevy_enhanced_input::prelude::Action;
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
    use hana_rubric::Keybindings;

    use super::ActivateWidgetAction;
    use super::HeldWidgetActivations;
    use super::InstalledWidgetInputBinding;
    use super::PendingActivateEndings;
    use super::PendingActivateStarts;
    use super::PendingWidgetInput;
    use super::PendingWidgetInputActions;
    use super::WidgetBindingGate;
    use super::WidgetInputContext;
    use super::WidgetInputDisabled;
    use super::WidgetInputInstallation;
    use super::classify_activate_endings;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::ImeOpenSession;
    use crate::ImeTarget;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::RequestPanelFocus;
    use crate::RequestWidgetFocus;
    use crate::WidgetFocused;
    use crate::ime::ImePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetFocusAuthority;
    use crate::widgets::WidgetFocusTarget;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::interaction::focus::WidgetFocusVisible;
    use crate::widgets::interaction::input::SemanticWidgetIntent;
    use crate::widgets::interaction::input::WidgetInput;
    use crate::widgets::interaction::input::WidgetInputBindings;
    use crate::widgets::interaction::input::WidgetInputMode;
    use crate::widgets::interaction::input::WidgetInputPlugin;
    use crate::widgets::interaction::input::WidgetInputSystems;
    use crate::widgets::interaction::input::bindings::WidgetInputAction;

    const PANEL_HEIGHT: f32 = 50.0;
    const PANEL_WIDTH: f32 = 100.0;

    #[test]
    fn held_activations_emit_one_press_and_one_terminal_for_overlapping_actions() {
        let mut world = World::new();
        let window = world.spawn_empty().id();
        let space = world.spawn_empty().id();
        let enter = world.spawn_empty().id();
        let mut held = HeldWidgetActivations::default();

        assert!(held.start(window, space));
        assert!(!held.start(window, enter));
        assert!(!held.end(window, space));
        assert!(held.end(window, enter));
        assert!(!held.end(window, enter));
    }

    #[test]
    fn classifier_marks_a_removed_held_action_as_interrupted() {
        let mut app = App::new();
        app.init_resource::<HeldWidgetActivations>()
            .init_resource::<PendingActivateStarts>()
            .init_resource::<PendingActivateEndings>()
            .init_resource::<PendingWidgetInputActions>();
        let window = app
            .world_mut()
            .spawn(Window {
                focused: true,
                ..default()
            })
            .id();
        let action = app
            .world_mut()
            .spawn((
                Action::<ActivateWidgetAction>::new(),
                WidgetBindingGate { open: true },
            ))
            .id();
        assert!(
            app.world_mut()
                .resource_mut::<HeldWidgetActivations>()
                .start(window, action)
        );
        app.world_mut()
            .resource_mut::<PendingActivateEndings>()
            .0
            .push((window, action));
        app.world_mut().despawn(action);

        let result = app.world_mut().run_system_once(classify_activate_endings);
        assert!(result.is_ok());
        assert!(matches!(
            app.world().resource::<PendingWidgetInputActions>().inputs.as_slice(),
            [PendingWidgetInput::ActivationEdge(WidgetInput::ActivateInterrupted { window: ended })]
                if *ended == window
        ));
    }

    #[test]
    fn inactive_widget_context_interrupts_held_activation_with_open_gate_and_focused_window() {
        let mut app = App::new();
        app.init_resource::<HeldWidgetActivations>()
            .init_resource::<PendingActivateStarts>()
            .init_resource::<PendingActivateEndings>()
            .init_resource::<PendingWidgetInputActions>();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    focused: true,
                    ..default()
                },
                ContextActivity::<WidgetInputContext>::ACTIVE,
            ))
            .id();
        let action = app
            .world_mut()
            .spawn((
                Action::<ActivateWidgetAction>::new(),
                WidgetBindingGate { open: true },
            ))
            .id();
        assert!(
            app.world_mut()
                .resource_mut::<HeldWidgetActivations>()
                .start(window, action)
        );
        app.world_mut()
            .entity_mut(window)
            .insert(ContextActivity::<WidgetInputContext>::INACTIVE);
        app.world_mut()
            .resource_mut::<PendingActivateEndings>()
            .0
            .push((window, action));

        let result = app.world_mut().run_system_once(classify_activate_endings);
        assert!(result.is_ok());
        assert!(matches!(
            app.world().resource::<PendingWidgetInputActions>().inputs.as_slice(),
            [PendingWidgetInput::ActivationEdge(WidgetInput::ActivateInterrupted { window: ended })]
                if *ended == window
        ));
    }

    #[derive(Clone, Copy)]
    enum TestIme {
        Absent,
        Present,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedIntentAction {
        Activate,
        Cancel,
        Step,
    }

    #[derive(Default, Resource)]
    struct RecordedIntents(Vec<(Entity, RecordedIntentAction)>);

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
            SemanticWidgetIntent::Activate { entity, .. }
            | SemanticWidgetIntent::ActivateRelease { entity, .. }
            | SemanticWidgetIntent::ActivateInterrupted { entity, .. } => {
                (entity, RecordedIntentAction::Activate)
            },
            SemanticWidgetIntent::Cancel { entity } => (entity, RecordedIntentAction::Cancel),
            SemanticWidgetIntent::Step { entity, .. } => (entity, RecordedIntentAction::Step),
        };
        recorded.0.push(entry);
    }

    fn test_app(test_ime: TestIme) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<RecordedIntents>()
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .add_observer(record_intent);
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
        app.add_plugins((MinimalPlugins, InputPlugin, WidgetInputPlugin))
            .add_systems(
                PreUpdate,
                open_adapter_test_gates
                    .after(WidgetInputSystems::ActivateContexts)
                    .before(EnhancedInputSystems::Prepare),
            );
        app.finish();
        app
    }

    fn open_adapter_test_gates(mut gates: Query<&mut WidgetBindingGate>) {
        for mut gate in &mut gates {
            gate.open = true;
        }
    }

    #[test]
    fn default_mode_records_installed_bindings_and_spawns_actions() {
        let mut app = adapter_app();
        let window = app.world_mut().spawn(Window::default()).id();
        app.update();

        assert_eq!(
            app.world().get::<WidgetInputInstallation>(window),
            Some(&WidgetInputInstallation::Installed(
                WidgetInputBindings::default()
            ))
        );
        assert_eq!(
            installed_actions(&app, window).len(),
            default_action_count()
        );
    }

    #[test]
    fn manual_mode_records_application_ownership_without_actions() {
        let mut app = adapter_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), WidgetInputMode::Manual))
            .id();
        app.update();

        assert_eq!(
            app.world().get::<WidgetInputInstallation>(window),
            Some(&WidgetInputInstallation::ApplicationOwned)
        );
        assert_eq!(installed_actions(&app, window), []);
    }

    #[test]
    fn disabling_installed_input_records_disabled_and_despawns_actions() {
        let mut app = adapter_app();
        let window = app.world_mut().spawn(Window::default()).id();
        app.update();
        app.world_mut()
            .entity_mut(window)
            .insert(WidgetInputDisabled);
        app.update();

        assert_eq!(
            app.world().get::<WidgetInputInstallation>(window),
            Some(&WidgetInputInstallation::Disabled)
        );
        assert_eq!(installed_actions(&app, window), []);
    }

    #[test]
    fn reenabling_input_restores_installed_bindings_and_actions() {
        let mut app = adapter_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), WidgetInputDisabled))
            .id();
        app.update();
        app.world_mut()
            .entity_mut(window)
            .remove::<WidgetInputDisabled>();
        app.update();

        assert_eq!(
            app.world().get::<WidgetInputInstallation>(window),
            Some(&WidgetInputInstallation::Installed(
                WidgetInputBindings::default()
            ))
        );
        assert_eq!(
            installed_actions(&app, window).len(),
            default_action_count()
        );
    }

    #[test]
    fn manual_and_disabled_installations_remain_distinct() {
        let mut app = adapter_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), WidgetInputMode::Manual))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<WidgetInputInstallation>(window),
            Some(&WidgetInputInstallation::ApplicationOwned)
        );

        app.world_mut()
            .entity_mut(window)
            .insert(WidgetInputDisabled);
        app.update();
        assert_eq!(
            app.world().get::<WidgetInputInstallation>(window),
            Some(&WidgetInputInstallation::Disabled)
        );
        assert_eq!(installed_actions(&app, window), []);

        app.world_mut()
            .entity_mut(window)
            .remove::<WidgetInputDisabled>();
        app.update();
        assert_eq!(
            app.world().get::<WidgetInputInstallation>(window),
            Some(&WidgetInputInstallation::ApplicationOwned)
        );
        assert_eq!(installed_actions(&app, window), []);
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
        bindings.next.len()
            + bindings.previous.len()
            + bindings.first.len()
            + bindings.last.len()
            + bindings.activate.len() * 2
            + bindings.cancel.len()
            + bindings.steps.len() * 2
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
            .filter(|input| matches!(input, WidgetInput::ActivatePress { .. }))
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
        assert_eq!(installed_actions(&app, manual_window), []);

        app.world_mut()
            .entity_mut(bound_window)
            .insert(WidgetInputMode::Manual);
        app.update();

        assert_eq!(installed_actions(&app, default_window), default_actions);
        assert_eq!(installed_actions(&app, bound_window), []);
        assert_eq!(installed_actions(&app, manual_window), []);
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
        assert_eq!(installed_actions(&app, window), []);
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
        assert_eq!(installed_actions(&app, window), []);
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
        assert_eq!(installed_actions(&app, window), []);
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
            WidgetFocusTarget::Widget(first)
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

        assert_eq!(app.world().resource::<RecordedIntents>().0, []);
        assert!(app.world().get::<WidgetFocused>(target).is_some());
    }
}
