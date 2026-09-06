//! Success domain: every set emits `Loaded` once, the no-failure global events
//! stay ordered, and progress is readable until finalization removes it.

use bevy::asset::Assets;
use bevy::asset::Handle;
use bevy::image::Image;
use bevy::image::ImageLoaderSettings;
use bevy::image::ImageSampler;
use bevy::prelude::On;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use hana_lading::AllSetsLoaded;
use hana_lading::AllSetsResolved;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;
use hana_lading::LoadProgress;
use hana_lading::Loaded;

use crate::support::SETTLE_UPDATES;
use crate::support::image_app;
use crate::support::update_until;

// fixture paths relative to the test asset source root
const CHECKER_PATH: &str = "textures/checker.png";
const PLAIN_PATH: &str = "textures/plain.png";
const SOLO_PATH: &str = "textures/solo.png";

#[derive(Resource)]
struct SetOne {
    checker: Handle<Image>,
    plain:   Handle<Image>,
}

impl DiskAssets for SetOne {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            checker: loader.load_with_settings(
                CHECKER_PATH,
                |settings: &mut ImageLoaderSettings| {
                    settings.sampler = ImageSampler::nearest();
                },
            ),
            plain:   loader.load(PLAIN_PATH),
        }
    }
}

#[derive(Resource)]
pub(crate) struct SetTwo {
    pub(crate) solo: Handle<Image>,
}

impl DiskAssets for SetTwo {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            solo: loader.load(SOLO_PATH),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum SuccessEntry {
    SetOne,
    SetTwo,
    AllLoaded,
    AllResolved,
}

#[derive(Resource, Default)]
struct SuccessLog {
    entries:           Vec<SuccessEntry>,
    loaded_progress:   Option<(usize, usize)>,
    resolved_progress: Option<(usize, usize)>,
    resolved_failures: Option<usize>,
}

#[test]
fn success_two_sets() {
    let mut app = image_app();
    app.add_plugins((
        DiskAssetsPlugin::<SetOne>::default(),
        DiskAssetsPlugin::<SetTwo>::default(),
    ))
    .init_resource::<SuccessLog>()
    .add_observer(|_: On<Loaded<SetOne>>, mut log: ResMut<SuccessLog>| {
        log.entries.push(SuccessEntry::SetOne);
    })
    .add_observer(|_: On<Loaded<SetTwo>>, mut log: ResMut<SuccessLog>| {
        log.entries.push(SuccessEntry::SetTwo);
    })
    .add_observer(
        |_: On<AllSetsLoaded>, progress: Res<LoadProgress>, mut log: ResMut<SuccessLog>| {
            log.entries.push(SuccessEntry::AllLoaded);
            log.loaded_progress = Some((progress.loaded(), progress.total()));
        },
    )
    .add_observer(
        |event: On<AllSetsResolved>, progress: Res<LoadProgress>, mut log: ResMut<SuccessLog>| {
            log.entries.push(SuccessEntry::AllResolved);
            log.resolved_progress = Some((progress.loaded(), progress.total()));
            log.resolved_failures = Some(event.failures());
        },
    );

    update_until(&mut app, "success_two_sets terminal", |world| {
        world
            .resource::<SuccessLog>()
            .entries
            .contains(&SuccessEntry::AllResolved)
    });
    for _ in 0..SETTLE_UPDATES {
        app.update();
    }

    let world = app.world();
    let log = world.resource::<SuccessLog>();
    assert_eq!(log.entries.len(), 4);
    assert!(log.entries[..2].contains(&SuccessEntry::SetOne));
    assert!(log.entries[..2].contains(&SuccessEntry::SetTwo));
    assert_eq!(
        log.entries[2..],
        [SuccessEntry::AllLoaded, SuccessEntry::AllResolved]
    );
    assert_eq!(log.loaded_progress, Some((2, 2)));
    assert_eq!(log.resolved_progress, Some((2, 2)));
    assert_eq!(log.resolved_failures, Some(0));
    assert!(!world.contains_resource::<LoadProgress>());

    let images = world.resource::<Assets<Image>>();
    let set_one = world.resource::<SetOne>();
    let checker = images.get(&set_one.checker).expect("checker.png loads");
    assert_eq!(checker.sampler, ImageSampler::nearest());
    let plain = images.get(&set_one.plain).expect("plain.png loads");
    assert_eq!(plain.sampler, ImageSampler::Default);
    let set_two = world.resource::<SetTwo>();
    assert!(images.get(&set_two.solo).is_some());
}
