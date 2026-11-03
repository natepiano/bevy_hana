use std::collections::HashMap;

use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::RelationshipTarget;
use bevy::prelude::With;

use super::authority;
use super::authority::FocusIndicator;
use super::authority::WidgetFocusAuthority;
use super::authority::WidgetFocusChangeCause;
use super::authority::WidgetFocusTarget;
use super::authority::WidgetFocusable;
use super::authority::WindowFocusScope;
use crate::panel::ComputedDiegeticPanel;
use crate::widgets::PanelWidget;
use crate::widgets::PanelWidgets;
use crate::widgets::WidgetOf;

#[derive(Clone, Copy)]
pub(in crate::widgets::interaction) enum WidgetTraversal {
    Next,
    Previous,
    First,
    Last,
}
pub(in crate::widgets::interaction) fn traverse_focus(
    window: Entity,
    traversal: WidgetTraversal,
    authority: &mut WidgetFocusAuthority,
    panels: &Query<(&ComputedDiegeticPanel, &PanelWidgets)>,
    widgets: &Query<(&PanelWidget, &WidgetOf), With<WidgetFocusable>>,
    commands: &mut Commands<'_, '_>,
) {
    let Some(panel) = authority.active_panel(window) else {
        return;
    };
    let Ok((computed, panel_widgets)) = panels.get(panel) else {
        return;
    };
    let widgets_by_id = panel_widgets
        .iter()
        .filter_map(|widget| {
            let (panel_widget, widget_of) = widgets.get(widget).ok()?;
            (widget_of.panel() == panel).then_some((panel_widget.id(), widget))
        })
        .collect::<HashMap<_, _>>();
    let mut ordered = computed
        .widget_records()
        .iter()
        .filter_map(|record| {
            widgets_by_id
                .get(record.id())
                .copied()
                .map(|widget| (record.preorder(), widget))
        })
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(preorder, _)| *preorder);
    let Some(target_index) = traversal_index(
        traversal,
        authority.focused_widget(window),
        ordered.as_slice(),
    ) else {
        return;
    };
    let target = ordered[target_index].1;
    let interaction_camera = authority
        .scopes
        .get(&window)
        .and_then(|scope| scope.interaction_camera);
    authority::transition_focus(
        authority,
        window,
        Some(WindowFocusScope {
            active_panel: panel,
            indicator: FocusIndicator::Visible,
            widget: WidgetFocusTarget::Widget(target),
            interaction_camera,
        }),
        WidgetFocusChangeCause::Traversal,
        commands,
    );
}

fn traversal_index(
    traversal: WidgetTraversal,
    focused: WidgetFocusTarget,
    ordered: &[(usize, Entity)],
) -> Option<usize> {
    if ordered.is_empty() {
        return None;
    }
    let focused_index = match focused {
        WidgetFocusTarget::Widget(focused) => ordered
            .iter()
            .position(|(_, candidate)| *candidate == focused),
        WidgetFocusTarget::Unfocused => None,
    };
    match traversal {
        WidgetTraversal::First => Some(0),
        WidgetTraversal::Last => Some(ordered.len() - 1),
        WidgetTraversal::Next => Some(focused_index.map_or(0, |index| (index + 1) % ordered.len())),
        WidgetTraversal::Previous => Some(focused_index.map_or(ordered.len() - 1, |index| {
            index.checked_sub(1).unwrap_or(ordered.len() - 1)
        })),
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Window;

    use super::*;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::WidgetInput;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::RequestWidgetFocus;
    use crate::widgets::WidgetsPlugin;

    const PANEL_HEIGHT: f32 = 50.0;
    const PANEL_WIDTH: f32 = 100.0;
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn widget_tree(ids: &[&str]) -> LayoutTree {
        let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
        for id in ids {
            builder.with(El::new().button(*id), |_| {});
        }
        builder.build()
    }

    fn widget_and_field_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(PANEL_WIDTH, PANEL_HEIGHT);
        builder.with(El::new().button("first"), |_| {});
        builder.with(
            El::new().editable_field(
                "editable",
                ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test")),
                crate::TextStyle::new(10.0),
            ),
            |_| {},
        );
        builder.with(El::new().button("last"), |_| {});
        builder.build()
    }

    fn spawn_panel(app: &mut App, ids: &[&str]) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
            .with_tree(widget_tree(ids))
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        app.world_mut().spawn(panel).id()
    }

    fn widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let Ok(widget) = result else {
            return Entity::PLACEHOLDER;
        };
        assert!(widget.is_some());
        widget.unwrap_or(Entity::PLACEHOLDER)
    }

    fn focused(app: &App, window: Entity) -> Option<Entity> {
        match app
            .world()
            .resource::<WidgetFocusAuthority>()
            .focused_widget(window)
        {
            WidgetFocusTarget::Widget(widget) => Some(widget),
            WidgetFocusTarget::Unfocused => None,
        }
    }

    fn request_focus(app: &mut App, window: Entity, widget: Entity) {
        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
    }

    #[test]
    fn traversal_uses_record_preorder_and_wraps() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["first", "second", "third"]);
        app.update();
        let first = widget(&mut app, panel, "first");
        let second = widget(&mut app, panel, "second");
        let third = widget(&mut app, panel, "third");
        request_focus(&mut app, window, second);

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();
        assert_eq!(focused(&app, window), Some(third));

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();
        assert_eq!(focused(&app, window), Some(first));

        app.world_mut()
            .write_message(WidgetInput::FocusPrevious { window });
        app.update();
        assert_eq!(focused(&app, window), Some(third));

        app.world_mut()
            .write_message(WidgetInput::FocusFirst { window });
        app.update();
        assert_eq!(focused(&app, window), Some(first));

        app.world_mut()
            .write_message(WidgetInput::FocusLast { window });
        app.update();
        assert_eq!(focused(&app, window), Some(third));
    }

    #[test]
    fn traversal_includes_editable_fields_without_author_focus_configuration() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let result = DiegeticPanel::world()
            .size(Mm(PANEL_WIDTH), Mm(PANEL_HEIGHT))
            .with_tree(widget_and_field_tree())
            .build();
        assert!(result.is_ok());
        let Ok(panel_data) = result else {
            return;
        };
        let panel = app.world_mut().spawn(panel_data).id();
        app.update();
        let first = widget(&mut app, panel, "first");
        let editable = widget(&mut app, panel, "editable");
        let last = widget(&mut app, panel, "last");
        assert!(app.world().get::<WidgetFocusable>(editable).is_some());
        request_focus(&mut app, window, first);

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();
        assert_eq!(focused(&app, window), Some(editable));

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();
        assert_eq!(focused(&app, window), Some(last));
    }

    #[test]
    fn reorder_changes_traversal_without_respawning_widgets() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(&mut app, &["first", "second", "third"]);
        app.update();
        let first = widget(&mut app, panel, "first");
        let second = widget(&mut app, panel, "second");
        let third = widget(&mut app, panel, "third");
        request_focus(&mut app, window, third);

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, widget_tree(&["third", "first", "second"]));
        assert!(result.is_ok());
        app.update();

        assert_eq!(widget(&mut app, panel, "first"), first);
        assert_eq!(widget(&mut app, panel, "second"), second);
        assert_eq!(widget(&mut app, panel, "third"), third);
        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();
        assert_eq!(focused(&app, window), Some(first));
    }
}
