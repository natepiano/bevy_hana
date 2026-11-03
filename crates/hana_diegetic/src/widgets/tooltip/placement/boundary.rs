//! Placement area shared by world and screen tooltips.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::ChildOf;
use bevy::prelude::Entity;
use bevy::prelude::InheritedVisibility;
use bevy::prelude::Query;
use bevy::prelude::Rect;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;

use crate::layout::Anchor;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::PanelScreenBounds;
use crate::panel::ResolvedScreenPanelPosition;
use crate::screen_space;
use crate::widgets::tooltip::MaterializedTooltip;

#[derive(Clone, Copy)]
pub(super) struct CoveringPanel {
    pub(super) anchor:   Anchor,
    pub(super) top_left: Vec2,
    pub(super) size:     Vec2,
}

#[derive(Clone, Copy)]
enum ReservedEdge {
    Left(f32),
    Top(f32),
    Right(f32),
    Bottom(f32),
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct PanelInsets {
    left:   f32,
    top:    f32,
    right:  f32,
    bottom: f32,
}

pub(super) fn placement_area(
    window: Entity,
    target_panel: Entity,
    tooltip_target: Entity,
    tooltip_layer: TooltipScreenLayer,
    window_size: Vec2,
    viewport: Rect,
    panels: &Query<
        (
            Entity,
            &DiegeticPanel,
            Option<&Transform>,
            Option<&ResolvedScreenPanelPosition>,
            Option<&Visibility>,
            Option<&InheritedVisibility>,
            Option<&PanelAttachmentAuthored>,
            Option<&ChildOf>,
        ),
        Without<MaterializedTooltip>,
    >,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Rect {
    let covering = panels.iter().filter_map(
        |(entity, panel, transform, resolved, visibility, inherited, attachment, parent)| {
            if entity == target_panel
                || target_ancestor(entity, target_panel, panels)
                || attached_to_tooltip_target(attachment, tooltip_target)
                || !panel_is_drawn(visibility, inherited, parent)
            {
                return None;
            }
            let CoordinateSpace::Screen {
                window: panel_window,
                camera_order,
                render_layers,
                ..
            } = panel.coordinate_space()
            else {
                return None;
            };
            if !panel_draws_above(*camera_order, render_layers, &tooltip_layer) {
                return None;
            }
            let panel_window = match panel_window {
                WindowRef::Primary => primary_window.single().ok()?,
                WindowRef::Entity(entity) => *entity,
            };
            if panel_window != window {
                return None;
            }
            let bounds = screen_space::screen_panel_rect(panel, resolved, transform, window_size)?
                .projected_bounds()?;
            Some(CoveringPanel {
                anchor:   panel.anchor(),
                top_left: bounds.top_left(),
                size:     bounds.size(),
            })
        },
    );
    placement_area_for_panels(window_size, viewport, covering)
}

#[derive(Clone)]
pub(super) enum TooltipScreenLayer {
    World(isize),
    Screen { camera_order: isize },
}

const fn panel_draws_above(
    order: isize,
    _layers: &RenderLayers,
    tooltip: &TooltipScreenLayer,
) -> bool {
    match tooltip {
        TooltipScreenLayer::World(camera_order) | TooltipScreenLayer::Screen { camera_order } => {
            order >= *camera_order
        },
    }
}

pub(super) fn horizontal_room_before_floating_panels(
    window: Entity,
    target_panel: Entity,
    tooltip_target: Entity,
    tooltip_layer: TooltipScreenLayer,
    window_size: Vec2,
    tooltip_top: f32,
    tooltip_height: f32,
    right_start: f32,
    left_end: f32,
    mut rooms: [f32; 2],
    panels: &Query<
        (
            Entity,
            &DiegeticPanel,
            Option<&Transform>,
            Option<&ResolvedScreenPanelPosition>,
            Option<&Visibility>,
            Option<&InheritedVisibility>,
            Option<&PanelAttachmentAuthored>,
            Option<&ChildOf>,
        ),
        Without<MaterializedTooltip>,
    >,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> [f32; 2] {
    for (entity, panel, transform, resolved, visibility, inherited, attachment, parent) in panels {
        if entity == target_panel
            || target_ancestor(entity, target_panel, panels)
            || attached_to_tooltip_target(attachment, tooltip_target)
            || !panel_is_drawn(visibility, inherited, parent)
        {
            continue;
        }
        let CoordinateSpace::Screen {
            window: panel_window,
            camera_order,
            render_layers,
            ..
        } = panel.coordinate_space()
        else {
            continue;
        };
        if !panel_draws_above(*camera_order, render_layers, &tooltip_layer) {
            continue;
        }
        let panel_window = match panel_window {
            WindowRef::Primary => primary_window.single().ok(),
            WindowRef::Entity(entity) => Some(*entity),
        };
        if panel_window != Some(window) {
            continue;
        }
        let Some(bounds) = screen_space::screen_panel_rect(panel, resolved, transform, window_size)
            .and_then(crate::screen_space::ScreenPanelRect::projected_bounds)
        else {
            continue;
        };
        let panel_start = bounds.top_left();
        let panel_end = panel_start + bounds.size();
        if panel_start.y >= tooltip_top + tooltip_height || panel_end.y <= tooltip_top {
            continue;
        }
        let margin = crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN;
        if panel_end.x > right_start {
            rooms[0] = rooms[0].min(panel_start.x - margin - right_start);
        }
        if panel_start.x < left_end {
            rooms[1] = rooms[1].min(left_end - panel_end.x - margin);
        }
    }
    rooms.map(|room| room.max(0.0))
}

fn panel_is_drawn(
    visibility: Option<&Visibility>,
    inherited: Option<&InheritedVisibility>,
    parent: Option<&ChildOf>,
) -> bool {
    match visibility {
        Some(Visibility::Hidden) => false,
        Some(Visibility::Visible) => true,
        _ => parent.is_none() || inherited.is_none_or(|inherited| inherited.get()),
    }
}

fn target_ancestor(
    candidate: Entity,
    target: Entity,
    panels: &Query<
        (
            Entity,
            &DiegeticPanel,
            Option<&Transform>,
            Option<&ResolvedScreenPanelPosition>,
            Option<&Visibility>,
            Option<&InheritedVisibility>,
            Option<&PanelAttachmentAuthored>,
            Option<&ChildOf>,
        ),
        Without<MaterializedTooltip>,
    >,
) -> bool {
    let mut current = target;
    for _ in 0..64 {
        let Ok((_, _, _, _, _, _, Some(attachment), _)) = panels.get(current) else {
            return false;
        };
        current = attachment.target();
        if current == candidate {
            return true;
        }
    }
    false
}

fn attached_to_tooltip_target(
    attachment: Option<&PanelAttachmentAuthored>,
    tooltip_target: Entity,
) -> bool {
    attachment.is_some_and(|attachment| attachment.target() == tooltip_target)
}

pub(super) fn floating_panel_overlaps(
    window: Entity,
    target_panel: Entity,
    tooltip_target: Entity,
    tooltip_layer: TooltipScreenLayer,
    window_size: Vec2,
    bounds: PanelScreenBounds,
    panels: &Query<
        (
            Entity,
            &DiegeticPanel,
            Option<&Transform>,
            Option<&ResolvedScreenPanelPosition>,
            Option<&Visibility>,
            Option<&InheritedVisibility>,
            Option<&PanelAttachmentAuthored>,
            Option<&ChildOf>,
        ),
        Without<MaterializedTooltip>,
    >,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> bool {
    panels.iter().any(
        |(entity, panel, transform, resolved, visibility, inherited, attachment, parent)| {
            if entity == target_panel
                || target_ancestor(entity, target_panel, panels)
                || attached_to_tooltip_target(attachment, tooltip_target)
                || !panel_is_drawn(visibility, inherited, parent)
            {
                return false;
            }
            let CoordinateSpace::Screen {
                window: panel_window,
                camera_order: panel_order,
                render_layers: panel_layers,
                ..
            } = panel.coordinate_space()
            else {
                return false;
            };
            if !panel_draws_above(*panel_order, panel_layers, &tooltip_layer) {
                return false;
            }
            let panel_window = match panel_window {
                WindowRef::Primary => primary_window.single().ok(),
                WindowRef::Entity(entity) => Some(*entity),
            };
            if panel_window != Some(window) {
                return false;
            }
            let Some(panel_bounds) =
                screen_space::screen_panel_rect(panel, resolved, transform, window_size)
                    .and_then(crate::screen_space::ScreenPanelRect::projected_bounds)
            else {
                return false;
            };
            if !reserved_edge_choices(
                window_size,
                CoveringPanel {
                    anchor:   panel.anchor(),
                    top_left: panel_bounds.top_left(),
                    size:     panel_bounds.size(),
                },
            )
            .is_empty()
            {
                return false;
            }
            let a = bounds.top_left();
            let b = panel_bounds.top_left();
            a.x < b.x + panel_bounds.size().x
                && b.x < a.x + bounds.size().x
                && a.y < b.y + panel_bounds.size().y
                && b.y < a.y + bounds.size().y
        },
    )
}

pub(super) fn floating_panel_covers(
    window: Entity,
    target_panel: Entity,
    tooltip_target: Entity,
    tooltip_layer: TooltipScreenLayer,
    window_size: Vec2,
    bounds: PanelScreenBounds,
    panels: &Query<
        (
            Entity,
            &DiegeticPanel,
            Option<&Transform>,
            Option<&ResolvedScreenPanelPosition>,
            Option<&Visibility>,
            Option<&InheritedVisibility>,
            Option<&PanelAttachmentAuthored>,
            Option<&ChildOf>,
        ),
        Without<MaterializedTooltip>,
    >,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> bool {
    // A floating panel that covers every reference sample hides the reference.
    let corners = [bounds.top_left(), bounds.top_left() + bounds.size()];
    panels.iter().any(
        |(entity, panel, transform, resolved, visibility, inherited, attachment, parent)| {
            if entity == target_panel
                || target_ancestor(entity, target_panel, panels)
                || attached_to_tooltip_target(attachment, tooltip_target)
                || !panel_is_drawn(visibility, inherited, parent)
            {
                return false;
            }
            let CoordinateSpace::Screen {
                window: panel_window,
                camera_order,
                render_layers,
                ..
            } = panel.coordinate_space()
            else {
                return false;
            };
            if !panel_draws_above(*camera_order, render_layers, &tooltip_layer) {
                return false;
            }
            let panel_window = match panel_window {
                WindowRef::Primary => primary_window.single().ok(),
                WindowRef::Entity(entity) => Some(*entity),
            };
            if panel_window != Some(window) {
                return false;
            }
            let Some(cover) =
                screen_space::screen_panel_rect(panel, resolved, transform, window_size)
                    .and_then(crate::screen_space::ScreenPanelRect::projected_bounds)
            else {
                return false;
            };
            corners[0].x >= cover.top_left().x
                && corners[0].y >= cover.top_left().y
                && corners[1].x <= cover.top_left().x + cover.size().x
                && corners[1].y <= cover.top_left().y + cover.size().y
        },
    )
}

pub(super) fn placement_area_for_panels(
    window_size: Vec2,
    viewport: Rect,
    panels: impl IntoIterator<Item = CoveringPanel>,
) -> Rect {
    let insets = covering_panel_insets(window_size, panels);
    Rect {
        min: viewport.min.max(Vec2::new(insets.left, insets.top)),
        max: viewport
            .max
            .min(window_size - Vec2::new(insets.right, insets.bottom)),
    }
}

fn covering_panel_insets(
    window_size: Vec2,
    panels: impl IntoIterator<Item = CoveringPanel>,
) -> PanelInsets {
    let choices: Vec<Vec<ReservedEdge>> = panels
        .into_iter()
        .map(|panel| reserved_edge_choices(window_size, panel))
        .filter(|edges| !edges.is_empty())
        .collect();
    let mut best = PanelInsets::default();
    let mut best_area = f32::NEG_INFINITY;
    let combinations: usize = choices.iter().map(Vec::len).product();
    for combination in 0..combinations {
        let mut remaining = combination;
        let mut sides = [0.0_f32; 4];
        for edges in &choices {
            let edge = edges[remaining % edges.len()];
            remaining /= edges.len();
            match edge {
                ReservedEdge::Left(depth) => sides[0] = sides[0].max(depth),
                ReservedEdge::Top(depth) => sides[1] = sides[1].max(depth),
                ReservedEdge::Right(depth) => sides[2] = sides[2].max(depth),
                ReservedEdge::Bottom(depth) => sides[3] = sides[3].max(depth),
            }
        }
        let free =
            (window_size - Vec2::new(sides[0] + sides[2], sides[1] + sides[3])).max(Vec2::ZERO);
        let area = free.x * free.y;
        if area > best_area {
            best_area = area;
            best = PanelInsets {
                left:   sides[0],
                top:    sides[1],
                right:  sides[2],
                bottom: sides[3],
            };
        }
    }
    best
}

fn reserved_edge_choices(window_size: Vec2, panel: CoveringPanel) -> Vec<ReservedEdge> {
    let bottom_right = panel.top_left + panel.size;
    let left = (panel.top_left.x <= 0.0).then_some(ReservedEdge::Left(bottom_right.x));
    let top = (panel.top_left.y <= 0.0).then_some(ReservedEdge::Top(bottom_right.y));
    let right = (bottom_right.x >= window_size.x)
        .then_some(ReservedEdge::Right(window_size.x - panel.top_left.x));
    let bottom = (bottom_right.y >= window_size.y)
        .then_some(ReservedEdge::Bottom(window_size.y - panel.top_left.y));
    match panel.anchor {
        Anchor::TopLeft => [left, top].into_iter().flatten().collect(),
        Anchor::TopCenter => top.into_iter().collect(),
        Anchor::TopRight => [right, top].into_iter().flatten().collect(),
        Anchor::CenterLeft => left.into_iter().collect(),
        Anchor::Center => Vec::new(),
        Anchor::CenterRight => right.into_iter().collect(),
        Anchor::BottomLeft => [left, bottom].into_iter().flatten().collect(),
        Anchor::BottomCenter => bottom.into_iter().collect(),
        Anchor::BottomRight => [right, bottom].into_iter().flatten().collect(),
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Window;

    use super::*;
    use crate::layout::LayoutBuilder;
    use crate::layout::Px;

    #[test]
    fn bottom_left_help_panel_reserves_bottom_band() {
        let window = Vec2::new(1269.0, 1006.0);
        let help = CoveringPanel {
            anchor:   Anchor::BottomLeft,
            top_left: Vec2::new(0.0, 744.0),
            size:     Vec2::new(482.0, 262.0),
        };
        let insets = covering_panel_insets(window, [help]);
        assert_eq!(insets.bottom.to_bits(), 262.0_f32.to_bits());
        assert_eq!(insets.left.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn panel_hidden_by_ancestor_does_not_block_placement() {
        let parent = ChildOf(Entity::PLACEHOLDER);
        assert!(!panel_is_drawn(
            Some(&Visibility::Inherited),
            Some(&InheritedVisibility::HIDDEN),
            Some(&parent),
        ));
        assert!(panel_is_drawn(
            Some(&Visibility::Inherited),
            Some(&InheritedVisibility::HIDDEN),
            None,
        ));
    }

    #[allow(
        clippy::expect_used,
        reason = "invalid screen panel fixtures must fail immediately"
    )]
    fn overlapping_panel_blocks_tooltip(
        panel_order: isize,
        panel_layer: usize,
        tooltip_order: isize,
        _tooltip_layer: usize,
    ) -> bool {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let panel = DiegeticPanel::screen()
            .size(Px(180.0), Px(90.0))
            .anchor(Anchor::Center)
            .window_entity(window)
            .camera_order(panel_order)
            .render_layers(RenderLayers::layer(panel_layer))
            .with_tree(LayoutBuilder::new(180.0, 90.0).build())
            .build();
        let panel = panel.expect("screen panel");
        app.world_mut().spawn(panel);
        let bounds = PanelScreenBounds::new(Vec2::new(350.0, 280.0), Vec2::new(40.0, 20.0));
        let bounds = bounds.expect("screen bounds");
        app.world_mut()
            .run_system_once(
                move |panels: Query<
                    (
                        Entity,
                        &DiegeticPanel,
                        Option<&Transform>,
                        Option<&ResolvedScreenPanelPosition>,
                        Option<&Visibility>,
                        Option<&InheritedVisibility>,
                        Option<&PanelAttachmentAuthored>,
                        Option<&ChildOf>,
                    ),
                    Without<MaterializedTooltip>,
                >,
                      primary: Query<Entity, With<PrimaryWindow>>| {
                    floating_panel_overlaps(
                        window,
                        Entity::PLACEHOLDER,
                        Entity::PLACEHOLDER,
                        TooltipScreenLayer::Screen {
                            camera_order: tooltip_order,
                        },
                        Vec2::new(800.0, 600.0),
                        bounds,
                        &panels,
                        &primary,
                    )
                },
            )
            .expect("floating overlap query")
    }

    #[test]
    fn equal_order_panel_on_other_screen_layer_blocks_a_tooltip() {
        assert!(overlapping_panel_blocks_tooltip(100, 1, 100, 0));
    }

    #[test]
    fn higher_order_panel_blocks_disjoint_render_layers() {
        assert!(overlapping_panel_blocks_tooltip(101, 1, 100, 0));
    }

    #[test]
    fn lower_order_panel_cannot_block_a_tooltip() {
        assert!(!overlapping_panel_blocks_tooltip(99, 0, 100, 0));
    }
}
