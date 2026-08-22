use bevy::ecs::system::SystemParam;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;
use bevy::prelude::With;

use super::PanelWidget;
use super::WidgetOf;
use crate::cascade::Cascade;
use crate::cascade::Resolved;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPanelChangeClassification;

/// Effective enabled or disabled state of a panel widget.
///
/// Insert this as a resource to set the value every widget inherits unless
/// something between it and the cascade root overrides it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect, Resource)]
#[reflect(Resource)]
pub enum WidgetInteractivity {
    /// The widget can receive interaction.
    Enabled,
    /// The widget cannot receive interaction.
    Disabled,
}

/// Marks a widget whose resolved interactivity is disabled.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq, Reflect)]
#[reflect(Component)]
pub struct WidgetDisabled(());

#[cfg(test)]
impl WidgetDisabled {
    pub(crate) const fn test_marker() -> Self { Self(()) }
}

/// Writes widget interactivity into the owning panel's authored layout tree.
#[derive(SystemParam)]
pub struct PanelWidgetWriter<'w, 's> {
    widgets: Query<'w, 's, (&'static PanelWidget, &'static WidgetOf)>,
    panels: Query<
        'w,
        's,
        (
            &'static mut DiegeticPanel,
            &'static mut DiegeticPanelChangeClassification,
        ),
    >,
}

impl PanelWidgetWriter<'_, '_> {
    /// Authors a local interactivity override on `widget`.
    ///
    /// Returns `false` when the live widget, owning panel, or authored widget
    /// element cannot be resolved. A successful unchanged write returns `true`
    /// without dirtying the panel.
    pub fn override_interactivity(&mut self, widget: Entity, value: WidgetInteractivity) -> bool {
        self.write(widget, Cascade::Override(value))
    }

    /// Makes `widget` inherit interactivity from its nearest authored parent.
    ///
    /// Returns `false` when the live widget, owning panel, or authored widget
    /// element cannot be resolved. A successful unchanged write returns `true`
    /// without dirtying the panel.
    pub fn inherit_interactivity(&mut self, widget: Entity) -> bool {
        self.write(widget, Cascade::Inherit)
    }

    fn write(&mut self, widget_entity: Entity, authored: Cascade<WidgetInteractivity>) -> bool {
        let Ok((widget, widget_of)) = self.widgets.get(widget_entity) else {
            return false;
        };
        let id = widget.id().clone();
        let Ok((mut panel, mut classification)) = self.panels.get_mut(widget_of.panel()) else {
            return false;
        };
        let Some(current) = panel.tree().widget_interactivity(&id) else {
            return false;
        };
        if current == authored {
            return true;
        }
        if !panel.set_widget_interactivity(&id, authored) {
            return false;
        }
        classification.note_widget_interactivity_edit();
        true
    }
}

pub(super) fn resolve_interactivity(
    widgets: Query<
        (Entity, &Resolved<WidgetInteractivity>, Has<WidgetDisabled>),
        (With<PanelWidget>, Changed<Resolved<WidgetInteractivity>>),
    >,
    mut commands: Commands,
) {
    for (entity, resolved, disabled) in &widgets {
        match (resolved.0, disabled) {
            (WidgetInteractivity::Disabled, false) => {
                commands.entity(entity).insert(WidgetDisabled(()));
            },
            (WidgetInteractivity::Enabled, true) => {
                commands.entity(entity).remove::<WidgetDisabled>();
            },
            (WidgetInteractivity::Enabled, false) | (WidgetInteractivity::Disabled, true) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Changed;
    use bevy::prelude::ChildOf;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::Query;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;

    use super::PanelWidgetWriter;
    use super::WidgetDisabled;
    use super::WidgetInteractivity;
    use crate::CascadeEntityCommandsExt as _;
    use crate::ComputedDiegeticPanel;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidget;
    use crate::PanelWidgetReader;
    use crate::WidgetOf;
    use crate::cascade::Cascade;
    use crate::cascade::CascadeFrom;
    use crate::cascade::Resolved;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ComputedWidgetRecord;
    use crate::widgets::WidgetsPlugin;

    #[derive(Default, Resource)]
    struct DisabledChanges(usize);

    fn count_disabled_changes(
        changed: Query<(), Changed<WidgetDisabled>>,
        mut counter: ResMut<DisabledChanges>,
    ) {
        counter.0 += changed.iter().count();
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn widget_tree(interactivity: Option<WidgetInteractivity>) -> LayoutTree {
        let element = El::new().button("action");
        let element = match interactivity {
            Some(value) => element.widget_interactivity(value),
            None => element,
        };
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(element, |_| {});
        builder.build()
    }

    fn subtree_tree(parent: WidgetInteractivity) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::column().widget_interactivity(parent), |builder| {
            builder.with(El::new().button("inherited"), |_| {});
            builder.with(
                El::new()
                    .button("enabled")
                    .widget_interactivity(WidgetInteractivity::Enabled),
                |_| {},
            );
        });
        builder.build()
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        app.world_mut().spawn(panel).id()
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let Ok(widget) = result else {
            return Entity::PLACEHOLDER;
        };
        assert!(widget.is_some());
        let Some(widget) = widget else {
            return Entity::PLACEHOLDER;
        };
        widget
    }

    fn override_widget(app: &mut App, widget: Entity, value: WidgetInteractivity) -> bool {
        let result = app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.override_interactivity(widget, value)
            });
        assert!(result.is_ok());
        let Ok(written) = result else {
            return false;
        };
        written
    }

    fn inherit_widget(app: &mut App, widget: Entity) -> bool {
        let result = app
            .world_mut()
            .run_system_once(move |mut writer: PanelWidgetWriter| {
                writer.inherit_interactivity(widget)
            });
        assert!(result.is_ok());
        let Ok(written) = result else {
            return false;
        };
        written
    }

    #[test]
    fn disabled_widget_is_marked_in_its_reification_frame() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(Some(WidgetInteractivity::Disabled)));

        app.update();

        let widget = resolve_widget(&mut app, panel, "action");
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetInteractivity>>(widget)
                .map(|resolved| resolved.0),
            Some(WidgetInteractivity::Disabled)
        );
    }

    #[test]
    fn cascade_relation_is_explicit_and_independent_of_child_of() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(None));
        app.update();
        let widget = resolve_widget(&mut app, panel, "action");

        assert_eq!(
            app.world()
                .get::<CascadeFrom>(widget)
                .map(CascadeFrom::target),
            Some(panel)
        );
        app.world_mut().entity_mut(widget).remove::<ChildOf>();
        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.update();

        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert!(app.world().get::<ChildOf>(widget).is_none());
    }

    #[test]
    fn global_and_panel_overrides_propagate_through_shared_cache() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(None));
        app.update();
        let widget = resolve_widget(&mut app, panel, "action");
        assert!(app.world().get::<WidgetDisabled>(widget).is_none());

        *app.world_mut().resource_mut::<WidgetInteractivity>() = WidgetInteractivity::Disabled;
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());

        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Enabled);
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_none());

        app.world_mut()
            .commands()
            .entity(panel)
            .inherit_widget_interactivity();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
    }

    #[test]
    fn nearest_layout_override_controls_each_widget() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, subtree_tree(WidgetInteractivity::Disabled));
        app.update();

        let inherited = resolve_widget(&mut app, panel, "inherited");
        let enabled = resolve_widget(&mut app, panel, "enabled");
        assert!(app.world().get::<WidgetDisabled>(inherited).is_some());
        assert!(app.world().get::<WidgetDisabled>(enabled).is_none());
    }

    #[test]
    fn writer_updates_tree_and_inherit_reveals_parent_rule() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, subtree_tree(WidgetInteractivity::Disabled));
        app.update();
        let direct = resolve_widget(&mut app, panel, "inherited");

        assert!(override_widget(
            &mut app,
            direct,
            WidgetInteractivity::Enabled
        ));
        app.update();
        assert!(app.world().get::<WidgetDisabled>(direct).is_none());

        let resolved = resolve_widget(&mut app, panel, "inherited");
        assert_eq!(resolved, direct);
        let panel_component = app.world_mut().get_mut::<DiegeticPanel>(panel);
        assert!(panel_component.is_some());
        let Some(mut panel_component) = panel_component else {
            return;
        };
        panel_component.set_width(120.0);
        app.update();
        assert_eq!(resolve_widget(&mut app, panel, "inherited"), direct);
        assert!(app.world().get::<WidgetDisabled>(direct).is_none());

        assert!(inherit_widget(&mut app, direct));
        app.update();
        assert!(app.world().get::<WidgetDisabled>(direct).is_some());
    }

    #[test]
    fn writer_rejects_stale_targets_and_skips_unchanged_writes() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(Some(WidgetInteractivity::Disabled)));
        app.update();
        let widget = resolve_widget(&mut app, panel, "action");
        let revision = app
            .world()
            .get::<DiegeticPanel>(panel)
            .map(DiegeticPanel::tree_revision);

        assert!(override_widget(
            &mut app,
            widget,
            WidgetInteractivity::Disabled
        ));
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(panel)
                .map(DiegeticPanel::tree_revision),
            revision
        );

        app.world_mut().entity_mut(widget).despawn();
        assert!(!inherit_widget(&mut app, widget));
        let missing_source = app
            .world_mut()
            .spawn((
                PanelWidget::new(PanelElementId::named("missing")),
                WidgetOf::new(panel),
            ))
            .id();
        assert!(!inherit_widget(&mut app, missing_source));
    }

    #[test]
    fn visual_only_subtree_replacement_reuses_widgets_and_refreshes_descendants() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, subtree_tree(WidgetInteractivity::Enabled));
        app.update();
        let inherited_before = resolve_widget(&mut app, panel, "inherited");
        let enabled_before = resolve_widget(&mut app, panel, "enabled");
        let computed_before = app.world().get::<ComputedDiegeticPanel>(panel);
        assert!(computed_before.is_some());
        let Some(computed_before) = computed_before else {
            return;
        };
        let layout_solves = computed_before.layout_solves();
        assert_eq!(
            computed_before
                .widget_records()
                .iter()
                .map(ComputedWidgetRecord::interactivity)
                .collect::<Vec<_>>(),
            vec![
                Cascade::Override(WidgetInteractivity::Enabled),
                Cascade::Override(WidgetInteractivity::Enabled),
            ]
        );

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, subtree_tree(WidgetInteractivity::Disabled));
        assert!(result.is_ok());
        app.update();

        let inherited_after = resolve_widget(&mut app, panel, "inherited");
        let enabled_after = resolve_widget(&mut app, panel, "enabled");
        assert_eq!(inherited_after, inherited_before);
        assert_eq!(enabled_after, enabled_before);
        let computed_after = app.world().get::<ComputedDiegeticPanel>(panel);
        assert!(computed_after.is_some());
        let Some(computed_after) = computed_after else {
            return;
        };
        assert_eq!(
            computed_after
                .widget_records()
                .iter()
                .map(ComputedWidgetRecord::interactivity)
                .collect::<Vec<_>>(),
            vec![
                Cascade::Override(WidgetInteractivity::Disabled),
                Cascade::Override(WidgetInteractivity::Enabled),
            ]
        );
        assert_eq!(computed_after.layout_solves(), layout_solves);
        assert!(app.world().get::<WidgetDisabled>(inherited_after).is_some());
        assert!(app.world().get::<WidgetDisabled>(enabled_after).is_none());
    }

    #[test]
    fn unchanged_resolved_value_does_not_rewrite_disabled_marker() {
        let mut app = test_app();
        app.init_resource::<DisabledChanges>()
            .add_systems(PostUpdate, count_disabled_changes);
        let panel = spawn_panel(&mut app, widget_tree(Some(WidgetInteractivity::Disabled)));
        app.update();
        app.update();
        assert_eq!(app.world().resource::<DisabledChanges>().0, 1);

        let panel_component = app.world_mut().get_mut::<DiegeticPanel>(panel);
        assert!(panel_component.is_some());
        let Some(mut panel_component) = panel_component else {
            return;
        };
        panel_component.set_width(120.0);
        app.update();
        app.update();

        assert_eq!(app.world().resource::<DisabledChanges>().0, 1);
    }

    #[test]
    fn reify_synchronizes_authored_cascade_without_rewriting_equal_values() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(Some(WidgetInteractivity::Disabled)));
        app.update();
        let widget = resolve_widget(&mut app, panel, "action");

        assert_eq!(
            app.world().get::<Cascade<WidgetInteractivity>>(widget),
            Some(&Cascade::Override(WidgetInteractivity::Disabled))
        );
        let before = app
            .world()
            .entity(widget)
            .get_ref::<Cascade<WidgetInteractivity>>()
            .map(|authored| authored.last_changed());
        let panel_component = app.world_mut().get_mut::<DiegeticPanel>(panel);
        assert!(panel_component.is_some());
        let Some(mut panel_component) = panel_component else {
            return;
        };
        panel_component.set_width(120.0);
        app.update();

        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetInteractivity>>()
                .map(|authored| authored.last_changed()),
            before
        );
    }

    #[test]
    fn reader_and_writer_resolve_and_author_in_one_system() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(None));
        app.update();
        let id = PanelElementId::named("action");

        let result = app.world_mut().run_system_once(
            move |reader: PanelWidgetReader, mut writer: PanelWidgetWriter| {
                let widget = reader.entity(panel, &id);
                assert!(widget.is_some());
                let Some(widget) = widget else {
                    return (Entity::PLACEHOLDER, false);
                };
                let written = writer.override_interactivity(widget, WidgetInteractivity::Disabled);
                (widget, written)
            },
        );
        assert!(result.is_ok());
        let Ok((widget, written)) = result else {
            return;
        };
        assert!(written);

        app.update();

        assert_eq!(resolve_widget(&mut app, panel, "action"), widget);
        assert_eq!(
            app.world().get::<DiegeticPanel>(panel).and_then(|panel| {
                panel
                    .tree()
                    .widget_interactivity(&PanelElementId::named("action"))
            }),
            Some(Cascade::Override(WidgetInteractivity::Disabled))
        );
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
    }
}
