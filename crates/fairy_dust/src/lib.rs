//! Workspace example helper for `bevy_hana`.
//!
//!
//! Use [`sprinkle_example`] to construct a [`SprinkleBuilder`] before Fairy
//! Dust installs its baseline plugins. The first builder operation installs
//! `DefaultPlugins` with a quiet log filter, then the chain opts into specific
//! dev conveniences:
//!
//! ```ignore
//! fairy_dust::sprinkle_example()
//!     .with_orbit_cam_preset_pose(
//!         OrbitCamPose {
//!             focus:  Vec3::ZERO,
//!             yaw:    0.0,
//!             pitch:  0.3,
//!             radius: 5.0,
//!         },
//!         OrbitCamPreset::blender_like(),
//!     )
//!     .with_stable_transparency()       // only callable after with_orbit_cam_*
//!     .with_save_window_position()
//!     .with_brp_extras()
//!     .with_camera_control_panel()
//!     .add_systems(Startup, setup)
//!     .run();
//! ```
//!
//! ## Typestate
//!
//! The builder has independent baseline and orbit-camera typestates. The
//! baseline starts as [`AssetRootPending`], where
//! [`SprinkleBuilder::with_asset_root`] can configure `AssetPlugin` before
//! `DefaultPlugins` is installed. That method and every ordinary builder
//! operation return [`BaselineInstalled`], where `with_asset_root` is no longer
//! available.
//!
//! The orbit-camera marker starts as [`NoOrbitCam`] and transitions to
//! [`WithOrbitCam`]. Methods that act on the spawned `OrbitCam` entity (such as
//! [`SprinkleBuilder::with_stable_transparency`]) are only defined on
//! `SprinkleBuilder<WithOrbitCam>`, so calling them before
//! [`SprinkleBuilder::with_orbit_cam_preset`] is a compile error.
//!
//! ## Plugin deduplication
//!
//! Capabilities that share infrastructure (for example a `DiegeticUiPlugin` for
//! HUD panels) ensure the required plugin is registered exactly once via
//! `ensure_plugin`, regardless of how many capabilities pull it in.

mod atmosphere;
mod bloom;
mod brp_extras;
mod builder;
mod camera_control_panel;
mod camera_home;
mod command_palette;
mod connector;
mod constants;
mod cube_spin;
mod environment_map;
mod fold_controls;
mod hdr;
mod keymap;
mod lighting;
mod orbit_cam;
mod panel_insets;
mod primitive;
mod release_hold;
mod restart;
mod restart_camera;
mod save_window_position;
mod screen_panels;
mod screen_space_lights;
mod shortcuts;
mod smaa;
mod transparency;
mod unclamp;
mod widget_drag_claim;

use std::time::Instant;

use bevy::app::PluginGroupBuilder;
use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::App;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::Plugin;
use bevy::prelude::PluginGroup;
use bevy::prelude::Resource;
#[cfg(test)]
use bevy::prelude::default;
#[cfg(test)]
use bevy::render::RenderPlugin;
#[cfg(test)]
use bevy::render::settings::RenderCreation;
#[cfg(test)]
use bevy::render::settings::WgpuSettings;
#[cfg(test)]
use bevy::winit::WinitPlugin;
pub use builder::AssetRootPending;
pub use builder::BaselineInstalled;
pub use builder::CameraHomeBuilder;
pub use builder::NoOrbitCam;
pub use builder::PrimitiveBuilder;
pub use builder::SprinkleBuilder;
pub use builder::StudioLightingBuilder;
pub use builder::TitleBarBuilder;
pub use builder::WithOrbitCam;
pub use camera_control_panel::CameraGuidance;
pub use camera_control_panel::CameraGuidanceAction;
pub use camera_control_panel::CameraGuidanceRow;
pub use camera_home::CameraHomeEntity;
pub use camera_home::CameraHomeTarget;
pub use camera_home::FrameCameraTarget;
pub use camera_home::FrameHomeFace;
pub use camera_home::HomeFace;
pub use command_palette::PaletteBindingPresentation;
pub use command_palette::palette_binding_presentation;
pub use constants::CUBE_FACE_LABEL_SIZE;
pub use constants::CUBE_FACE_PANEL_BLUE;
pub use constants::CUBE_FACE_PANEL_RELEASE_HOLD;
pub use constants::DEFAULT_PANEL_BACKGROUND;
pub use constants::EXAMPLE_CUBE_COLOR;
pub use constants::EXAMPLE_CUBE_SIZE;
pub use constants::LABEL_SIZE;
pub use constants::LOG_FILTER;
pub use constants::TITLE_COLOR;
pub use constants::TITLE_SIZE;
pub use constants::example_cube_on_ground;
pub use cube_spin::CubeSpinConfig;
pub use cube_spin::CubeSpinControl;
pub use cube_spin::CubeSpinMode;
pub use cube_spin::CubeSpinMotion;
pub use cube_spin::CubeSpinTimeSource;
pub use cube_spin::FairyDustCubeSpinTarget;
pub use fold_controls::FairyDustFoldTarget;
pub use fold_controls::FoldControlAction;
pub use fold_controls::FoldControlDiagnostic;
pub use fold_controls::FoldControlDiagnosticReason;
pub use fold_controls::FoldControlDiagnostics;
pub use hana_diegetic::Anchor;
use hana_diegetic::DiegeticUiPlugin;
use hana_lagrange::CameraInputRoutingConfig;
use hana_lagrange::LagrangePlugin;
use hana_lagrange::NoPositionFallback;
pub use hana_lagrange::OrbitCam;
pub use keymap::CommandPaletteKeymap;
pub use keymap::command_palette_recovery_command_id;
pub use keymap::command_palette_recovery_keystroke;
pub use lighting::FairyDustStudioLightingSet;
pub use orbit_cam::FairyDustOrbitCam;
pub use orbit_cam::OrbitCamPose;
pub use orbit_cam::apply_example_orbit_cam_limits;
pub use primitive::CubeFaceLabel;
pub use primitive::CubeFacePanelActivity;
pub use primitive::CubeFacePanelContent;
pub use primitive::CubeFacePanelStyle;
pub use primitive::CubeFaceSpacing;
pub use primitive::Face;
pub use primitive::FairyDustCube;
pub use primitive::cube_face_label;
pub use primitive::cube_face_panel;
pub use primitive::cube_face_panel_material;
pub use primitive::cube_face_panel_tree;
pub use primitive::cube_face_panel_with_tree;
pub use primitive::cube_face_text;
pub use primitive::cube_face_transform;
pub use primitive::set_cube_face_panel_tree;
pub use release_hold::HoldState;
pub use release_hold::ReleaseHold;
pub use restart_camera::RestartCameraRestore;
pub use restart_camera::RestoreWindowAnimation;
pub use screen_panels::ControlActivation;
pub use screen_panels::DescriptionPanel;
pub use screen_panels::SCREEN_PANEL_GAP;
pub use screen_panels::StatsPanelRow;
pub use screen_panels::StatsPanelSection;
pub use screen_panels::TitleBar;
pub use screen_panels::TitleBarControl;
pub use screen_panels::TitleBarOrientation;
pub use screen_panels::TitleBarSegment;
pub use screen_panels::TitleBarStateWriters;
pub use screen_panels::TitleChip;
pub use screen_panels::TitleChipActivation;
pub use screen_panels::diegetic_stats_panel;
pub use screen_panels::diegetic_stats_panel_with_integral_advance;
pub use screen_panels::diegetic_stats_sections_panel;
pub use screen_panels::diegetic_stats_sections_panel_with_integral_advance;
pub use screen_panels::diegetic_stats_sections_tree;
pub use screen_panels::diegetic_stats_sections_tree_with_integral_advance;
pub use screen_panels::diegetic_stats_tree;
pub use screen_panels::diegetic_stats_tree_with_integral_advance;
pub use screen_panels::fps_stats_panel;
pub use screen_panels::gpu_meter_panel;
pub use screen_panels::screen_panel_frame;
pub use screen_panels::screen_panel_material;
pub use screen_panels::screen_panel_material_handle;
pub use screen_panels::screen_panel_text_edge_trim;
pub use screen_panels::separation::EdgePanelPlacementState;
pub use screen_panels::separation::EdgePanelPlacementStatus;
pub use widget_drag_claim::WidgetDragClaimPlugin;

/// Construct a fresh [`SprinkleBuilder`] whose Fairy Dust baseline is installed
/// by the first builder operation. Chain capability methods, then call `.run()`.
///
/// [`hana_diegetic::DiegeticUiPlugin`] is registered unconditionally so any
/// example can spawn `WorldText` or `DiegeticPanel` without an explicit
/// `add_plugins` call.
///
/// The Ctrl+Shift+R hot-restart shortcut is wired up unconditionally — when
/// pressed, the example process exits and spawns `cargo run --example <name>`
/// from the workspace root. Cargo handles the incremental rebuild, so any
/// source changes since the last build are picked up automatically.
#[must_use]
pub fn sprinkle_example() -> SprinkleBuilder<NoOrbitCam, AssetRootPending> {
    SprinkleBuilder::new(App::new())
}

/// The workspace's quiet-filter [`LogPlugin`], used by [`sprinkle_example`].
#[cfg(not(test))]
fn quiet_log_plugin() -> LogPlugin {
    LogPlugin {
        filter: LOG_FILTER.to_string(),
        ..LogPlugin::default()
    }
}

/// Wall-clock endpoints for the expensive baseline plugin builds.
#[derive(Resource)]
pub struct BaselineBuildTimings {
    /// Instant after Bevy's default plugins finish building.
    pub default_plugins: Instant,
    /// Instant after the diegetic UI plugin finishes building.
    pub diegetic_ui:     Instant,
    /// Instant after the camera input plugin finishes building.
    pub lagrange:        Instant,
    /// Instant after all Fairy Dust baseline installers finish.
    pub complete:        Instant,
}

/// Install `DefaultPlugins` and the Fairy Dust baseline with `asset_plugin`.
pub(crate) fn install_baseline(app: &mut App, asset_plugin: AssetPlugin) {
    #[cfg(not(test))]
    let plugins = DefaultPlugins.build().set(quiet_log_plugin());
    #[cfg(test)]
    let plugins = DefaultPlugins
        .build()
        .set(RenderPlugin {
            render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                backends: None,
                ..default()
            })),
            ..default()
        })
        .disable::<LogPlugin>()
        .disable::<WinitPlugin>();
    install_baseline_with_plugins(app, asset_plugin, plugins);
}

pub(crate) fn install_baseline_with_plugins(
    app: &mut App,
    asset_plugin: AssetPlugin,
    plugins: PluginGroupBuilder,
) {
    app.add_plugins(plugins.enable::<AssetPlugin>().set(asset_plugin));
    let default_plugins = Instant::now();
    ensure_plugin(app, DiegeticUiPlugin);
    let diegetic_ui = Instant::now();
    ensure_plugin(app, LagrangePlugin);
    let lagrange = Instant::now();
    ensure_plugin(app, WidgetDragClaimPlugin);
    app.insert_resource(
        CameraInputRoutingConfig::cursor_hit_test()
            .with_no_position_fallback(NoPositionFallback::OnlyEligibleCamera),
    );
    screen_panels::install_overlay_picking(app);
    restart::install(app);
    screen_space_lights::install(app);
    app.insert_resource(BaselineBuildTimings {
        default_plugins,
        diegetic_ui,
        lagrange,
        complete: Instant::now(),
    });
}

/// Add `plugin` to `app` if no plugin of the same type is already registered.
///
/// Bevy panics on duplicate plugin registration, so capabilities that share
/// infrastructure (for instance multiple HUD capabilities both needing
/// `DiegeticUiPlugin`) route their plugin adds through this helper.
pub(crate) fn ensure_plugin<P: Plugin>(app: &mut App, plugin: P) {
    if !app.is_plugin_added::<P>() {
        app.add_plugins(plugin);
    }
}

#[cfg(test)]
use hana_lagrange::input_test_app as test_app;

#[cfg(test)]
mod tests {
    use bevy::image::Image;
    use bevy::prelude::AssetServer;
    use bevy::prelude::Assets;
    use bevy::prelude::Commands;
    use bevy::prelude::Handle;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Startup;

    use super::*;

    #[derive(Resource)]
    struct SeededImage(Handle<Image>);

    #[derive(Resource)]
    struct ImageStoreExistedBeforeRender(bool);

    struct SeedImagePlugin;

    impl Plugin for SeedImagePlugin {
        fn build(&self, app: &mut App) {
            app.insert_resource(ImageStoreExistedBeforeRender(
                app.world().contains_resource::<Assets<Image>>(),
            ));
            app.add_systems(
                Startup,
                |mut commands: Commands, mut images: ResMut<Assets<Image>>| {
                    commands.insert_resource(SeededImage(images.add(Image::default())));
                },
            );
        }
    }

    #[test]
    fn baseline_preserves_image_seeded_by_plugin_before_render() {
        let mut app = crate::test_app();
        let plugins = DefaultPlugins
            .build()
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: None,
                    ..default()
                })),
                ..default()
            })
            .disable::<LogPlugin>()
            .disable::<WinitPlugin>()
            .add_before::<RenderPlugin>(SeedImagePlugin);
        install_baseline_with_plugins(&mut app, AssetPlugin::default(), plugins);
        app.update();
        assert!(!app.world().resource::<ImageStoreExistedBeforeRender>().0);
        let handle = &app.world().resource::<SeededImage>().0;
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(handle)
                .is_some()
        );
    }

    #[test]
    fn baseline_enables_asset_plugin_disabled_by_caller() {
        let mut app = crate::test_app();
        let plugins = DefaultPlugins
            .build()
            .disable::<AssetPlugin>()
            .disable::<LogPlugin>()
            .disable::<WinitPlugin>();
        install_baseline_with_plugins(&mut app, AssetPlugin::default(), plugins);
        assert!(app.is_plugin_added::<AssetPlugin>());
        assert!(app.world().contains_resource::<AssetServer>());
    }

    #[test]
    fn baseline_uses_only_eligible_camera_for_no_position_input() {
        let mut app = crate::test_app();
        install_baseline(&mut app, AssetPlugin::default());

        assert_eq!(
            app.world()
                .resource::<CameraInputRoutingConfig>()
                .no_position_fallback,
            NoPositionFallback::OnlyEligibleCamera,
        );
    }
}
