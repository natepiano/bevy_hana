use std::time::Duration;

// diagnostics
/// Bounds memory retained when the application does not drain disk diagnostics.
pub(crate) const MAX_RETAINED_DIAGNOSTICS: usize = 64;

// filenames
pub(super) const DEFAULT_KEYMAP_FILE_NAME: &str = "keymap.default.jsonc";
pub(super) const SCHEMA_FILE_NAME: &str = "keymap.schema.json";
pub(super) const USER_KEYMAP_FILE_NAME: &str = "keymap.jsonc";

// permissions
pub(super) const COMPANION_FILE_MODE: u32 = 0o444;

// timings
pub(super) const DEBOUNCE_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const POLL_INTERVAL: Duration = Duration::from_secs(5);
pub(super) const RETRY_INTERVAL: Duration = Duration::from_millis(50);

pub(super) const TEMPORARY_FILE_ATTEMPTS: usize = 32;
pub(super) const XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
