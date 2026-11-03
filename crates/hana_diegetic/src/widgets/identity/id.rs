use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::With;

use crate::PanelElementId;
use crate::panel::DiegeticPanel;
use crate::panel::PanelEntity;
use crate::panel::PanelSpace;
use crate::panel::WidgetEntity;
use crate::widgets::WidgetOf;

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
