//! Window IME and keyboard input routing for the active edit session.

use std::borrow::Cow;

use bevy::diagnostic::FrameCount;
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::ButtonInput;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MessageReader;
use bevy::prelude::Messages;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::World;
use bevy::window::Ime;

use super::ActiveImeSession;
use super::ImeApplied;
use super::ImeCancelCause;
use super::ImeCanceled;
use super::ImeCommitCause;
use super::ImeInputBlocker;
use super::ImeRequestCancel;
use super::ImeSessionId;
use super::ImeTarget;
use super::ImeValidationRejected;
use super::ImeVerticalArrow;
use super::ImeVerticalArrowPressed;
use super::buffer::ImeEditCommand;
use super::buffer::ImeMovementDirection;
use super::buffer::ImeMovementUnit;
use super::buffer::ImeSelectionMode;
use super::events::ImeTextChanged;
use super::field::ImeBlurBehavior;
use crate::widgets::WidgetFocusAuthority;
use crate::widgets::WidgetFocusTarget;
use crate::widgets::WidgetInput;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextKeyConvention {
    Mac,
    Pc,
}

/// Whether an editing key consumes the event or leaves it for other input handling.
#[derive(Clone, Debug, Eq, PartialEq)]
enum KeyboardEditDecision {
    Edit(ImeEditCommand),
    ContinueInput,
}

/// The platform's editing-key convention; both tables compile and are tested everywhere.
const TEXT_KEY_CONVENTION: TextKeyConvention = if cfg!(target_os = "macos") {
    TextKeyConvention::Mac
} else {
    TextKeyConvention::Pc
};

/// Per-frame IME input routing state.
#[derive(Resource, Clone, Debug, Default)]
pub(super) struct ImeInputFrame {
    saw_platform_ime: bool,
}

/// Widget traversal deferred until a panel field commits or cancels on Tab.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub(super) struct PendingImeTraversal(Option<(ImeSessionId, WidgetInput)>);

/// Window toggled by the previous IME update pass.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub(super) struct ImeWindowState {
    active_window: Option<Entity>,
}

/// Synchronous app-owned keyboard routing hook.
#[derive(Resource, Default)]
pub struct ImeAppInputDispositionHook {
    handler: Option<Box<ImeAppInputHandler>>,
}

impl ImeAppInputDispositionHook {
    /// Installs the app-owned input hook.
    pub fn set(
        &mut self,
        handler: impl for<'a> FnMut(ImeAppInputContext<'a>) -> ImeAppInputDisposition
        + Send
        + Sync
        + 'static,
    ) {
        self.handler = Some(Box::new(handler));
    }

    /// Clears the app-owned input hook.
    pub fn clear(&mut self) { self.handler = None; }

    fn disposition(&mut self, context: ImeAppInputContext<'_>) -> ImeAppInputDisposition {
        let Some(handler) = self.handler.as_mut() else {
            return ImeAppInputDisposition::Edit;
        };
        handler(context)
    }
}

type ImeAppInputHandler =
    dyn for<'a> FnMut(ImeAppInputContext<'a>) -> ImeAppInputDisposition + Send + Sync + 'static;

/// Keyboard input context passed to the app-owned input hook.
pub struct ImeAppInputContext<'a> {
    /// Id of the active session.
    pub session_id: ImeSessionId,
    /// App-owned semantic target.
    pub target:     &'a ImeTarget,
    /// Current key button state.
    pub keys:       &'a ButtonInput<KeyCode>,
    /// Keyboard events observed this frame.
    pub events:     &'a [KeyboardInput],
}

/// App-owned input decision for a keyboard frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImeAppInputDisposition {
    /// Let the built-in editor consume the input.
    Edit,
    /// The app surface consumed the input; built-in editing should not run.
    Surface,
    /// Request commit for the app-owned session.
    Commit,
    /// Request cancellation for the app-owned session.
    Cancel,
}

pub(super) fn clear_frame_input(mut frame: ResMut<ImeInputFrame>) {
    frame.saw_platform_ime = false;
}

pub(super) fn update_window_ime(
    active_session: Res<ActiveImeSession>,
    mut window_state: ResMut<ImeWindowState>,
    mut windows: Query<&mut Window>,
) {
    let active_window = active_session.active_window();
    if let Some(previous) = window_state.active_window
        && Some(previous) != active_window
        && let Ok(mut window) = windows.get_mut(previous)
    {
        window.ime_enabled = false;
    }

    if let Some(entity) = active_window
        && let Ok(mut window) = windows.get_mut(entity)
    {
        window.ime_enabled = true;
    }

    window_state.active_window = active_window;
}

pub(super) fn handle_window_ime(
    mut ime_events: MessageReader<Ime>,
    mut active_session: ResMut<ActiveImeSession>,
    input_blocker: Res<ImeInputBlocker>,
    mut frame: ResMut<ImeInputFrame>,
    mut commands: Commands,
) {
    for event in ime_events.read() {
        let changed = match event {
            Ime::Preedit {
                window,
                value,
                cursor,
            } => active_session.apply_preedit(*window, value, *cursor, &input_blocker),
            Ime::Commit { window, value } => {
                active_session.apply_ime_commit(*window, value, &input_blocker)
            },
            Ime::Enabled { window } | Ime::Disabled { window } => {
                active_session.clear_preedit(*window, &input_blocker)
            },
        };
        if changed.is_some() {
            frame.saw_platform_ime = true;
        }
        trigger_text_changed(changed, &mut commands);
    }
}

pub(super) fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut key_events: MessageReader<KeyboardInput>,
    mut active_session: ResMut<ActiveImeSession>,
    input_blocker: Res<ImeInputBlocker>,
    frame_count: Option<Res<FrameCount>>,
    mut app_hook: ResMut<ImeAppInputDispositionHook>,
    mut pending_traversal: ResMut<PendingImeTraversal>,
    frame: Res<ImeInputFrame>,
    mut commands: Commands,
) {
    if !active_session.is_leased(&input_blocker) {
        key_events.clear();
        return;
    }

    // Consume activation keys, but keep unrelated text typed in the same frame
    // as a pointer activation. Otherwise the first minus or digit disappears.
    let captured_activation = frame_count
        .as_ref()
        .is_some_and(|count| input_blocker.captured_activation_frame(count.0));
    if captured_activation {
        if !frame.saw_platform_ime && !active_session.is_composing() {
            insert_activation_frame_text(
                &keys,
                &mut key_events,
                &mut active_session,
                &input_blocker,
                &mut commands,
            );
        } else {
            key_events.clear();
        }
        return;
    }

    if active_session.is_composing() {
        if keys.just_pressed(KeyCode::Escape) {
            let changed = active_session.clear_active_preedit(&input_blocker);
            trigger_text_changed(changed, &mut commands);
        }
        key_events.clear();
        return;
    }

    let events: Vec<KeyboardInput> = key_events.read().cloned().collect();
    if let Some(disposition) =
        app_disposition(&mut app_hook, &active_session, &keys, events.as_slice())
        && apply_app_disposition(disposition, &active_session, &mut commands)
    {
        return;
    }

    if active_session.commits_on_tab()
        && let Some((session_id, input)) = panel_traversal_from_tab(&keys, &active_session)
    {
        pending_traversal.0 = Some((session_id, input));
        commit_before_widget_traversal(session_id, input.window(), &mut commands);
        key_events.clear();
        return;
    }

    if let Some((session_id, input)) = panel_traversal_from_tab(&keys, &active_session) {
        match active_session.blur_behavior() {
            ImeBlurBehavior::KeepEditing => {},
            ImeBlurBehavior::Cancel => {
                pending_traversal.0 = Some((session_id, input));
                commands.trigger(ImeRequestCancel {
                    session_id,
                    cause: ImeCancelCause::Blur,
                });
            },
            ImeBlurBehavior::Commit => {
                pending_traversal.0 = Some((session_id, input));
                commands.trigger(super::ImeRequestCommit {
                    session_id,
                    cause: ImeCommitCause::Blur,
                });
            },
        }
        key_events.clear();
        return;
    }

    if let Some(request) = request_from_keys(&keys, &active_session) {
        if matches!(request, ImeSessionRequest::Commit(_)) {
            edit_before_enter(
                &keys,
                &events,
                &mut active_session,
                &input_blocker,
                &frame,
                &mut commands,
            );
        }
        trigger_session_request(request, &mut commands);
        return;
    }

    edit_keyboard_events(
        TEXT_KEY_CONVENTION,
        &keys,
        &events,
        &mut active_session,
        &input_blocker,
        &frame,
        &mut commands,
    );
}

fn edit_before_enter(
    keys: &ButtonInput<KeyCode>,
    events: &[KeyboardInput],
    active_session: &mut ActiveImeSession,
    input_blocker: &ImeInputBlocker,
    frame: &ImeInputFrame,
    commands: &mut Commands,
) {
    let end = events
        .iter()
        .position(|event| {
            event.state == ButtonState::Pressed
                && matches!(event.key_code, KeyCode::Enter | KeyCode::NumpadEnter)
                && Some(event.window) == active_session.active_window()
        })
        .unwrap_or(events.len());
    // Rewind later modifier transitions to retain their event-time state.
    let mut before_enter = keys.clone();
    for event in events[end..].iter().rev() {
        if is_text_modifier(event.key_code) && !event.repeat {
            match event.state {
                ButtonState::Pressed => before_enter.release(event.key_code),
                ButtonState::Released => before_enter.press(event.key_code),
            }
        }
    }
    edit_keyboard_events(
        TEXT_KEY_CONVENTION,
        &before_enter,
        &events[..end],
        active_session,
        input_blocker,
        frame,
        commands,
    );
}

fn edit_keyboard_events(
    convention: TextKeyConvention,
    keys: &ButtonInput<KeyCode>,
    events: &[KeyboardInput],
    active_session: &mut ActiveImeSession,
    input_blocker: &ImeInputBlocker,
    frame: &ImeInputFrame,
    commands: &mut Commands,
) {
    if !frame.saw_platform_ime && !active_session.is_pending_commit() {
        trigger_vertical_arrows(convention, keys, active_session, events, commands);
    }

    if let KeyboardEditDecision::Edit(command) =
        command_from_keyboard_events(convention, keys, events)
    {
        let changed = active_session.apply_edit_command(command, input_blocker);
        trigger_text_changed(changed, commands);
        return;
    }

    if frame.saw_platform_ime || active_session.is_pending_commit() {
        return;
    }

    insert_keyboard_text(keys, events, active_session, input_blocker, commands);
}

fn insert_activation_frame_text(
    keys: &ButtonInput<KeyCode>,
    key_events: &mut MessageReader<KeyboardInput>,
    active_session: &mut ActiveImeSession,
    input_blocker: &ImeInputBlocker,
    commands: &mut Commands,
) {
    let events = key_events
        .read()
        .filter(|event| {
            !matches!(
                event.key_code,
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    insert_keyboard_text(keys, &events, active_session, input_blocker, commands);
}

fn insert_keyboard_text(
    keys: &ButtonInput<KeyCode>,
    events: &[KeyboardInput],
    active_session: &mut ActiveImeSession,
    input_blocker: &ImeInputBlocker,
    commands: &mut Commands,
) {
    let text_events = keyboard_events_with_modifiers(keys, events, |event, modifiers| {
        if event.state != ButtonState::Pressed || command_modifier_pressed(modifiers) {
            return None;
        }
        let text = editable_keyboard_text(event.text.as_deref()?)?;
        Some((event.window, text))
    });
    for (window, text) in text_events.flatten() {
        let changed = active_session.apply_keyboard_text(window, text, input_blocker);
        trigger_text_changed(changed, commands);
    }
}

fn commit_before_widget_traversal(
    session_id: ImeSessionId,
    window: Entity,
    commands: &mut Commands,
) {
    commands.queue(move |world: &mut World| {
        // Keyboard traversal must start at the edited widget. A pointer-only
        // focus would otherwise be cleared by the accepted commit.
        if let Some(authority) = world.get_resource::<WidgetFocusAuthority>()
            && let WidgetFocusTarget::Widget(widget) = authority.focused_widget(window)
        {
            world.trigger(crate::RequestWidgetFocus { window, widget });
        }
        world.trigger(super::ImeRequestCommit {
            session_id,
            cause: ImeCommitCause::Blur,
        });
    });
}

fn panel_traversal_from_tab(
    keys: &ButtonInput<KeyCode>,
    active_session: &ActiveImeSession,
) -> Option<(ImeSessionId, WidgetInput)> {
    if !keys.just_pressed(KeyCode::Tab) || active_session.is_pending_commit() {
        return None;
    }
    if !matches!(
        active_session.active_target(),
        Some(ImeTarget::WorldPanelField { .. } | ImeTarget::ScreenPanelField { .. })
    ) {
        return None;
    }
    let window = active_session.active_window()?;
    let input = if shift_pressed(keys) {
        WidgetInput::FocusPrevious { window }
    } else {
        WidgetInput::FocusNext { window }
    };
    Some((active_session.active_session_id()?, input))
}

pub(super) fn continue_widget_traversal_after_apply(
    applied: On<ImeApplied>,
    mut pending: ResMut<PendingImeTraversal>,
    mut widget_input: Option<ResMut<Messages<WidgetInput>>>,
) {
    let Some((session_id, input)) = pending.0 else {
        return;
    };
    if session_id != applied.session_id {
        return;
    }
    pending.0 = None;
    if let Some(widget_input) = widget_input.as_deref_mut() {
        widget_input.write(input);
    }
}

pub(super) fn clear_widget_traversal_after_rejection(
    rejected: On<ImeValidationRejected>,
    mut pending: ResMut<PendingImeTraversal>,
) {
    if pending
        .0
        .is_some_and(|(session_id, _)| session_id == rejected.session_id)
    {
        pending.0 = None;
    }
}

pub(super) fn clear_widget_traversal_after_cancel(
    canceled: On<ImeCanceled>,
    mut pending: ResMut<PendingImeTraversal>,
    mut widget_input: Option<ResMut<Messages<WidgetInput>>>,
) {
    if let Some((session_id, input)) = pending.0
        && session_id == canceled.session_id
    {
        pending.0 = None;
        if canceled.cause == ImeCancelCause::Blur
            && let Some(widget_input) = widget_input.as_deref_mut()
        {
            widget_input.write(input);
        }
    }
}

/// Hands each Up or Down press in the session's window to the field's owner.
fn trigger_vertical_arrows(
    convention: TextKeyConvention,
    keys: &ButtonInput<KeyCode>,
    active_session: &ActiveImeSession,
    events: &[KeyboardInput],
    commands: &mut Commands,
) {
    let (Some(session_id), Some(window), Some(target)) = (
        active_session.active_session_id(),
        active_session.active_window(),
        active_session.active_target(),
    ) else {
        return;
    };
    for arrow in vertical_arrows(convention, keys, events, window) {
        commands.trigger(ImeVerticalArrowPressed {
            session_id,
            target: target.clone(),
            arrow,
        });
    }
}

fn vertical_arrows<'a>(
    convention: TextKeyConvention,
    keys: &'a ButtonInput<KeyCode>,
    events: &'a [KeyboardInput],
    window: Entity,
) -> impl Iterator<Item = ImeVerticalArrow> + 'a {
    keyboard_events_with_modifiers(keys, events, move |event, modifiers| {
        let command_moves_caret = convention == TextKeyConvention::Mac && super_pressed(modifiers);
        if command_moves_caret || event.state != ButtonState::Pressed || event.window != window {
            return None;
        }
        match event.key_code {
            KeyCode::ArrowUp => Some(ImeVerticalArrow::Up),
            KeyCode::ArrowDown => Some(ImeVerticalArrow::Down),
            _ => None,
        }
    })
    .flatten()
}

/// Rewinds Bevy's final button state, then routes each event with its own modifiers.
fn keyboard_events_with_modifiers<'a, T>(
    keys: &'a ButtonInput<KeyCode>,
    events: &'a [KeyboardInput],
    mut route: impl FnMut(&'a KeyboardInput, &ButtonInput<KeyCode>) -> T + 'a,
) -> impl Iterator<Item = T> + 'a {
    // Borrow the state on frames without modifier transitions; only changed
    // batches need a private copy. Repeats do not change a modifier's state.
    let mut modifiers = Cow::Borrowed(keys);
    for event in events.iter().rev() {
        if is_text_modifier(event.key_code) && !event.repeat {
            match event.state {
                ButtonState::Pressed => modifiers.to_mut().release(event.key_code),
                ButtonState::Released => modifiers.to_mut().press(event.key_code),
            }
        }
    }
    events.iter().map(move |event| {
        if is_text_modifier(event.key_code) && !event.repeat {
            match event.state {
                ButtonState::Pressed => modifiers.to_mut().press(event.key_code),
                ButtonState::Released => modifiers.to_mut().release(event.key_code),
            }
        }
        route(event, &modifiers)
    })
}

const fn is_text_modifier(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::ShiftLeft
            | KeyCode::ShiftRight
            | KeyCode::ControlLeft
            | KeyCode::ControlRight
            | KeyCode::AltLeft
            | KeyCode::AltRight
            | KeyCode::SuperLeft
            | KeyCode::SuperRight
    )
}

fn editable_keyboard_text(text: &str) -> Option<&str> {
    text.chars()
        .any(|character| !character.is_control())
        .then_some(text)
}

fn app_disposition(
    app_hook: &mut ImeAppInputDispositionHook,
    active_session: &ActiveImeSession,
    keys: &ButtonInput<KeyCode>,
    events: &[KeyboardInput],
) -> Option<ImeAppInputDisposition> {
    let session_id = active_session.active_session_id()?;
    let target = active_session.active_target()?;
    if !matches!(target, ImeTarget::AppOwned { .. }) {
        return None;
    }
    Some(app_hook.disposition(ImeAppInputContext {
        session_id,
        target,
        keys,
        events,
    }))
}

fn apply_app_disposition(
    disposition: ImeAppInputDisposition,
    active_session: &ActiveImeSession,
    commands: &mut Commands,
) -> bool {
    let Some(session_id) = active_session.active_session_id() else {
        return true;
    };
    match disposition {
        ImeAppInputDisposition::Edit => false,
        ImeAppInputDisposition::Surface => true,
        ImeAppInputDisposition::Commit => {
            commands.trigger(super::ImeRequestCommit {
                session_id,
                cause: ImeCommitCause::Request,
            });
            true
        },
        ImeAppInputDisposition::Cancel => {
            commands.trigger(ImeRequestCancel {
                session_id,
                cause: ImeCancelCause::Request,
            });
            true
        },
    }
}

fn request_from_keys(
    keys: &ButtonInput<KeyCode>,
    active_session: &ActiveImeSession,
) -> Option<ImeSessionRequest> {
    let session_id = active_session.active_session_id()?;
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        return Some(ImeSessionRequest::Commit(super::ImeRequestCommit {
            session_id,
            cause: ImeCommitCause::Enter,
        }));
    }

    keys.just_pressed(KeyCode::Escape)
        .then_some(ImeSessionRequest::Cancel(ImeRequestCancel {
            session_id,
            cause: ImeCancelCause::Escape,
        }))
}

fn command_from_keyboard_events(
    convention: TextKeyConvention,
    keys: &ButtonInput<KeyCode>,
    events: &[KeyboardInput],
) -> KeyboardEditDecision {
    keyboard_events_with_modifiers(keys, events, move |event, modifiers| {
        if event.state == ButtonState::Pressed {
            command_from_key_code(convention, modifiers, event.key_code)
        } else {
            KeyboardEditDecision::ContinueInput
        }
    })
    .find(|decision| matches!(decision, KeyboardEditDecision::Edit(_)))
    .unwrap_or(KeyboardEditDecision::ContinueInput)
}

fn command_from_key_code(
    convention: TextKeyConvention,
    keys: &ButtonInput<KeyCode>,
    key_code: KeyCode,
) -> KeyboardEditDecision {
    let selection = if shift_pressed(keys) {
        ImeSelectionMode::Extend
    } else {
        ImeSelectionMode::Move
    };
    let (direction, unit) = match (convention, key_code) {
        (TextKeyConvention::Pc, _) if super_pressed(keys) => {
            return KeyboardEditDecision::ContinueInput;
        },
        (TextKeyConvention::Pc, KeyCode::KeyA) if control_pressed(keys) => {
            return KeyboardEditDecision::Edit(ImeEditCommand::SelectAll);
        },
        (TextKeyConvention::Mac, KeyCode::KeyA) if super_pressed(keys) => {
            return KeyboardEditDecision::Edit(ImeEditCommand::SelectAll);
        },
        (TextKeyConvention::Mac, KeyCode::KeyA) if control_pressed(keys) => {
            (ImeMovementDirection::Backward, ImeMovementUnit::Line)
        },
        (TextKeyConvention::Mac, KeyCode::KeyE) if control_pressed(keys) => {
            (ImeMovementDirection::Forward, ImeMovementUnit::Line)
        },
        (TextKeyConvention::Mac, KeyCode::ArrowLeft | KeyCode::ArrowRight)
            if control_pressed(keys) =>
        {
            return KeyboardEditDecision::ContinueInput;
        },
        (TextKeyConvention::Mac, KeyCode::ArrowUp) if super_pressed(keys) => {
            (ImeMovementDirection::Backward, ImeMovementUnit::Line)
        },
        (TextKeyConvention::Mac, KeyCode::ArrowDown) if super_pressed(keys) => {
            (ImeMovementDirection::Forward, ImeMovementUnit::Line)
        },
        (_, KeyCode::Home) => (ImeMovementDirection::Backward, ImeMovementUnit::Line),
        (_, KeyCode::End) => (ImeMovementDirection::Forward, ImeMovementUnit::Line),
        (_, KeyCode::ArrowLeft) => (
            ImeMovementDirection::Backward,
            movement_unit(convention, keys),
        ),
        (_, KeyCode::ArrowRight) => (
            ImeMovementDirection::Forward,
            movement_unit(convention, keys),
        ),
        (_, KeyCode::Backspace) => {
            return KeyboardEditDecision::Edit(ImeEditCommand::DeleteBackward(delete_unit(
                convention, keys,
            )));
        },
        (_, KeyCode::Delete) => {
            return KeyboardEditDecision::Edit(ImeEditCommand::DeleteForward(delete_unit(
                convention, keys,
            )));
        },
        _ => return KeyboardEditDecision::ContinueInput,
    };
    KeyboardEditDecision::Edit(ImeEditCommand::Move {
        direction,
        unit,
        selection,
    })
}

fn movement_unit(convention: TextKeyConvention, keys: &ButtonInput<KeyCode>) -> ImeMovementUnit {
    match convention {
        TextKeyConvention::Mac if super_pressed(keys) => ImeMovementUnit::Line,
        TextKeyConvention::Mac if alt_pressed(keys) => ImeMovementUnit::Word,
        TextKeyConvention::Pc if control_pressed(keys) => ImeMovementUnit::Word,
        _ => ImeMovementUnit::Character,
    }
}

fn delete_unit(convention: TextKeyConvention, keys: &ButtonInput<KeyCode>) -> ImeMovementUnit {
    movement_unit(convention, keys)
}

fn trigger_text_changed(changed: Option<ImeTextChanged>, commands: &mut Commands) {
    if let Some(changed) = changed {
        commands.trigger(changed);
    }
}

fn shift_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)
}

fn command_modifier_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    control_pressed(keys) || super_pressed(keys)
}

fn control_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight)
}

fn super_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::SuperLeft) || keys.pressed(KeyCode::SuperRight)
}

fn alt_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)
}

enum ImeSessionRequest {
    Commit(super::ImeRequestCommit),
    Cancel(ImeRequestCancel),
}

fn trigger_session_request(request: ImeSessionRequest, commands: &mut Commands) {
    match request {
        ImeSessionRequest::Commit(request) => commands.trigger(request),
        ImeSessionRequest::Cancel(request) => commands.trigger(request),
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::ButtonState;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::prelude::App;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Commands;
    use bevy::prelude::Entity;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MessageReader;
    use bevy::prelude::Messages;
    use bevy::prelude::On;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::World;

    use super::KeyboardEditDecision;
    use super::TextKeyConvention;
    use super::command_from_key_code;
    use super::command_from_keyboard_events;
    use super::editable_keyboard_text;
    use super::vertical_arrows;
    use crate::ime::ActiveImeSession;
    use crate::ime::ImeAppOwnedFieldSpec;
    use crate::ime::ImeCursorState;
    use crate::ime::ImeEditableFieldSpec;
    use crate::ime::ImeOpenSession;
    use crate::ime::ImePlugin;
    use crate::ime::ImeTarget;
    use crate::ime::ImeVerticalArrow;
    use crate::ime::buffer::ImeEditCommand;
    use crate::ime::buffer::ImeMovementDirection;
    use crate::ime::buffer::ImeMovementUnit;
    use crate::ime::buffer::ImeSelectionMode;

    fn keys(pressed: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut keys = ButtonInput::default();
        for key in pressed {
            keys.press(*key);
        }
        keys
    }

    fn key_event(key_code: KeyCode, repeat: bool) -> KeyboardInput {
        KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat,
            window: Entity::PLACEHOLDER,
        }
    }

    #[test]
    fn tab_in_a_keep_editing_field_keeps_the_session() -> Result<(), String> {
        assert_tab_keeps_editing(ImeAppOwnedFieldSpec::new("query").keep_editing_on_blur())
    }

    #[test]
    fn keep_editing_prevents_tab_commit_and_traversal_in_either_builder_order() -> Result<(), String>
    {
        for spec in [
            ImeAppOwnedFieldSpec::new("query")
                .commit_on_tab()
                .keep_editing_on_blur(),
            ImeAppOwnedFieldSpec::new("query")
                .keep_editing_on_blur()
                .commit_on_tab(),
        ] {
            assert_tab_keeps_editing(spec)?;
        }
        Ok(())
    }

    fn assert_tab_keeps_editing(spec: ImeAppOwnedFieldSpec) -> Result<(), String> {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(ImePlugin);
        let window = app.world_mut().spawn(bevy::prelude::Window::default()).id();
        let panel = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::ScreenPanelField {
                panel,
                field_id: "query".into(),
            },
            window,
            initial_text: "reset".to_owned(),
            field_spec: ImeEditableFieldSpec::AppOwned(spec),
            anchor: None,
        });
        app.world_mut().flush();
        let session = app
            .world()
            .resource::<ActiveImeSession>()
            .active_session_id();
        assert!(session.is_some());
        for pressed in [&[KeyCode::Tab][..], &[KeyCode::Tab, KeyCode::ShiftLeft][..]] {
            app.insert_resource(keys(pressed));
            app.world_mut()
                .run_system_once(super::handle_keyboard)
                .map_err(|error| error.to_string())?;
            app.world_mut().flush();
            let active = app.world().resource::<ActiveImeSession>();
            assert_eq!(active.active_session_id(), session);
            assert!(!active.is_pending_commit());
            assert!(
                app.world()
                    .resource::<super::PendingImeTraversal>()
                    .0
                    .is_none()
            );
        }
        Ok(())
    }

    #[test]
    fn mac_command_a_maps_to_select_all() {
        let keys = keys(&[KeyCode::SuperLeft]);

        assert_eq!(
            command_from_key_code(TextKeyConvention::Mac, &keys, KeyCode::KeyA),
            KeyboardEditDecision::Edit(ImeEditCommand::SelectAll)
        );
    }

    #[test]
    fn mac_alt_arrow_left_maps_to_word_movement() {
        let keys = keys(&[KeyCode::AltLeft]);

        assert_eq!(
            command_from_key_code(TextKeyConvention::Mac, &keys, KeyCode::ArrowLeft),
            KeyboardEditDecision::Edit(ImeEditCommand::Move {
                direction: ImeMovementDirection::Backward,
                unit:      ImeMovementUnit::Word,
                selection: ImeSelectionMode::Move,
            })
        );
    }

    #[test]
    fn shift_end_extends_to_line_end() {
        let keys = keys(&[KeyCode::ShiftLeft]);

        assert_eq!(
            command_from_key_code(TextKeyConvention::Mac, &keys, KeyCode::End),
            KeyboardEditDecision::Edit(ImeEditCommand::Move {
                direction: ImeMovementDirection::Forward,
                unit:      ImeMovementUnit::Line,
                selection: ImeSelectionMode::Extend,
            })
        );
    }

    #[test]
    fn backspace_maps_to_delete_backward() {
        let keys = keys(&[]);

        assert_eq!(
            command_from_key_code(TextKeyConvention::Mac, &keys, KeyCode::Backspace),
            KeyboardEditDecision::Edit(ImeEditCommand::DeleteBackward(ImeMovementUnit::Character))
        );
    }

    #[test]
    fn mac_command_backspace_maps_to_line_delete() {
        let keys = keys(&[KeyCode::SuperLeft]);

        assert_eq!(
            command_from_key_code(TextKeyConvention::Mac, &keys, KeyCode::Backspace),
            KeyboardEditDecision::Edit(ImeEditCommand::DeleteBackward(ImeMovementUnit::Line))
        );
    }

    #[test]
    fn mac_option_backspace_maps_to_word_delete() {
        let keys = keys(&[KeyCode::AltLeft]);

        assert_eq!(
            command_from_key_code(TextKeyConvention::Mac, &keys, KeyCode::Backspace),
            KeyboardEditDecision::Edit(ImeEditCommand::DeleteBackward(ImeMovementUnit::Word))
        );
    }

    #[test]
    fn repeated_backspace_maps_to_delete_backward() {
        let keys = keys(&[]);
        let events = [key_event(KeyCode::Backspace, true)];

        assert_eq!(
            command_from_keyboard_events(TextKeyConvention::Pc, &keys, &events),
            KeyboardEditDecision::Edit(ImeEditCommand::DeleteBackward(ImeMovementUnit::Character))
        );
    }

    #[test]
    fn vertical_arrows_count_each_press_and_repeat_in_the_session_window() {
        let session_window = Entity::PLACEHOLDER;
        let other_window = World::new().spawn_empty().id();
        let mut released_up = key_event(KeyCode::ArrowUp, false);
        released_up.state = ButtonState::Released;
        let mut other_window_down = key_event(KeyCode::ArrowDown, false);
        other_window_down.window = other_window;
        let events = [
            key_event(KeyCode::ArrowDown, false),
            key_event(KeyCode::ArrowDown, true),
            released_up,
            other_window_down,
            key_event(KeyCode::ArrowLeft, false),
            key_event(KeyCode::ArrowUp, false),
        ];

        assert_eq!(
            vertical_arrows(TextKeyConvention::Pc, &keys(&[]), &events, session_window)
                .collect::<Vec<_>>(),
            [
                ImeVerticalArrow::Down,
                ImeVerticalArrow::Down,
                ImeVerticalArrow::Up
            ]
        );
    }

    #[test]
    fn control_only_keyboard_text_is_ignored() {
        assert_eq!(editable_keyboard_text("\u{8}"), None);
        assert_eq!(editable_keyboard_text("\u{7f}"), None);
        assert_eq!(editable_keyboard_text("a"), Some("a"));
    }

    fn assert_moves(
        convention: TextKeyConvention,
        modifiers: &[KeyCode],
        key: KeyCode,
        direction: ImeMovementDirection,
        unit: ImeMovementUnit,
    ) {
        for (shift, selection) in [
            (false, ImeSelectionMode::Move),
            (true, ImeSelectionMode::Extend),
        ] {
            let mut held = keys(modifiers);
            if shift {
                held.press(KeyCode::ShiftRight);
            }
            assert_eq!(
                command_from_key_code(convention, &held, key),
                KeyboardEditDecision::Edit(ImeEditCommand::Move {
                    direction,
                    unit,
                    selection
                }),
                "{convention:?}, {modifiers:?}, {key:?}, shift={shift}"
            );
        }
    }

    #[test]
    fn mac_control_a_moves_to_line_start() {
        for modifier in [KeyCode::ControlLeft, KeyCode::ControlRight] {
            assert_moves(
                TextKeyConvention::Mac,
                &[modifier],
                KeyCode::KeyA,
                ImeMovementDirection::Backward,
                ImeMovementUnit::Line,
            );
            assert_moves(
                TextKeyConvention::Mac,
                &[modifier],
                KeyCode::KeyE,
                ImeMovementDirection::Forward,
                ImeMovementUnit::Line,
            );
        }
    }

    #[test]
    fn pc_control_a_selects_all() {
        for modifier in [KeyCode::ControlLeft, KeyCode::ControlRight] {
            assert_eq!(
                command_from_key_code(TextKeyConvention::Pc, &keys(&[modifier]), KeyCode::KeyA),
                KeyboardEditDecision::Edit(ImeEditCommand::SelectAll)
            );
        }
    }

    #[test]
    fn pc_super_left_does_nothing() {
        for modifier in [KeyCode::SuperLeft, KeyCode::SuperRight] {
            for key in [
                KeyCode::KeyA,
                KeyCode::KeyE,
                KeyCode::Home,
                KeyCode::End,
                KeyCode::ArrowLeft,
                KeyCode::ArrowRight,
                KeyCode::ArrowUp,
                KeyCode::ArrowDown,
                KeyCode::Backspace,
                KeyCode::Delete,
            ] {
                for shift in [false, true] {
                    let mut held = keys(&[modifier]);
                    if shift {
                        held.press(KeyCode::ShiftLeft);
                    }
                    assert_eq!(
                        command_from_key_code(TextKeyConvention::Pc, &held, key),
                        KeyboardEditDecision::ContinueInput,
                        "{modifier:?}, {key:?}, shift={shift}"
                    );
                }
            }
        }
    }

    #[test]
    fn mac_control_left_does_nothing() {
        for modifier in [KeyCode::ControlLeft, KeyCode::ControlRight] {
            for key in [KeyCode::ArrowLeft, KeyCode::ArrowRight] {
                for shift in [false, true] {
                    let mut held = keys(&[modifier]);
                    if shift {
                        held.press(KeyCode::ShiftLeft);
                    }
                    assert_eq!(
                        command_from_key_code(TextKeyConvention::Mac, &held, key),
                        KeyboardEditDecision::ContinueInput
                    );
                }
            }
        }
    }

    #[test]
    fn mac_command_left_moves_to_line_start() {
        for modifier in [KeyCode::SuperLeft, KeyCode::SuperRight] {
            for (key, direction) in [
                (KeyCode::ArrowLeft, ImeMovementDirection::Backward),
                (KeyCode::ArrowRight, ImeMovementDirection::Forward),
                (KeyCode::ArrowUp, ImeMovementDirection::Backward),
                (KeyCode::ArrowDown, ImeMovementDirection::Forward),
            ] {
                assert_moves(
                    TextKeyConvention::Mac,
                    &[modifier],
                    key,
                    direction,
                    ImeMovementUnit::Line,
                );
            }
        }
    }

    #[test]
    fn home_and_end_move_by_line_under_both_conventions() {
        for convention in [TextKeyConvention::Mac, TextKeyConvention::Pc] {
            for modifiers in [&[][..], &[KeyCode::ControlLeft][..]] {
                assert_moves(
                    convention,
                    modifiers,
                    KeyCode::Home,
                    ImeMovementDirection::Backward,
                    ImeMovementUnit::Line,
                );
                assert_moves(
                    convention,
                    modifiers,
                    KeyCode::End,
                    ImeMovementDirection::Forward,
                    ImeMovementUnit::Line,
                );
            }
        }
    }

    #[test]
    fn pc_control_arrows_and_mac_option_arrows_move_by_word() {
        for (convention, modifiers) in [
            (
                TextKeyConvention::Pc,
                [KeyCode::ControlLeft, KeyCode::ControlRight],
            ),
            (
                TextKeyConvention::Mac,
                [KeyCode::AltLeft, KeyCode::AltRight],
            ),
        ] {
            for modifier in modifiers {
                assert_moves(
                    convention,
                    &[modifier],
                    KeyCode::ArrowLeft,
                    ImeMovementDirection::Backward,
                    ImeMovementUnit::Word,
                );
                assert_moves(
                    convention,
                    &[modifier],
                    KeyCode::ArrowRight,
                    ImeMovementDirection::Forward,
                    ImeMovementUnit::Word,
                );
            }
        }
    }

    #[test]
    fn plain_edit_keys_use_characters_under_both_conventions() {
        for convention in [TextKeyConvention::Mac, TextKeyConvention::Pc] {
            assert_moves(
                convention,
                &[],
                KeyCode::ArrowLeft,
                ImeMovementDirection::Backward,
                ImeMovementUnit::Character,
            );
            assert_moves(
                convention,
                &[],
                KeyCode::ArrowRight,
                ImeMovementDirection::Forward,
                ImeMovementUnit::Character,
            );
            assert_eq!(
                command_from_key_code(convention, &keys(&[]), KeyCode::Backspace),
                KeyboardEditDecision::Edit(ImeEditCommand::DeleteBackward(
                    ImeMovementUnit::Character
                ))
            );
            assert_eq!(
                command_from_key_code(convention, &keys(&[]), KeyCode::Delete),
                KeyboardEditDecision::Edit(ImeEditCommand::DeleteForward(
                    ImeMovementUnit::Character
                ))
            );
        }
    }

    #[test]
    fn pc_alt_arrows_and_deletes_keep_character_units() {
        for modifier in [KeyCode::AltLeft, KeyCode::AltRight] {
            assert_moves(
                TextKeyConvention::Pc,
                &[modifier],
                KeyCode::ArrowLeft,
                ImeMovementDirection::Backward,
                ImeMovementUnit::Character,
            );
            assert_moves(
                TextKeyConvention::Pc,
                &[modifier],
                KeyCode::ArrowRight,
                ImeMovementDirection::Forward,
                ImeMovementUnit::Character,
            );
            assert_eq!(
                command_from_key_code(
                    TextKeyConvention::Pc,
                    &keys(&[modifier]),
                    KeyCode::Backspace
                ),
                KeyboardEditDecision::Edit(ImeEditCommand::DeleteBackward(
                    ImeMovementUnit::Character
                ))
            );
            assert_eq!(
                command_from_key_code(TextKeyConvention::Pc, &keys(&[modifier]), KeyCode::Delete),
                KeyboardEditDecision::Edit(ImeEditCommand::DeleteForward(
                    ImeMovementUnit::Character
                ))
            );
        }
    }

    #[test]
    fn mac_option_backspace_deletes_a_word() {
        assert_delete_units(
            TextKeyConvention::Mac,
            &[KeyCode::AltLeft, KeyCode::AltRight],
            ImeMovementUnit::Word,
        );
    }

    #[test]
    fn pc_control_backspace_deletes_a_word() {
        assert_delete_units(
            TextKeyConvention::Pc,
            &[KeyCode::ControlLeft, KeyCode::ControlRight],
            ImeMovementUnit::Word,
        );
    }

    #[test]
    fn mac_command_backspace_deletes_to_line_start() {
        assert_delete_units(
            TextKeyConvention::Mac,
            &[KeyCode::SuperLeft, KeyCode::SuperRight],
            ImeMovementUnit::Line,
        );
    }

    fn assert_delete_units(
        convention: TextKeyConvention,
        modifiers: &[KeyCode],
        unit: ImeMovementUnit,
    ) {
        for modifier in modifiers {
            assert_eq!(
                command_from_key_code(convention, &keys(&[*modifier]), KeyCode::Backspace),
                KeyboardEditDecision::Edit(ImeEditCommand::DeleteBackward(unit))
            );
            assert_eq!(
                command_from_key_code(convention, &keys(&[*modifier]), KeyCode::Delete),
                KeyboardEditDecision::Edit(ImeEditCommand::DeleteForward(unit))
            );
        }
    }

    #[test]
    fn mac_command_up_sends_no_vertical_arrow() {
        let events = [
            key_event(KeyCode::ArrowUp, false),
            key_event(KeyCode::ArrowDown, true),
        ];
        for modifier in [KeyCode::SuperLeft, KeyCode::SuperRight] {
            assert_eq!(
                vertical_arrows(
                    TextKeyConvention::Mac,
                    &keys(&[modifier]),
                    &events,
                    Entity::PLACEHOLDER
                )
                .count(),
                0
            );
        }
    }

    #[test]
    fn plain_up_sends_a_vertical_arrow_under_both_conventions() {
        let events = [
            key_event(KeyCode::ArrowUp, false),
            key_event(KeyCode::ArrowDown, true),
        ];
        for convention in [TextKeyConvention::Mac, TextKeyConvention::Pc] {
            for modifiers in [
                &[][..],
                &[KeyCode::ShiftLeft][..],
                &[KeyCode::ControlLeft][..],
                &[KeyCode::AltLeft][..],
            ] {
                assert_eq!(
                    vertical_arrows(convention, &keys(modifiers), &events, Entity::PLACEHOLDER)
                        .collect::<Vec<_>>(),
                    [ImeVerticalArrow::Up, ImeVerticalArrow::Down]
                );
            }
        }
        assert_eq!(
            vertical_arrows(
                TextKeyConvention::Pc,
                &keys(&[KeyCode::SuperLeft]),
                &events,
                Entity::PLACEHOLDER
            )
            .count(),
            2
        );
    }

    fn released(key_code: KeyCode) -> KeyboardInput {
        let mut event = key_event(key_code, false);
        event.state = ButtonState::Released;
        event
    }

    #[test]
    fn mac_command_up_uses_modifiers_held_at_the_arrow() {
        let events = [
            key_event(KeyCode::SuperLeft, false),
            key_event(KeyCode::ArrowUp, false),
            released(KeyCode::SuperLeft),
        ];
        assert_eq!(
            command_from_keyboard_events(TextKeyConvention::Mac, &keys(&[]), &events),
            KeyboardEditDecision::Edit(ImeEditCommand::Move {
                direction: ImeMovementDirection::Backward,
                unit:      ImeMovementUnit::Line,
                selection: ImeSelectionMode::Move,
            })
        );
        assert_eq!(
            vertical_arrows(
                TextKeyConvention::Mac,
                &keys(&[]),
                &events,
                Entity::PLACEHOLDER
            )
            .count(),
            0
        );
    }

    #[test]
    fn mac_up_before_command_press_still_steps_the_owner_row() {
        let events = [
            key_event(KeyCode::ArrowUp, false),
            key_event(KeyCode::SuperLeft, false),
        ];
        let held = keys(&[KeyCode::SuperLeft]);
        assert_eq!(
            vertical_arrows(TextKeyConvention::Mac, &held, &events, Entity::PLACEHOLDER)
                .collect::<Vec<_>>(),
            [ImeVerticalArrow::Up]
        );
        assert_eq!(
            command_from_keyboard_events(TextKeyConvention::Mac, &held, &events),
            KeyboardEditDecision::ContinueInput
        );
    }

    #[test]
    fn edit_commands_follow_control_option_and_shift_event_order() {
        for (convention, modifier, key, command) in [
            (
                TextKeyConvention::Pc,
                KeyCode::ControlLeft,
                KeyCode::KeyA,
                ImeEditCommand::SelectAll,
            ),
            (
                TextKeyConvention::Mac,
                KeyCode::AltRight,
                KeyCode::ArrowLeft,
                ImeEditCommand::Move {
                    direction: ImeMovementDirection::Backward,
                    unit:      ImeMovementUnit::Word,
                    selection: ImeSelectionMode::Move,
                },
            ),
            (
                TextKeyConvention::Pc,
                KeyCode::ShiftRight,
                KeyCode::End,
                ImeEditCommand::Move {
                    direction: ImeMovementDirection::Forward,
                    unit:      ImeMovementUnit::Line,
                    selection: ImeSelectionMode::Extend,
                },
            ),
        ] {
            let events = [
                key_event(modifier, false),
                key_event(key, false),
                released(modifier),
            ];
            assert_eq!(
                command_from_keyboard_events(convention, &keys(&[]), &events),
                KeyboardEditDecision::Edit(command),
                "{convention:?}, {modifier:?}"
            );
        }
    }

    #[test]
    fn held_modifier_before_batch_survives_its_repeat_and_later_release() {
        let events = [
            key_event(KeyCode::SuperRight, true),
            key_event(KeyCode::ArrowUp, true),
            released(KeyCode::SuperRight),
        ];
        assert_eq!(
            command_from_keyboard_events(TextKeyConvention::Mac, &keys(&[]), &events),
            KeyboardEditDecision::Edit(ImeEditCommand::Move {
                direction: ImeMovementDirection::Backward,
                unit:      ImeMovementUnit::Line,
                selection: ImeSelectionMode::Move,
            })
        );
        assert_eq!(
            vertical_arrows(
                TextKeyConvention::Mac,
                &keys(&[]),
                &events,
                Entity::PLACEHOLDER
            )
            .count(),
            0
        );
    }

    #[derive(bevy::prelude::Resource, Default)]
    struct ObservedBufferText(String);

    #[derive(bevy::prelude::Resource, Default)]
    struct ObservedCaretAndArrows {
        cursor: Option<ImeCursorState>,
        arrows: Vec<ImeVerticalArrow>,
    }

    #[test]
    fn mac_command_up_released_in_one_batch_moves_the_session_caret_without_owner_navigation()
    -> Result<(), String> {
        for modifier in [KeyCode::SuperLeft, KeyCode::SuperRight] {
            let (mut app, window) = keyboard_app("first last");
            app.init_resource::<ObservedCaretAndArrows>();
            app.add_observer(
                |changed: On<crate::ime::ImeTextChanged>,
                 mut observed: ResMut<ObservedCaretAndArrows>| {
                    observed.cursor = Some(changed.snapshot.cursor.clone());
                },
            );
            app.add_observer(
                |arrow: On<crate::ime::ImeVerticalArrowPressed>,
                 mut observed: ResMut<ObservedCaretAndArrows>| {
                    observed.arrows.push(arrow.arrow);
                },
            );
            app.insert_resource(keys(&[]));
            for mut event in [
                key_event(modifier, false),
                key_event(KeyCode::ArrowUp, false),
                released(modifier),
            ] {
                event.window = window;
                app.world_mut().write_message(event);
            }
            app.world_mut()
                .run_system_once(
                    |keys: Res<ButtonInput<KeyCode>>,
                     mut events: MessageReader<KeyboardInput>,
                     mut active: ResMut<ActiveImeSession>,
                     blocker: Res<crate::ime::ImeInputBlocker>,
                     frame: Res<super::ImeInputFrame>,
                     mut commands: Commands| {
                        let events = events.read().cloned().collect::<Vec<_>>();
                        super::edit_keyboard_events(
                            TextKeyConvention::Mac,
                            &keys,
                            &events,
                            &mut active,
                            &blocker,
                            &frame,
                            &mut commands,
                        );
                    },
                )
                .map_err(|error| error.to_string())?;
            app.world_mut().flush();
            let observed = app.world().resource::<ObservedCaretAndArrows>();
            assert!(matches!(
                observed.cursor,
                Some(crate::ime::ImeCursorState::Insertion(boundary)) if boundary.as_usize() == 0
            ));
            assert!(observed.arrows.is_empty(), "Cmd-Up navigated the owner row");
            assert_eq!(
                app.world().resource::<ActiveImeSession>().active_text(),
                Some("first last")
            );
        }
        Ok(())
    }

    #[test]
    fn text_insertion_suppresses_control_and_super_but_accepts_option() -> Result<(), String> {
        for (modifier, key, text, expected) in [
            (KeyCode::ControlLeft, KeyCode::KeyB, "b", ""),
            (KeyCode::ControlRight, KeyCode::KeyB, "b", ""),
            (KeyCode::SuperLeft, KeyCode::KeyB, "b", ""),
            (KeyCode::SuperRight, KeyCode::KeyB, "b", ""),
            (KeyCode::AltLeft, KeyCode::KeyE, "é", "é"),
            (KeyCode::AltRight, KeyCode::KeyE, "é", "é"),
        ] {
            let mut typed = key_event(key, false);
            typed.text = Some(text.into());
            // These printable keys must reach insertion, rather than an edit-command return.
            for convention in [TextKeyConvention::Pc, TextKeyConvention::Mac] {
                assert_eq!(
                    command_from_keyboard_events(
                        convention,
                        &keys(&[modifier]),
                        std::slice::from_ref(&typed)
                    ),
                    KeyboardEditDecision::ContinueInput,
                    "{convention:?}, {modifier:?}, {key:?} bypassed text insertion"
                );
            }
            assert_keyboard_buffer(&keys(&[modifier]), std::slice::from_ref(&typed), expected)?;
            // End-of-frame state has no modifier, but it was held at the text event.
            assert_keyboard_buffer(
                &keys(&[]),
                &[key_event(modifier, false), typed, released(modifier)],
                expected,
            )?;
        }
        Ok(())
    }

    #[test]
    fn enter_commits_text_before_it_in_the_same_keyboard_batch() -> Result<(), String> {
        let mut five = key_event(KeyCode::Digit5, false);
        five.text = Some("5".into());
        let mut six = key_event(KeyCode::Digit6, false);
        six.text = Some("6".into());
        assert_keyboard_buffer(
            &keys(&[KeyCode::Enter, KeyCode::SuperLeft]),
            &[
                five,
                key_event(KeyCode::Enter, false),
                six,
                key_event(KeyCode::SuperLeft, false),
            ],
            "5",
        )
    }

    fn assert_keyboard_buffer(
        held: &ButtonInput<KeyCode>,
        events: &[KeyboardInput],
        expected: &str,
    ) -> Result<(), String> {
        let (mut app, window) = keyboard_app("");
        app.insert_resource(held.clone());
        for event in events {
            let mut event = event.clone();
            event.window = window;
            app.world_mut()
                .resource_mut::<Messages<KeyboardInput>>()
                .write(event);
        }
        app.world_mut()
            .run_system_once(super::handle_keyboard)
            .map_err(|error| error.to_string())?;
        app.world_mut().flush();
        assert_eq!(
            app.world().resource::<ObservedBufferText>().0,
            expected,
            "keyboard events {events:?}"
        );
        assert_eq!(
            app.world().resource::<ActiveImeSession>().active_text(),
            Some(expected),
            "active buffer after keyboard events {events:?}"
        );
        Ok(())
    }

    fn keyboard_app(initial_text: &str) -> (App, Entity) {
        let mut app = bevy::prelude::App::new();
        app.add_plugins(ImePlugin)
            .init_resource::<ObservedBufferText>();
        app.add_observer(
            |changed: On<crate::ime::ImeTextChanged>, mut observed: ResMut<ObservedBufferText>| {
                observed.0.clone_from(&changed.snapshot.committed_text);
            },
        );
        let window = app.world_mut().spawn(bevy::prelude::Window::default()).id();
        let owner = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(ImeOpenSession {
            target: ImeTarget::AppOwned {
                owner,
                field_id: "query".into(),
            },
            window,
            initial_text: initial_text.to_owned(),
            field_spec: ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("query")),
            anchor: None,
        });
        app.world_mut().flush();
        assert!(
            app.world()
                .resource::<ActiveImeSession>()
                .active_session_id()
                .is_some()
        );
        (app, window)
    }
}
