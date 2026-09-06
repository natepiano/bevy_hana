//! Keeps `WindowResized` in agreement with `Window.resolution` when this crate resizes a window.
//!
//! # Why this exists
//!
//! `bevy_render`'s `camera_system` takes *whether* a camera needs a new
//! `physical_target_size` from the `WindowResized` / `WindowScaleFactorChanged` **message stream**,
//! then reads the new size from the **live `Window` component**
//! (`bevy_render/src/camera.rs`, `RenderTarget::get_render_target_info`). The two agree for an
//! OS-driven resize because winit writes the component and emits the message on the same frame.
//!
//! Restore writes `Window.resolution` directly
//! (`restore/target_position/application.rs`) and winit echoes the resize back a frame later. For
//! that one frame the component is ahead of the message stream, so `camera_system` sees no resize
//! message and refreshes no camera on the window. Any camera admitted through the gate for an
//! unrelated reason — an `is_added()` camera, a `Projection` some other system marked changed —
//! still reads the live component and adopts the new size while its siblings keep the old one.
//!
//! Two cameras on one window then hold different `physical_target_size` values.
//! `prepare_view_targets` keys the shared main color texture on
//! `(target, usages, format, msaa)` — not on size — so both
//! cameras receive one color texture sized for whichever was prepared first, while
//! `prepare_core_3d_depth_textures` allocates each camera's depth texture at that camera's own
//! size. The resulting render pass pairs mismatched attachments, wgpu rejects it, and the
//! application exits.
//!
//! Announcing the resize this crate performed restores the invariant winit maintains: a
//! `Window.resolution` change is always accompanied by a `WindowResized`. Every camera on the
//! window then updates on the same frame.
//!
//! Both underlying defects are in `bevy_render` and are unchanged in 0.19.0, 0.19.1, and upstream
//! `main`. This module removes the precondition they need; it does not fix them.
//!
//! # Why this observes the component instead of routing every write
//!
//! [`announce_unpublished_resizes`] observes `Window.resolution` itself, so a resize written from a
//! call site that does not exist yet is still announced. Routing every write through one helper
//! would cover only the call sites that call it.

use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::MessageWriter;
use bevy::prelude::Query;
use bevy::prelude::UVec2;
use bevy::prelude::Window;
use bevy::prelude::trace;
use bevy::window::WindowResized;

/// The physical size [`announce_unpublished_resizes`] last published for this window.
///
/// Seeded without announcing on first observation: a window's initial size reaches `camera_system`
/// through `WindowCreated`.
#[derive(Component, Debug)]
pub(crate) struct AnnouncedPhysicalSize(UVec2);

/// Emit `WindowResized` for any window whose physical size changed without one.
///
/// Runs in `PostUpdate` before `CameraUpdateSystems`, so a resolution written anywhere in `First`,
/// `PreUpdate`, or `Update` reaches `camera_system` on the same frame it was written.
///
/// A duplicate announcement is harmless — winit itself emits `WindowResized` more than once for a
/// single scale-factor change — so this makes no attempt to detect whether winit already reported
/// the resize. It could not: `MessageReader<WindowResized>` and `MessageWriter<WindowResized>`
/// cannot coexist in one system.
pub(crate) fn announce_unpublished_resizes(
    mut commands: Commands,
    mut resized: MessageWriter<WindowResized>,
    mut windows: Query<(Entity, &Window, Option<&mut AnnouncedPhysicalSize>)>,
) {
    for (entity, window, announced) in &mut windows {
        let physical_size = window.resolution.physical_size();
        match announced {
            Some(mut announced) if announced.0 != physical_size => {
                announced.0 = physical_size;
                trace!(
                    "[announce_unpublished_resizes] announcing {physical_size:?} for {entity} \
                     ahead of the winit echo"
                );
                resized.write(WindowResized {
                    window: entity,
                    width:  window.resolution.width(),
                    height: window.resolution.height(),
                });
            },
            Some(_) => {},
            None => {
                commands
                    .entity(entity)
                    .insert(AnnouncedPhysicalSize(physical_size));
            },
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::Messages;
    use bevy::prelude::PostUpdate;
    use bevy::prelude::With;
    use bevy::prelude::default;
    use bevy::window::WindowPlugin;
    use bevy::window::WindowResolution;
    use hana_kana::ToF32;
    use hana_kana::ToU32;

    use super::*;

    /// Clamp a `WindowResized` logical extent to `u32`'s range before truncating its fraction.
    fn logical_extent(value: f32) -> u32 {
        let nonnegative = value.max(0.0);
        if nonnegative >= u32::MAX.to_f32() {
            u32::MAX
        } else {
            nonnegative.to_u32()
        }
    }

    /// An application holding one window, with the guard registered where the plugin puts it.
    fn windowed_app() -> App {
        let mut app = App::new();
        app.add_plugins(WindowPlugin::default())
            .add_systems(PostUpdate, announce_unpublished_resizes);
        app
    }

    fn window_entity(app: &mut App) -> Entity {
        let mut query = app.world_mut().query_filtered::<Entity, With<Window>>();
        query
            .single(app.world())
            .expect("the application holds one window")
    }

    fn resize(app: &mut App, entity: Entity, physical_width: u32, physical_height: u32) {
        app.world_mut()
            .get_mut::<Window>(entity)
            .expect("the window entity holds a window")
            .resolution
            .set_physical_resolution(physical_width, physical_height);
    }

    /// Announced sizes, as `(window, logical size)`. Headless windows keep scale factor 1.0, so
    /// these equal the physical sizes the test wrote.
    fn announcements(app: &App) -> Vec<(Entity, UVec2)> {
        let messages = app.world().resource::<Messages<WindowResized>>();
        let mut cursor = messages.get_cursor();
        cursor
            .read(messages)
            .map(|resized| {
                (
                    resized.window,
                    UVec2::new(
                        logical_extent(resized.width),
                        logical_extent(resized.height),
                    ),
                )
            })
            .collect()
    }

    fn clear_announcements(app: &mut App) {
        app.world_mut()
            .resource_mut::<Messages<WindowResized>>()
            .clear();
    }

    #[test]
    fn announces_a_resolution_written_without_a_message() {
        let mut app = windowed_app();
        app.update();
        let entity = window_entity(&mut app);
        clear_announcements(&mut app);

        resize(&mut app, entity, 3_456, 2_104);
        app.update();

        assert_eq!(
            announcements(&app),
            vec![(entity, UVec2::new(3_456, 2_104))]
        );
    }

    #[test]
    fn announces_nothing_for_a_window_whose_size_did_not_change() {
        let mut app = windowed_app();
        app.update();
        clear_announcements(&mut app);

        app.update();

        assert!(announcements(&app).is_empty());
    }

    #[test]
    fn announces_nothing_on_the_frame_a_window_first_appears() {
        let mut app = App::new();
        app.add_plugins(WindowPlugin {
            primary_window: None,
            ..default()
        })
        .add_systems(PostUpdate, announce_unpublished_resizes);
        app.world_mut().spawn(Window {
            resolution: WindowResolution::new(3_456, 2_104),
            ..default()
        });

        app.update();

        assert!(announcements(&app).is_empty());
    }

    #[test]
    fn announces_each_change_once() {
        let mut app = windowed_app();
        app.update();
        let entity = window_entity(&mut app);
        clear_announcements(&mut app);

        resize(&mut app, entity, 3_456, 2_104);
        app.update();
        clear_announcements(&mut app);
        app.update();

        assert!(announcements(&app).is_empty());
    }
}
