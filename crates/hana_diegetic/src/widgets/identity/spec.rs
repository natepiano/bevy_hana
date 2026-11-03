use bevy::prelude::Component;
use bitflags::bitflags;

use crate::ImePanelField;
use crate::PanelBuildError;
use crate::PanelElementId;
use crate::cascade::Cascade;
use crate::layout::BoundingBox;
use crate::layout::LayoutTree;
use crate::widgets::Button;
use crate::widgets::ComputedVisualSlot;
use crate::widgets::Slider;
use crate::widgets::StateAppearance;
use crate::widgets::TooltipBoundary;
use crate::widgets::WidgetInteractivity;

/// Semantic input role of a reified widget entity.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub enum WidgetKind {
    /// A push button.
    Button,
    /// A single-line editable value field.
    EditableField,
    /// A slider track and thumb.
    Slider,
}

impl WidgetKind {
    /// Whether a focused widget of this kind owns text-entry keystrokes.
    #[must_use]
    pub const fn accepts_text_entry(self) -> bool { matches!(self, Self::EditableField) }
}

#[derive(Clone, Component, Debug, PartialEq)]
pub(crate) enum WidgetSpec {
    Button(Button),
    EditableField(ImePanelField),
    Slider(Slider),
}

bitflags! {
    /// Retained-record properties a layout element can present for a widget.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub(crate) struct VisualElementCapabilities: u8 {
        /// A retained SDF fill record.
        const SDF_FILL = 1 << 0;
        /// A retained SDF border record.
        const SDF_BORDER = 1 << 1;
        /// A retained SDF material override.
        const SDF_MATERIAL = 1 << 2;
        /// Retained text content.
        const TEXT = 1 << 3;
        /// Retained image content.
        const IMAGE = 1 << 4;
        /// Retained panel-draw primitives.
        const DRAW = 1 << 5;
    }
}

impl WidgetSpec {
    pub(crate) const fn kind(&self) -> WidgetKind {
        match self {
            Self::Button(_) => WidgetKind::Button,
            Self::EditableField(_) => WidgetKind::EditableField,
            Self::Slider(_) => WidgetKind::Slider,
        }
    }
}

/// The part of a widget the pointer can hit once its ancestors' clips apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WidgetHitArea {
    /// Ancestor clips hide the whole widget, so it takes no pointer hits.
    FullyClipped,
    /// The visible part of the widget's rect.
    Visible(BoundingBox),
}

impl WidgetHitArea {
    /// The part of a widget at `rect` that ancestor clips leave visible.
    pub(crate) fn within(rect: BoundingBox, clip: BoundingBox) -> Self {
        rect.intersect(&clip)
            .map_or(Self::FullyClipped, Self::Visible)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ComputedWidgetRecord {
    id:                        PanelElementId,
    kind:                      WidgetKind,
    preorder:                  usize,
    authored:                  WidgetSpec,
    appearance:                StateAppearance,
    part_appearances:          Vec<(usize, StateAppearance)>,
    interactivity:             Cascade<WidgetInteractivity>,
    rect:                      BoundingBox,
    hit_area:                  WidgetHitArea,
    tooltip_boundary:          TooltipBoundary,
    interaction_rank:          usize,
    generated_editor_elements: Vec<usize>,
    visual_elements:           Vec<(usize, VisualElementCapabilities)>,
    visual_slots:              Vec<ComputedVisualSlot>,
}

impl ComputedWidgetRecord {
    pub(crate) const fn new(
        id: PanelElementId,
        preorder: usize,
        authored: WidgetSpec,
        appearance: StateAppearance,
        interactivity: Cascade<WidgetInteractivity>,
        rect: BoundingBox,
        hit_area: WidgetHitArea,
        tooltip_boundary: TooltipBoundary,
    ) -> Self {
        let kind = authored.kind();
        Self {
            id,
            kind,
            preorder,
            authored,
            appearance,
            part_appearances: Vec::new(),
            interactivity,
            rect,
            hit_area,
            tooltip_boundary,
            interaction_rank: 0,
            generated_editor_elements: Vec::new(),
            visual_elements: Vec::new(),
            visual_slots: Vec::new(),
        }
    }

    pub(crate) const fn id(&self) -> &PanelElementId { &self.id }

    pub(crate) const fn kind(&self) -> WidgetKind { self.kind }

    pub(crate) const fn preorder(&self) -> usize { self.preorder }

    pub(crate) const fn authored(&self) -> &WidgetSpec { &self.authored }

    pub(crate) const fn appearance(&self) -> &StateAppearance { &self.appearance }

    pub(crate) const fn interactivity(&self) -> Cascade<WidgetInteractivity> { self.interactivity }

    pub(crate) const fn rect(&self) -> BoundingBox { self.rect }

    pub(crate) const fn hit_area(&self) -> WidgetHitArea { self.hit_area }

    pub(crate) const fn tooltip_boundary(&self) -> TooltipBoundary { self.tooltip_boundary }

    pub(crate) const fn interaction_rank(&self) -> usize { self.interaction_rank }

    pub(crate) const fn set_interaction_rank(&mut self, interaction_rank: usize) {
        self.interaction_rank = interaction_rank;
    }

    pub(crate) fn visual_elements(&self) -> &[(usize, VisualElementCapabilities)] {
        &self.visual_elements
    }

    /// Returns the editor-generated element indices owned by this widget.
    pub(crate) fn generated_editor_elements(&self) -> &[usize] { &self.generated_editor_elements }

    /// Records one editor-generated element index.
    pub(crate) fn push_generated_editor_element(&mut self, element_index: usize) {
        let insertion_index = self
            .generated_editor_elements
            .partition_point(|existing_index| *existing_index < element_index);
        self.generated_editor_elements
            .insert(insertion_index, element_index);
    }

    pub(crate) fn push_visual_element(
        &mut self,
        element_index: usize,
        capabilities: VisualElementCapabilities,
    ) {
        let insertion_index = self
            .visual_elements
            .partition_point(|(existing_index, _)| *existing_index < element_index);
        self.visual_elements
            .insert(insertion_index, (element_index, capabilities));
    }

    pub(crate) fn part_appearances(&self) -> &[(usize, StateAppearance)] { &self.part_appearances }

    pub(crate) fn push_part_appearance(
        &mut self,
        element_index: usize,
        appearance: StateAppearance,
    ) {
        let insertion_index = self
            .part_appearances
            .partition_point(|(existing_index, _)| *existing_index < element_index);
        self.part_appearances
            .insert(insertion_index, (element_index, appearance));
    }

    pub(crate) fn visual_slots(&self) -> &[ComputedVisualSlot] { &self.visual_slots }

    pub(crate) fn push_visual_slot(&mut self, slot: ComputedVisualSlot) {
        self.visual_slots.push(slot);
    }
}

pub(crate) fn validate_tree(tree: &LayoutTree) -> Result<(), PanelBuildError> {
    if let Some(duplicate) = tree.duplicate_named_element_id() {
        return Err(PanelBuildError::DuplicateElementId(duplicate.clone()));
    }
    tree.validate_widgets()
}
