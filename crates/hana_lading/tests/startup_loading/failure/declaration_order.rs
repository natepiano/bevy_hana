//! With two missing roots resolved in one pass, the reported first failure
//! follows handle declaration order.

use bevy::asset::AssetServer;
use bevy::asset::Handle;
use bevy::asset::LoadState;
use bevy::image::Image;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use hana_lading::AllSetsResolved;
use hana_lading::AssetSetLoadFailed;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;
use hana_lading::LoadFailed;

use crate::support::SETTLE_UPDATES;
use crate::support::app_with_test_assets;
use crate::support::assert_fixture_absent;
use crate::support::drain_asset_events_until;
use crate::support::preregister_image_loader;
use crate::support::register_image_loader;
use crate::support::update_until;

// fixture paths relative to the test asset source root
const MISSING_FIRST_PATH: &str = "missing/first.png";
const MISSING_SECOND_PATH: &str = "missing/second.png";

#[derive(Resource)]
struct FailOrder {
    first:  Handle<Image>,
    second: Handle<Image>,
}

impl DiskAssets for FailOrder {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            first:  loader.load(MISSING_FIRST_PATH),
            second: loader.load(MISSING_SECOND_PATH),
        }
    }
}

#[derive(Resource, Default)]
struct OrderLog {
    generic_paths:     Vec<String>,
    typed_paths:       Vec<String>,
    resolved_failures: Option<usize>,
}

#[test]
fn first_failure_follows_declaration_order() {
    assert_fixture_absent(MISSING_FIRST_PATH);
    assert_fixture_absent(MISSING_SECOND_PATH);
    let mut app = app_with_test_assets();
    preregister_image_loader(&mut app);
    app.add_plugins(DiskAssetsPlugin::<FailOrder>::default())
        .init_resource::<OrderLog>()
        .add_observer(|event: On<AssetSetLoadFailed>, mut log: ResMut<OrderLog>| {
            log.generic_paths.push(event.tracked_path().to_string());
        })
        .add_observer(
            |event: On<LoadFailed<FailOrder>>, mut log: ResMut<OrderLog>| {
                log.typed_paths.push(event.tracked_path().to_string());
            },
        )
        .add_observer(|event: On<AllSetsResolved>, mut log: ResMut<OrderLog>| {
            log.resolved_failures = Some(event.failures());
        });

    for _ in 0..SETTLE_UPDATES {
        app.update();
    }
    {
        let log = app.world().resource::<OrderLog>();
        assert!(
            log.generic_paths.is_empty(),
            "no failure may land while the png loader is only preregistered"
        );
    }

    let (first_id, second_id) = {
        let fail_order = app.world().resource::<FailOrder>();
        (fail_order.first.id(), fail_order.second.id())
    };
    register_image_loader(&mut app);
    drain_asset_events_until(
        &mut app,
        "both missing roots record failed states",
        move |world| {
            let asset_server = world.resource::<AssetServer>();
            matches!(
                asset_server.get_load_state(first_id),
                Some(LoadState::Failed(_))
            ) && matches!(
                asset_server.get_load_state(second_id),
                Some(LoadState::Failed(_))
            )
        },
    );
    update_until(
        &mut app,
        "first_failure_follows_declaration_order terminal",
        |world| world.resource::<OrderLog>().resolved_failures.is_some(),
    );

    let log = app.world().resource::<OrderLog>();
    assert_eq!(log.generic_paths, [MISSING_FIRST_PATH]);
    assert_eq!(log.typed_paths, [MISSING_FIRST_PATH]);
    assert_eq!(log.resolved_failures, Some(1));
}
