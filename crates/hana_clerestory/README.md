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
`MonitorReporter` classifies panel evidence into a kernel `DeviceKey`, and
`MonitorDeviceAssociation` is the private exact-key bridge back to current geometry.

### `CurrentMonitor` Component

Automatically maintained on the primary window and every `ManagedWindow`. Query it to get monitor
information and the window's effective mode:

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

- `WindowManagerPlugin` – Uses executable name for config directory
- `WindowManagerPlugin::with_app_name("name")` – Custom app name
- `WindowManagerPlugin::with_path(path)` – Full control over state file path
- `WindowManagerPlugin::with_persistence(mode)` – Set persistence behavior for managed windows

### Multi-Window Support

Add `ManagedWindow` to any secondary window to opt it into save/restore:

```rust
use bevy::prelude::*;
use hana_clerestory::ManagedWindow;

# fn spawn_inspector(mut commands: Commands) {
commands.spawn((
    Window {
        title: "Inspector".into(),
        ..default()
    },
    ManagedWindow {
        name: "inspector".to_string(),
    },
));
# }
```

Each managed window gets the same restore treatment as the primary window — scale factor compensation, position clamping, and platform workarounds.

Control what happens when windows are closed with `ManagedWindowPersistence`:

- `RememberAll` (default) — closed windows keep their saved state for next launch
- `ActiveOnly` — only currently open windows are persisted

```rust
use bevy::prelude::App;
use hana_clerestory::ManagedWindowPersistence;
use hana_clerestory::WindowManagerPlugin;

# fn configure(app: &mut App) {
app.add_plugins(WindowManagerPlugin::with_persistence(ManagedWindowPersistence::ActiveOnly));
# }
```

See `examples/restore_window.rs` for a complete interactive example.

### Kernel-backed recovery model

Durable display identity, roles, policies, and attempts belong to `hana_rigging`. Clerestory keeps
only current monitor geometry and window-specific target preparation. A display reporter produces
the exact durable `DeviceKey`; the kernel issues `DeviceId`, `RoleKey`, and `AttemptId` values and
owns their lifetimes.

Window recovery uses the kernel policies directly:

| `RecoveryPolicy` | Window behavior |
| --- | --- |
| `Forget` | Retain no recovery configuration after the display leaves. This is the default. |
| `ReapplyOnRequest` | Report that the role is waiting and accept an application reapply request only while the kernel records that request as owed. |
| `ReapplyOnReturn` | Preserve the last configuration established by safe driver readback for an automatic return. |
| `Retain` | Preserve and report state without allowing automatic window output. |

`LastKnownGoodConfiguration::Known` contains the driver-specific window placement established by a
safe readback. `RoleState` and configured offline mode decide whether persistence may write that
value; Clerestory has no second writable/frozen registry.

The driver-backed window `Binding`, endpoint driver, and live fallback execution are intentionally
not registered by this migration layer. Those consumers will use the exact `MonitorDeviceLookup`
boundary rather than deriving identity from monitor index, proximity, or enumeration order.

#### Requests, availability, and retirement

`RoleAwaiting` and `RoleAvailable` report kernel role availability. An application-controlled
request uses `ReapplyConfiguration`, targeted at the binding entity for that role. The kernel acts
only when the binding uses `RecoveryPolicy::ReapplyOnRequest` and its `WaitingWork` is
`ApplicationRequestOwed`; the same event cannot start a startup restore or bypass another policy.

Removing a primary or managed window triggers `RetireRole { role }`. `RoleKey` remains stable across
window entities: Clerestory uses `window:primary` for the primary role and
`window:managed:<name>` for managed roles.

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
and semantic restore-result positions. Live endpoint-driver reconnect behavior remains outside this
layer until the window driver is registered.

### State File Format

The state file uses a versioned v4 schema:

- `version: 4`
- `entries: [{ key, state }, ...]`

All spatial values are stored in **logical pixels**. Current window positions are offsets from a
monitor's top-left corner and are converted with that monitor's live scale during restore. Where
the platform supplies durable panel evidence, `monitor_panel` targets that panel after a replug,
dock change, or driver renumbering. A current monitor-relative offset with anonymous or unmatched
panel identity is discarded during startup restore; Clerestory does not apply it to an arbitrary
primary display.

The v4 writer persists neither winit's current monitor-enumeration index nor a native display
handle. Both are runtime adapter values whose numbers may change across restarts. `key` is typed
(`Primary` or `Managed("<name>")`), so the primary window and a managed window named `"primary"`
remain distinct.

Unversioned, v1, v2, and v3 files remain readable and are written as v4 on their next save. Their
legacy `monitor_index` is parsed only for wire compatibility and never selects a live display.
Pre-v3 absolute coordinates retain the scale that wrote them. Restore reconstructs the saved
window center and rebases the coordinate only when exactly one current monitor's physical bounds
contain that center. No match or overlapping matches discard the coordinate safely. A migrated v3
`Unrebased` coordinate receives the same conservative treatment.

## Version Compatibility

| Version                     | Bevy |
|-----------------------------|------|
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
