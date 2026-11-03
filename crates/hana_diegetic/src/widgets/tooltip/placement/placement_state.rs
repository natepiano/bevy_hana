#[cfg(test)]
use bevy::camera::visibility::RenderLayers;
#[cfg(test)]
use bevy::prelude::App;
use bevy::prelude::Component;
#[cfg(test)]
use bevy::prelude::Entity;
use bevy::prelude::Mat4;
use bevy::prelude::Mut;
use bevy::prelude::Rect;
#[cfg(test)]
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
#[cfg(test)]
use bevy::prelude::Window;
#[cfg(test)]
use bevy::window::PrimaryWindow;

use crate::layout::Unit;
use crate::panel::PanelAttachment;
#[cfg(test)]
use crate::panel::PanelScreenBounds;
use crate::widgets::constants::QUAD_ANCHOR_COUNT;
#[cfg(test)]
use crate::widgets::tooltip::PrepareTooltip;
#[cfg(test)]
use crate::widgets::tooltip::Tooltip;
#[cfg(test)]
#[cfg(test)]
use crate::widgets::tooltip::TooltipReadiness;
use crate::widgets::tooltip::TooltipWidthConstraint;

#[derive(Clone, Component, Debug, PartialEq)]
pub(in crate::widgets::tooltip) struct TooltipPlacementState {
    pub(in crate::widgets::tooltip) attachment: PanelAttachment,
    pub(in crate::widgets::tooltip) result:     TooltipPlacementResult,
    pub(super) world_target:                    WorldTargetObservation,
    pub(super) world_occlusion:                 WorldTooltipOcclusion,
    pub(super) side_room_width:                 SideRoomWidth,
    world_view:                                 WorldTooltipViewObservation,
    screen_viewport_width:                      ScreenViewportWidth,
}

impl TooltipPlacementState {
    pub(in crate::widgets::tooltip) const fn pending(attachment: PanelAttachment) -> Self {
        Self {
            attachment,
            result: TooltipPlacementResult::Pending,
            world_target: WorldTargetObservation::Unobserved,
            world_occlusion: WorldTooltipOcclusion::SceneBound,
            side_room_width: SideRoomWidth::Unused,
            world_view: WorldTooltipViewObservation::Unobserved,
            screen_viewport_width: ScreenViewportWidth::Unobserved,
        }
    }

    pub(in crate::widgets::tooltip) const fn reset_new_show(&mut self) {
        self.side_room_width = SideRoomWidth::Unused;
        self.world_occlusion = WorldTooltipOcclusion::SceneBound;
        self.result = TooltipPlacementResult::Pending;
    }

    pub(super) fn observe_screen_viewport_width(&mut self, width: f32) -> bool {
        let changed = matches!(
            self.screen_viewport_width,
            ScreenViewportWidth::Observed(previous) if (previous - width).abs() > f32::EPSILON
        );
        self.screen_viewport_width = ScreenViewportWidth::Observed(width);
        changed
    }

    pub(super) fn observe_world_view(
        placement: &mut Mut<'_, Self>,
        view: WorldTooltipView,
    ) -> bool {
        if placement.world_view == WorldTooltipViewObservation::LastObserved(view) {
            return false;
        }
        let changed = matches!(
            placement.world_view,
            WorldTooltipViewObservation::LastObserved(_)
        );
        placement.world_view = WorldTooltipViewObservation::LastObserved(view);
        changed
    }
}

/// The camera values that can change a world tooltip's available room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WorldTooltipView {
    pub(super) clip_from_view:  Mat4,
    pub(super) world_from_view: Mat4,
    pub(super) viewport:        Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum WorldTooltipViewObservation {
    Unobserved,
    LastObserved(WorldTooltipView),
}

/// Which occlusion contract the chosen world placement follows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorldTooltipOcclusion {
    /// The tooltip occupies the scene and must be clear of every opaque mesh.
    SceneBound,
    /// The no-room fallback intentionally overlays its visible host card.
    /// The target must remain visible, while host content may lie behind the
    /// tooltip's already-frontmost panel plane.
    CardOverlay,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ScreenViewportWidth {
    Unobserved,
    Observed(f32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SideRoomWidth {
    Unused,
    Applied,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TooltipWidthLimit {
    Natural,
    Constrained(f32),
}

impl TooltipWidthLimit {
    pub(super) const fn from_constraint(constraint: Option<&TooltipWidthConstraint>) -> Self {
        match constraint {
            Some(TooltipWidthConstraint(width)) => Self::Constrained(*width),
            None => Self::Natural,
        }
    }
}

pub(super) fn set_tooltip_placement(
    placement: &mut Mut<'_, TooltipPlacementState>,
    attachment: PanelAttachment,
    result: TooltipPlacementResult,
) {
    if placement.attachment != attachment || placement.result != result {
        placement.attachment = attachment;
        placement.result = result;
    }
}

pub(in crate::widgets::tooltip) fn set_tooltip_placement_result(
    placement: &mut Mut<'_, TooltipPlacementState>,
    result: TooltipPlacementResult,
) {
    set_tooltip_placement(placement, placement.attachment, result);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::widgets::tooltip) enum TooltipPlacementResult {
    Pending,
    Fits,
    Unavailable,
}

#[derive(Clone, Copy)]
pub(super) enum TooltipPlacementDecision {
    ConstrainWidth(f32),
    ConstrainSideRoomWidth(f32),
    RestoreWidth,
    Move(PanelAttachment),
    Fits,
    Unavailable,
}

#[cfg(test)]
#[derive(Default, Resource)]
pub(in crate::widgets::tooltip) struct TooltipPlacementRunCount {
    pub(in crate::widgets::tooltip) screen: usize,
    pub(super) world:                       usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::widgets::tooltip) struct WorldAnchorFrame {
    pub(super) position: Vec3,
    pub(super) right:    Vec3,
    pub(super) up:       Vec3,
    pub(super) normal:   Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::widgets::tooltip) struct WorldTargetGeometry {
    pub(super) anchors:                 [WorldAnchorFrame; QUAD_ANCHOR_COUNT],
    pub(super) layout_unit:             Unit,
    pub(super) world_per_layout_unit:   Vec2,
    pub(super) world_per_layout_unit_z: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::widgets::tooltip) enum WorldTargetObservation {
    Unobserved,
    LastObserved(Box<WorldTargetGeometry>),
}

#[cfg(test)]
mod tests {
    use bevy::prelude::MinimalPlugins;
    use bevy::transform::TransformPlugin;

    use super::*;
    use crate::HeadlessLayoutPlugin;
    use crate::layout::Anchor;
    use crate::layout::El;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::ScreenAnchorCommandsExt;
    use crate::widgets::tooltip::TooltipCommandsExt;

    const MATERIALIZATION_UPDATES: usize = 5;

    const TEST_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

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
    fn pending_placement_has_no_world_target_observation() {
        let attachment = PanelAttachment::new(Anchor::TopLeft, Anchor::BottomRight);
        let placement = TooltipPlacementState::pending(attachment);

        assert_eq!(placement.attachment, attachment);
        assert_eq!(placement.result, TooltipPlacementResult::Pending);
        assert_eq!(placement.world_target, WorldTargetObservation::Unobserved);
    }

    #[test]
    fn width_limit_names_natural_and_constrained_widths() {
        assert_eq!(
            TooltipWidthLimit::from_constraint(None),
            TooltipWidthLimit::Natural
        );
        assert_eq!(
            TooltipWidthLimit::from_constraint(Some(&TooltipWidthConstraint(42.0))),
            TooltipWidthLimit::Constrained(42.0),
        );
    }

    #[test]
    fn settled_screen_tooltip_has_a_quiet_placement_frame() {
        let mut app = materialization_app();
        app.init_resource::<TooltipPlacementRunCount>();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(350.0, 260.0), Vec2::new(100.0, 80.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, fixed_size_tooltip(Vec2::new(60.0, 30.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        app.update();
        app.update();

        let placement_runs = app.world().resource::<TooltipPlacementRunCount>().screen;
        let placement_tick = app
            .world()
            .entity(controller)
            .get_change_ticks::<TooltipPlacementState>()
            .map(|ticks| ticks.changed);
        let placement = app
            .world()
            .get::<TooltipPlacementState>(controller)
            .cloned();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        app.update();

        assert_eq!(
            app.world().resource::<TooltipPlacementRunCount>().screen,
            placement_runs,
        );
        assert_eq!(
            app.world()
                .entity(controller)
                .get_change_ticks::<TooltipPlacementState>()
                .map(|ticks| ticks.changed),
            placement_tick,
        );
        assert_eq!(
            app.world()
                .get::<TooltipPlacementState>(controller)
                .cloned(),
            placement,
        );
    }
}
