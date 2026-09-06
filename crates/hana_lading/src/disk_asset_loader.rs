use bevy_asset::Asset;
use bevy_asset::AssetPath;
use bevy_asset::AssetServer;
use bevy_asset::Handle;
use bevy_asset::UntypedHandle;
use bevy_asset::meta::Settings;
use bevy_ecs::resource::Resource;

/// Starts disk-asset loads while retaining handles for completion tracking.
///
/// A `DiskAssetLoader` is provided to [`DiskAssets::load`]. Applications cannot
/// construct one or access its tracking storage directly, which keeps every
/// startup load visible to the completion events.
pub struct DiskAssetLoader<'a> {
    asset_server: &'a AssetServer,
    handles:      Vec<UntypedHandle>,
}

impl DiskAssetLoader<'_> {
    pub(crate) const fn new(asset_server: &AssetServer) -> DiskAssetLoader<'_> {
        DiskAssetLoader {
            asset_server,
            handles: Vec::new(),
        }
    }

    pub(crate) fn into_handles(self) -> Vec<UntypedHandle> { self.handles }

    /// Starts a tracked load for `A` at `path`.
    ///
    /// The returned typed handle belongs in the asset-set resource. The loader
    /// retains a second strong reference until tracking for the set ends.
    ///
    /// # Panics
    ///
    /// Panics if Bevy returns a pathless handle for `path`, because path-based
    /// completion tracking cannot resolve it.
    #[must_use = "store the returned handle in the DiskAssets resource"]
    pub fn load<'p, A: Asset>(&mut self, path: impl Into<AssetPath<'p>>) -> Handle<A> {
        let asset_path = path.into();
        let handle = self.asset_server.load(asset_path.clone());
        self.record(handle, &asset_path)
    }

    /// Starts a tracked load for `A` at `path` with asset-loader settings.
    ///
    /// `settings` configures the settings type used by the matching Bevy asset
    /// loader. The returned typed handle belongs in the asset-set resource.
    ///
    /// # Panics
    ///
    /// Panics if Bevy returns a pathless handle for `path`, because path-based
    /// completion tracking cannot resolve it.
    #[must_use = "store the returned handle in the DiskAssets resource"]
    pub fn load_with_settings<'p, A: Asset, S: Settings>(
        &mut self,
        path: impl Into<AssetPath<'p>>,
        settings: impl Fn(&mut S) + Send + Sync + 'static,
    ) -> Handle<A> {
        let asset_path = path.into();
        let handle = self
            .asset_server
            .load_builder()
            .with_settings(settings)
            .load(asset_path.clone());
        self.record(handle, &asset_path)
    }

    #[allow(
        clippy::panic,
        reason = "a pathless handle must fail immediately or its load tracking remains unresolved"
    )]
    fn record<A: Asset>(&mut self, handle: Handle<A>, attempted_path: &AssetPath<'_>) -> Handle<A> {
        handle.path().unwrap_or_else(|| {
            panic!(
                "DiskAssetLoader cannot track `{attempted_path}` because Bevy returned a pathless handle"
            )
        });
        self.handles.push(handle.clone().untyped());
        handle
    }
}

/// A startup asset set supplied by an application or another library.
///
/// This trait is a downstream extension point: each implementation starts all
/// of its disk loads through the provided [`DiskAssetLoader`] and stores the
/// returned typed handles in the resulting resource.
pub trait DiskAssets: Resource + Sized {
    /// Starts every load through the tracked loader and returns the resource
    /// that owns the resulting handles.
    fn load(loader: &mut DiskAssetLoader<'_>) -> Self;
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use bevy::asset::AssetApp;
    use bevy::asset::AssetPath;
    use bevy::asset::AssetPlugin;
    use bevy::asset::AssetServer;
    use bevy::prelude::MinimalPlugins;
    use bevy::reflect::TypePath;

    use super::DiskAssetLoader;

    #[derive(bevy::asset::Asset, TypePath)]
    struct PathlessAsset;

    #[test]
    #[should_panic(expected = "because Bevy returned a pathless handle")]
    fn pathless_rejection_panics() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<PathlessAsset>();
        let asset_server = app.world().resource::<AssetServer>();
        let pathless_handle = asset_server.add(PathlessAsset);
        let mut disk_asset_loader = DiskAssetLoader::new(asset_server);
        disk_asset_loader.record(pathless_handle, &AssetPath::from("tests/pathless.png"));
    }
}
