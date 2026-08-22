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
mod primitive;
mod release_hold;
mod restart;
mod restart_camera;
mod save_window_position;
mod screen_panels;
mod screen_space_lights;
mod shortcuts;
mod transparency;
mod unclamp;

use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::App;
use bevy::prelude::DefaultPlugins;
use bevy::prelude::Plugin;
use bevy::prelude::PluginGroup;
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
use hana_lagrange::LagrangePlugin;
pub use hana_lagrange::OrbitCam;
pub use keymap::CommandPaletteKeymap;
/// Returns the fixed Command+P (macOS) or Control+P recovery chord that an
/// externally installed contextual Rubric keymap must associate with the palette command.
///
/// Pass this and [`command_palette_recovery_command_id`] to
/// [`hana_rubric::KeymapPlugin::with_protected_command_binding`] before installing Fairy Dust
/// so authored bindings cannot compete with the command palette's direct recovery route.
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
pub use screen_panels::StatsPanelRow;
pub use screen_panels::StatsPanelSection;
pub use screen_panels::TitleBar;
pub use screen_panels::TitleBarControl;
pub use screen_panels::TitleBarOrientation;
pub use screen_panels::TitleBarSegment;
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

/// Install `DefaultPlugins` and the Fairy Dust baseline with `asset_plugin`.
pub(crate) fn install_baseline(app: &mut App, asset_plugin: AssetPlugin) {
    #[cfg(not(test))]
    app.add_plugins(DefaultPlugins.set(asset_plugin).set(quiet_log_plugin()));
    #[cfg(test)]
    app.add_plugins(
        DefaultPlugins
            .build()
            .set(asset_plugin)
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: None,
                    ..default()
                })),
                ..default()
            })
            .disable::<LogPlugin>()
            .disable::<WinitPlugin>(),
    );
    ensure_plugin(app, DiegeticUiPlugin);
    ensure_plugin(app, LagrangePlugin);
    screen_panels::install_overlay_picking(app);
    restart::install(app);
    screen_space_lights::install(app);
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
