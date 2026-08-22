//! Public IME lifecycle events.

use bevy::prelude::Event;

use super::ImeBufferSnapshot;
use super::ImeBuiltInApplied;
use super::ImeCommitAttemptId;
use super::ImeEditableFieldSpec;
use super::ImeSessionId;
use super::ImeTarget;
use super::ImeValueRevision;

/// Fired after an IME session starts.
#[derive(Event, Clone, Debug)]
pub struct ImeStarted {
    /// Id assigned to the new session.
    pub session_id: ImeSessionId,
    /// Semantic backing target for the session.
    pub target:     ImeTarget,
}

/// Fired when the active edit buffer, selection, or preedit state changes.
#[derive(Event, Clone, Debug)]
pub struct ImeTextChanged {
    /// Id of the active session.
    pub session_id: ImeSessionId,
    /// Semantic backing target for the session.
    pub target:     ImeTarget,
    /// Current single-line buffer snapshot.
    pub snapshot:   ImeBufferSnapshot,
}

/// Why a field-level commit was requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImeCommitCause {
    /// User pressed Enter while not composing.
    Enter,
    /// Focus policy requested commit on blur.
    Blur,
    /// App explicitly requested commit.
    Request,
}

/// Fired when text is ready for commit validation.
#[derive(Event, Clone, Debug)]
pub struct ImeCommitRequested {
    /// Id of the active session.
    pub session_id: ImeSessionId,
    /// Id of this commit attempt.
    pub attempt_id: ImeCommitAttemptId,
    /// Semantic backing target for the session.
    pub target:     ImeTarget,
    /// Cause that requested this commit.
    pub cause:      ImeCommitCause,
    /// Editable field contract for the session.
    pub field_spec: ImeEditableFieldSpec,
    /// Committed buffer text to validate and apply.
    pub text:       String,
}

/// Why an IME session was canceled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImeCancelCause {
    /// User pressed Escape while not composing.
    Escape,
    /// Focus policy canceled on blur.
    Blur,
    /// A newer session replaced this one.
    Replaced,
    /// App explicitly requested cancellation.
    Request,
    /// The requested session id did not match the active session.
    SessionMismatch,
    /// The focused window was closed.
    WindowClosed,
    /// The focused window lost OS focus.
    FocusLost,
    /// The target entity no longer exists.
    TargetStale,
    /// The active window lease no longer matches the session.
    LeaseLost,
}

/// Fired after an IME session is canceled.
#[derive(Event, Clone, Debug)]
pub struct ImeCanceled {
    /// Id of the canceled session.
    pub session_id: ImeSessionId,
    /// Semantic backing target for the session.
    pub target:     ImeTarget,
    /// Cause that canceled the session.
    pub cause:      ImeCancelCause,
}

/// Commit rejection reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImeRejection {
    /// The commit attempt no longer matches the active pending attempt.
    StaleAttempt,
    /// The text did not parse for the field.
    InvalidText(String),
    /// The parsed value was outside the field's accepted range.
    OutOfRange(String),
    /// The app rejected the commit for an app-specific reason.
    AppOwned(String),
}

/// App or built-in response accepting a pending commit.
#[derive(Event, Clone, Debug)]
pub struct ImeAcceptCommit {
    /// Id of the active session.
    pub session_id: ImeSessionId,
    /// Id of the commit attempt being accepted.
    pub attempt_id: ImeCommitAttemptId,
    /// Result metadata for the accepted commit.
    pub result:     ImeAppliedResult,
}

/// App or built-in response rejecting a pending commit.
#[derive(Event, Clone, Debug)]
pub struct ImeRejectCommit {
    /// Id of the active session.
    pub session_id: ImeSessionId,
    /// Id of the commit attempt being rejected.
    pub attempt_id: ImeCommitAttemptId,
    /// Reason the commit was rejected.
    pub reason:     ImeRejection,
}

/// Result metadata for an accepted commit.
#[derive(Clone, Debug, PartialEq)]
pub enum ImeAppliedResult {
    /// Built-in field value applied by `hana_diegetic`.
    BuiltIn(ImeBuiltInApplied),
    /// App-owned field was already applied by caller code.
    AppOwned {
        /// Optional display text after apply.
        display_text:   Option<String>,
        /// Optional app/model revision after apply.
        value_revision: Option<ImeValueRevision>,
    },
}

/// Fired after a commit response is accepted and applied.
#[derive(Event, Clone, Debug)]
pub struct ImeApplied {
    /// Id of the completed session.
    pub session_id: ImeSessionId,
    /// Id of the accepted commit attempt.
    pub attempt_id: ImeCommitAttemptId,
    /// Semantic backing target for the session.
    pub target:     ImeTarget,
    /// Result metadata for the accepted commit.
    pub result:     ImeAppliedResult,
}

/// Fired when a commit attempt is rejected while the editor remains active.
#[derive(Event, Clone, Debug)]
pub struct ImeValidationRejected {
    /// Id of the active session.
    pub session_id: ImeSessionId,
    /// Id of the rejected commit attempt.
    pub attempt_id: ImeCommitAttemptId,
    /// Semantic backing target for the session.
    pub target:     ImeTarget,
    /// Reason the commit was rejected.
    pub reason:     ImeRejection,
}
