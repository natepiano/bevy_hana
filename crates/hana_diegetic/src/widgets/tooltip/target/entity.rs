use std::marker::PhantomData;

#[cfg(test)]
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
#[cfg(test)]
use bevy::prelude::Vec3;
#[cfg(test)]
use hana_valence::AnchorFrame;
#[cfg(test)]
use hana_valence::AnchorSite;
#[cfg(test)]
use hana_valence::Edge;
#[cfg(test)]
use hana_valence::Orientation;
#[cfg(test)]
use hana_valence::Position;
#[cfg(test)]
use hana_valence::ResolvedAnchorGeometry;

use crate::panel::PanelEntity;
use crate::panel::PanelSpace;
use crate::panel::Screen;
use crate::panel::WidgetEntity;
use crate::panel::World;
use crate::screen_space::ScreenAnchorTarget;
#[cfg(test)]
use crate::widgets::tooltip::Tooltip;

/// A typed identity for a general tooltip target.
///
/// The coordinate space travels in the type, so an API that places in screen
/// space accepts only screen-space targets:
///
/// ```
/// use hana_diegetic::Screen;
/// use hana_diegetic::TooltipTarget;
/// use hana_diegetic::TooltipTargetEntity;
///
/// fn requires_screen_target<Target: TooltipTarget<Space = Screen>>(_target: Target) {}
///
/// fn accept_screen_target(target: TooltipTargetEntity<Screen>) { requires_screen_target(target); }
/// ```
///
/// A world-space target is refused there. The acceptance above is what keeps
/// this case meaningful — a rename would break that one loudly rather than
/// leaving this one succeeding for an unrelated reason:
///
/// ```compile_fail,E0271
/// use hana_diegetic::{Screen, TooltipTarget, TooltipTargetEntity, World};
///
/// fn requires_screen_target<Target: TooltipTarget<Space = Screen>>(_target: Target) {}
///
/// fn reject_world_target(target: TooltipTargetEntity<World>) {
///     requires_screen_target(target);
/// }
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TooltipTargetEntity<Space> {
    entity: Entity,
    marker: PhantomData<fn() -> Space>,
}

impl<Space> TooltipTargetEntity<Space> {
    /// Returns the wrapped Bevy entity, for ECS work outside the tooltip API.
    #[must_use]
    pub const fn entity(&self) -> Entity { self.entity }

    pub(in crate::widgets::tooltip) const fn from_validated(entity: Entity) -> Self {
        Self {
            entity,
            marker: PhantomData,
        }
    }
}

/// Sealed coordinate-space marker accepted by [`TooltipTarget`].
pub trait TooltipTargetSpace: private::SealedSpace {
    #[doc(hidden)]
    const PANEL_SPACE: PanelSpace;
}

impl TooltipTargetSpace for World {
    const PANEL_SPACE: PanelSpace = PanelSpace::World;
}

impl TooltipTargetSpace for Screen {
    const PANEL_SPACE: PanelSpace = PanelSpace::Screen;
}

/// Typed entity handle that can be described by a tooltip.
pub trait TooltipTarget {
    /// Coordinate space supplied by the target's live placement data.
    type Space: TooltipTargetSpace;

    /// Returns the target entity.
    fn tooltip_target_entity(&self) -> Entity;

    /// Returns the captured panel entity for Hana's typed panel handle.
    #[doc(hidden)]
    fn tooltip_target_panel(&self) -> Option<Entity> { None }

    /// Returns the captured panel owner for Hana's typed widget handle.
    #[doc(hidden)]
    fn tooltip_target_widget_owner(&self) -> Option<Entity> { None }
}

impl<Space: TooltipTargetSpace> TooltipTarget for TooltipTargetEntity<Space> {
    type Space = Space;

    fn tooltip_target_entity(&self) -> Entity { self.entity }
}

impl<Space: TooltipTargetSpace> TooltipTarget for PanelEntity<Space> {
    type Space = Space;

    fn tooltip_target_entity(&self) -> Entity { self.entity() }

    fn tooltip_target_panel(&self) -> Option<Entity> { Some(self.entity()) }
}

impl<Space: TooltipTargetSpace> TooltipTarget for WidgetEntity<Space> {
    type Space = Space;

    fn tooltip_target_entity(&self) -> Entity { self.entity() }

    fn tooltip_target_widget_owner(&self) -> Option<Entity> { Some(self.owner()) }
}
/// Extension methods for checked general screen anchor authoring.
pub trait ScreenAnchorCommandsExt {
    /// Publishes screen placement data and returns a typed target handle.
    fn screen_anchor_target(
        &mut self,
        entity: Entity,
        data: ScreenAnchorTarget,
    ) -> TooltipTargetEntity<Screen>;
}

impl ScreenAnchorCommandsExt for Commands<'_, '_> {
    fn screen_anchor_target(
        &mut self,
        entity: Entity,
        data: ScreenAnchorTarget,
    ) -> TooltipTargetEntity<Screen> {
        self.queue(move |world: &mut bevy::ecs::world::World| {
            if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                entity_mut.insert(data);
            }
        });
        TooltipTargetEntity::from_validated(entity)
    }
}
mod private {
    use crate::panel::Screen;
    pub trait SealedSpace {}

    impl SealedSpace for crate::panel::World {}

    impl SealedSpace for Screen {}
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::MinimalPlugins;

    use super::*;
    use crate::DiegeticPanel;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::WidgetOf;
    use crate::layout::El;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::TooltipCommandsExt;

    struct ApplicationWorldTarget(Entity);

    impl TooltipTarget for ApplicationWorldTarget {
        type Space = World;

        fn tooltip_target_entity(&self) -> Entity { self.0 }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
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

    fn center_anchor_geometry() -> ResolvedAnchorGeometry {
        let frame = AnchorFrame::try_new(Position::from(Vec3::ZERO), Orientation::default());
        assert!(
            frame.is_ok(),
            "the center anchor uses finite identity geometry"
        );
        let geometry = ResolvedAnchorGeometry::try_new(
            [(AnchorSite::Center, frame.unwrap_or_default())],
            Vec::<Edge>::new(),
        );
        assert!(
            geometry.is_ok(),
            "a single center anchor has no invalid edges"
        );
        geometry.unwrap_or_default()
    }

    #[test]
    fn stale_panel_handle_does_not_fall_back_to_general_geometry() {
        let mut app = test_app();
        let target = spawn_panel(&mut app, tooltip_tree(None));
        let handle = PanelEntity::<World>::from_validated(target, PanelSpace::World);
        app.world_mut().entity_mut(target).remove::<DiegeticPanel>();
        app.world_mut()
            .entity_mut(target)
            .insert(center_anchor_geometry());
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.update();

        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_some());
        assert!(app.world().get_entity(controller).is_err());
    }

    #[test]
    fn stale_widget_owner_does_not_fall_back_to_general_geometry() {
        let mut app = test_app();
        let owner = spawn_panel(&mut app, tooltip_tree(Some(Tooltip::new(El::new()))));
        let replacement_owner = spawn_panel(&mut app, tooltip_tree(None));
        app.update();
        let target = widget(&mut app, owner);
        let handle = WidgetEntity::<World>::from_validated(target, owner, PanelSpace::World);
        app.world_mut()
            .entity_mut(target)
            .insert((WidgetOf::new(replacement_owner), center_anchor_geometry()));
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.update();

        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_some());
        assert!(app.world().get_entity(controller).is_err());
    }

    #[test]
    fn general_target_implementations_do_not_invent_panel_ownership() {
        let mut app = test_app();
        let target = spawn_panel(&mut app, tooltip_tree(None));
        app.world_mut()
            .entity_mut(target)
            .insert(center_anchor_geometry());
        let general_handle = TooltipTargetEntity::<World>::from_validated(target);
        let general_controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(general_handle, Tooltip::new(El::new()));
        let application_controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(ApplicationWorldTarget(target), Tooltip::new(El::new()));
        app.update();

        assert!(app.world().get::<Tooltip>(general_controller).is_some());
        assert!(
            app.world()
                .get::<crate::panel::PanelOwned>(general_controller)
                .is_none()
        );
        assert!(app.world().get::<Tooltip>(application_controller).is_some());
        assert!(
            app.world()
                .get::<crate::panel::PanelOwned>(application_controller)
                .is_none()
        );
    }
}
