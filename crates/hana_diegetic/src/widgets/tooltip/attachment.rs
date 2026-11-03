use bevy::prelude::*;
use hana_valence::ResolvedAnchorGeometry;

use super::controller::TooltipFor;
use super::materialization::MaterializedTooltip;
use super::materialization::TooltipPresentationSpace;
use super::placement::TooltipPlacementState;
use super::target::MeshAnchorTarget;
use crate::layout::Unit;
use crate::panel::DiegeticPanel;
use crate::panel::DiegeticPanelCommands;
use crate::panel::PanelAttachment;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::PanelEntityReader;
use crate::panel::PanelSpace;
use crate::screen_space::ScreenAnchorTarget;
use crate::widgets::PanelWidget;
use crate::widgets::PanelWidgetReader;
use crate::widgets::WidgetOf;

#[derive(Component)]
pub(super) struct ScreenTooltipAttachmentCorrection;

// Screen panel text carries a 64-unit sort bias. Clear it so a tooltip can
// paint above another panel attached to the same hovered widget.
const SCREEN_TOOLTIP_ATTACHED_PANEL_CLEARANCE: f32 = 65.0;

pub(super) fn screen_tooltip_attachment(
    attachment: PanelAttachment,
    layout_unit: Unit,
) -> PanelAttachment {
    let offset = attachment.offset();
    let authored_depth = offset.to_layout_units(layout_unit).z;
    attachment.with_offset(offset.with_z(authored_depth + SCREEN_TOOLTIP_ATTACHED_PANEL_CLEARANCE))
}

pub(super) fn screen_tooltip_attachment_correction_requested(
    corrections: Query<(), With<ScreenTooltipAttachmentCorrection>>,
) -> bool {
    !corrections.is_empty()
}

pub(super) fn clear_screen_tooltip_attachment_corrections(
    corrections: Query<Entity, With<ScreenTooltipAttachmentCorrection>>,
    mut commands: Commands,
) {
    for entity in &corrections {
        commands
            .entity(entity)
            .remove::<ScreenTooltipAttachmentCorrection>();
    }
}

pub(super) fn attach_materialized_tooltips(
    controllers: Query<
        (
            Entity,
            &TooltipFor,
            &MaterializedTooltip,
            &TooltipPlacementState,
        ),
        Without<PanelAttachmentAuthored>,
    >,
    panel_reader: PanelEntityReader,
    widget_reader: PanelWidgetReader,
    widgets: Query<(&PanelWidget, &WidgetOf)>,
    mut commands: Commands,
) {
    for (controller, tooltip_for, materialized, placement) in &controllers {
        if tooltip_for.target() != materialized.target {
            continue;
        }
        match materialized.space() {
            PanelSpace::World => {
                let Some(source) = panel_reader.world(controller) else {
                    continue;
                };
                if let Some(target) = panel_reader.world(materialized.target) {
                    commands.attach_to_panel(source, target, placement.attachment);
                    continue;
                }
                if let Ok((widget, widget_of)) = widgets.get(materialized.target)
                    && let Some(owner) = panel_reader.world(widget_of.panel())
                    && let Some(target) = widget_reader.typed_entity(owner, widget.id())
                    && target.entity() == materialized.target
                {
                    commands.attach_to_widget(source, target, placement.attachment);
                    continue;
                }
                queue_general_tooltip_attachment(
                    &mut commands,
                    controller,
                    materialized,
                    placement.attachment,
                );
            },
            PanelSpace::Screen => {
                let Some(source) = panel_reader.screen(controller) else {
                    continue;
                };
                let attachment =
                    screen_tooltip_attachment(placement.attachment, materialized.layout_unit);
                if let Some(target) = panel_reader.screen(materialized.target) {
                    commands.attach_to_panel(source, target, attachment);
                    continue;
                }
                if let Ok((widget, widget_of)) = widgets.get(materialized.target)
                    && let Some(owner) = panel_reader.screen(widget_of.panel())
                    && let Some(target) = widget_reader.typed_entity(owner, widget.id())
                    && target.entity() == materialized.target
                {
                    commands.attach_to_widget(source, target, attachment);
                    continue;
                }
                queue_general_tooltip_attachment(
                    &mut commands,
                    controller,
                    materialized,
                    attachment,
                );
            },
        }
    }
}

#[derive(Clone, Copy)]
struct GeneralTooltipAttachmentOperation {
    controller:   Entity,
    target:       Entity,
    presentation: TooltipPresentationSpace,
    attachment:   PanelAttachment,
}

pub(super) fn queue_general_tooltip_attachment(
    commands: &mut Commands<'_, '_>,
    controller: Entity,
    materialized: &MaterializedTooltip,
    attachment: PanelAttachment,
) {
    commands.run_system_cached_with(
        apply_general_tooltip_attachment,
        GeneralTooltipAttachmentOperation {
            controller,
            target: materialized.target,
            presentation: materialized.presentation,
            attachment,
        },
    );
}

fn apply_general_tooltip_attachment(
    In(operation): In<GeneralTooltipAttachmentOperation>,
    controllers: Query<(&DiegeticPanel, &TooltipFor, &MaterializedTooltip)>,
    anchor_geometry: Query<(), With<ResolvedAnchorGeometry>>,
    mesh_targets: Query<(), With<MeshAnchorTarget>>,
    screen_targets: Query<&ScreenAnchorTarget>,
    windows: Query<(), With<Window>>,
    mut commands: Commands,
) {
    let Ok((panel, tooltip_for, materialized)) = controllers.get(operation.controller) else {
        return;
    };
    if PanelSpace::from(panel.coordinate_space()) != operation.presentation.space()
        || tooltip_for.target() != operation.target
        || materialized.target != operation.target
        || materialized.presentation.space() != operation.presentation.space()
    {
        return;
    }
    let target_is_current = match operation.presentation {
        TooltipPresentationSpace::World => {
            anchor_geometry.contains(operation.target) || mesh_targets.contains(operation.target)
        },
        TooltipPresentationSpace::Screen { window, .. } => screen_targets
            .get(operation.target)
            .is_ok_and(|target| target.window() == window && windows.contains(target.window())),
    };
    if !target_is_current {
        return;
    }
    commands.entity(operation.controller).insert((
        crate::panel::PanelAttachmentAuthored::new(
            operation.target,
            operation.attachment.source_anchor(),
            operation.attachment.target_anchor(),
        ),
        operation.attachment.offset(),
    ));
}

#[cfg(test)]
mod tests {
    use bevy::camera::visibility::RenderLayers;
    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Vec2;
    use bevy::prelude::Visibility;
    use bevy::prelude::Window;
    use bevy::transform::TransformPlugin;
    use bevy::window::PrimaryWindow;
    use hana_valence::AnchoredTo;

    use super::*;
    use crate::HeadlessLayoutPlugin;
    use crate::layout::El;
    use crate::layout::Unit;
    use crate::panel::PanelScreenBounds;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::declaration::PrepareTooltip;
    use crate::widgets::tooltip::materialization::TooltipReadiness;
    use crate::widgets::tooltip::*;

    const MATERIALIZATION_UPDATES: usize = 5;

    const SMALL_SCREEN_VIEWPORT: Vec2 = Vec2::new(240.0, 180.0);

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

    fn fixed_size_tooltip(size: Vec2) -> Tooltip {
        let mut tooltip = Tooltip::new(El::new());
        tooltip.with(El::new().size(size.x, size.y), |_| {});
        tooltip
    }

    #[test]
    fn changed_tooltip_target_detaches_the_materialized_old_target() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let first_target = app.world_mut().spawn_empty().id();
        let second_target = app.world_mut().spawn_empty().id();
        let first_bounds = PanelScreenBounds::new(Vec2::new(70.0, 60.0), Vec2::new(40.0, 20.0));
        let second_bounds = PanelScreenBounds::new(Vec2::new(140.0, 100.0), Vec2::new(40.0, 20.0));
        assert!(first_bounds.is_ok());
        assert!(second_bounds.is_ok());
        let (Some(first_bounds), Some(second_bounds)) = (first_bounds.ok(), second_bounds.ok())
        else {
            return;
        };
        let first_handle = app.world_mut().commands().screen_anchor_target(
            first_target,
            crate::screen_space::ScreenAnchorTarget::new(
                first_bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        app.world_mut().commands().screen_anchor_target(
            second_target,
            crate::screen_space::ScreenAnchorTarget::new(
                second_bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(first_handle, fixed_size_tooltip(Vec2::new(60.0, 30.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipFor::new(second_target));
        app.update();

        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
        assert!(
            app.world()
                .get::<crate::panel::PanelAttachmentAuthored>(controller)
                .is_none()
        );
        assert!(app.world().get::<AnchoredTo>(controller).is_none());
        assert!(
            app.world()
                .get::<crate::panel::ResolvedScreenPanelPosition>(controller)
                .is_some_and(|position| position.anchor_position.is_none())
        );
        assert_eq!(
            app.world()
                .get::<MaterializedTooltip>(controller)
                .map(MaterializedTooltip::target),
            Some(first_target)
        );
    }
}
