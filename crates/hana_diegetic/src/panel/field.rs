//! Editable panel field records computed at the panel boundary.

use bevy::math::Vec2;
use bevy::prelude::Color;

use crate::AlignX;
use crate::AlignY;
use crate::Border;
use crate::CornerRadius;
use crate::ImeEditableFieldSpec;
use crate::Padding;
use crate::PanelElementId;
use crate::TextStyle;
use crate::layout::BoundingBox;
use crate::layout::EditorPart;
use crate::layout::LayoutResult;
use crate::layout::LayoutTree;

/// Authored visual presentation copied by the transient field editor.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PanelFieldPresentation {
    pub(crate) background:        Option<Color>,
    pub(crate) border:            Option<Border>,
    pub(crate) corner_radius:     CornerRadius,
    pub(crate) padding:           Padding,
    pub(crate) align_x:           AlignX,
    pub(crate) align_y:           AlignY,
    pub(crate) text_style:        Option<TextStyle>,
    pub(crate) editor_text:       Option<EditorPart>,
    pub(crate) editor_selection:  Option<EditorPart>,
    pub(crate) editor_caret:      Option<EditorPart>,
    pub(crate) editor_validation: Option<EditorPart>,
}

/// Computed editable field on a laid-out panel.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelFieldRecord {
    /// Panel-local semantic identity.
    pub field_id:            PanelElementId,
    /// Element bounds in panel-local layout points.
    pub bounds:              BoundingBox,
    /// Editable behavior authored for this field.
    pub field_spec:          ImeEditableFieldSpec,
    /// Text displayed by this field when the record was computed.
    pub display_text:        String,
    /// Source element index in the panel's `LayoutTree`.
    pub element_index:       usize,
    /// Whether this id is duplicated elsewhere in the panel.
    pub duplicate_id:        bool,
    /// Authored surface and text presentation for the transient editor.
    pub(crate) presentation: PanelFieldPresentation,
}

impl PanelFieldRecord {
    /// Returns `true` when `panel_local` lies inside this record's bounds.
    #[must_use]
    pub fn contains(&self, panel_local: Vec2) -> bool { self.bounds.contains(panel_local) }

    pub(crate) const fn presentation(&self) -> &PanelFieldPresentation { &self.presentation }
}

pub(super) fn collect_panel_field_records(
    tree: &LayoutTree,
    result: &LayoutResult,
) -> (Vec<PanelFieldRecord>, Vec<PanelElementId>) {
    let mut records = Vec::new();
    let mut seen = Vec::new();
    let mut duplicates = Vec::new();

    for (element_index, computed) in result.computed.iter().enumerate() {
        let Some(field) = tree.editable_field(element_index) else {
            continue;
        };
        let Some(presentation) = tree.editable_field_presentation(element_index) else {
            continue;
        };
        if seen.contains(&field.field_id) && !duplicates.contains(&field.field_id) {
            duplicates.push(field.field_id.clone());
        }
        seen.push(field.field_id.clone());
        records.push(PanelFieldRecord {
            field_id: field.field_id.clone(),
            bounds: computed.bounds,
            field_spec: field.field_spec.clone(),
            display_text: tree
                .field_display_text(element_index)
                .unwrap_or_default()
                .to_owned(),
            element_index,
            duplicate_id: false,
            presentation,
        });
    }

    for record in &mut records {
        record.duplicate_id = duplicates.contains(&record.field_id);
    }

    (records, duplicates)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::math::Vec2;
    use bevy_kana::ToF32;

    use super::collect_panel_field_records;
    use crate::El;
    use crate::ImeBuiltInFieldKind;
    use crate::ImeBuiltInFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::TextDimensions;
    use crate::TextMeasure;
    use crate::TextStyle;
    use crate::layout::LayoutEngine;

    fn field_spec() -> ImeEditableFieldSpec {
        ImeEditableFieldSpec::BuiltIn(ImeBuiltInFieldSpec::new(ImeBuiltInFieldKind::Text))
    }

    fn measure(text: &str, measure: &TextMeasure) -> TextDimensions {
        TextDimensions {
            width:       text.len().to_f32() * measure.size,
            height:      measure.size,
            line_height: measure.size,
        }
    }

    #[test]
    fn collects_authored_field_record_text_and_bounds() {
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(El::new().editable_field("name", field_spec()), |builder| {
            builder.text(("Gain", TextStyle::new(10.0)));
        });
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(measure));
        let result = engine.compute(&tree, 100.0, 40.0, 1.0);

        let (records, duplicates) = collect_panel_field_records(&tree, &result);

        assert!(duplicates.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].field_id.as_str(), Some("name"));
        assert_eq!(records[0].display_text, "Gain");
        assert!(records[0].contains(Vec2::new(0.0, 0.0)));
    }

    #[test]
    fn marks_duplicate_field_ids() {
        let mut builder = LayoutBuilder::new(100.0, 40.0);
        builder.with(El::new().editable_field("value", field_spec()), |_| {});
        builder.with(El::new().editable_field("value", field_spec()), |_| {});
        let tree = builder.build();
        let engine = LayoutEngine::new(Arc::new(measure));
        let result = engine.compute(&tree, 100.0, 40.0, 1.0);

        let (records, duplicates) = collect_panel_field_records(&tree, &result);

        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0].as_str(), Some("value"));
        assert!(records.iter().all(|record| record.duplicate_id));
    }
}
