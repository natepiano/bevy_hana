# Changelog

## [Unreleased]

### Added

- Monitor identity and window recovery now run on the
  [`hana_rigging`](https://crates.io/crates/hana_rigging) kernel, a new
  dependency. Clerestory keeps what only it can do — list the monitors, hold
  window-system geometry, talk to winit — and hands the rest to the kernel:
  telling one physical display from another, tying a window to one, and deciding
  what happens when that display comes back.
- `MonitorDescriptor` carries the geometry and window-system data for one live
  monitor. `MonitorDeviceAssociation` is the Clerestory-owned table that ties
  live monitors to the identifiers the kernel issued for them, so Clerestory and
  the kernel always agree on which display is which.
- `MonitorDeviceKeyLookup` finds the kernel's identifier for a live monitor. When
  it has no answer it returns nothing: it never infers a display's identity from
  its geometry, its position in the list, or whether it is the primary, because
  tying a window to a position in a list would hand that window to whichever
  display later occupies it.
- `DisplayProductName` reports the name the operating system shows the user for
  a display. It is for showing the user, and must never be used to tell one
  display from another.
- `CurrentMonitorIndex`, for APIs such as `MonitorSelection::Index`. It has no
  serde implementation, and turning it into an integer takes an explicit call.
- `RecoverOnReturn` and `RecoverOnRequest` opt a window into coming back after
  its display is unplugged and plugged in again — on its own, or when the
  application asks. A window with neither marker stays where it is. If a window
  carries both, `RecoverOnRequest` wins, one warning is logged, and the result
  does not depend on which was added first.
- `ConfiguredWindowManagerPlugin`, returned by `with_app_name`, `with_path`, and
  `with_persistence`. Its `recover_on_return()` and `recover_on_request()`
  builders make that choice for the primary window.
- `primary_window_role()` and `managed_window_role(name)` build the checked
  `RoleKey` values Clerestory uses. Both return `Result`.
- `ExpectedPhysicalPosition`, `ExpectedLogicalPosition`,
  `ObservedPhysicalPosition`, and `ObservedLogicalPosition` replace the bare
  `Option<IVec2>` coordinates on the restore events. A compositor that cannot
  report a position is no longer indistinguishable from a window at `(0, 0)`,
  and a missing expected coordinate now says why — the platform owns placement,
  nothing was saved, or an old absolute coordinate was thrown out as no longer
  safe to reuse.
- Under the `test` feature: `DisplayTestAdapter`, `DisplayTestDescriptor`,
  `DisplayTestDeviceKey`, `DisplayTestEnumeration`, and
  `DisplayTestReporterLookup` script a set of displays, so a consumer can test
  against the real display code instead of a stand-in.

### Changed

- **Breaking, and silent — read this before upgrading:** `ManagedWindow` is now
  a fieldless marker on every window Clerestory manages, **the primary window
  included**. The plugin puts it there itself. Its name moved to a new
  `ManagedWindowName(String)` component, which a secondary window carries beside
  it; the primary window never has one, because its role is `window:primary`.

  Neither the compiler nor your tests report either of these two consequences:

  1. **`With<ManagedWindow>` filters silently widen to include the primary
     window.** Code that used `With<ManagedWindow>` to mean "secondary windows
     only" now also matches the primary. Nothing catches this — it compiles, it
     runs, and it quietly does more than it did. Add
     `Without<PrimaryWindow>`, or filter on `With<ManagedWindowName>`, wherever
     you meant secondaries.
  2. **Saved bevy scenes holding the old `ManagedWindow { name }` definition fail
     to load.** `ManagedWindow` is auto-registered for reflection, so its old
     field list is recorded in any scene that captured a managed window. Re-author
     those scenes, or strip the component from them, before loading.

  Spawning changes too:

  | Your earlier 0.4.0-dev code | What to write now |
  | --------------------------- | ----------------- |
  | `ManagedWindow { name: "inspector".into() }` | `ManagedWindowName("inspector".into())` |
  | `Query<&ManagedWindow>` for the name | `Query<&ManagedWindowName>`, read `.0` |
  | `remove::<ManagedWindow>()` to detach | unchanged; removing `ManagedWindowName` also detaches |

  `ManagedWindowName` requires `ManagedWindow`, so spawning the name alone is
  enough to put a window under Clerestory. Save-file role keys are unchanged: `window:primary` for
  the primary window and `window:managed:<name>` for a secondary. Existing
  state files keep loading.

- **Breaking:** `WindowRestored` and `WindowRestoreMismatch` keep their names
  but not their fields. Their position fields are the four types above instead of
  `Option<IVec2>`, and `window_key: WindowKey` is replaced by `role: RoleKey`
  from `hana_rigging`. Match on the variants rather than testing for `None`.
- **Breaking:** bringing a window back after its display is unplugged and
  plugged in again is now opt in per window, and is a separate decision from
  restoring a window's saved position at launch. Add `RecoverOnReturn` where you
  used `WindowRecovery::FallbackAndReturn`, or `RecoverOnRequest` where you used
  `WindowRecovery::ApplicationControlled`. A window with neither marker still
  restores its saved position at launch, but stays where it lands when a display
  is unplugged — which is what leaving `WindowRecovery` off already did in
  0.3.0. Restoring at launch needs no marker: the primary window always
  restores, and a secondary window restores when it carries
  `ManagedWindowName`.

  | Your 0.3.0 code | What to do in 0.4.0 |
  | --------------- | ------------------- |
  | `WindowRecovery::Disabled` | delete it; add nothing in its place |
  | `WindowRecovery::FallbackAndReturn` | replace it with `RecoverOnReturn` |
  | `WindowRecovery::ApplicationControlled` | replace it with `RecoverOnRequest` |
- `with_app_name`, `with_path`, and `with_persistence` return the concrete
  `ConfiguredWindowManagerPlugin` instead of `impl Plugin`. Existing callers can
  still box the result as `Box<dyn Plugin>` or return it from
  `fn make() -> impl Plugin`.
- The saved window-state file moves to v5, which records the display a window
  was on using the kernel's identifier for it. A window saved by v4 that the
  kernel has not yet matched to a live display keeps its old evidence until it
  can be matched. Files written by the old single-window format and by v1
  through v4 still load and are converted forward, so existing saved state
  survives the upgrade; a file written by this version cannot be read by 0.3.0.
- The sequence dependency is now
  [`hana_kana`](https://crates.io/crates/hana_kana) rather than `bevy_kana`,
  whose final release under the old name was 0.3.1.
- `bevy_camera` joins the crate's bevy features, only so systems can be ordered
  against it. It pulls in no render or wgpu dependency.

### Removed

- **Breaking:** the application-facing window-recovery API — `WindowRecovery`,
  `RestoreWindow`, `CancelWindowRecovery`, `WindowRecoveryAvailable`, and
  `WindowRecoveryPending`. Migrate `WindowRecovery` variants with the table
  under Changed. Migrate the two window-recovery availability edges to kernel role status,
  `RestoreWindow` to
  `ReapplyConfiguration` aimed at the window, and `CancelWindowRecovery` to
  `RetireRole`. These replacements belong to `hana_rigging`; a consumer that
  uses them directly must depend on `hana_rigging` too.
- **Breaking:** `MonitorId`, `MonitorIdentity`, and `MonitorInfo`, replaced by
  `MonitorDescriptor` plus the two lookups above. `MonitorConnected` and
  `MonitorDisconnected` are gone with them — displays appearing and
  disappearing now reach applications through the kernel rather than through
  Clerestory events.
- **Breaking:** `WindowKey`. Windows are identified by the kernel's `RoleKey`.

### Fixed

- A window whose saved display is not plugged in at launch no longer stays
  hidden. It opens on a display that is available, fitted to that display, and
  remembers the display it was saved against until the user moves it — so it can
  go back when that display returns. That is separate from a display being
  unplugged while the application runs: a window with no marker settles on the
  display it landed on, while `RecoverOnReturn` and `RecoverOnRequest` keep the
  one that left.
- Moving a window onto a surviving display no longer cancels its pending
  return. The window still goes back when its display returns, or when the
  application asks for it with `ReapplyConfiguration`.
- Destroying a window, or removing its `PrimaryWindow` or `ManagedWindow`
  component, no longer gives up on it. Its saved placement and its pending
  return survive, and attach to a window spawned in its place. Only `RetireRole`
  ends a window's recovery for good.
- A rejected entry in the saved file whose key still decodes now names the
  window in the warning and says that window will open at its default position.
- A restore that landed somewhere other than where it was aimed — macOS pulling
  an overhanging window back on screen, or the user dragging it mid-restore —
  used to wait out a two-second timeout and record a failure. That failure then
  stopped Clerestory from saving any window move for the rest of the session, so
  moves were lost and the next launch restored a stale file. Such a landing is
  now accepted as soon as the window stops moving, and is recorded as where the
  window actually is.
- Moving a window between displays is now saved correctly. The saved record
  could name the display the window launched on beside an offset measured
  against the one it had been moved to, sending the window back on the next
  launch. Moves within one display now reach the saved file too, and v1 and v2
  files, which record nothing about the display, adopt the display the window is
  on.
- Restoring a window across displays of different pixel density resized the
  window without announcing it, so a camera read the old size and paired a depth
  buffer sized to the display with a color buffer sized to the window. wgpu
  rejected the frame and Bevy quit. Clerestory now announces its own resizes in
  `PostUpdate`, before cameras read them.

## [0.3.0] - 2026-07-30

### Changed

- Renamed from `bevy_clerestory` to `hana_clerestory`. No API changes; update the
  dependency name and any `bevy_clerestory::` paths, including reflected
  `TypePath` strings in serialized scenes. Feature names are unchanged.
  `bevy_clerestory` 0.2.1 is a deprecated re-export shim and the final release
  under the old name.

## [0.2.0] - 2026-07-29

### Added
- While the application remains running, its windows can now return to the same
  physical monitor after it is disconnected and reconnected. If the operating
  system moves a surviving window to another display, Clerestory can return it
  later. If Bevy deletes the window with the disconnected monitor, Clerestory
  can create one replacement `Window` on an available display and return that
  replacement when the monitor comes back.
- Opt a primary or managed window into reconnect handling by adding
  `WindowRecovery`. Clerestory waits until the window is associated with a
  verified monitor, then remembers that monitor until recovery is cancelled.
  Moving the window or changing/removing the component does not silently choose
  a new target.
- `WindowRecovery` provides three policies:
  - `Disabled` leaves the window outside reconnect handling.
  - `ApplicationControlled` notifies the application when the monitor
    disappears and returns. The application creates or selects the window and
    sends `RestoreWindow` when it is ready.
  - `FallbackAndReturn` tracks a surviving window on another display or creates
    a replacement window, then returns it automatically when the same verified
    monitor comes back.
- Clerestory restores the Bevy `Window` and its settings, but it does not clone
  application-owned cameras, UI, or other content. Applications attach that
  content when a replacement gains its `PrimaryWindow` or `ManagedWindow`
  component.
- `WindowRecoveryPending` reports that a registered monitor disappeared. For
  `ApplicationControlled` recovery, `WindowRecoveryAvailable` reports that it
  returned. Both identify the affected window with its stable `WindowKey`,
  which identifies the primary window or a named managed window across entity
  replacement. `RestoreWindow` names the replacement entity for an
  application-controlled restore.
  `CancelWindowRecovery` uses the stable key, so it still works after the
  original entity has been deleted; it keeps any surviving window where it is
  and stops automatic return.
- Physical monitor matching within one running application. When the operating
  system supplies enough identifying information, Clerestory assigns
  `MonitorIdentity::Verified(MonitorId)` and can recognize the same monitor
  after its Bevy entity or enumeration index changes. Otherwise the monitor is
  `Unverified`, and Clerestory does not guess from its connector, position, or
  index. A `MonitorId` is valid only in the current process and is never saved.
- `MonitorConnected` and `MonitorDisconnected` events report changes to the
  available monitors. Each event includes the affected monitor entity and a
  copy of its `MonitorInfo`; disconnect events retain the last known
  information after the entity is gone.
- `Monitors::iter()` returns `LiveMonitor` values containing each current
  monitor entity and its information. `MonitorTopologyRevision` changes when
  Clerestory installs an updated monitor inventory.
- Recovery notifications, monitor connection events, and restore results can
  be observed through the Bevy Remote Protocol (BRP) with
  `world.observe+watch`. BRP clients can send `RestoreWindow` and
  `CancelWindowRecovery` through `world.trigger_event`.
- The `restore_after_reconnect` example demonstrates both recovery policies,
  records an ordered diagnostic log, includes automated tests, and provides a
  two-cycle manual monitor-disconnect script.
- Linux monitors are now identified from `/sys/class/drm` when X11 reports no
  EDID: the kernel's EDID blob, or the connector name for built-in panels
  (`eDP`, `LVDS`, `DSI`, `DPI`), so they reach `MonitorIdentity::Verified`
  under XWayland and on EDID-less internal panels.

### Changed
- `MonitorInfo` gains an `identity: MonitorIdentity` field. Breaking for code
  that constructs `MonitorInfo` directly; existing field access is unaffected.
- `Monitors::list` is no longer public. Use `Monitors::iter()` for live
  monitor entities and information, or use the existing lookup methods. This
  breaks code that read or replaced the list directly.
- Monitor order and index lookup no longer use the position-sorted list from
  0.1.1. `first()` now returns the first monitor in Bevy's current winit
  enumeration order, which is not necessarily the primary or leftmost monitor.
  `by_index(i)` finds the monitor currently reporting index `i`; it is not the
  same as indexing a dense vector. Use `at()` or `closest_to()` for position,
  and `by_id()` for a verified physical monitor.
- The saved state file is read once during startup. Clerestory then keeps the
  current window state in memory and writes only after that state changes.
  While a window is waiting to return automatically, its saved placement stays
  unchanged until restoration finishes or the application cancels recovery.
- The saved state file moves to schema v3. A window position is stored as an
  offset from its monitor instead of an absolute desktop coordinate, and each
  entry records a fingerprint of the panel it was saved against — an EDID hash
  on Windows and X11, the ColorSync UUID on macOS — so the saved monitor is
  still found after a replug, dock, or driver update renumbers the displays.
  Where no panel evidence is available (Wayland, or a virtual display with no
  usable EDID) the entry stays anonymous and the saved monitor index is used,
  as in earlier versions. Unversioned, v1, and v2 files are still read and are
  migrated to v3 on the next save; a migrated coordinate is checked against the
  live monitor layout and discarded if it no longer lands on a display.
- Added a direct `winit` dependency (pinned to Bevy 0.19's version) to read
  platform-specific monitor identification data.

### Fixed
- A window with no saved position restoring onto a lower-density monitor was
  sized from the scale of the monitor it started on, so it returned at the
  wrong size and the restore never settled. Present in 0.1.1.
- macOS: a fullscreen restore waited without limit for a monitor move that
  winit never resolved, which could leave the window hidden.

### Verification
- Automated Bevy tests exercise the platform-specific recovery logic for
  macOS, Windows, X11, and Wayland. They cover surviving and deleted windows,
  application-controlled and automatic recovery, operation with no displays or
  no windows, cancellation, and platform capability limits.
- Real monitor disconnects still depend on operating-system behavior that a
  simulated test cannot prove. The example README contains one earlier macOS
  disconnect/reconnect record; the complete two-cycle macOS, Windows, X11, and
  Wayland results have not yet been recorded. Reconstructed windows may also
  return in a different front-to-back order because Clerestory does not
  preserve stacking order.

## [0.1.1] - 2026-07-02

### Fixed
- Fix macOS automatic window tabbing merging same-app fullscreen windows into one tab group (blacking out the vacated monitor): `WindowManagerPlugin` now sets the app-wide `NSWindow.allowsAutomaticWindowTabbing = false` at plugin build, before any OS window exists

## 0.1.0 — Initial release

`hana_clerestory` (formerly published as `bevy_window_manager`).

- Primary-window position/size persistence across launches.
- Multi-monitor support with scale-factor-correct positioning (mixed
  Retina / non-Retina setups).
- Correct placement when dragging across monitors with different scale factors.
- Platform workarounds: macOS, Windows, Linux X11 and Wayland.
- `Monitors` resource, `MonitorInfo`, `CurrentMonitor`, `ManagedWindow`,
  `ManagedWindowPersistence`, `WindowKey`, `Platform`,
  `WindowRestored` / `WindowRestoreMismatch` events.
- `WindowManagerPlugin` with `with_app_name` / `with_path` / `with_persistence`
  builders.
