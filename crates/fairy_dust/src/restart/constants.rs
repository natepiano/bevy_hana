//! Constants for the `restart` module.

// cargo command
pub(super) const CARGO_BIN: &str = "cargo";
pub(super) const CARGO_EXAMPLES_DIR: &str = "examples";
pub(super) const CARGO_EXAMPLE_FLAG: &str = "--example";
pub(super) const CARGO_MANIFEST_PATH_FLAG: &str = "--manifest-path";
pub(super) const CARGO_RELEASE_FLAG: &str = "--release";
pub(super) const CARGO_RUN_SUBCOMMAND: &str = "run";

// exit status
pub(super) const EXIT_STATUS_FAILURE: i32 = 1;
#[cfg(not(unix))]
pub(super) const EXIT_STATUS_SUCCESS: i32 = 0;
