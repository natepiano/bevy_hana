#[cfg(all(unix, not(target_os = "macos")))]
use std::time::Duration;

#[cfg(target_os = "macos")]
pub(super) const CORE_GRAPHICS_SUCCESS: i32 = 0;
#[cfg(target_os = "macos")]
pub(super) const MACOS_BEGIN_CONFIGURATION_FLAG: u32 = 1;
#[cfg(target_os = "windows")]
pub(super) const WINDOWS_NOTIFICATION_CLASS_PREFIX: &str = "BevyClerestoryMonitorConfiguration";
#[cfg(any(test, target_os = "windows"))]
pub(super) const WINDOWS_NOTIFICATION_WAIT_MILLISECONDS: u32 = 25;
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) const X11_NOTIFICATION_POLL_INTERVAL: Duration = Duration::from_millis(25);
