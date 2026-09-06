//! Application policy keeps startup in `Loading` after a required asset fails.
//!
//! `hana_lading` reports the terminal failure instead of leaving startup
//! waiting forever. The application records the generic failure before global
//! completion, renders the evidence, and never enters `Ready`.
//!
//! Hana Lading is installed on the underlying Bevy `App`; Fairy Dust supplies
//! the surrounding failure presentation.

mod loading_evidence;

use std::path::Path;

use bevy::prelude::Alpha;
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::Handle;
use bevy::prelude::Image;
use bevy::prelude::On;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::debug;
use bevy::prelude::error;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use hana_lading::AllSetsResolved;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;
use loading_evidence::FailureRecord;
use loading_evidence::LoadingEvidencePlugin;
use loading_evidence::spawn_failure_panel;

const DECISION: &str = "Remain in Loading because required startup content failed.";
const DISPLAY_NAME: &str = "Catastrophic Failure";
const MISSING_ASSET: &str = "intentionally-absent.png";
const OPAQUE_ALPHA: f32 = 1.0;

#[derive(Resource)]
struct RequiredAssets {
    image: Handle<Image>,
}

impl DiskAssets for RequiredAssets {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            image: loader.load(MISSING_ASSET),
        }
    }
}

fn main() {
    let asset_root = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");
    assert!(
        !Path::new(asset_root).join(MISSING_ASSET).exists(),
        "the catastrophic example requires its missing fixture to stay absent"
    );

    let mut example = fairy_dust::sprinkle_example().with_asset_root(asset_root);

    example
        .app_mut()
        .add_plugins(DiskAssetsPlugin::<RequiredAssets>::default())
        .add_plugins(LoadingEvidencePlugin)
        .add_observer(on_all_sets_resolved);

    example.with_brp_extras().with_save_window_position().run();
}

fn on_all_sets_resolved(
    event: On<AllSetsResolved>,
    required: Res<RequiredAssets>,
    record: Res<FailureRecord>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if event.failures() == 0 {
        error!("catastrophic example unexpectedly resolved without a failure");
        return;
    }
    debug!(asset = ?required.image.id(), "required asset reached a terminal failure");
    spawn_failure_panel(
        &mut commands,
        &mut materials,
        &record,
        DISPLAY_NAME,
        DECISION,
        DEFAULT_PANEL_BACKGROUND.with_alpha(OPAQUE_ALPHA),
    );
}
