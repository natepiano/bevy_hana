//! Temporary linked shader files owned by one test.

use std::io::Error;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::SystemTimeError;
use std::time::UNIX_EPOCH;

/// An error that stopped creation of a shader test directory.
#[derive(Debug, thiserror::Error)]
pub enum ShaderTestDirectoryError {
    /// The system clock is earlier than the Unix epoch.
    #[error("{0}")]
    Clock(#[from] SystemTimeError),
    /// The directory could not be created.
    #[error("{source}")]
    Create {
        /// Path of the directory to create.
        path:   PathBuf,
        /// Error from the file system.
        #[source]
        source: Error,
    },
}

/// Owns a temporary directory for shader test files.
pub struct ShaderTestDirectory(
    /// The path to the temporary directory.
    pub PathBuf,
);

impl ShaderTestDirectory {
    /// Creates a temporary directory for one test.
    ///
    /// # Errors
    /// Returns an error if the clock or directory creation fails.
    pub fn new() -> Result<Self, ShaderTestDirectoryError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hana-shader-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&path).map_err(|source| ShaderTestDirectoryError::Create {
            path: path.clone(),
            source,
        })?;
        Ok(Self(path))
    }
}

impl Drop for ShaderTestDirectory {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(
            removed.is_ok(),
            "remove this test's own shader directory: {removed:?}"
        );
    }
}
