//! Restore configuration.

use std::path::PathBuf;

use bevy::prelude::Resource;

/// Configuration for the `RestoreWindowPlugin`.
#[derive(Resource, Clone)]
pub(crate) struct RestoreWindowConfig {
    /// Full path to the state file.
    pub(crate) path: PathBuf,
}
