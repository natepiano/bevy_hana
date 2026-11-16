//! Resizes materialized world help without changing its reveal lifecycle.

use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::In;
use bevy::prelude::Query;
use bevy::prelude::SystemSet;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use hana_kana::Resolved;

use super::declaration::Tooltip;
use super::materialization::MaterializedTooltip;
use super::materialization::WorldTooltipFrameProjection;
use super::placement::TooltipPlacementState;
use crate::cascade::FontUnit;
use crate::layout::Dimension;
use crate::panel;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachment;
use crate::panel::PanelSpace;

/// Updates camera-dependent world tooltip targets before their final anchor pass.
/// Schedule callers after their camera controller and projection update.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub struct TooltipWorldUpdate;

pub(super) struct TooltipResize {
    pub(super) controller: Entity,
    pub(super) label_size: Dimension,
}

/// The current drawing bridges a late resize until layout runs next frame.
#[derive(Component)]
pub(super) struct TooltipResizePending {
    pub(super) layout_scale: Vec3,
}

/// A rebuilt tooltip needs placement with its new label gap and pixel grid.
#[derive(Component)]
pub(super) struct WorldTooltipResizeFitPending;

#[derive(Component)]
pub(super) struct WorldTooltipUpdatePending;

pub(super) fn world_tooltip_update_requested(
    pending: Query<(), With<WorldTooltipUpdatePending>>,
) -> bool {
    !pending.is_empty()
}

pub(super) fn clear_world_tooltip_updates(
    pending: Query<Entity, With<WorldTooltipUpdatePending>>,
    mut commands: Commands,
) {
    for entity in &pending {
        commands
            .entity(entity)
            .remove::<WorldTooltipUpdatePending>();
    }
}

pub(super) fn resolve_world_tooltip_updates(mut commands: Commands) {
    commands.run_system_cached(panel::write_panel_anchor_offsets);
    commands.run_system_cached(hana_valence::resolve_anchors);
}

pub(super) fn resize_world_tooltip(
    In(request): In<TooltipResize>,
    mut controllers: Query<(
        &mut Tooltip,
        &MaterializedTooltip,
        &Resolved<FontUnit>,
        &mut Transform,
        &mut GlobalTransform,
        &mut PanelAnchorOffset,
        &mut TooltipPlacementState,
        &WorldTooltipFrameProjection,
    )>,
    mut commands: Commands,
) {
    let Ok((
        mut tooltip,
        materialized,
        font,
        mut pose,
        mut global,
        mut offset,
        mut placement,
        frame,
    )) = controllers.get_mut(request.controller)
    else {
        return;
    };
    if materialized.space() != PanelSpace::World {
        return;
    }
    let unit = font.0.0.meters_per_unit();
    let old_size = tooltip.drawn_label_size.to_meters(unit);
    let new_size = request.label_size.to_meters(unit);
    if !new_size.is_finite()
        || !old_size.is_finite()
        || new_size <= 0.0
        || old_size <= 0.0
        || (new_size - old_size).abs() <= f32::EPSILON
    {
        return;
    }
    let scale = new_size / old_size;
    tooltip.resize_label_size(request.label_size, scale);
    pose.scale *= scale;
    *global = GlobalTransform::from(*pose);
    let mut x = offset.x();
    x.value *= scale;
    let mut y = offset.y();
    y.value *= scale;
    *offset = PanelAnchorOffset::new(x, y).with_z(offset.z());
    placement.attachment = PanelAttachment::new(
        placement.attachment.source_anchor(),
        placement.attachment.target_anchor(),
    )
    .with_offset(*offset);
    commands.entity(request.controller).insert((
        TooltipResizePending {
            layout_scale: frame.layout_scale(),
        },
        WorldTooltipUpdatePending,
    ));
}
