//! Layout-aware adapter over the shared real cursor input helper.
#![allow(
    clippy::expect_used,
    reason = "invalid hover fixtures must fail immediately"
)]
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::picking::hover::PickingInteraction;
use bevy::prelude::*;
use bevy::shader::Shader;

use super::test_pointer;
use crate::ComputedDiegeticPanel;
use crate::DiegeticPanel;
use crate::PanelWidget;
use crate::WidgetOf;
use crate::cascade;
use crate::render;
use crate::render::PanelGeometryPlugin;

/// Installs production picking and actual panel geometry before app completion.
pub(crate) fn install(app: &mut App) {
    test_pointer::install_mesh(app);
    if !app.world().contains_resource::<Assets<Shader>>() {
        app.init_asset::<Shader>();
    }
    if !app.world().contains_resource::<crate::SdfMaterial>() {
        render::seed_default_material_cascades(app);
        app.add_plugins(cascade::cascade_plugin::<crate::SdfMaterial>());
    }
    if !app.is_plugin_added::<PanelGeometryPlugin>() {
        app.add_plugins(PanelGeometryPlugin);
    }
}

/// Centre of the actual visible widget rectangle or mesh target.
fn point(world: &World, target: Entity) -> Vec3 {
    if let (Some(widget), Some(owner)) = (
        world.get::<PanelWidget>(target),
        world.get::<WidgetOf>(target),
    ) {
        let panel = world
            .get::<DiegeticPanel>(owner.panel())
            .expect("widget panel");
        let computed = world
            .get::<ComputedDiegeticPanel>(owner.panel())
            .expect("widget layout");
        let (min, max) = computed
            .visible_widget_bounds(panel)
            .filter(|(id, _, _)| *id == widget.id())
            .map(|(_, _, b)| {
                (
                    Vec2::new(b.x, b.y),
                    Vec2::new(b.x + b.width, b.y + b.height),
                )
            })
            .fold(
                (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
                |(min, max), (lo, hi)| (min.min(lo), max.max(hi)),
            );
        assert!(min.is_finite() && max.is_finite(), "visible widget bounds");
        let centre = min.lerp(max, 0.5);
        let (x, y) = panel.anchor_offsets();
        let scale = panel.points_to_world();
        let local = Vec3::new(
            centre.x.mul_add(scale, -x),
            centre.y.mul_add(-scale, y),
            0.0,
        );
        // Root edits in a reveal-frame test have not propagated yet.
        if world.get::<ChildOf>(owner.panel()).is_none()
            && let Some(pose) = world.get::<Transform>(owner.panel())
        {
            return pose.transform_point(local);
        }
        return world
            .get::<GlobalTransform>(owner.panel())
            .expect("panel pose")
            .transform_point(local);
    }
    world
        .get::<GlobalTransform>(target)
        .expect("target pose")
        .translation()
}

/// Finds the matching window camera and moves the cursor through the real backend.
pub(crate) fn hover(app: &mut App, target: Entity) {
    let world = app.world();
    let owner = world
        .get::<WidgetOf>(target)
        .map_or(target, WidgetOf::panel);
    let layers = world
        .get::<RenderLayers>(owner)
        .cloned()
        .unwrap_or_default();
    let camera = world
        .iter_entities()
        .find(|entity| {
            entity.contains::<Camera>()
                && matches!(
                    entity.get::<RenderTarget>(),
                    Some(RenderTarget::Window(_)) | None
                )
                && entity
                    .get::<RenderLayers>()
                    .cloned()
                    .unwrap_or_default()
                    .intersects(&layers)
        })
        .expect("matching window hover camera")
        .id();
    hover_with(app, camera, target);
}

/// Uses the specified view, including image previews whose cursor is hosted in a window.
pub(crate) fn hover_with(app: &mut App, camera: Entity, target: Entity) {
    let point = point(app.world(), target);
    test_pointer::move_world(app, camera, point);
}

/// Leaves through cursor input and releases a held primary button.
pub(crate) fn leave(app: &mut App) {
    let window = test_pointer::current_window(app.world());
    test_pointer::move_cursor(app, window, Vec2::splat(-1000.0));
    test_pointer::release(app, PointerButton::Primary);
}

/// Updates the input action while keeping ownership and interaction production-driven.
pub(crate) fn interaction(app: &mut App, target: Entity, wanted: PickingInteraction) {
    if wanted == PickingInteraction::None {
        leave(app);
        return;
    }
    let held = app.world().get::<PickingInteraction>(target) == Some(&PickingInteraction::Pressed);
    hover(app, target);
    if wanted == PickingInteraction::Pressed && !held {
        test_pointer::press(app, PointerButton::Primary);
    }
    if wanted == PickingInteraction::Hovered && held {
        test_pointer::release(app, PointerButton::Primary);
    }
}
