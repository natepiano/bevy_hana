//! Editable field specifications and applied value summaries.

use bevy::prelude::Component;

use super::ImeValueRevision;
use super::PanelElementId;
use crate::TextStyle;

/// Full authored text for a panel field's next editor opening.
///
/// This metadata is independent of the displayed layout. Updating it does not
/// replace the panel tree or change an editor that is already open. Attach it
/// to the panel that owns the named field.
#[derive(Component, Clone, Debug, Eq, PartialEq)]
pub struct ImeAuthoredFieldText {
    field_id: PanelElementId,
    text:     String,
}

impl ImeAuthoredFieldText {
    /// Supplies authored editor text for one field on this panel.
    #[must_use]
    pub fn new(field_id: impl Into<PanelElementId>, text: impl Into<String>) -> Self {
        Self {
            field_id: field_id.into(),
            text:     text.into(),
        }
    }

    /// The panel-local field whose editor uses this authored text.
    #[must_use]
    pub const fn field_id(&self) -> &PanelElementId { &self.field_id }

    /// The unformatted authored text used when the field is opened.
    #[must_use]
    pub const fn text(&self) -> &str { self.text.as_str() }
}

/// Editable field contract stored on a layout field or app-owned session.
#[derive(Clone, Debug, PartialEq)]
pub enum ImeEditableFieldSpec {
    /// Field parsed and applied by `hana_diegetic`.
    BuiltIn(ImeBuiltInFieldSpec),
    /// Field parsed and applied by the caller.
    AppOwned(ImeAppOwnedFieldSpec),
}

/// Built-in editable field behavior.
#[derive(Clone, Debug, PartialEq)]
pub struct ImeBuiltInFieldSpec {
    /// Value kind and optional range constraints.
    pub kind: ImeBuiltInFieldKind,
}

impl ImeBuiltInFieldSpec {
    /// Creates a built-in field spec for the given value kind.
    #[must_use]
    pub const fn new(kind: ImeBuiltInFieldKind) -> Self { Self { kind } }
}

/// Caller-owned editable field behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImeAppOwnedFieldSpec {
    /// Stable app-defined parser or apply key.
    pub key:           String,
    blur:              ImeBlurBehavior,
    initial_edit_text: AppOwnedInitialEditText,
    first_activation:  FirstActivation,
    commit_on_tab:     bool,
    validation:        ImeValidationPlacement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FirstActivation {
    DefaultSelection,
    SelectUntouched(Box<str>),
}

/// Where a caller-owned field reserves rejection feedback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ImeValidationPlacement {
    /// Use the editor's ordinary growing-field or adjacent-panel presentation.
    Field,
    /// Reserve a line after this complete element, moving the following layout down.
    AfterElement(PanelElementId),
}

impl ImeAppOwnedFieldSpec {
    /// Creates an app-owned field spec with a stable app key.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key:               key.into(),
            blur:              ImeBlurBehavior::Commit,
            initial_edit_text: AppOwnedInitialEditText::UseDisplayText,
            first_activation:  FirstActivation::DefaultSelection,
            commit_on_tab:     false,
            validation:        ImeValidationPlacement::Field,
        }
    }

    /// Reserves validation below the named element in this field's panel layout.
    ///
    /// Name the complete field frame or its surrounding group. The editor keeps
    /// that element's geometry and adds a feedback line with one spacing step
    /// above and below it. Clearing validation restores the authored layout.
    #[must_use]
    pub fn validation_after(mut self, element_id: impl Into<PanelElementId>) -> Self {
        self.validation = ImeValidationPlacement::AfterElement(element_id.into());
        self
    }

    /// Opens the editor with `text` instead of the field's displayed text.
    #[must_use]
    pub fn with_initial_text(mut self, text: impl Into<String>) -> Self {
        self.initial_edit_text = AppOwnedInitialEditText::Specified(text.into().into_boxed_str());
        self
    }

    /// Selects all on this field's first activation if its text still equals `initial_text`.
    ///
    /// Later activations use ordinary caret placement, even if an edit restores
    /// the initial text. Redrawing the owning panel does not reset this option.
    #[must_use]
    pub fn select_all_on_first_activation(mut self, initial_text: impl Into<String>) -> Self {
        self.first_activation =
            FirstActivation::SelectUntouched(initial_text.into().into_boxed_str());
        self
    }

    /// Commits before Tab traverses, even when clicking away cancels the edit.
    /// A keep-editing field still ignores Tab regardless of builder order.
    #[must_use]
    pub const fn commit_on_tab(mut self) -> Self {
        self.commit_on_tab = true;
        self
    }

    /// Cancels this field's edit when focus moves away without pressing Enter.
    #[must_use]
    pub const fn cancel_on_blur(mut self) -> Self {
        self.blur = ImeBlurBehavior::Cancel;
        self
    }

    /// Keeps this field's session open when clicked outside or when Tab is pressed.
    #[must_use]
    pub const fn keep_editing_on_blur(mut self) -> Self {
        self.blur = ImeBlurBehavior::KeepEditing;
        self
    }

    fn initial_text(&self, display_text: &str) -> String {
        match &self.initial_edit_text {
            AppOwnedInitialEditText::UseDisplayText => display_text.to_owned(),
            AppOwnedInitialEditText::Specified(text) => text.to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AppOwnedInitialEditText {
    UseDisplayText,
    Specified(Box<str>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImeBlurBehavior {
    Commit,
    Cancel,
    KeepEditing,
}

impl ImeEditableFieldSpec {
    pub(super) const fn first_activation(&self) -> &FirstActivation {
        match self {
            Self::BuiltIn(_) => &FirstActivation::DefaultSelection,
            Self::AppOwned(spec) => &spec.first_activation,
        }
    }

    pub(super) const fn validation_placement(&self) -> &ImeValidationPlacement {
        match self {
            Self::BuiltIn(_) => &ImeValidationPlacement::Field,
            Self::AppOwned(spec) => &spec.validation,
        }
    }

    pub(super) const fn commits_on_tab(&self) -> bool {
        match self {
            Self::BuiltIn(_) => true,
            Self::AppOwned(spec) => match spec.blur {
                ImeBlurBehavior::Commit => true,
                ImeBlurBehavior::Cancel => spec.commit_on_tab,
                ImeBlurBehavior::KeepEditing => false,
            },
        }
    }

    pub(super) fn initial_text(&self, display_text: &str) -> String {
        match self {
            Self::BuiltIn(_) => display_text.to_owned(),
            Self::AppOwned(spec) => spec.initial_text(display_text),
        }
    }

    pub(super) const fn blur_behavior(&self) -> ImeBlurBehavior {
        match self {
            Self::BuiltIn(_) => ImeBlurBehavior::Commit,
            Self::AppOwned(spec) => spec.blur,
        }
    }
}

/// Built-in value kind and commit-time constraints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImeBuiltInFieldKind {
    /// Free-form single-line text.
    Text,
    /// Floating-point value with optional inclusive bounds.
    Float {
        /// Minimum accepted value.
        min: Option<f32>,
        /// Maximum accepted value.
        max: Option<f32>,
    },
    /// Integer value with optional inclusive bounds.
    Integer {
        /// Minimum accepted value.
        min: Option<i64>,
        /// Maximum accepted value.
        max: Option<i64>,
    },
}

/// Built-in value written by `hana_diegetic`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImeBuiltInApplied {
    /// Parsed value written to the panel tree.
    pub value:          ImeBuiltInValue,
    /// Display text written back to the panel.
    pub display_text:   String,
    /// Panel value revision after the write.
    pub value_revision: ImeValueRevision,
}

/// Parsed built-in value written by `hana_diegetic`.
#[derive(Clone, Debug, PartialEq)]
pub enum ImeBuiltInValue {
    /// Applied text value.
    Text(String),
    /// Applied floating-point value.
    Float(f32),
    /// Applied integer value.
    Integer(i64),
}

/// Authored editable panel field metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ImePanelField {
    /// Panel-local semantic identity.
    pub field_id:   PanelElementId,
    /// Editable behavior for this field.
    pub field_spec: ImeEditableFieldSpec,
    /// Text styling retained for an inline editor even without a text child.
    pub text_style: TextStyle,
}

impl ImePanelField {
    /// Creates editable metadata for an authored panel field.
    #[must_use]
    pub fn new(
        field_id: impl Into<PanelElementId>,
        field_spec: ImeEditableFieldSpec,
        text_style: TextStyle,
    ) -> Self {
        Self {
            field_id: field_id.into(),
            field_spec,
            text_style,
        }
    }
}
