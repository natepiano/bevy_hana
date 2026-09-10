//! Shared app construction and update helpers.

use std::path::Path;
use std::path::PathBuf;

use bevy::app::App;
use bevy::app::Last;
use bevy::app::PreUpdate;
use bevy::asset::AssetApp;
use bevy::asset::AssetPlugin;
use bevy::ecs::world::World;
use bevy::image::CompressedImageFormats;
use bevy::image::ImageLoader;
use bevy::image::ImagePlugin;
use bevy::prelude::MinimalPlugins;

// pacing shared by every test app
pub(crate) const SETTLE_UPDATES: usize = 3;

fn assets_root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/assets") }

pub(crate) fn test_asset_plugin() -> AssetPlugin {
    AssetPlugin {
        file_path: assets_root().to_string_lossy().into_owned(),
        ..AssetPlugin::default()
    }
}

pub(crate) fn app_with_test_assets() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, test_asset_plugin()));
    app
}

/// Initializes `Assets<Image>` and preregisters `ImageLoader` for its
/// extensions without registering a working loader instance. Loads that reach
/// the preregistered entry wait until [`register_image_loader`] runs.
pub(crate) fn preregister_image_loader(app: &mut App) { app.add_plugins(ImagePlugin::default()); }

pub(crate) fn register_image_loader(app: &mut App) {
    app.register_asset_loader(ImageLoader::new(CompressedImageFormats::empty()));
}

pub(crate) fn image_app() -> App {
    let mut app = app_with_test_assets();
    preregister_image_loader(&mut app);
    register_image_loader(&mut app);
    app
}

pub(crate) fn assert_fixture_absent(relative_path: &str) {
    let absolute = assets_root().join(relative_path);
    assert!(
        !absolute.exists(),
        "fixture `{}` must stay absent for missing-file coverage",
        absolute.display()
    );
}

/// Runs full app updates until `observed` returns true.
///
/// There is no deadline. The asset work being waited on runs on the IO pool, so a deadline
/// decides from how busy the machine is that a load still on its way had never arrived. A
/// predicate that can never hold parks here for nextest's `slow-timeout` to end, which reports
/// it as stuck.
pub(crate) fn update_until(app: &mut App, mut observed: impl FnMut(&World) -> bool) {
    while !observed(app.world()) {
        app.update();
    }
}

/// Runs only the `Last` task-pool tick and the `PreUpdate` asset-event drain
/// until `observed` returns true. Bevy records asynchronous load results while
/// `hana_lading` never polls, so a following [`App::update`] observes every
/// drained state in one pass.
///
/// Unbounded, for the reason given on [`update_until`].
pub(crate) fn drain_asset_events_until(app: &mut App, mut observed: impl FnMut(&World) -> bool) {
    while !observed(app.world()) {
        app.world_mut().run_schedule(Last);
        app.world_mut().run_schedule(PreUpdate);
    }
}
