//! Single-line IME session lifecycle.
//!
//! The module stays private; approved public API is re-exported from the crate
//! root with `Ime...` names so callers do not need a public module namespace.

mod activation;
mod apply;
mod buffer;
mod editor;
mod events;
mod field;
mod ids;
mod input;
mod lease;
mod session;
mod target;

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::App;
use bevy::prelude::ButtonInput;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Plugin;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::window::Ime;
use bevy::window::WindowClosed;
use bevy::window::WindowFocused;
pub use buffer::ImeBufferBoundary;
pub use buffer::ImeBufferRange;
pub use buffer::ImeBufferSnapshot;
pub use buffer::ImeCursorState;
pub use buffer::ImePreedit;
pub use buffer::ImePreeditBoundary;
pub use buffer::ImeSelectionSnapshot;
pub(crate) use editor::ImeBlurIntent;
pub(crate) use editor::ImeEditorState;
pub use editor::ImeReplacePanelTree;
use editor::PendingImePanelAnchor;
pub(crate) use editor::classify_widget_click;
#[cfg(test)]
pub(crate) use editor::handle_blur_intent;
pub use events::ImeAcceptCommit;
pub use events::ImeApplied;
pub use events::ImeAppliedResult;
pub use events::ImeCancelCause;
pub use events::ImeCanceled;
pub use events::ImeCommitCause;
pub use events::ImeCommitRequested;
pub use events::ImeRejectCommit;
pub use events::ImeRejection;
pub use events::ImeStarted;
pub use events::ImeTextChanged;
pub use events::ImeValidationRejected;
pub use field::ImeAppOwnedFieldSpec;
pub use field::ImeBuiltInApplied;
pub use field::ImeBuiltInFieldKind;
pub use field::ImeBuiltInFieldSpec;
pub use field::ImeBuiltInValue;
pub use field::ImeEditableFieldSpec;
pub use field::ImePanelField;
pub use ids::ImeCommitAttemptId;
pub use ids::ImeSessionId;
pub use ids::ImeValueRevision;
pub use ids::PanelElementId;
pub use input::ImeAppInputContext;
pub use input::ImeAppInputDisposition;
pub use input::ImeAppInputDispositionHook;
use input::ImeInputFrame;
use input::ImeWindowState;
use input::PendingImeTraversal;
pub use lease::ImeInputBlocker;
use session::ActiveImeSession;
pub use session::ImeCommitAuthority;
pub use session::ImeCommitAuthorityToken;
pub use session::ImeOpenSession;
pub use session::ImeRequestCancel;
pub use session::ImeRequestCommit;
pub use target::ImeSessionAnchor;
pub use target::ImeTarget;

use crate::PanelSystems;

/// Installs the single-line IME session lifecycle.
pub(crate) struct ImePlugin;

/// Named scheduling points for IME focus, window state, input, and cleanup.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ImeSystemSet {
    /// Point where input blockers are ready for same-frame input consumers.
    PublishInputBlockers,
    /// Point where `Window::ime_enabled` is updated.
    UpdateWindowIme,
    /// Point where platform IME and keyboard edits are consumed.
    Input,
    /// Point where the transient editor anchor, panel position, and caret are updated.
    UpdateEditorGeometry,
    /// Point where final `Window::ime_position` is written from editor caret geometry.
    UpdateImePosition,
    /// Point where stale sessions are canceled.
    Cleanup,
}

impl Plugin for ImePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Ime>()
            .add_message::<KeyboardInput>()
            .add_message::<WindowClosed>()
            .add_message::<WindowFocused>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ActiveImeSession>()
            .init_resource::<ImeCommitAuthority>()
            .init_resource::<ImeInputBlocker>()
            .init_resource::<ImeInputFrame>()
            .init_resource::<ImeAppInputDispositionHook>()
            .init_resource::<ImeWindowState>()
            .init_resource::<PendingImePanelAnchor>()
            .init_resource::<ImeEditorState>()
            .init_resource::<ImeBlurIntent>()
            .init_resource::<PendingImeTraversal>()
            .configure_sets(
                Update,
                (
                    ImeSystemSet::PublishInputBlockers,
                    ImeSystemSet::UpdateWindowIme,
                    ImeSystemSet::Input,
                    ImeSystemSet::UpdateEditorGeometry,
                    ImeSystemSet::UpdateImePosition,
                    ImeSystemSet::Cleanup,
                )
                    .chain(),
            )
            .add_observer(activation::observe_panel_clicks)
            .add_observer(activation::open_from_semantic_activation)
            .add_observer(editor::observe_panel_clicks)
            .add_observer(editor::classify_non_panel_click)
            .add_observer(session::open_session)
            .add_observer(session::request_commit)
            .add_observer(session::request_cancel)
            .add_observer(session::accept_commit)
            .add_observer(session::reject_commit)
            .add_observer(apply::apply_builtin_commit)
            .add_observer(input::continue_widget_traversal_after_apply)
            .add_observer(input::clear_widget_traversal_after_rejection)
            .add_observer(input::clear_widget_traversal_after_cancel)
            .add_observer(editor::replace_panel_tree)
            .add_observer(editor::update_editor_from_text_changed)
            .add_observer(editor::update_editor_validation)
            .add_observer(editor::close_editor_on_cancel)
            .add_observer(editor::close_editor_on_apply)
            .add_systems(
                Update,
                input::clear_frame_input.in_set(ImeSystemSet::PublishInputBlockers),
            )
            .add_systems(
                Update,
                input::update_window_ime.in_set(ImeSystemSet::UpdateWindowIme),
            )
            .add_systems(
                Update,
                (
                    editor::handle_blur_intent,
                    input::handle_window_ime,
                    input::handle_keyboard,
                )
                    .chain()
                    .in_set(ImeSystemSet::Input),
            )
            .add_systems(
                Update,
                editor::update_editor_anchor
                    .in_set(ImeSystemSet::UpdateEditorGeometry)
                    .after(PanelSystems::ResolvePanelAttachments)
                    .before(PanelSystems::PositionScreenSpace),
            )
            .add_systems(
                Update,
                editor::update_window_ime_position.in_set(ImeSystemSet::UpdateImePosition),
            )
            .add_systems(
                Update,
                session::cleanup_stale_sessions.in_set(ImeSystemSet::Cleanup),
            );
    }
}
