# hana_lading

`hana_lading` provides startup disk-asset loading, completion tracking, and
failure reporting for Bevy applications.

## Workflow

1. Define a resource that implements `DiskAssets`.
2. Load every startup handle through the supplied `DiskAssetLoader`.
3. Add `DiskAssetsPlugin::<YourAssets>::default()` to the application.
4. Observe `Loaded<YourAssets>` or `LoadFailed<YourAssets>` for set-specific
   work.
5. Observe `AllSetsLoaded` for the all-success path or `AllSetsResolved` for
   every terminal batch.
6. Apply application policy: continue, enter a degraded mode, remain in a
   loading state, or exit.

The resource is inserted before its typed completion event. A successful set
resolves only after every tracked root and recursive dependency reaches Bevy's
loaded state. Renderer-specific GPU preparation may still be pending.

The Hana Lading registration uses ordinary Bevy `App` methods:

```rust
app
    .add_plugins(DiskAssetsPlugin::<StartupAssets>::default())
    .add_observer(on_startup_assets_loaded)
    .add_observer(on_all_sets_loaded);
```

The visual examples obtain that `&mut App` through Fairy Dust's `app_mut()`,
install the Hana Lading plugins and observers on it, then let Fairy Dust supply
the camera, lighting, scene framing, controls, and panels. Customer applications
use the same `App` calls without depending on Fairy Dust.

## Failure semantics

Each set emits exactly one terminal typed event. A failure also emits
`AssetSetLoadFailed` immediately before `LoadFailed<T>`, allowing one generic
observer to record every set. Direct resource writes made by per-set observers
are visible to global completion observers; deferred commands are not promised
to flush between them. `AllSetsLoaded` occurs only when every set succeeds,
while `AllSetsResolved` occurs for every completed batch.

`hana_lading` supplies evidence, not application policy. The examples show a
normal startup and two different failure decisions from the same event model:

```sh
cargo run -p hana_lading --example successful_loading
cargo run -p hana_lading --example degraded_failure
cargo run -p hana_lading --example catastrophic_failure
```

The successful example applies the loaded PNG to its cube, receives
`AllSetsLoaded`, enters `Ready`, and leaves the complete event sequence visible
in a persistent panel. The degraded example keeps required content and enters
`Ready` after an optional set fails. The catastrophic example displays the
required-set failure and remains in `Loading`.

## What the examples protect

1. Every handle returned by `DiskAssetLoader` is retained for tracking.
2. Terminal success waits for recursive dependencies. The
   `recursive_dependencies_gate_and_fail` contract test is the executable proof
   because the example PNG has no recursive child.
3. Failed loads resolve with evidence instead of waiting indefinitely.
4. Generic failure observers record any asset-set type.
5. Direct failure-record writes occur before global completion policy runs.
6. The application—not the loading crate—chooses continue, degrade, block, or
   exit.

## Headless exit on failure

A service or test runner can observe `AssetSetLoadFailed` and write
`AppExit::error()` through `MessageWriter<AppExit>`. The
`exit_on_failure_pattern` contract test runs that recipe with
`ScheduleRunnerPlugin` and verifies the non-success exit result.
