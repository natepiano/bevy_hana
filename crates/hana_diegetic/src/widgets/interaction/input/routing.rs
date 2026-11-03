use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::ButtonInput;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::EntityEvent;
use bevy::prelude::Event;
use bevy::prelude::Has;
use bevy::prelude::KeyCode;
use bevy::prelude::Message;
use bevy::prelude::MessageReader;
use bevy::prelude::MessageWriter;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::window::Ime;

use super::bindings::WidgetInputAction;
use crate::ime::ImeCommitCause;
use crate::ime::ImeInputBlocker;
use crate::ime::ImeRequestCommit;
use crate::ime::ImeTextChanged;
use crate::panel::ComputedDiegeticPanel;
use crate::widgets::ButtonPress;
use crate::widgets::PanelWidget;
use crate::widgets::PanelWidgets;
use crate::widgets::SliderDrag;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetFocusAuthority;
use crate::widgets::WidgetFocusTarget;
use crate::widgets::WidgetFocusable;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;
use crate::widgets::interaction::focus;
use crate::widgets::interaction::focus::WidgetTraversal;

pub(super) type WidgetActionTargets<'w, 's> = Query<
    'w,
    's,
    (
        &'static WidgetKind,
        Has<WidgetDisabled>,
        Has<ButtonPress>,
        Has<SliderDrag>,
    ),
    With<PanelWidget>,
>;

pub(super) fn can_widget_input_act(
    action: WidgetInputAction,
    window: Entity,
    authority: &WidgetFocusAuthority,
    panels: &Query<(&ComputedDiegeticPanel, &PanelWidgets)>,
    focusable_widgets: &Query<(&PanelWidget, &WidgetOf), With<WidgetFocusable>>,
    action_targets: &WidgetActionTargets,
) -> bool {
    match action {
        WidgetInputAction::Next
        | WidgetInputAction::Previous
        | WidgetInputAction::First
        | WidgetInputAction::Last => {
            let Some(panel) = authority.active_panel(window) else {
                return false;
            };
            let Ok((computed, panel_widgets)) = panels.get(panel) else {
                return false;
            };
            panel_widgets.iter().any(|&widget| {
                let Ok((panel_widget, owner)) = focusable_widgets.get(widget) else {
                    return false;
                };
                owner.panel() == panel
                    && computed
                        .widget_records()
                        .iter()
                        .any(|record| record.id() == panel_widget.id())
            })
        },
        WidgetInputAction::Activate | WidgetInputAction::Cancel | WidgetInputAction::Step(_) => {
            let WidgetFocusTarget::Widget(widget) = authority.focused_widget(window) else {
                return false;
            };
            let Ok((kind, disabled, pressed, dragging)) = action_targets.get(widget) else {
                return false;
            };
            match action {
                WidgetInputAction::Activate => {
                    !disabled && matches!(kind, WidgetKind::Button | WidgetKind::EditableField)
                },
                WidgetInputAction::Cancel => pressed || dragging,
                WidgetInputAction::Step(_) => !disabled && *kind == WidgetKind::Slider && !dragging,
                _ => false,
            }
        },
    }
}

/// Screen direction of one semantic slider step.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum WidgetStepDirection {
    /// Left on the panel.
    Left,
    /// Right on the panel.
    Right,
    /// Up on the panel.
    Up,
    /// Down on the panel.
    Down,
}

/// Requests one operation from the widget currently associated with a window.
///
/// `WidgetInput` is buffered so every producer feeds one ordered stream that
/// `route_semantic_input` processes after [`crate::ime::ImeSystemSet::PublishInputBlockers`].
/// Applications may write these requests directly or add [`super::WidgetInputPlugin`]
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
    /// Begins activation of the focused widget in the window.
    ActivatePress {
        /// Window whose focused widget should receive activation.
        window: Entity,
    },
    /// Releases activation of the focused widget in the window.
    ActivateRelease {
        /// Window whose focused widget should receive activation.
        window: Entity,
    },
    /// Ends activation after the held input is interrupted.
    ActivateInterrupted {
        /// Window whose focused widget should receive activation.
        window: Entity,
    },
    /// Proposes one step for the focused slider.
    Step {
        /// Window whose focused slider should receive the step.
        window:    Entity,
        /// Direction pressed on the panel.
        direction: WidgetStepDirection,
    },
    /// Cancels the focused widget's active interaction in the window.
    Cancel {
        /// Window whose focused widget should receive cancellation.
        window: Entity,
    },
}

impl WidgetInput {
    const fn action(&self) -> WidgetInputAction {
        match self {
            Self::FocusNext { .. } => WidgetInputAction::Next,
            Self::FocusPrevious { .. } => WidgetInputAction::Previous,
            Self::FocusFirst { .. } => WidgetInputAction::First,
            Self::FocusLast { .. } => WidgetInputAction::Last,
            Self::ActivatePress { .. }
            | Self::ActivateRelease { .. }
            | Self::ActivateInterrupted { .. } => WidgetInputAction::Activate,
            Self::Cancel { .. } => WidgetInputAction::Cancel,
            Self::Step { direction, .. } => WidgetInputAction::Step(*direction),
        }
    }

    /// Returns the window whose widget focus scope receives this request.
    #[must_use]
    pub const fn window(&self) -> Entity {
        match *self {
            Self::FocusNext { window }
            | Self::FocusPrevious { window }
            | Self::FocusFirst { window }
            | Self::FocusLast { window }
            | Self::ActivatePress { window }
            | Self::ActivateRelease { window }
            | Self::ActivateInterrupted { window }
            | Self::Cancel { window }
            | Self::Step { window, .. } => window,
        }
    }
}

#[derive(Clone, Copy, Debug, Event)]
#[event(trigger = bevy::ecs::event::EntityTrigger)]
pub(crate) enum SemanticWidgetIntent {
    Activate {
        entity: Entity,
        window: Entity,
    },
    ActivateRelease {
        entity: Entity,
        window: Entity,
    },
    ActivateInterrupted {
        entity: Entity,
        window: Entity,
    },
    Cancel {
        entity: Entity,
    },
    Step {
        entity:    Entity,
        direction: WidgetStepDirection,
    },
}

impl EntityEvent for SemanticWidgetIntent {
    fn event_target(&self) -> Entity {
        match *self {
            Self::Activate { entity, .. }
            | Self::ActivateRelease { entity, .. }
            | Self::ActivateInterrupted { entity, .. }
            | Self::Cancel { entity }
            | Self::Step { entity, .. } => entity,
        }
    }
}

pub(in crate::widgets) fn route_semantic_input(
    mut widget_input: MessageReader<WidgetInput>,
    mut keyboard_input: Option<MessageReader<KeyboardInput>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut ime_input: Option<MessageWriter<Ime>>,
    input_blocker: Option<Res<ImeInputBlocker>>,
    mut authority: ResMut<WidgetFocusAuthority>,
    panels: Query<(&ComputedDiegeticPanel, &PanelWidgets)>,
    focusable_widgets: Query<(&PanelWidget, &WidgetOf), With<WidgetFocusable>>,
    action_targets: WidgetActionTargets,
    mut commands: Commands,
) {
    let mut starter_edit_outcome = if let (Some(keyboard_input), Some(ime_input)) =
        (keyboard_input.as_mut(), ime_input.as_mut())
    {
        route_keyboard_to_editable(
            keyboard_input,
            keys.as_deref(),
            ime_input,
            input_blocker.as_deref(),
            &authority,
            &action_targets,
            &mut commands,
        )
    } else {
        StarterEditOutcome::default()
    };

    for input in widget_input.read() {
        if !matches!(input, WidgetInput::ActivateInterrupted { .. })
            && !can_widget_input_act(
                input.action(),
                input.window(),
                &authority,
                &panels,
                &focusable_widgets,
                &action_targets,
            )
        {
            continue;
        }
        if let StarterActivationDisposition::RouteIfStarterEditFailed { window, widget } =
            starter_edit_outcome.activation_disposition(input, &authority)
        {
            route_activation_if_starter_edit_failed(window, widget, &mut commands);
            continue;
        }
        match *input {
            WidgetInput::FocusNext { window } => route_traversal(
                window,
                WidgetTraversal::Next,
                input_blocker.as_deref(),
                &mut authority,
                &panels,
                &focusable_widgets,
                &action_targets,
                &mut commands,
            ),
            WidgetInput::FocusPrevious { window } => route_traversal(
                window,
                WidgetTraversal::Previous,
                input_blocker.as_deref(),
                &mut authority,
                &panels,
                &focusable_widgets,
                &action_targets,
                &mut commands,
            ),
            WidgetInput::FocusFirst { window } => route_traversal(
                window,
                WidgetTraversal::First,
                input_blocker.as_deref(),
                &mut authority,
                &panels,
                &focusable_widgets,
                &action_targets,
                &mut commands,
            ),
            WidgetInput::FocusLast { window } => route_traversal(
                window,
                WidgetTraversal::Last,
                input_blocker.as_deref(),
                &mut authority,
                &panels,
                &focusable_widgets,
                &action_targets,
                &mut commands,
            ),
            WidgetInput::ActivatePress { window } => {
                route_activate_intent(window, input_blocker.as_deref(), &authority, &mut commands);
            },
            WidgetInput::ActivateRelease { window } => {
                if let WidgetFocusTarget::Widget(entity) = authority.focused_widget(window) {
                    commands.trigger(SemanticWidgetIntent::ActivateRelease { entity, window });
                }
            },
            WidgetInput::ActivateInterrupted { window } => {
                if let WidgetFocusTarget::Widget(entity) = authority.focused_widget(window) {
                    commands.trigger(SemanticWidgetIntent::ActivateInterrupted { entity, window });
                }
            },
            WidgetInput::Cancel { window } => {
                route_cancel_intent(window, input_blocker.as_deref(), &authority, &mut commands);
            },
            WidgetInput::Step { window, direction } => route_step_intent(
                window,
                direction,
                input_blocker.as_deref(),
                &authority,
                &mut commands,
            ),
        }
    }
}

fn route_step_intent(
    window: Entity,
    direction: WidgetStepDirection,
    input_blocker: Option<&ImeInputBlocker>,
    authority: &WidgetFocusAuthority,
    commands: &mut Commands<'_, '_>,
) {
    if input_blocker.is_some_and(|blocker| blocker.blocks_window(window)) {
        return;
    }
    if let WidgetFocusTarget::Widget(entity) = authority.focused_widget(window) {
        commands.trigger(SemanticWidgetIntent::Step { entity, direction });
    }
}

#[derive(Default)]
struct StarterEditOutcome {
    same_frame_commits: Vec<SameFrameStarterCommit>,
}

#[derive(Clone, Copy)]
struct SameFrameStarterCommit {
    window: Entity,
    widget: Entity,
}

enum StarterActivationDisposition {
    Route,
    RouteIfStarterEditFailed { window: Entity, widget: Entity },
}

impl StarterEditOutcome {
    fn record_same_frame_commit(&mut self, window: Entity, widget: Entity) {
        self.same_frame_commits
            .push(SameFrameStarterCommit { window, widget });
    }

    fn activation_disposition(
        &mut self,
        input: &WidgetInput,
        authority: &WidgetFocusAuthority,
    ) -> StarterActivationDisposition {
        let WidgetInput::ActivatePress { window } = *input else {
            return StarterActivationDisposition::Route;
        };
        let WidgetFocusTarget::Widget(widget) = authority.focused_widget(window) else {
            return StarterActivationDisposition::Route;
        };
        let Some(index) = self
            .same_frame_commits
            .iter()
            .position(|candidate| candidate.window == window && candidate.widget == widget)
        else {
            return StarterActivationDisposition::Route;
        };
        self.same_frame_commits.swap_remove(index);
        StarterActivationDisposition::RouteIfStarterEditFailed { window, widget }
    }
}

fn route_activation_if_starter_edit_failed(
    window: Entity,
    widget: Entity,
    commands: &mut Commands<'_, '_>,
) {
    commands.queue(move |world: &mut World| {
        let starter_edit_started = world
            .get_resource::<ImeInputBlocker>()
            .is_some_and(|blocker| blocker.blocks_window(window));
        if !starter_edit_started {
            world.trigger(SemanticWidgetIntent::Activate {
                entity: widget,
                window,
            });
        }
    });
}

fn route_keyboard_to_editable(
    keyboard_input: &mut MessageReader<'_, '_, KeyboardInput>,
    keys: Option<&ButtonInput<KeyCode>>,
    ime_input: &mut MessageWriter<'_, Ime>,
    input_blocker: Option<&ImeInputBlocker>,
    authority: &WidgetFocusAuthority,
    action_targets: &WidgetActionTargets,
    commands: &mut Commands<'_, '_>,
) -> StarterEditOutcome {
    let events = keyboard_input.read().collect::<Vec<_>>();
    let mut outcome = StarterEditOutcome::default();
    for &event in &events {
        let Some(text) = editable_starter_text(event) else {
            continue;
        };
        if input_blocker.is_some_and(|blocker| blocker.session_id().is_some())
            || command_modifier_pressed(keys)
        {
            continue;
        }
        let WidgetFocusTarget::Widget(entity) = authority.focused_widget(event.window) else {
            continue;
        };
        if !action_targets
            .get(entity)
            .is_ok_and(|(kind, disabled, _, _)| *kind == WidgetKind::EditableField && !disabled)
        {
            continue;
        }
        commands.trigger(SemanticWidgetIntent::Activate {
            entity,
            window: event.window,
        });
        if enter_pressed_in_window(&events, event.window) {
            outcome.record_same_frame_commit(event.window, entity);
            commit_started_edit_after_first_text(event.window, commands);
        }
        ime_input.write(Ime::Commit {
            window: event.window,
            value:  text.to_owned(),
        });
    }
    outcome
}

fn route_traversal(
    window: Entity,
    traversal: WidgetTraversal,
    input_blocker: Option<&ImeInputBlocker>,
    authority: &mut WidgetFocusAuthority,
    panels: &Query<(&ComputedDiegeticPanel, &PanelWidgets)>,
    focusable_widgets: &Query<(&PanelWidget, &WidgetOf), With<WidgetFocusable>>,
    action_targets: &WidgetActionTargets,
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
    let WidgetFocusTarget::Widget(widget) = current else {
        return;
    };
    let Ok((kind, disabled, _, _)) = action_targets.get(widget) else {
        return;
    };
    if *kind != WidgetKind::EditableField || disabled {
        return;
    }
    commands.trigger(SemanticWidgetIntent::Activate {
        entity: widget,
        window,
    });
}

fn enter_pressed_in_window(events: &[&KeyboardInput], window: Entity) -> bool {
    events.iter().any(|event| {
        event.window == window
            && event.state == ButtonState::Pressed
            && !event.repeat
            && matches!(event.key_code, KeyCode::Enter | KeyCode::NumpadEnter)
    })
}

fn commit_started_edit_after_first_text(window: Entity, commands: &mut Commands<'_, '_>) {
    commands.queue(move |world: &mut World| {
        let blocker = world.resource::<ImeInputBlocker>();
        let Some(session_id) = blocker.session_id() else {
            return;
        };
        if !blocker.blocks_window(window) {
            return;
        }
        world.add_observer(move |changed: On<ImeTextChanged>, mut commands: Commands| {
            if changed.session_id != session_id {
                return;
            }
            commands.entity(changed.observer()).despawn();
            commands.trigger(ImeRequestCommit {
                session_id,
                cause: ImeCommitCause::Enter,
            });
        });
    });
}

fn editable_starter_text(event: &KeyboardInput) -> Option<&str> {
    if event.state != ButtonState::Pressed
        || !matches!(
            event.key_code,
            KeyCode::Digit0
                | KeyCode::Digit1
                | KeyCode::Digit2
                | KeyCode::Digit3
                | KeyCode::Digit4
                | KeyCode::Digit5
                | KeyCode::Digit6
                | KeyCode::Digit7
                | KeyCode::Digit8
                | KeyCode::Digit9
                | KeyCode::Numpad0
                | KeyCode::Numpad1
                | KeyCode::Numpad2
                | KeyCode::Numpad3
                | KeyCode::Numpad4
                | KeyCode::Numpad5
                | KeyCode::Numpad6
                | KeyCode::Numpad7
                | KeyCode::Numpad8
                | KeyCode::Numpad9
                | KeyCode::Minus
                | KeyCode::NumpadSubtract
                | KeyCode::Period
                | KeyCode::NumpadDecimal
        )
    {
        return None;
    }
    event.text.as_deref().filter(|text| {
        matches!(
            *text,
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "-" | "."
        )
    })
}

fn command_modifier_pressed(keys: Option<&ButtonInput<KeyCode>>) -> bool {
    keys.is_some_and(|keys| {
        keys.pressed(KeyCode::ControlLeft)
            || keys.pressed(KeyCode::ControlRight)
            || keys.pressed(KeyCode::SuperLeft)
            || keys.pressed(KeyCode::SuperRight)
    })
}

fn route_activate_intent(
    window: Entity,
    input_blocker: Option<&ImeInputBlocker>,
    authority: &WidgetFocusAuthority,
    commands: &mut Commands<'_, '_>,
) {
    if input_blocker.is_some_and(|blocker| blocker.blocks_window(window)) {
        return;
    }
    let WidgetFocusTarget::Widget(widget) = authority.focused_widget(window) else {
        return;
    };
    commands.trigger(SemanticWidgetIntent::Activate {
        entity: widget,
        window,
    });
}

fn route_cancel_intent(
    window: Entity,
    input_blocker: Option<&ImeInputBlocker>,
    authority: &WidgetFocusAuthority,
    commands: &mut Commands<'_, '_>,
) {
    if input_blocker.is_some_and(|blocker| blocker.blocks_window(window)) {
        return;
    }
    let WidgetFocusTarget::Widget(widget) = authority.focused_widget(window) else {
        return;
    };
    commands.trigger(SemanticWidgetIntent::Cancel { entity: widget });
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::camera::RenderTarget;
    use bevy::diagnostic::FrameCount;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Camera;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::KeyCode;
    use bevy::prelude::Messages;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PointerPress;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;
    use bevy::prelude::Window;
    use bevy::window::Ime;
    use bevy::window::WindowClosed;
    use bevy::window::WindowFocused;
    use bevy::window::WindowRef;

    use super::SemanticWidgetIntent;
    use super::WidgetInput;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeApplied;
    use crate::ImeAppliedResult;
    use crate::ImeBufferSnapshot;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeBuiltInValue;
    use crate::ImeCommitCause;
    use crate::ImeCursorState;
    use crate::ImeEditableFieldSpec;
    use crate::ImeInputBlocker;
    use crate::ImeOpenSession;
    use crate::ImeRequestCommit;
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
    use crate::widgets::WidgetFocusTarget;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::interaction::focus::WidgetFocusVisible;

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
        Step,
    }

    #[derive(Default, Resource)]
    struct RecordedIntents(Vec<(Entity, RecordedIntentAction)>);

    #[derive(Default, Resource)]
    struct ImeLeaseAfterInput(Option<Entity>);

    #[derive(Default, Resource)]
    struct RecordedImeTextChanges(Vec<ImeBufferSnapshot>);

    #[derive(Default, Resource)]
    struct RecordedBuiltInValues(Vec<ImeBuiltInValue>);

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

    fn record_applied_value(applied: On<ImeApplied>, mut values: ResMut<RecordedBuiltInValues>) {
        if let ImeAppliedResult::BuiltIn(result) = &applied.result {
            values.0.push(result.value.clone());
        }
    }

    fn test_app(test_ime: TestIme) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<RecordedIntents>()
            .init_resource::<RecordedImeTextChanges>()
            .init_resource::<RecordedBuiltInValues>()
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .add_observer(record_intent)
            .add_observer(record_ime_text_change)
            .add_observer(record_applied_value);
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

    #[test]
    fn number_box_starter_key_table_accepts_only_numeric_edit_starters() {
        let accepted = [
            (KeyCode::Digit0, "0"),
            (KeyCode::Digit1, "1"),
            (KeyCode::Digit2, "2"),
            (KeyCode::Digit3, "3"),
            (KeyCode::Digit4, "4"),
            (KeyCode::Digit5, "5"),
            (KeyCode::Digit6, "6"),
            (KeyCode::Digit7, "7"),
            (KeyCode::Digit8, "8"),
            (KeyCode::Digit9, "9"),
            (KeyCode::Numpad0, "0"),
            (KeyCode::Numpad1, "1"),
            (KeyCode::Numpad2, "2"),
            (KeyCode::Numpad3, "3"),
            (KeyCode::Numpad4, "4"),
            (KeyCode::Numpad5, "5"),
            (KeyCode::Numpad6, "6"),
            (KeyCode::Numpad7, "7"),
            (KeyCode::Numpad8, "8"),
            (KeyCode::Numpad9, "9"),
            (KeyCode::Minus, "-"),
            (KeyCode::NumpadSubtract, "-"),
            (KeyCode::Period, "."),
            (KeyCode::NumpadDecimal, "."),
        ];
        for (key_code, text) in accepted {
            let event = KeyboardInput {
                key_code,
                logical_key: Key::Unidentified(NativeKey::Unidentified),
                state: ButtonState::Pressed,
                text: Some(text.into()),
                repeat: false,
                window: Entity::PLACEHOLDER,
            };
            assert_eq!(super::editable_starter_text(&event), Some(text));
        }

        let rejected = [
            (KeyCode::Digit0, None, ButtonState::Pressed),
            (KeyCode::Digit0, Some(""), ButtonState::Pressed),
            (KeyCode::Digit1, Some("10"), ButtonState::Pressed),
            (KeyCode::Digit1, Some("e"), ButtonState::Pressed),
            (KeyCode::Digit1, Some(","), ButtonState::Pressed),
            (KeyCode::Digit1, Some("\u{2212}"), ButtonState::Pressed),
            (KeyCode::KeyA, Some("1"), ButtonState::Pressed),
            (KeyCode::Digit1, Some("1"), ButtonState::Released),
        ];
        for (key_code, text, state) in rejected {
            let event = KeyboardInput {
                key_code,
                logical_key: Key::Unidentified(NativeKey::Unidentified),
                state,
                text: text.map(Into::into),
                repeat: false,
                window: Entity::PLACEHOLDER,
            };
            assert_eq!(super::editable_starter_text(&event), None);
        }
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
                crate::TextStyle::new(10.0),
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
                crate::TextStyle::new(10.0),
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

    fn spawn_number_box_panel(app: &mut App) -> Entity {
        let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
        builder.with(
            El::new().editable_field(
                "number",
                ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                    ImeBuiltInFieldKind::Float {
                        min: None,
                        max: None,
                    },
                )),
                crate::TextStyle::new(10.0),
            ),
            |builder| {
                builder.text("5");
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

    fn press_text_key(app: &mut App, window: Entity, key_code: KeyCode, text: &'static str) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key_code);
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: Some(text.into()),
            repeat: false,
            window,
        });
    }

    fn pointer_focus(app: &mut App, window: Entity, camera: Entity, target: Entity) {
        let Some(window_ref) = WindowRef::Entity(window).normalize(None) else {
            return;
        };
        app.world_mut().trigger(PointerPress {
            entity:  target,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::Window(window_ref),
                    position: Vec2::ZERO,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(camera, 0.0, None, None),
            count:   1,
        });
        app.world_mut().flush();
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
        app.world_mut().trigger(PointerPress {
            entity:  target,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::Window(window_ref),
                    position: Vec2::ZERO,
                },
            ),
            button:  PointerButton::Primary,
            hit:     HitData::new(camera, 0.0, None, None),
            count:   1,
        });
        app.world_mut()
            .write_message(WidgetInput::ActivatePress { window });
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
            .write_message(WidgetInput::ActivatePress { window });
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
            WidgetFocusTarget::Widget(second)
        );

        app.world_mut().resource_mut::<RecordedIntents>().0.clear();
        request_focus(&mut app, window, first);
        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.world_mut()
            .write_message(WidgetInput::ActivatePress { window });
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
            .write_message(WidgetInput::ActivatePress { window });
        app.update();

        assert!(app.world().get::<WidgetDisabled>(target).is_some());
        assert!(app.world().get::<WidgetFocused>(target).is_some());
        assert_eq!(app.world().resource::<RecordedIntents>().0, []);
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
        app.world_mut().write_message(WidgetInput::ActivatePress {
            window: blocked_window,
        });
        app.world_mut().write_message(WidgetInput::ActivatePress {
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
            .write_message(WidgetInput::ActivatePress { window });
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
            WidgetFocusTarget::Widget(editable)
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
            WidgetFocusTarget::Widget(editable)
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
            WidgetFocusTarget::Widget(editable)
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
            WidgetFocusTarget::Widget(editable)
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
            WidgetFocusTarget::Widget(next)
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
            .write_message(WidgetInput::ActivatePress { window });
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
            .write_message(WidgetInput::ActivatePress { window });
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

    #[test]
    fn number_box_starter_and_enter_in_one_frame_saves_and_closes() {
        let mut app = test_app(TestIme::Present);
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                GlobalTransform::IDENTITY,
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_number_box_panel(&mut app);
        app.update();
        let number = widget(&mut app, panel, "number");
        pointer_focus(&mut app, window, camera, number);

        press_text_key(&mut app, window, KeyCode::Digit5, "5");
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        press_key(&mut app, window, KeyCode::Enter);
        app.update();

        assert_eq!(
            app.world().resource::<RecordedBuiltInValues>().0,
            vec![ImeBuiltInValue::Float(5.0)]
        );
        assert_eq!(app.world().resource::<ImeInputBlocker>().window(), None);
        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            WidgetFocusTarget::Unfocused
        );
    }

    #[test]
    fn number_box_widget_input_plugin_starter_and_enter_activate_once_and_close() {
        let mut app = test_app(TestIme::Present);
        app.add_plugins((InputPlugin, crate::WidgetInputPlugin));
        app.finish();
        app.cleanup();
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                GlobalTransform::IDENTITY,
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_number_box_panel(&mut app);
        app.update();
        let number = widget(&mut app, panel, "number");
        pointer_focus(&mut app, window, camera, number);

        press_text_key(&mut app, window, KeyCode::Digit5, "5");
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        press_key(&mut app, window, KeyCode::Enter);
        app.update();

        let activations = app
            .world()
            .resource::<RecordedIntents>()
            .0
            .iter()
            .filter(|(_, action)| *action == RecordedIntentAction::Activate)
            .count();
        assert_eq!(
            activations, 1,
            "one physical Enter must not reactivate the field"
        );
        assert_eq!(
            app.world().resource::<RecordedBuiltInValues>().0,
            vec![ImeBuiltInValue::Float(5.0)]
        );
        assert_eq!(app.world().resource::<ImeInputBlocker>().window(), None);
        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            WidgetFocusTarget::Unfocused
        );
    }

    #[test]
    fn number_box_starter_does_not_consume_activation_after_focus_moves() {
        let mut app = test_app(TestIme::Present);
        app.add_plugins((InputPlugin, crate::WidgetInputPlugin));
        app.finish();
        app.cleanup();
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                GlobalTransform::IDENTITY,
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_number_box_panel(&mut app);
        app.update();
        let number = widget(&mut app, panel, "number");
        let next = widget(&mut app, panel, "next");
        pointer_focus(&mut app, window, camera, number);

        press_text_key(&mut app, window, KeyCode::Digit5, "5");
        press_key(&mut app, window, KeyCode::Enter);
        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();

        assert!(
            app.world()
                .resource::<RecordedIntents>()
                .0
                .contains(&(next, RecordedIntentAction::Activate)),
            "the activation after traversal must reach the newly focused widget",
        );
    }

    #[test]
    fn number_box_enter_in_another_window_does_not_close_started_edit() {
        let mut app = test_app(TestIme::Present);
        let enter_window = app.world_mut().spawn(Window::default()).id();
        let edit_window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                GlobalTransform::IDENTITY,
                RenderTarget::Window(WindowRef::Entity(edit_window)),
            ))
            .id();
        let panel = spawn_number_box_panel(&mut app);
        app.update();
        let number = widget(&mut app, panel, "number");
        pointer_focus(&mut app, edit_window, camera, number);

        press_text_key(&mut app, edit_window, KeyCode::Digit5, "5");
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        press_key(&mut app, enter_window, KeyCode::Enter);
        let routed = app.world_mut().run_system_once(super::route_semantic_input);
        assert!(routed.is_ok());
        app.world_mut().flush();
        assert_eq!(
            app.world().resource::<ImeInputBlocker>().window(),
            Some(edit_window),
            "the starter should open the edit before later IME systems run",
        );
        let session_id = app
            .world()
            .resource::<ImeInputBlocker>()
            .session_id()
            .unwrap_or(crate::ImeSessionId::new(0));
        let snapshot = app
            .world()
            .resource::<RecordedImeTextChanges>()
            .0
            .last()
            .cloned();
        assert!(snapshot.is_some());
        let Some(snapshot) = snapshot else {
            return;
        };
        app.world_mut().trigger(ImeTextChanged {
            session_id,
            target: ImeTarget::WorldPanelField {
                panel,
                field_id: PanelElementId::named("number"),
            },
            snapshot,
        });
        app.world_mut().flush();

        assert_eq!(
            app.world().resource::<RecordedBuiltInValues>().0,
            Vec::<ImeBuiltInValue>::new()
        );
        assert_eq!(
            app.world().resource::<ImeInputBlocker>().window(),
            Some(edit_window)
        );
        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(edit_window),
            WidgetFocusTarget::Widget(number)
        );
    }

    #[test]
    fn number_box_rejected_enter_keeps_edit_and_pointer_focus() {
        let mut app = test_app(TestIme::Present);
        let window = app.world_mut().spawn(Window::default()).id();
        let camera = app
            .world_mut()
            .spawn((
                Camera::default(),
                GlobalTransform::IDENTITY,
                RenderTarget::Window(WindowRef::Entity(window)),
            ))
            .id();
        let panel = spawn_number_box_panel(&mut app);
        app.update();
        let number = widget(&mut app, panel, "number");
        pointer_focus(&mut app, window, camera, number);
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::WorldPanelField {
                panel,
                field_id: PanelElementId::named("number"),
            },
            window,
            initial_text: "-".to_owned(),
            field_spec: ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(
                ImeBuiltInFieldKind::Float {
                    min: None,
                    max: None,
                },
            )),
            anchor: None,
        });
        app.world_mut().flush();
        let session_id = app
            .world()
            .resource::<ImeInputBlocker>()
            .session_id()
            .unwrap_or(crate::ImeSessionId::new(0));
        app.world_mut().trigger(ImeRequestCommit {
            session_id,
            cause: ImeCommitCause::Enter,
        });
        app.world_mut().flush();

        assert_eq!(
            app.world().resource::<ImeInputBlocker>().window(),
            Some(window)
        );
        assert_eq!(
            app.world().resource::<RecordedBuiltInValues>().0,
            Vec::<ImeBuiltInValue>::new()
        );
        assert_eq!(
            app.world()
                .resource::<WidgetFocusAuthority>()
                .focused_widget(window),
            WidgetFocusTarget::Widget(number)
        );
    }
}
