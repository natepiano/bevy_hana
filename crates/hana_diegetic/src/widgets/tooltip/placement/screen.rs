use bevy::camera::visibility::RenderLayers;
#[cfg(test)]
use bevy::prelude::App;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
#[cfg(test)]
use bevy::prelude::GlobalTransform;
use bevy::prelude::InheritedVisibility;
use bevy::prelude::Mut;
#[cfg(test)]
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Rect;
#[cfg(test)]
use bevy::prelude::ResMut;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::window::PrimaryWindow;

use super::boundary;
use super::boundary::TooltipScreenLayer;
use super::placement_state;
use super::placement_state::SideRoomWidth;
use super::placement_state::TooltipPlacementDecision;
use super::placement_state::TooltipPlacementResult;
#[cfg(test)]
use super::placement_state::TooltipPlacementRunCount;
use super::placement_state::TooltipPlacementState;
use super::placement_state::TooltipWidthLimit;
use super::viewport;
use super::viewport::HostBorder;
use super::viewport::OrientedScreenRect;
use super::viewport::TooltipProjectedFootprint;
use super::viewport::TooltipSlideBounds;
use super::world;
use crate::layout::Anchor;
use crate::layout::BoundingBox;
use crate::layout::LayoutResult;
#[cfg(test)]
use crate::layout::Px;
use crate::layout::RenderCommandKind;
#[cfg(test)]
use crate::layout::Text;
use crate::layout::Unit;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachment;
use crate::panel::PanelAttachmentAuthored;
#[cfg(test)]
use crate::panel::PanelEntity;
use crate::panel::PanelLayout;
use crate::panel::PanelScreenBounds;
use crate::panel::PanelSpace;
use crate::panel::ResolvedScreenPanelPosition;
#[cfg(test)]
use crate::panel::Screen;
#[cfg(test)]
use crate::panel::WidgetEntity;
use crate::screen_space;
use crate::screen_space::ScreenAnchorTarget;
use crate::screen_space::ScreenPanelRect;
use crate::widgets::PanelWidget;
use crate::widgets::TOOLTIP_SHADOW_EXTENT;
use crate::widgets::TooltipBoundary;
use crate::widgets::WidgetAnchorRect;
use crate::widgets::WidgetOf;
use crate::widgets::constants::TooltipPaintedReach;
use crate::widgets::tooltip;
use crate::widgets::tooltip::MaterializedTooltip;
#[cfg(test)]
use crate::widgets::tooltip::PrepareTooltip;
use crate::widgets::tooltip::ScreenTooltipAttachmentCorrection;
#[cfg(test)]
use crate::widgets::tooltip::Tooltip;
use crate::widgets::tooltip::TooltipFor;
use crate::widgets::tooltip::TooltipPresentationSpace;
#[cfg(test)]
use crate::widgets::tooltip::TooltipReadiness;
use crate::widgets::tooltip::TooltipWidthConstraint;
use crate::widgets::tooltip::TooltipWidthConstraintRequest;
use crate::widgets::tooltip::attachment;

#[derive(Clone)]
pub(in crate::widgets::tooltip) struct ScreenTooltipTarget {
    pub(super) bounds:        PanelScreenBounds,
    pub(super) rect:          ScreenPanelRect,
    pub(super) window:        Entity,
    pub(super) camera_order:  isize,
    pub(super) render_layers: RenderLayers,
    pub(super) layout_unit:   Unit,
    pub(super) layout_scale:  Vec2,
}

impl ScreenTooltipTarget {
    pub(in crate::widgets::tooltip) fn presentation_matches(
        &self,
        materialized: &MaterializedTooltip,
    ) -> bool {
        materialized.presentation
            == TooltipPresentationSpace::Screen {
                window:       self.window,
                camera_order: self.camera_order,
            }
            && materialized.render_layers == self.render_layers
            && materialized.layout_unit == self.layout_unit
    }
}

pub(in crate::widgets::tooltip) fn resolve_screen_tooltip_placements(
    mut controllers: Query<(
        Entity,
        &TooltipFor,
        &MaterializedTooltip,
        &super::super::declaration::TooltipPhase,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &Transform,
        &ResolvedScreenPanelPosition,
        Option<&TooltipWidthConstraint>,
        Option<&TooltipWidthConstraintRequest>,
        &mut TooltipPlacementState,
    )>,
    panels: Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    panel_layouts: Query<&ComputedDiegeticPanel>,
    boundary_panels: Query<
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
    widgets: Query<(&WidgetOf, &WidgetAnchorRect), With<PanelWidget>>,
    tooltip_boundaries: Query<&TooltipBoundary>,
    screen_targets: Query<&ScreenAnchorTarget>,
    windows: Query<&Window>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
    #[cfg(test)] mut run_count: Option<ResMut<TooltipPlacementRunCount>>,
) {
    #[cfg(test)]
    if let Some(run_count) = run_count.as_mut() {
        run_count.screen += 1;
    }
    for (
        controller,
        tooltip_for,
        materialized,
        phase,
        panel,
        computed,
        transform,
        resolved_position,
        width_constraint,
        width_request,
        mut placement,
    ) in &mut controllers
    {
        if materialized.space() != PanelSpace::Screen {
            continue;
        }
        if phase.is_visible() {
            continue;
        }
        if matches!(width_request, Some(TooltipWidthConstraintRequest::Restore)) {
            placement_state::set_tooltip_placement_result(
                &mut placement,
                TooltipPlacementResult::Pending,
            );
            continue;
        }
        if tooltip_for.target() != materialized.target {
            placement_state::set_tooltip_placement_result(
                &mut placement,
                TooltipPlacementResult::Unavailable,
            );
            continue;
        }
        let decision = screen_tooltip_placement(
            ScreenTooltipPlacementInput {
                materialized,
                panel,
                computed,
                transform,
                resolved_position,
                width_limit: TooltipWidthLimit::from_constraint(width_constraint),
                side_room_width: placement.side_room_width,
                current_attachment: placement.attachment,
            },
            &panels,
            &panel_layouts,
            &boundary_panels,
            &widgets,
            &tooltip_boundaries,
            &screen_targets,
            &windows,
            &primary_window,
            &mut placement,
        );
        apply_screen_placement_decision(
            controller,
            materialized,
            &mut placement,
            decision,
            &mut commands,
        );
    }
}

fn apply_screen_placement_decision(
    controller: Entity,
    materialized: &MaterializedTooltip,
    placement: &mut Mut<'_, TooltipPlacementState>,
    decision: TooltipPlacementDecision,
    commands: &mut Commands<'_, '_>,
) {
    match decision {
        TooltipPlacementDecision::ConstrainWidth(width)
        | TooltipPlacementDecision::ConstrainSideRoomWidth(width) => {
            if matches!(
                decision,
                TooltipPlacementDecision::ConstrainSideRoomWidth(_)
            ) {
                placement.side_room_width = SideRoomWidth::Applied;
            }
            placement_state::set_tooltip_placement_result(
                placement,
                TooltipPlacementResult::Pending,
            );
            commands
                .entity(controller)
                .insert(TooltipWidthConstraintRequest::Apply(width));
        },
        TooltipPlacementDecision::RestoreWidth => {
            placement.side_room_width = SideRoomWidth::Unused;
            placement_state::set_tooltip_placement_result(
                placement,
                TooltipPlacementResult::Pending,
            );
            commands
                .entity(controller)
                .insert(TooltipWidthConstraintRequest::Restore);
        },
        TooltipPlacementDecision::Move(attachment) => {
            placement_state::set_tooltip_placement(
                placement,
                attachment,
                TooltipPlacementResult::Fits,
            );
            let rendered_attachment =
                attachment::screen_tooltip_attachment(attachment, materialized.layout_unit);
            commands.entity(controller).insert((
                crate::panel::PanelAttachmentAuthored::new(
                    materialized.target,
                    rendered_attachment.source_anchor(),
                    rendered_attachment.target_anchor(),
                ),
                rendered_attachment.offset(),
                ScreenTooltipAttachmentCorrection,
            ));
        },
        TooltipPlacementDecision::Fits => {
            placement_state::set_tooltip_placement_result(placement, TooltipPlacementResult::Fits);
        },
        TooltipPlacementDecision::Unavailable => {
            placement_state::set_tooltip_placement_result(
                placement,
                TooltipPlacementResult::Unavailable,
            );
        },
    }
}

struct ScreenTooltipPlacementInput<'a> {
    materialized:       &'a MaterializedTooltip,
    panel:              &'a DiegeticPanel,
    computed:           &'a ComputedDiegeticPanel,
    transform:          &'a Transform,
    resolved_position:  &'a ResolvedScreenPanelPosition,
    width_limit:        TooltipWidthLimit,
    side_room_width:    SideRoomWidth,
    current_attachment: PanelAttachment,
}

struct ScreenTooltipGeometrySnapshot {
    target:       ScreenTooltipTarget,
    viewport:     Rect,
    max_width:    f32,
    reach:        TooltipPaintedReach,
    source_rect:  ScreenPanelRect,
    target_panel: Entity,
    window_size:  Vec2,
}

fn screen_tooltip_placement(
    input: ScreenTooltipPlacementInput<'_>,
    panels: &Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    panel_layouts: &Query<&ComputedDiegeticPanel>,
    boundary_panels: &Query<
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
    widgets: &Query<(&WidgetOf, &WidgetAnchorRect), With<PanelWidget>>,
    tooltip_boundaries: &Query<&TooltipBoundary>,
    screen_targets: &Query<&ScreenAnchorTarget>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    placement: &mut Mut<'_, TooltipPlacementState>,
) -> TooltipPlacementDecision {
    let geometry = match resolve_screen_tooltip_geometry(
        &input,
        panels,
        widgets,
        screen_targets,
        windows,
        primary_window,
        boundary_panels,
        placement,
    ) {
        Ok(geometry) => geometry,
        Err(decision) => return decision,
    };
    fit_screen_tooltip(
        &input,
        &geometry,
        panels,
        panel_layouts,
        boundary_panels,
        widgets,
        tooltip_boundaries,
        windows,
        primary_window,
    )
}

fn resolve_screen_tooltip_geometry(
    input: &ScreenTooltipPlacementInput<'_>,
    panels: &Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    widgets: &Query<(&WidgetOf, &WidgetAnchorRect), With<PanelWidget>>,
    screen_targets: &Query<&ScreenAnchorTarget>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    boundary_panels: &Query<
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
    placement: &mut Mut<'_, TooltipPlacementState>,
) -> Result<ScreenTooltipGeometrySnapshot, TooltipPlacementDecision> {
    let Some(target) = screen_tooltip_target(
        input.materialized.target,
        panels,
        widgets,
        screen_targets,
        windows,
        primary_window,
    ) else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    if !target.presentation_matches(input.materialized) {
        return Err(TooltipPlacementDecision::Unavailable);
    }
    let Ok(window) = windows.get(target.window) else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    let window_size = Vec2::new(window.width(), window.height());
    let target_panel = widgets
        .get(input.materialized.target)
        .map_or(input.materialized.target, |(widget_of, _)| {
            widget_of.panel()
        });
    let viewport = boundary::placement_area(
        target.window,
        target_panel,
        input.materialized.target,
        TooltipScreenLayer::Screen {
            camera_order: target.camera_order,
        },
        window_size,
        tooltip::zero_origin_viewport(window_size),
        boundary_panels,
        primary_window,
    );
    if screen_reference_hidden(
        &target,
        target_panel,
        input.materialized.target,
        viewport,
        window_size,
        boundary_panels,
        primary_window,
    ) {
        return Err(TooltipPlacementDecision::Unavailable);
    }
    let reach = TOOLTIP_SHADOW_EXTENT.painted_reach(input.panel.points_to_world());
    let Some(max_width) =
        viewport::usable_viewport_axis(reach.sides.mul_add(-2.0, viewport.width()))
    else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    let viewport_width_changed = placement
        .bypass_change_detection()
        .observe_screen_viewport_width(max_width);
    if screen_viewport_width_cap_needed(input.width_limit, max_width, viewport_width_changed) {
        return Err(TooltipPlacementDecision::ConstrainWidth(max_width));
    }
    let source_size = Vec2::new(input.panel.width(), input.panel.height());
    if !viewport::valid_screen_size(source_size) {
        return Err(TooltipPlacementDecision::Unavailable);
    }
    let Some(source_rect) = screen_space::screen_panel_rect(
        input.panel,
        Some(input.resolved_position),
        Some(input.transform),
        window_size,
    ) else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    let Some(usable_height) =
        viewport::usable_viewport_axis(viewport.height() - reach.above - reach.below)
    else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    if !tooltip_layout_fits_panel(input.panel, input.computed)
        || input.computed.content_height() > usable_height
    {
        return Err(TooltipPlacementDecision::Unavailable);
    }
    Ok(ScreenTooltipGeometrySnapshot {
        target,
        viewport,
        max_width,
        reach,
        source_rect,
        target_panel,
        window_size,
    })
}

fn fit_screen_tooltip(
    input: &ScreenTooltipPlacementInput<'_>,
    geometry: &ScreenTooltipGeometrySnapshot,
    panels: &Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    panel_layouts: &Query<&ComputedDiegeticPanel>,
    boundary_panels: &Query<
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
    widgets: &Query<(&WidgetOf, &WidgetAnchorRect), With<PanelWidget>>,
    tooltip_boundaries: &Query<&TooltipBoundary>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> TooltipPlacementDecision {
    let ScreenTooltipGeometrySnapshot {
        target,
        viewport,
        max_width,
        reach,
        source_rect,
        target_panel,
        window_size,
    } = geometry;
    if matches!(input.width_limit, TooltipWidthLimit::Constrained(_))
        && input.side_room_width == SideRoomWidth::Applied
    {
        let reflow = screen_side_room_reflow(
            input,
            target,
            *viewport,
            *max_width,
            *reach,
            *target_panel,
            *window_size,
            boundary_panels,
            primary_window,
        );
        if matches!(reflow, TooltipPlacementDecision::ConstrainSideRoomWidth(_)) {
            return reflow;
        }
    }
    let slide_bounds = screen_slide_bounds(
        input.materialized.target,
        panels,
        panel_layouts,
        widgets,
        tooltip_boundaries,
        windows,
        primary_window,
    );
    let attachment = fit_tooltip_in_viewport_avoiding_panels(
        input.materialized.authored_attachment,
        *source_rect,
        *reach,
        target,
        slide_bounds,
        *viewport,
        *target_panel,
        input.materialized.target,
        *window_size,
        boundary_panels,
        primary_window,
    );
    match attachment {
        Some(attachment) if attachment == input.current_attachment => {
            TooltipPlacementDecision::Fits
        },
        Some(attachment) => TooltipPlacementDecision::Move(attachment),
        None => screen_side_room_reflow(
            input,
            target,
            *viewport,
            *max_width,
            *reach,
            *target_panel,
            *window_size,
            boundary_panels,
            primary_window,
        ),
    }
}

fn screen_viewport_width_cap_needed(
    width_limit: TooltipWidthLimit,
    max_width: f32,
    viewport_width_changed: bool,
) -> bool {
    match width_limit {
        TooltipWidthLimit::Natural => true,
        TooltipWidthLimit::Constrained(current) => {
            current > max_width + f32::EPSILON
                || (viewport_width_changed && current < max_width - f32::EPSILON)
        },
    }
}

fn screen_reference_hidden(
    target: &ScreenTooltipTarget,
    target_panel: Entity,
    tooltip_target: Entity,
    viewport: Rect,
    window_size: Vec2,
    boundary_panels: &Query<
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
    !viewport::bounds_intersect_viewport(target.bounds, viewport)
        || boundary::floating_panel_covers(
            target.window,
            target_panel,
            tooltip_target,
            TooltipScreenLayer::Screen {
                camera_order: target.camera_order,
            },
            window_size,
            target.bounds,
            boundary_panels,
            primary_window,
        )
}

fn screen_side_room_reflow(
    input: &ScreenTooltipPlacementInput<'_>,
    target: &ScreenTooltipTarget,
    viewport: Rect,
    max_width: f32,
    reach: TooltipPaintedReach,
    target_panel: Entity,
    window_size: Vec2,
    boundary_panels: &Query<
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
) -> TooltipPlacementDecision {
    let gap = input
        .materialized
        .authored_attachment
        .offset()
        .to_layout_units(target.layout_unit)
        .truncate()
        .length()
        * target.layout_scale.x.abs();
    let right_start = target.bounds.top_left().x + target.bounds.size().x + gap + reach.sides;
    let left_end = target.bounds.top_left().x - gap - reach.sides;
    let right = viewport.max.x - crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN - right_start;
    let left = left_end - viewport.min.x - crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN;
    let top =
        (target.bounds.size().y - input.panel.height()).mul_add(0.5, target.bounds.top_left().y);
    let rooms = boundary::horizontal_room_before_floating_panels(
        target.window,
        target_panel,
        input.materialized.target,
        TooltipScreenLayer::Screen {
            camera_order: target.camera_order,
        },
        window_size,
        top - reach.above,
        input.panel.height() + reach.above + reach.below,
        right_start,
        left_end,
        [right.min(max_width), left.min(max_width)],
        boundary_panels,
        primary_window,
    );
    let available = rooms.into_iter().fold(0.0, f32::max);
    let desired = available / target.layout_scale.x.abs();
    if desired.is_finite()
        && desired > 0.0
        && (input.side_room_width == SideRoomWidth::Applied || desired < input.panel.width() - 0.01)
        && match input.width_limit {
            TooltipWidthLimit::Natural => true,
            TooltipWidthLimit::Constrained(current) => {
                (desired - current).abs() > 0.01
                    && (input.side_room_width == SideRoomWidth::Applied || desired < current)
            },
        }
    {
        TooltipPlacementDecision::ConstrainSideRoomWidth(desired)
    } else {
        TooltipPlacementDecision::Unavailable
    }
}

pub(in crate::widgets::tooltip) fn screen_tooltip_target(
    target: Entity,
    panels: &Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    widgets: &Query<(&WidgetOf, &WidgetAnchorRect), With<PanelWidget>>,
    screen_targets: &Query<&ScreenAnchorTarget>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<ScreenTooltipTarget> {
    if let Ok((panel, transform, resolved)) = panels.get(target) {
        return screen_panel_target(panel, transform, resolved, windows, primary_window);
    }
    if let Ok((widget_of, anchor_rect)) = widgets.get(target) {
        let (panel, transform, resolved) = panels.get(widget_of.panel()).ok()?;
        let owner = screen_panel_target(panel, transform, resolved, windows, primary_window)?;
        let rect =
            crate::screen_space::ScreenPanelRect::from_widget(owner.rect, *anchor_rect, transform)?;
        return Some(ScreenTooltipTarget {
            bounds: rect.projected_bounds()?,
            rect,
            layout_scale: rect.layout_scale(),
            ..owner
        });
    }
    let target = screen_targets.get(target).ok()?;
    windows.get(target.window()).ok()?;
    let rect = crate::screen_space::ScreenPanelRect::from_screen_target(target);
    Some(ScreenTooltipTarget {
        bounds: rect.projected_bounds()?,
        rect,
        window: target.window(),
        camera_order: target.camera_order(),
        render_layers: target.render_layers().clone(),
        layout_unit: target.layout_unit(),
        layout_scale: Vec2::ONE,
    })
}

fn screen_panel_target(
    panel: &DiegeticPanel,
    transform: &Transform,
    resolved: Option<&ResolvedScreenPanelPosition>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<ScreenTooltipTarget> {
    let CoordinateSpace::Screen {
        camera_order,
        render_layers,
        window,
        ..
    } = panel.coordinate_space()
    else {
        return None;
    };
    let window = tooltip::live_window(*window, windows, primary_window)?;
    let window_component = windows.get(window).ok()?;
    let viewport = Vec2::new(window_component.width(), window_component.height());
    let rect = screen_space::screen_panel_rect(panel, resolved, Some(transform), viewport)?;
    Some(ScreenTooltipTarget {
        bounds: rect.projected_bounds()?,
        rect,
        window,
        camera_order: *camera_order,
        render_layers: render_layers.clone(),
        layout_unit: panel.layout_unit(),
        layout_scale: Vec2::ONE,
    })
}

pub(super) fn tooltip_layout_fits_panel(
    panel: &DiegeticPanel,
    computed: &ComputedDiegeticPanel,
) -> bool {
    let PanelLayout::Solved(result) = computed.layout() else {
        return false;
    };
    let Some(root) = result.computed.first().map(|layout| layout.bounds) else {
        return false;
    };
    // Content dimensions are physical world units; panel dimensions are
    // authored layout units. Compare their common source in layout points.
    // Fit divides points into layout units, so the return multiplication may
    // round one float step below the solved root.
    let viewport = Vec2::new(panel.width(), panel.height()) * panel.layout_unit().to_points();
    if root.width > viewport.x.next_up() || root.height > viewport.y.next_up() {
        return false;
    }
    result.computed.iter().skip(1).all(|layout| {
        let bounds = layout.bounds;
        bounds.x + f32::EPSILON >= root.x
            && bounds.y + f32::EPSILON >= root.y
            && bounds.x + bounds.width <= root.x + root.width + f32::EPSILON
            && bounds.y + bounds.height <= root.y + root.height + f32::EPSILON
    })
}

#[cfg(test)]
fn fit_tooltip_in_viewport(
    authored: PanelAttachment,
    source: ScreenPanelRect,
    target: &ScreenTooltipTarget,
    viewport: Rect,
) -> Option<PanelAttachment> {
    fit_tooltip_in_viewport_avoiding_occluders(
        authored,
        source,
        pixel_layout_reach(),
        target,
        TooltipSlideBounds::PlacementAreaOnly,
        viewport,
        |_| false,
    )
}

/// Shadow reach of a tooltip laid out in pixels.
#[cfg(test)]
fn pixel_layout_reach() -> TooltipPaintedReach {
    TOOLTIP_SHADOW_EXTENT.painted_reach(Unit::Pixels.to_points().recip())
}

/// What bounds a widget target's tooltip slide: its marked element and its host
/// panel, each projected through the panel as the target is, the host's border
/// turned with the panel. Any other target slides within the placement area
/// alone.
fn screen_slide_bounds(
    tooltip_target: Entity,
    panels: &Query<(
        &DiegeticPanel,
        &Transform,
        Option<&ResolvedScreenPanelPosition>,
    )>,
    panel_layouts: &Query<&ComputedDiegeticPanel>,
    widgets: &Query<(&WidgetOf, &WidgetAnchorRect), With<PanelWidget>>,
    tooltip_boundaries: &Query<&TooltipBoundary>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> TooltipSlideBounds {
    let Ok((widget_of, _)) = widgets.get(tooltip_target) else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    let Ok((panel, transform, resolved)) = panels.get(widget_of.panel()) else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    let Some(owner) = screen_panel_target(panel, transform, resolved, windows, primary_window)
    else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    let place = |bounds: BoundingBox| {
        ScreenPanelRect::from_widget(owner.rect, WidgetAnchorRect::new(panel, bounds), transform)
    };
    let project = |bounds: BoundingBox| place(bounds).and_then(ScreenPanelRect::projected_bounds);
    let orient = |bounds: BoundingBox| {
        let rect = place(bounds)?;
        Some(OrientedScreenRect::from_corners(
            rect.oriented_anchor_point(Anchor::TopLeft)?,
            rect.oriented_anchor_point(Anchor::TopRight)?,
            rect.oriented_anchor_point(Anchor::BottomLeft)?,
        ))
    };
    let Some(host) = panel_layouts
        .get(widget_of.panel())
        .ok()
        .and_then(|computed| computed.layout().solved())
        .and_then(|result| {
            let outer = result.computed.first()?.bounds;
            Some(HostBorder::new(
                orient(outer)?,
                orient(border_inner_edge(result, outer))?,
            ))
        })
    else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    match tooltip_boundaries.get(tooltip_target) {
        Ok(TooltipBoundary::Element(element)) => project(*element)
            .map_or(TooltipSlideBounds::Host(host), |element| {
                TooltipSlideBounds::marked_element(element, host)
            }),
        Ok(TooltipBoundary::PlacementAreaOnly) | Err(_) => TooltipSlideBounds::Host(host),
    }
}

/// The inner edge of the border drawn on the element of `result` whose bounds
/// are `element`; an element with no border has `element` as its inner edge.
pub(super) fn border_inner_edge(result: &LayoutResult, element: BoundingBox) -> BoundingBox {
    result
        .commands
        .iter()
        .find_map(|command| match &command.kind {
            RenderCommandKind::Border { border } if command.bounds == element => {
                Some(BoundingBox {
                    x:      element.x + border.left.value,
                    y:      element.y + border.top.value,
                    width:  (element.width - border.horizontal()).max(0.0),
                    height: (element.height - border.vertical()).max(0.0),
                })
            },
            _ => None,
        })
        .unwrap_or(element)
}

/// `shift` moved by under half a pixel so that `body`, moved by it, lands its
/// top-left corner on whole pixels and draws each edge of its frame crisp.
fn whole_pixel_shift(body: PanelScreenBounds, shift: Vec2) -> Vec2 {
    let corner = body.top_left() + shift;
    shift + corner.round() - corner
}

/// Body and painted screen bounds of `source` placed by `source_anchor`.
fn placed_footprint(
    source: ScreenPanelRect,
    source_anchor: Anchor,
    source_anchor_position: Vec2,
    reach: TooltipPaintedReach,
) -> Option<TooltipProjectedFootprint> {
    Some(TooltipProjectedFootprint {
        body:    source.placed_bounds(source_anchor, source_anchor_position)?,
        painted: source.placed_outset_bounds(
            source_anchor,
            source_anchor_position,
            Vec2::new(-reach.sides, -reach.above),
            Vec2::new(reach.sides, reach.below),
        )?,
    })
}

fn fit_tooltip_in_viewport_avoiding_panels(
    authored: PanelAttachment,
    source: ScreenPanelRect,
    reach: TooltipPaintedReach,
    target: &ScreenTooltipTarget,
    slide_bounds: TooltipSlideBounds,
    viewport: Rect,
    target_panel: Entity,
    tooltip_target: Entity,
    window_size: Vec2,
    boundary_panels: &Query<
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
) -> Option<PanelAttachment> {
    fit_tooltip_in_viewport_avoiding_occluders(
        authored,
        source,
        reach,
        target,
        slide_bounds,
        viewport,
        |bounds| {
            boundary::floating_panel_overlaps(
                target.window,
                target_panel,
                tooltip_target,
                TooltipScreenLayer::Screen {
                    camera_order: target.camera_order,
                },
                window_size,
                bounds,
                boundary_panels,
                primary_window,
            )
        },
    )
}

/// The first side and slide, in order, whose painted bounds fit `viewport`,
/// meet no occluder and keep clear of the host's border: beside the control
/// first, then out past the host. A placement across the host's border stays
/// only when no other fits.
fn fit_tooltip_in_viewport_avoiding_occluders(
    authored: PanelAttachment,
    source: ScreenPanelRect,
    reach: TooltipPaintedReach,
    target: &ScreenTooltipTarget,
    slide_bounds: TooltipSlideBounds,
    viewport: Rect,
    occluded: impl Fn(PanelScreenBounds) -> bool,
) -> Option<PanelAttachment> {
    viewport::usable_viewport_axis(viewport.width())?;
    viewport::usable_viewport_axis(viewport.height())?;
    let authored_layout = authored.offset().to_layout_units(target.layout_unit);
    let authored_local_pixels = Vec2::new(
        authored_layout.x * target.layout_scale.x,
        authored_layout.y * target.layout_scale.y,
    );
    let preferred = viewport::preferred_side(authored, authored_local_pixels);
    let order = viewport::tooltip_side_order(preferred, target.bounds, viewport);
    let mut crossing_host_border = None;
    for (side, slide) in slide_bounds.slide_order(order, target.bounds) {
        let (source_anchor, target_anchor, side_pixels) = if side == preferred {
            (
                authored.source_anchor(),
                authored.target_anchor(),
                authored_local_pixels,
            )
        } else {
            let (source_anchor, target_anchor) = side.anchors();
            let gap = preferred.gap_kept_by(authored_local_pixels);
            (source_anchor, target_anchor, side.gap_offset(gap))
        };
        let target_point = target.rect.oriented_anchor_point(target_anchor)?;
        let source_anchor_position =
            target_point + viewport::rotate_screen_vector(side_pixels, target.rect.angle());
        let natural = placed_footprint(source, source_anchor, source_anchor_position, reach)?;
        let shift = whole_pixel_shift(
            natural.body,
            slide.shift(side, natural, target_point, viewport),
        );
        let shift = shift
            + world::tooltip_source_clearance_shift(side, natural.painted, shift, target.bounds);
        let local_pixels =
            side_pixels + viewport::rotate_screen_vector(shift, -target.rect.angle());
        let source_anchor_position =
            target_point + viewport::rotate_screen_vector(local_pixels, target.rect.angle());
        let painted =
            placed_footprint(source, source_anchor, source_anchor_position, reach)?.painted;
        if !viewport::bounds_fit_viewport(painted.top_left(), painted.size(), viewport)
            || !world::tooltip_clears_source(painted, target.bounds)
        {
            continue;
        }
        let clears_host_border = slide_bounds.clears_host_border(painted);
        if (!clears_host_border && crossing_host_border.is_some()) || occluded(painted) {
            continue;
        }
        let attachment = PanelAttachment::new(source_anchor, target_anchor).with_offset(
            PanelAnchorOffset::new(
                local_pixels.x / target.layout_scale.x,
                local_pixels.y / target.layout_scale.y,
            )
            .with_z(authored_layout.z),
        );
        if clears_host_border {
            return Some(attachment);
        }
        crossing_host_border = Some(attachment);
    }
    crossing_host_border
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "inline placement fixtures fail with a direct message"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::hover::PickingInteraction;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::TimerMode;
    use bevy::time::TimeUpdateStrategy;
    use bevy::transform::TransformPlugin;

    use super::boundary::CoveringPanel;
    use super::*;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::FitMax;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::Padding;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::TextStyle;
    use crate::TextWrap;
    use crate::layout::El;
    use crate::panel::PanelAttachmentAuthored;
    use crate::panel::ResolvedScreenPanelPosition;
    use crate::screen_space::ScreenAttachmentResolveDiagnostics;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::ScreenAnchorCommandsExt;
    use crate::widgets::tooltip::TooltipCommandsExt;
    use crate::widgets::tooltip::TooltipPhase;
    use crate::widgets::tooltip::Tooltips;

    fn screen_tooltip_target(bounds: PanelScreenBounds) -> ScreenTooltipTarget {
        let target = crate::screen_space::ScreenAnchorTarget::new(
            bounds,
            Entity::PLACEHOLDER,
            0,
            RenderLayers::layer(0),
            Unit::Pixels,
        );
        let rect = crate::screen_space::ScreenPanelRect::from_screen_target(&target);
        ScreenTooltipTarget {
            bounds,
            rect,
            window: Entity::PLACEHOLDER,
            camera_order: 0,
            render_layers: RenderLayers::layer(0),
            layout_unit: Unit::Pixels,
            layout_scale: Vec2::ONE,
        }
    }

    fn screen_tooltip_source(size: Vec2) -> Option<ScreenPanelRect> {
        let bounds = PanelScreenBounds::new(Vec2::ZERO, size).ok()?;
        Some(screen_tooltip_target(bounds).rect)
    }

    const GEOMETRY_EPSILON: f32 = 1e-4;

    const GROWN_SCREEN_VIEWPORT: Vec2 = Vec2::new(520.0, 180.0);

    const MATERIALIZATION_UPDATES: usize = 5;

    const ROTATED_TARGET_ANGLE: f32 = -std::f32::consts::FRAC_PI_4;

    const SMALL_SCREEN_VIEWPORT: Vec2 = Vec2::new(240.0, 180.0);

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

    fn resolved_screen_tooltip_bounds(
        app: &App,
        controller: Entity,
        window: Entity,
    ) -> Option<PanelScreenBounds> {
        let panel = app.world().get::<DiegeticPanel>(controller)?;
        let transform = app.world().get::<Transform>(controller)?;
        let resolved = app.world().get::<ResolvedScreenPanelPosition>(controller)?;
        let window = app.world().get::<Window>(window)?;
        crate::screen_space::screen_panel_rect(
            panel,
            Some(resolved),
            Some(transform),
            Vec2::new(window.width(), window.height()),
        )?
        .projected_bounds()
    }

    fn tooltip_target_tree() -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(50.0, 20.0).button("action"), |_| {});
        builder.build()
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

    #[test]
    fn hovered_fitmax_screen_widget_shows_a_sized_tooltip() {
        let mut app = materialization_app();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ));
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
        let mut tooltip = Tooltip::new(El::column());
        tooltip.text("Attach");
        tooltip.text("Press to attach or detach the card that follows the Pin button");
        let mut tree = LayoutBuilder::new(250.0, 120.0);
        tree.with(
            El::new()
                .size(100.0, 30.0)
                .button("action")
                .tooltip(tooltip),
            |_| {},
        );
        let panel = DiegeticPanel::screen()
            .size(FitMax(Px(250.0).into()), FitMax(Px(120.0).into()))
            .anchor(Anchor::TopRight)
            .window_entity(window)
            .with_tree(tree.build())
            .build()
            .expect("FitMax screen panel");
        let panel = app.world_mut().spawn(panel).id();
        update_materialization(&mut app);
        let button = widget(&mut app, panel);
        let controller = app
            .world()
            .get::<Tooltips>(button)
            .and_then(|tooltips| tooltips.iter().next())
            .expect("button tooltip");
        let mut card_tree = LayoutBuilder::new(180.0, 50.0);
        card_tree.text("Attached status card");
        let card = DiegeticPanel::screen()
            .size(FitMax(Px(180.0).into()), FitMax(Px(50.0).into()))
            .anchor(Anchor::CenterRight)
            .window_entity(window)
            .with_tree(card_tree.build())
            .build()
            .expect("attached screen card");
        let card = app.world_mut().spawn(card).id();
        app.world_mut().commands().attach_to_widget(
            PanelEntity::<Screen>::from_validated(card, PanelSpace::Screen),
            WidgetEntity::<Screen>::from_validated(button, panel, PanelSpace::Screen),
            PanelAttachment::new(Anchor::TopRight, Anchor::BottomRight),
        );
        app.world_mut()
            .entity_mut(button)
            .insert(PickingInteraction::Hovered);
        for _ in 0..60 {
            app.update();
        }
        let tooltip_panel = app
            .world()
            .get::<DiegeticPanel>(controller)
            .expect("hovered tooltip materialized");
        assert!(tooltip_panel.width() > 0.0 && tooltip_panel.height() > 0.0);
        assert_eq!(
            app.world()
                .get::<TooltipPhase>(controller)
                .map(TooltipPhase::is_visible),
            Some(true),
            "readiness: {:?}, placement: {:?}",
            app.world().get::<TooltipReadiness>(controller),
            app.world()
                .get::<TooltipPlacementState>(controller)
                .map(|state| state.result),
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Inherited)
        );
        let tooltip_depth = app
            .world()
            .get::<Transform>(controller)
            .expect("tooltip pose")
            .translation
            .z;
        let card_depth = app
            .world()
            .get::<Transform>(card)
            .expect("card pose")
            .translation
            .z;
        assert!(tooltip_depth > card_depth + 64.0);
    }

    #[test]
    fn equal_order_disjoint_layer_panel_hides_screen_control_tooltip() {
        let mut app = materialization_app();
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
        let target_panel = DiegeticPanel::screen()
            .size(Px(200.0), Px(100.0))
            .anchor(Anchor::Center)
            .screen_position(400.0, 300.0)
            .window_entity(window)
            .camera_order(37)
            .render_layers(RenderLayers::layer(6))
            .with_tree(tooltip_target_tree())
            .build()
            .expect("screen control panel");
        let target_panel = app.world_mut().spawn(target_panel).id();
        app.update();
        let control = widget(&mut app, target_panel);
        let overlay = DiegeticPanel::screen()
            .size(Px(300.0), Px(160.0))
            .anchor(Anchor::Center)
            .screen_position(400.0, 300.0)
            .window_entity(window)
            .camera_order(37)
            .render_layers(RenderLayers::layer(7))
            .with_tree(LayoutBuilder::new(300.0, 160.0).build())
            .build()
            .expect("equal order overlay");
        app.world_mut().spawn(overlay);
        let controller = app.world_mut().commands().spawn_tooltip(
            WidgetEntity::<Screen>::from_validated(control, target_panel, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(50.0, 20.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn screen_reflows_into_room_before_far_floating_panel() {
        let mut app = materialization_app();
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
        let target_bounds = PanelScreenBounds::new(Vec2::new(20.0, 290.0), Vec2::new(40.0, 20.0))
            .expect("screen control bounds");
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                target_bounds,
                window,
                5,
                RenderLayers::layer(0),
                Unit::Pixels,
            ),
        );
        let overlay = DiegeticPanel::screen()
            .size(Px(170.0), Px(120.0))
            .anchor(Anchor::Center)
            .screen_position(425.0, 300.0)
            .window_entity(window)
            .camera_order(6)
            .with_tree(LayoutBuilder::new(170.0, 120.0).build())
            .build()
            .expect("floating overlay");
        app.world_mut().spawn(overlay);
        let mut tooltip = Tooltip::new(El::new())
            .source_anchor(Anchor::CenterLeft)
            .target_anchor(Anchor::CenterRight);
        tooltip.text(Text::new("alpha ".repeat(28), TextStyle::new(20.0)).wrap(TextWrap::Words));
        let controller = app.world_mut().commands().spawn_tooltip(handle, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        for _ in 0..10 {
            app.update();
        }
        let bounds = resolved_screen_tooltip_bounds(&app, controller, window)
            .expect("placed screen tooltip bounds");
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}, width={:?}, request={:?}, panel_width={:?}, content_height={:?}, bounds={bounds:?}",
            app.world().get::<TooltipPlacementState>(controller),
            app.world()
                .get::<TooltipWidthConstraint>(controller)
                .map(|width| width.0),
            app.world()
                .get::<TooltipWidthConstraintRequest>(controller)
                .is_some(),
            app.world()
                .get::<DiegeticPanel>(controller)
                .map(DiegeticPanel::width),
            app.world()
                .get::<ComputedDiegeticPanel>(controller)
                .map(ComputedDiegeticPanel::content_height),
        );
        assert!(
            bounds.top_left().x >= target_bounds.top_left().x + target_bounds.size().x,
            "tooltip must stay right: {bounds:?}"
        );
        assert!(
            bounds.top_left().x + bounds.size().x <= 425.0 - 85.0 - 8.0 + 1.0,
            "tooltip crosses overlay: {bounds:?}"
        );
        assert!(
            bounds.size().x > 100.0,
            "tooltip did not use the available gap: {bounds:?}"
        );
    }

    fn waiting_side_room_tooltip() -> (App, Entity, Entity, Entity) {
        let mut app = materialization_app();
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
        let bounds = PanelScreenBounds::new(Vec2::new(20.0, 290.0), Vec2::new(40.0, 20.0))
            .expect("screen control bounds");
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            ScreenAnchorTarget::new(bounds, window, 5, RenderLayers::layer(0), Unit::Pixels),
        );
        let overlay = DiegeticPanel::screen()
            .size(Px(170.0), Px(120.0))
            .anchor(Anchor::Center)
            .screen_position(425.0, 300.0)
            .window_entity(window)
            .camera_order(6)
            .with_tree(LayoutBuilder::new(170.0, 120.0).build())
            .build()
            .expect("floating overlay");
        let overlay = app.world_mut().spawn(overlay).id();
        let mut tooltip = Tooltip::new(El::new())
            .source_anchor(Anchor::CenterLeft)
            .target_anchor(Anchor::CenterRight);
        tooltip.text(
            Text::new("alpha ".repeat(28), TextStyle::new(20.0))
                .wrap(crate::layout::TextWrap::Words),
        );
        let controller = app.world_mut().commands().spawn_tooltip(handle, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        (app, window, controller, overlay)
    }

    #[test]
    fn cursor_changes_do_not_restart_side_room_reflow() {
        let (mut app, window, controller, _) = waiting_side_room_tooltip();
        let mut cursor_x = 100.0;
        for _ in 0..18 {
            app.world_mut()
                .get_mut::<Window>(window)
                .expect("window")
                .set_cursor_position(Some(Vec2::new(cursor_x, 100.0)));
            app.update();
            cursor_x += 1.0;
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        let bounds =
            resolved_screen_tooltip_bounds(&app, controller, window).expect("placed tooltip");
        assert!(
            bounds.size().x > 100.0 && bounds.size().x < 275.0,
            "reflowed bounds: {bounds:?}"
        );
    }

    #[test]
    fn moving_overlay_away_restores_waiting_screen_tooltip_width() {
        let (mut app, window, controller, overlay) = waiting_side_room_tooltip();
        for _ in 0..10 {
            app.update();
        }
        let narrow = app
            .world()
            .get::<DiegeticPanel>(controller)
            .expect("tooltip")
            .width();
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        assert!(
            app.world_mut()
                .get_mut::<DiegeticPanel>(overlay)
                .expect("overlay")
                .set_screen_position(Vec2::new(700.0, 300.0))
        );
        for _ in 0..12 {
            app.update();
        }
        let expanded = app
            .world()
            .get::<DiegeticPanel>(controller)
            .expect("tooltip")
            .width();
        assert!(
            expanded > narrow + 50.0,
            "waiting tooltip stayed narrow: {narrow} -> {expanded}"
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        assert!(resolved_screen_tooltip_bounds(&app, controller, window).is_some());
    }

    #[test]
    fn screen_panel_and_widget_targets_transfer_panel_presentation_context() {
        let mut app = materialization_app();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let window = app
            .world_mut()
            .spawn(Window {
                resolution: TEST_VIEWPORT.as_uvec2().into(),
                ..Default::default()
            })
            .id();
        let camera_order = 37;
        let render_layers = RenderLayers::layer(6);
        let target_panel = DiegeticPanel::screen()
            .size(Px(200.0), Px(100.0))
            .window_entity(window)
            .camera_order(camera_order)
            .render_layers(render_layers.clone())
            .with_tree(tooltip_target_tree())
            .build();
        assert!(target_panel.is_ok());
        let target_panel = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        assert_ne!(target_panel, Entity::PLACEHOLDER);
        app.update();
        let target_widget = widget(&mut app, target_panel);

        let panel_controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<Screen>::from_validated(target_panel, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(40.0, 20.0)),
        );
        let widget_controller = app.world_mut().commands().spawn_tooltip(
            WidgetEntity::<Screen>::from_validated(target_widget, target_panel, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(40.0, 20.0)),
        );
        app.update();
        for controller in [panel_controller, widget_controller] {
            prepare_tooltip(&mut app, controller);
        }
        update_materialization(&mut app);

        for controller in [panel_controller, widget_controller] {
            let materialized = app.world().get::<MaterializedTooltip>(controller);
            assert!(materialized.is_some());
            if let Some(materialized) = materialized {
                assert_eq!(materialized.layout_unit(), Unit::Pixels);
                assert_eq!(
                    materialized.presentation,
                    TooltipPresentationSpace::Screen {
                        window,
                        camera_order
                    }
                );
                assert_eq!(materialized.render_layers, render_layers);
            }
            let diagnostics = app
                .world()
                .resource::<ScreenAttachmentResolveDiagnostics>()
                .current()
                .copied()
                .collect::<Vec<_>>();
            assert!(
                app.world()
                    .get::<crate::panel::ResolvedScreenPanelPosition>(controller)
                    .is_some_and(|position| position.anchor_position.is_some()),
                "controller {controller:?}: target_transform={}, placement={:?}, attachment={:?}, diagnostics={diagnostics:?}",
                app.world().get::<Transform>(target_panel).is_some(),
                app.world().get::<TooltipPlacementState>(controller),
                app.world()
                    .get::<crate::panel::PanelAttachmentAuthored>(controller),
            );
            assert_eq!(
                app.world().get::<TooltipReadiness>(controller),
                Some(&TooltipReadiness::Ready)
            );
        }
    }

    #[test]
    fn screen_keep_visible_reflows_words_and_rejects_an_unbreakable_overflow() {
        let mut app = materialization_app();
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
        let bounds =
            PanelScreenBounds::new(TEST_VIEWPORT * 0.5 - Vec2::splat(20.0), Vec2::splat(40.0));
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
        let font_size = 20.0;
        let mut reflowing = Tooltip::new(El::new());
        reflowing
            .text(Text::new("alpha ".repeat(40), TextStyle::new(font_size)).wrap(TextWrap::Words));
        let mut overflowing = Tooltip::new(El::new());
        overflowing.text(
            Text::new("x".repeat(100), TextStyle::new(font_size))
                .wrap(TextWrap::None)
                .layout(El::new().width(crate::layout::Sizing::fit_min(TEST_VIEWPORT.x + 1.0))),
        );
        let mut long_word_reflowing = Tooltip::new(El::new());
        long_word_reflowing
            .text(Text::new("x".repeat(100), TextStyle::new(font_size)).wrap(TextWrap::Words));
        let reflowing_controller = app.world_mut().commands().spawn_tooltip(handle, reflowing);
        let overflowing_controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, overflowing);
        let long_word_controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, long_word_reflowing);
        app.update();
        prepare_tooltip(&mut app, reflowing_controller);
        prepare_tooltip(&mut app, overflowing_controller);
        prepare_tooltip(&mut app, long_word_controller);
        update_materialization(&mut app);

        let usable_width = viewport::usable_viewport_axis(TEST_VIEWPORT.x);
        assert!(usable_width.is_some());
        let Some(usable_width) = usable_width else {
            return;
        };
        let reflowing_panel = app.world().get::<DiegeticPanel>(reflowing_controller);
        let reflowing_computed = app
            .world()
            .get::<ComputedDiegeticPanel>(reflowing_controller);
        assert!(reflowing_panel.is_some());
        assert!(reflowing_computed.is_some());
        if let (Some(panel), Some(computed)) = (reflowing_panel, reflowing_computed) {
            assert!(panel.width() <= usable_width);
            assert!(computed.content_height() > font_size);
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(reflowing_controller),
            Some(&TooltipReadiness::Ready)
        );
        let long_word_panel = app.world().get::<DiegeticPanel>(long_word_controller);
        let long_word_computed = app
            .world()
            .get::<ComputedDiegeticPanel>(long_word_controller);
        assert!(long_word_panel.is_some());
        assert!(long_word_computed.is_some());
        if let (Some(panel), Some(computed)) = (long_word_panel, long_word_computed) {
            assert!(panel.width() <= usable_width);
            assert!(computed.content_height() > font_size);
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(long_word_controller),
            Some(&TooltipReadiness::Ready)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(overflowing_controller),
            Some(&TooltipReadiness::Pending)
        );
        assert_eq!(
            app.world().get::<Visibility>(overflowing_controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn editable_field_frame_change_is_used_on_reveal_frame() {
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
        let field = ImeEditableFieldSpec::AppOwned(crate::ImeAppOwnedFieldSpec::new("test"));
        let mut tree = LayoutBuilder::new(240.0, 180.0);
        tree.with(
            El::new()
                .size(40.0, 20.0)
                .editable_field("action", field, TextStyle::new(10.0)),
            |_| {},
        );
        let panel = DiegeticPanel::screen()
            .size(Px(240.0), Px(180.0))
            .anchor(Anchor::Center)
            .screen_position(120.0, 90.0)
            .window_entity(window)
            .with_tree(tree.build())
            .build()
            .expect("field panel");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let field = widget(&mut app, panel);
        let controller = app.world_mut().commands().spawn_tooltip(
            WidgetEntity::<Screen>::from_validated(field, panel, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(70.0, 30.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        let initial = app
            .world()
            .get::<TooltipPlacementState>(controller)
            .expect("initial placement")
            .attachment;
        let field_spec = ImeEditableFieldSpec::AppOwned(crate::ImeAppOwnedFieldSpec::new("test"));
        let mut moved_tree = LayoutBuilder::new(240.0, 180.0);
        moved_tree.with(
            El::column().size(240.0, 180.0).padding(Padding {
                left: 180.0.into(),
                top: 130.0.into(),
                ..Default::default()
            }),
            |builder| {
                builder.with(
                    El::new().size(40.0, 20.0).editable_field(
                        "action",
                        field_spec,
                        TextStyle::new(10.0),
                    ),
                    |_| {},
                );
            },
        );
        app.world_mut()
            .commands()
            .set_tree(panel, moved_tree.build())
            .expect("field tree edit");
        app.world_mut()
            .entity_mut(field)
            .insert(PickingInteraction::Hovered);
        let timer = bevy::prelude::Timer::new(std::time::Duration::ZERO, TimerMode::Once);
        app.world_mut()
            .entity_mut(controller)
            .insert(crate::widgets::tooltip::declaration::TooltipPhase::WaitingToShow(timer));
        app.update();
        let placement = app
            .world()
            .get::<TooltipPlacementState>(controller)
            .expect("reveal placement");
        assert_ne!(placement.attachment, initial);
        assert_eq!(placement.attachment.source_anchor(), Anchor::BottomCenter);
        assert_eq!(placement.attachment.target_anchor(), Anchor::TopCenter);
        assert!(matches!(
            app.world()
                .get::<crate::widgets::tooltip::declaration::TooltipPhase>(controller),
            Some(crate::widgets::tooltip::declaration::TooltipPhase::Visible),
        ));
    }

    #[test]
    fn visible_screen_tooltip_keeps_attachment_and_width_after_target_moves() {
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
        let bounds = PanelScreenBounds::new(Vec2::new(80.0, 80.0), Vec2::new(40.0, 20.0))
            .expect("target bounds");
        let target = app.world_mut().spawn_empty().id();
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                0,
                RenderLayers::layer(0),
                Unit::Pixels,
            ),
        );
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, fixed_size_tooltip(Vec2::new(70.0, 30.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        let attachment = app
            .world()
            .get::<TooltipPlacementState>(controller)
            .expect("placement")
            .attachment;
        let width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .expect("panel")
            .width();
        app.world_mut()
            .entity_mut(controller)
            .insert(crate::widgets::tooltip::declaration::TooltipPhase::Visible);
        app.world_mut()
            .entity_mut(target)
            .insert(PickingInteraction::Hovered);
        let moved = PanelScreenBounds::new(Vec2::new(180.0, 130.0), Vec2::new(40.0, 20.0))
            .expect("moved bounds");
        app.world_mut()
            .entity_mut(target)
            .insert(crate::screen_space::ScreenAnchorTarget::new(
                moved,
                window,
                0,
                RenderLayers::layer(0),
                Unit::Pixels,
            ));
        app.update();
        assert_eq!(
            app.world()
                .get::<TooltipPlacementState>(controller)
                .expect("frozen placement")
                .attachment,
            attachment
        );
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(controller)
                .expect("frozen panel")
                .width()
                .to_bits(),
            width.to_bits()
        );
    }

    #[test]
    fn screen_keep_visible_tracks_rotated_target_moves_in_resolved_geometry() {
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
        let target_panel = DiegeticPanel::screen()
            .size(Px(80.0), Px(40.0))
            .anchor(Anchor::Center)
            .screen_position(185.0, 50.0)
            .window_entity(window)
            .with_tree(LayoutBuilder::new(80.0, 40.0).build())
            .build();
        assert!(target_panel.is_ok());
        let target_panel = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut()
                .spawn((
                    panel,
                    Transform::from_rotation(Quat::from_rotation_z(ROTATED_TARGET_ANGLE)),
                ))
                .id()
        });
        assert_ne!(target_panel, Entity::PLACEHOLDER);
        app.update();
        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<Screen>::from_validated(target_panel, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(70.0, 30.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let initial_attachment = app.world().get::<PanelAttachmentAuthored>(controller);
        assert!(initial_attachment.is_some());
        if let Some(initial_attachment) = initial_attachment {
            assert_eq!(initial_attachment.source_anchor(), Anchor::TopCenter);
            assert_eq!(initial_attachment.target_anchor(), Anchor::BottomCenter);
        }
        let initial_bounds = resolved_screen_tooltip_bounds(&app, controller, window);
        assert!(initial_bounds.is_some());
        if let Some(initial_bounds) = initial_bounds {
            assert!(viewport::bounds_fit_viewport(
                initial_bounds.top_left(),
                initial_bounds.size(),
                tooltip::zero_origin_viewport(SMALL_SCREEN_VIEWPORT),
            ));
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        let moved = app
            .world_mut()
            .get_mut::<DiegeticPanel>(target_panel)
            .is_some_and(|mut panel| panel.set_screen_position(Vec2::new(55.0, 130.0)));
        assert!(moved);
        app.update();
        app.update();

        let moved_attachment = app.world().get::<PanelAttachmentAuthored>(controller);
        assert!(moved_attachment.is_some());
        if let Some(moved_attachment) = moved_attachment {
            assert_eq!(moved_attachment.source_anchor(), Anchor::BottomCenter);
            assert_eq!(moved_attachment.target_anchor(), Anchor::TopCenter);
        }
        let moved_bounds = resolved_screen_tooltip_bounds(&app, controller, window);
        assert!(moved_bounds.is_some());
        if let Some(moved_bounds) = moved_bounds {
            assert!(viewport::bounds_fit_viewport(
                moved_bounds.top_left(),
                moved_bounds.size(),
                tooltip::zero_origin_viewport(SMALL_SCREEN_VIEWPORT),
            ));
        }
        let local_transform = app.world().get::<Transform>(controller);
        let global_transform = app.world().get::<GlobalTransform>(controller);
        assert!(local_transform.is_some());
        assert!(global_transform.is_some());
        if let (Some(local_transform), Some(global_transform)) = (local_transform, global_transform)
        {
            assert!(
                global_transform
                    .translation()
                    .abs_diff_eq(local_transform.translation, GEOMETRY_EPSILON,)
            );
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
    }

    #[test]
    fn attached_screen_target_move_updates_tooltip_placement_in_the_same_frame() {
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
        let root = DiegeticPanel::screen()
            .size(Px(40.0), Px(20.0))
            .anchor(Anchor::Center)
            .screen_position(120.0, 40.0)
            .window_entity(window)
            .with_tree(LayoutBuilder::new(40.0, 20.0).build())
            .build();
        let target = DiegeticPanel::screen()
            .size(Px(40.0), Px(20.0))
            .anchor(Anchor::Center)
            .window_entity(window)
            .with_tree(LayoutBuilder::new(40.0, 20.0).build())
            .build();
        assert!(root.is_ok());
        assert!(target.is_ok());
        let root = root.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        let target = target.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        assert_ne!(root, Entity::PLACEHOLDER);
        assert_ne!(target, Entity::PLACEHOLDER);
        app.update();
        app.world_mut().commands().attach_to_panel(
            PanelEntity::<Screen>::from_validated(target, PanelSpace::Screen),
            PanelEntity::<Screen>::from_validated(root, PanelSpace::Screen),
            PanelAttachment::new(Anchor::Center, Anchor::Center),
        );
        app.update();

        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<Screen>::from_validated(target, PanelSpace::Screen),
            fixed_size_tooltip(Vec2::new(60.0, 30.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        let moved = app
            .world_mut()
            .get_mut::<DiegeticPanel>(root)
            .is_some_and(|mut panel| panel.set_screen_position(Vec2::new(120.0, 140.0)));
        assert!(moved);
        app.update();

        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let placement = app.world().get::<TooltipPlacementState>(controller);
        assert!(placement.is_some());
        if let Some(placement) = placement {
            assert_eq!(placement.attachment.source_anchor(), Anchor::BottomCenter);
            assert_eq!(placement.attachment.target_anchor(), Anchor::TopCenter);
        }
        let bounds = resolved_screen_tooltip_bounds(&app, controller, window);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(viewport::bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                tooltip::zero_origin_viewport(SMALL_SCREEN_VIEWPORT),
            ));
        }
    }

    #[test]
    fn screen_viewport_growth_relaxes_the_previous_keep_visible_width_cap() {
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
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(100.0, 80.0), Vec2::new(40.0, 20.0));
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
        let mut tooltip = Tooltip::new(El::new());
        tooltip.text(Text::new("alpha ".repeat(3), TextStyle::new(20.0)).wrap(TextWrap::Words));
        let controller = app.world_mut().commands().spawn_tooltip(handle, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let constrained_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        assert!(constrained_width.is_some());
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );

        if let Some(mut window) = app.world_mut().get_mut::<Window>(window) {
            window
                .resolution
                .set(GROWN_SCREEN_VIEWPORT.x, GROWN_SCREEN_VIEWPORT.y);
        }
        update_materialization(&mut app);

        let grown_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        assert!(grown_width.is_some());
        if let (Some(constrained_width), Some(grown_width)) = (constrained_width, grown_width) {
            assert!(
                grown_width > constrained_width,
                "expected width to grow beyond {constrained_width}, got {grown_width}",
            );
        }
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        let bounds = resolved_screen_tooltip_bounds(&app, controller, window);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(viewport::bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                tooltip::zero_origin_viewport(GROWN_SCREEN_VIEWPORT),
            ));
        }
    }
    #[test]
    fn keep_visible_uses_the_opposite_side_and_preserves_the_margin() {
        let target = PanelScreenBounds::new(Vec2::new(40.0, 2.0), Vec2::new(20.0, 20.0));
        assert!(target.is_ok());
        let Some(target) = target.ok() else {
            return;
        };
        let attachment = PanelAttachment::new(Anchor::BottomCenter, Anchor::TopCenter);
        let target_context = screen_tooltip_target(target);
        let Some(source) = screen_tooltip_source(Vec2::new(30.0, 20.0)) else {
            return;
        };
        let fitted = fit_tooltip_in_viewport(
            attachment,
            source,
            &target_context,
            tooltip::zero_origin_viewport(Vec2::splat(100.0)),
        );
        assert!(fitted.is_some());
        let Some(fitted) = fitted else {
            return;
        };
        assert_eq!(fitted.source_anchor(), Anchor::TopCenter);
        assert_eq!(fitted.target_anchor(), Anchor::BottomCenter);
        let target_point = target_context
            .rect
            .oriented_anchor_point(fitted.target_anchor());
        assert!(target_point.is_some());
        let Some(target_point) = target_point else {
            return;
        };
        let local_offset = fitted.offset().to_layout_units(Unit::Pixels).truncate();
        let source_anchor_position = target_point
            + viewport::rotate_screen_vector(local_offset, target_context.rect.angle());
        let bounds = source.placed_bounds(fitted.source_anchor(), source_anchor_position);
        assert!(bounds.is_some());
        let Some(bounds) = bounds else {
            return;
        };
        assert!(viewport::bounds_fit_viewport(
            bounds.top_left(),
            bounds.size(),
            tooltip::zero_origin_viewport(Vec2::splat(100.0))
        ));
    }

    #[test]
    fn keep_visible_tooltip_stays_above_bottom_left_help_panel() {
        let window_size = Vec2::splat(100.0);
        let viewport = boundary::placement_area_for_panels(
            window_size,
            tooltip::zero_origin_viewport(window_size),
            [CoveringPanel {
                anchor:   Anchor::BottomLeft,
                top_left: Vec2::new(0.0, 60.0),
                size:     Vec2::new(50.0, 40.0),
            }],
        );
        let source = screen_tooltip_source(Vec2::new(30.0, 20.0));
        assert!(source.is_some());
        let Some(source) = source else { return };
        // Leave room beside the control: the old centred fixture could fit only
        // by covering its source with the tooltip's 46 by 39 px painted bounds.
        let target_bounds = PanelScreenBounds::new(Vec2::new(70.0, 30.0), Vec2::splat(10.0));
        assert!(target_bounds.is_ok());
        let Ok(target_bounds) = target_bounds else {
            return;
        };
        let target = screen_tooltip_target(target_bounds);
        let authored = PanelAttachment::new(Anchor::TopCenter, Anchor::BottomCenter);
        let fitted = fit_tooltip_in_viewport(authored, source, &target, viewport);
        assert!(fitted.is_some(), "tooltip fits above the help panel");
        let Some(fitted) = fitted else { return };
        let target_point = target.rect.oriented_anchor_point(fitted.target_anchor());
        assert!(target_point.is_some());
        let Some(target_point) = target_point else {
            return;
        };
        let offset = fitted.offset().to_layout_units(Unit::Pixels).truncate();
        let footprint = placed_footprint(
            source,
            fitted.source_anchor(),
            target_point + offset,
            pixel_layout_reach(),
        );
        assert!(footprint.is_some());
        let Some(footprint) = footprint else { return };
        assert!(world::tooltip_clears_source(
            footprint.painted,
            target.bounds
        ));
        assert!(
            footprint.painted.top_left().y + footprint.painted.size().y
                <= 60.0 - crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN
        );
    }

    #[test]
    fn keep_visible_rejects_oversize_and_off_viewport_results() {
        let centered = PanelScreenBounds::new(Vec2::new(45.0, 45.0), Vec2::splat(10.0));
        assert!(centered.is_ok());
        let Some(centered) = centered.ok() else {
            return;
        };
        let attachment = PanelAttachment::new(Anchor::BottomCenter, Anchor::TopCenter);
        let context = screen_tooltip_target(centered);
        let usable_width = viewport::usable_viewport_axis(100.0);
        assert!(usable_width.is_some());
        let Some(usable_width) = usable_width else {
            return;
        };
        let Some(oversize_source) = screen_tooltip_source(Vec2::new(usable_width + 1.0, 20.0))
        else {
            return;
        };
        assert!(
            fit_tooltip_in_viewport(
                attachment,
                oversize_source,
                &context,
                tooltip::zero_origin_viewport(Vec2::splat(100.0)),
            )
            .is_none()
        );

        let outside = PanelScreenBounds::new(Vec2::new(-100.0, 45.0), Vec2::splat(10.0));
        assert!(outside.is_ok());
        let Some(outside) = outside.ok() else {
            return;
        };
        let Some(source) = screen_tooltip_source(Vec2::splat(10.0)) else {
            return;
        };
        assert!(
            fit_tooltip_in_viewport(
                attachment,
                source,
                &screen_tooltip_target(outside),
                tooltip::zero_origin_viewport(Vec2::splat(100.0)),
            )
            .is_none()
        );
    }
}
