//! Active session resource and lifecycle observers.

use bevy::diagnostic::FrameCount;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::MessageReader;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::WindowClosed;
use bevy::window::WindowFocused;

use super::ImeAcceptCommit;
use super::ImeBufferSnapshot;
use super::ImeCancelCause;
use super::ImeCanceled;
use super::ImeCommitAttemptId;
use super::ImeCommitCause;
use super::ImeCommitRequested;
use super::ImeEditableFieldSpec;
use super::ImeInputBlocker;
use super::ImeRejectCommit;
use super::ImeSessionAnchor;
use super::ImeSessionId;
use super::ImeStarted;
use super::ImeTarget;
use super::ImeTextChanged;
use super::ImeValidationRejected;
use super::buffer;
use super::buffer::ImeBufferEdit;
use super::buffer::ImeEditBuffer;
use super::buffer::ImeEditCommand;
use super::buffer::ImePreedit;

/// Request event that opens an IME session for an already-resolved target.
#[derive(Event, Clone, Debug)]
pub struct ImeOpenSession {
    /// Semantic backing target for the session.
    pub target:       ImeTarget,
    /// Window that owns OS IME state for the session.
    pub window:       Entity,
    /// Initial committed buffer text.
    pub initial_text: String,
    /// Editable field contract for the session.
    pub field_spec:   ImeEditableFieldSpec,
    /// Optional screen-space anchor for app-owned sessions.
    pub anchor:       Option<ImeSessionAnchor>,
}

/// Request event that asks the active session to commit.
#[derive(Event, Clone, Debug)]
pub struct ImeRequestCommit {
    /// Id of the session being committed.
    pub session_id: ImeSessionId,
    /// Cause assigned to the commit attempt.
    pub cause:      ImeCommitCause,
}

/// Request event that asks the active session to cancel.
#[derive(Event, Clone, Debug)]
pub struct ImeRequestCancel {
    /// Id of the session being canceled.
    pub session_id: ImeSessionId,
    /// Cause assigned to the cancellation.
    pub cause:      ImeCancelCause,
}

#[derive(Clone, Debug)]
struct ImeSession {
    session_id: ImeSessionId,
    target:     ImeTarget,
    window:     Entity,
    field_spec: ImeEditableFieldSpec,
    anchor:     Option<ImeSessionAnchor>,
    buffer:     ImeEditBuffer,
    state:      ImeSessionState,
}

#[derive(Clone, Debug)]
enum ImeSessionState {
    Editing,
    Composing(ImePreedit),
    PendingCommit(ImeCommitAttemptId),
}

impl ImeSessionState {
    const fn preedit(&self) -> Option<&ImePreedit> {
        match self {
            Self::Composing(preedit) => Some(preedit),
            Self::Editing | Self::PendingCommit(_) => None,
        }
    }

    const fn pending_attempt(&self) -> Option<ImeCommitAttemptId> {
        match *self {
            Self::PendingCommit(attempt_id) => Some(attempt_id),
            Self::Editing | Self::Composing(_) => None,
        }
    }

    const fn is_composing(&self) -> bool { matches!(self, Self::Composing(_)) }

    const fn is_pending_commit(&self) -> bool { matches!(self, Self::PendingCommit(_)) }
}

/// Resource holding the single active IME session.
#[derive(Resource, Debug)]
pub(crate) struct ActiveImeSession {
    active:       Option<ImeSession>,
    next_session: u64,
    next_attempt: u64,
}

/// Current commit attempt guard for app-owned apply responses.
#[derive(Resource, Clone, Debug, Default)]
pub struct ImeCommitAuthority {
    current: Option<ImeCommitAuthorityToken>,
}

impl ImeCommitAuthority {
    /// Returns `true` when `session_id` and `attempt_id` still name the active
    /// pending commit.
    #[must_use]
    pub fn is_current(&self, session_id: ImeSessionId, attempt_id: ImeCommitAttemptId) -> bool {
        self.current
            .as_ref()
            .is_some_and(|token| token.session_id == session_id && token.attempt_id == attempt_id)
    }

    /// Returns the current pending commit token.
    #[must_use]
    pub const fn current(&self) -> Option<&ImeCommitAuthorityToken> { self.current.as_ref() }

    fn set(&mut self, session: &ImeSession, attempt_id: ImeCommitAttemptId) {
        self.current = Some(ImeCommitAuthorityToken {
            session_id: session.session_id,
            attempt_id,
            target: session.target.clone(),
        });
    }

    fn clear_session(&mut self, session_id: ImeSessionId) {
        if self
            .current
            .as_ref()
            .is_some_and(|token| token.session_id == session_id)
        {
            self.current = None;
        }
    }
}

/// Public token proving which commit attempt is still current.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImeCommitAuthorityToken {
    /// Id of the active session.
    pub session_id: ImeSessionId,
    /// Id of the active commit attempt.
    pub attempt_id: ImeCommitAttemptId,
    /// Semantic target being committed.
    pub target:     ImeTarget,
}

impl Default for ActiveImeSession {
    fn default() -> Self {
        Self {
            active:       None,
            next_session: 1,
            next_attempt: 1,
        }
    }
}

impl ActiveImeSession {
    fn next_session_id(&mut self) -> ImeSessionId {
        let session_id = ImeSessionId::new(self.next_session);
        self.next_session = self.next_session.wrapping_add(1).max(1);
        session_id
    }

    fn next_attempt_id(&mut self) -> ImeCommitAttemptId {
        let attempt_id = ImeCommitAttemptId::new(self.next_attempt);
        self.next_attempt = self.next_attempt.wrapping_add(1).max(1);
        attempt_id
    }

    pub(super) fn active_session_id(&self) -> Option<ImeSessionId> {
        self.active.as_ref().map(|session| session.session_id)
    }

    pub(super) fn active_window(&self) -> Option<Entity> {
        self.active.as_ref().map(|session| session.window)
    }

    pub(super) fn active_anchor(&self) -> Option<ImeSessionAnchor> {
        self.active.as_ref().and_then(|session| session.anchor)
    }

    pub(super) fn active_target(&self) -> Option<&ImeTarget> {
        self.active.as_ref().map(|session| &session.target)
    }

    pub(super) fn is_composing(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|session| session.state.is_composing())
    }

    pub(super) fn is_pending_commit(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|session| session.state.is_pending_commit())
    }

    pub(super) fn is_leased(&self, input_blocker: &ImeInputBlocker) -> bool {
        self.active.as_ref().is_some_and(|session| {
            input_blocker.matches_session_window(session.session_id, session.window)
        })
    }

    pub(super) fn apply_keyboard_text(
        &mut self,
        window: Entity,
        text: &str,
        input_blocker: &ImeInputBlocker,
    ) -> Option<ImeTextChanged> {
        self.apply_edit(
            window,
            ImeEditCommand::InsertText(text.to_owned()),
            input_blocker,
        )
    }

    pub(super) fn apply_edit_command(
        &mut self,
        command: ImeEditCommand,
        input_blocker: &ImeInputBlocker,
    ) -> Option<ImeTextChanged> {
        let window = self.active_window()?;
        self.apply_edit(window, command, input_blocker)
    }

    pub(super) fn apply_preedit(
        &mut self,
        window: Entity,
        text: &str,
        cursor: Option<(usize, usize)>,
        input_blocker: &ImeInputBlocker,
    ) -> Option<ImeTextChanged> {
        let session = self.editable_session(window, input_blocker)?;
        if text.is_empty() {
            return session.clear_preedit();
        }

        let preedit = ImePreedit {
            text:        text.to_owned(),
            replacement: session.buffer.replacement_range(),
            cursor:      buffer::preedit_cursor_boundary(text, cursor),
        };
        session.state = ImeSessionState::Composing(preedit);
        Some(session.text_changed_event())
    }

    pub(super) fn apply_ime_commit(
        &mut self,
        window: Entity,
        text: &str,
        input_blocker: &ImeInputBlocker,
    ) -> Option<ImeTextChanged> {
        let session = self.editable_session(window, input_blocker)?;
        let was_composing = session.state.is_composing();
        session.state = ImeSessionState::Editing;
        match session
            .buffer
            .apply(ImeEditCommand::InsertText(text.to_owned()))
        {
            ImeBufferEdit::Changed => Some(session.text_changed_event()),
            ImeBufferEdit::Unchanged if was_composing => Some(session.text_changed_event()),
            ImeBufferEdit::Unchanged => None,
        }
    }

    pub(super) fn clear_preedit(
        &mut self,
        window: Entity,
        input_blocker: &ImeInputBlocker,
    ) -> Option<ImeTextChanged> {
        let session = self.editable_session(window, input_blocker)?;
        session.clear_preedit()
    }

    pub(super) fn clear_active_preedit(
        &mut self,
        input_blocker: &ImeInputBlocker,
    ) -> Option<ImeTextChanged> {
        let window = self.active_window()?;
        self.clear_preedit(window, input_blocker)
    }

    fn apply_edit(
        &mut self,
        window: Entity,
        command: ImeEditCommand,
        input_blocker: &ImeInputBlocker,
    ) -> Option<ImeTextChanged> {
        let session = self.editable_session(window, input_blocker)?;
        match session.buffer.apply(command) {
            ImeBufferEdit::Changed => Some(session.text_changed_event()),
            ImeBufferEdit::Unchanged => None,
        }
    }

    fn editable_session(
        &mut self,
        window: Entity,
        input_blocker: &ImeInputBlocker,
    ) -> Option<&mut ImeSession> {
        let session = self.active.as_mut()?;
        if session.window != window
            || session.state.is_pending_commit()
            || !input_blocker.matches_session_window(session.session_id, session.window)
        {
            return None;
        }
        Some(session)
    }
}

impl ImeSession {
    fn snapshot(&self) -> ImeBufferSnapshot { self.buffer.snapshot(self.state.preedit().cloned()) }

    fn text_changed_event(&self) -> ImeTextChanged {
        ImeTextChanged {
            session_id: self.session_id,
            target:     self.target.clone(),
            snapshot:   self.snapshot(),
        }
    }

    fn clear_preedit(&mut self) -> Option<ImeTextChanged> {
        if !self.state.is_composing() {
            return None;
        }
        self.state = ImeSessionState::Editing;
        Some(self.text_changed_event())
    }
}

pub(super) fn open_session(
    request: On<ImeOpenSession>,
    mut active_session: ResMut<ActiveImeSession>,
    mut input_blocker: ResMut<ImeInputBlocker>,
    mut authority: ResMut<ImeCommitAuthority>,
    frame_count: Option<Res<FrameCount>>,
    mut commands: Commands,
) {
    let request = request.event();
    if let Some(previous) = active_session.active.take() {
        input_blocker.clear_session(previous.session_id);
        authority.clear_session(previous.session_id);
        commands.trigger(ImeCanceled {
            session_id: previous.session_id,
            target:     previous.target,
            cause:      ImeCancelCause::Replaced,
        });
    }

    let session_id = active_session.next_session_id();
    let buffer = match &request.target {
        ImeTarget::WorldPanelField { .. } | ImeTarget::ScreenPanelField { .. } => {
            ImeEditBuffer::new_selected(request.initial_text.clone())
        },
        ImeTarget::AppOwned { .. } => ImeEditBuffer::new(request.initial_text.clone()),
    };
    let session = ImeSession {
        session_id,
        target: request.target.clone(),
        window: request.window,
        field_spec: request.field_spec.clone(),
        anchor: request.anchor,
        buffer,
        state: ImeSessionState::Editing,
    };
    let changed = session.text_changed_event();
    active_session.active = Some(session);
    input_blocker.begin_session(session_id, request.window, frame_count.map(|count| count.0));

    commands.trigger(ImeStarted {
        session_id,
        target: request.target.clone(),
    });
    commands.trigger(changed);
}

pub(super) fn request_commit(
    request: On<ImeRequestCommit>,
    mut active_session: ResMut<ActiveImeSession>,
    mut authority: ResMut<ImeCommitAuthority>,
    mut commands: Commands,
) {
    let request = request.event();
    commit_matching_session(
        &mut active_session,
        &mut authority,
        request.session_id,
        request.cause,
        &mut commands,
    );
}

pub(super) fn request_cancel(
    request: On<ImeRequestCancel>,
    mut active_session: ResMut<ActiveImeSession>,
    mut input_blocker: ResMut<ImeInputBlocker>,
    mut authority: ResMut<ImeCommitAuthority>,
    mut commands: Commands,
) {
    let request = request.event();
    if cancel_matching_session(
        &mut active_session,
        request.session_id,
        request.cause,
        &mut commands,
    ) {
        input_blocker.clear_session(request.session_id);
        authority.clear_session(request.session_id);
    }
}

pub(super) fn accept_commit(
    response: On<ImeAcceptCommit>,
    mut active_session: ResMut<ActiveImeSession>,
    mut input_blocker: ResMut<ImeInputBlocker>,
    mut authority: ResMut<ImeCommitAuthority>,
    mut commands: Commands,
) {
    let response = response.event();
    let Some(session) = active_session.active.take() else {
        return;
    };
    if session.session_id != response.session_id
        || session.state.pending_attempt() != Some(response.attempt_id)
    {
        active_session.active = Some(session);
        return;
    }

    input_blocker.clear_session(response.session_id);
    authority.clear_session(response.session_id);
    commands.trigger(super::ImeApplied {
        session_id: response.session_id,
        attempt_id: response.attempt_id,
        target:     session.target,
        result:     response.result.clone(),
    });
}

pub(super) fn reject_commit(
    response: On<ImeRejectCommit>,
    mut active_session: ResMut<ActiveImeSession>,
    mut authority: ResMut<ImeCommitAuthority>,
    mut commands: Commands,
) {
    let response = response.event();
    let Some(session) = active_session.active.as_mut() else {
        return;
    };
    if session.session_id != response.session_id
        || session.state.pending_attempt() != Some(response.attempt_id)
    {
        return;
    }

    session.state = ImeSessionState::Editing;
    authority.clear_session(response.session_id);
    commands.trigger(ImeValidationRejected {
        session_id: response.session_id,
        attempt_id: response.attempt_id,
        target:     session.target.clone(),
        reason:     response.reason.clone(),
    });
}

pub(super) fn cleanup_stale_sessions(
    windows: Query<(), With<Window>>,
    entities: Query<(), ()>,
    mut focus_events: MessageReader<WindowFocused>,
    mut closed_events: MessageReader<WindowClosed>,
    mut active_session: ResMut<ActiveImeSession>,
    mut input_blocker: ResMut<ImeInputBlocker>,
    mut authority: ResMut<ImeCommitAuthority>,
    mut commands: Commands,
) {
    let Some(session) = active_session.active.as_ref() else {
        input_blocker.clear();
        focus_events.clear();
        closed_events.clear();
        return;
    };

    let window = session.window;
    let window_closed = closed_events.read().any(|event| event.window == window);
    let focus_lost = focus_events
        .read()
        .any(|event| event.window == window && !event.focused);

    let cause = if !input_blocker.matches_session_window(session.session_id, window) {
        Some(ImeCancelCause::LeaseLost)
    } else if window_closed || !windows.contains(window) {
        Some(ImeCancelCause::WindowClosed)
    } else if focus_lost {
        Some(ImeCancelCause::FocusLost)
    } else if !target_exists(&session.target, &entities) {
        Some(ImeCancelCause::TargetStale)
    } else {
        None
    };

    if let Some(cause) = cause {
        let session_id = session.session_id;
        if cancel_matching_session(&mut active_session, session_id, cause, &mut commands) {
            input_blocker.clear_session(session_id);
            authority.clear_session(session_id);
        }
    }
}

fn target_exists(target: &ImeTarget, entities: &Query<(), ()>) -> bool {
    let entity = match *target {
        ImeTarget::WorldPanelField { panel, .. } | ImeTarget::ScreenPanelField { panel, .. } => {
            panel
        },
        ImeTarget::AppOwned { owner, .. } => owner,
    };
    entities.contains(entity)
}

fn commit_matching_session(
    active_session: &mut ActiveImeSession,
    authority: &mut ImeCommitAuthority,
    session_id: ImeSessionId,
    cause: ImeCommitCause,
    commands: &mut Commands,
) {
    let Some(active) = active_session.active.as_ref() else {
        return;
    };
    if active.session_id != session_id || !matches!(active.state, ImeSessionState::Editing) {
        return;
    }

    let attempt_id = active_session.next_attempt_id();
    let Some(active) = active_session.active.as_mut() else {
        return;
    };
    active.state = ImeSessionState::PendingCommit(attempt_id);
    authority.set(active, attempt_id);

    commands.trigger(ImeCommitRequested {
        session_id,
        attempt_id,
        target: active.target.clone(),
        cause,
        field_spec: active.field_spec.clone(),
        text: active.buffer.committed_text().to_owned(),
    });
}

fn cancel_matching_session(
    active_session: &mut ActiveImeSession,
    session_id: ImeSessionId,
    cause: ImeCancelCause,
    commands: &mut Commands,
) -> bool {
    let Some(session) = active_session.active.take() else {
        return false;
    };
    if session.session_id != session_id {
        active_session.active = Some(session);
        return false;
    }

    commands.trigger(ImeCanceled {
        session_id,
        target: session.target,
        cause,
    });
    true
}
