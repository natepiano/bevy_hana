//! `StableTransparency` — camera marker that opts into OIT + `Msaa::Off`
//! for view-angle-stable `AlphaMode::Blend` ordering on coplanar text.
//!
//! # When to use it
//!
//! Slug's analytic coverage AA is the crate default: text is `AlphaMode::Blend`
//! and antialiases per-pixel without MSAA. Blend sorts view-dependently, so
//! coplanar glyphs lying on a shared plane (a paragraph on the ground, labels
//! on a wall) can flip composite order at grazing angles and show a view-angle
//! color shift. [`StableTransparency`] routes that camera through
//! order-independent transparency (OIT), which composites Blend fragments by
//! depth regardless of draw order — the shift goes away without coarsening the
//! coverage AA the way `AlphaMode::AlphaToCoverage` would.
//!
//! # Two paths, opt-in
//!
//! - **Default (not present):** no OIT, MSAA stays on (`Msaa::default()` on every camera). AA'd
//!   mesh edges; coplanar Blend text may shift at angles.
//! - **Opt-in (`StableTransparency` on a `Camera3d`):** three observers in this module manage OIT
//!   and MSAA across every camera that shares the window:
//!   - On add: insert `Msaa::Off` and `OrderIndependentTransparencySettings` on the OIT camera, set
//!     its depth texture to `TEXTURE_BINDING`, and propagate OIT + `Msaa::Off` to every existing
//!     `ScreenSpaceCamera`.
//!   - On any new `ScreenSpaceCamera` spawned afterward (e.g. when a `DiegeticPanel::screen()`
//!     appears mid-app): force OIT + `Msaa::Off` on it too, so the late-spawn case stays
//!     consistent.
//!   - On remove: strip OIT and restore `Msaa::default()` everywhere it forced `Off`.
//!
//! # Why MSAA is forced off on every camera in the window
//!
//! OIT and MSAA cannot coexist on cameras that share a framebuffer. Bevy's OIT
//! plugin panics on a single camera that has both; even a sibling camera with
//! default MSAA stalls the swap chain on macOS Metal, producing a window that
//! only repaints on OS-level events (move it to another monitor and the latest
//! frame appears). The three observers exist to keep that mismatch from
//! happening in either spawn order. For mesh-edge AA alongside OIT, use a
//! post-process AA (FXAA/SMAA/TAA) — MSAA is the one AA incompatible with OIT.

use bevy::camera::Camera3d;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::prelude::Add;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::render::render_resource::TextureUsages;
use bevy::render::view::Msaa;

use crate::screen_space::ScreenSpaceCamera;

/// Sizing factor for Bevy's shared OIT fragment pool: the GPU node buffer holds
/// `viewport_pixels × this` transparent fragments, and every fragment past that
/// budget is **discarded** for the rest of the frame — visible as randomly
/// flashing black blocks wherever the rasterizer tiles lost their fragments.
///
/// Bevy's 4.0 default is too small for diegetic scenes: a close-up camera with
/// the typography overlay stacks glyph quads, overlay boxes, and panels on most
/// pixels, and live-tuning over BRP put that view's demand between 4 and 6
/// fragments per pixel. 8.0 keeps a ~2× margin over the worst observed view and
/// costs 96 bytes of GPU memory per viewport pixel (12-byte nodes, double the
/// default's 48), allocated once and resized with the window. The pool is a
/// GPU-private storage buffer with no CPU copy; on Apple Silicon that is wired
/// unified memory — it counts toward the process footprint (Activity Monitor's
/// Memory column, `footprint`'s graphics category) but never appears in RSS.
const OIT_FRAGMENTS_PER_PIXEL_AVERAGE: f32 = 8.0;

/// Camera marker that opts a `Camera3d` into **Order Independent Transparency**.
///
/// Gives view-angle-stable compositing of
/// [`AlphaMode::Blend`](bevy::prelude::AlphaMode::Blend) (and
/// [`AlphaMode::Premultiplied`](bevy::prelude::AlphaMode::Premultiplied)) text.
///
/// # MSAA is managed while this marker is present
///
/// OIT and MSAA cannot coexist on cameras that share a framebuffer: Bevy's
/// OIT plugin panics on a single camera that has both, and even when only a
/// sibling camera has MSAA the attachment formats mismatch — on macOS Metal
/// the swap chain stalls and the window appears frozen, only repainting on
/// OS-level events (window move, focus change). To prevent that, this crate
/// owns the `Msaa` component on every camera that could share a framebuffer
/// with an OIT camera, for the entire time `StableTransparency` is present:
///
/// 1. **OIT camera turns on**: `on_stable_transparency_added` inserts `Msaa::Off` and
///    `OrderIndependentTransparencySettings` on the OIT camera and propagates OIT + `Msaa::Off` to
///    every existing `ScreenSpaceCamera`.
/// 2. **Screen-space camera spawns later** (e.g. when a `DiegeticPanel::screen()` triggers
///    `setup_screen_space_view`): `on_screen_space_camera_added` detects the active OIT camera and
///    forces OIT + `Msaa::Off` on the new overlay camera before it can render with a non-OIT
///    transparent pipeline.
/// 3. **OIT camera turns off**: `on_stable_transparency_removed` strips OIT and restores
///    `Msaa::default()` on both the OIT camera and every `ScreenSpaceCamera`.
///
/// Net effect: do not set `Msaa` manually on a camera that renders to the
/// same window as a `StableTransparency` camera — this module will
/// overwrite it. If you need mesh-edge AA in the scene, use a post-process
/// AA (FXAA/SMAA/TAA) rather than MSAA.
///
/// Also inserted on the OIT camera: depth texture `TEXTURE_BINDING` usage
/// (required by the OIT resolve pass).
///
/// Use this marker when coplanar text shows a view-angle color shift under
/// the default Blend path. Pair with `AlphaMode::Blend` or
/// `AlphaMode::Premultiplied` on text (the cascade default; `Opaque`/`Mask`
/// bypass OIT), configured via
/// `TextAlpha`, per-panel
/// override ([`DiegeticPanel`](crate::DiegeticPanel)), or per-style
/// override ([`TextStyle`](crate::TextStyle) /
/// [`TextStyle`](crate::TextStyle)).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct StableTransparency;

/// Turns OIT on for a `StableTransparency` camera: inserts
/// `OrderIndependentTransparencySettings` and `Msaa::Off`, sets the depth
/// texture to `TEXTURE_BINDING` (required by the OIT resolve pass), and forces
/// OIT + `Msaa::Off` on every existing `ScreenSpaceCamera` that shares the
/// window.
pub(super) fn on_stable_transparency_added(
    trigger: On<Add, StableTransparency>,
    mut cameras: Query<&mut Camera3d>,
    mut msaa_overlays: Query<Entity, With<ScreenSpaceCamera>>,
    mut commands: Commands,
) {
    let cam = trigger.entity;
    if let Ok(mut camera_3d) = cameras.get_mut(cam) {
        camera_3d.depth_texture_usages.0 |= TextureUsages::TEXTURE_BINDING.bits();
    }
    commands
        .entity(cam)
        .insert((stable_transparency_oit_settings(), Msaa::Off));
    for overlay in &mut msaa_overlays {
        commands
            .entity(overlay)
            .insert((stable_transparency_oit_settings(), Msaa::Off));
    }
}

/// Turns OIT off when `StableTransparency` is removed: strips
/// `OrderIndependentTransparencySettings` and restores `Msaa::default()` on the
/// camera and on every `ScreenSpaceCamera`.
pub(super) fn on_stable_transparency_removed(
    trigger: On<Remove, StableTransparency>,
    mut overlays: Query<Entity, With<ScreenSpaceCamera>>,
    mut commands: Commands,
) {
    let cam = trigger.entity;
    commands
        .entity(cam)
        .remove::<OrderIndependentTransparencySettings>()
        .insert(Msaa::default());
    for overlay in &mut overlays {
        commands
            .entity(overlay)
            .remove::<OrderIndependentTransparencySettings>()
            .insert(Msaa::default());
    }
}

/// Fires when a new `ScreenSpaceCamera` is added. If any camera already has
/// `OrderIndependentTransparencySettings`, the framebuffer is in OIT mode and
/// every camera sharing it must run with OIT + `Msaa::Off`. The OIT half matters
/// for screen-space panel ordering: same-z-index SDF, text and geometry batches keep
/// Bevy's hardware `depth_bias` at one value per `DrawZIndexRank`, so the
/// overlay camera must use the shader-authored OIT offsets instead of non-OIT
/// transparent draw-call sorting.
pub(super) fn on_screen_space_camera_added(
    trigger: On<Add, ScreenSpaceCamera>,
    oit_cameras: Query<Entity, With<OrderIndependentTransparencySettings>>,
    mut commands: Commands,
) {
    if oit_cameras.iter().next().is_none() {
        return;
    }
    commands
        .entity(trigger.entity)
        .insert((stable_transparency_oit_settings(), Msaa::Off));
}

fn stable_transparency_oit_settings() -> OrderIndependentTransparencySettings {
    OrderIndependentTransparencySettings {
        fragments_per_pixel_average: OIT_FRAGMENTS_PER_PIXEL_AVERAGE,
        ..default()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected missing ECS state"
)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::MinimalPlugins;
    use bevy::window::PrimaryWindow;
    use bevy::window::Window;

    use super::*;
    use crate::DiegeticPanel;
    use crate::Px;
    use crate::screen_space::ScreenSpacePlugin;

    fn transparency_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(ScreenSpacePlugin);
        app.add_observer(on_stable_transparency_added);
        app.add_observer(on_stable_transparency_removed);
        app.add_observer(on_screen_space_camera_added);
        app.world_mut().spawn((
            Window {
                resolution: (800_u32, 600_u32).into(),
                ..default()
            },
            PrimaryWindow,
        ));
        app
    }

    fn spawn_screen_panel(app: &mut App) -> Entity {
        let panel = DiegeticPanel::screen()
            .size(Px(100.0), Px(50.0))
            .layout(|_| {})
            .build()
            .expect("screen panel builds");
        app.world_mut().spawn(panel).id()
    }

    fn spawn_stable_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((Camera3d::default(), StableTransparency))
            .id()
    }

    fn screen_camera_entity(app: &mut App) -> Entity {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<ScreenSpaceCamera>>();
        query.single(app.world()).expect("one screen-space camera")
    }

    #[track_caller]
    fn assert_oit_msaa_off(app: &App, entity: Entity) {
        assert!(
            app.world()
                .get::<OrderIndependentTransparencySettings>(entity)
                .is_some(),
            "camera should have OIT settings",
        );
        assert_eq!(app.world().get::<Msaa>(entity), Some(&Msaa::Off));
    }

    #[test]
    fn stable_transparency_updates_existing_screen_space_camera_for_oit() {
        let mut app = transparency_test_app();
        spawn_screen_panel(&mut app);
        app.update();
        let overlay = screen_camera_entity(&mut app);
        assert!(
            app.world()
                .get::<OrderIndependentTransparencySettings>(overlay)
                .is_none(),
            "screen camera starts without OIT before StableTransparency is active",
        );

        let stable = spawn_stable_camera(&mut app);
        app.update();

        assert_oit_msaa_off(&app, stable);
        assert_oit_msaa_off(&app, overlay);
    }

    #[test]
    fn late_screen_space_camera_inherits_active_stable_transparency_oit() {
        let mut app = transparency_test_app();
        let stable = spawn_stable_camera(&mut app);
        app.update();

        spawn_screen_panel(&mut app);
        app.update();
        let overlay = screen_camera_entity(&mut app);

        assert_oit_msaa_off(&app, stable);
        assert_oit_msaa_off(&app, overlay);
    }
}
