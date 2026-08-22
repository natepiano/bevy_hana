use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::With;
use bitflags::bitflags;

use super::Button;
use super::Slider;
use super::StateAppearance;
use super::WidgetOf;
use crate::ImePanelField;
use crate::PanelBuildError;
use crate::PanelElementId;
use crate::cascade::Cascade;
use crate::layout::BoundingBox;
use crate::layout::LayoutTree;
use crate::panel::DiegeticPanel;
use crate::panel::PanelEntity;
use crate::panel::PanelSpace;
use crate::panel::WidgetEntity;

/// Runtime identity of a reified panel widget.
#[derive(Component, Clone, Debug, Eq, PartialEq)]
pub struct PanelWidget {
    id: PanelElementId,
}

impl PanelWidget {
    pub(crate) const fn new(id: PanelElementId) -> Self { Self { id } }

    /// Returns the widget's panel-local authored id.
    #[must_use]
    pub const fn id(&self) -> &PanelElementId { &self.id }
}

/// Read-only lookup from panel-local widget identity to a live widget entity.
#[derive(SystemParam)]
pub struct PanelWidgetReader<'w, 's> {
    panels:  Query<'w, 's, (&'static PanelWidgetIndex, &'static PanelSpace), With<DiegeticPanel>>,
    widgets: Query<'w, 's, (&'static PanelWidget, &'static WidgetOf)>,
}

impl PanelWidgetReader<'_, '_> {
    /// Resolves `id` within `panel` to its live reified widget entity.
    ///
    /// Returns `None` when the panel or id is missing, the widget has not been
    /// reified, or the panel-local index points at a stale or mismatched entity.
    #[must_use]
    pub fn entity(&self, panel: Entity, id: &PanelElementId) -> Option<Entity> {
        let (index, _) = self.panels.get(panel).ok()?;
        let entity = index.entity(id)?;
        let (widget, widget_of) = self.widgets.get(entity).ok()?;
        (widget.id() == id && widget_of.panel() == panel).then_some(entity)
    }

    /// Resolves `id` within a typed owner panel to a widget in the same space.
    ///
    /// A stale owner handle returns `None`, as does a stale panel-local widget
    /// index or a widget whose [`WidgetOf`] owner changed.
    #[must_use]
    pub fn typed_entity<Space>(
        &self,
        panel: PanelEntity<Space>,
        id: &PanelElementId,
    ) -> Option<WidgetEntity<Space>> {
        let panel_entity = panel.entity();
        let (index, live_space) = self.panels.get(panel_entity).ok()?;
        if *live_space != panel.expected_space() {
            return None;
        }
        let entity = index.entity(id)?;
        let (widget, widget_of) = self.widgets.get(entity).ok()?;
        (widget.id() == id && widget_of.panel() == panel_entity)
            .then(|| WidgetEntity::from_validated(entity, panel_entity, panel.expected_space()))
    }
}

#[derive(Component, Default)]
pub(crate) struct PanelWidgetIndex(HashMap<PanelElementId, Entity>);

impl PanelWidgetIndex {
    pub(crate) fn clear(&mut self) { self.0.clear(); }

    fn entity(&self, id: &PanelElementId) -> Option<Entity> { self.0.get(id).copied() }

    pub(crate) fn maps_to(&self, id: &PanelElementId, entity: Entity) -> bool {
        self.entity(id) == Some(entity)
    }

    #[cfg(test)]
    pub(crate) fn insert(&mut self, id: PanelElementId, entity: Entity) {
        self.0.insert(id, entity);
    }

    pub(crate) fn replace(&mut self, index: HashMap<PanelElementId, Entity>) { self.0 = index; }
}

#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub(crate) enum WidgetKind {
    Button,
    EditableField,
    Slider,
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ComputedWidgetRecord {
    id:                        PanelElementId,
    kind:                      WidgetKind,
    preorder:                  usize,
    authored:                  WidgetSpec,
    appearance:                StateAppearance,
    part_appearances:          Vec<(usize, StateAppearance)>,
    interactivity:             Cascade<super::WidgetInteractivity>,
    rect:                      BoundingBox,
    clipped_rect:              Option<BoundingBox>,
    interaction_rank:          usize,
    generated_editor_elements: Vec<usize>,
    visual_elements:           Vec<(usize, VisualElementCapabilities)>,
    visual_slots:              Vec<super::ComputedVisualSlot>,
}

impl ComputedWidgetRecord {
    pub(crate) const fn new(
        id: PanelElementId,
        preorder: usize,
        authored: WidgetSpec,
        appearance: StateAppearance,
        interactivity: Cascade<super::WidgetInteractivity>,
        rect: BoundingBox,
        clipped_rect: Option<BoundingBox>,
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
            clipped_rect,
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

    pub(crate) const fn interactivity(&self) -> Cascade<super::WidgetInteractivity> {
        self.interactivity
    }

    pub(crate) const fn rect(&self) -> BoundingBox { self.rect }

    pub(crate) const fn clipped_rect(&self) -> Option<BoundingBox> { self.clipped_rect }

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

    pub(crate) fn visual_slots(&self) -> &[super::ComputedVisualSlot] { &self.visual_slots }

    pub(crate) fn push_visual_slot(&mut self, slot: super::ComputedVisualSlot) {
        self.visual_slots.push(slot);
    }
}

pub(crate) fn validate_tree(tree: &LayoutTree) -> Result<(), PanelBuildError> {
    if let Some(duplicate) = tree.duplicate_named_element_id() {
        return Err(PanelBuildError::DuplicateElementId(duplicate.clone()));
    }
    tree.validate_widgets()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;

    use super::PanelWidget;
    use super::PanelWidgetIndex;
    use super::PanelWidgetReader;
    use crate::DiegeticPanel;
    use crate::PanelElementId;
    use crate::PanelEntityReader;
    use crate::WidgetOf;

    #[test]
    fn reader_rejects_index_after_owner_panel_component_is_removed() {
        let mut app = App::new();
        let panel = app.world_mut().spawn(DiegeticPanel::default()).id();
        let id = PanelElementId::named("action");
        let widget = app
            .world_mut()
            .spawn((PanelWidget::new(id.clone()), WidgetOf::new(panel)))
            .id();
        let index = app.world_mut().get_mut::<PanelWidgetIndex>(panel);
        assert!(index.is_some());
        let Some(mut index) = index else {
            return;
        };
        index.replace(HashMap::from([(id.clone(), widget)]));

        let before = app.world_mut().run_system_once({
            let id = id.clone();
            move |reader: PanelWidgetReader| reader.entity(panel, &id)
        });
        assert!(before.is_ok());
        assert_eq!(before.ok().flatten(), Some(widget));

        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
        assert!(app.world().get_entity(panel).is_ok());
        assert!(app.world().get::<PanelWidgetIndex>(panel).is_some());

        let after = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(after.is_ok());
        assert_eq!(after.ok().flatten(), None);
    }

    #[test]
    fn typed_reader_rejects_owner_handle_after_space_changes() {
        let mut app = App::new();
        app.add_plugins(crate::HeadlessLayoutPlugin);
        let panel = app.world_mut().spawn(DiegeticPanel::default()).id();
        let id = PanelElementId::named("action");
        let widget = app
            .world_mut()
            .spawn((PanelWidget::new(id.clone()), WidgetOf::new(panel)))
            .id();
        let index = app.world_mut().get_mut::<PanelWidgetIndex>(panel);
        assert!(index.is_some());
        let Some(mut index) = index else {
            return;
        };
        index.replace(HashMap::from([(id.clone(), widget)]));
        let owner = app
            .world_mut()
            .run_system_once(move |reader: PanelEntityReader| reader.world(panel));
        assert!(matches!(owner, Ok(Some(_))));
        let Some(owner) = owner.ok().flatten() else {
            return;
        };

        let world_widget = app.world_mut().run_system_once({
            let id = id.clone();
            move |reader: PanelWidgetReader| reader.typed_entity(owner, &id)
        });
        assert!(world_widget.is_ok());
        assert_eq!(
            world_widget.ok().flatten().map(|widget| widget.entity()),
            Some(widget)
        );

        let screen_panel = DiegeticPanel::screen()
            .size(crate::Px(100.0), crate::Px(40.0))
            .screen_position(0.0, 0.0)
            .layout(|_| {})
            .build();
        assert!(screen_panel.is_ok());
        let Some(screen_panel) = screen_panel.ok() else {
            return;
        };
        app.world_mut().entity_mut(panel).insert(screen_panel);

        let typed = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.typed_entity(owner, &id));
        assert!(typed.is_ok());
        assert_eq!(typed.ok().flatten(), None);
    }
}
