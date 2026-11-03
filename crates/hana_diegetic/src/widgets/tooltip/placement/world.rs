use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
#[cfg(test)]
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
#[cfg(test)]
use bevy::mesh::Mesh3d;
#[cfg(test)]
use bevy::prelude::App;
use bevy::prelude::Camera;
use bevy::prelude::ChildOf;
use bevy::prelude::Commands;
use bevy::prelude::ContainsEntity;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::InheritedVisibility;
use bevy::prelude::Mut;
#[cfg(test)]
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Rect;
use bevy::prelude::Ref;
#[cfg(test)]
use bevy::prelude::ResMut;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::window::PrimaryWindow;
#[cfg(test)]
use bevy::window::WindowRef;
use hana_valence::ResolvedAnchorGeometry;

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
use super::placement_state::WorldAnchorFrame;
use super::placement_state::WorldTargetGeometry;
use super::placement_state::WorldTargetObservation;
use super::placement_state::WorldTooltipOcclusion;
use super::placement_state::WorldTooltipView;
use super::reveal_occlusion;
use super::reveal_occlusion::SceneOccluders;
use super::reveal_occlusion::SceneOcclusion;
use super::screen;
use super::viewport;
use super::viewport::TooltipProjectedFootprint;
use super::viewport::TooltipShiftBoundary;
use super::viewport::TooltipSide;
use super::viewport::TooltipSlide;
use super::world_target;
use super::world_target::CardTooltipPlacement;
use super::world_target::TargetControlRow;
use super::world_target::TooltipCameraView;
use super::world_target::WorldTargetChange;
use super::world_target::WorldTooltipCardSources;
use super::world_target::WorldTooltipPlacementArea;
use super::world_target::WorldTooltipTarget;
use crate::layout::Anchor;
#[cfg(test)]
use crate::layout::Text;
use crate::layout::Unit;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachment;
use crate::panel::PanelAttachmentAuthored;
#[cfg(test)]
use crate::panel::PanelEntity;
use crate::panel::PanelScreenBounds;
use crate::panel::PanelSpace;
use crate::panel::ResolvedScreenPanelPosition;
#[cfg(test)]
use crate::panel::World;
use crate::widgets::PanelWidget;
use crate::widgets::TOOLTIP_SHADOW_EXTENT;
use crate::widgets::WidgetAnchorRect;
use crate::widgets::WidgetOf;
use crate::widgets::tooltip;
use crate::widgets::tooltip::MaterializedTooltip;
#[cfg(test)]
use crate::widgets::tooltip::MeshFace;
#[cfg(test)]
use crate::widgets::tooltip::PrepareTooltip;
use crate::widgets::tooltip::Tooltip;
use crate::widgets::tooltip::TooltipFor;
use crate::widgets::tooltip::TooltipPresentationCamera;
#[cfg(test)]
use crate::widgets::tooltip::TooltipReadiness;
#[cfg(test)]
use crate::widgets::tooltip::TooltipTargetEntity;
use crate::widgets::tooltip::TooltipWidthConstraint;
use crate::widgets::tooltip::TooltipWidthConstraintRequest;
use crate::widgets::tooltip::declaration::TooltipPhase;
use crate::widgets::tooltip::materialization::WorldTooltipRasterFitAwaitingPlacement;

// Keep a small physical gap between a world tooltip and the surface it describes.
// Mesh targets can use meters while panels use points or pixels, so convert
// through the target's layout scale.
pub(in crate::widgets::tooltip) const WORLD_TOOLTIP_FRONT_CLEARANCE_METERS: f32 = 0.005;
/// A fitted help cap draws at about four fifths of its authored label size.
/// Using the same fraction for its physical gap keeps one drawn cap between
/// the help paint and its card edge at every camera distance.
const WORLD_TOOLTIP_HELP_CAP_GAP_LABEL_RATIO: f32 = 0.8;

fn update_world_target_geometry(
    placement: &mut Mut<'_, TooltipPlacementState>,
    geometry: &WorldTargetGeometry,
) -> WorldTargetChange {
    match &placement.world_target {
        WorldTargetObservation::Unobserved => {
            placement.world_target = WorldTargetObservation::LastObserved(Box::new(*geometry));
            WorldTargetChange::Initial
        },
        WorldTargetObservation::LastObserved(current) if **current == *geometry => {
            WorldTargetChange::Stable
        },
        WorldTargetObservation::LastObserved(_) => {
            placement.world_target = WorldTargetObservation::LastObserved(Box::new(*geometry));
            WorldTargetChange::Changed
        },
    }
}

pub(in crate::widgets::tooltip) fn resolve_world_tooltip_placements(
    mut controllers: Query<(
        Entity,
        &Tooltip,
        &TooltipFor,
        &MaterializedTooltip,
        &TooltipPhase,
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &GlobalTransform,
        Option<Ref<TooltipPresentationCamera>>,
        Option<&TooltipWidthConstraint>,
        Option<&TooltipWidthConstraintRequest>,
        &mut TooltipPlacementState,
        Option<&WorldTooltipRasterFitAwaitingPlacement>,
    )>,
    panels: Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: Query<&WidgetOf, With<PanelWidget>>,
    panel_widgets: Query<
        (Entity, &WidgetOf, &WidgetAnchorRect, &GlobalTransform),
        With<PanelWidget>,
    >,
    anchor_geometry: Query<(&ResolvedAnchorGeometry, &GlobalTransform)>,
    card_sources: WorldTooltipCardSources,
    cameras: Query<(
        Ref<Camera>,
        Ref<GlobalTransform>,
        Option<Ref<RenderTarget>>,
        Option<Ref<RenderLayers>>,
    )>,
    windows: Query<Ref<Window>>,
    camera_transforms: Query<(Ref<Transform>, Option<&ChildOf>)>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    screen_panels: Query<
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
    mut commands: Commands,
    mut occluders: SceneOccluders,
    #[cfg(test)] mut run_count: Option<ResMut<TooltipPlacementRunCount>>,
) {
    #[cfg(test)]
    if let Some(run_count) = run_count.as_mut() {
        run_count.world += 1;
    }
    for (
        controller,
        tooltip,
        tooltip_for,
        materialized,
        phase,
        panel,
        computed,
        panel_global,
        presentation_camera,
        width_constraint,
        width_request,
        mut placement,
        raster_fit,
    ) in &mut controllers
    {
        if materialized.space() != PanelSpace::World {
            continue;
        }
        if phase.is_visible() && raster_fit.is_none() {
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
        let camera_layers = presentation_camera
            .and_then(|input| cameras.get(input.camera).ok())
            .and_then(|(_, _, _, layers)| layers.map(|layers| layers.into_inner().clone()))
            .unwrap_or_else(|| RenderLayers::layer(0));
        let mut scene_occlusion =
            occluders.scene_occlusion(controller, materialized.target, camera_layers);
        let decision = keep_visible_world_tooltip_placement(
            tooltip,
            materialized,
            phase,
            panel,
            computed,
            panel_global,
            presentation_camera,
            TooltipWidthLimit::from_constraint(width_constraint),
            &mut placement,
            &panels,
            &widgets,
            &panel_widgets,
            &anchor_geometry,
            &card_sources,
            &cameras,
            &windows,
            &camera_transforms,
            &primary_window,
            &screen_panels,
            &mut scene_occlusion,
        );
        if phase.is_visible()
            && matches!(
                decision,
                TooltipPlacementDecision::Move(_) | TooltipPlacementDecision::Fits
            )
        {
            commands
                .entity(controller)
                .remove::<WorldTooltipRasterFitAwaitingPlacement>();
        }
        apply_world_placement_decision(
            controller,
            materialized,
            &mut placement,
            decision,
            &mut commands,
        );
    }
}

fn apply_world_placement_decision(
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
            commands.entity(controller).insert((
                crate::panel::PanelAttachmentAuthored::new(
                    materialized.target,
                    attachment.source_anchor(),
                    attachment.target_anchor(),
                ),
                attachment.offset(),
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

fn world_tooltip_front_attachment(
    attachment: PanelAttachment,
    unit: Unit,
    world_per_layout_unit_z: f32,
    front_clearance_meters: f32,
) -> PanelAttachment {
    let mut offset = attachment.offset().to_layout_units(unit);
    offset.z = offset
        .z
        .max(front_clearance_meters / world_per_layout_unit_z);
    panel_attachment(
        attachment.source_anchor(),
        attachment.target_anchor(),
        offset,
    )
}

struct WorldTooltipGeometrySnapshot<'a> {
    camera:                &'a Camera,
    camera_transform:      GlobalTransform,
    window:                Entity,
    viewport:              Rect,
    window_size:           Vec2,
    target:                WorldTooltipTarget,
    source:                TooltipProjectedFootprint,
    usable_width:          f32,
    window_width:          f32,
    neighbouring_controls: NeighbouringControlBounds,
}

impl WorldTooltipGeometrySnapshot<'_> {
    const fn camera_view(&self) -> TooltipCameraView<'_> {
        TooltipCameraView {
            camera:           self.camera,
            global_transform: &self.camera_transform,
            window:           self.window,
            viewport:         self.viewport,
        }
    }
}

/// Projected bounds of the target widget's neighbouring controls.
///
/// These are captured once with the rest of the placement geometry, then used
/// to keep every final, pixel-aligned candidate clear of another control.
struct NeighbouringControlBounds(Vec<PanelScreenBounds>);

/// The screen-space bounds occupied by a target and its row's controls.
#[derive(Clone, Copy)]
struct TargetRowBounds {
    min: Vec2,
    max: Vec2,
}

impl NeighbouringControlBounds {
    fn clears_tooltip(
        &self,
        inside_overlay: bool,
        pass: CardPlacementPass,
        painted: PanelScreenBounds,
        target: PanelScreenBounds,
    ) -> bool {
        if inside_overlay {
            // card_spot_fit checks the source; this fallback may cover other controls.
            true
        } else if pass.overlays_card() {
            self.clears_target_row(painted, target)
        } else {
            !self.overlaps(painted)
        }
    }

    fn with_authored_row(mut self, area: &WorldTooltipPlacementArea) -> Self {
        if let WorldTooltipPlacementArea::Card {
            target_row: TargetControlRow::CompleteRow(row),
            ..
        } = area
        {
            self.0.push(*row);
        }
        self
    }

    fn around(
        target: Entity,
        camera: &TooltipCameraView<'_>,
        widgets: &Query<
            (Entity, &WidgetOf, &WidgetAnchorRect, &GlobalTransform),
            With<PanelWidget>,
        >,
    ) -> Self {
        let Ok((_, target_owner, _, _)) = widgets.get(target) else {
            return Self(Vec::new());
        };
        let panel = target_owner.panel();
        let bounds = widgets
            .iter()
            .filter(|(entity, owner, rect, _)| {
                *entity != target && owner.panel() == panel && rect.space() == PanelSpace::World
            })
            .filter_map(|(_, _, rect, transform)| {
                let half_size = rect.size() * 0.5;
                world_target::projected_points_bounds(
                    [
                        Vec3::new(-half_size.x, half_size.y, 0.0),
                        Vec3::new(half_size.x, half_size.y, 0.0),
                        Vec3::new(half_size.x, -half_size.y, 0.0),
                        Vec3::new(-half_size.x, -half_size.y, 0.0),
                    ]
                    .map(|point| transform.transform_point(point)),
                    camera,
                )
            })
            .collect();
        Self(bounds)
    }

    fn overlaps(&self, candidate: PanelScreenBounds) -> bool {
        let candidate_min = candidate.top_left();
        let candidate_max = candidate_min + candidate.size();
        self.0.iter().any(|control| {
            let control_min = control.top_left();
            let control_max = control_min + control.size();
            candidate_max.x > control_min.x
                && candidate_min.x < control_max.x
                && candidate_max.y > control_min.y
                && candidate_min.y < control_max.y
        })
    }

    /// The target and every neighbouring control whose vertical span meets it.
    /// This captures a jack's value control without coupling the placement
    /// widget to the application's row types.
    fn target_row_bounds(&self, target: PanelScreenBounds) -> TargetRowBounds {
        let target_min = target.top_left();
        let target_max = target_min + target.size();
        let (row_min, row_max) = self
            .0
            .iter()
            .filter(|control| {
                let control_min = control.top_left().y;
                let control_max = control_min + control.size().y;
                control_max > target_min.y && control_min < target_max.y
            })
            .fold((target_min, target_max), |(minimum, maximum), control| {
                (
                    minimum.min(control.top_left()),
                    maximum.max(control.top_left() + control.size()),
                )
            });
        TargetRowBounds {
            min: row_min,
            max: row_max,
        }
    }

    /// The normal-axis move that flips `candidate` wholly past the target row.
    fn target_row_clearance_shift(
        &self,
        side: TooltipSide,
        candidate_top_left: Vec2,
        candidate_size: Vec2,
        target: PanelScreenBounds,
    ) -> Vec2 {
        let row = self.target_row_bounds(target);
        let candidate_min = candidate_top_left;
        let candidate_max = candidate_min + candidate_size;
        match side {
            TooltipSide::Above => Vec2::new(
                0.0,
                (row.min.y - CARD_HELP_CLEARANCE_PIXELS - candidate_max.y).min(0.0),
            ),
            TooltipSide::Below => Vec2::new(
                0.0,
                (row.max.y + CARD_HELP_CLEARANCE_PIXELS - candidate_min.y).max(0.0),
            ),
            TooltipSide::Right => Vec2::new(
                (row.max.x + CARD_HELP_CLEARANCE_PIXELS - candidate_min.x).max(0.0),
                0.0,
            ),
            TooltipSide::Left => Vec2::new(
                (row.min.x - CARD_HELP_CLEARANCE_PIXELS - candidate_max.x).min(0.0),
                0.0,
            ),
        }
    }

    /// Whether `candidate` leaves the target's complete control row clear.
    fn clears_target_row(&self, candidate: PanelScreenBounds, target: PanelScreenBounds) -> bool {
        let row = self.target_row_bounds(target);
        let candidate_min = candidate.top_left();
        let candidate_max = candidate_min + candidate.size();
        let far_edge = row.min - Vec2::splat(CARD_HELP_CLEARANCE_PIXELS)
            + Vec2::splat(viewport::TOOLTIP_PROJECTION_ROUNDING_PIXELS);
        let near_edge = row.max + Vec2::splat(CARD_HELP_CLEARANCE_PIXELS)
            - Vec2::splat(viewport::TOOLTIP_PROJECTION_ROUNDING_PIXELS);
        candidate_max.x <= far_edge.x
            || candidate_min.x >= near_edge.x
            || candidate_max.y <= far_edge.y
            || candidate_min.y >= near_edge.y
    }
}

fn keep_visible_world_tooltip_placement(
    tooltip: &Tooltip,
    materialized: &MaterializedTooltip,
    phase: &TooltipPhase,
    panel: &DiegeticPanel,
    computed: &ComputedDiegeticPanel,
    panel_global: &GlobalTransform,
    presentation_camera: Option<Ref<'_, TooltipPresentationCamera>>,
    width_limit: TooltipWidthLimit,
    placement: &mut Mut<'_, TooltipPlacementState>,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
    panel_widgets: &Query<
        (Entity, &WidgetOf, &WidgetAnchorRect, &GlobalTransform),
        With<PanelWidget>,
    >,
    anchor_geometry: &Query<(&ResolvedAnchorGeometry, &GlobalTransform)>,
    card_sources: &WorldTooltipCardSources<'_, '_>,
    cameras: &Query<(
        Ref<Camera>,
        Ref<GlobalTransform>,
        Option<Ref<RenderTarget>>,
        Option<Ref<RenderLayers>>,
    )>,
    windows: &Query<Ref<Window>>,
    camera_transforms: &Query<(Ref<Transform>, Option<&ChildOf>)>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    screen_panels: &Query<
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
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> TooltipPlacementDecision {
    let geometry = match resolve_world_tooltip_geometry(
        tooltip,
        materialized,
        phase,
        panel,
        panel_global,
        presentation_camera,
        width_limit,
        placement,
        panels,
        widgets,
        panel_widgets,
        anchor_geometry,
        card_sources,
        cameras,
        windows,
        camera_transforms,
        primary_window,
        screen_panels,
        scene_occlusion,
    ) {
        Ok(geometry) => geometry,
        Err(decision) => return decision,
    };
    if phase.is_visible() {
        return snap_refitted_world_tooltip(panel, computed, panel_global, placement, &geometry);
    }
    fit_world_tooltip(
        materialized,
        panel,
        computed,
        panel_global,
        width_limit,
        placement,
        &geometry,
        screen_panels,
        primary_window,
        scene_occlusion,
    )
}

fn snap_refitted_world_tooltip(
    panel: &DiegeticPanel,
    computed: &ComputedDiegeticPanel,
    global: &GlobalTransform,
    placement: &TooltipPlacementState,
    geometry: &WorldTooltipGeometrySnapshot<'_>,
) -> TooltipPlacementDecision {
    if !computed.is_solved() {
        return TooltipPlacementDecision::Unavailable;
    }
    let attachment = placement.attachment;
    let camera = geometry.camera_view();
    let target = &geometry.target;
    let shift = view_aligned_body_pixel_shift(
        panel,
        global,
        target,
        attachment,
        geometry.source.body,
        &camera,
        TooltipPixelAlignment::Nearest,
    );
    if shift == Vec2::ZERO {
        return TooltipPlacementDecision::Fits;
    }
    let offset = attachment.offset().to_layout_units(target.layout_unit);
    let Some(offset) = shifted_world_layout_offset(
        offset,
        shift,
        target.anchor(attachment.target_anchor()),
        target,
        &camera,
    ) else {
        return TooltipPlacementDecision::Unavailable;
    };
    TooltipPlacementDecision::Move(panel_attachment(
        attachment.source_anchor(),
        attachment.target_anchor(),
        offset,
    ))
}

fn resolve_world_tooltip_geometry<'a>(
    tooltip: &Tooltip,
    materialized: &MaterializedTooltip,
    phase: &TooltipPhase,
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    presentation_camera: Option<Ref<'_, TooltipPresentationCamera>>,
    width_limit: TooltipWidthLimit,
    placement: &mut Mut<'_, TooltipPlacementState>,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
    panel_widgets: &Query<
        (Entity, &WidgetOf, &WidgetAnchorRect, &GlobalTransform),
        With<PanelWidget>,
    >,
    anchor_geometry: &Query<(&ResolvedAnchorGeometry, &GlobalTransform)>,
    card_sources: &WorldTooltipCardSources<'_, '_>,
    cameras: &'a Query<(
        Ref<Camera>,
        Ref<GlobalTransform>,
        Option<Ref<RenderTarget>>,
        Option<Ref<RenderLayers>>,
    )>,
    windows: &Query<Ref<Window>>,
    camera_transforms: &Query<(Ref<Transform>, Option<&ChildOf>)>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    screen_panels: &Query<
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
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> Result<WorldTooltipGeometrySnapshot<'a>, TooltipPlacementDecision> {
    let camera_entity = presentation_camera.as_ref().map(|input| input.camera);
    let Some(initial_camera_view) = presentation_camera.and_then(|input| {
        compatible_tooltip_camera(input, materialized, cameras, windows, primary_window)
    }) else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    let camera = initial_camera_view.camera;
    let current_camera =
        camera_entity.and_then(|entity| current_camera_transform(entity, camera_transforms));
    let camera_transform = current_camera.unwrap_or(*initial_camera_view.global_transform);
    let mut camera_view = TooltipCameraView {
        camera:           initial_camera_view.camera,
        global_transform: &camera_transform,
        window:           initial_camera_view.window,
        viewport:         initial_camera_view.viewport,
    };
    let Ok(window) = windows.get(camera_view.window) else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    let window_size = Vec2::new(window.width(), window.height());
    camera_view.viewport = world_placement_area(
        &camera_view,
        materialized.target,
        window_size,
        screen_panels,
        primary_window,
    );
    let Some(target) = world_target::world_tooltip_target(
        materialized,
        tooltip
            .drawn_label_size
            .to_meters(materialized.layout_unit.meters_per_unit())
            * WORLD_TOOLTIP_HELP_CAP_GAP_LABEL_RATIO,
        WORLD_TOOLTIP_FRONT_CLEARANCE_METERS,
        &camera_view,
        panels,
        widgets,
        anchor_geometry,
        card_sources,
    ) else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    let neighbouring_controls =
        NeighbouringControlBounds::around(materialized.target, &camera_view, panel_widgets)
            .with_authored_row(&target.placement_area);
    if world_reference_hidden(
        &target,
        &camera_view,
        materialized,
        window_size,
        screen_panels,
        primary_window,
        scene_occlusion,
    ) {
        return Err(TooltipPlacementDecision::Unavailable);
    }
    if world_width_restore_needed(placement, &target, phase, width_limit, &camera_view) {
        return Err(TooltipPlacementDecision::RestoreWidth);
    }
    let Some(source) = world_source_footprint(
        materialized,
        panel,
        panel_global,
        &target,
        placement.attachment,
        &camera_view,
        panels,
        widgets,
    ) else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    let shadow_width = source.painted.size().x - source.body.size().x;
    let Some(usable_width) =
        viewport::usable_viewport_axis(camera_view.viewport.width() - shadow_width)
    else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    let Some(window_width) =
        viewport::available_window_axis(camera_view.viewport.width() - shadow_width)
    else {
        return Err(TooltipPlacementDecision::Unavailable);
    };
    Ok(WorldTooltipGeometrySnapshot {
        camera,
        camera_transform,
        window: camera_view.window,
        viewport: camera_view.viewport,
        window_size,
        target,
        source,
        usable_width,
        window_width,
        neighbouring_controls,
    })
}

fn fit_world_tooltip(
    materialized: &MaterializedTooltip,
    panel: &DiegeticPanel,
    computed: &ComputedDiegeticPanel,
    panel_global: &GlobalTransform,
    width_limit: TooltipWidthLimit,
    placement: &mut Mut<'_, TooltipPlacementState>,
    geometry: &WorldTooltipGeometrySnapshot<'_>,
    screen_panels: &Query<
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
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> TooltipPlacementDecision {
    let camera_view = geometry.camera_view();
    let target = &geometry.target;
    let source = geometry.source;
    let usable_width = geometry.usable_width;
    let layout_width = if matches!(
        target.placement_area,
        WorldTooltipPlacementArea::Card { .. }
    ) {
        geometry.window_width
    } else {
        usable_width
    };
    let layout_decision =
        world_layout_decision(panel, computed, source.body, layout_width, width_limit);
    if !matches!(layout_decision, TooltipPlacementDecision::Fits) {
        return layout_decision;
    }
    if placement.side_room_width == SideRoomWidth::Applied {
        let reflow = world_side_room_reflow(
            materialized,
            target,
            panel,
            source,
            geometry.viewport,
            usable_width,
            width_limit,
            &camera_view,
            screen_panels,
            primary_window,
            geometry.window_size,
            true,
        );
        if matches!(reflow, TooltipPlacementDecision::ConstrainSideRoomWidth(_)) {
            return reflow;
        }
    }
    let decision = projected_world_tooltip_placement(
        materialized,
        target,
        panel,
        panel_global,
        &camera_view,
        placement,
        screen_panels,
        primary_window,
        geometry.window_size,
        &geometry.neighbouring_controls,
        scene_occlusion,
    );
    if !matches!(decision, TooltipPlacementDecision::Unavailable) {
        return decision;
    }
    world_side_room_reflow(
        materialized,
        target,
        panel,
        source,
        geometry.viewport,
        usable_width,
        width_limit,
        &camera_view,
        screen_panels,
        primary_window,
        geometry.window_size,
        false,
    )
}

fn world_placement_area(
    camera: &TooltipCameraView<'_>,
    target: Entity,
    window_size: Vec2,
    screen_panels: &Query<
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
    boundary::placement_area(
        camera.window,
        target,
        target,
        TooltipScreenLayer::World(camera.camera.order),
        window_size,
        camera.viewport,
        screen_panels,
        primary_window,
    )
}

fn world_source_footprint(
    materialized: &MaterializedTooltip,
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
    camera: &TooltipCameraView<'_>,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
) -> Option<TooltipProjectedFootprint> {
    let front = world_tooltip_front_attachment(
        attachment,
        target.layout_unit,
        world_target::world_tooltip_depth_scale(materialized, panels, widgets),
        WORLD_TOOLTIP_FRONT_CLEARANCE_METERS,
    );
    projected_world_tooltip_footprint(panel, panel_global, target, front, camera)
}

fn world_width_restore_needed(
    placement: &mut Mut<'_, TooltipPlacementState>,
    target: &WorldTooltipTarget,
    phase: &TooltipPhase,
    width_limit: TooltipWidthLimit,
    camera: &TooltipCameraView<'_>,
) -> bool {
    let change = update_world_target_geometry(placement, &target.geometry());
    // Change ticks include writes that leave the view unchanged. Restoring on
    // those writes makes a constrained tooltip restart fitting every frame.
    let view_changed = TooltipPlacementState::observe_world_view(
        placement,
        WorldTooltipView {
            clip_from_view:  camera.camera.clip_from_view(),
            world_from_view: camera.global_transform.to_matrix(),
            viewport:        camera.viewport,
        },
    );
    !phase.is_visible()
        && matches!(width_limit, TooltipWidthLimit::Constrained(_))
        && (view_changed || change == WorldTargetChange::Changed)
}

fn world_reference_hidden(
    target: &WorldTooltipTarget,
    camera: &TooltipCameraView<'_>,
    materialized: &MaterializedTooltip,
    window_size: Vec2,
    screen_panels: &Query<
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
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> bool {
    !viewport::bounds_intersect_viewport(target.bounds, camera.viewport)
        || reveal_occlusion::world_target_fully_occluded(target, camera, scene_occlusion)
        || boundary::floating_panel_covers(
            camera.window,
            materialized.target,
            materialized.target,
            TooltipScreenLayer::World(camera.camera.order),
            window_size,
            target.bounds,
            screen_panels,
            primary_window,
        )
}

fn world_layout_decision(
    panel: &DiegeticPanel,
    computed: &ComputedDiegeticPanel,
    source_bounds: PanelScreenBounds,
    usable_width: f32,
    width_limit: TooltipWidthLimit,
) -> TooltipPlacementDecision {
    let width_decision = world_viewport_width_decision(
        panel.width(),
        source_bounds.size().x,
        usable_width,
        width_limit,
    );
    if !matches!(width_decision, TooltipPlacementDecision::Fits) {
        return width_decision;
    }
    if !screen::tooltip_layout_fits_panel(panel, computed) {
        return TooltipPlacementDecision::Unavailable;
    }
    TooltipPlacementDecision::Fits
}

fn world_viewport_width_decision(
    panel_width: f32,
    projected_width: f32,
    usable_width: f32,
    width_limit: TooltipWidthLimit,
) -> TooltipPlacementDecision {
    // A width constrained to fit projects back a few millionths of a pixel
    // wide; without the rounding allowance it would shrink again every frame.
    if projected_width <= usable_width + viewport::TOOLTIP_PROJECTION_ROUNDING_PIXELS {
        return TooltipPlacementDecision::Fits;
    }
    let constrained_width = panel_width * usable_width / projected_width;
    if constrained_width.is_finite()
        && constrained_width > 0.0
        && match width_limit {
            TooltipWidthLimit::Natural => true,
            TooltipWidthLimit::Constrained(current) => constrained_width < current,
        }
    {
        TooltipPlacementDecision::ConstrainWidth(constrained_width)
    } else {
        TooltipPlacementDecision::Unavailable
    }
}

fn world_side_room_reflow(
    materialized: &MaterializedTooltip,
    target: &WorldTooltipTarget,
    panel: &DiegeticPanel,
    source: TooltipProjectedFootprint,
    viewport: Rect,
    usable_width: f32,
    width_limit: TooltipWidthLimit,
    camera: &TooltipCameraView<'_>,
    screen_panels: &Query<
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
    window_size: Vec2,
    previously_applied: bool,
) -> TooltipPlacementDecision {
    let gap = materialized
        .authored_attachment
        .offset()
        .to_layout_units(target.layout_unit)
        .truncate()
        .length();
    let body = source.body;
    let projected_gap = gap * body.size().x / panel.width();
    let shadow_side = (source.painted.size().x - body.size().x) * 0.5;
    let shadow_above = body.top_left().y - source.painted.top_left().y;
    let right_start =
        target.bounds.top_left().x + target.bounds.size().x + projected_gap + shadow_side;
    let left_end = target.bounds.top_left().x - projected_gap - shadow_side;
    let right = viewport.max.x - crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN - right_start;
    let left = left_end - viewport.min.x - crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN;
    let top = (target.bounds.size().y - body.size().y).mul_add(0.5, target.bounds.top_left().y);
    let rooms = boundary::horizontal_room_before_floating_panels(
        camera.window,
        materialized.target,
        materialized.target,
        TooltipScreenLayer::World(camera.camera.order),
        window_size,
        top - shadow_above,
        source.painted.size().y,
        right_start,
        left_end,
        [right.min(usable_width), left.min(usable_width)],
        screen_panels,
        primary_window,
    );
    let available = rooms.into_iter().fold(0.0, f32::max);
    let desired = panel.width() * available / body.size().x;
    if desired.is_finite()
        && desired > 0.0
        && (previously_applied || desired < panel.width() - 0.01)
        && match width_limit {
            TooltipWidthLimit::Natural => true,
            TooltipWidthLimit::Constrained(current) => {
                (desired - current).abs() > 0.01 && (previously_applied || desired < current)
            },
        }
    {
        TooltipPlacementDecision::ConstrainSideRoomWidth(desired)
    } else {
        TooltipPlacementDecision::Unavailable
    }
}

fn projected_world_tooltip_placement(
    materialized: &MaterializedTooltip,
    target: &WorldTooltipTarget,
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    camera: &TooltipCameraView<'_>,
    placement: &mut Mut<'_, TooltipPlacementState>,
    screen_panels: &Query<
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
    window_size: Vec2,
    neighbouring_controls: &NeighbouringControlBounds,
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> TooltipPlacementDecision {
    let (authored, authored_layout, preferred, passes) =
        world_placement_context(materialized, target);
    for &pass in passes {
        let mut spots = Vec::new();
        for (side, slide) in slides_for_placement_pass(preferred, target, camera.viewport, pass) {
            let (source_anchor, target_anchor, layout_offset) =
                world_candidate_layout(authored_layout, authored, preferred, side);
            let attachment = panel_attachment(source_anchor, target_anchor, layout_offset);
            let Some(natural) =
                projected_world_tooltip_footprint(panel, panel_global, target, attachment, camera)
            else {
                continue;
            };
            let target_anchor_frame = target.anchor(target_anchor);
            let shifts = placement_candidate_shifts(
                &target.placement_area,
                pass,
                side,
                slide,
                natural,
                target.bounds.top_left() + target.bounds.size() * 0.5,
                target.bounds,
                camera.viewport,
            );
            for shift in shifts {
                let candidate = WorldTooltipCandidate {
                    source_anchor,
                    target_anchor,
                    layout_offset,
                    target_anchor_frame,
                    natural,
                    pass,
                    side,
                };
                if let WorldSpotEvaluation::Fits(spot) = world_help_spot_for_shift(
                    candidate,
                    shift,
                    panel,
                    panel_global,
                    target,
                    camera,
                    neighbouring_controls,
                ) {
                    spots.push(spot);
                }
            }
        }
        // Spots clear of the border first, then fewest covered regions; the
        // stable sort keeps side and shift order on ties.
        spots.sort_by_key(|spot| (spot.crosses_border, spot.covered));
        if let Some(spot) = spots.into_iter().find(|spot| {
            world_tooltip_candidate_is_visible(
                panel,
                panel_global,
                target,
                spot.attachment,
                spot.painted,
                spot.occlusion,
                camera,
                materialized.target,
                window_size,
                screen_panels,
                primary_window,
                scene_occlusion,
            )
        }) {
            if placement.world_occlusion != spot.occlusion {
                placement.world_occlusion = spot.occlusion;
            }
            let current_attachment = placement.attachment;
            return if current_attachment == spot.attachment {
                TooltipPlacementDecision::Fits
            } else {
                TooltipPlacementDecision::Move(spot.attachment)
            };
        }
    }
    TooltipPlacementDecision::Unavailable
}

fn world_help_spot_for_shift(
    candidate: WorldTooltipCandidate,
    shift: Vec2,
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    camera: &TooltipCameraView<'_>,
    neighbouring_controls: &NeighbouringControlBounds,
) -> WorldSpotEvaluation {
    let shift = world_help_clearance_shift(&candidate, shift, target, neighbouring_controls);
    let WorldTooltipCandidate {
        source_anchor,
        target_anchor,
        layout_offset,
        target_anchor_frame,
        natural,
        pass,
        side,
    } = candidate;
    if matches!(
        card_spot_fit(
            &target.placement_area,
            pass,
            side,
            natural,
            shift,
            target.bounds
        ),
        CardSpotFit::Refused
    ) {
        return WorldSpotEvaluation::Refused;
    }
    let Some(layout_offset) =
        shifted_world_layout_offset(layout_offset, shift, target_anchor_frame, target, camera)
    else {
        return WorldSpotEvaluation::Refused;
    };
    let attachment = panel_attachment(source_anchor, target_anchor, layout_offset);
    let Some(placed) =
        projected_world_tooltip_footprint(panel, panel_global, target, attachment, camera)
    else {
        return WorldSpotEvaluation::Refused;
    };
    let alignment = if pass.overlays_card() {
        TooltipPixelAlignment::ClearOfTargetRow(side)
    } else {
        TooltipPixelAlignment::Nearest
    };
    let pixel_shift = view_aligned_body_pixel_shift(
        panel,
        panel_global,
        target,
        attachment,
        placed.body,
        camera,
        alignment,
    );
    let layout_offset = if pixel_shift == Vec2::ZERO {
        layout_offset
    } else {
        let Some(snapped) = shifted_world_layout_offset(
            layout_offset,
            pixel_shift,
            target_anchor_frame,
            target,
            camera,
        ) else {
            return WorldSpotEvaluation::Refused;
        };
        snapped
    };
    let attachment = panel_attachment(source_anchor, target_anchor, layout_offset);
    let Some(placed) =
        projected_world_tooltip_footprint(panel, panel_global, target, attachment, camera)
    else {
        return WorldSpotEvaluation::Refused;
    };
    checked_world_help_spot(
        pass,
        side,
        attachment,
        placed,
        target,
        neighbouring_controls,
    )
}

/// The shadow belongs to the tooltip. Screen and world placement share this rule.
pub(super) fn tooltip_clears_source(painted: PanelScreenBounds, source: PanelScreenBounds) -> bool {
    let painted_min = painted.top_left();
    let painted_max = painted_min + painted.size();
    let source_min = source.top_left();
    let source_max = source_min + source.size();
    !(painted_max.cmpgt(source_min).all() && painted_min.cmplt(source_max).all())
}

/// Move the full painted footprint past the source on the selected fallback side.
pub(super) fn tooltip_source_clearance_shift(
    side: TooltipSide,
    painted: PanelScreenBounds,
    shift: Vec2,
    source: PanelScreenBounds,
) -> Vec2 {
    if let Ok(shifted) = PanelScreenBounds::new(painted.top_left() + shift, painted.size())
        && tooltip_clears_source(shifted, source)
    {
        return Vec2::ZERO;
    }
    NeighbouringControlBounds(Vec::new()).target_row_clearance_shift(
        side,
        painted.top_left() + shift,
        painted.size(),
        source,
    )
}

fn world_help_clearance_shift(
    candidate: &WorldTooltipCandidate,
    mut shift: Vec2,
    target: &WorldTooltipTarget,
    neighbours: &NeighbouringControlBounds,
) -> Vec2 {
    if candidate.pass.overlays_card() {
        shift += neighbours.target_row_clearance_shift(
            candidate.side,
            candidate.natural.painted.top_left() + shift,
            candidate.natural.painted.size(),
            target.bounds,
        );
    } else if !matches!(candidate.pass, CardPlacementPass::Authored) {
        shift += tooltip_source_clearance_shift(
            candidate.side,
            candidate.natural.painted,
            shift,
            target.bounds,
        );
    }
    shift
}

fn checked_world_help_spot(
    pass: CardPlacementPass,
    side: TooltipSide,
    attachment: PanelAttachment,
    placed: TooltipProjectedFootprint,
    target: &WorldTooltipTarget,
    neighbouring_controls: &NeighbouringControlBounds,
) -> WorldSpotEvaluation {
    if matches!(pass, CardPlacementPass::Authored)
        && !target.placement_area.clears_border(placed.painted)
    {
        return WorldSpotEvaluation::Refused;
    }
    if !tooltip_clears_source(placed.painted, target.bounds) {
        return WorldSpotEvaluation::Refused;
    }
    let inside_overlay = inside_card_overlay(&target.placement_area, pass);
    if !neighbouring_controls.clears_tooltip(inside_overlay, pass, placed.painted, target.bounds) {
        return WorldSpotEvaluation::Refused;
    }
    let CardSpotFit::Covers(covered) = card_spot_fit(
        &target.placement_area,
        pass,
        side,
        placed,
        Vec2::ZERO,
        target.bounds,
    ) else {
        return WorldSpotEvaluation::Refused;
    };
    WorldSpotEvaluation::Fits(HelpSpot {
        attachment,
        painted: placed.painted,
        crosses_border: !target.placement_area.clears_border(placed.painted),
        covered,
        occlusion: if inside_overlay {
            WorldTooltipOcclusion::CardOverlay
        } else {
            pass.world_occlusion()
        },
    })
}

/// The inside-only fallback may cover neighbouring controls, but not its source.
const fn inside_card_overlay(area: &WorldTooltipPlacementArea, pass: CardPlacementPass) -> bool {
    matches!(
        (area, pass),
        (
            WorldTooltipPlacementArea::Card {
                placement: world_target::CardTooltipPlacement::InsideOnly,
                ..
            },
            CardPlacementPass::InsideOverlapping
        )
    )
}

/// Pixel alignment that preserves a flipped overlay's row clearance.
#[derive(Clone, Copy)]
enum TooltipPixelAlignment {
    Nearest,
    ClearOfTargetRow(TooltipSide),
}

impl TooltipPixelAlignment {
    fn aligned(self, point: Vec2) -> Vec2 {
        let mut aligned = point.round();
        if let Self::ClearOfTargetRow(side) = self {
            match side {
                TooltipSide::Above => aligned.y = point.y.floor(),
                TooltipSide::Below => aligned.y = point.y.ceil(),
                TooltipSide::Left => aligned.x = point.x.floor(),
                TooltipSide::Right => aligned.x = point.x.ceil(),
            }
        }
        aligned
    }
}

/// The final screen-space move for a front-facing body whose size was rounded
/// to physical pixels during materialization. The shift is converted back to
/// the target plane before any fit or occlusion decision is made.
fn view_aligned_body_pixel_shift(
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
    body: PanelScreenBounds,
    camera: &TooltipCameraView<'_>,
    alignment: TooltipPixelAlignment,
) -> Vec2 {
    let Some(corners) = world_tooltip_corners(panel, panel_global, target, attachment) else {
        return Vec2::ZERO;
    };
    let [top_left, top_right, _, bottom_left] = corners;
    let normal = (top_right - top_left)
        .cross(bottom_left - top_left)
        .normalize();
    if !normal.is_finite()
        || normal
            .dot(camera.global_transform.forward().as_vec3())
            .abs()
            < 0.9999
    {
        return Vec2::ZERO;
    }
    let (Some(top_left), Some(top_right), Some(bottom_left)) = (
        world_target::project_world_point(top_left, camera),
        world_target::project_world_point(top_right, camera),
        world_target::project_world_point(bottom_left, camera),
    ) else {
        return Vec2::ZERO;
    };
    if (top_right.y - top_left.y).abs() > 0.01 || (bottom_left.x - top_left.x).abs() > 0.01 {
        return Vec2::ZERO;
    }
    let (Some(physical), Some(logical)) = (
        camera.camera.physical_viewport_size(),
        camera.camera.logical_viewport_size(),
    ) else {
        return Vec2::ZERO;
    };
    let physical_per_logical = physical.as_vec2() / logical;
    let physical_per_layout =
        body.size() * physical_per_logical / Vec2::new(panel.width(), panel.height());
    if !physical_per_layout.is_finite() || physical_per_layout.min_element() < 1.0 {
        return Vec2::ZERO;
    }
    alignment.aligned(body.top_left() * physical_per_logical) / physical_per_logical
        - body.top_left()
}

fn world_placement_context(
    materialized: &MaterializedTooltip,
    target: &WorldTooltipTarget,
) -> (
    PanelAttachment,
    Vec3,
    TooltipSide,
    &'static [CardPlacementPass],
) {
    let authored = world_tooltip_front_attachment(
        materialized.authored_attachment,
        target.layout_unit,
        target.world_per_layout_unit_z,
        WORLD_TOOLTIP_FRONT_CLEARANCE_METERS,
    );
    let authored_layout = authored.offset().to_layout_units(target.layout_unit);
    let preferred = viewport::preferred_side(authored, authored_layout.truncate());
    let passes = card_placement_passes(&target.placement_area);
    (authored, authored_layout, preferred, passes)
}

const fn card_placement_passes(area: &WorldTooltipPlacementArea) -> &'static [CardPlacementPass] {
    match area {
        WorldTooltipPlacementArea::Viewport(_) => {
            &[CardPlacementPass::Authored, CardPlacementPass::Inside]
        },
        WorldTooltipPlacementArea::Card {
            placement: CardTooltipPlacement::InsideOnly,
            ..
        } => &[
            CardPlacementPass::Authored,
            CardPlacementPass::Inside,
            CardPlacementPass::InsideOverlapping,
        ],
        WorldTooltipPlacementArea::Card { .. } => &[
            CardPlacementPass::Authored,
            CardPlacementPass::Inside,
            CardPlacementPass::Outside,
            CardPlacementPass::InsideOverlapping,
            CardPlacementPass::OverlayShiftedIntoWindow,
        ],
    }
}

fn world_tooltip_candidate_is_visible(
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
    painted: PanelScreenBounds,
    occlusion: WorldTooltipOcclusion,
    camera: &TooltipCameraView<'_>,
    tooltip_target: Entity,
    window_size: Vec2,
    screen_panels: &Query<
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
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> bool {
    let fits_viewport = if occlusion == WorldTooltipOcclusion::CardOverlay {
        viewport::bounds_fit_window(painted.top_left(), painted.size(), camera.viewport)
    } else {
        viewport::bounds_fit_viewport(painted.top_left(), painted.size(), camera.viewport)
    };
    let clears_floating = !boundary::floating_panel_overlaps(
        camera.window,
        tooltip_target,
        tooltip_target,
        TooltipScreenLayer::World(camera.camera.order),
        window_size,
        painted,
        screen_panels,
        primary_window,
    );
    let clears_scene = occlusion == WorldTooltipOcclusion::CardOverlay
        || !tooltip_quad_occluded(
            panel,
            panel_global,
            target,
            attachment,
            camera,
            scene_occlusion,
        );
    fits_viewport && clears_floating && clears_scene
}

/// Where card help may go, tried in order until one spot shows.
#[derive(Clone, Copy)]
enum CardPlacementPass {
    /// The authored anchors and offset, without a shift.
    Authored,
    /// Inside the card beside its jack, clear of every occupied region.
    Inside,
    /// Wholly outside the card: at the jack's row on the nearer side, then the
    /// farther side, then below and above at the jack's column.
    Outside,
    /// Inside the card beside its jack, covering the fewest occupied regions, so
    /// help still shows when no clear spot fits the viewport.
    InsideOverlapping,
    /// Flipped clear of the hovered control's complete row, then shifted wholly
    /// inside the window over the card when no clear outside spot exists.
    ///
    /// This is the established flip-then-shift fallback: Floating UI's `flip`
    /// and `shift` middleware, KDE Plasma/Qt tooltips, and macOS help tags all
    /// prefer another side and then keep the full overlay inside its window.
    /// The help keeps its natural size and shadow; only its position changes.
    OverlayShiftedIntoWindow,
}

impl CardPlacementPass {
    const fn overlays_card(self) -> bool { matches!(self, Self::OverlayShiftedIntoWindow) }

    const fn world_occlusion(self) -> WorldTooltipOcclusion {
        if self.overlays_card() {
            WorldTooltipOcclusion::CardOverlay
        } else {
            WorldTooltipOcclusion::SceneBound
        }
    }
}

/// Whether a help box may stand at a spot under its pass's rule.
#[derive(Clone, Copy)]
enum CardSpotFit {
    Refused,
    /// Allowed, covering this many occupied card regions.
    Covers(usize),
}

/// One spot a help box may take, with what it paints and the occupied card
/// regions it covers.
struct HelpSpot {
    attachment:     PanelAttachment,
    painted:        PanelScreenBounds,
    /// Whether it lies across the border it keeps clear of, kept only when no
    /// clear spot shows.
    crosses_border: bool,
    covered:        usize,
    occlusion:      WorldTooltipOcclusion,
}

/// Geometry shared by the screen shifts evaluated for one side of a world tooltip.
struct WorldTooltipCandidate {
    source_anchor:       Anchor,
    target_anchor:       Anchor,
    layout_offset:       Vec3,
    target_anchor_frame: WorldAnchorFrame,
    natural:             TooltipProjectedFootprint,
    pass:                CardPlacementPass,
    side:                TooltipSide,
}

enum WorldSpotEvaluation {
    Refused,
    Fits(HelpSpot),
}

const CARD_HELP_CLEARANCE_PIXELS: f32 = 2.0;

/// The sides tried in `pass`, in order, each with how the help slides there.
/// Card help slides by its card's rule, so its slide is the placement area's.
fn slides_for_placement_pass(
    preferred: TooltipSide,
    target: &WorldTooltipTarget,
    viewport: Rect,
    pass: CardPlacementPass,
) -> Vec<(TooltipSide, TooltipSlide)> {
    if matches!(pass, CardPlacementPass::Authored) {
        return vec![(
            preferred,
            TooltipSlide::Beside(TooltipShiftBoundary::PlacementAreaOnly),
        )];
    }
    let sides = match (&target.placement_area, pass) {
        (WorldTooltipPlacementArea::Viewport(bounds), _) => {
            let sides = viewport::tooltip_side_order(preferred, target.bounds, viewport);
            return bounds.slide_order(sides, target.bounds).collect();
        },
        (WorldTooltipPlacementArea::Card { bounds, .. }, CardPlacementPass::Outside) => {
            let nearer_horizontal =
                nearer_horizontal_outside_side(target.bounds, screen_rect(*bounds));
            let farther_horizontal = if nearer_horizontal == TooltipSide::Right {
                TooltipSide::Left
            } else {
                TooltipSide::Right
            };
            let nearer_vertical = nearer_vertical_outside_side(target.bounds, screen_rect(*bounds));
            let farther_vertical = if nearer_vertical == TooltipSide::Above {
                TooltipSide::Below
            } else {
                TooltipSide::Above
            };
            [
                nearer_horizontal,
                farther_horizontal,
                nearer_vertical,
                farther_vertical,
            ]
        },
        (WorldTooltipPlacementArea::Card { .. }, _) => {
            viewport::tooltip_side_order(preferred, target.bounds, viewport)
        },
    };
    sides
        .into_iter()
        .map(|side| {
            (
                side,
                TooltipSlide::Beside(TooltipShiftBoundary::PlacementAreaOnly),
            )
        })
        .collect()
}

/// The card's left or right side, whichever is nearer the jack; a tie goes right.
fn nearer_horizontal_outside_side(target: PanelScreenBounds, card: Rect) -> TooltipSide {
    let centre = target.size().x.mul_add(0.5, target.top_left().x);
    if card.max.x - centre <= centre - card.min.x {
        TooltipSide::Right
    } else {
        TooltipSide::Left
    }
}

/// The card's top or bottom side, whichever is nearer the jack; a tie goes above.
fn nearer_vertical_outside_side(target: PanelScreenBounds, card: Rect) -> TooltipSide {
    let centre = target.size().y.mul_add(0.5, target.top_left().y);
    if centre - card.min.y <= card.max.y - centre {
        TooltipSide::Above
    } else {
        TooltipSide::Below
    }
}

fn screen_rect(bounds: PanelScreenBounds) -> Rect {
    Rect::from_corners(bounds.top_left(), bounds.top_left() + bounds.size())
}

fn placement_candidate_shifts(
    area: &WorldTooltipPlacementArea,
    pass: CardPlacementPass,
    side: TooltipSide,
    tooltip_slide: TooltipSlide,
    natural: TooltipProjectedFootprint,
    target_point: Vec2,
    target: PanelScreenBounds,
    viewport: Rect,
) -> Vec<Vec2> {
    let body = natural.body;
    let alignment = target.top_left() + target.size() * 0.5 - body.top_left() - body.size() * 0.5;
    match (area, pass) {
        (_, CardPlacementPass::Authored) => vec![Vec2::ZERO],
        (WorldTooltipPlacementArea::Viewport(_), _) => {
            vec![tooltip_slide.shift(side, natural, target_point, viewport)]
        },
        (
            WorldTooltipPlacementArea::Card {
                bounds, occupied, ..
            },
            CardPlacementPass::Inside | CardPlacementPass::InsideOverlapping,
        ) => inside_card_shifts(
            side,
            natural,
            target_point,
            screen_rect(*bounds),
            occupied,
            alignment,
        ),
        (
            WorldTooltipPlacementArea::Card {
                outside_bounds,
                gap_pixels,
                ..
            },
            CardPlacementPass::Outside,
        ) => vec![outside_card_aligned_shift(
            side,
            natural.painted,
            screen_rect(*outside_bounds),
            *gap_pixels,
            alignment,
            viewport,
        )],
        (WorldTooltipPlacementArea::Card { .. }, CardPlacementPass::OverlayShiftedIntoWindow) => {
            let along = along_edge_inside_window(side, natural.painted, alignment, viewport);
            vec![if side.is_vertical() {
                Vec2::new(along, 0.0)
            } else {
                Vec2::new(0.0, along)
            }]
        },
    }
}

/// Shifts for help inside `card`, nearest first.
///
/// The help keeps the jack's row or column and moves along it: unmoved, slid
/// along its edge within the card, fitted inside the card's clearance, and
/// past each side of every `occupied` region.
fn inside_card_shifts(
    side: TooltipSide,
    natural: TooltipProjectedFootprint,
    target_point: Vec2,
    card: Rect,
    occupied: &[PanelScreenBounds],
    alignment: Vec2,
) -> Vec<Vec2> {
    let painted = natural.painted;
    let edge_shift = viewport::limited_along_edge_shift(
        side,
        natural,
        target_point,
        TooltipShiftBoundary::PlacementAreaOnly,
        card,
    );
    let horizontal = matches!(side, TooltipSide::Right | TooltipSide::Left);
    let axis = |point: Vec2| if horizontal { point.x } else { point.y };
    let painted_start = axis(painted.top_left());
    let painted_size = axis(painted.size());
    let (card_start, card_end) = if horizontal {
        (card.min.x, card.max.x)
    } else {
        (card.min.y, card.max.y)
    };
    let mut offsets = vec![
        0.0,
        axis(edge_shift),
        viewport::containment_shift(
            painted_start,
            painted_size,
            card_start + CARD_HELP_CLEARANCE_PIXELS,
            card_end - CARD_HELP_CLEARANCE_PIXELS,
        ),
    ];
    for region in occupied {
        offsets.extend([
            axis(region.top_left()) - painted_start - painted_size - CARD_HELP_CLEARANCE_PIXELS,
            axis(region.top_left() + region.size()) - painted_start + CARD_HELP_CLEARANCE_PIXELS,
        ]);
    }
    sort_dedup_shifts(&mut offsets);
    let mut shifts: Vec<_> = offsets
        .into_iter()
        .map(|offset| {
            if horizontal {
                Vec2::new(offset, alignment.y)
            } else {
                Vec2::new(alignment.x, offset)
            }
        })
        .collect();
    shifts.sort_by(|left, right| left.length_squared().total_cmp(&right.length_squared()));
    shifts
}

/// The shift for help outside `card` on `side`, centred on the jack's row or
/// column, then slid along the card edge into the viewport.
fn outside_card_aligned_shift(
    side: TooltipSide,
    painted: PanelScreenBounds,
    card: Rect,
    gap_pixels: f32,
    alignment: Vec2,
    viewport: Rect,
) -> Vec2 {
    let mut shift = outside_card_shift(side, painted, card, gap_pixels);
    let along = along_edge_in_viewport(side, painted, alignment, viewport);
    if side.is_vertical() {
        shift.x = along;
    } else {
        shift.y = along;
    }
    shift
}

/// The shift along `side`'s edge that centres help on its jack's row or column
/// by `alignment`, then slides it until `painted`, the help's drawn extent with
/// its shadow, stands one margin inside `viewport`.
fn along_edge_in_viewport(
    side: TooltipSide,
    painted: PanelScreenBounds,
    alignment: Vec2,
    viewport: Rect,
) -> f32 {
    let margin = crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN;
    let axis = |point: Vec2| if side.is_vertical() { point.x } else { point.y };
    axis(alignment)
        + viewport::containment_shift(
            axis(painted.top_left()) + axis(alignment),
            axis(painted.size()),
            axis(viewport.min) + margin,
            axis(viewport.max) - margin,
        )
}

/// Centres help on its jack's row or column, then shifts its complete painted
/// extent, shadow included, just far enough to remain inside the window.
fn along_edge_inside_window(
    side: TooltipSide,
    painted: PanelScreenBounds,
    alignment: Vec2,
    viewport: Rect,
) -> f32 {
    let axis = |point: Vec2| if side.is_vertical() { point.x } else { point.y };
    axis(alignment)
        + viewport::containment_shift(
            axis(painted.top_left()) + axis(alignment),
            axis(painted.size()),
            axis(viewport.min),
            axis(viewport.max),
        )
}

fn sort_dedup_shifts(shifts: &mut Vec<f32>) {
    shifts.sort_by(f32::total_cmp);
    shifts.dedup_by(|left, right| (*left - *right).abs() < 0.01);
}

/// Inside help sits on its jack's row or column, beyond the jack on `side`.
fn help_beside_jack(
    side: TooltipSide,
    top_left: Vec2,
    help_size: Vec2,
    target: PanelScreenBounds,
) -> bool {
    let bottom_right = top_left + help_size;
    let target_min = target.top_left();
    let target_max = target_min + target.size();
    let help_centre = top_left + help_size * 0.5;
    let target_centre = Vec2::new(
        f32::midpoint(target_min.x, target_max.x),
        f32::midpoint(target_min.y, target_max.y),
    );
    let horizontal_overlap = (help_centre.x - target_centre.x).abs() <= 1.0;
    let vertical_overlap = (help_centre.y - target_centre.y).abs() <= 1.0;
    match side {
        TooltipSide::Above => {
            bottom_right.y <= target_min.y - CARD_HELP_CLEARANCE_PIXELS && horizontal_overlap
        },
        TooltipSide::Right => {
            top_left.x >= target_max.x + CARD_HELP_CLEARANCE_PIXELS && vertical_overlap
        },
        TooltipSide::Below => {
            top_left.y >= target_max.y + CARD_HELP_CLEARANCE_PIXELS && horizontal_overlap
        },
        TooltipSide::Left => {
            bottom_right.x <= target_min.x - CARD_HELP_CLEARANCE_PIXELS && vertical_overlap
        },
    }
}

/// Help stands clear of its jack on `side`, wherever it sits along that side.
fn help_clears_jack(
    side: TooltipSide,
    top_left: Vec2,
    help_size: Vec2,
    target: PanelScreenBounds,
) -> bool {
    let bottom_right = top_left + help_size;
    let target_min = target.top_left();
    let target_max = target_min + target.size();
    match side {
        TooltipSide::Above => bottom_right.y <= target_min.y - CARD_HELP_CLEARANCE_PIXELS + 1.0,
        TooltipSide::Right => top_left.x >= target_max.x + CARD_HELP_CLEARANCE_PIXELS - 1.0,
        TooltipSide::Below => top_left.y >= target_max.y + CARD_HELP_CLEARANCE_PIXELS - 1.0,
        TooltipSide::Left => bottom_right.x <= target_min.x - CARD_HELP_CLEARANCE_PIXELS + 1.0,
    }
}

fn outside_card_shift(side: TooltipSide, help: PanelScreenBounds, card: Rect, gap: f32) -> Vec2 {
    let top_left = help.top_left();
    let bottom_right = top_left + help.size();
    match side {
        TooltipSide::Above => Vec2::new(0.0, card.min.y - gap - bottom_right.y),
        TooltipSide::Right => Vec2::new(card.max.x + gap - top_left.x, 0.0),
        TooltipSide::Below => Vec2::new(0.0, card.max.y + gap - top_left.y),
        TooltipSide::Left => Vec2::new(card.min.x - gap - bottom_right.x, 0.0),
    }
}

fn outside_card_clears(
    side: TooltipSide,
    top_left: Vec2,
    help_size: Vec2,
    card: Rect,
    gap: f32,
) -> bool {
    let bottom_right = top_left + help_size;
    match side {
        TooltipSide::Above => bottom_right.y <= card.min.y - gap + 1.0,
        TooltipSide::Right => top_left.x >= card.max.x + gap - 1.0,
        TooltipSide::Below => top_left.y >= card.max.y + gap - 1.0,
        TooltipSide::Left => bottom_right.x <= card.min.x - gap + 1.0,
    }
}

/// Whether `footprint`, moved by `shift`, may stand under `pass`'s rule.
/// Outside, the painted bounds clear the card. Inside, the body keeps beside
/// its jack and the painted bounds stay inside the card, so the shadow never
/// spills past the card's edge; the painted bounds also count the occupied
/// regions covered, so a shadow over a jack or label counts as covering it.
fn card_spot_fit(
    area: &WorldTooltipPlacementArea,
    pass: CardPlacementPass,
    side: TooltipSide,
    footprint: TooltipProjectedFootprint,
    shift: Vec2,
    target: PanelScreenBounds,
) -> CardSpotFit {
    let WorldTooltipPlacementArea::Card {
        bounds,
        occupied,
        target_row,
        ..
    } = area
    else {
        return CardSpotFit::Covers(0);
    };
    let painted_min = footprint.painted.top_left() + shift;
    let painted_size = footprint.painted.size();
    let painted_max = painted_min + painted_size;
    if let TargetControlRow::CompleteRow(row) = target_row {
        let clearance = Vec2::splat(CARD_HELP_CLEARANCE_PIXELS);
        let row_min = row.top_left() - clearance;
        let row_max = row.top_left() + row.size() + clearance;
        if painted_max.x > row_min.x
            && painted_min.x < row_max.x
            && painted_max.y > row_min.y
            && painted_min.y < row_max.y
        {
            return CardSpotFit::Refused;
        }
    }
    let allowed = match pass {
        CardPlacementPass::Authored => {
            painted_min.cmpge(bounds.top_left()).all()
                && painted_max.cmple(bounds.top_left() + bounds.size()).all()
        },
        CardPlacementPass::Outside => {
            outside_card_clears(side, painted_min, painted_size, screen_rect(*bounds), 0.0)
        },
        CardPlacementPass::Inside | CardPlacementPass::InsideOverlapping => {
            let clearance = Vec2::splat(CARD_HELP_CLEARANCE_PIXELS);
            help_beside_jack(
                side,
                footprint.body.top_left() + shift,
                footprint.body.size(),
                target,
            ) && painted_min.cmpge(bounds.top_left() + clearance).all()
                && painted_max
                    .cmple(bounds.top_left() + bounds.size() - clearance)
                    .all()
        },
        CardPlacementPass::OverlayShiftedIntoWindow => help_clears_jack(
            side,
            footprint.body.top_left() + shift,
            footprint.body.size(),
            target,
        ),
    };
    if !allowed {
        return CardSpotFit::Refused;
    }
    if matches!(pass, CardPlacementPass::Outside) {
        return CardSpotFit::Covers(0);
    }
    let covered = occupied
        .iter()
        .filter(|region| {
            let region_min = region.top_left() - Vec2::splat(CARD_HELP_CLEARANCE_PIXELS);
            let region_max =
                region.top_left() + region.size() + Vec2::splat(CARD_HELP_CLEARANCE_PIXELS);
            painted_max.x > region_min.x
                && painted_min.x < region_max.x
                && painted_max.y > region_min.y
                && painted_min.y < region_max.y
        })
        .count();
    match (pass, covered) {
        (CardPlacementPass::Inside | CardPlacementPass::Authored, 1..) => CardSpotFit::Refused,
        _ => CardSpotFit::Covers(covered),
    }
}

fn world_candidate_layout(
    authored_layout: Vec3,
    authored: PanelAttachment,
    preferred: TooltipSide,
    side: TooltipSide,
) -> (Anchor, Anchor, Vec3) {
    if side == preferred {
        return (
            authored.source_anchor(),
            authored.target_anchor(),
            authored_layout,
        );
    }
    let (source_anchor, target_anchor) = side.anchors();
    let gap = preferred.gap_kept_by(authored_layout.truncate());
    (
        source_anchor,
        target_anchor,
        side.gap_offset(gap).extend(authored_layout.z),
    )
}

fn panel_attachment(
    source_anchor: Anchor,
    target_anchor: Anchor,
    layout_offset: Vec3,
) -> PanelAttachment {
    PanelAttachment::new(source_anchor, target_anchor).with_offset(
        PanelAnchorOffset::new(layout_offset.x, layout_offset.y).with_z(layout_offset.z),
    )
}

fn current_camera_transform(
    camera: Entity,
    transforms: &Query<(Ref<Transform>, Option<&ChildOf>)>,
) -> Option<GlobalTransform> {
    let mut entity = camera;
    let mut chain = Vec::new();
    loop {
        let (transform, parent) = transforms.get(entity).ok()?;
        chain.push(*transform);
        let Some(parent) = parent else {
            break;
        };
        entity = parent.parent();
    }
    let global = chain
        .into_iter()
        .rev()
        .fold(GlobalTransform::IDENTITY, |global, transform| {
            global.mul_transform(transform)
        });
    Some(global)
}

fn compatible_tooltip_camera<'a>(
    input: Ref<'_, TooltipPresentationCamera>,
    materialized: &MaterializedTooltip,
    cameras: &'a Query<(
        Ref<Camera>,
        Ref<GlobalTransform>,
        Option<Ref<RenderTarget>>,
        Option<Ref<RenderLayers>>,
    )>,
    windows: &Query<Ref<Window>>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<TooltipCameraView<'a>> {
    let (camera, global_transform, render_target, camera_layers) =
        cameras.get(input.camera).ok()?;
    let camera = camera.into_inner();
    let global_transform = global_transform.into_inner();
    if !camera.is_active {
        return None;
    }
    let primary = primary_window.single().ok();
    let window_entity = match render_target?.into_inner().normalize(primary)? {
        NormalizedRenderTarget::Window(window) => window.entity(),
        NormalizedRenderTarget::Image(_)
        | NormalizedRenderTarget::TextureView(_)
        | NormalizedRenderTarget::None { .. } => return None,
    };
    let window = windows.get(window_entity).ok()?;
    let camera_layers = camera_layers.map_or_else(
        || RenderLayers::layer(0),
        |render_layers| render_layers.into_inner().clone(),
    );
    if !camera_layers.intersects(&materialized.render_layers) {
        return None;
    }
    let viewport = camera.logical_viewport_rect().unwrap_or_else(|| {
        tooltip::zero_origin_viewport(Vec2::new(window.width(), window.height()))
    });
    viewport::valid_screen_size(viewport.size()).then_some(TooltipCameraView {
        camera,
        global_transform,
        window: window_entity,
        viewport,
    })
}

/// World corners of a placed tooltip's body.
fn world_tooltip_corners(
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
) -> Option<[Vec3; 4]> {
    placed_world_tooltip_corners(panel, panel_global, target, attachment, [Vec3::ZERO; 4])
}

/// World corners of everything a placed tooltip paints: its body grown in its
/// own plane by the shadow's reach, so a tilted tooltip's shadow tilts with it.
fn world_tooltip_painted_corners(
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
) -> Option<[Vec3; 4]> {
    let reach = TOOLTIP_SHADOW_EXTENT.painted_reach(panel.points_to_world());
    placed_world_tooltip_corners(
        panel,
        panel_global,
        target,
        attachment,
        [
            Vec3::new(-reach.sides, reach.above, 0.0),
            Vec3::new(reach.sides, reach.above, 0.0),
            Vec3::new(reach.sides, -reach.below, 0.0),
            Vec3::new(-reach.sides, -reach.below, 0.0),
        ],
    )
}

/// World corners of a placed tooltip, top left first and clockwise, each moved
/// by its panel-local `outsets` entry.
fn placed_world_tooltip_corners(
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
    outsets: [Vec3; 4],
) -> Option<[Vec3; 4]> {
    let source_scale = panel_global.to_scale_rotation_translation().0;
    if !source_scale.is_finite() {
        return None;
    }
    let target_anchor = target.anchor(attachment.target_anchor());
    let layout_offset = attachment.offset().to_layout_units(target.layout_unit);
    let source_anchor = world_target::panel_local_anchor(panel, attachment.source_anchor());
    let target_position = target_anchor.position
        + target_anchor.right * (layout_offset.x * target.world_per_layout_unit.x)
        - target_anchor.up * (layout_offset.y * target.world_per_layout_unit.y)
        + target_anchor.normal * (layout_offset.z * target.world_per_layout_unit_z);
    let [top_left, top_right, bottom_right, bottom_left] = outsets;
    let corners = [
        (Anchor::TopLeft, top_left),
        (Anchor::TopRight, top_right),
        (Anchor::BottomRight, bottom_right),
        (Anchor::BottomLeft, bottom_left),
    ];
    Some(corners.map(|(corner, outset)| {
        let corner = world_target::panel_local_anchor(panel, corner) + outset;
        let offset = source_scale * (corner - source_anchor);
        target_position
            + target_anchor.right * offset.x
            + target_anchor.up * offset.y
            + target_anchor.normal * offset.z
    }))
}

fn tooltip_quad_occluded(
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
    camera: &TooltipCameraView<'_>,
    scene_occlusion: &mut SceneOcclusion<'_, '_, '_>,
) -> bool {
    let Some(corners) = world_tooltip_corners(panel, panel_global, target, attachment) else {
        return true;
    };
    let centre = (corners[0] + corners[1] + corners[2] + corners[3]) * 0.25;
    let origin = camera.global_transform.translation();
    corners
        .into_iter()
        .chain([centre])
        .any(|point| scene_occlusion.blocks(origin, point))
}

fn projected_world_tooltip_footprint(
    panel: &DiegeticPanel,
    panel_global: &GlobalTransform,
    target: &WorldTooltipTarget,
    attachment: PanelAttachment,
    camera: &TooltipCameraView<'_>,
) -> Option<TooltipProjectedFootprint> {
    Some(TooltipProjectedFootprint {
        body:    world_target::projected_points_bounds(
            world_tooltip_corners(panel, panel_global, target, attachment)?,
            camera,
        )?,
        painted: world_target::projected_points_bounds(
            world_tooltip_painted_corners(panel, panel_global, target, attachment)?,
            camera,
        )?,
    })
}

fn shifted_world_layout_offset(
    layout_offset: Vec3,
    viewport_shift: Vec2,
    anchor: WorldAnchorFrame,
    target: &WorldTooltipTarget,
    camera: &TooltipCameraView<'_>,
) -> Option<Vec3> {
    let depth_offset = layout_offset.z * target.world_per_layout_unit_z;
    let plane_origin = anchor.position + anchor.normal * depth_offset;
    let source_anchor_position = plane_origin
        + anchor.right * (layout_offset.x * target.world_per_layout_unit.x)
        - anchor.up * (layout_offset.y * target.world_per_layout_unit.y);
    let viewport_anchor = world_target::project_world_point(source_anchor_position, camera)?;
    let ray = camera
        .camera
        .viewport_to_world(camera.global_transform, viewport_anchor + viewport_shift)
        .ok()?;
    let denominator = ray.direction.dot(anchor.normal);
    if !denominator.is_finite() || denominator.abs() <= f32::EPSILON {
        return None;
    }
    let distance = (plane_origin - ray.origin).dot(anchor.normal) / denominator;
    if !distance.is_finite() || distance < 0.0 {
        return None;
    }
    let point = ray.get_point(distance);
    let delta = point - plane_origin;
    Some(Vec3::new(
        delta.dot(anchor.right) / target.world_per_layout_unit.x,
        -delta.dot(anchor.up) / target.world_per_layout_unit.y,
        layout_offset.z,
    ))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::too_many_lines,
    reason = "inline placement scenarios retain their full setup and assertions"
)]
mod tests {
    use bevy::camera::PerspectiveProjection;
    use bevy::camera::Projection;
    use bevy::camera::RenderTargetInfo;
    use bevy::camera::Viewport;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::TimerMode;
    use bevy::prelude::Vec3A;
    use bevy::transform::TransformPlugin;

    use super::world_target::CardTooltipPlacement;
    use super::*;
    use crate::DiegeticPanel;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::PanelDefaults;
    use crate::Pt;
    use crate::Px;
    use crate::TextStyle;
    use crate::TextWrap;
    use crate::layout::El;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::MeshAnchorCommandsExt;
    use crate::widgets::tooltip::TooltipCommandsExt;
    use crate::widgets::tooltip::declaration::TooltipPhase;
    use crate::widgets::tooltip::placement::viewport::TooltipSlideBounds;

    const DISTANT_WORLD_TARGET_Z: f32 = -45.0;

    const GEOMETRY_EPSILON: f32 = 1e-4;

    const MATERIALIZATION_UPDATES: usize = 5;

    const NARROW_WORLD_VIEWPORT: Vec2 = Vec2::new(96.0, 180.0);

    const OFFSET_VIEWPORT_ORIGIN: Vec2 = Vec2::new(180.0, 90.0);

    const OFFSET_VIEWPORT_SIZE: Vec2 = Vec2::new(240.0, 160.0);

    const TEST_CAMERA_DISTANCE: f32 = 5.0;

    const TEST_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

    const WIDE_WORLD_VIEWPORT: Vec2 = Vec2::new(800.0, 180.0);

    #[test]
    fn outside_card_fallback_keeps_the_whole_help_beyond_each_edge() {
        let card = Rect::from_corners(Vec2::new(100.0, 100.0), Vec2::new(180.0, 160.0));
        let help = PanelScreenBounds::new(Vec2::new(120.0, 120.0), Vec2::new(40.0, 12.0))
            .expect("valid help bounds");
        let near_right = PanelScreenBounds::new(Vec2::new(170.0, 125.0), Vec2::splat(8.0))
            .expect("valid jack bounds");
        assert_eq!(
            nearer_horizontal_outside_side(near_right, card),
            TooltipSide::Right
        );
        let near_top = PanelScreenBounds::new(Vec2::new(135.0, 102.0), Vec2::splat(8.0))
            .expect("valid title-row jack bounds");
        assert_eq!(
            nearer_vertical_outside_side(near_top, card),
            TooltipSide::Above
        );
        for side in [
            TooltipSide::Above,
            TooltipSide::Right,
            TooltipSide::Below,
            TooltipSide::Left,
        ] {
            let shifted = help.top_left() + outside_card_shift(side, help, card, 2.0);
            assert!(outside_card_clears(side, shifted, help.size(), card, 2.0));
            assert!(!outside_card_clears(
                side,
                help.top_left(),
                help.size(),
                card,
                2.0
            ));
        }
    }

    #[test]
    fn inside_only_card_help_uses_overlap_even_when_outside_is_free() {
        let card = PanelScreenBounds::new(Vec2::splat(100.0), Vec2::new(80.0, 60.0)).expect("card");
        let jack = PanelScreenBounds::new(Vec2::new(130.0, 120.0), Vec2::splat(8.0)).expect("jack");
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       vec![card],
            gap_pixels:     2.0,
            placement:      CardTooltipPlacement::InsideOnly,
            target_row:     TargetControlRow::TargetOnly,
        };
        assert!(matches!(
            card_spot_fit(
                &area,
                CardPlacementPass::Outside,
                TooltipSide::Right,
                unshadowed(Vec2::new(185.0, 118.0), Vec2::new(20.0, 12.0)),
                Vec2::ZERO,
                jack
            ),
            CardSpotFit::Covers(0)
        ));
        assert!(matches!(
            card_placement_passes(&area),
            [
                CardPlacementPass::Authored,
                CardPlacementPass::Inside,
                CardPlacementPass::InsideOverlapping
            ]
        ));
        assert!(matches!(
            card_spot_fit(
                &area,
                CardPlacementPass::InsideOverlapping,
                TooltipSide::Right,
                unshadowed(Vec2::new(140.0, 118.0), Vec2::new(20.0, 12.0)),
                Vec2::ZERO,
                jack
            ),
            CardSpotFit::Covers(1)
        ));
    }

    #[test]
    fn card_help_stays_beside_its_jack_after_collision_shifts() {
        let card = PanelScreenBounds::new(Vec2::new(100.0, 100.0), Vec2::new(80.0, 60.0))
            .expect("valid card bounds");
        let jack = PanelScreenBounds::new(Vec2::new(130.0, 120.0), Vec2::splat(8.0))
            .expect("valid jack bounds");
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       Vec::new(),
            gap_pixels:     2.0,
            placement:      CardTooltipPlacement::InsideOrOutside,
            target_row:     TargetControlRow::TargetOnly,
        };
        assert!(help_beside_jack(
            TooltipSide::Right,
            Vec2::new(140.0, 118.0),
            Vec2::new(20.0, 12.0),
            jack,
        ));
        assert!(!help_beside_jack(
            TooltipSide::Right,
            Vec2::new(110.0, 118.0),
            Vec2::new(20.0, 12.0),
            jack,
        ));
        assert!(matches!(
            card_spot_fit(
                &area,
                CardPlacementPass::Inside,
                TooltipSide::Right,
                unshadowed(Vec2::new(140.0, 118.0), Vec2::new(20.0, 12.0)),
                Vec2::ZERO,
                jack,
            ),
            CardSpotFit::Covers(0)
        ));
        assert!(matches!(
            card_spot_fit(
                &WorldTooltipPlacementArea::Viewport(TooltipSlideBounds::PlacementAreaOnly),
                CardPlacementPass::Inside,
                TooltipSide::Right,
                unshadowed(Vec2::new(110.0, 118.0), Vec2::new(20.0, 12.0)),
                Vec2::ZERO,
                jack,
            ),
            CardSpotFit::Covers(0)
        ));
    }

    #[test]
    fn card_help_keeps_its_shadow_inside_the_card() {
        let card = PanelScreenBounds::new(Vec2::new(100.0, 100.0), Vec2::new(80.0, 60.0))
            .expect("valid card bounds");
        let jack = PanelScreenBounds::new(Vec2::new(130.0, 120.0), Vec2::splat(8.0))
            .expect("valid jack bounds");
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       Vec::new(),
            gap_pixels:     2.0,
            placement:      CardTooltipPlacement::InsideOrOutside,
            target_row:     TargetControlRow::TargetOnly,
        };
        let bare = unshadowed(Vec2::new(150.0, 118.0), Vec2::new(26.0, 12.0));
        let shadowed = TooltipProjectedFootprint {
            body:    bare.body,
            painted: PanelScreenBounds::new(Vec2::new(142.0, 110.0), Vec2::new(42.0, 32.0))
                .expect("valid painted bounds"),
        };
        let fit = |footprint| {
            card_spot_fit(
                &area,
                CardPlacementPass::Inside,
                TooltipSide::Right,
                footprint,
                Vec2::ZERO,
                jack,
            )
        };
        assert!(matches!(fit(bare), CardSpotFit::Covers(0)));
        assert!(matches!(fit(shadowed), CardSpotFit::Refused));
    }

    #[test]
    fn crowded_card_help_may_cover_another_row_but_clears_its_own() {
        let card = PanelScreenBounds::new(Vec2::new(100.0, 100.0), Vec2::new(80.0, 60.0))
            .expect("valid card bounds");
        let jack = PanelScreenBounds::new(Vec2::new(130.0, 140.0), Vec2::splat(8.0))
            .expect("valid jack bounds");
        let label = PanelScreenBounds::new(Vec2::new(121.0, 120.0), Vec2::new(20.0, 8.0))
            .expect("valid label bounds");
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       vec![label],
            gap_pixels:     2.0,
            placement:      CardTooltipPlacement::InsideOrOutside,
            target_row:     TargetControlRow::CompleteRow(
                PanelScreenBounds::new(Vec2::new(110.0, 140.0), Vec2::new(50.0, 8.0))
                    .expect("valid target row"),
            ),
        };
        let spot = |pass| {
            card_spot_fit(
                &area,
                pass,
                TooltipSide::Above,
                unshadowed(Vec2::new(119.0, 118.0), Vec2::new(30.0, 12.0)),
                Vec2::ZERO,
                jack,
            )
        };
        assert!(matches!(
            spot(CardPlacementPass::Inside),
            CardSpotFit::Refused
        ));
        assert!(matches!(
            spot(CardPlacementPass::InsideOverlapping),
            CardSpotFit::Covers(1)
        ));
    }

    #[test]
    fn viewport_filling_card_help_clears_its_complete_target_row_in_every_pass() {
        let card =
            PanelScreenBounds::new(Vec2::ZERO, Vec2::new(1440.0, 900.0)).expect("valid card");
        let jack =
            PanelScreenBounds::new(Vec2::new(120.0, 430.0), Vec2::splat(20.0)).expect("valid jack");
        let row = PanelScreenBounds::new(Vec2::new(120.0, 420.0), Vec2::new(600.0, 40.0))
            .expect("valid row");
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       vec![row],
            gap_pixels:     2.0,
            placement:      CardTooltipPlacement::InsideOrOutside,
            target_row:     TargetControlRow::CompleteRow(row),
        };
        for pass in [
            CardPlacementPass::Inside,
            CardPlacementPass::Outside,
            CardPlacementPass::InsideOverlapping,
            CardPlacementPass::OverlayShiftedIntoWindow,
        ] {
            let covered = shadowed(Vec2::new(150.0, 421.0), Vec2::new(350.0, 100.0));
            assert!(matches!(
                card_spot_fit(&area, pass, TooltipSide::Right, covered, Vec2::ZERO, jack),
                CardSpotFit::Refused
            ));
            let clear = shadowed(Vec2::new(150.0, 250.0), Vec2::new(350.0, 100.0));
            if let CardSpotFit::Covers(_) =
                card_spot_fit(&area, pass, TooltipSide::Above, clear, Vec2::ZERO, jack)
            {
                assert!(
                    clear.painted.top_left().y + clear.painted.size().y
                        <= row.top_left().y - CARD_HELP_CLEARANCE_PIXELS
                );
            }
        }
    }

    #[test]
    fn card_help_whose_shadow_covers_a_region_counts_as_covering_it() {
        let card = PanelScreenBounds::new(Vec2::new(100.0, 100.0), Vec2::new(160.0, 80.0))
            .expect("valid card bounds");
        let jack = PanelScreenBounds::new(Vec2::new(110.0, 130.0), Vec2::splat(8.0))
            .expect("valid jack bounds");
        let label = PanelScreenBounds::new(Vec2::new(156.0, 130.0), Vec2::new(20.0, 8.0))
            .expect("valid label bounds");
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       vec![label],
            gap_pixels:     2.0,
            placement:      CardTooltipPlacement::InsideOrOutside,
            target_row:     TargetControlRow::TargetOnly,
        };
        // The body ends short of the label; only the shadow reaches it.
        let help = shadowed(Vec2::new(122.0, 128.0), Vec2::new(30.0, 12.0));
        let spot = |pass, shift| card_spot_fit(&area, pass, TooltipSide::Right, help, shift, jack);
        assert!(matches!(
            spot(CardPlacementPass::Inside, Vec2::ZERO),
            CardSpotFit::Refused
        ));
        assert!(matches!(
            spot(CardPlacementPass::InsideOverlapping, Vec2::ZERO),
            CardSpotFit::Covers(1)
        ));
        let shifts = placement_candidate_shifts(
            &area,
            CardPlacementPass::Inside,
            TooltipSide::Right,
            card_slide(),
            help,
            Vec2::new(114.0, 134.0),
            jack,
            Rect::new(0.0, 0.0, 600.0, 400.0),
        );
        assert!(
            shifts.iter().any(|shift| matches!(
                spot(CardPlacementPass::Inside, *shift),
                CardSpotFit::Covers(0)
            )),
            "an inside spot whose shadow clears the label: {shifts:?}"
        );
    }

    #[test]
    fn outside_card_help_keeps_its_shadow_off_the_card() {
        let viewport = Rect::new(0.0, 0.0, 600.0, 400.0);
        let card = PanelScreenBounds::new(Vec2::new(200.0, 150.0), Vec2::new(120.0, 80.0))
            .expect("valid card bounds");
        let card_rect = screen_rect(card);
        let jack = PanelScreenBounds::new(Vec2::new(300.0, 180.0), Vec2::splat(10.0))
            .expect("valid jack bounds");
        let gap = 4.0;
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       Vec::new(),
            gap_pixels:     gap,
            placement:      CardTooltipPlacement::InsideOrOutside,
            target_row:     TargetControlRow::TargetOnly,
        };
        let help = shadowed(Vec2::new(250.0, 185.0), Vec2::new(40.0, 12.0));
        for side in [
            TooltipSide::Above,
            TooltipSide::Right,
            TooltipSide::Below,
            TooltipSide::Left,
        ] {
            let shifts = placement_candidate_shifts(
                &area,
                CardPlacementPass::Outside,
                side,
                card_slide(),
                help,
                Vec2::new(305.0, 185.0),
                jack,
                viewport,
            );
            assert_eq!(shifts.len(), 1);
            let painted_min = help.painted.top_left() + shifts[0];
            let painted_max = painted_min + help.painted.size();
            let clear = match side {
                TooltipSide::Above => painted_max.y <= card_rect.min.y - gap + GEOMETRY_EPSILON,
                TooltipSide::Right => painted_min.x >= card_rect.max.x + gap - GEOMETRY_EPSILON,
                TooltipSide::Below => painted_min.y >= card_rect.max.y + gap - GEOMETRY_EPSILON,
                TooltipSide::Left => painted_max.x <= card_rect.min.x - gap + GEOMETRY_EPSILON,
            };
            assert!(clear, "{side:?} shadow lands {gap} px past the card");
            assert!(matches!(
                card_spot_fit(
                    &area,
                    CardPlacementPass::Outside,
                    side,
                    help,
                    shifts[0],
                    jack
                ),
                CardSpotFit::Covers(0)
            ));
            let body_clear_only = outside_card_shift(side, help.body, card_rect, gap);
            assert!(
                matches!(
                    card_spot_fit(
                        &area,
                        CardPlacementPass::Outside,
                        side,
                        help,
                        body_clear_only,
                        jack,
                    ),
                    CardSpotFit::Refused
                ),
                "{side:?} body clears the card while its shadow paints over it"
            );
        }
    }

    #[test]
    fn outside_card_help_below_slides_along_the_card_edge_into_the_viewport() {
        let viewport = Rect::new(0.0, 0.0, 600.0, 400.0);
        let card = PanelScreenBounds::new(Vec2::new(150.0, 100.0), Vec2::new(300.0, 150.0))
            .expect("valid card bounds");
        let jack = PanelScreenBounds::new(Vec2::new(160.0, 150.0), Vec2::splat(10.0))
            .expect("valid jack bounds");
        let help = PanelScreenBounds::new(Vec2::new(15.0, 170.0), Vec2::new(320.0, 20.0))
            .expect("valid help bounds");
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       Vec::new(),
            gap_pixels:     6.0,
            placement:      CardTooltipPlacement::InsideOrOutside,
            target_row:     TargetControlRow::TargetOnly,
        };
        let shifts = placement_candidate_shifts(
            &area,
            CardPlacementPass::Outside,
            TooltipSide::Below,
            card_slide(),
            unshadowed(help.top_left(), help.size()),
            Vec2::new(165.0, 155.0),
            jack,
            viewport,
        );
        assert_eq!(shifts.len(), 1);
        let top_left = help.top_left() + shifts[0];
        let margin = crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN;
        assert!((top_left.y - 256.0).abs() < GEOMETRY_EPSILON);
        assert!((top_left.x - margin).abs() < GEOMETRY_EPSILON);
        assert!(viewport::bounds_fit_viewport(
            top_left,
            help.size(),
            viewport
        ));
        assert!(matches!(
            card_spot_fit(
                &area,
                CardPlacementPass::Outside,
                TooltipSide::Below,
                unshadowed(help.top_left(), help.size()),
                shifts[0],
                jack,
            ),
            CardSpotFit::Covers(0)
        ));
    }

    /// A help box that paints only its body.
    /// The slide card help takes: its card's rule alone.
    const fn card_slide() -> TooltipSlide {
        TooltipSlide::Beside(TooltipShiftBoundary::PlacementAreaOnly)
    }

    fn unshadowed(top_left: Vec2, size: Vec2) -> TooltipProjectedFootprint {
        let body = PanelScreenBounds::new(top_left, size).expect("valid help bounds");
        TooltipProjectedFootprint {
            body,
            painted: body,
        }
    }

    /// A help box whose shadow reaches 8 px above and to each side and 11 px
    /// below its body.
    fn shadowed(top_left: Vec2, size: Vec2) -> TooltipProjectedFootprint {
        TooltipProjectedFootprint {
            body:    PanelScreenBounds::new(top_left, size).expect("valid help bounds"),
            painted: PanelScreenBounds::new(top_left - 8.0, size + Vec2::new(16.0, 19.0))
                .expect("valid painted bounds"),
        }
    }

    #[test]
    fn help_narrowed_to_the_window_fits_when_it_projects_a_rounding_error_wide() {
        // Measured from a close Camera card's On help: narrowed once, it
        // projected 0.0001 px wider than the room and kept narrowing.
        let usable = 1370.4502;
        assert!(matches!(
            world_viewport_width_decision(
                0.110_567_5,
                1398.633,
                usable,
                TooltipWidthLimit::Natural
            ),
            TooltipPlacementDecision::ConstrainWidth(_)
        ));
        assert!(matches!(
            world_viewport_width_decision(
                0.108_339_53,
                1370.4503,
                usable,
                TooltipWidthLimit::Constrained(0.108_339_53),
            ),
            TooltipPlacementDecision::Fits
        ));
    }

    #[test]
    fn help_too_big_for_a_close_card_slides_into_the_window_beside_its_jack() {
        // A card framed close fills the window; its help is too wide to sit
        // beside the jack inside it and too tall to fit above or below it.
        let viewport = Rect::new(0.0, 0.0, 1440.0, 900.0);
        let card = PanelScreenBounds::new(Vec2::new(240.0, 108.0), Vec2::new(959.0, 684.0))
            .expect("valid card bounds");
        let area = WorldTooltipPlacementArea::Card {
            bounds:         card,
            outside_bounds: card,
            occupied:       Vec::new(),
            gap_pixels:     6.0,
            placement:      CardTooltipPlacement::InsideOrOutside,
            target_row:     TargetControlRow::TargetOnly,
        };
        let help = shadowed(Vec2::new(-20.0, 532.0), Vec2::new(824.0, 215.0));
        let shifts = |pass, jack: PanelScreenBounds| {
            placement_candidate_shifts(
                &area,
                pass,
                TooltipSide::Below,
                card_slide(),
                help,
                jack.top_left() + jack.size() * 0.5,
                jack,
                viewport,
            )
        };
        let fit =
            |pass, shift, jack| card_spot_fit(&area, pass, TooltipSide::Below, help, shift, jack);
        let painted_fits_viewport = |shift: Vec2| {
            viewport::bounds_fit_viewport(
                help.painted.top_left() + shift,
                help.painted.size(),
                viewport,
            )
        };
        let painted_fits_window = |shift: Vec2| {
            viewport::bounds_fit_window(
                help.painted.top_left() + shift,
                help.painted.size(),
                viewport,
            )
        };

        let jack = PanelScreenBounds::new(Vec2::new(393.0, 335.0), Vec2::splat(189.0))
            .expect("valid jack bounds");
        for pass in [
            CardPlacementPass::Inside,
            CardPlacementPass::Outside,
            CardPlacementPass::InsideOverlapping,
        ] {
            assert!(shifts(pass, jack).into_iter().all(|shift| {
                matches!(fit(pass, shift, jack), CardSpotFit::Refused)
                    || !painted_fits_viewport(shift)
            }));
        }
        let in_window = shifts(CardPlacementPass::OverlayShiftedIntoWindow, jack);
        assert_eq!(in_window.len(), 1);
        let shift = in_window[0];
        assert!(
            shift.y.abs() < GEOMETRY_EPSILON,
            "help keeps its distance below the jack"
        );
        assert!(
            (help.body.top_left().x + shift.x - 75.5).abs() < GEOMETRY_EPSILON,
            "help centres on the jack's column, which the window holds"
        );
        assert!(matches!(
            fit(CardPlacementPass::OverlayShiftedIntoWindow, shift, jack),
            CardSpotFit::Covers(0)
        ));
        assert!(painted_fits_window(shift));
        assert!(
            matches!(
                fit(
                    CardPlacementPass::OverlayShiftedIntoWindow,
                    Vec2::new(95.5, -132.0),
                    jack,
                ),
                CardSpotFit::Refused
            ),
            "help over its own jack is refused"
        );

        // Near the window's left edge the final no-room fallback slides right
        // until its full drawn extent, shadow included, is inside the window.
        let edge_jack = PanelScreenBounds::new(Vec2::new(20.0, 335.0), Vec2::splat(189.0))
            .expect("valid jack bounds");
        let in_window = shifts(CardPlacementPass::OverlayShiftedIntoWindow, edge_jack);
        assert_eq!(in_window.len(), 1);
        let shift = in_window[0];
        assert!(
            (help.painted.top_left().x + shift.x).abs() < GEOMETRY_EPSILON,
            "the shadow's left edge stands at the window edge"
        );
        assert!(matches!(
            fit(
                CardPlacementPass::OverlayShiftedIntoWindow,
                shift,
                edge_jack,
            ),
            CardSpotFit::Covers(0)
        ));
        assert!(painted_fits_window(shift));
    }

    #[test]
    fn overlay_flips_clear_of_the_complete_row_on_every_side() {
        let target = PanelScreenBounds::new(Vec2::new(140.0, 200.0), Vec2::splat(20.0))
            .expect("target bounds");
        let row = PanelScreenBounds::new(Vec2::new(130.0, 190.0), Vec2::new(240.0, 40.0))
            .expect("row bounds");
        let neighbours = NeighbouringControlBounds(vec![row]);
        let candidate = PanelScreenBounds::new(Vec2::new(150.0, 190.0), Vec2::new(80.0, 40.0))
            .expect("help bounds");
        for side in [
            TooltipSide::Above,
            TooltipSide::Right,
            TooltipSide::Below,
            TooltipSide::Left,
        ] {
            let shift = neighbours.target_row_clearance_shift(
                side,
                candidate.top_left(),
                candidate.size(),
                target,
            );
            let moved = PanelScreenBounds::new(candidate.top_left() + shift, candidate.size())
                .expect("shifted help bounds");
            assert!(neighbours.clears_target_row(moved, target));
        }
    }

    #[test]
    fn flip_then_shift_overlay_clears_the_target_row_and_may_cover_another_row() {
        let target = PanelScreenBounds::new(Vec2::new(100.0, 100.0), Vec2::splat(10.0))
            .expect("valid target bounds");
        let value = PanelScreenBounds::new(Vec2::new(112.0, 98.0), Vec2::new(30.0, 14.0))
            .expect("valid value bounds");
        let other_row = PanelScreenBounds::new(Vec2::new(110.0, 135.0), Vec2::new(40.0, 10.0))
            .expect("valid other-row bounds");
        let neighbours = NeighbouringControlBounds(vec![value, other_row]);
        let over_other_row = PanelScreenBounds::new(Vec2::new(90.0, 114.0), Vec2::new(80.0, 50.0))
            .expect("valid overlay bounds");
        let over_target_row = PanelScreenBounds::new(Vec2::new(90.0, 108.0), Vec2::new(80.0, 50.0))
            .expect("valid overlay bounds");
        let beside_target_row =
            PanelScreenBounds::new(Vec2::new(40.0, 90.0), Vec2::new(58.0, 50.0))
                .expect("valid adjacent overlay bounds");

        assert!(neighbours.clears_target_row(over_other_row, target));
        assert!(neighbours.clears_target_row(beside_target_row, target));
        assert!(neighbours.overlaps(over_other_row));
        assert!(!neighbours.clears_target_row(over_target_row, target));
        assert_eq!(
            neighbours.target_row_clearance_shift(
                TooltipSide::Below,
                over_target_row.top_left(),
                over_target_row.size(),
                target,
            ),
            Vec2::new(0.0, 6.0),
        );
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

    fn mesh_target(app: &mut App, half_extents: Vec3A) -> Entity {
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center: Vec3A::ZERO,
                    half_extents,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        target
    }

    #[test]
    fn world_tooltip_plane_stays_in_front_of_overlapping_host_content() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let mut host_tree = crate::LayoutBuilder::new(100.0, 50.0);
        host_tree.with(
            crate::layout::El::new()
                .size(100.0, 50.0)
                .background(bevy::prelude::Color::srgb(0.1, 0.1, 0.1)),
            |builder| {
                builder.text(Text::new("Host content", crate::TextStyle::new(12.0)));
            },
        );
        let target = DiegeticPanel::world()
            .size(crate::Pt(100.0), crate::Pt(50.0))
            .world_width(1.0)
            .with_tree(host_tree.build())
            .build();
        assert!(target.is_ok());
        let Ok(target) = target else { return };
        let target = app
            .world_mut()
            .spawn((
                target,
                Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4)),
            ))
            .id();
        let tooltip = fixed_size_tooltip(Vec2::new(30.0, 10.0))
            .source_anchor(Anchor::Center)
            .target_anchor(Anchor::Center);
        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<World>::from_validated(target, PanelSpace::World),
            tooltip,
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        let target_global = app.world().get::<GlobalTransform>(target);
        assert!(target_global.is_some(), "host panel transform");
        let Some(target_global) = target_global else {
            return;
        };
        let host_panel = app.world().get::<DiegeticPanel>(target);
        assert!(host_panel.is_some(), "host panel");
        let Some(host_panel) = host_panel else { return };
        let host_plane = crate::panel::PanelPlane::from_panel(host_panel, target_global);
        assert!(host_plane.is_ok(), "host panel plane");
        let Ok(host_plane) = host_plane else { return };
        let target_center = host_plane.point(Anchor::Center);
        let target_normal = host_plane.normal();
        let tooltip_panel = app.world().get::<DiegeticPanel>(controller);
        let tooltip_global = app.world().get::<GlobalTransform>(controller);
        assert!(tooltip_panel.is_some(), "materialized tooltip panel");
        assert!(tooltip_global.is_some(), "resolved tooltip transform");
        let (Some(tooltip_panel), Some(tooltip_global)) = (tooltip_panel, tooltip_global) else {
            return;
        };
        let tooltip_plane = crate::panel::PanelPlane::from_panel(tooltip_panel, tooltip_global);
        assert!(tooltip_plane.is_ok(), "tooltip plane");
        let Ok(tooltip_plane) = tooltip_plane else {
            return;
        };
        let separation = (tooltip_plane.point(Anchor::Center) - target_center).dot(target_normal);
        assert!(
            separation > 0.0,
            "tooltip must be ahead of host content: {separation}"
        );
        assert!(tooltip_plane.normal().dot(target_normal) > 0.999);
    }

    #[test]
    fn cube_tooltip_is_withheld_when_no_outside_side_fits() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
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
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        for (anchor, size) in [
            (Anchor::TopLeft, Vec2::new(800.0, 100.0)),
            (Anchor::BottomLeft, Vec2::new(800.0, 150.0)),
            (Anchor::TopRight, Vec2::new(200.0, 600.0)),
        ] {
            let panel = DiegeticPanel::screen()
                .size(Px(size.x), Px(size.y))
                .anchor(anchor)
                .window_entity(window)
                .with_tree(LayoutBuilder::new(size.x, size.y).build())
                .build();
            assert!(panel.is_ok());
            if let Ok(panel) = panel {
                app.world_mut().spawn(panel);
            }
        }
        let target = mesh_target(&mut app, Vec3A::ONE);
        let controller = standalone_world_tooltip(
            &mut app,
            target,
            fixed_size_tooltip(Vec2::new(1_600.0, 500.0))
                .source_anchor(Anchor::CenterLeft)
                .target_anchor(Anchor::CenterRight),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        for rotation in [0.0, std::f32::consts::FRAC_PI_6] {
            if let Some(mut transform) = app.world_mut().get_mut::<Transform>(target) {
                transform.rotation = Quat::from_rotation_y(rotation);
            }
            update_materialization(&mut app);
            assert_ne!(
                app.world().get::<TooltipReadiness>(controller),
                Some(&TooltipReadiness::Ready),
                "tooltip must not cover its target when no side fits",
            );
        }
    }

    fn standalone_world_tooltip(app: &mut App, target: Entity, tooltip: Tooltip) -> Entity {
        let handle = TooltipTargetEntity::<World>::from_validated(target);
        app.world_mut().commands().spawn_tooltip(handle, tooltip)
    }

    fn prepare_tooltip(app: &mut App, controller: Entity) {
        app.world_mut()
            .entity_mut(controller)
            .insert(PrepareTooltip);
    }

    fn seeded_perspective_camera() -> Camera { seeded_perspective_camera_for(TEST_VIEWPORT) }

    fn seeded_perspective_camera_for(viewport: Vec2) -> Camera {
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: viewport.as_uvec2(),
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(viewport.x, viewport.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        camera
    }

    fn seeded_perspective_camera_with_offset_viewport() -> Camera {
        let mut camera = seeded_perspective_camera_for(TEST_VIEWPORT);
        camera.viewport = Some(Viewport {
            physical_position: OFFSET_VIEWPORT_ORIGIN.as_uvec2(),
            physical_size: OFFSET_VIEWPORT_SIZE.as_uvec2(),
            ..Default::default()
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(OFFSET_VIEWPORT_SIZE.x, OFFSET_VIEWPORT_SIZE.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        camera
    }

    fn spawn_presentation_camera(
        app: &mut App,
        window: Entity,
        render_layers: RenderLayers,
    ) -> Entity {
        let transform = Transform::from_xyz(0.0, 0.0, TEST_CAMERA_DISTANCE);
        app.world_mut()
            .spawn((
                seeded_perspective_camera(),
                RenderTarget::Window(WindowRef::Entity(window)),
                render_layers,
                transform,
                GlobalTransform::from(transform),
            ))
            .id()
    }

    fn set_presentation_viewport(app: &mut App, camera: Entity, window: Entity, viewport: Vec2) {
        if let Some(mut window) = app.world_mut().get_mut::<Window>(window) {
            window.resolution.set(viewport.x, viewport.y);
        }
        if let Some(mut camera) = app.world_mut().get_mut::<Camera>(camera) {
            *camera = seeded_perspective_camera_for(viewport);
        }
    }

    fn fixed_size_tooltip(size: Vec2) -> Tooltip {
        let mut tooltip = Tooltip::new(El::new());
        tooltip.with(El::new().size(size.x, size.y), |_| {});
        tooltip
    }

    fn projected_world_panel_bounds(
        app: &App,
        panel_entity: Entity,
        camera_entity: Entity,
    ) -> Option<PanelScreenBounds> {
        let panel = app.world().get::<DiegeticPanel>(panel_entity)?;
        let panel_global = app.world().get::<GlobalTransform>(panel_entity)?;
        let camera = app.world().get::<Camera>(camera_entity)?;
        let camera_global = app.world().get::<GlobalTransform>(camera_entity)?;
        let viewport = camera.logical_viewport_rect()?;
        let camera_view = TooltipCameraView {
            camera,
            global_transform: camera_global,
            window: Entity::PLACEHOLDER,
            viewport,
        };
        let plane = crate::panel::PanelPlane::from_panel(panel, panel_global).ok()?;
        world_target::projected_points_bounds(
            [
                plane.point(Anchor::TopLeft),
                plane.point(Anchor::TopRight),
                plane.point(Anchor::BottomRight),
                plane.point(Anchor::BottomLeft),
            ],
            &camera_view,
        )
    }

    #[test]
    fn world_target_observation_tracks_initial_stable_and_changed_geometry() {
        let mut app = App::new();
        let controller = app
            .world_mut()
            .spawn(TooltipPlacementState::pending(PanelAttachment::new(
                Anchor::TopLeft,
                Anchor::BottomRight,
            )))
            .id();
        let frame = WorldAnchorFrame {
            position: Vec3::ZERO,
            right:    Vec3::X,
            up:       Vec3::Y,
            normal:   Vec3::Z,
        };
        let geometry = WorldTargetGeometry {
            anchors:                 [frame; crate::widgets::constants::QUAD_ANCHOR_COUNT],
            layout_unit:             Unit::Pixels,
            world_per_layout_unit:   Vec2::ONE,
            world_per_layout_unit_z: 1.0,
        };
        let changed = WorldTargetGeometry {
            world_per_layout_unit: Vec2::splat(2.0),
            ..geometry
        };

        assert!(
            app.world()
                .get::<TooltipPlacementState>(controller)
                .is_some()
        );
        let Some(mut placement) = app.world_mut().get_mut::<TooltipPlacementState>(controller)
        else {
            return;
        };
        assert_eq!(
            update_world_target_geometry(&mut placement, &geometry),
            WorldTargetChange::Initial,
        );
        assert_eq!(
            placement.world_target,
            WorldTargetObservation::LastObserved(Box::new(geometry))
        );
        assert_eq!(
            update_world_target_geometry(&mut placement, &geometry),
            WorldTargetChange::Stable,
        );
        assert_eq!(
            placement.world_target,
            WorldTargetObservation::LastObserved(Box::new(geometry))
        );
        assert_eq!(
            update_world_target_geometry(&mut placement, &changed),
            WorldTargetChange::Changed,
        );
        assert_eq!(
            placement.world_target,
            WorldTargetObservation::LastObserved(Box::new(changed))
        );
    }

    #[test]
    fn world_keep_visible_tracks_oblique_target_moves_in_propagated_geometry() {
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
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target_panel = DiegeticPanel::world()
            .size(Pt(400.0), Pt(200.0))
            .world_width(2.0)
            .with_tree(LayoutBuilder::new(400.0, 200.0).build())
            .build();
        assert!(target_panel.is_ok());
        let target_transform = Transform::from_xyz(2.7, 1.2, 0.0)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_3));
        let target_panel = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn((panel, target_transform)).id()
        });
        assert_ne!(target_panel, Entity::PLACEHOLDER);
        app.update();
        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<World>::from_validated(target_panel, PanelSpace::World),
            fixed_size_tooltip(Vec2::new(1_600.0, 500.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let initial_bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(initial_bounds.is_some());
        if let Some(initial_bounds) = initial_bounds {
            assert!(viewport::bounds_fit_viewport(
                initial_bounds.top_left(),
                initial_bounds.size(),
                tooltip::zero_origin_viewport(TEST_VIEWPORT),
            ));
        }
        let initial_translation = app
            .world()
            .get::<GlobalTransform>(controller)
            .map(GlobalTransform::translation);

        if let Some(mut transform) = app.world_mut().get_mut::<Transform>(target_panel) {
            transform.translation = Vec3::new(-2.7, -1.2, 0.0);
            transform.rotation = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_3);
        }
        app.update();

        let moved_translation = app
            .world()
            .get::<GlobalTransform>(controller)
            .map(GlobalTransform::translation);
        assert!(initial_translation.is_some());
        assert!(moved_translation.is_some());
        assert_ne!(initial_translation, moved_translation);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let moved_bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(moved_bounds.is_some());
        if let Some(moved_bounds) = moved_bounds {
            assert!(viewport::bounds_fit_viewport(
                moved_bounds.top_left(),
                moved_bounds.size(),
                tooltip::zero_origin_viewport(TEST_VIEWPORT),
            ));
        }
    }

    #[test]
    fn world_tooltip_moves_from_authored_right_side_at_viewport_edge() {
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
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target = DiegeticPanel::world()
            .size(Pt(400.0), Pt(200.0))
            .world_width(2.0)
            .with_tree(LayoutBuilder::new(400.0, 200.0).build())
            .build();
        assert!(target.is_ok());
        let target = target.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut()
                .spawn((panel, Transform::from_xyz(2.7, 0.0, 0.0)))
                .id()
        });
        assert_ne!(target, Entity::PLACEHOLDER);
        let tooltip = fixed_size_tooltip(Vec2::new(250.0, 80.0))
            .source_anchor(Anchor::CenterLeft)
            .target_anchor(Anchor::CenterRight)
            .offset(PanelAnchorOffset::new(Pt(20.0), Pt(0.0)));
        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<World>::from_validated(target, PanelSpace::World),
            tooltip,
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        let placement = app.world().get::<TooltipPlacementState>(controller);
        assert!(placement.is_some());
        assert_ne!(
            placement.map(|placement| placement.attachment.source_anchor()),
            Some(Anchor::CenterLeft),
            "authored right side should extend past the viewport",
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
        );
        let bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(viewport::bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                tooltip::zero_origin_viewport(TEST_VIEWPORT),
            ));
        }
    }

    #[test]
    fn visible_world_tooltip_moves_with_target_without_redeciding() {
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
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target_panel = DiegeticPanel::world()
            .size(Pt(200.0), Pt(100.0))
            .world_width(2.0)
            .with_tree(LayoutBuilder::new(200.0, 100.0).build())
            .build();
        assert!(target_panel.is_ok());
        let target = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn((panel, Transform::default())).id()
        });
        assert_ne!(target, Entity::PLACEHOLDER);
        app.update();

        let controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<World>::from_validated(target, PanelSpace::World),
            fixed_size_tooltip(Vec2::new(200.0, 100.0)),
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        let settled_runs = app.world().resource::<TooltipPlacementRunCount>().world;
        app.update();
        assert_eq!(
            app.world().resource::<TooltipPlacementRunCount>().world,
            settled_runs,
            "a settled world tooltip should not rerun placement on a quiet frame",
        );

        let initial_target = app
            .world()
            .get::<GlobalTransform>(target)
            .map(GlobalTransform::translation);
        let initial_tooltip = app
            .world()
            .get::<GlobalTransform>(controller)
            .map(GlobalTransform::translation);
        assert!(initial_target.is_some());
        assert!(initial_tooltip.is_some());

        let attachment = app
            .world()
            .get::<TooltipPlacementState>(controller)
            .expect("placed tooltip")
            .attachment;
        let width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .expect("tooltip panel")
            .width();
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPhase::WaitingToHide(bevy::prelude::Timer::new(
                std::time::Duration::from_secs(10),
                TimerMode::Once,
            )));
        let movement = Vec3::new(1.25, -0.75, 0.5);
        if let Some(mut transform) = app.world_mut().get_mut::<Transform>(target) {
            transform.translation += movement;
        }
        app.update();

        let moved_target = app
            .world()
            .get::<GlobalTransform>(target)
            .map(GlobalTransform::translation);
        let moved_tooltip = app
            .world()
            .get::<GlobalTransform>(controller)
            .map(GlobalTransform::translation);
        assert!(moved_target.is_some());
        assert!(moved_tooltip.is_some());
        if let (
            Some(initial_target),
            Some(initial_tooltip),
            Some(moved_target),
            Some(moved_tooltip),
        ) = (initial_target, initial_tooltip, moved_target, moved_tooltip)
        {
            assert!(
                moved_target.abs_diff_eq(initial_target + movement, GEOMETRY_EPSILON),
                "target did not publish its current-frame pose",
            );
            assert!(
                moved_tooltip.abs_diff_eq(initial_tooltip + movement, GEOMETRY_EPSILON),
                "tooltip should follow the same target movement in the same frame",
            );
        }
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
    fn unchanged_view_writes_allow_constrained_world_tooltip_to_become_ready() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: NARROW_WORLD_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        set_presentation_viewport(&mut app, camera, window, NARROW_WORLD_VIEWPORT);
        let target = mesh_target(&mut app, Vec3A::ONE);
        let mut tooltip = Tooltip::new(El::new());
        tooltip.text(Text::new("alpha ".repeat(20), TextStyle::new(100.0)).wrap(TextWrap::Words));
        let controller = standalone_world_tooltip(&mut app, target, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));

        for _ in 0..MATERIALIZATION_UPDATES {
            // Bevy's text-input IME system writes Window each frame, even when
            // the viewport is unchanged. Camera writes can be unchanged too.
            let ime_enabled = app
                .world()
                .get::<Window>(window)
                .expect("window")
                .ime_enabled;
            app.world_mut()
                .get_mut::<Window>(window)
                .expect("window")
                .ime_enabled = ime_enabled;
            let unchanged = app.world().get::<Camera>(camera).expect("camera").clone();
            *app.world_mut().get_mut::<Camera>(camera).expect("camera") = unchanged;
            app.update();
        }

        assert!(
            app.world()
                .get::<TooltipWidthConstraint>(controller)
                .is_some()
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "unchanged view writes must not restart width fitting: {:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
    }

    #[test]
    fn world_camera_growth_restores_natural_width_before_rechecking_the_cap() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: NARROW_WORLD_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        set_presentation_viewport(&mut app, camera, window, NARROW_WORLD_VIEWPORT);
        let target = mesh_target(&mut app, Vec3A::ONE);
        let mut tooltip = Tooltip::new(El::new());
        tooltip.text(Text::new("alpha ".repeat(20), TextStyle::new(100.0)).wrap(TextWrap::Words));
        let controller = standalone_world_tooltip(&mut app, target, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        let constrained_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        let constrained_max = app
            .world()
            .get::<TooltipWidthConstraint>(controller)
            .map(|constraint| constraint.0);
        assert!(constrained_width.is_some());
        assert!(constrained_max.is_some());
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );

        set_presentation_viewport(&mut app, camera, window, WIDE_WORLD_VIEWPORT);
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
        let grown_max = app
            .world()
            .get::<TooltipWidthConstraint>(controller)
            .map(|constraint| constraint.0);
        assert!(
            grown_max.is_none()
                || constrained_max
                    .zip(grown_max)
                    .is_some_and(|(constrained_max, grown_max)| grown_max > constrained_max)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(viewport::bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                tooltip::zero_origin_viewport(WIDE_WORLD_VIEWPORT),
            ));
        }
    }

    #[test]
    fn world_target_move_restores_natural_width_before_rechecking_the_cap() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: NARROW_WORLD_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        set_presentation_viewport(&mut app, camera, window, NARROW_WORLD_VIEWPORT);
        let target = mesh_target(&mut app, Vec3A::ONE);
        let mut tooltip = Tooltip::new(El::new());
        tooltip.text(Text::new("alpha ".repeat(20), TextStyle::new(100.0)).wrap(TextWrap::Words));
        let controller = standalone_world_tooltip(&mut app, target, tooltip);
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        let constrained_width = app
            .world()
            .get::<DiegeticPanel>(controller)
            .map(DiegeticPanel::width);
        let camera_tick = app
            .world()
            .entity(camera)
            .get_change_ticks::<Camera>()
            .map(|ticks| ticks.changed);
        assert!(constrained_width.is_some());
        assert!(
            app.world()
                .get::<TooltipWidthConstraint>(controller)
                .is_some()
        );

        if let Some(mut transform) = app.world_mut().get_mut::<Transform>(target) {
            transform.translation.z = DISTANT_WORLD_TARGET_Z;
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
            app.world()
                .entity(camera)
                .get_change_ticks::<Camera>()
                .map(|ticks| ticks.changed),
            camera_tick,
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
    }

    #[test]
    fn world_keep_visible_honors_an_offset_camera_viewport() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
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
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        app.world_mut()
            .entity_mut(camera)
            .insert(seeded_perspective_camera_with_offset_viewport());
        let target = mesh_target(&mut app, Vec3A::ONE);
        let controller =
            standalone_world_tooltip(&mut app, target, fixed_size_tooltip(Vec2::new(30.0, 10.0)));
        app.update();
        prepare_tooltip(&mut app, controller);
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);

        let viewport = Rect::from_corners(
            OFFSET_VIEWPORT_ORIGIN,
            OFFSET_VIEWPORT_ORIGIN + OFFSET_VIEWPORT_SIZE,
        );
        assert_eq!(
            app.world()
                .get::<Camera>(camera)
                .and_then(Camera::logical_viewport_rect),
            Some(viewport),
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready),
            "placement={:?}",
            app.world().get::<TooltipPlacementState>(controller),
        );
        let bounds = projected_world_panel_bounds(&app, controller, camera);
        assert!(bounds.is_some());
        if let Some(bounds) = bounds {
            assert!(viewport::bounds_fit_viewport(
                bounds.top_left(),
                bounds.size(),
                viewport,
            ));
        }
    }
}
