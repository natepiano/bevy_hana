//! Built-in commit validation and panel writeback.

use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;

use super::ImeAcceptCommit;
use super::ImeAppliedResult;
use super::ImeBuiltInApplied;
use super::ImeBuiltInFieldKind;
use super::ImeBuiltInFieldSpec;
use super::ImeBuiltInValue;
use super::ImeCommitAuthority;
use super::ImeCommitRequested;
use super::ImeEditableFieldSpec;
use super::ImeEditorState;
use super::ImeRejectCommit;
use super::ImeRejection;
use super::ImeTarget;
use super::ImeValueRevision;
use crate::DiegeticPanel;
use crate::DiegeticPanelCommands;
use crate::layout::FieldDisplayTextUpdate;

const FLOAT_INVALID_MESSAGE: &str = "expected a finite floating-point value";
const INTEGER_INVALID_MESSAGE: &str = "expected an integer value";
const TARGET_UNAVAILABLE_MESSAGE: &str = "target panel is unavailable";
const DUPLICATE_FIELD_MESSAGE: &str = "field id is duplicated";
const MISSING_FIELD_MESSAGE: &str = "field id is missing";
const MISSING_TEXT_MESSAGE: &str = "field has no text display";

pub(super) fn apply_builtin_commit(
    event: On<ImeCommitRequested>,
    authority: Res<ImeCommitAuthority>,
    editor_state: Res<ImeEditorState>,
    panels: Query<&DiegeticPanel>,
    mut commands: Commands,
) {
    let event = event.event();
    if !authority.is_current(event.session_id, event.attempt_id) {
        return;
    }

    let ImeEditableFieldSpec::BuiltIn(spec) = &event.field_spec else {
        return;
    };
    let Some(panel_entity) = target_panel(&event.target) else {
        return;
    };
    let Ok(panel) = panels.get(panel_entity) else {
        reject(
            event,
            ImeRejection::InvalidText(TARGET_UNAVAILABLE_MESSAGE.to_owned()),
            &mut commands,
        );
        return;
    };

    let parsed = match parse_builtin_value(spec, &event.text) {
        Ok(parsed) => parsed,
        Err(rejection) => {
            reject(event, rejection, &mut commands);
            return;
        },
    };

    let mut tree = editor_state
        .authoritative_tree(event.session_id, panel_entity)
        .unwrap_or_else(|| panel.tree())
        .clone();
    let update = tree.set_field_display_text(target_field_id(&event.target), parsed.display_text());
    if update != FieldDisplayTextUpdate::Updated {
        reject(event, update_rejection(update), &mut commands);
        return;
    }

    let value_revision = ImeValueRevision::new(u64::from(panel.next_tree_revision()));
    if let Err(error) = commands.set_tree(panel_entity, tree) {
        reject(
            event,
            ImeRejection::InvalidText(error.to_string()),
            &mut commands,
        );
        return;
    }
    commands.trigger(ImeAcceptCommit {
        session_id: event.session_id,
        attempt_id: event.attempt_id,
        result:     ImeAppliedResult::BuiltIn(parsed.into_applied(value_revision)),
    });
}

const fn target_panel(target: &ImeTarget) -> Option<Entity> {
    match *target {
        ImeTarget::WorldPanelField { panel, .. } | ImeTarget::ScreenPanelField { panel, .. } => {
            Some(panel)
        },
        ImeTarget::AppOwned { .. } => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedBuiltInValue {
    value:        ImeBuiltInValue,
    display_text: String,
}

impl ParsedBuiltInValue {
    fn display_text(&self) -> &str { &self.display_text }

    fn into_applied(self, value_revision: ImeValueRevision) -> ImeBuiltInApplied {
        ImeBuiltInApplied {
            value: self.value,
            display_text: self.display_text,
            value_revision,
        }
    }
}

const fn target_field_id(target: &ImeTarget) -> &super::PanelElementId {
    match target {
        ImeTarget::WorldPanelField { field_id, .. }
        | ImeTarget::ScreenPanelField { field_id, .. }
        | ImeTarget::AppOwned { field_id, .. } => field_id,
    }
}

fn parse_builtin_value(
    spec: &ImeBuiltInFieldSpec,
    text: &str,
) -> Result<ParsedBuiltInValue, ImeRejection> {
    match spec.kind {
        ImeBuiltInFieldKind::Text => Ok(ParsedBuiltInValue {
            value:        ImeBuiltInValue::Text(text.to_owned()),
            display_text: text.to_owned(),
        }),
        ImeBuiltInFieldKind::Float { min, max } => parse_float(text, min, max),
        ImeBuiltInFieldKind::Integer { min, max } => parse_integer(text, min, max),
    }
}

fn parse_float(
    text: &str,
    min: Option<f32>,
    max: Option<f32>,
) -> Result<ParsedBuiltInValue, ImeRejection> {
    let value = text
        .trim()
        .parse::<f32>()
        .map_err(|_| ImeRejection::InvalidText(FLOAT_INVALID_MESSAGE.to_owned()))?;
    if !value.is_finite() {
        return Err(ImeRejection::InvalidText(FLOAT_INVALID_MESSAGE.to_owned()));
    }
    if let Some(min) = min
        && value < min
    {
        return Err(ImeRejection::OutOfRange(format!("minimum is {min}")));
    }
    if let Some(max) = max
        && value > max
    {
        return Err(ImeRejection::OutOfRange(format!("maximum is {max}")));
    }

    Ok(ParsedBuiltInValue {
        value:        ImeBuiltInValue::Float(value),
        display_text: value.to_string(),
    })
}

fn parse_integer(
    text: &str,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<ParsedBuiltInValue, ImeRejection> {
    let value = text
        .trim()
        .parse::<i64>()
        .map_err(|_| ImeRejection::InvalidText(INTEGER_INVALID_MESSAGE.to_owned()))?;
    if let Some(min) = min
        && value < min
    {
        return Err(ImeRejection::OutOfRange(format!("minimum is {min}")));
    }
    if let Some(max) = max
        && value > max
    {
        return Err(ImeRejection::OutOfRange(format!("maximum is {max}")));
    }

    Ok(ParsedBuiltInValue {
        value:        ImeBuiltInValue::Integer(value),
        display_text: value.to_string(),
    })
}

fn update_rejection(update: FieldDisplayTextUpdate) -> ImeRejection {
    let message = match update {
        FieldDisplayTextUpdate::Updated => return ImeRejection::StaleAttempt,
        FieldDisplayTextUpdate::MissingField => MISSING_FIELD_MESSAGE,
        FieldDisplayTextUpdate::DuplicateField => DUPLICATE_FIELD_MESSAGE,
        FieldDisplayTextUpdate::MissingText => MISSING_TEXT_MESSAGE,
    };
    ImeRejection::InvalidText(message.to_owned())
}

fn reject(event: &ImeCommitRequested, reason: ImeRejection, commands: &mut Commands) {
    commands.trigger(ImeRejectCommit {
        session_id: event.session_id,
        attempt_id: event.attempt_id,
        reason,
    });
}

#[cfg(test)]
mod tests {
    use super::parse_builtin_value;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeBuiltInValue;
    use crate::ImeRejection;

    #[test]
    fn parses_integer_with_bounds() -> Result<(), ImeRejection> {
        let spec = ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Integer {
            min: Some(1),
            max: Some(9),
        });

        let parsed = parse_builtin_value(&spec, " 7 ")?;

        assert_eq!(parsed.value, ImeBuiltInValue::Integer(7));
        assert_eq!(parsed.display_text, "7");
        Ok(())
    }

    #[test]
    fn rejects_out_of_range_float() {
        let spec = ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Float {
            min: Some(0.0),
            max: Some(1.0),
        });

        assert!(matches!(
            parse_builtin_value(&spec, "2.5"),
            Err(ImeRejection::OutOfRange(_))
        ));
    }
}
