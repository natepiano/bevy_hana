use std::collections::HashMap;
use std::ops::Deref;
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
use bevy::camera::primitives::Aabb;
#[cfg(test)]
use bevy::prelude::App;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::EntityCommands;
use bevy::prelude::FromWorld;
use bevy::prelude::GlobalTransform;
use bevy::prelude::In;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::ReflectFromWorld;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Visibility;
use bevy::prelude::World as BevyWorld;
#[cfg(test)]
use hana_valence::AnchoredHere;
use hana_valence::ResolvedAnchorGeometry;

use super::attachment::ScreenTooltipAttachmentCorrection;
use super::declaration::AuthoredTooltipTargetSpace;
use super::declaration::PrepareTooltip;
use super::declaration::Tooltip;
use super::materialization::MaterializedTooltip;
use super::materialization::PreMaterializationComponent;
use super::materialization::TooltipPresentationCamera;
use super::materialization::TooltipReadiness;
use super::materialization::TooltipWidthConstraint;
use super::materialization::TooltipWidthConstraintRequest;
use super::placement::TooltipPlacementState;
use super::target::MeshAnchorTarget;
use super::target::TooltipTarget;
#[cfg(test)]
use super::target::TooltipTargetEntity;
use super::target::TooltipTargetSpace;
use super::visibility::TooltipShownPending;
#[cfg(test)]
use crate::layout::Anchor;
#[cfg(test)]
use crate::layout::LayoutTree;
#[cfg(test)]
use crate::layout::Px;
#[cfg(test)]
use crate::layout::Text;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
#[cfg(test)]
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelOwned;
use crate::panel::PanelSpace;
#[cfg(test)]
use crate::panel::World;
use crate::screen_space::ScreenAnchorTarget;
use crate::widgets::PanelWidget;
use crate::widgets::PanelWidgetReader;
use crate::widgets::WidgetOf;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component, PartialEq, Debug, FromWorld, Clone)]
#[relationship(relationship_target = Tooltips)]
/// Relationship source on each tooltip controller, pointing at its target.
pub struct TooltipFor(#[entities] Entity);

impl TooltipFor {
    pub(crate) const fn new(target: Entity) -> Self { Self(target) }

    /// Returns the entity described by this tooltip.
    #[must_use]
    pub const fn target(&self) -> Entity { self.0 }
}

impl FromWorld for TooltipFor {
    fn from_world(_: &mut BevyWorld) -> Self { Self(Entity::PLACEHOLDER) }
}

/// Reverse membership for tooltip controllers describing one target.
#[derive(Component, Default, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component, FromWorld, Default)]
#[relationship_target(relationship = TooltipFor, linked_spawn)]
pub struct Tooltips(Vec<Entity>);

impl Tooltips {
    /// Iterates over the target's tooltip controller entities.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ { self.0.iter().copied() }
}

impl Deref for Tooltips {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target { &self.0 }
}

/// Extension methods for checked standalone tooltip authoring.
pub trait TooltipCommandsExt {
    /// Reserves and returns a tooltip controller entity.
    ///
    /// A target is a checked handle that carries its coordinate space —
    /// Hana's typed panel and widget handles, a general
    /// [`TooltipTargetEntity`](crate::TooltipTargetEntity), or an application's own [`TooltipTarget`]:
    ///
    /// ```
    /// use bevy::ecs::system::Commands;
    /// use bevy::prelude::Entity;
    /// use hana_diegetic::El;
    /// use hana_diegetic::PanelEntity;
    /// use hana_diegetic::Tooltip;
    /// use hana_diegetic::TooltipCommandsExt;
    /// use hana_diegetic::TooltipTarget;
    /// use hana_diegetic::TooltipTargetEntity;
    /// use hana_diegetic::WidgetEntity;
    /// use hana_diegetic::World;
    ///
    /// struct ApplicationWorldTarget(Entity);
    ///
    /// impl TooltipTarget for ApplicationWorldTarget {
    ///     type Space = World;
    ///
    ///     fn tooltip_target_entity(&self) -> Entity { self.0 }
    /// }
    ///
    /// fn typed_targets(
    ///     commands: &mut Commands,
    ///     panel: PanelEntity<World>,
    ///     widget: WidgetEntity<World>,
    ///     general: TooltipTargetEntity<World>,
    ///     application: ApplicationWorldTarget,
    /// ) {
    ///     commands.spawn_tooltip(panel, Tooltip::new(El::new()));
    ///     commands.spawn_tooltip(widget, Tooltip::new(El::new()));
    ///     commands.spawn_tooltip(general, Tooltip::new(El::new()));
    ///     commands.spawn_tooltip(application, Tooltip::new(El::new()));
    /// }
    /// ```
    ///
    /// A bare [`Entity`] says nothing about what it is or where it is placed,
    /// so it is not one. The typed targets above are what keep this case
    /// meaningful — a rename would break those loudly rather than leaving this
    /// one succeeding for an unrelated reason:
    ///
    /// ```compile_fail,E0277
    /// use bevy::ecs::system::Commands;
    /// use bevy::prelude::Entity;
    /// use hana_diegetic::{El, Tooltip, TooltipCommandsExt};
    ///
    /// fn raw_entity_target(commands: &mut Commands, target: Entity) {
    ///     commands.spawn_tooltip(target, Tooltip::new(El::new()));
    /// }
    /// ```
    fn spawn_tooltip<Target>(&mut self, target: Target, tooltip: Tooltip) -> Entity
    where
        Target: TooltipTarget;
}

impl TooltipCommandsExt for Commands<'_, '_> {
    fn spawn_tooltip<Target>(&mut self, target: Target, tooltip: Tooltip) -> Entity
    where
        Target: TooltipTarget,
    {
        let controller = self.spawn_empty().id();
        let operation = StandaloneTooltipOperation {
            controller,
            target: target.tooltip_target_entity(),
            space: Target::Space::PANEL_SPACE,
            provenance: target.tooltip_target_panel().map_or_else(
                || {
                    target
                        .tooltip_target_widget_owner()
                        .map_or(StandaloneTargetProvenance::General, |owner| {
                            StandaloneTargetProvenance::Widget { owner }
                        })
                },
                |_| StandaloneTargetProvenance::Panel,
            ),
            tooltip,
        };
        self.run_system_cached_with(apply_standalone_tooltip, operation);
        controller
    }
}

#[derive(Clone, Copy)]
enum StandaloneTargetProvenance {
    General,
    Panel,
    Widget { owner: Entity },
}

#[derive(Clone)]
struct StandaloneTooltipOperation {
    controller: Entity,
    target:     Entity,
    space:      PanelSpace,
    provenance: StandaloneTargetProvenance,
    tooltip:    Tooltip,
}

fn apply_standalone_tooltip(
    In(operation): In<StandaloneTooltipOperation>,
    targets: Query<(
        Option<&DiegeticPanel>,
        Option<&PanelWidget>,
        Option<&WidgetOf>,
        Option<&ResolvedAnchorGeometry>,
        Option<&MeshAnchorTarget>,
        Option<&ScreenAnchorTarget>,
    )>,
    mut commands: Commands,
) {
    let Ok((panel, widget, widget_of, geometry, mesh_target, screen_target)) =
        targets.get(operation.target)
    else {
        commands.entity(operation.controller).despawn();
        return;
    };
    let owner = match operation.provenance {
        StandaloneTargetProvenance::General => None,
        StandaloneTargetProvenance::Panel => panel
            .filter(|panel| PanelSpace::from(panel.coordinate_space()) == operation.space)
            .map(|_| operation.target),
        StandaloneTargetProvenance::Widget { owner } => widget
            .zip(widget_of)
            .filter(|(_, widget_of)| widget_of.panel() == owner)
            .and_then(|_| {
                targets
                    .get(owner)
                    .ok()
                    .and_then(|(panel, ..)| panel)
                    .filter(|panel| PanelSpace::from(panel.coordinate_space()) == operation.space)
                    .map(|_| owner)
            }),
    };
    let valid_target = match operation.provenance {
        StandaloneTargetProvenance::General => match operation.space {
            PanelSpace::World => geometry.is_some() || mesh_target.is_some(),
            PanelSpace::Screen => screen_target.is_some(),
        },
        StandaloneTargetProvenance::Panel | StandaloneTargetProvenance::Widget { .. } => {
            owner.is_some()
        },
    };
    if !valid_target {
        commands.entity(operation.controller).despawn();
        return;
    }

    let mut controller = commands.entity(operation.controller);
    controller.insert((
        operation.tooltip,
        TooltipFor::new(operation.target),
        AuthoredTooltipTargetSpace(operation.space),
    ));
    if let Some(owner) = owner {
        controller.insert(crate::panel::PanelOwned::from(owner));
    }
}

pub(crate) fn remove_materialized_state(
    entity: &mut EntityCommands<'_>,
    materialized: &MaterializedTooltip,
) {
    let previous_transform = materialized.previous_transform;
    let previous_global = materialized.previous_global;
    let previous_visibility = materialized.previous_visibility;
    entity.remove::<(
        MaterializedTooltip,
        TooltipReadiness,
        TooltipPlacementState,
        TooltipWidthConstraint,
        TooltipWidthConstraintRequest,
        TooltipPresentationCamera,
        TooltipShownPending,
        ScreenTooltipAttachmentCorrection,
        PrepareTooltip,
    )>();
    entity.remove::<(Transform, GlobalTransform, Visibility)>();
    if let PreMaterializationComponent::Present(transform) = previous_transform {
        entity.insert(transform);
    }
    if let PreMaterializationComponent::Present(global_transform) = previous_global {
        entity.insert(global_transform);
    }
    if let PreMaterializationComponent::Present(visibility) = previous_visibility {
        entity.insert(visibility);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ComputedTooltipRecord {
    widget_id: crate::PanelElementId,
    tooltip:   Tooltip,
}

impl ComputedTooltipRecord {
    pub(crate) const fn new(widget_id: crate::PanelElementId, tooltip: Tooltip) -> Self {
        Self { widget_id, tooltip }
    }

    pub(crate) const fn widget_id(&self) -> &crate::PanelElementId { &self.widget_id }

    pub(crate) const fn tooltip(&self) -> &Tooltip { &self.tooltip }
}

#[derive(Component, Default)]
pub(crate) struct TooltipControllerIndex(HashMap<crate::PanelElementId, Entity>);

impl TooltipControllerIndex {
    pub(crate) fn entity(&self, id: &crate::PanelElementId) -> Option<Entity> {
        self.0.get(id).copied()
    }

    pub(crate) fn entities(&self) -> impl Iterator<Item = Entity> + '_ { self.0.values().copied() }

    pub(crate) fn replace(&mut self, index: HashMap<crate::PanelElementId, Entity>) {
        self.0 = index;
    }
}

/// Reuses equal associated tooltip controllers and replaces changed declarations.
pub(in crate::widgets) fn reify_tooltip_controllers(
    mut panels: Query<
        (Entity, &ComputedDiegeticPanel, &mut TooltipControllerIndex),
        Changed<ComputedDiegeticPanel>,
    >,
    widget_reader: PanelWidgetReader,
    controllers: Query<(&Tooltip, &TooltipFor, Option<&PanelOwned>)>,
    live_entities: Query<()>,
    mut commands: Commands,
) {
    for (panel, computed, mut controller_index) in &mut panels {
        let mut next_index = HashMap::with_capacity(computed.tooltip_records().len());
        for record in computed.tooltip_records() {
            let Some(widget) = widget_reader.entity(panel, record.widget_id()) else {
                continue;
            };
            let reusable = controller_index
                .entity(record.widget_id())
                .filter(|entity| {
                    controllers
                        .get(*entity)
                        .is_ok_and(|(tooltip, _, _)| tooltip == record.tooltip())
                });
            let controller = reusable.unwrap_or_else(|| {
                commands
                    .spawn((
                        record.tooltip().clone(),
                        TooltipFor::new(widget),
                        PanelOwned::from(panel),
                    ))
                    .id()
            });
            if let Ok((tooltip, tooltip_for, ownership)) = controllers.get(controller) {
                debug_assert_eq!(tooltip, record.tooltip());
                let mut controller_commands = commands.entity(controller);
                if tooltip_for.target() != widget {
                    controller_commands.insert(TooltipFor::new(widget));
                }
                if ownership.is_none_or(|ownership| ownership.owner() != panel) {
                    controller_commands.insert(PanelOwned::from(panel));
                }
            }
            next_index.insert(record.widget_id().clone(), controller);
        }

        let retained = next_index.values().copied().collect::<Vec<_>>();
        for controller in controller_index.entities() {
            if !retained.contains(&controller) && live_entities.contains(controller) {
                commands.entity(controller).despawn();
            }
        }
        controller_index.replace(next_index);
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::visibility::RenderLayers;
    use bevy::ecs::error;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::Despawn;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::DetectChangesMut;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Vec3A;
    use bevy::transform::TransformPlugin;
    use hana_valence::AnchoredTo;

    use super::*;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Slider;
    use crate::layout::El;
    use crate::layout::LayoutTreeChange;
    use crate::layout::Unit;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetSpec;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::declaration::TooltipDisabledPolicy;
    use crate::widgets::tooltip::materialization::TooltipPresentationSpace;
    use crate::widgets::tooltip::target;
    use crate::widgets::tooltip::target::MeshFace;

    const MATERIALIZATION_UPDATES: usize = 5;

    #[test]
    fn removing_materialized_state_restores_only_present_components() {
        let mut app = App::new();
        let target = app.world_mut().spawn_empty().id();
        let tooltip = Tooltip::new(El::new());
        let absent = MaterializedTooltip {
            target,
            presentation: TooltipPresentationSpace::World,
            layout_unit: Unit::Millimeters,
            render_layers: RenderLayers::layer(0),
            authored_attachment: tooltip.attachment(),
            previous_transform: PreMaterializationComponent::Absent,
            previous_global: PreMaterializationComponent::Absent,
            previous_visibility: PreMaterializationComponent::Absent,
        };
        let transform = Transform::from_xyz(4.0, -2.0, 1.5);
        let global_transform = GlobalTransform::from(transform);
        let mut present = absent.clone();
        present.previous_transform = PreMaterializationComponent::Present(transform);
        present.previous_global = PreMaterializationComponent::Present(global_transform);
        present.previous_visibility = PreMaterializationComponent::Present(Visibility::Visible);
        let absent_controller = app
            .world_mut()
            .spawn((
                absent.clone(),
                Transform::default(),
                GlobalTransform::default(),
                Visibility::Hidden,
            ))
            .id();
        let present_controller = app
            .world_mut()
            .spawn((
                present.clone(),
                Transform::default(),
                GlobalTransform::default(),
                Visibility::Hidden,
            ))
            .id();

        let mut commands = app.world_mut().commands();
        remove_materialized_state(&mut commands.entity(absent_controller), &absent);
        remove_materialized_state(&mut commands.entity(present_controller), &present);
        app.world_mut().flush();

        assert!(app.world().get::<Transform>(absent_controller).is_none());
        assert!(
            app.world()
                .get::<GlobalTransform>(absent_controller)
                .is_none()
        );
        assert!(app.world().get::<Visibility>(absent_controller).is_none());
        assert_eq!(
            app.world().get::<Transform>(present_controller),
            Some(&transform)
        );
        assert_eq!(
            app.world().get::<GlobalTransform>(present_controller),
            Some(&global_transform)
        );
        assert_eq!(
            app.world().get::<Visibility>(present_controller),
            Some(&Visibility::Visible)
        );
    }

    #[derive(Default, Resource)]
    struct TooltipCleanupCount(usize);

    fn count_tooltip_cleanup(
        _despawn: On<Despawn<Tooltip>>,
        mut cleanup_count: ResMut<TooltipCleanupCount>,
    ) {
        cleanup_count.0 += 1;
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn materialization_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app
    }

    fn update_materialization(app: &mut App) {
        for _ in 0..MATERIALIZATION_UPDATES {
            app.update();
        }
    }

    fn prepare_tooltip(app: &mut App, controller: Entity) {
        app.world_mut()
            .entity_mut(controller)
            .insert(PrepareTooltip);
    }

    fn tooltip_tree(tooltip: Option<Tooltip>) -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        let button = El::new().button("action");
        match tooltip {
            Some(tooltip) => {
                builder.with(button.tooltip(tooltip), |_| {});
            },
            None => {
                builder.with(button, |_| {});
            },
        }
        builder.build()
    }

    fn slider_tooltip_tree(slider: Slider, tooltip: Option<Tooltip>) -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        let slider = El::new().widget("action", slider);
        match tooltip {
            Some(tooltip) => builder.with(slider.tooltip(tooltip), |_| {}),
            None => builder.with(slider, |_| {}),
        };
        builder.build()
    }

    fn editable_tooltip_tree(tooltip: Tooltip) -> crate::LayoutTree {
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .editable_field("action", field, crate::TextStyle::new(10.0))
                .tooltip(tooltip),
            |_| {},
        );
        builder.build()
    }

    fn spawn_panel(app: &mut App, tree: crate::LayoutTree) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(result.is_ok());
        let panel = result.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        assert_ne!(panel, Entity::PLACEHOLDER);
        panel
    }

    fn widget(app: &mut App, panel: Entity) -> Entity {
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("action"))
            })
            .ok()
            .flatten()
            .unwrap_or(Entity::PLACEHOLDER);
        assert_ne!(widget, Entity::PLACEHOLDER);
        widget
    }

    fn controller(app: &App, widget: Entity) -> Entity {
        let controller = app
            .world()
            .get::<crate::widgets::Tooltips>(widget)
            .and_then(|tooltips| tooltips.iter().next())
            .unwrap_or(Entity::PLACEHOLDER);
        assert_ne!(controller, Entity::PLACEHOLDER);
        controller
    }

    #[track_caller]
    fn assert_lightweight_controller(app: &App, widget: Entity, controller: Entity) {
        assert_eq!(
            app.world()
                .get::<crate::widgets::Tooltips>(widget)
                .map(|tooltips| tooltips.iter().collect::<Vec<_>>()),
            Some(vec![controller])
        );
        assert!(app.world().get::<AnchoredHere>(widget).is_none());
        assert!(
            app.world()
                .get::<crate::widgets::ScreenWidgetAnchoredHere>(widget)
                .is_none()
        );
        assert!(app.world().get::<DiegeticPanel>(controller).is_none());
        assert!(
            app.world()
                .get::<crate::ComputedDiegeticPanel>(controller)
                .is_none()
        );
        assert!(
            app.world()
                .get::<crate::PanelTextRuns>(controller)
                .is_none()
        );
        assert!(
            app.world()
                .get::<crate::panel::PanelAttachmentAuthored>(controller)
                .is_none()
        );
        assert!(app.world().get::<AnchoredTo>(controller).is_none());
        assert!(
            app.world()
                .get::<crate::widgets::ScreenWidgetAnchoredTo>(controller)
                .is_none()
        );
        assert!(app.world().get::<Aabb>(controller).is_none());
        assert!(
            app.world()
                .get::<ResolvedAnchorGeometry>(controller)
                .is_none()
        );
        assert_eq!(
            app.world()
                .get::<TooltipFor>(controller)
                .map(TooltipFor::target),
            Some(widget)
        );
    }

    fn replace_associated_tooltip(
        app: &mut App,
        panel: Entity,
        widget: Entity,
        previous_controller: Entity,
        baseline: &LayoutTree,
        replacement: Tooltip,
    ) -> Entity {
        let replacement_tree = tooltip_tree(Some(replacement.clone()));
        assert_eq!(
            baseline.classify_change(&replacement_tree),
            LayoutTreeChange::VisualOnly
        );
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, replacement_tree)
                .is_ok()
        );
        app.update();
        let replacement_controller = controller(app, widget);
        assert_ne!(replacement_controller, previous_controller);
        assert!(app.world().get_entity(previous_controller).is_err());
        assert_eq!(
            app.world().get::<Tooltip>(replacement_controller),
            Some(&replacement)
        );
        replacement_controller
    }

    #[test]
    fn editable_field_tooltip_reifies_against_its_widget_entity() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, editable_tooltip_tree(Tooltip::new(El::new())));
        app.update();
        let field = widget(&mut app, panel);
        let tooltip = controller(&app, field);

        assert_eq!(
            app.world()
                .get::<TooltipFor>(tooltip)
                .map(TooltipFor::target),
            Some(field),
        );
    }

    #[test]
    fn associated_controller_replaces_identity_for_changed_declarations() {
        let mut app = test_app();
        let tooltip = Tooltip::new(El::new());
        let tree = tooltip_tree(Some(tooltip.clone()));
        let tree_without_tooltip = tooltip_tree(None);
        assert_eq!(
            tree_without_tooltip.classify_change(&tree),
            LayoutTreeChange::VisualOnly
        );
        let panel = spawn_panel(&mut app, tree_without_tooltip);
        app.update();
        let widget = widget(&mut app, panel);
        let button_declaration = app.world().get::<WidgetSpec>(widget).cloned();
        assert!(button_declaration.is_some());
        assert!(
            app.world()
                .get::<crate::widgets::Tooltips>(widget)
                .is_none()
        );

        let attached = app.world_mut().commands().set_tree(panel, tree.clone());
        assert!(attached.is_ok());
        app.update();
        assert_eq!(self::widget(&mut app, panel), widget);
        assert_eq!(
            app.world().get::<crate::widgets::WidgetSpec>(widget),
            button_declaration.as_ref()
        );
        let original = controller(&app, widget);
        assert_lightweight_controller(&app, widget, original);

        let indexed_controller = app
            .world()
            .get::<TooltipControllerIndex>(panel)
            .and_then(|index| index.entity(&PanelElementId::named("action")));
        assert_eq!(indexed_controller, Some(original));
        let computed_tick = app
            .world()
            .entity(panel)
            .get_ref::<crate::ComputedDiegeticPanel>()
            .map(|computed| computed.last_changed());
        assert!(computed_tick.is_some());

        let identical = app.world_mut().commands().set_tree(panel, tree.clone());
        assert!(identical.is_ok());
        app.update();
        assert_eq!(controller(&app, widget), original);
        assert_eq!(
            app.world()
                .get::<TooltipControllerIndex>(panel)
                .and_then(|index| index.entity(&PanelElementId::named("action"))),
            indexed_controller
        );
        assert_eq!(
            app.world()
                .entity(panel)
                .get_ref::<crate::ComputedDiegeticPanel>()
                .map(|computed| computed.last_changed()),
            computed_tick
        );

        let replacements = [
            tooltip.clone().show_after(Duration::from_secs(1)),
            tooltip.clone().hide_after(Duration::from_secs(1)),
            tooltip.clone().disabled_policy(TooltipDisabledPolicy::Show),
            tooltip.clone().source_anchor(Anchor::BottomLeft),
            tooltip.clone().target_anchor(Anchor::TopRight),
            tooltip.offset(PanelAnchorOffset::new(Px(3.0), Px(4.0))),
        ];
        let mut previous_controller = original;
        for replacement in replacements {
            previous_controller = replace_associated_tooltip(
                &mut app,
                panel,
                widget,
                previous_controller,
                &tree,
                replacement,
            );
        }
        assert_eq!(
            app.world().get::<crate::widgets::WidgetSpec>(widget),
            button_declaration.as_ref()
        );

        let mut blueprint = Tooltip::new(El::column());
        blueprint.text("replacement");
        let replaced = app
            .world_mut()
            .commands()
            .set_tree(panel, tooltip_tree(Some(blueprint.clone())));
        assert!(replaced.is_ok());
        app.update();
        let blueprint_controller = controller(&app, widget);
        assert_ne!(blueprint_controller, previous_controller);
        assert!(app.world().get_entity(previous_controller).is_err());
        assert_eq!(
            app.world().get::<Tooltip>(blueprint_controller),
            Some(&blueprint)
        );
    }

    #[test]
    fn attaching_a_tooltip_preserves_the_slider_declaration_and_identity() {
        let mut app = test_app();
        let slider = Slider::new(0.0..=1.0).value(0.25);
        let tree_without_tooltip = slider_tooltip_tree(slider.clone(), None);
        let tree = slider_tooltip_tree(slider.clone(), Some(Tooltip::new(El::new())));
        let declaration = WidgetSpec::Slider(slider);

        let panel = spawn_panel(&mut app, tree_without_tooltip);
        app.update();
        let widget = widget(&mut app, panel);
        assert_eq!(
            app.world().get::<crate::widgets::WidgetSpec>(widget),
            Some(&declaration)
        );
        assert!(
            app.world()
                .get::<crate::widgets::Tooltips>(widget)
                .is_none()
        );

        assert!(app.world_mut().commands().set_tree(panel, tree).is_ok());
        app.update();

        assert_eq!(self::widget(&mut app, panel), widget);
        assert_eq!(
            app.world().get::<crate::widgets::WidgetSpec>(widget),
            Some(&declaration)
        );
        assert_eq!(
            app.world()
                .get::<TooltipFor>(controller(&app, widget))
                .map(TooltipFor::target),
            Some(widget)
        );
    }

    #[test]
    fn tree_replacement_linked_cleanup_precedes_index_retirement_once() {
        let mut app = test_app();
        app.set_error_handler(error::panic)
            .init_resource::<TooltipCleanupCount>()
            .add_observer(count_tooltip_cleanup);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        app.update();
        let widget = widget(&mut app, panel);
        let controller = controller(&app, widget);

        let replacement = LayoutBuilder::new(100.0, 50.0).build();
        let result = app.world_mut().commands().set_tree(panel, replacement);
        assert!(result.is_ok());
        app.update();

        assert!(app.world().get_entity(widget).is_err());
        assert!(app.world().get_entity(controller).is_err());
        assert_eq!(
            app.world()
                .get::<TooltipControllerIndex>(panel)
                .and_then(|index| index.entity(&PanelElementId::named("action"))),
            None
        );
        assert_eq!(app.world().resource::<TooltipCleanupCount>().0, 1);
    }

    #[test]
    fn retirement_despawns_a_live_malformed_indexed_controller() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, tooltip_tree(None));
        app.update();
        let malformed = app.world_mut().spawn_empty().id();
        let mut replacement = HashMap::new();
        replacement.insert(PanelElementId::named("action"), malformed);
        let index = app.world_mut().get_mut::<TooltipControllerIndex>(panel);
        assert!(index.is_some());
        if let Some(mut index) = index {
            index.replace(replacement);
        }

        let computed = app
            .world_mut()
            .get_mut::<crate::ComputedDiegeticPanel>(panel);
        assert!(computed.is_some());
        if let Some(mut computed) = computed {
            computed.set_changed();
        }
        app.update();

        assert!(app.world().get_entity(malformed).is_err());
    }

    #[test]
    fn ordinary_panel_reifies_widget_layout_text() {
        let mut app = test_app();
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.text(
            Text::new("action", crate::TextStyle::default()).layout(El::new().button("action")),
        );
        let panel = spawn_panel(&mut app, builder.build());

        app.update();

        let widget = widget(&mut app, panel);
        assert!(
            app.world()
                .get::<crate::widgets::PanelWidget>(widget)
                .is_some()
        );
    }

    #[test]
    fn missing_standalone_target_removes_reserved_controller() {
        let mut app = test_app();
        let target = app.world_mut().spawn_empty().id();
        let handle = TooltipTargetEntity::<World>::from_validated(target);
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.world_mut().despawn(target);
        app.update();

        assert!(app.world().get_entity(controller).is_err());
    }

    #[test]
    fn linked_target_despawn_removes_standalone_controller() {
        let mut app = test_app();
        let geometry = target::mesh_face_geometry(
            Aabb {
                center:       Vec3A::ZERO,
                half_extents: Vec3A::ONE,
            },
            MeshFace::PositiveZ,
        );
        assert!(geometry.is_some());
        let target = geometry.map_or(Entity::PLACEHOLDER, |geometry| {
            app.world_mut().spawn(geometry).id()
        });
        assert_ne!(target, Entity::PLACEHOLDER);
        let handle = TooltipTargetEntity::<World>::from_validated(target);
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.update();
        assert!(app.world().get::<Tooltip>(controller).is_some());

        app.world_mut().despawn(target);

        assert!(app.world().get_entity(controller).is_err());
    }

    #[test]
    fn panel_role_removal_cleans_associated_and_standalone_controllers() {
        let mut app = materialization_app();
        app.init_resource::<TooltipCleanupCount>()
            .add_observer(count_tooltip_cleanup);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        let panel_handle = crate::PanelEntity::<World>::from_validated(panel, PanelSpace::World);
        let standalone = app
            .world_mut()
            .commands()
            .spawn_tooltip(panel_handle, Tooltip::new(El::new()));
        app.update();
        let widget = widget(&mut app, panel);
        let associated = controller(&app, widget);
        assert!(app.world().get::<Tooltip>(standalone).is_some());
        prepare_tooltip(&mut app, associated);
        prepare_tooltip(&mut app, standalone);
        update_materialization(&mut app);
        assert!(app.world().get::<MaterializedTooltip>(associated).is_some());
        assert!(app.world().get::<MaterializedTooltip>(standalone).is_some());

        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
        app.update();

        assert!(app.world().get_entity(panel).is_ok());
        assert!(app.world().get_entity(associated).is_err());
        assert!(app.world().get_entity(standalone).is_err());
        assert_eq!(app.world().resource::<TooltipCleanupCount>().0, 2);
    }

    #[test]
    fn panel_despawn_cleans_associated_and_standalone_controllers() {
        let mut app = materialization_app();
        app.init_resource::<TooltipCleanupCount>()
            .add_observer(count_tooltip_cleanup);
        let panel = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        let panel_handle = crate::PanelEntity::<World>::from_validated(panel, PanelSpace::World);
        let standalone = app
            .world_mut()
            .commands()
            .spawn_tooltip(panel_handle, Tooltip::new(El::new()));
        app.update();
        let widget = widget(&mut app, panel);
        let associated = controller(&app, widget);
        assert!(app.world().get::<Tooltip>(standalone).is_some());
        prepare_tooltip(&mut app, associated);
        prepare_tooltip(&mut app, standalone);
        update_materialization(&mut app);
        assert!(app.world().get::<MaterializedTooltip>(associated).is_some());
        assert!(app.world().get::<MaterializedTooltip>(standalone).is_some());

        app.world_mut().despawn(panel);
        app.update();

        assert!(app.world().get_entity(associated).is_err());
        assert!(app.world().get_entity(standalone).is_err());
        assert_eq!(app.world().resource::<TooltipCleanupCount>().0, 2);
    }
}
