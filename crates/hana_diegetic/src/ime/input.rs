//! Window IME and keyboard input routing for the active edit session.

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
use super::buffer::ImeEditCommand;
use super::buffer::ImeMovementDirection;
use super::buffer::ImeMovementUnit;
use super::buffer::ImeSelectionMode;
use super::events::ImeTextChanged;
use crate::widgets::WidgetInput;

/// Per-frame IME input routing state.
#[derive(Resource, Clone, Debug, Default)]
pub(super) struct ImeInputFrame {
    saw_platform_ime: bool,
}

/// Widget traversal deferred until a panel field accepts its Tab commit.
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

    // A key that activates an editable widget must not also edit, commit, or
    // cancel the session it just opened. The lease records that activation
    // frame so the physical input edge belongs to widget activation only.
    let captured_activation = frame_count
        .as_ref()
        .is_some_and(|count| input_blocker.captured_activation_frame(count.0));
    if captured_activation {
        key_events.clear();
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

    if let Some((session_id, input)) = panel_traversal_from_tab(&keys, &active_session) {
        pending_traversal.0 = Some((session_id, input));
        commands.trigger(super::ImeRequestCommit {
            session_id,
            cause: ImeCommitCause::Blur,
        });
        key_events.clear();
        return;
    }

    if let Some(request) = request_from_keys(&keys, &active_session) {
        trigger_session_request(request, &mut commands);
        return;
    }

    if let Some(command) = command_from_keyboard_events(&keys, events.as_slice()) {
        let changed = active_session.apply_edit_command(command, &input_blocker);
        trigger_text_changed(changed, &mut commands);
        return;
    }

    if frame.saw_platform_ime || active_session.is_pending_commit() {
        key_events.clear();
        return;
    }

    for event in &events {
        if event.state != ButtonState::Pressed || command_modifier_pressed(&keys) {
            continue;
        }
        let Some(text) = event.text.as_deref() else {
            continue;
        };
        let Some(text) = editable_keyboard_text(text) else {
            continue;
        };
        let changed = active_session.apply_keyboard_text(event.window, text, &input_blocker);
        trigger_text_changed(changed, &mut commands);
    }
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
) {
    if pending
        .0
        .is_some_and(|(session_id, _)| session_id == canceled.session_id)
    {
        pending.0 = None;
    }
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
    keys: &ButtonInput<KeyCode>,
    events: &[KeyboardInput],
) -> Option<ImeEditCommand> {
    events.iter().find_map(|event| {
        (event.state == ButtonState::Pressed)
            .then(|| command_from_key_code(keys, event.key_code))
            .flatten()
    })
}

fn command_from_key_code(keys: &ButtonInput<KeyCode>, key_code: KeyCode) -> Option<ImeEditCommand> {
    let selection = if shift_pressed(keys) {
        ImeSelectionMode::Extend
    } else {
        ImeSelectionMode::Move
    };

    if primary_modifier_pressed(keys) && key_code == KeyCode::KeyA {
        return Some(ImeEditCommand::SelectAll);
    }

    if key_code == KeyCode::ArrowLeft {
        return Some(ImeEditCommand::Move {
            direction: ImeMovementDirection::Backward,
            unit: movement_unit(keys),
            selection,
        });
    }

    if key_code == KeyCode::ArrowRight {
        return Some(ImeEditCommand::Move {
            direction: ImeMovementDirection::Forward,
            unit: movement_unit(keys),
            selection,
        });
    }

    if key_code == KeyCode::Home {
        return Some(ImeEditCommand::Move {
            direction: ImeMovementDirection::Backward,
            unit: ImeMovementUnit::Line,
            selection,
        });
    }

    if key_code == KeyCode::End {
        return Some(ImeEditCommand::Move {
            direction: ImeMovementDirection::Forward,
            unit: ImeMovementUnit::Line,
            selection,
        });
    }

    if key_code == KeyCode::Backspace {
        return Some(ImeEditCommand::DeleteBackward(delete_unit(keys)));
    }

    (key_code == KeyCode::Delete).then_some(ImeEditCommand::DeleteForward(delete_unit(keys)))
}

fn movement_unit(keys: &ButtonInput<KeyCode>) -> ImeMovementUnit {
    if word_modifier_pressed(keys) {
        ImeMovementUnit::Word
    } else if super_pressed(keys) {
        ImeMovementUnit::Line
    } else {
        ImeMovementUnit::Character
    }
}

fn delete_unit(keys: &ButtonInput<KeyCode>) -> ImeMovementUnit {
    if super_pressed(keys) {
        ImeMovementUnit::Line
    } else if word_modifier_pressed(keys) {
        ImeMovementUnit::Word
    } else {
        ImeMovementUnit::Character
    }
}

fn trigger_text_changed(changed: Option<ImeTextChanged>, commands: &mut Commands) {
    if let Some(changed) = changed {
        commands.trigger(changed);
    }
}

fn shift_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)
}

fn primary_modifier_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    control_pressed(keys) || super_pressed(keys)
}

fn command_modifier_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    control_pressed(keys) || super_pressed(keys)
}

fn word_modifier_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    control_pressed(keys) || alt_pressed(keys)
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
    use bevy::input::ButtonState;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Entity;
    use bevy::prelude::KeyCode;

    use super::command_from_key_code;
    use super::command_from_keyboard_events;
    use super::editable_keyboard_text;
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
    fn primary_a_maps_to_select_all() {
        let keys = keys(&[KeyCode::SuperLeft]);

        assert_eq!(
            command_from_key_code(&keys, KeyCode::KeyA),
            Some(ImeEditCommand::SelectAll)
        );
    }

    #[test]
    fn alt_arrow_left_maps_to_word_movement() {
        let keys = keys(&[KeyCode::AltLeft]);

        assert_eq!(
            command_from_key_code(&keys, KeyCode::ArrowLeft),
            Some(ImeEditCommand::Move {
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
            command_from_key_code(&keys, KeyCode::End),
            Some(ImeEditCommand::Move {
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
            command_from_key_code(&keys, KeyCode::Backspace),
            Some(ImeEditCommand::DeleteBackward(ImeMovementUnit::Character))
        );
    }

    #[test]
    fn command_backspace_maps_to_line_delete() {
        let keys = keys(&[KeyCode::SuperLeft]);

        assert_eq!(
            command_from_key_code(&keys, KeyCode::Backspace),
            Some(ImeEditCommand::DeleteBackward(ImeMovementUnit::Line))
        );
    }

    #[test]
    fn option_backspace_maps_to_word_delete() {
        let keys = keys(&[KeyCode::AltLeft]);

        assert_eq!(
            command_from_key_code(&keys, KeyCode::Backspace),
            Some(ImeEditCommand::DeleteBackward(ImeMovementUnit::Word))
        );
    }

    #[test]
    fn repeated_backspace_maps_to_delete_backward() {
        let keys = keys(&[]);
        let events = [key_event(KeyCode::Backspace, true)];

        assert_eq!(
            command_from_keyboard_events(&keys, &events),
            Some(ImeEditCommand::DeleteBackward(ImeMovementUnit::Character))
        );
    }

    #[test]
    fn control_only_keyboard_text_is_ignored() {
        assert_eq!(editable_keyboard_text("\u{8}"), None);
        assert_eq!(editable_keyboard_text("\u{7f}"), None);
        assert_eq!(editable_keyboard_text("a"), Some("a"));
    }
}
