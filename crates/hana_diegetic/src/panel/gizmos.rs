//! Panel debug gizmo rendering — text-bounds overlays.

use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::ChildOf;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Gizmo;
use bevy::prelude::GizmoAsset;
use bevy::prelude::GizmoConfigGroup;
use bevy::prelude::GizmoConfigStore;
use bevy::prelude::GizmoLineConfig;
use bevy::prelude::GizmoLineJoint;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::default;

use super::PanelOwned;
use super::constants::DEBUG_TEXT_GIZMO_COLOR;
use super::constants::DEBUG_TEXT_GIZMO_LINE_WIDTH;
use super::constants::GIZMO_LINE_JOINT_SEGMENTS;
use super::diegetic_panel::ComputedDiegeticPanel;
use super::diegetic_panel::DiegeticPanel;
use super::diegetic_panel::PanelLayout;
use crate::layout::BoundingBox;
use crate::layout::RenderCommandKind;

/// Gizmo group for diegetic panel debug wireframes.
///
/// Enable or disable via Bevy's [`GizmoConfigStore`].
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct DiegeticPanelGizmoGroup;

/// Controls whether debug gizmos (text bounding boxes, element outlines)
/// are drawn. Toggle at runtime to debug layout measurement and positioning.
#[derive(Resource, Default)]
pub enum ShowTextGizmos {
    /// Debug gizmos are not drawn (default).
    #[default]
    Hidden,
    /// Debug gizmos are drawn.
    Shown,
}

/// Marker on gizmo entities spawned by the debug gizmo renderer.
#[derive(Component)]
pub(super) struct DebugGizmoChild;

/// Enables perspective-scaled line widths on panel debug gizmos.
pub(super) fn configure_panel_gizmos(mut config_store: ResMut<GizmoConfigStore>) {
    let (config, _) = config_store.config_mut::<DiegeticPanelGizmoGroup>();
    config.line.perspective = true;
}

struct GizmoRect<'a> {
    bounds:          &'a BoundingBox,
    points_to_world: f32,
    anchor_x:        f32,
    anchor_y:        f32,
    color:           Color,
    line_width:      f32,
}

fn spawn_rect_gizmo(
    commands: &mut Commands,
    panel_entity: Entity,
    gizmo_assets: &mut Assets<GizmoAsset>,
    rect: &GizmoRect<'_>,
) {
    let mut asset = GizmoAsset::default();
    add_rect_to_gizmo(
        &mut asset,
        rect.bounds,
        rect.points_to_world,
        rect.anchor_x,
        rect.anchor_y,
        rect.color,
    );
    let gizmo = Gizmo {
        handle: gizmo_assets.add(asset),
        line_config: GizmoLineConfig {
            width: rect.line_width,
            perspective: false,
            joints: GizmoLineJoint::Round(GIZMO_LINE_JOINT_SEGMENTS),
            ..default()
        },
        ..default()
    };
    commands.entity(panel_entity).with_child((
        DebugGizmoChild,
        gizmo,
        Transform::IDENTITY,
        PanelOwned::from(panel_entity),
    ));
}

/// Renders debug overlays (text bounding boxes, element outlines) as
/// retained gizmos. Only active when [`ShowTextGizmos`] is enabled.
/// Separate from layout gizmos so debug can be toggled independently.
pub(super) fn render_debug_gizmos(
    changed_panels: Query<
        (Entity, &DiegeticPanel, &ComputedDiegeticPanel),
        Changed<ComputedDiegeticPanel>,
    >,
    existing_gizmos: Query<(Entity, &ChildOf), With<DebugGizmoChild>>,
    show_text: Res<ShowTextGizmos>,
    mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
    mut commands: Commands,
) {
    if !matches!(*show_text, ShowTextGizmos::Shown) || changed_panels.is_empty() {
        return;
    }

    for (panel_entity, panel, computed) in &changed_panels {
        let PanelLayout::Solved(result) = computed.layout() else {
            continue;
        };

        let points_to_world = panel.points_to_world();
        despawn_gizmo_children(&mut commands, &existing_gizmos, panel_entity);
        let (anchor_x, anchor_y) = panel.anchor_offsets();

        for cmd in &result.commands {
            if matches!(cmd.kind, RenderCommandKind::Text { .. }) {
                spawn_rect_gizmo(
                    &mut commands,
                    panel_entity,
                    &mut gizmo_assets,
                    &GizmoRect {
                        bounds: &cmd.bounds,
                        points_to_world,
                        anchor_x,
                        anchor_y,
                        color: DEBUG_TEXT_GIZMO_COLOR,
                        line_width: DEBUG_TEXT_GIZMO_LINE_WIDTH,
                    },
                );
            }
        }
    }
}

fn despawn_gizmo_children<T: Component>(
    commands: &mut Commands,
    existing_gizmos: &Query<(Entity, &ChildOf), With<T>>,
    panel_entity: Entity,
) {
    for (entity, child_of) in existing_gizmos {
        if child_of.parent() == panel_entity {
            commands.entity(entity).despawn();
        }
    }
}

/// Adds a rectangle outline to a `GizmoAsset` in panel-local coordinates.
fn add_rect_to_gizmo(
    asset: &mut GizmoAsset,
    bounds: &BoundingBox,
    scale: f32,
    anchor_x: f32,
    anchor_y: f32,
    color: Color,
) {
    let left = bounds.x.mul_add(scale, -anchor_x);
    let right = (bounds.x + bounds.width).mul_add(scale, -anchor_x);
    let top = (-bounds.y).mul_add(scale, anchor_y);
    let bottom = (-(bounds.y + bounds.height)).mul_add(scale, anchor_y);

    let tl = Vec3::new(left, top, 0.0);
    let tr = Vec3::new(right, top, 0.0);
    let br = Vec3::new(right, bottom, 0.0);
    let bl = Vec3::new(left, bottom, 0.0);

    asset.line(tl, tr, color);
    asset.line(tr, br, color);
    asset.line(br, bl, color);
    asset.line(bl, tl, color);
}
