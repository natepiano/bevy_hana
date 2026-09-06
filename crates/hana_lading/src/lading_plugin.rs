use std::any::type_name;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use bevy_app::App;
use bevy_app::Plugin;
use bevy_app::PostStartup;
use bevy_app::PreStartup;
use bevy_app::Update;
use bevy_asset::AssetLoadError;
use bevy_asset::AssetPath;
use bevy_asset::AssetServer;
use bevy_asset::DependencyLoadState;
use bevy_asset::LoadState;
use bevy_asset::RecursiveDependencyLoadState;
use bevy_asset::UntypedHandle;
use bevy_ecs::prelude::Commands;
use bevy_ecs::prelude::Res;
use bevy_ecs::prelude::ResMut;
use bevy_ecs::prelude::Resource;
use bevy_ecs::prelude::SystemSet;
use bevy_ecs::schedule::ApplyDeferred;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::schedule::common_conditions::resource_exists;

use crate::AllSetsLoaded;
use crate::AllSetsResolved;
use crate::AssetSetLoadFailed;
use crate::DiskAssetLoader;
use crate::DiskAssets;
use crate::LoadFailed;
use crate::LoadProgress;
use crate::Loaded;

/// The global plugin entry point for startup asset loading.
///
/// Applications may install this plugin without registering any asset sets. In
/// that case it emits [`AllSetsLoaded`] followed by [`AllSetsResolved`] during
/// `PostStartup`.
pub struct LadingPlugin;

impl Plugin for LadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadProgress>()
            .configure_sets(
                Update,
                AssetTracking.run_if(resource_exists::<LoadProgress>),
            )
            .configure_sets(
                Update,
                (AssetPoll, AssetFinalize).chain().in_set(AssetTracking),
            )
            .add_systems(
                Update,
                ApplyDeferred
                    .in_set(AssetTracking)
                    .after(AssetPoll)
                    .before(AssetFinalize),
            )
            .add_systems(PostStartup, finish_empty)
            .add_systems(Update, finalize_batch.in_set(AssetFinalize));
    }
}

/// Registers one [`DiskAssets`] implementation as a startup asset set.
///
/// Adds [`LadingPlugin`] if it is not already installed, loads the asset set
/// during `PreStartup`, and polls its recursive dependency state during
/// `Update`.
pub struct DiskAssetsPlugin<T: DiskAssets>(PhantomData<fn() -> T>);

impl<T: DiskAssets> Default for DiskAssetsPlugin<T> {
    fn default() -> Self { Self(PhantomData) }
}

impl<T: DiskAssets> Plugin for DiskAssetsPlugin<T> {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<LadingPlugin>() {
            app.add_plugins(LadingPlugin);
        }

        app.world_mut()
            .resource_mut::<LoadProgress>()
            .register_set();
        app.add_systems(PreStartup, load_set::<T>)
            .add_systems(Update, check_set::<T>.in_set(AssetPoll));
    }

    fn finish(&self, app: &mut App) {
        assert!(
            app.world().contains_resource::<AssetServer>(),
            "DiskAssetsPlugin<{}> requires Bevy's AssetServer; add AssetPlugin before running the app",
            type_name::<T>()
        );
    }
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
struct AssetTracking;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
struct AssetPoll;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
struct AssetFinalize;

#[derive(Resource)]
struct Tracked<T: DiskAssets> {
    handles: Vec<UntypedHandle>,
    started: Instant,
    notice:  SlowLoadNotice,
    marker:  PhantomData<fn() -> T>,
}

impl<T: DiskAssets> Tracked<T> {
    const SLOW_LOAD_NOTICE_AFTER: Duration = Duration::from_secs(10);

    fn new(handles: Vec<UntypedHandle>) -> Self {
        Self {
            handles,
            started: Instant::now(),
            notice: SlowLoadNotice::Pending,
            marker: PhantomData,
        }
    }
}

enum SlowLoadNotice {
    Pending,
    Logged,
}

enum HandleResolution {
    Loaded,
    Pending,
    Failed(Arc<AssetLoadError>),
}

enum SetResolution {
    Loaded,
    Pending,
    Failed {
        index: usize,
        error: Arc<AssetLoadError>,
    },
}

impl SetResolution {
    fn include(self, index: usize, handle: HandleResolution) -> Self {
        match (self, handle) {
            (failed @ Self::Failed { .. }, _) => failed,
            (_, HandleResolution::Failed(error)) => Self::Failed { index, error },
            (Self::Loaded, HandleResolution::Pending) => Self::Pending,
            (current, _) => current,
        }
    }
}

fn load_set<T: DiskAssets>(mut commands: Commands, asset_server: Option<Res<AssetServer>>) {
    #[allow(
        clippy::panic,
        reason = "a missing AssetServer is an invalid DiskAssetsPlugin configuration; loading cannot proceed without it, so fail immediately with the required fix"
    )]
    let asset_server = asset_server.unwrap_or_else(|| {
        panic!(
            "DiskAssetsPlugin<{}> cannot load because AssetServer is absent; update-only test harnesses must insert AssetServer before running PreStartup",
            type_name::<T>()
        )
    });
    let mut loader = DiskAssetLoader::new(&asset_server);
    let asset_set = T::load(&mut loader);
    let handles = loader.into_handles();

    assert!(
        !handles.is_empty(),
        "DiskAssets::load for {} recorded zero handles; every registered asset set must start at least one disk load",
        type_name::<T>()
    );

    commands.insert_resource(asset_set);
    commands.insert_resource(Tracked::<T>::new(handles));
}

fn check_set<T: DiskAssets>(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    tracked: Option<ResMut<Tracked<T>>>,
    mut progress: ResMut<LoadProgress>,
) {
    let Some(mut tracked) = tracked else {
        return;
    };

    let set_resolution = tracked.handles.iter().enumerate().fold(
        SetResolution::Loaded,
        |set_resolution, (index, handle)| {
            set_resolution.include(
                index,
                handle_resolution(asset_server.get_load_states(handle.id())),
            )
        },
    );

    match set_resolution {
        SetResolution::Failed { index, error } => {
            let tracked_path = owned_tracked_path(&tracked.handles[index]);
            progress.resolve_failure();
            tracing::error!(
                asset_set = type_name::<T>(),
                path = %tracked_path,
                error = %error,
                "startup disk asset set failed"
            );
            commands.trigger(AssetSetLoadFailed::new::<T>(
                tracked_path.clone(),
                error.clone(),
            ));
            commands.trigger(LoadFailed::<T>::new(tracked_path, error));
            commands.remove_resource::<Tracked<T>>();
        },
        SetResolution::Loaded => {
            progress.resolve_loaded();
            commands.trigger(Loaded::<T>::new());
            commands.remove_resource::<Tracked<T>>();
        },
        SetResolution::Pending
            if matches!(tracked.notice, SlowLoadNotice::Pending)
                && tracked.started.elapsed() >= Tracked::<T>::SLOW_LOAD_NOTICE_AFTER =>
        {
            let root_paths: Vec<_> = tracked
                .handles
                .iter()
                .filter_map(UntypedHandle::path)
                .collect();
            tracing::warn!(
                asset_set = type_name::<T>(),
                roots = ?root_paths,
                "startup disk asset set is still loading"
            );
            tracked.notice = SlowLoadNotice::Logged;
        },
        SetResolution::Pending => {},
    }
}

fn handle_resolution(
    states: Option<(LoadState, DependencyLoadState, RecursiveDependencyLoadState)>,
) -> HandleResolution {
    let Some((root, direct, recursive)) = states else {
        return HandleResolution::Pending;
    };

    if let LoadState::Failed(error) = root {
        return HandleResolution::Failed(error);
    }
    if let DependencyLoadState::Failed(error) = direct {
        return HandleResolution::Failed(error);
    }
    if let RecursiveDependencyLoadState::Failed(error) = recursive {
        return HandleResolution::Failed(error);
    }

    if matches!(root, LoadState::Loaded)
        && matches!(direct, DependencyLoadState::Loaded)
        && matches!(recursive, RecursiveDependencyLoadState::Loaded)
    {
        HandleResolution::Loaded
    } else {
        HandleResolution::Pending
    }
}

#[allow(
    clippy::panic,
    reason = "Tracked only contains path-backed handles validated by DiskAssetLoader"
)]
fn owned_tracked_path(handle: &UntypedHandle) -> AssetPath<'static> {
    handle.path().cloned().unwrap_or_else(|| {
        panic!("Tracked contains a pathless handle that bypassed DiskAssetLoader validation")
    })
}

fn finalize_batch(mut commands: Commands, progress: Res<LoadProgress>) {
    if !progress.is_complete() {
        return;
    }

    trigger_global_completion(&mut commands, progress.failures());
    commands.remove_resource::<LoadProgress>();
}

fn finish_empty(mut commands: Commands, progress: Res<LoadProgress>) {
    if progress.total() != 0 {
        return;
    }

    trigger_global_completion(&mut commands, 0);
    commands.remove_resource::<LoadProgress>();
}

fn trigger_global_completion(commands: &mut Commands, failures: usize) {
    if failures == 0 {
        commands.trigger(AllSetsLoaded::new());
    }
    commands.trigger(AllSetsResolved::new(failures));
}

#[cfg(test)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;

    use super::LadingPlugin;
    use crate::AllSetsLoaded;
    use crate::AllSetsResolved;
    use crate::LoadProgress;

    #[derive(Debug, PartialEq, Eq)]
    enum Completion {
        Loaded,
        Resolved { failures: usize },
    }

    #[derive(Resource, Default)]
    struct CompletionOrder(Vec<Completion>);

    #[test]
    fn zero_sets_complete_in_order_and_remove_progress() {
        let mut app = App::new();
        app.init_resource::<CompletionOrder>()
            .add_observer(|_: On<AllSetsLoaded>, mut order: ResMut<CompletionOrder>| {
                order.0.push(Completion::Loaded);
            })
            .add_observer(
                |event: On<AllSetsResolved>, mut order: ResMut<CompletionOrder>| {
                    order.0.push(Completion::Resolved {
                        failures: event.failures(),
                    });
                },
            )
            .add_plugins(LadingPlugin);

        app.update();

        assert_eq!(
            app.world().resource::<CompletionOrder>().0,
            [Completion::Loaded, Completion::Resolved { failures: 0 }]
        );
        assert!(!app.world().contains_resource::<LoadProgress>());
    }
}
