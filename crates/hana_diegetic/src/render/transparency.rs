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

use bevy::app::Last;
use bevy::asset::AssetEvent;
use bevy::asset::AssetEventSystems;
use bevy::asset::Assets;
use bevy::camera::Camera;
use bevy::camera::Camera3d;
use bevy::core_pipeline::oit::OitFragmentNode;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::ecs::message::MessageReader;
use bevy::math::UVec2;
use bevy::prelude::Add;
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::DetectChangesMut;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Local;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Remove;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Shader;
use bevy::prelude::With;
use bevy::prelude::default;
use bevy::render::render_resource::TextureUsages;
use bevy::render::renderer::RenderDevice;
use bevy::render::view::Msaa;
use bevy::shader::ShaderImport;
use bevy::shader::Source;
use hana_kana::ToF32;
use hana_kana::ToF64;
use hana_kana::ToUsize;

use crate::screen_space::ScreenSpaceCamera;

/// Sizing factor for Bevy's shared OIT fragment pool, in twelve-byte allocation
/// units per viewport pixel. Every fragment past the pool's node capacity is
/// **discarded** for the rest of the frame — visible as randomly
/// flashing black blocks wherever the rasterizer tiles lost their fragments.
///
/// Bevy's 4.0 default is too small for diegetic scenes: a close-up camera with
/// the typography overlay stacks glyph quads, overlay boxes, and panels on most
/// pixels, and live-tuning over BRP put that view's demand between 4 and 6
/// fragments per pixel. Six actual sixteen-byte nodes cover that demand within
/// a budget of 96 bytes per viewport pixel (versus Bevy's default 48),
/// allocated once and resized with the window. The pool is a
/// GPU-private storage buffer with no CPU copy; on Apple Silicon that is wired
/// unified memory — it counts toward the process footprint (Activity Monitor's
/// Memory column, `footprint`'s graphics category) but never appears in RSS.
// Eight twelve-byte allocation units hold six sixteen-byte nodes. Keeping the
// existing byte budget avoids increasing VRAM pressure across concurrent apps.
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

/// Installs separate physical-depth and local-paint keys in Bevy's OIT library. All OIT
/// writers and the resolve pass must use the same encoding. Updating the
/// original asset keeps its import identity and dependent pipelines intact.
/// Asset events also cover loading after plugin build and shader hot reload.
pub(super) fn install_oit_depth_precision(app: &mut App) {
    if app.world().contains_resource::<Assets<Shader>>() {
        app.add_message::<AssetEvent<Shader>>();
        app.add_systems(Last, preserve_oit_depth_precision.after(AssetEventSystems));
    }
}

fn preserve_oit_depth_precision(
    mut events: MessageReader<AssetEvent<Shader>>,
    mut shaders: ResMut<Assets<Shader>>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::Modified { id }) = event else {
            continue;
        };
        let Some(shader) = shaders.get(*id) else {
            continue;
        };
        let Source::Wesl(source) = &shader.source else {
            continue;
        };
        let replacement = match &shader.import_path {
            ShaderImport::Custom(path) if path == "bevy_core_pipeline::oit::draw" => {
                if source.contains("struct OitPaintNode") {
                    continue;
                }
                include_str!("../shaders/oit.wesl").to_owned()
            },
            ShaderImport::Custom(path) if path == "bevy_pbr::render::mesh_view_bindings" => {
                // Preserve Bevy's binding and allocation, exposing its storage
                // as words so the library and resolve share sixteen-byte nodes.
                if !source.contains("array<types::OitFragmentNode>") {
                    continue;
                }
                source.replace("array<types::OitFragmentNode>", "array<u32>")
            },
            _ if shader
                .path
                .ends_with("bevy_core_pipeline/oit/resolve/oit_resolve.wesl") =>
            {
                if source.contains("oit_in_front(") {
                    continue;
                }
                include_str!("../shaders/oit_resolve.wesl").to_owned()
            },
            _ => continue,
        };
        let parsed = Shader::from_wesl(replacement, shader.path.clone());
        if let Some(mut shader) = shaders.get_mut(*id) {
            shader.source = parsed.source;
            shader.imports = parsed.imports;
        }
    }
}

/// Turns OIT on for a `StableTransparency` camera: inserts
/// `OrderIndependentTransparencySettings` and `Msaa::Off`, sets the depth
/// texture to `TEXTURE_BINDING` (required by the OIT resolve pass), and forces
/// OIT + `Msaa::Off` on every existing `ScreenSpaceCamera` that shares the
/// window.
pub(super) fn on_stable_transparency_added(
    trigger: On<Add<StableTransparency>>,
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
/// camera, and on every `ScreenSpaceCamera` once no other `StableTransparency`
/// camera remains.
///
/// An `On<Remove<StableTransparency>>` observer runs while the component is still on its entity,
/// so the remaining-camera check skips the triggering camera.
pub(super) fn on_stable_transparency_removed(
    trigger: On<Remove<StableTransparency>>,
    stable_cameras: Query<Entity, With<StableTransparency>>,
    mut overlays: Query<Entity, With<ScreenSpaceCamera>>,
    mut commands: Commands,
) {
    let cam = trigger.entity;
    commands
        .entity(cam)
        .remove::<OrderIndependentTransparencySettings>()
        .insert(Msaa::default());
    if stable_cameras.iter().any(|stable| stable != cam) {
        return;
    }
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
    trigger: On<Add<ScreenSpaceCamera>>,
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

/// Keeps Bevy's shared OIT fragment pool inside the device's storage-binding limit.
///
/// Bevy allocates one node pool for every OIT camera: the largest camera's
/// physical pixels × the largest `fragments_per_pixel_average` × the size of an
/// [`OitFragmentNode`], bound whole as one storage buffer. A large window on a
/// scale-2 display passes the device limit, and Bevy then quits on a validation
/// error before the first frame. Bevy sizes the pool from active cameras only;
/// this counts every OIT camera, so a camera that turns active never grows the
/// pool past the limit. When the window grows past the limit, every camera's
/// average drops to the most the device holds; when it fits again, every camera
/// returns to [`OIT_FRAGMENTS_PER_PIXEL_AVERAGE`]. Between those changes an
/// average set by hand stays, unless it passes the limit.
///
/// This caps the node pool only. Bevy also binds a heads buffer of 4 bytes per
/// pixel, 150.4 MB at 6880 × 5464, and no average can shrink it.
pub(super) fn fit_oit_fragment_pool_to_device(
    render_device: Option<Res<RenderDevice>>,
    mut cameras: Query<(&Camera, &mut OrderIndependentTransparencySettings)>,
    mut last_fit: Local<LastComputedPoolTarget>,
) {
    let Some(render_device) = render_device else {
        return;
    };
    let limits = render_device.limits();
    let largest = cameras
        .iter()
        .filter_map(|(camera, _)| camera.physical_target_size())
        .fold(UVec2::ZERO, UVec2::max);
    let ceiling = oit_fragment_ceiling(
        largest,
        limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size),
    );
    let fit = OIT_FRAGMENTS_PER_PIXEL_AVERAGE.min(ceiling);
    let fit_changed = last_fit.differs_from(fit);
    *last_fit = LastComputedPoolTarget::Computed(fit);
    for (_, settings) in &mut cameras {
        let average = settings.fragments_per_pixel_average;
        let next = if fit_changed {
            fit
        } else {
            average.min(ceiling)
        };
        settings
            .map_unchanged(|settings| &mut settings.fragments_per_pixel_average)
            .set_if_neq(next);
    }
}

/// The pool target [`fit_oit_fragment_pool_to_device`] last computed. A changed
/// target is written to every OIT camera; an unchanged one only lowers a camera
/// above the device ceiling, so a camera's own lower average stays.
#[derive(Default, Clone, Copy)]
pub(super) enum LastComputedPoolTarget {
    /// The fitter has not run yet, so its first target reaches every camera.
    #[default]
    NeverComputed,
    /// The fitter last computed this average.
    Computed(f32),
}

impl LastComputedPoolTarget {
    /// Whether `fit` differs from the last computed target.
    fn differs_from(self, fit: f32) -> bool {
        match self {
            Self::NeverComputed => true,
            Self::Computed(last) => last.total_cmp(&fit).is_ne(),
        }
    }
}

/// The largest `fragments_per_pixel_average` whose node pool for `size` physical
/// pixels fits in `binding_limit` bytes, checked with the f32 arithmetic Bevy
/// sizes the pool with.
fn oit_fragment_ceiling(size: UVec2, binding_limit: u64) -> f32 {
    let pixels = size.x.saturating_mul(size.y);
    if pixels == 0 {
        return f32::INFINITY;
    }
    let nodes = usize::try_from(binding_limit).unwrap_or(usize::MAX) / size_of::<OitFragmentNode>();
    let mut ceiling = (nodes.to_f64() / pixels.to_f64()).to_f32();
    while oit_pool_nodes(pixels, ceiling) > nodes {
        ceiling = ceiling.next_down();
    }
    ceiling
}

/// The node count Bevy allocates for `pixels` at `fragments_per_pixel_average`.
fn oit_pool_nodes(pixels: u32, fragments_per_pixel_average: f32) -> usize {
    (pixels.to_f32() * fragments_per_pixel_average).to_usize()
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
    use bevy::asset::AssetApp;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::App;
    use bevy::prelude::MinimalPlugins;
    use bevy::window::PrimaryWindow;
    use bevy::window::Window;

    use super::*;
    use crate::DiegeticPanel;
    use crate::Px;
    use crate::screen_space::ScreenSpacePlugin;

    #[test]
    fn partial_app_without_shader_asset_messages_can_update() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Assets<Shader>>();
        install_oit_depth_precision(&mut app);
        app.update();
        app.update();
    }

    #[test]
    fn oit_depth_library_is_ready_on_its_first_frame_and_after_reload() -> Result<(), String> {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_asset::<Shader>();
        install_oit_depth_precision(&mut app);
        let original = Shader::from_wesl(
            "fn pack_24bit_depth_8bit_alpha(depth: f32, alpha: f32) -> u32 { return 0u; }",
            "embedded://bevy_core_pipeline/oit/draw.wesl",
        );
        let handle = app
            .world_mut()
            .resource_mut::<Assets<Shader>>()
            .add(original.clone());
        let resolve = Shader::from_wesl(
            "@group(0) @binding(0) var<uniform> view: View;\n\
             fn viewport_width() -> f32 { return view.viewport.z; }",
            "bevy_core_pipeline/oit/resolve/oit_resolve.wesl",
        );
        let resolve_handle = app
            .world_mut()
            .resource_mut::<Assets<Shader>>()
            .add(resolve.clone());
        let bindings = Shader::from_wesl(
            "@group(0) @binding(28) var<storage, read_write> oit_nodes: array<types::OitFragmentNode>;",
            "embedded://bevy_pbr/render/mesh_view_bindings.wesl",
        );
        let binding_handle = app
            .world_mut()
            .resource_mut::<Assets<Shader>>()
            .add(bindings.clone());
        for frame in 0..2 {
            if frame == 1 {
                app.world_mut()
                    .resource_mut::<Assets<Shader>>()
                    .insert(handle.id(), original.clone())
                    .expect("live shader handle");
                app.world_mut()
                    .resource_mut::<Assets<Shader>>()
                    .insert(resolve_handle.id(), resolve.clone())
                    .expect("live resolve shader handle");
            }
            if frame == 1 {
                app.world_mut()
                    .resource_mut::<Assets<Shader>>()
                    .insert(binding_handle.id(), bindings.clone())
                    .expect("live bindings handle");
            }
            app.update();
            let shaders = app.world().resource::<Assets<Shader>>();
            let shader = shaders.get(&handle).expect("OIT library");
            let Source::Wesl(source) = &shader.source else {
                return Err("expected a WESL library".into());
            };
            assert!(source.contains("fn oit_draw_layer("), "frame {frame}");
            assert!(source.contains("fn oit_in_front("), "frame {frame}");
            assert_eq!(shader.import_path, original.import_path);
            let shader = shaders.get(&binding_handle).expect("OIT bindings");
            let Source::Wesl(source) = &shader.source else {
                return Err("expected WESL bindings".into());
            };
            assert!(source.contains("oit_nodes: array<u32>"));
            assert!(!source.contains("array<types::OitFragmentNode>"));
            let shader = shaders.get(&resolve_handle).expect("OIT resolve shader");
            let Source::Wesl(source) = &shader.source else {
                return Err("expected a WESL resolve shader".into());
            };
            assert!(source.contains("nodes: array<u32>"));
            assert!(source.contains("oit_in_front("));
            assert!(source.contains("var<uniform> view:"));
            assert!(shader.imports.contains(&ShaderImport::Custom(
                "bevy_core_pipeline::oit::draw".into()
            )));
        }
        Ok(())
    }

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

    /// The binding limits the node pool must fit under: Apple Silicon's 2 GiB, a
    /// discrete card's 4 GiB, and the 128 MiB of CI's software device. The cap
    /// covers the node pool only: Bevy's heads buffer, 4 bytes per pixel, is
    /// 150.4 MB at 6880 × 5464 and passes CI's 128 MiB on its own.
    const BINDING_LIMITS: [u64; 3] = [1 << 31, (1 << 32) - 1, 1 << 27];

    #[test]
    fn oit_nodes_keep_the_standard_window_byte_budget() {
        // The default window and modulation captures share one pool, sized
        // for their largest viewport, at 96 B/pixel.
        for size in [UVec2::new(1280, 720), UVec2::new(1440, 900)] {
            let pixels = size.x * size.y;
            let settings = stable_transparency_oit_settings();
            let bytes = oit_pool_nodes(pixels, settings.fragments_per_pixel_average)
                * size_of::<OitFragmentNode>();
            let budget = pixels.to_usize() * 96;
            assert!(
                bytes <= budget,
                "{size:?}: oit_nodes asks for {bytes} bytes, above its {budget}-byte budget"
            );
            // The observed panel demand is four to six 16-byte nodes/pixel.
            assert!(bytes / 16 >= pixels.to_usize() * 6);
        }
    }

    #[test]
    fn fragment_ceiling_keeps_a_scale_two_window_pool_inside_the_binding_limit() {
        let size = UVec2::new(6880, 5464);
        let pixels = size.x * size.y;
        for limit in BINDING_LIMITS {
            let ceiling = oit_fragment_ceiling(size, limit);
            let limit = usize::try_from(limit).expect("binding limit fits in usize");
            let node_bytes = size_of::<OitFragmentNode>();
            assert!(oit_pool_nodes(pixels, ceiling) * node_bytes <= limit);
            let exact = (limit / node_bytes).to_f64() / pixels.to_f64();
            assert!(f64::from(ceiling) > exact * 0.999);
        }
    }

    #[test]
    fn fragment_ceiling_leaves_a_window_that_fits_at_full_average() {
        assert!(
            oit_fragment_ceiling(UVec2::new(1280, 720), 1 << 27) > OIT_FRAGMENTS_PER_PIXEL_AVERAGE
        );
        assert!(oit_fragment_ceiling(UVec2::ZERO, 1 << 27).is_infinite());
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
    fn removing_one_stable_camera_keeps_oit_while_another_remains() {
        let mut app = transparency_test_app();
        spawn_screen_panel(&mut app);
        let first = spawn_stable_camera(&mut app);
        let second = spawn_stable_camera(&mut app);
        app.update();
        let overlay = screen_camera_entity(&mut app);

        app.world_mut()
            .entity_mut(first)
            .remove::<StableTransparency>();
        app.update();

        assert!(
            app.world()
                .get::<OrderIndependentTransparencySettings>(first)
                .is_none(),
            "the camera that lost StableTransparency should leave OIT",
        );
        assert_eq!(app.world().get::<Msaa>(first), Some(&Msaa::default()));
        assert_oit_msaa_off(&app, second);
        assert_oit_msaa_off(&app, overlay);

        app.world_mut()
            .entity_mut(second)
            .remove::<StableTransparency>();
        app.update();

        for camera in [second, overlay] {
            assert!(
                app.world()
                    .get::<OrderIndependentTransparencySettings>(camera)
                    .is_none(),
                "OIT should end once no StableTransparency camera remains",
            );
            assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::default()));
        }
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
