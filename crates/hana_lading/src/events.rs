use std::any::TypeId;
use std::marker::PhantomData;
use std::sync::Arc;

use bevy_asset::AssetLoadError;
use bevy_asset::AssetPath;
use bevy_ecs::event::Event;
use bevy_ecs::resource::Resource;

use crate::DiskAssets;

/// Reports that one typed startup asset set loaded successfully.
#[derive(Event)]
pub struct Loaded<T: DiskAssets> {
    marker: PhantomData<fn() -> T>,
}

/// Reports that one typed startup asset set failed to load.
#[derive(Event)]
pub struct LoadFailed<T: DiskAssets> {
    tracked_path: AssetPath<'static>,
    error:        Arc<AssetLoadError>,
    marker:       PhantomData<fn() -> T>,
}

impl<T: DiskAssets> LoadFailed<T> {
    pub(crate) const fn new(tracked_path: AssetPath<'static>, error: Arc<AssetLoadError>) -> Self {
        Self {
            tracked_path,
            error,
            marker: PhantomData,
        }
    }

    /// Returns the tracked asset path that failed to resolve.
    #[must_use]
    pub const fn tracked_path(&self) -> &AssetPath<'static> { &self.tracked_path }

    /// Returns the shared Bevy asset-load error.
    #[must_use]
    pub const fn error(&self) -> &Arc<AssetLoadError> { &self.error }
}

/// Reports a failed startup asset set without requiring its concrete type.
#[derive(Event)]
pub struct AssetSetLoadFailed {
    set_type_id:  TypeId,
    set_name:     &'static str,
    tracked_path: AssetPath<'static>,
    error:        Arc<AssetLoadError>,
}

impl AssetSetLoadFailed {
    pub(crate) fn new<T: DiskAssets>(
        tracked_path: AssetPath<'static>,
        error: Arc<AssetLoadError>,
    ) -> Self {
        Self {
            set_type_id: TypeId::of::<T>(),
            set_name: std::any::type_name::<T>(),
            tracked_path,
            error,
        }
    }

    /// Returns the [`TypeId`] of the failed [`DiskAssets`] implementation.
    #[must_use]
    pub const fn set_type_id(&self) -> TypeId { self.set_type_id }

    /// Returns the fully qualified name of the failed asset-set type.
    #[must_use]
    pub const fn set_name(&self) -> &'static str { self.set_name }

    /// Returns the tracked asset path that failed to resolve.
    #[must_use]
    pub const fn tracked_path(&self) -> &AssetPath<'static> { &self.tracked_path }

    /// Returns the shared Bevy asset-load error.
    #[must_use]
    pub const fn error(&self) -> &Arc<AssetLoadError> { &self.error }
}

/// Reports that every registered startup asset set loaded successfully.
#[derive(Event)]
#[non_exhaustive]
pub struct AllSetsLoaded;

/// Reports that every registered startup asset set reached success or failure.
#[derive(Event)]
pub struct AllSetsResolved {
    failures: usize,
}

impl AllSetsResolved {
    pub(crate) const fn new(failures: usize) -> Self { Self { failures } }

    /// Returns the number of asset sets that failed.
    #[must_use]
    pub const fn failures(&self) -> usize { self.failures }
}

/// Stores readable aggregate state while startup asset sets are resolving.
#[derive(Resource, Default)]
pub struct LoadProgress {
    total:    usize,
    resolved: usize,
    failures: usize,
}

impl LoadProgress {
    pub(crate) const fn register_set(&mut self) { self.total += 1; }

    pub(crate) const fn resolve_loaded(&mut self) { self.resolved += 1; }

    pub(crate) const fn resolve_failure(&mut self) {
        self.resolved += 1;
        self.failures += 1;
    }

    pub(crate) const fn is_complete(&self) -> bool { self.resolved == self.total }

    pub(crate) const fn failures(&self) -> usize { self.failures }

    /// Returns the number of asset sets that resolved successfully.
    #[must_use]
    pub const fn loaded(&self) -> usize { self.resolved - self.failures }

    /// Returns the total number of registered asset sets.
    #[must_use]
    pub const fn total(&self) -> usize { self.total }
}

impl<T: DiskAssets> Loaded<T> {
    pub(crate) const fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

impl AllSetsLoaded {
    pub(crate) const fn new() -> Self { Self }
}
