//! Cross-module constants.

use std::time::Duration;

// fullscreen restore
/// Maximum duration (in seconds) a macOS fullscreen restore spends in
/// `FullscreenRestoreState::MoveWindowedToTarget` waiting for the windowed window to arrive on
/// the target monitor.
///
/// The move is a request, not a guarantee: winit drops a `WindowPosition::Centered` whose
/// `MonitorSelection::Index` it cannot resolve, and the compositor is free to leave the window
/// where it is. Without a deadline the state re-requests the move every frame while the window
/// stays hidden. On expiry the restore applies the fullscreen mode anyway — winit resolves the
/// monitor index itself at that point — so the window becomes visible instead of waiting forever.
pub(crate) const FULLSCREEN_MONITOR_MOVE_TIMEOUT_SECS: f32 = SETTLE_TIMEOUT_SECS;

// managed window names
/// First numeric suffix appended to deduplicate a managed window name (e.g. `name-2`).
pub(crate) const FIRST_DUPLICATE_SUFFIX: u32 = 2;
pub(crate) const MANAGED_WINDOW_NAME_SEPARATOR: &str = "-";

// monitor discovery
#[cfg(any(test, feature = "test"))]
pub(crate) const DISPLAY_TEST_ENUMERATION_FAILURE: &str =
    "the display test adapter failed the scripted enumeration";
/// Longest the rigging kernel waits before re-enumerating displays when no operating-system
/// display-configuration notification has arrived.
///
/// The notification is the normal trigger; this only catches one that never arrived, such as a
/// replug delivered while the application was suspended. A second is short enough that a missed
/// unplug does not outlive a user's attention and long enough that an idle application does not
/// re-enumerate every frame.
pub(crate) const MONITOR_DISCOVERY_BACKSTOP: Duration = Duration::from_secs(1);

// monitor identity
/// Directory the kernel populates with one `card<N>-<connector>` entry per DRM connector.
#[cfg(target_os = "linux")]
pub(crate) const DRM_CLASS_DIRECTORY: &str = "/sys/class/drm";
/// File inside a DRM connector directory holding that display's raw EDID block.
#[cfg(target_os = "linux")]
pub(crate) const DRM_CONNECTOR_EDID_FILE: &str = "edid";
/// Separates the card name from the connector name in a DRM connector directory name, as in
/// `card2-HDMI-A-1`.
///
/// Connector names contain further separators of their own, so only the first occurrence splits
/// the directory name.
#[cfg(target_os = "linux")]
pub(crate) const DRM_CONNECTOR_NAME_SEPARATOR: char = '-';
/// Hashed ahead of a connector name by `QualifiedEvidence::stable_bytes`, so a `DisplayFingerprint`
/// built from a connector can never equal one built from an EDID block.
#[cfg(any(test, target_os = "linux"))]
pub(crate) const DRM_INTERNAL_CONNECTOR_EVIDENCE_TAG: &[u8] = b"drm-internal-connector:";
/// DRM connector-name prefixes whose display is built into the machine rather than plugged into it.
///
/// `eDP` and `LVDS` carry laptop displays; `DSI` and `DPI` carry displays wired straight to the
/// board. Nothing else can ever appear on these connectors, so the connector name identifies the
/// display and not merely the port it occupies. Every other connector type is a socket that accepts
/// any display, where the same name would name a different display after a replug.
///
/// Each prefix includes [`DRM_CONNECTOR_NAME_SEPARATOR`] so that `DP-1` cannot match `DPI-`.
#[cfg(any(test, target_os = "linux"))]
pub(crate) const DRM_INTERNAL_CONNECTOR_PREFIXES: [&str; 4] = ["DPI-", "DSI-", "LVDS-", "eDP-"];
/// Shared identity scheme for serial values published in a display's EDID.
pub(crate) const EDID_SERIAL_SCHEME: &str = "edid-serial";
/// FNV-1a 64-bit offset basis, fixed by the algorithm's specification.
pub(crate) const FNV_1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime, fixed by the algorithm's specification.
pub(crate) const FNV_1A_PRIME: u64 = 0x0000_0100_0000_01b3;
/// Length of a `ColorSync` display UUID, fixed by `CFUUIDBytes`.
#[cfg(target_os = "macos")]
pub(crate) const MACOS_DISPLAY_UUID_BYTES: usize = 16;
/// `RandR` output property carrying a display's EDID block.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) const X11_EDID_PROPERTY_NAME: &[u8] = b"EDID";

// monitor lookup
pub(crate) const MONITOR_SOURCE_EXISTING: &str = "existing";
pub(crate) const MONITOR_SOURCE_FALLBACK: &str = "fallback";
pub(crate) const MONITOR_SOURCE_POSITION: &str = "position";
pub(crate) const MONITOR_SOURCE_WINIT: &str = "winit";
pub(crate) const PRIMARY_MONITOR_INDEX: usize = 0;

// monitor probe
#[cfg(feature = "monitor-probe")]
pub(crate) const MONITOR_PROBE_TARGET: &str = "hana_clerestory::monitor_probe";

// persisted state
pub(crate) const CURRENT_STATE_VERSION: u8 = 6;
pub(crate) const PRIMARY_WINDOW_KEY: &str = "primary";
/// Header comment prepended to the RON file to document the coordinate contract.
pub(crate) const RON_HEADER: &str = "\
// Sizes are in logical pixels.
// position: MonitorOffset is logical pixels from the persisted target's top-left corner.
// position: Unrebased is a pre-v3 absolute desktop coordinate awaiting a live monitor layout.
";
pub(crate) const STATE_FILE: &str = "windows.ron";

// platform detection
#[cfg(target_os = "linux")]
pub(crate) const WAYLAND_DISPLAY_ENVIRONMENT_VARIABLE: &str = "WAYLAND_DISPLAY";

// restore strategies
pub(crate) const RESTORE_STRATEGY_APPLY_UNCHANGED: &str = "ApplyUnchanged";
pub(crate) const RESTORE_STRATEGY_LOWER_TO_HIGHER: &str = "LowerToHigher";

// scale factors
/// Fallback scale factor when the monitor cannot be determined.
pub(crate) const DEFAULT_SCALE_FACTOR: f64 = 1.0;
/// Threshold for considering two scale factors equal.
///
/// Accounts for floating-point imprecision when comparing scale factors.
/// A difference less than this epsilon is considered negligible.
pub(crate) const SCALE_FACTOR_EPSILON: f64 = 0.01;

// settling
/// Maximum duration (in seconds) a cross-DPI restore waits for the scale change that moves it
/// out of `WindowRestoreState::WaitingForScaleChange`.
///
/// Neither signal that ends the wait is guaranteed. `WM_DPICHANGED` reaches a hidden window only
/// because `windows_dpi_fix` forwards it, and a target monitor that no longer matches any live
/// display is never arrived at. Without a deadline such a restore waits forever with the window
/// still hidden; on expiry it applies the final size and reveals the window instead, so settling
/// can report the mismatch.
pub(crate) const SCALE_CHANGE_WAIT_TIMEOUT_SECS: f32 = SETTLE_TIMEOUT_SECS;
/// Duration (in seconds) that all values must remain stable before declaring success.
pub(crate) const SETTLE_STABILITY_SECS: f32 = 0.2;
/// Maximum total duration (in seconds) to wait for values to stabilize.
pub(crate) const SETTLE_TIMEOUT_SECS: f32 = 2.0;
// time conversion
pub(crate) const MILLIS_PER_SECOND: f32 = 1000.0;

// window reveal
/// Maximum duration (in seconds) each hidden managed window's `SavedDisplayRevealWait` waits for
/// a live display that can satisfy its saved target before revealing that window on the display
/// where it launched.
///
/// A saved display that is not plugged in never resolves, so the kernel issues no restore attempt
/// and the startup hide is never lifted. Without a deadline the window stays hidden for the whole
/// session. On expiry the saved geometry is fitted onto the live display and the window is
/// revealed, leaving the persisted target untouched so a normal restore can still move it once
/// that display returns.
pub(crate) const EXACT_DISPLAY_WAIT_TIMEOUT_SECS: f32 = SETTLE_TIMEOUT_SECS;

/// How often (in seconds) a window still hidden past [`EXACT_DISPLAY_WAIT_TIMEOUT_SECS`] restates
/// why it is waiting.
///
/// Two placement decisions keep waiting after that deadline expires: the kernel is mid-apply, and
/// a saved configuration whose display is live so placement is still expected to arrive. Neither
/// is bounded, so a placement that never arrives leaves the window hidden for the whole session
/// with nothing written to say so. The first overdue frame reports immediately and later ones
/// report on this cadence, which keeps a permanent wait visible without writing a line per frame.

// windows display-config acquisition
#[cfg(target_os = "windows")]
pub(crate) const DISPLAY_CONFIG_ACQUISITION_ATTEMPTS: usize = 3;

// windows dpi interception
#[cfg(all(target_os = "windows", feature = "workaround-winit-4341"))]
pub(crate) const DPI_CHANGE_HANDLED_RESULT: isize = 0;
/// Win32 subclass identifier for DPI-change interception.
#[cfg(all(target_os = "windows", feature = "workaround-winit-4341"))]
pub(crate) const SUBCLASS_ID: usize = 1;
#[cfg(all(target_os = "windows", feature = "workaround-winit-4341"))]
pub(crate) const SUBCLASS_REFERENCE_DATA: usize = 0;

// x11 frame extents
/// Number of values in `_NET_FRAME_EXTENTS` (left, right, top, bottom).
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
pub(crate) const FRAME_EXTENT_COUNT: u32 = 4;
/// X11 property offset used when querying `_NET_FRAME_EXTENTS`.
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
pub(crate) const FRAME_EXTENT_PROPERTY_OFFSET: u32 = 0;
/// Index of the "top" extent in `_NET_FRAME_EXTENTS`.
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
pub(crate) const FRAME_EXTENT_TOP_INDEX: usize = 2;
#[cfg(all(target_os = "linux", feature = "workaround-winit-4445"))]
pub(crate) const FRAME_EXTENTS_ATOM_NAME: &[u8] = b"_NET_FRAME_EXTENTS";
