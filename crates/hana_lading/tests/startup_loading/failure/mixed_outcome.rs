//! Mixed outcome: one set fails while another loads, and the successful set
//! stays usable after resolution.

use bevy::asset::Assets;
use bevy::image::Image;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use hana_lading::AllSetsLoaded;
use hana_lading::AllSetsResolved;
use hana_lading::DiskAssetsPlugin;
use hana_lading::LoadFailed;
use hana_lading::Loaded;

use super::missing_file::MISSING_ABSENT_PATH;
use super::missing_file::MissingAssets;
use crate::success::SetTwo;
use crate::support::SETTLE_UPDATES;
use crate::support::assert_fixture_absent;
use crate::support::image_app;
use crate::support::update_until;

#[derive(Resource, Default)]
struct MixedLog {
    good_loaded:     usize,
    bad_failures:    Vec<String>,
    all_loaded:      usize,
    resolved_events: Vec<usize>,
}

#[test]
fn mixed_outcome() {
    assert_fixture_absent(MISSING_ABSENT_PATH);
    let mut app = image_app();
    app.add_plugins((
        DiskAssetsPlugin::<SetTwo>::default(),
        DiskAssetsPlugin::<MissingAssets>::default(),
    ))
    .init_resource::<MixedLog>()
    .add_observer(|_: On<Loaded<SetTwo>>, mut log: ResMut<MixedLog>| {
        log.good_loaded += 1;
    })
    .add_observer(
        |event: On<LoadFailed<MissingAssets>>, mut log: ResMut<MixedLog>| {
            log.bad_failures.push(event.tracked_path().to_string());
        },
    )
    .add_observer(|_: On<AllSetsLoaded>, mut log: ResMut<MixedLog>| {
        log.all_loaded += 1;
    })
    .add_observer(|event: On<AllSetsResolved>, mut log: ResMut<MixedLog>| {
        log.resolved_events.push(event.failures());
    });

    update_until(&mut app, "mixed_outcome terminal", |world| {
        !world.resource::<MixedLog>().resolved_events.is_empty()
    });
    for _ in 0..SETTLE_UPDATES {
        app.update();
    }

    let world = app.world();
    let log = world.resource::<MixedLog>();
    assert_eq!(log.good_loaded, 1);
    assert_eq!(log.bad_failures, [MISSING_ABSENT_PATH]);
    assert_eq!(log.all_loaded, 0);
    assert_eq!(log.resolved_events, [1]);

    let set_two = world.resource::<SetTwo>();
    assert!(
        world
            .resource::<Assets<Image>>()
            .get(&set_two.solo)
            .is_some(),
        "the successful set must remain usable after the other set fails"
    );
}
