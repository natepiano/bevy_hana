// persistence paths
pub(super) const EXAMPLES_DIRECTORY_NAME: &str = "examples";
pub(super) const RON_EXTENSION: &str = ".ron";
/// Extension for the temporary file that a state write lands in before being renamed over the
/// real one.
pub(super) const STATE_TEMPORARY_EXTENSION: &str = "ron.tmp";

// unreadable state file backups
/// Label used in place of a version number when the file is too damaged to probe.
pub(super) const BACKUP_CORRUPT_LABEL: &str = "corrupt";
/// Exclusive upper bound on the numeric suffix appended to a backup name: suffixes 1 through 99
/// are searched, and past those no unused name is produced.
pub(super) const BACKUP_MAX_ATTEMPTS: u32 = 100;
/// Suffix inserted before the version label: `windows.ron.bak.v2`.
pub(super) const BACKUP_SUFFIX: &str = "bak";

// state versions
pub(super) const PERSISTED_STATE_VERSION_V1: u8 = 1;
pub(super) const PERSISTED_STATE_VERSION_V2: u8 = 2;
pub(super) const PERSISTED_STATE_VERSION_V3: u8 = 3;
pub(super) const PERSISTED_STATE_VERSION_V4: u8 = 4;
pub(super) const PERSISTED_STATE_VERSION_V5: u8 = 5;
