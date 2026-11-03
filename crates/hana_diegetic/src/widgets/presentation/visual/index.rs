use std::collections::HashMap;

use bevy::prelude::Changed;
use bevy::prelude::Entity;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::RemovedComponents;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::With;

use super::VisualSlotOverride;
use super::WidgetVisualOverrides;
use super::WidgetVisualSlots;
use crate::widgets::PanelWidget;
use crate::widgets::WidgetOf;

/// Resolved override lookup consumed by the retained-batch route systems.
///
/// Keys are `(panel entity, LayoutTree element index)` — the identity every
/// route already has in hand while rebuilding a record.
#[derive(Default, Resource)]
pub(crate) struct VisualOverrideIndex {
    by_record: HashMap<(Entity, usize), VisualSlotOverride>,
    by_widget: HashMap<Entity, Vec<(Entity, usize)>>,
}

impl VisualOverrideIndex {
    /// Current override for one panel element's retained records.
    #[must_use]
    pub(crate) fn get(&self, panel: Entity, element_index: usize) -> Option<&VisualSlotOverride> {
        self.by_record.get(&(panel, element_index))
    }

    fn insert_widget(
        &mut self,
        widget: Entity,
        entries: Vec<((Entity, usize), VisualSlotOverride)>,
    ) {
        if entries.is_empty() {
            return;
        }
        let mut keys = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            keys.push(key);
            self.by_record.insert(key, value);
        }
        self.by_widget.insert(widget, keys);
    }

    fn remove_widget(&mut self, widget: Entity) {
        let Some(keys) = self.by_widget.remove(&widget) else {
            return;
        };
        for key in keys {
            self.by_record.remove(&key);
        }
    }
}

/// Resolves changed widget overrides into the [`VisualOverrideIndex`].
///
/// Runs after `WidgetSystems::PresentationCommandsApplied`, so slot
/// references attached by this frame's reify and the button state writer's
/// first override insertion are both visible; the `PostUpdate` batch routes
/// read the index later the same frame.
///
/// Removal of stale keys happens for every changed and removed widget before
/// any insertion. A structural tree edit renumbers element indices, so one
/// widget's previous `(panel, element_index)` key can equal another changed
/// widget's current key; removing after inserting would delete that widget's
/// fresh [`VisualOverrideIndex`] entry.
pub(in crate::widgets) fn dispatch_visual_overrides(
    changed_widgets: Query<
        (
            Entity,
            &WidgetOf,
            &WidgetVisualSlots,
            Option<&WidgetVisualOverrides>,
        ),
        Or<(Changed<WidgetVisualOverrides>, Changed<WidgetVisualSlots>)>,
    >,
    live_overrides: Query<(), With<WidgetVisualOverrides>>,
    mut removed_widgets: RemovedComponents<PanelWidget>,
    mut removed_overrides: RemovedComponents<WidgetVisualOverrides>,
    mut index: ResMut<VisualOverrideIndex>,
) {
    for (widget, ..) in &changed_widgets {
        index.remove_widget(widget);
    }
    for widget in removed_overrides.read() {
        if live_overrides.get(widget).is_err() {
            index.remove_widget(widget);
        }
    }
    for widget in removed_widgets.read() {
        index.remove_widget(widget);
    }
    for (widget, widget_of, slots, overrides) in &changed_widgets {
        let entries = overrides.map_or_else(Vec::new, |overrides| {
            let mut by_element = HashMap::<usize, VisualSlotOverride>::new();
            for (slot, value) in overrides.slot_overrides() {
                let Some(element_index) = slots.element_index(slot) else {
                    continue;
                };
                by_element
                    .entry(element_index)
                    .and_modify(|existing| existing.apply(value))
                    .or_insert_with(|| value.clone());
            }
            for &(element_index, ref value) in overrides.element_overrides() {
                by_element
                    .entry(element_index)
                    .and_modify(|existing| existing.apply_element(value))
                    .or_insert_with(|| value.clone());
            }
            by_element
                .into_iter()
                .map(|(element_index, value)| ((widget_of.panel(), element_index), value))
                .collect()
        });
        index.insert_widget(widget, entries);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::Entity;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;

    use crate::PanelElementId;
    use crate::layout::BoundingBox;
    use crate::widgets::PanelWidget;
    use crate::widgets::VisualElementCapabilities;
    use crate::widgets::WidgetOf;
    use crate::widgets::presentation::visual::ComputedVisualSlot;
    use crate::widgets::presentation::visual::VisualOverrideIndex;
    use crate::widgets::presentation::visual::VisualSlotId;
    use crate::widgets::presentation::visual::VisualSlotOverride;
    use crate::widgets::presentation::visual::WidgetVisualOverrides;
    use crate::widgets::presentation::visual::WidgetVisualSlots;

    const SLOT: VisualSlotId = VisualSlotId::new(7);
    const SLOT_ELEMENT_INDEX: usize = 3;
    const OVERRIDE_COLOR: Color = Color::srgb(0.9, 0.1, 0.2);
    const PEER_ELEMENT_INDEX: usize = 5;
    const PEER_OVERRIDE_COLOR: Color = Color::srgb(0.2, 0.8, 0.9);
    const TEXT_OVERRIDE_COLOR: Color = Color::srgb(0.3, 0.2, 0.7);
    const PATH_OVERRIDE_COLOR: Color = Color::srgb(0.7, 0.2, 0.3);

    fn computed_slot(slot: VisualSlotId, element_index: usize) -> ComputedVisualSlot {
        ComputedVisualSlot {
            slot,
            element_index,
            border_box: BoundingBox::default(),
            content_box: BoundingBox::default(),
        }
    }

    fn dispatch_app() -> App {
        let mut app = App::new();
        app.init_resource::<VisualOverrideIndex>()
            .add_systems(Update, super::dispatch_visual_overrides);
        app
    }

    fn spawn_slotted_widget(app: &mut App, panel: Entity) -> Entity {
        app.world_mut()
            .spawn((
                PanelWidget::new(PanelElementId::named("styled")),
                WidgetOf::new(panel),
                WidgetVisualSlots::new(vec![computed_slot(SLOT, SLOT_ELEMENT_INDEX)]),
            ))
            .id()
    }

    fn spawn_overridden_widget(
        app: &mut App,
        panel: Entity,
        name: &str,
        element_index: usize,
        color: Color,
    ) -> Entity {
        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(SLOT, VisualSlotOverride::default().with_fill_color(color));
        app.world_mut()
            .spawn((
                PanelWidget::new(PanelElementId::named(name)),
                WidgetOf::new(panel),
                WidgetVisualSlots::new(vec![computed_slot(SLOT, element_index)]),
                overrides,
            ))
            .id()
    }

    fn indexed_fill_color(app: &App, panel: Entity) -> Option<Color> {
        indexed_fill_color_at(app, panel, SLOT_ELEMENT_INDEX)
    }

    fn indexed_fill_color_at(app: &App, panel: Entity, element_index: usize) -> Option<Color> {
        app.world()
            .resource::<VisualOverrideIndex>()
            .get(panel, element_index)
            .and_then(|value| value.fill_color)
    }

    #[test]
    fn element_override_composes_over_a_slot_without_replacing_its_offset() {
        let mut app = dispatch_app();
        let panel = app.world_mut().spawn_empty().id();
        let slots =
            WidgetVisualSlots::new(vec![computed_slot(SLOT, SLOT_ELEMENT_INDEX)]).with_elements(
                vec![(SLOT_ELEMENT_INDEX, VisualElementCapabilities::default())],
            );
        let mut overrides = WidgetVisualOverrides::default();
        let offset = Vec2::new(2.0, -3.0);
        overrides.set(SLOT, VisualSlotOverride::default().with_offset(offset));
        overrides.set_element(
            SLOT_ELEMENT_INDEX,
            VisualSlotOverride::default().with_fill_color(OVERRIDE_COLOR),
        );
        app.world_mut().spawn((
            PanelWidget::new(PanelElementId::named("styled")),
            WidgetOf::new(panel),
            slots,
            overrides,
        ));

        app.update();

        assert_eq!(
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, SLOT_ELEMENT_INDEX),
            Some(&VisualSlotOverride {
                fill_color: Some(OVERRIDE_COLOR),
                offset: Some(offset),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn element_text_and_path_colors_compose_over_a_slot_override() {
        let mut app = dispatch_app();
        let panel = app.world_mut().spawn_empty().id();
        let slots = WidgetVisualSlots::new(vec![computed_slot(SLOT, SLOT_ELEMENT_INDEX)])
            .with_elements(vec![(SLOT_ELEMENT_INDEX, VisualElementCapabilities::TEXT)]);
        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(
            SLOT,
            VisualSlotOverride::default().with_fill_color(OVERRIDE_COLOR),
        );
        overrides.set_element(
            SLOT_ELEMENT_INDEX,
            VisualSlotOverride::default()
                .with_text_color(TEXT_OVERRIDE_COLOR)
                .with_path_color(PATH_OVERRIDE_COLOR),
        );
        app.world_mut().spawn((
            PanelWidget::new(PanelElementId::named("styled")),
            WidgetOf::new(panel),
            slots,
            overrides,
        ));

        app.update();

        assert_eq!(
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, SLOT_ELEMENT_INDEX),
            Some(&VisualSlotOverride {
                fill_color: Some(OVERRIDE_COLOR),
                text_color: Some(TEXT_OVERRIDE_COLOR),
                path_color: Some(PATH_OVERRIDE_COLOR),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn dispatch_indexes_override_by_panel_and_element_index() {
        let mut app = dispatch_app();
        let panel = app.world_mut().spawn_empty().id();
        let widget = spawn_slotted_widget(&mut app, panel);
        app.update();
        assert_eq!(indexed_fill_color(&app, panel), None);

        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(
            SLOT,
            VisualSlotOverride::default().with_fill_color(OVERRIDE_COLOR),
        );
        app.world_mut().entity_mut(widget).insert(overrides);
        app.update();

        assert_eq!(indexed_fill_color(&app, panel), Some(OVERRIDE_COLOR));
        assert!(
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, SLOT_ELEMENT_INDEX + 1)
                .is_none(),
            "unrelated element indices must stay unindexed",
        );
    }

    #[test]
    fn unknown_slot_ids_index_nothing() {
        let mut app = dispatch_app();
        let panel = app.world_mut().spawn_empty().id();
        let widget = spawn_slotted_widget(&mut app, panel);
        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(
            VisualSlotId::new(99),
            VisualSlotOverride::default().with_fill_color(OVERRIDE_COLOR),
        );
        app.world_mut().entity_mut(widget).insert(overrides);
        app.update();

        assert!(
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, SLOT_ELEMENT_INDEX)
                .is_none()
        );
    }

    #[test]
    fn clearing_and_removal_retire_index_entries() {
        let mut app = dispatch_app();
        let panel = app.world_mut().spawn_empty().id();
        let widget = spawn_slotted_widget(&mut app, panel);
        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(
            SLOT,
            VisualSlotOverride::default().with_fill_color(OVERRIDE_COLOR),
        );
        app.world_mut().entity_mut(widget).insert(overrides);
        app.update();
        assert_eq!(indexed_fill_color(&app, panel), Some(OVERRIDE_COLOR));

        let mut overrides = app
            .world_mut()
            .get_mut::<WidgetVisualOverrides>(widget)
            .expect("widget should keep its override component");
        overrides.clear(SLOT);
        app.update();
        assert_eq!(indexed_fill_color(&app, panel), None);

        let mut overrides = app
            .world_mut()
            .get_mut::<WidgetVisualOverrides>(widget)
            .expect("widget should keep its override component");
        overrides.set(
            SLOT,
            VisualSlotOverride::default().with_fill_color(OVERRIDE_COLOR),
        );
        app.update();
        assert_eq!(indexed_fill_color(&app, panel), Some(OVERRIDE_COLOR));

        app.world_mut()
            .entity_mut(widget)
            .remove::<WidgetVisualOverrides>();
        app.update();
        assert_eq!(indexed_fill_color(&app, panel), None);

        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(
            SLOT,
            VisualSlotOverride::default().with_fill_color(OVERRIDE_COLOR),
        );
        app.world_mut().entity_mut(widget).insert(overrides);
        app.update();
        assert_eq!(indexed_fill_color(&app, panel), Some(OVERRIDE_COLOR));

        app.world_mut().entity_mut(widget).despawn();
        app.update();
        assert_eq!(indexed_fill_color(&app, panel), None);
    }

    #[test]
    fn removed_widget_stale_key_keeps_renumbered_widget_entry() {
        let mut app = dispatch_app();
        let panel = app.world_mut().spawn_empty().id();
        let removed = spawn_overridden_widget(
            &mut app,
            panel,
            "removed",
            SLOT_ELEMENT_INDEX,
            OVERRIDE_COLOR,
        );
        let renumbered = spawn_overridden_widget(
            &mut app,
            panel,
            "renumbered",
            PEER_ELEMENT_INDEX,
            PEER_OVERRIDE_COLOR,
        );
        app.update();
        assert_eq!(
            indexed_fill_color_at(&app, panel, SLOT_ELEMENT_INDEX),
            Some(OVERRIDE_COLOR),
        );
        assert_eq!(
            indexed_fill_color_at(&app, panel, PEER_ELEMENT_INDEX),
            Some(PEER_OVERRIDE_COLOR),
        );

        // One structural edit removes a widget and renumbers the survivor
        // onto the removed widget's old element index in the same frame.
        app.world_mut().entity_mut(removed).despawn();
        app.world_mut()
            .entity_mut(renumbered)
            .insert(WidgetVisualSlots::new(vec![computed_slot(
                SLOT,
                SLOT_ELEMENT_INDEX,
            )]));
        app.update();

        assert_eq!(
            indexed_fill_color_at(&app, panel, SLOT_ELEMENT_INDEX),
            Some(PEER_OVERRIDE_COLOR),
            "the removed widget's stale key must not delete the renumbered widget's entry",
        );
        assert_eq!(
            indexed_fill_color_at(&app, panel, PEER_ELEMENT_INDEX),
            None,
            "the renumbered widget's old key must retire",
        );
    }

    #[test]
    fn overlapping_renumber_keeps_both_changed_widget_entries() {
        let mut app = dispatch_app();
        let panel = app.world_mut().spawn_empty().id();
        let first =
            spawn_overridden_widget(&mut app, panel, "first", SLOT_ELEMENT_INDEX, OVERRIDE_COLOR);
        let second = spawn_overridden_widget(
            &mut app,
            panel,
            "second",
            PEER_ELEMENT_INDEX,
            PEER_OVERRIDE_COLOR,
        );
        app.update();

        // One structural edit swaps the two widgets' element indices, so each
        // widget's old key equals the other's current key and single-pass
        // removal would delete a fresh entry in either iteration order.
        app.world_mut()
            .entity_mut(first)
            .insert(WidgetVisualSlots::new(vec![computed_slot(
                SLOT,
                PEER_ELEMENT_INDEX,
            )]));
        app.world_mut()
            .entity_mut(second)
            .insert(WidgetVisualSlots::new(vec![computed_slot(
                SLOT,
                SLOT_ELEMENT_INDEX,
            )]));
        app.update();

        assert_eq!(
            indexed_fill_color_at(&app, panel, PEER_ELEMENT_INDEX),
            Some(OVERRIDE_COLOR),
        );
        assert_eq!(
            indexed_fill_color_at(&app, panel, SLOT_ELEMENT_INDEX),
            Some(PEER_OVERRIDE_COLOR),
        );
    }
}
