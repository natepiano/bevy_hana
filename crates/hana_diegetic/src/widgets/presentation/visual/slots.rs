use bevy::prelude::Component;

use crate::layout::BoundingBox;
use crate::widgets::StateAppearance;
use crate::widgets::VisualElementCapabilities;

/// Stable private id for one widget-owned visual slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct VisualSlotId(u32);

impl VisualSlotId {
    /// Root-surface slot authored by [`El::button`](crate::El::button) on the
    /// element carrying the widget. Button state presentation writes this slot
    /// and resolved part-element overrides for owned descendant visual recipients.
    pub(crate) const BUTTON_ROOT: Self = Self(u32::MAX);
    /// Root slot authored by [`El::slider`](crate::El::slider) on the element
    /// carrying the widget. Pointer projection reads its solved content box.
    pub(crate) const SLIDER_ROOT: Self = Self(u32::MAX - 1);
    /// Thumb slot authored by [`El::slider_thumb`](crate::El::slider_thumb) on
    /// one ordinary descendant of a slider. Value presentation reads its border
    /// box for the active-axis extent and solved authored center, then writes
    /// the slot's panel-local translation.
    pub(crate) const SLIDER_THUMB: Self = Self(u32::MAX - 2);
    /// Optional invisible thumb envelope for pointer hits and directed travel.
    pub(crate) const SLIDER_THUMB_HIT: Self = Self(u32::MAX - 10);
    /// Fill slot authored by [`El::slider_fill`](crate::El::slider_fill) on
    /// one ordinary, noninteractive descendant of a slider. Its authored
    /// border box starts at the track's leading edge.
    pub(crate) const SLIDER_FILL: Self = Self(u32::MAX - 7);
    /// Faint reachable interval behind a slider's live fill and authored thumb.
    pub(crate) const SLIDER_BAND: Self = Self(u32::MAX - 9);
    /// Add-only drop shadow translated with the authored knob.
    pub(crate) const SLIDER_KNOB_SHADOW: Self = Self(u32::MAX - 11);
    /// Marker slot authored by [`El::slider_marker`](crate::El::slider_marker).
    pub(crate) const SLIDER_MARKER: Self = Self(u32::MAX - 4);
    /// Track slot authored by [`El::switch`](crate::El::switch).
    pub(crate) const SWITCH_ROOT: Self = Self(u32::MAX - 5);
    /// Moving knob slot authored by [`El::switch_knob`](crate::El::switch_knob).
    pub(crate) const SWITCH_KNOB: Self = Self(u32::MAX - 6);
    /// Ordinary caption records linked to a sibling widget's dim state.
    pub(crate) const DIMMED_CAPTION: Self = Self(u32::MAX - 8);
    /// Root-surface slot authored by [`El::editable_field`](crate::El::editable_field).
    /// Visible keyboard-focus presentation writes this slot and resolved part-element
    /// overrides for owned descendant visual recipients.
    pub(crate) const EDITABLE_ROOT: Self = Self(u32::MAX - 3);

    /// Creates a slot id from a test-chosen stable value in renderer tests.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn new(value: u32) -> Self { Self(value) }
}

/// One slot-to-record reference carried by a `ComputedWidgetRecord`.
///
/// The element index resolves to every retained record the slot element
/// authored: its SDF fill/border surface, image quad, text runs, and
/// panel-line groups all carry the same `LayoutTree` element index.
/// `border_box` and `content_box` carry the slot element's solved outer bounds
/// and its padding/border-excluded interior, both in panel-layout coordinates,
/// so slider pointer projection reads the live content box without inspecting
/// retained render batches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ComputedVisualSlot {
    /// Authored stable slot id.
    pub slot:          VisualSlotId,
    /// Index of the slot element in the panel's `LayoutTree`.
    pub element_index: usize,
    /// Solved border box of the slot element in panel-layout coordinates.
    pub border_box:    BoundingBox,
    /// Solved padding/border-excluded content box of the slot element.
    pub content_box:   BoundingBox,
}

/// Reified slot-to-record references owned by one widget entity.
#[derive(Clone, Component, Debug, Default, PartialEq)]
pub(crate) struct WidgetVisualSlots {
    slots:                                Vec<ComputedVisualSlot>,
    elements:                             Vec<(usize, VisualElementCapabilities)>,
    pub(super) generated_editor_elements: Vec<usize>,
    part_appearances:                     Vec<(usize, StateAppearance)>,
}

impl WidgetVisualSlots {
    #[must_use]
    pub(crate) const fn new(slots: Vec<ComputedVisualSlot>) -> Self {
        Self {
            slots,
            elements: Vec::new(),
            generated_editor_elements: Vec::new(),
            part_appearances: Vec::new(),
        }
    }

    /// Whether this element is a caption linked from outside the widget subtree.
    #[must_use]
    pub(crate) fn is_dimmed_caption(&self, element_index: usize) -> bool {
        self.slots.iter().any(|slot| {
            slot.slot == VisualSlotId::DIMMED_CAPTION && slot.element_index == element_index
        })
    }

    #[must_use]
    pub(crate) fn with_elements(
        mut self,
        elements: Vec<(usize, VisualElementCapabilities)>,
    ) -> Self {
        self.elements = elements;
        self
    }

    #[must_use]
    /// Stores sorted editor-generated indices excluded from widget-level inheritance.
    pub(crate) fn with_generated_editor_elements(
        mut self,
        generated_editor_elements: Vec<usize>,
    ) -> Self {
        self.generated_editor_elements = generated_editor_elements;
        self
    }

    #[must_use]
    pub(crate) fn with_part_appearances(
        mut self,
        part_appearances: Vec<(usize, StateAppearance)>,
    ) -> Self {
        self.part_appearances = part_appearances;
        self
    }

    /// Returns every retained-record recipient owned by the widget declaration.
    #[must_use]
    pub(crate) fn elements(&self) -> &[(usize, VisualElementCapabilities)] { &self.elements }

    /// Whether this retained recipient came from inline-editor generation.
    pub(super) fn is_generated_editor_element(&self, element_index: usize) -> bool {
        self.generated_editor_elements
            .binary_search(&element_index)
            .is_ok()
    }

    /// Returns every descendant with an explicitly authored state appearance.
    #[must_use]
    pub(crate) fn part_appearances(&self) -> &[(usize, StateAppearance)] { &self.part_appearances }

    /// Resolves a stable slot id to its current `LayoutTree` element index.
    #[must_use]
    pub(crate) fn element_index(&self, slot: VisualSlotId) -> Option<usize> {
        self.slots
            .iter()
            .find(|computed| computed.slot == slot)
            .map(|computed| computed.element_index)
    }

    /// Returns the slot's solved padding/border-excluded content box.
    #[must_use]
    pub(crate) fn content_box(&self, slot: VisualSlotId) -> Option<BoundingBox> {
        self.slots
            .iter()
            .find(|computed| computed.slot == slot)
            .map(|computed| computed.content_box)
    }

    /// Returns the slot's solved border box.
    #[must_use]
    pub(crate) fn border_box(&self, slot: VisualSlotId) -> Option<BoundingBox> {
        self.slots
            .iter()
            .find(|computed| computed.slot == slot)
            .map(|computed| computed.border_box)
    }
}

#[cfg(test)]
mod tests {
    use super::VisualSlotId;

    #[test]
    fn builtin_visual_slots_have_distinct_ids() {
        let slots = [
            VisualSlotId::BUTTON_ROOT,
            VisualSlotId::SLIDER_ROOT,
            VisualSlotId::SLIDER_THUMB,
            VisualSlotId::SLIDER_THUMB_HIT,
            VisualSlotId::SLIDER_FILL,
            VisualSlotId::SLIDER_BAND,
            VisualSlotId::SLIDER_KNOB_SHADOW,
            VisualSlotId::SLIDER_MARKER,
            VisualSlotId::SWITCH_ROOT,
            VisualSlotId::SWITCH_KNOB,
            VisualSlotId::DIMMED_CAPTION,
            VisualSlotId::EDITABLE_ROOT,
        ];
        for (index, slot) in slots.iter().enumerate() {
            assert!(
                !slots[..index].contains(slot),
                "duplicate visual slot {slot:?}"
            );
        }
    }
}
