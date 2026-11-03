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
mod pointer;
mod session;
mod target;
mod value_drag;

pub use activation::ImeFieldAuthoringLock;
use bevy::input::keyboard::KeyboardInput;
use bevy::picking::PickingSystems;
use bevy::prelude::App;
use bevy::prelude::ButtonInput;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy::prelude::SystemSet;
use bevy::prelude::Update;
use bevy::window::Ime;
use bevy::window::WindowClosed;
use bevy::window::WindowFocused;
use bevy_enhanced_input::EnhancedInputSystems;
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
pub use events::ImeVerticalArrow;
pub use events::ImeVerticalArrowPressed;
pub use field::ImeAppOwnedFieldSpec;
pub use field::ImeAuthoredFieldText;
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
use pointer::FieldSelectionDrag;
use session::ActiveImeSession;
pub use session::ImeCommitAuthority;
pub use session::ImeCommitAuthorityToken;
pub use session::ImeInsertText;
pub use session::ImeOpenSession;
pub use session::ImeRequestCancel;
pub use session::ImeRequestCommit;
pub use target::ImeSessionAnchor;
pub use target::ImeTarget;
pub use value_drag::ImeNumberFieldDrag;
pub(crate) use value_drag::ImeValueDragging;

use crate::PanelSystems;

/// Installs the single-line IME session lifecycle.
pub(crate) struct ImePlugin;

/// Named scheduling points for IME focus, window state, input, and cleanup.
#[derive(SystemSet, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ImeSystemSet {
    /// Point where input blockers are ready for same-frame input consumers.
    /// In `PreUpdate`, pointer blur completes after picking and before keyboard routing.
    /// In `Update`, semantic input starts from the published session state.
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
        value_drag::install(app);
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
            .init_resource::<FieldSelectionDrag>()
            .add_observer(pointer::click)
            .add_observer(pointer::press)
            .add_observer(pointer::start_drag)
            .add_observer(pointer::drag)
            .add_observer(pointer::end_drag)
            .add_observer(pointer::cancel_drag)
            .add_observer(activation::open_from_semantic_activation)
            .add_observer(editor::classify_non_panel_click)
            .add_observer(session::open_session)
            .add_observer(activation::cancel_locked_session)
            .add_observer(session::insert_text)
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
                PreUpdate,
                editor::handle_blur_intent
                    .in_set(ImeSystemSet::PublishInputBlockers)
                    .after(PickingSystems::Last)
                    .before(EnhancedInputSystems::Prepare),
            )
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
