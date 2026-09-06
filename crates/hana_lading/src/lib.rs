//! Public API for startup disk-asset loading and completion reporting in Bevy.
//!
//! Applications define a startup asset set by implementing [`DiskAssets`] and
//! register its plugin type with [`DiskAssetsPlugin`]. The crate defines the
//! asset-set, loader, plugin, readable [`LoadProgress`] resource, and
//! completion-event APIs.
//!
//! Each set resource exists before its [`Loaded`] event. At that point every
//! tracked root and recursive dependency is loaded into Bevy's asset storage;
//! renderer-specific GPU preparation may still be pending. Progress is updated
//! before per-set observers run, and per-set observers run before global
//! completion observers: their direct resource mutations are visible to global
//! completion observers, but commands they queue are not guaranteed to apply
//! before global completion. Each set emits exactly one of
//! [`Loaded`] or [`LoadFailed`]. On failure, the type-erased
//! [`AssetSetLoadFailed`] event is delivered immediately before the typed
//! [`LoadFailed`] event. Cross-set terminal-event order is unspecified.
//!
//! [`AllSetsLoaded`] is emitted only when every registered set succeeds.
//! [`AllSetsResolved`] always follows it for a batch with no failures, and is
//! the only global event emitted for a batch containing failures. Only direct resource
//! mutations made by one completion observer are guaranteed to be visible to
//! the following completion observer; deferred commands wait for the next
//! command flush.
//!
//! This crate applies no failure policy. Whether a failed set blocks startup,
//! switches the application to a degraded mode, or records additional failure
//! details is left to the application.
//!
//! # Startup sequence
//!
//! 1. Define a resource that implements [`DiskAssets`].
//! 2. Register [`DiskAssetsPlugin`] for each startup set.
//! 3. Load every handle through [`DiskAssetLoader`] in [`DiskAssets::load`].
//! 4. Observe [`Loaded`] or [`LoadFailed`] for per-set work.
//! 5. Observe [`AllSetsLoaded`] or [`AllSetsResolved`] for the whole-batch decision.
//!
//! [`AssetSetLoadFailed`] is suitable for one durable failure recorder because
//! it identifies every failed set without its concrete resource type. Its
//! observer runs before the typed [`LoadFailed`] observer and before global
//! completion. Direct resource writes are therefore visible to the global
//! decision observer.
//!
//! # Failure policy and headless exit
//!
//! A windowed application can remain in its loading state after a required
//! failure or enter a ready state after only optional content fails. A headless
//! application can instead observe [`AssetSetLoadFailed`] and write
//! `AppExit::error()` through `MessageWriter<AppExit>`. The
//! `exit_on_failure_pattern` integration test runs that sequence with Bevy's
//! `ScheduleRunnerPlugin`.

mod disk_asset_loader;
mod events;
mod lading_plugin;

pub use disk_asset_loader::DiskAssetLoader;
pub use disk_asset_loader::DiskAssets;
pub use events::AllSetsLoaded;
pub use events::AllSetsResolved;
pub use events::AssetSetLoadFailed;
pub use events::LoadFailed;
pub use events::LoadProgress;
pub use events::Loaded;
pub use lading_plugin::DiskAssetsPlugin;
pub use lading_plugin::LadingPlugin;
