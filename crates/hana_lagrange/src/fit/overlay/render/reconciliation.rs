use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::With;
use bevy::prelude::Without;

use super::fit_target_bounds::FitMarginPercents;
use super::visual::FitOverlayVisual;
use crate::fit::overlay::FitOverlay;
use crate::fit::overlay::geometry::FitOverlayEmptyReason;

#[derive(Clone, Copy)]
pub(super) struct RetainedVisualEntity {
    pub(super) entity: Entity,
    pub(super) visual: FitOverlayVisual,
}

pub(super) fn clear_empty_frame(
    commands: &mut Commands,
    camera: Entity,
    reason: FitOverlayEmptyReason,
    visual_query: &Query<(Entity, &FitOverlayVisual)>,
) {
    bevy::log::trace!("clearing fit overlay for {camera:?}: {reason:?}");
    clear_camera_visuals(commands, camera, visual_query);
}

pub(super) fn clear_camera_visuals(
    commands: &mut Commands,
    camera: Entity,
    visual_query: &Query<(Entity, &FitOverlayVisual)>,
) {
    commands.entity(camera).try_remove::<FitMarginPercents>();

    for (entity, visual) in visual_query {
        if visual.camera == camera {
            despawn_visual_root(commands, entity);
        }
    }
}

pub(in crate::fit::overlay) fn deduplicate_fit_overlay_visuals(
    mut commands: Commands,
    visual_query: Query<(Entity, &FitOverlayVisual)>,
) {
    let visuals = retained_visual_entities(&visual_query);
    let mut survivors: Vec<FitOverlayVisual> = Vec::new();
    for RetainedVisualEntity { entity, visual } in visuals {
        if survivors.contains(&visual) {
            despawn_visual_root(&mut commands, entity);
        } else {
            survivors.push(visual);
        }
    }
}

pub(in crate::fit::overlay) fn cleanup_orphan_fit_overlay_visuals(
    mut commands: Commands,
    visual_query: Query<(Entity, &FitOverlayVisual)>,
    fit_overlay_query: Query<(), With<FitOverlay>>,
    stale_margin_query: Query<Entity, (With<FitMarginPercents>, Without<FitOverlay>)>,
) {
    for (entity, visual) in &visual_query {
        if fit_overlay_query.get(visual.camera).is_err() {
            despawn_visual_root(&mut commands, entity);
        }
    }

    for camera in &stale_margin_query {
        commands.entity(camera).try_remove::<FitMarginPercents>();
    }
}

pub(super) fn retained_visual_entities(
    visual_query: &Query<(Entity, &FitOverlayVisual)>,
) -> Vec<RetainedVisualEntity> {
    let mut visuals: Vec<RetainedVisualEntity> = visual_query
        .iter()
        .map(|(entity, visual)| RetainedVisualEntity {
            entity,
            visual: *visual,
        })
        .collect();
    visuals.sort_by_key(|visual| visual.entity);
    visuals
}

pub(super) fn repair_render_layers(
    commands: &mut Commands,
    entity: Entity,
    layers: &RenderLayers,
    current_layers: Option<&RenderLayers>,
) {
    if current_layers != Some(layers) {
        commands.entity(entity).insert(layers.clone());
    }
}

pub(super) fn despawn_visual_root(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).despawn_children().despawn();
}

#[cfg(test)]
mod tests {
    use bevy::camera::visibility::RenderLayers;
    use bevy::prelude::App;
    use bevy::prelude::Update;

    use super::*;
    use crate::fit::overlay::render::visual::FitOverlayVisualKind;

    #[test]
    fn deduplicate_fit_overlay_visuals_keeps_one_visual_per_camera_kind() {
        let mut app = App::new();
        app.add_systems(Update, deduplicate_fit_overlay_visuals);

        let camera = app.world_mut().spawn_empty().id();
        let visual = FitOverlayVisual {
            camera,
            kind: FitOverlayVisualKind::BoundsLabel,
        };
        let first = app.world_mut().spawn(visual).id();
        let second = app.world_mut().spawn(visual).id();

        app.update();

        let first_exists = app.world().get_entity(first).is_ok();
        let second_exists = app.world().get_entity(second).is_ok();
        assert_ne!(first_exists, second_exists);
    }

    #[test]
    fn cleanup_orphan_fit_overlay_visuals_removes_visuals_without_fit_overlay_owner() {
        let mut app = App::new();
        app.add_systems(Update, cleanup_orphan_fit_overlay_visuals);

        let camera = app.world_mut().spawn_empty().id();
        let visual = app
            .world_mut()
            .spawn(FitOverlayVisual {
                camera,
                kind: FitOverlayVisualKind::BoundsLabel,
            })
            .id();

        app.update();

        assert!(app.world().get_entity(visual).is_err());
    }

    #[test]
    fn cleanup_orphan_fit_overlay_visuals_keeps_visuals_with_fit_overlay_owner() {
        let mut app = App::new();
        app.add_systems(Update, cleanup_orphan_fit_overlay_visuals);

        let camera = app.world_mut().spawn(FitOverlay).id();
        let visual = app
            .world_mut()
            .spawn(FitOverlayVisual {
                camera,
                kind: FitOverlayVisualKind::BoundsLabel,
            })
            .id();

        app.update();

        assert!(app.world().get_entity(visual).is_ok());
    }

    #[test]
    fn repair_render_layers_replaces_missing_layers() {
        let mut app = App::new();
        let entity = app.world_mut().spawn_empty().id();
        app.add_systems(Update, move |mut commands: Commands| {
            repair_render_layers(&mut commands, entity, &RenderLayers::layer(3), None);
        });

        app.update();

        assert_eq!(
            app.world().get::<RenderLayers>(entity),
            Some(&RenderLayers::layer(3))
        );
    }
}
