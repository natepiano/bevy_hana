//! Missing-file evidence: the generic and typed failure events agree on path
//! and error, and the generic event supports an exit-on-failure pattern.

use std::sync::Arc;
use std::time::Duration;

use bevy::app::App;
use bevy::app::AppExit;
use bevy::app::PluginGroup;
use bevy::app::ScheduleRunnerPlugin;
use bevy::app::Startup;
use bevy::app::Update;
use bevy::asset::AssetLoadError;
use bevy::asset::AssetServer;
use bevy::asset::Handle;
use bevy::asset::LoadState;
use bevy::asset::io::AssetReaderError;
use bevy::image::Image;
use bevy::prelude::MessageWriter;
use bevy::prelude::MinimalPlugins;
use bevy::prelude::On;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::time::Real;
use bevy::time::Time;
use hana_lading::AllSetsLoaded;
use hana_lading::AllSetsResolved;
use hana_lading::AssetSetLoadFailed;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;
use hana_lading::LoadFailed;

use crate::support::SETTLE_UPDATES;
use crate::support::assert_fixture_absent;
use crate::support::image_app;
use crate::support::preregister_image_loader;
use crate::support::register_image_loader;
use crate::support::test_asset_plugin;
use crate::support::update_until;

// deadlines and pacing
const EXIT_FALLBACK_DEADLINE: Duration = Duration::from_secs(30);
const EXIT_LOOP_WAIT: Duration = Duration::from_millis(1);

// fixture paths relative to the test asset source root
pub(super) const MISSING_ABSENT_PATH: &str = "missing/absent.png";

#[derive(Resource)]
pub(super) struct MissingAssets {
    root: Handle<Image>,
}

impl DiskAssets for MissingAssets {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            root: loader.load(MISSING_ABSENT_PATH),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum FailureSource {
    Generic,
    Typed,
}

#[derive(Resource, Default)]
struct MissingLog {
    order:             Vec<FailureSource>,
    generic:           Option<(String, Arc<AssetLoadError>)>,
    typed:             Option<(String, Arc<AssetLoadError>)>,
    all_loaded:        usize,
    resolved_failures: Option<usize>,
}

fn assert_root_still_loading(set: Res<MissingAssets>, asset_server: Res<AssetServer>) {
    assert!(
        matches!(
            asset_server.get_load_state(set.root.id()),
            Some(LoadState::Loading)
        ),
        "the missing-file load must begin as a Loading handle"
    );
}

#[test]
fn failure_missing_file() {
    assert_fixture_absent(MISSING_ABSENT_PATH);
    let mut app = image_app();
    app.add_plugins(DiskAssetsPlugin::<MissingAssets>::default())
        .init_resource::<MissingLog>()
        .add_systems(Startup, assert_root_still_loading)
        .add_observer(
            |event: On<AssetSetLoadFailed>, mut log: ResMut<MissingLog>| {
                log.order.push(FailureSource::Generic);
                log.generic = Some((event.tracked_path().to_string(), event.error().clone()));
            },
        )
        .add_observer(
            |event: On<LoadFailed<MissingAssets>>, mut log: ResMut<MissingLog>| {
                log.order.push(FailureSource::Typed);
                log.typed = Some((event.tracked_path().to_string(), event.error().clone()));
            },
        )
        .add_observer(|_: On<AllSetsLoaded>, mut log: ResMut<MissingLog>| {
            log.all_loaded += 1;
        })
        .add_observer(|event: On<AllSetsResolved>, mut log: ResMut<MissingLog>| {
            log.resolved_failures = Some(event.failures());
        });

    update_until(&mut app, "failure_missing_file terminal", |world| {
        world.resource::<MissingLog>().resolved_failures.is_some()
    });
    for _ in 0..SETTLE_UPDATES {
        app.update();
    }

    let log = app.world().resource::<MissingLog>();
    assert_eq!(log.order, [FailureSource::Generic, FailureSource::Typed]);
    let (generic_path, generic_error) = log.generic.as_ref().expect("generic failure evidence");
    let (typed_path, typed_error) = log.typed.as_ref().expect("typed failure evidence");
    assert_eq!(generic_path, typed_path);
    assert_eq!(generic_path, MISSING_ABSENT_PATH);
    assert_eq!(generic_error.to_string(), typed_error.to_string());
    assert!(matches!(
        generic_error.as_ref(),
        AssetLoadError::AssetReaderError(AssetReaderError::NotFound(_))
    ));
    assert_eq!(log.all_loaded, 0);
    assert_eq!(log.resolved_failures, Some(1));
}

fn exit_after_fallback_deadline(time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>) {
    if time.elapsed() > EXIT_FALLBACK_DEADLINE {
        exit.write(AppExit::Success);
    }
}

#[test]
fn exit_on_failure_pattern() {
    assert_fixture_absent(MISSING_ABSENT_PATH);
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(EXIT_LOOP_WAIT)),
        test_asset_plugin(),
    ));
    preregister_image_loader(&mut app);
    register_image_loader(&mut app);
    app.add_plugins(DiskAssetsPlugin::<MissingAssets>::default())
        .add_observer(
            |_: On<AssetSetLoadFailed>, mut exit: MessageWriter<AppExit>| {
                exit.write(AppExit::error());
            },
        )
        .add_systems(Update, exit_after_fallback_deadline);

    assert_eq!(app.run(), AppExit::error());
}
