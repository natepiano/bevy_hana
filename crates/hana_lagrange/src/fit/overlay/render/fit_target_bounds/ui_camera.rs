use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::prelude::Camera;
use bevy::prelude::Camera2d;
use bevy::prelude::Camera3d;
use bevy::prelude::Entity;

pub(super) fn label_ui_camera<'a>(
    source_camera: Entity,
    source_target: &NormalizedRenderTarget,
    primary_window: Option<Entity>,
    cameras: impl IntoIterator<
        Item = (
            Entity,
            &'a Camera,
            &'a RenderTarget,
            Option<&'a Camera2d>,
            Option<&'a Camera3d>,
        ),
    >,
) -> Entity {
    // Bevy UI only extracts Camera2d/Camera3d views. The source camera remains
    // the fallback when it is the only suitable same-target UI camera.
    cameras
        .into_iter()
        .filter(|(_, camera, target, camera_2d, camera_3d)| {
            camera.is_active
                && target.normalize(primary_window).as_ref() == Some(source_target)
                && (camera_2d.is_some() || camera_3d.is_some())
        })
        .max_by_key(|(entity, camera, _, _, _)| (camera.order, *entity))
        .map_or(source_camera, |(entity, _, _, _, _)| entity)
}

#[cfg(test)]
mod tests {
    use bevy::prelude::World;
    use bevy::prelude::default;
    use bevy::window::WindowRef;

    use super::*;

    #[test]
    fn label_ui_camera_uses_top_camera_on_same_primary_window() -> Result<(), &'static str> {
        let mut world = World::new();
        let primary_window = world.spawn_empty().id();
        let other_window_entity = world.spawn_empty().id();
        let source_camera = world.spawn_empty().id();
        let overlay_camera = world.spawn_empty().id();
        let inactive_camera = world.spawn_empty().id();
        let other_window_camera = world.spawn_empty().id();

        let source = camera(0, true);
        let overlay = camera(100, true);
        let inactive = camera(200, false);
        let other_window = camera(300, true);
        let source_3d = Camera3d::default();
        let overlay_3d = Camera3d::default();
        let inactive_3d = Camera3d::default();
        let other_window_3d = Camera3d::default();

        let source_target = RenderTarget::Window(WindowRef::Primary);
        let overlay_target = RenderTarget::Window(WindowRef::Entity(primary_window));
        let other_target = RenderTarget::Window(WindowRef::Entity(other_window_entity));
        let normalized_source_target = source_target
            .normalize(Some(primary_window))
            .ok_or("source target should normalize")?;
        let cameras = [
            (
                source_camera,
                &source,
                &source_target,
                None,
                Some(&source_3d),
            ),
            (
                overlay_camera,
                &overlay,
                &overlay_target,
                None,
                Some(&overlay_3d),
            ),
            (
                inactive_camera,
                &inactive,
                &overlay_target,
                None,
                Some(&inactive_3d),
            ),
            (
                other_window_camera,
                &other_window,
                &other_target,
                None,
                Some(&other_window_3d),
            ),
        ];

        assert_eq!(
            label_ui_camera(
                source_camera,
                &normalized_source_target,
                Some(primary_window),
                cameras
            ),
            overlay_camera
        );
        Ok(())
    }

    #[test]
    fn label_ui_camera_falls_back_to_ui_renderable_source_camera() -> Result<(), &'static str> {
        let mut world = World::new();
        let primary_window = world.spawn_empty().id();
        let source_camera = world.spawn_empty().id();
        let non_ui_camera = world.spawn_empty().id();

        let source = camera(0, true);
        let non_ui = camera(500, true);
        let source_3d = Camera3d::default();

        let target = RenderTarget::Window(WindowRef::Primary);
        let normalized_target = target
            .normalize(Some(primary_window))
            .ok_or("target should normalize")?;
        let cameras = [
            (source_camera, &source, &target, None, Some(&source_3d)),
            (non_ui_camera, &non_ui, &target, None, None),
        ];

        assert_eq!(
            label_ui_camera(
                source_camera,
                &normalized_target,
                Some(primary_window),
                cameras
            ),
            source_camera
        );
        Ok(())
    }

    #[test]
    fn label_ui_camera_skips_non_ui_render_camera_on_same_target() -> Result<(), &'static str> {
        let mut world = World::new();
        let primary_window = world.spawn_empty().id();
        let source_camera = world.spawn_empty().id();
        let non_ui_camera = world.spawn_empty().id();
        let overlay_camera = world.spawn_empty().id();

        let source = camera(0, true);
        let non_ui = camera(500, true);
        let overlay = camera(100, true);
        let source_3d = Camera3d::default();
        let overlay_3d = Camera3d::default();

        let target = RenderTarget::Window(WindowRef::Primary);
        let normalized_target = target
            .normalize(Some(primary_window))
            .ok_or("target should normalize")?;
        let cameras = [
            (source_camera, &source, &target, None, Some(&source_3d)),
            (non_ui_camera, &non_ui, &target, None, None),
            (overlay_camera, &overlay, &target, None, Some(&overlay_3d)),
        ];

        assert_eq!(
            label_ui_camera(
                source_camera,
                &normalized_target,
                Some(primary_window),
                cameras
            ),
            overlay_camera
        );
        Ok(())
    }

    fn camera(order: isize, is_active: bool) -> Camera {
        Camera {
            order,
            is_active,
            ..default()
        }
    }
}
