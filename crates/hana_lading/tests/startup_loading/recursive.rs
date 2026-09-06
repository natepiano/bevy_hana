//! Recursive-dependency domain: a gated child load keeps its root's set
//! pending, and the failed child produces the set's terminal failure.

use std::future::poll_fn;
use std::mem;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;
use std::task::Waker;

use bevy::asset::Asset;
use bevy::asset::AssetApp;
use bevy::asset::AssetLoader;
use bevy::asset::AssetServer;
use bevy::asset::Handle;
use bevy::asset::LoadContext;
use bevy::asset::LoadState;
use bevy::asset::io::Reader;
use bevy::prelude::On;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::reflect::TypePath;
use hana_lading::AllSetsLoaded;
use hana_lading::AllSetsResolved;
use hana_lading::DiskAssetLoader;
use hana_lading::DiskAssets;
use hana_lading::DiskAssetsPlugin;
use hana_lading::LoadFailed;
use hana_lading::LoadProgress;
use hana_lading::Loaded;

use crate::support::SETTLE_UPDATES;
use crate::support::app_with_test_assets;
use crate::support::update_until;

// fixture loader extensions
const DEPS_EXTENSION: &str = "deps";
const GATED_EXTENSION: &str = "gated";

// fixture paths relative to the test asset source root
const ROOT_DEPS_PATH: &str = "recursive/root.deps";

#[derive(Asset, TypePath)]
struct GatedAsset;

#[derive(Asset, TypePath)]
struct DependingAsset {
    #[dependency]
    child: Handle<GatedAsset>,
}

/// Lifecycle of the gated child load, held behind the [`ChildGate`] mutex.
#[derive(Default)]
enum GateState {
    #[default]
    NotEntered,
    Waiting(Waker),
    Released,
}

/// Test-controlled gate that parks the child load until released.
///
/// [`Self::await_release`] stores the loading task's [`Waker`] in
/// [`GateState::Waiting`] and returns [`Poll::Pending`] without waking
/// itself; [`Self::release`] replaces the state with [`GateState::Released`]
/// and wakes a stored task outside the lock. Both lock the same [`Mutex`], so
/// a release either finds the registered waker or the next poll observes
/// [`GateState::Released`] — the release cannot be lost.
#[derive(Default)]
struct ChildGate {
    state: Mutex<GateState>,
}

impl ChildGate {
    fn entered(&self) -> bool {
        !matches!(
            *self.state.lock().expect("child gate lock"),
            GateState::NotEntered
        )
    }

    fn release(&self) {
        let waker = {
            let mut state = self.state.lock().expect("child gate lock");
            match mem::replace(&mut *state, GateState::Released) {
                GateState::Waiting(waker) => Some(waker),
                GateState::NotEntered | GateState::Released => None,
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    async fn await_release(&self) {
        poll_fn(|context| {
            let mut state = self.state.lock().expect("child gate lock");
            match &*state {
                GateState::Released => Poll::Ready(()),
                GateState::NotEntered | GateState::Waiting(_) => {
                    *state = GateState::Waiting(context.waker().clone());
                    Poll::Pending
                },
            }
        })
        .await;
    }
}

#[derive(TypePath)]
struct GatedChildLoader {
    gate: Arc<ChildGate>,
}

impl AssetLoader for GatedChildLoader {
    type Asset = GatedAsset;
    type Error = std::io::Error;
    type Settings = ();

    async fn load(
        &self,
        _: &mut dyn Reader,
        (): &Self::Settings,
        _: &mut LoadContext<'_>,
    ) -> Result<GatedAsset, std::io::Error> {
        self.gate.await_release().await;
        Err(std::io::Error::other("gated child released into failure"))
    }

    fn extensions(&self) -> &[&str] { &[GATED_EXTENSION] }
}

#[derive(TypePath)]
struct DependingLoader;

impl AssetLoader for DependingLoader {
    type Asset = DependingAsset;
    type Error = std::io::Error;
    type Settings = ();

    async fn load(
        &self,
        reader: &mut dyn Reader,
        (): &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<DependingAsset, std::io::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let child_path = String::from_utf8_lossy(&bytes).trim().to_owned();
        Ok(DependingAsset {
            child: load_context.load(child_path),
        })
    }

    fn extensions(&self) -> &[&str] { &[DEPS_EXTENSION] }
}

#[derive(Resource)]
struct RecursiveAssets {
    root: Handle<DependingAsset>,
}

impl DiskAssets for RecursiveAssets {
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self {
        Self {
            root: loader.load(ROOT_DEPS_PATH),
        }
    }
}

#[derive(Resource, Default)]
struct RecursiveLog {
    loaded:            usize,
    failed_paths:      Vec<String>,
    all_loaded:        usize,
    resolved_failures: Option<usize>,
}

#[test]
fn recursive_dependencies_gate_and_fail() {
    let gate = Arc::new(ChildGate::default());
    let mut app = app_with_test_assets();
    app.init_asset::<DependingAsset>()
        .init_asset::<GatedAsset>()
        .register_asset_loader(DependingLoader)
        .register_asset_loader(GatedChildLoader { gate: gate.clone() })
        .add_plugins(DiskAssetsPlugin::<RecursiveAssets>::default())
        .init_resource::<RecursiveLog>()
        .add_observer(
            |_: On<Loaded<RecursiveAssets>>, mut log: ResMut<RecursiveLog>| {
                log.loaded += 1;
            },
        )
        .add_observer(
            |event: On<LoadFailed<RecursiveAssets>>, mut log: ResMut<RecursiveLog>| {
                log.failed_paths.push(event.tracked_path().to_string());
            },
        )
        .add_observer(|_: On<AllSetsLoaded>, mut log: ResMut<RecursiveLog>| {
            log.all_loaded += 1;
        })
        .add_observer(
            |event: On<AllSetsResolved>, mut log: ResMut<RecursiveLog>| {
                log.resolved_failures = Some(event.failures());
            },
        );

    let entered_gate = gate.clone();
    update_until(&mut app, "recursive child load begins", move |_| {
        entered_gate.entered()
    });
    let root_id = app.world().resource::<RecursiveAssets>().root.id();
    update_until(
        &mut app,
        "recursive root completes its own load",
        move |world| {
            matches!(
                world.resource::<AssetServer>().get_load_state(root_id),
                Some(LoadState::Loaded)
            )
        },
    );
    for _ in 0..SETTLE_UPDATES {
        app.update();
    }
    {
        let world = app.world();
        let log = world.resource::<RecursiveLog>();
        assert_eq!(
            log.loaded, 0,
            "the set must stay pending while its child is gated"
        );
        assert!(log.failed_paths.is_empty());
        assert!(world.contains_resource::<LoadProgress>());
    }

    gate.release();
    update_until(
        &mut app,
        "recursive_dependencies_gate_and_fail terminal",
        |world| world.resource::<RecursiveLog>().resolved_failures.is_some(),
    );

    let log = app.world().resource::<RecursiveLog>();
    assert_eq!(log.loaded, 0);
    assert_eq!(log.failed_paths, [ROOT_DEPS_PATH]);
    assert_eq!(log.all_loaded, 0);
    assert_eq!(log.resolved_failures, Some(1));
}
