//! Two sets tracking one shared missing asset record both failures in the
//! same polling frame, before resolution.

use bevy::asset::Handle;
use bevy::diagnostic::FrameCount;
use bevy::image::Image;
use bevy::prelude::On;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use hana_lading::AllSetsResolved;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;
use hana_lading::LoadFailed;

use crate::support::assert_fixture_absent;
use crate::support::image_app;
use crate::support::update_until;

// fixture paths relative to the test asset source root
const MISSING_SHARED_PATH: &str = "missing/shared.png";

#[derive(Resource)]
struct FailOne {
    shared: Handle<Image>,
}

impl DiskAssets for FailOne {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            shared: loader.load(MISSING_SHARED_PATH),
        }
    }
}

#[derive(Resource)]
struct FailTwo {
    shared: Handle<Image>,
}

impl DiskAssets for FailTwo {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            shared: loader.load(MISSING_SHARED_PATH),
        }
    }
}

#[derive(Resource, Default)]
struct PairLog {
    failed_frames:              Vec<u32>,
    failures_before_resolution: Option<usize>,
}

#[test]
fn two_failures_one_frame() {
    assert_fixture_absent(MISSING_SHARED_PATH);
    let mut app = image_app();
    app.add_plugins((
        DiskAssetsPlugin::<FailOne>::default(),
        DiskAssetsPlugin::<FailTwo>::default(),
    ))
    .init_resource::<PairLog>()
    .add_observer(
        |_: On<LoadFailed<FailOne>>, frames: Res<FrameCount>, mut log: ResMut<PairLog>| {
            log.failed_frames.push(frames.0);
        },
    )
    .add_observer(
        |_: On<LoadFailed<FailTwo>>, frames: Res<FrameCount>, mut log: ResMut<PairLog>| {
            log.failed_frames.push(frames.0);
        },
    )
    .add_observer(|_: On<AllSetsResolved>, mut log: ResMut<PairLog>| {
        log.failures_before_resolution = Some(log.failed_frames.len());
    });

    update_until(&mut app, "two_failures_one_frame terminal", |world| {
        world
            .resource::<PairLog>()
            .failures_before_resolution
            .is_some()
    });

    let world = app.world();
    assert_eq!(
        world.resource::<FailOne>().shared.id(),
        world.resource::<FailTwo>().shared.id(),
        "both sets track one shared missing asset, so both fail in the same polling pass"
    );
    let log = world.resource::<PairLog>();
    assert_eq!(log.failed_frames.len(), 2);
    assert_eq!(log.failed_frames[0], log.failed_frames[1]);
    assert_eq!(log.failures_before_resolution, Some(2));
}
