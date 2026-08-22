use bevy::prelude::Color;
use bevy::prelude::Val;

// colors
pub(crate) const DEFAULT_COLOR: Color = Color::WHITE;
pub(crate) const MISMATCH_COLOR: Color = Color::linear_rgb(1.0, 0.3, 0.3);
pub(crate) const MISMATCH_WARN_COLOR: Color = Color::linear_rgb(1.0, 0.7, 0.2);

// comparison layout
pub(crate) const COMPARISON_COLUMN_PADDING: usize = 2;
pub(crate) const FONT_SIZE: f32 = 14.0;
pub(crate) const LABEL_WIDTH: usize = 22;
pub(crate) const MARGIN: Val = Val::Px(20.0);
pub(crate) const MIN_COMPARISON_COLUMN_WIDTH: usize = 16;

// display text
pub(crate) const ACTIVE_VIDEO_MODE_SUFFIX: &str = " <- active";
pub(crate) const ACTUAL_COLUMN_TITLE: &str = "Actual";
pub(crate) const AUTOMATIC_TEXT: &str = "Automatic";
pub(crate) const CURRENT_COLUMN_TITLE: &str = "Current";
pub(crate) const EFFECTIVE_MODE_LABEL: &str = "Effective Mode:";
pub(crate) const EXPECTED_COLUMN_TITLE: &str = "Expected";
pub(crate) const MANAGED_WINDOWS_HEADER: &str = "\nManaged Windows:\n";
pub(crate) const MODE_LABEL: &str = "Mode:";
pub(crate) const MONITOR_ID_LABEL: &str = "Monitor ID   :";
pub(crate) const MONITOR_INDEX_LABEL: &str = "Monitor Index:";
pub(crate) const NO_MANAGED_WINDOWS_TEXT: &str = "  (none)\n";
pub(crate) const NO_RESTORE_DATA_TEXT: &str = "State: No restore data\n\n";
pub(crate) const NO_VIDEO_MODES_TEXT: &str = "  (no video modes available)";
pub(crate) const NON_PRIMARY_MONITOR_MARKER: &str = " -";
pub(crate) const NONE_TEXT: &str = "None";
pub(crate) const NOT_AVAILABLE_TEXT: &str = "N/A";
pub(crate) const POSITION_LOGICAL_LABEL: &str = "Position (logical):";
pub(crate) const POSITION_PHYSICAL_LABEL: &str = "Position (physical):";
pub(crate) const PRIMARY_MONITOR_MARKER: &str = " Primary Monitor -";
pub(crate) const REFRESH_RATE_LABEL: &str = "Refresh Rate:";
pub(crate) const RESTORED_COLUMN_TITLE: &str = "Restored";
pub(crate) const SCALE_LABEL: &str = "Scale:";
pub(crate) const SECONDARY_WINDOW_CONTROLS: &str = "\nControls:\n\
                 [Enter] Exclusive Fullscreen\n\
                 [B] Borderless Fullscreen\n\
                 [W] Windowed\n\
                 [Space] Spawn managed window\n\
                 [P] Toggle persistence\n\
                 [Ctrl+Shift+Backspace] Clear state and quit\n\
                 [Q] Quit\n";
pub(crate) const SECONDARY_WINDOW_NAME_LABEL: &str = "Window:";
pub(crate) const SELECTED_VIDEO_MODE_MARKER: &str = ">";
pub(crate) const SIZE_LOGICAL_LABEL: &str = "Size (logical):";
pub(crate) const SIZE_PHYSICAL_LABEL: &str = "Size (physical):";
pub(crate) const UNKNOWN_MANAGED_WINDOW_NAME: &str = "unknown";
pub(crate) const UNSELECTED_VIDEO_MODE_MARKER: &str = " ";
pub(crate) const VIDEO_MODES_HEADER: &str = "\nVideo Modes (Up/Down to select):\n";
#[cfg(target_os = "linux")]
pub(crate) const WAYLAND_PLATFORM_SUFFIX: &str = " (Wayland)";
#[cfg(target_os = "linux")]
pub(crate) const X11_PLATFORM_SUFFIX: &str = " (X11)";

// managed windows
pub(crate) const MANAGED_WINDOW_NAME_PREFIX: &str = "window-";
pub(crate) const MANAGED_WINDOW_TITLE_PREFIX: &str = "Managed: ";
pub(crate) const SECONDARY_WINDOW_HEIGHT: u32 = 400;
pub(crate) const SECONDARY_WINDOW_WIDTH: u32 = 600;

// monitor selection
pub(crate) const PRIMARY_MONITOR_INDEX: usize = 0;

// persistence
pub(crate) const STATE_FILE: &str = "windows.ron";

// primary window
pub(crate) const PRIMARY_WINDOW_TITLE: &str = "Window Restore - Primary Window";

// refresh rates
pub(crate) const MILLIHERTZ_PER_HERTZ: u32 = 1000;

// test mode
pub(crate) const TEST_HTTP_PORT_ENVIRONMENT_VARIABLE: &str = "BRP_EXTRAS_PORT";
pub(crate) const TEST_LAUNCH_MONITOR_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_TEST_LAUNCH_MONITOR";
pub(crate) const TEST_LAUNCH_POSITION_ENVIRONMENT_VARIABLE: &str =
    "CLERESTORY_TEST_LAUNCH_POSITION";
/// Physical `width,height` for the initial window, used by the cross-DPI cases so the launch
/// window fits the (possibly small, software-rendered) monitor it is born on before the restore
/// relocates it. Absent for every other case, which keeps Bevy's default window size.
pub(crate) const TEST_LAUNCH_SIZE_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_TEST_LAUNCH_SIZE";
pub(crate) const TEST_MODE_ENVIRONMENT_VARIABLE: &str = "CLERESTORY_TEST_MODE";
pub(crate) const TEST_PERSISTENCE_PATH_ENVIRONMENT_VARIABLE: &str =
    "CLERESTORY_TEST_PERSISTENCE_PATH";

// remote methods
pub(crate) const TEST_SHUTDOWN_METHOD: &str = "clerestory/shutdown";
pub(crate) const TEST_MONITOR_SNAPSHOT_METHOD: &str = "clerestory/monitor_snapshot";
pub(crate) const TEST_WINDOW_SNAPSHOT_METHOD: &str = "clerestory/window_snapshot";

// video mode scrolling
/// When the selected video mode falls below the visible window, scroll down so
/// the new selection lands at row 2 of the 5-row window (i.e.
/// `selected - BACKWARD_SCROLL_OFFSET`). Mathematically:
/// `selected + FORWARD_SCROLL_OFFSET - VISIBLE_VIDEO_MODE_COUNT
///  = selected - BACKWARD_SCROLL_OFFSET`.
pub(crate) const BACKWARD_SCROLL_OFFSET: usize = 2;
pub(crate) const DEFAULT_VIDEO_MODE_INDEX: usize = 0;
/// Forward-scroll constant for the video-mode list. See `BACKWARD_SCROLL_OFFSET`.
pub(crate) const FORWARD_SCROLL_OFFSET: usize = 3;
/// Rows of context shown above the active video mode when centering the
/// 5-row visible window on it.
pub(crate) const VIDEO_MODE_CENTER_PADDING: usize = 2;
pub(crate) const VISIBLE_VIDEO_MODE_COUNT: usize = 5;
