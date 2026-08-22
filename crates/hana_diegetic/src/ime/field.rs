//! Editable field specifications and applied value summaries.

use super::ImeValueRevision;
use super::PanelElementId;

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
    pub key: String,
}

impl ImeAppOwnedFieldSpec {
    /// Creates an app-owned field spec with a stable app key.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self { Self { key: key.into() } }
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
}

impl ImePanelField {
    /// Creates editable metadata for an authored panel field.
    #[must_use]
    pub fn new(field_id: impl Into<PanelElementId>, field_spec: ImeEditableFieldSpec) -> Self {
        Self {
            field_id: field_id.into(),
            field_spec,
        }
    }
}
