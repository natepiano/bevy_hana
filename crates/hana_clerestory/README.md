# hana_clerestory

[![License](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](https://github.com/natepiano/hana/tree/main/crates/hana_clerestory#license)
[![Crates.io](https://img.shields.io/crates/v/hana_clerestory.svg)](https://crates.io/crates/hana_clerestory)
[![Downloads](https://img.shields.io/crates/d/hana_clerestory.svg)](https://crates.io/crates/hana_clerestory)
[![CI](https://github.com/natepiano/hana/actions/workflows/ci.yml/badge.svg)](https://github.com/natepiano/hana/actions/workflows/ci.yml)


A Bevy plugin that saves and restores window placement, handles mixed-scale
monitors, and can recover windows after a monitor reconnects.

> **Renamed.** This crate was published as `bevy_clerestory` through v0.2.0. The
> API and feature names are unchanged; only the crate name and its `TypePath`
> strings differ.

## Motivation

Originally created as a mechanism to restore the `PrimaryWindow` to its last known position when launching - the way you expect an app to work. I quickly discovered that on my `MacBook` Pro with Retina display (scale factor 2.0) and my external monitor (scale factor 1.0), there were numerous issues with saving/restoring positions across differently-scaled monitors. 

The first discovered issue is that winit uses the scale factor of the focused window from which you launch the application. And if the target monitor for the app has a different scale factor, then that will get factored into the size and position calculations resulting in something you definitely don't want.

`hana_clerestory` plugin works around this issue by using winit directly to capture actual monitor position/size/scale and comparing it to the target position/size for the window and does the conversions correctly.

Windows has similar scale factor issues, plus additional quirks like invisible window borders that prevent precise placement. Linux X11 has its own quirks with window manager keyboard shortcuts not firing position events. This plugin now supports macOS, Windows, and Linux (X11 and Wayland) with workarounds for platform-specific issues (see [Platform Support](#platform-support) for details).

Clerestory can also track monitor connections while an application is running and help recover windows after their monitor is disconnected and reconnected.

## Usage

```rust,no_run
use bevy::prelude::*;
use hana_clerestory::WindowManagerPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(WindowManagerPlugin)
        .run();
}
```

For a complete interactive example with fullscreen mode switching, run:

```bash
cargo run --example restore_window
```

## API

This crate exposes several types for working with monitors and windows beyond the plugin itself. See [docs.rs](https://docs.rs/hana_clerestory) for full API documentation.

### `Monitors` Resource

`Monitors` contains the displays currently known to Bevy. Its order comes from winit and can change
when displays are disconnected or reconnected, so an index is useful for the current inventory but
is not a permanent monitor identity.

- `monitors.at(physical_x, physical_y)` – Find the monitor containing a position (physical pixels)
- `monitors.by_index(index)` – Find the monitor currently reporting `index`
- `monitors.first()` – Get the first monitor in winit's current order; this is not necessarily the primary monitor
- `monitors.closest_to(physical_x, physical_y)` – Find the closest monitor to a position (physical pixels)
- `monitors.iter()` – Iterate over each current monitor entity and its `MonitorDescriptor`

### `MonitorDescriptor`

Geometry and window-system adapter data for one current monitor: `index`, `scale`,
`physical_position`, and `physical_size`. It intentionally contains no durable identity. The
`MonitorReporter` classifies display evidence into a kernel `DeviceKey` and projects a
`LiveDisplayEndpoint` capability onto that device with its current monitor entity, descriptor, and
legacy identity. `LiveDisplayEndpointLookup` supplies exact reverse lookups at boundaries that begin
from a descriptor or legacy identity rather than a resolved device.

### `CurrentMonitor` Component

Automatically maintained on every `ManagedWindow`, which is every window Clerestory manages — the
primary window included. Query it to get monitor information and the window's effective mode:

```rust
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use hana_clerestory::CurrentMonitor;

fn my_system(q: Query<(&Window, &CurrentMonitor), With<PrimaryWindow>>) {
    let Ok((window, monitor)) = q.single() else {
        return;
    };
    println!("Monitor {}, scale {}", monitor.index, monitor.scale);
    println!("Effective mode: {:?}", monitor.effective_window_mode);
}
```

- `monitor.index`, `monitor.scale`, `monitor.physical_position`, `monitor.physical_size` – Current geometry (via `Deref<Target = MonitorDescriptor>`)
- `monitor.effective_window_mode` – The actual window mode, even when `window.mode` is stale (e.g., macOS green button fullscreen reports `Windowed` but the window is actually fullscreen)

### Plugin Configuration

- `WindowManagerPlugin` – Uses the executable name for its config directory and restores saved
  positions without opting the primary window into runtime return recovery.
- `WindowManagerPlugin::with_app_name("name")` – Uses a custom app name.
- `WindowManagerPlugin::with_path(path)` – Uses an exact state-file path.
- `WindowManagerPlugin::with_persistence(mode)` – Selects managed-window record retention.

Each constructor returns the public `ConfiguredWindowManagerPlugin`. Chain `recover_on_return()` or
`recover_on_request()` to select the primary window's authored recovery policy; if both builders
are called, the last call wins:

```rust,no_run
use bevy::prelude::App;
use hana_clerestory::WindowManagerPlugin;

# fn configure(app: &mut App) {
app.add_plugins(WindowManagerPlugin::with_app_name("my-app").recover_on_return());
# }
```

### Multi-Window Support

Add `ManagedWindowName` to any secondary window to give it a save key and launch-time restore.
`ManagedWindow` arrives with the name and marks the window as one Clerestory manages; the plugin
puts it on the primary window itself, so `With<ManagedWindow>` matches the primary too. Add a
recovery marker at the spawn site only when that window should return after a runtime display
departure:

```rust,no_run
use bevy::prelude::App;
use bevy::prelude::Commands;
use bevy::prelude::Startup;
use bevy::prelude::Window;
use bevy::prelude::default;
use hana_clerestory::ManagedWindowName;
use hana_clerestory::RecoverOnRequest;
use hana_clerestory::RecoverOnReturn;
use hana_clerestory::WindowManagerPlugin;

# fn configure(app: &mut App) {
app.add_plugins(WindowManagerPlugin::with_app_name("my-app").recover_on_return())
    .add_systems(Startup, spawn_windows);
# }

fn spawn_windows(mut commands: Commands) {
    commands.spawn((
        Window {
            title: "Inspector".into(),
            ..default()
        },
        ManagedWindowName("inspector".into()),
        RecoverOnRequest,
    ));
    commands.spawn((
        Window {
            title: "Dashboard".into(),
            ..default()
        },
        ManagedWindowName("dashboard".into()),
        RecoverOnReturn,
    ));
}
```

The inspector waits for an application request after its display returns; the dashboard returns
automatically. A managed window with neither marker still restores its saved position at launch.
Every managed window receives scale-factor compensation, position clamping, and platform
workarounds.

Control what happens when windows are closed with `ManagedWindowPersistence`:

- `RememberAll` (default) — closed managed windows keep their saved state for next launch.
- `ActiveOnly` — records are kept only for windows that are managed right now. A managed window
  whose display is absent keeps its existing record even when it cannot produce a fresh
  configuration; closing that managed window removes the record.

```rust
use bevy::prelude::App;
use hana_clerestory::ManagedWindowPersistence;
use hana_clerestory::WindowManagerPlugin;

# fn configure(app: &mut App) {
app.add_plugins(WindowManagerPlugin::with_persistence(ManagedWindowPersistence::ActiveOnly));
# }
```

Run `cargo run --example restore_window` for a complete interactive example.

### Kernel-backed recovery model

Durable display identity, roles, policies, and attempts belong to `hana_rigging`. Clerestory keeps
only current monitor geometry and window-specific target preparation. A display reporter produces
the exact durable `DeviceKey`; the kernel issues `DeviceId`, `RoleKey`, and `AttemptRef` values and
owns their lifetimes.

Clerestory maps recovery markers to kernel policy when it authors a window binding:

| Window marker or markers | Authored `RecoveryPolicy` | Window behavior |
| --- | --- | --- |
| Neither marker | `Forget` | Restore the saved position at launch. After a runtime display departure, adopt the surviving display and stay there. This is the default. |
| `RecoverOnReturn` | `ReapplyOnReturn` | Preserve the departed endpoint and return automatically when that display is available. |
| `RecoverOnRequest` | `ReapplyOnRequest` | Preserve the departed endpoint and wait for binding-targeted `ReapplyConfiguration`. |
| Both markers | `ReapplyOnRequest` | Use request-controlled recovery and log one warning. Marker insertion order does not affect the result. |
| Not authored by Clerestory | `Retain` | Preserve and report state without providing a path that reapplies window output. |

The markers are authoring-time configuration. Inserting one after Clerestory has authored the
window's binding does not change that binding's policy. Markers are valid only on an entity
carrying `ManagedWindow`; a marker on another entity logs one warning and authors no role.

When a display leaves at runtime, the binding's `waiting_work` records the departure work a user or
application can act on. `RecoverOnReturn` records restoration work that the returning display
discharges automatically. `RecoverOnRequest` records application-request work that
`ReapplyConfiguration` consumes. A fallback rebind does not cancel this work.

`LastKnownGoodConfiguration::Known` contains the driver-specific window placement established by a
safe readback. The binding lifecycle and configured offline mode decide whether persistence may
write that value; Clerestory has no second writable/frozen registry.

`ConfiguredWindowManagerPlugin::build` registers the display integration, window endpoint driver,
binding authoring, and live fallback execution. These paths use the exact
`LiveDisplayEndpoint` capability through the kernel's resolved device relationships.
`LiveDisplayEndpointLookup` remains the evidence-to-device reverse-lookup boundary; neither path
derives durable identity from monitor index, proximity, or enumeration order.

#### Startup reveal and runtime departure

Each hidden managed window owns a `SavedDisplayRevealWait`. If no normal restore can
reveal it before that bounded wait ends, Clerestory distinguishes two outcomes:

- `NoSavedConfiguration` reveals a role that has never established a saved configuration.
- `SavedDisplayUnavailable` logs a warning naming the role and elapsed wait, fits the saved
  geometry onto the live display where the window launched, and reveals it.

For `SavedDisplayUnavailable`, the saved target continues to name the absent display for every
recovery policy until the user moves the window. That move adopts the live display. This startup
behavior is distinct from runtime departure: `Forget` adopts the surviving display during the
fallback, while `RecoverOnReturn` and `RecoverOnRequest` retain the departed endpoint for their
respective return paths.

#### Requests, availability, and retirement

`LiveRoleChanged::Status` reports kernel role availability. An application-controlled request uses
`ReapplyConfiguration`, targeted at the binding entity for that role. The kernel acts only when the
binding uses `RecoveryPolicy::ReapplyOnRequest` and its `WaitingWork` is
`ApplicationRequestOwed`; the same request cannot start a startup restore or bypass another policy.

Use the public `primary_window_role()` and `managed_window_role(name)` constructors instead of
reproducing Clerestory's role strings. Both return `Result<RoleKey, RoleKeyError>`.
`managed_window_role(name)` expects the current canonicalized `ManagedWindowName`; a duplicate
name is rewritten in that component before the role is derived.

Removing `PrimaryWindow` or `ManagedWindowName` detaches that window lifetime. An opted-in role keeps
its binding and departure work so another window entity can reattach. `RetireRole` is the signal
that permanently ends such a role, so an application that no longer intends to respawn the window
must emit `RetireRole { role }`. A `Forget` role has no return work to retain and Clerestory retires
it when its window lifetime ends.

#### Platform limits

An exact display key and a safe window placement are both required for automatic return. macOS,
Windows, and X11 allow applications to choose a windowed position. Wayland compositors choose
windowed positions, so a Wayland driver cannot promise a windowed return coordinate. The semantic
position fields on `WindowRestored` and `WindowRestoreMismatch` preserve whether a coordinate was
specified, unavailable on the platform, absent from the saved record, or discarded as unsafe legacy
data.

#### Keep the application alive with no windows

Bevy normally exits when its last window disappears. If the application must stay alive long enough
to recover deleted windows, configure `WindowPlugin` with `ExitCondition::DontExit`. The application
must then decide what should exit it. This example sends `AppExit` when the operating system requests
that any window close:

```rust,no_run
use bevy::prelude::*;
use bevy::window::ExitCondition;
use bevy::window::WindowCloseRequested;
use bevy::window::WindowPlugin;
use hana_clerestory::WindowManagerPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            exit_condition: ExitCondition::DontExit,
            ..default()
        }))
        .add_plugins(WindowManagerPlugin)
        .add_systems(Update, exit_when_close_is_requested)
        .run();
}

fn exit_when_close_is_requested(
    mut close_requests: MessageReader<WindowCloseRequested>,
    mut exit: MessageWriter<AppExit>,
) {
    if close_requests.read().next().is_some() {
        exit.write(AppExit::Success);
    }
}
```

#### Remote control through BRP

Bevy Remote Protocol (BRP) clients can observe `WindowRestored` and `WindowRestoreMismatch` with
`world.observe+watch`. Kernel role and recovery events retain their `hana_rigging` reflected paths;
application requests use the binding-targeted `ReapplyConfiguration` event and role retirement uses
`RetireRole`.

#### What the automated tests cover

Automated Bevy tests cover reporter identity classification, exact key-to-live-monitor association,
configured and previously observed absent devices, duplicate-key refusal, RoleKey-backed managed
window retirement, kernel-owned persistence eligibility, conservative legacy-coordinate migration,
and semantic restore-result positions. The production window driver and recovery lifecycle claims
above are checked by these named tests:

- Display-driven window loss, policy retention across respawn, and fresh per-window baselines:
  `display_driven_window_loss_respawns_across_all_recovery_marker_states` and
  `respawned_stranded_window_starts_fresh_display_and_placement_baselines`.
- Runtime departure work drives automatic and application-requested return:
  `recover_on_return_primary_window_goes_home_when_its_display_returns` and
  `recover_on_request_primary_window_waits_for_an_explicit_return_request`.
- A return that arrives while the window is detached waits for a replacement window:
  `automatic_return_arriving_while_detached_defers_until_respawn` and
  `application_request_arriving_while_detached_defers_until_respawn`.
- Explicit `RetireRole` ends every window policy and clears its recovery baselines:
  `explicit_retirement_clears_every_window_recovery_policy_and_baseline` in
  `recovery/fallback_and_return.rs`.
- Per-role bounded reveal, unavailable-target retention, adoption after a move, and a managed
  window created after startup: `reveals_a_bound_role_whose_display_is_absent`,
  `saved_display_unavailable_probe_reveals_then_adopts_after_move`,
  `a_startup_revealed_stranded_window_rebinds_when_moved_to_a_live_display`, and
  `a_late_managed_window_is_authored_revealed_and_only_saves_after_a_move`.

### State File Format

The state file uses a versioned v5 schema:

- `version: 5`
- `entries: [{ key, state }, ...]`

All spatial values are stored in **logical pixels**. Current window positions are offsets from a
monitor's top-left corner and are converted with that monitor's live scale during restore. A v5
target is either `Classified(DeviceKey)`, which names the exact reporter-classified display, or
`AwaitingLegacyEvidence`, which preserves v4 display evidence until a later reporter scan can resolve
it. Clerestory never applies an unmatched target to an arbitrary primary display.

The v5 writer persists neither winit's current monitor-enumeration index nor a native display
handle. Both are runtime adapter values whose numbers may change across restarts. `key` is typed
(`Primary` or `Managed("<name>")`), so the primary window and a managed window named `"primary"`
remain distinct.

Unversioned and v1 through v4 files remain readable and are written as v5 on their next save. Their
legacy `monitor_index` is parsed only for wire compatibility and never selects a live display.
Pre-v3 absolute coordinates retain the scale that wrote them. Restore reconstructs the saved
window center and rebases the coordinate only when exactly one current monitor's physical bounds
contain that center. No match or overlapping matches discard the coordinate safely. A migrated v3
`Unrebased` coordinate receives the same conservative treatment.

## Version Compatibility

| Version                     | Bevy |
|-----------------------------|------|
| `hana_clerestory` 0.4       | 0.19 |
| `hana_clerestory` 0.3       | 0.19 |
| `bevy_clerestory` 0.1 – 0.2 | 0.19 |

## Platform Support

This table records physical testing of window placement save/restore. Physical reconnect test
results are tracked separately (see
[What the automated tests cover](#what-the-automated-tests-cover)).

| Platform | Status | Notes |
|----------|--------|-------|
| macOS    | ✅ Tested | Native hardware with multiple monitors at different scales |
| Windows  | ✅ Tested | `VMware` VM with multi-monitor, different scale factors |
| Linux X11 | ✅ Tested | Position and size restoration with keyboard snap workaround |
| Linux Wayland | ✅ Tested | Size + fullscreen only (Wayland cannot query/set position) |


**Note on Windows testing**: Windows support has been tested in a `VMware` virtual machine with multiple monitors at different scale factors. Native Windows installations may behave differently - if you encounter issues, please open an issue with details about your monitor configuration.

**Note on Linux support**: Linux support has been tested on KDE Plasma (Asahi Linux on Fedora). X11 includes a workaround for keyboard snap shortcuts (Meta+Arrow) that don't fire position events ([winit #4443](https://github.com/rust-windowing/winit/issues/4443)). Wayland has an inherent limitation: clients cannot query or set window position, so only size and fullscreen state can be restored. If you encounter issues, please open an issue with details about your distribution, desktop environment, and monitor configuration.

## Feature Flags (Platform Workarounds)

This plugin includes workarounds for known issues in winit and Bevy. Each workaround is behind a feature flag, and **all are enabled by default**.

This design allows:
- **Easy testing of upstream fixes** - disable a workaround to verify an upstream fix works
- **Opt-out flexibility** - if a workaround doesn't suit your setup, you can exclude it
- **Minimal code when not needed** - platform-specific workarounds are compiled out on other platforms

### Available Feature Flags

| Feature | Platform | Issue | Description |
|---------|----------|-------|-------------|
| `workaround-winit-4341` | Windows | [winit #4041](https://github.com/rust-windowing/winit/issues/4041) | DPI drag bounce fix |
| `workaround-winit-3124` | Windows | [winit #3124](https://github.com/rust-windowing/winit/issues/3124) | DX12/DXGI fullscreen crash fix |
| `workaround-winit-4443` | Linux X11 | [winit #4443](https://github.com/rust-windowing/winit/issues/4443) | Keyboard snap position fix |
| `workaround-winit-4440` | Windows, macOS, Linux X11 | [winit #4440](https://github.com/rust-windowing/winit/issues/4440) | Multi-monitor scale factor compensation |

### Disabling Workarounds

To test without a specific workaround (e.g., to verify an upstream fix):

```bash
# Disable all workarounds
cargo run --example restore_window --no-default-features
```

In your `Cargo.toml`, you can selectively enable features:

```toml
[dependencies]
hana_clerestory = { version = "0.3", default-features = false, features = ["workaround-winit-4341"] }
```

## License

`hana_clerestory` is free, open source and permissively licensed!
Except where noted (below and/or in individual files), all code in this repository is dual-licensed under either:

* MIT License ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))

at your option.

### Your contributions

Unless you explicitly state otherwise,
any contribution intentionally submitted for inclusion in the work by you,
as defined in the Apache-2.0 license,
shall be dual licensed as above,
without any additional terms or conditions.
