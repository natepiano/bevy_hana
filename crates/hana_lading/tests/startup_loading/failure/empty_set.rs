//! An asset set that records zero handles panics at plugin startup.

use bevy::prelude::Resource;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;

use crate::support::app_with_test_assets;

#[derive(Resource)]
struct EmptyAssets;

impl DiskAssets for EmptyAssets {
    fn load(_: &mut DiskAssetLoader<'_>) -> Self { Self }
}

#[test]
#[should_panic(expected = "EmptyAssets recorded zero handles")]
fn empty_set_panics() {
    let mut app = app_with_test_assets();
    app.add_plugins(DiskAssetsPlugin::<EmptyAssets>::default());
    app.update();
}
