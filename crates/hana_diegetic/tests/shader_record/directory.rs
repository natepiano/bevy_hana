//! Temporary shader evidence owned by one test.

use std::time::UNIX_EPOCH;

pub(super) struct RecordDirectory(pub(super) std::path::PathBuf);

impl RecordDirectory {
    pub(super) fn new() -> Result<Self, String> {
        use std::sync::atomic::AtomicU64;
        use std::sync::atomic::Ordering;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hana-shader-record-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }

    pub(super) fn read(&self) -> Result<serde_json::Value, String> {
        let bytes =
            std::fs::read(self.0.join("variants.json")).map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }
}

impl Drop for RecordDirectory {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(
            removed.is_ok(),
            "remove this test's own record directory: {removed:?}"
        );
    }
}
