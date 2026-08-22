use std::io::Error;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
#[cfg(test)]
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;

#[cfg(test)]
use super::watch::TestWatcher;
use crate::Diagnostic;
use crate::DiagnosticKind;
use crate::DiagnosticOrigin;
use crate::DiagnosticSeverity;
use crate::disk::constants::MAX_RETAINED_DIAGNOSTICS;
use crate::keymap::UserKeymapContents;

/// A complete user-keymap state produced by the disk worker.
pub(crate) struct DiskSnapshot {
    /// User-keymap path associated with this state.
    pub(crate) source_path: PathBuf,
    /// Current user-keymap bytes, or their confirmed absence.
    pub(crate) contents:    UserKeymapContents,
}

/// What one disk-worker delivery carries beyond its diagnostics.
pub(crate) enum DiskDelivery {
    /// The worker read a complete user-keymap state, which supersedes the live keymap.
    Snapshot(DiskSnapshot),
    /// The worker observed no new user-keymap state, so the delivery reports diagnostics alone.
    DiagnosticsOnly,
}

/// One coalesced disk-worker delivery to the application thread.
pub(crate) struct DiskWorkerMessage {
    /// Latest complete user-keymap state, when the worker read one.
    pub(crate) delivery:              DiskDelivery,
    /// Disk and companion diagnostics accumulated before this delivery.
    pub(crate) diagnostics:           Vec<Diagnostic>,
    pub(super) discarded_diagnostics: usize,
}

/// Application-side handles for a running disk worker.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "plugin assembly will retain the disk worker channels through application shutdown"
    )
)]
pub(crate) struct DiskWorkerChannels {
    pub(super) slot:               CoalescingSlot,
    pub(super) control_sender:     Sender<WorkerControl>,
    pub(super) join_handle:        Option<JoinHandle<()>>,
    pub(super) status:             Arc<WorkerStatus>,
    #[cfg(test)]
    pub(super) test_watcher:       Option<TestWatcher>,
    /// Releases a worker started with `WatchMode::InjectedHoldingFirstRead` from the window
    /// between arming its watcher and its first read.
    #[cfg(test)]
    pub(super) first_read_release: SyncSender<()>,
}

impl DiskWorkerChannels {
    /// Takes the newest worker message, dropping no newer state.
    pub(super) fn take_message(&self) -> Option<DiskWorkerMessage> { self.slot.take() }

    #[cfg(test)]
    pub(super) fn is_watching(&self) -> bool { self.status.watching.load(Ordering::Acquire) }

    #[cfg(test)]
    pub(super) fn dirty_notifications(&self) -> usize {
        self.status.dirty_notifications.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn not_found_observations(&self) -> usize {
        self.status.not_found_observations.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn read_attempts(&self) -> usize {
        self.status.read_attempts.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn inject_watcher_dirty(&self) -> Result<(), String> {
        self.inject_watcher_notification(None, true)
    }

    #[cfg(test)]
    pub(super) fn inject_watcher_failure(&self) -> Result<(), String> {
        self.inject_watcher_notification(
            Some(String::from(
                "The injected keymap watcher requested a complete rescan.",
            )),
            false,
        )
    }

    #[cfg(test)]
    fn inject_watcher_notification(
        &self,
        health_message: Option<String>,
        dirty: bool,
    ) -> Result<(), String> {
        let test_watcher = self
            .test_watcher
            .as_ref()
            .ok_or_else(|| String::from("worker was not started with an injected watcher"))?;
        let mut watcher_notifications = test_watcher
            .watcher_notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        watcher_notifications.health_message = health_message;
        watcher_notifications.dirty |= dirty;
        drop(watcher_notifications);

        let _ = test_watcher.watcher_sender.try_send(());
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn release_first_read(&self) -> Result<(), String> {
        self.first_read_release
            .try_send(())
            .map_err(|error| format!("the disk worker's first read was not held: {error}"))
    }

    #[cfg(test)]
    pub(super) fn shutdown(&mut self) { self.shutdown_inner(); }

    fn shutdown_inner(&mut self) {
        let _ = self.control_sender.send(WorkerControl::Stop);

        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl Drop for DiskWorkerChannels {
    fn drop(&mut self) { self.shutdown_inner(); }
}

#[derive(Clone)]
pub(super) struct CoalescingSlot {
    message: Arc<Mutex<Option<DiskWorkerMessage>>>,
}

impl CoalescingSlot {
    pub(super) fn new() -> Self {
        Self {
            message: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn publish(&self, mut message: DiskWorkerMessage) {
        let mut message_slot = self
            .message
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Some(previous) = message_slot.take() {
            if matches!(message.delivery, DiskDelivery::DiagnosticsOnly) {
                message.delivery = previous.delivery;
            }

            message.diagnostics.splice(0..0, previous.diagnostics);
            message.discarded_diagnostics += previous.discarded_diagnostics;
        }

        let discarded_diagnostics = message
            .diagnostics
            .len()
            .saturating_sub(MAX_RETAINED_DIAGNOSTICS);
        if discarded_diagnostics > 0 {
            message.diagnostics.drain(0..discarded_diagnostics);
            message.discarded_diagnostics += discarded_diagnostics;
        }

        *message_slot = Some(message);
    }

    pub(super) fn take(&self) -> Option<DiskWorkerMessage> {
        let mut message = self
            .message
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()?;

        if message.discarded_diagnostics > 0 {
            message.diagnostics.push(discarded_diagnostics_diagnostic(
                message.discarded_diagnostics,
            ));
        }

        Some(message)
    }
}

#[derive(Default)]
pub(super) struct WorkerStatus {
    pub(super) watching:               AtomicBool,
    #[cfg(test)]
    pub(super) dirty_notifications:    AtomicUsize,
    #[cfg(test)]
    pub(super) not_found_observations: AtomicUsize,
    #[cfg(test)]
    pub(super) read_attempts:          AtomicUsize,
}

pub(super) enum WorkerControl {
    Stop,
}

pub(super) fn disk_error_diagnostic(path: &Path, action: &str, error: &Error) -> Diagnostic {
    disk_diagnostic(
        DiagnosticOrigin::KeymapFile(path.to_path_buf()),
        &format!("{action}: {error}"),
    )
}

pub(super) fn disk_diagnostic(origin: DiagnosticOrigin, message: &str) -> Diagnostic {
    Diagnostic {
        origin,
        byte_range: 0..0,
        line: 0,
        column: 0,
        block_index: 0,
        context: String::new(),
        original_keystroke: String::new(),
        command_id: String::new(),
        kind: DiagnosticKind::Disk,
        severity: DiagnosticSeverity::Failure,
        message: message.to_owned(),
        suggestions: Vec::new(),
    }
}

fn discarded_diagnostics_diagnostic(discarded_diagnostics: usize) -> Diagnostic {
    disk_diagnostic(
        DiagnosticOrigin::DiskWorker,
        &format!("{discarded_diagnostics} older disk diagnostics were discarded before delivery."),
    )
}

/// Whether a delivered snapshot carries the bytes a test expects, where `None` expects the
/// confirmed absence of the user keymap file.
#[cfg(test)]
pub(super) fn contents_match(
    contents: &UserKeymapContents,
    expected_contents: Option<&[u8]>,
) -> bool {
    match (contents, expected_contents) {
        (UserKeymapContents::Read(contents), Some(expected)) => contents.as_ref() == expected,
        (UserKeymapContents::Absent, None) => true,
        (UserKeymapContents::Read(_) | UserKeymapContents::Absent, _) => false,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests stop when their isolated disk-worker setup fails"
)]
mod tests {
    use std::sync::Arc;

    use super::CoalescingSlot;
    use super::DiskDelivery;
    use super::DiskSnapshot;
    use super::DiskWorkerMessage;
    use super::UserKeymapContents;
    use super::disk_diagnostic;
    use crate::DiagnosticOrigin;
    use crate::disk::KeymapPathAvailability;
    use crate::disk::KeymapPaths;
    use crate::disk::constants::MAX_RETAINED_DIAGNOSTICS;
    use crate::disk::paths::ENVIRONMENT_LOCK;
    use crate::disk::paths::TestDirectory;
    use crate::disk::paths::XdgConfigHome;
    use crate::disk::worker::runtime::WorkerTimings;

    const TEST_APP_NAME: &str = "hana-rubric-channel-test";
    const SNAPSHOT_BURST_COUNT: usize = 1000;

    fn isolated_paths(temporary_directory: &TestDirectory) -> Result<KeymapPaths, String> {
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .map_err(|keymap_path_failure| {
                format!("test keymap paths should resolve: {keymap_path_failure:?}")
            })?;

        if !paths
            .config_directory()
            .starts_with(temporary_directory.path())
        {
            return Err(String::from(
                "test keymap path escaped the temporary directory",
            ));
        }

        Ok(paths)
    }

    #[test]
    fn coalescing_slot_caps_retained_diagnostics() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("coalescing-diagnostics").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let slot = CoalescingSlot::new();
        let diagnostic_count = MAX_RETAINED_DIAGNOSTICS.saturating_mul(2);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        for index in 0..diagnostic_count {
            slot.publish(DiskWorkerMessage {
                delivery:              DiskDelivery::DiagnosticsOnly,
                diagnostics:           vec![disk_diagnostic(
                    DiagnosticOrigin::KeymapFile(paths.user_keymap().to_path_buf()),
                    &format!("distinct disk diagnostic {index}"),
                )],
                discarded_diagnostics: 0,
            });
        }

        let message = slot
            .take()
            .ok_or_else(|| String::from("coalescing slot has no diagnostics"))?;
        if message.diagnostics.len() > MAX_RETAINED_DIAGNOSTICS + 1 {
            return Err(String::from(
                "coalescing slot retained more diagnostics than its configured cap",
            ));
        }
        let truncation_diagnostic = message
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("older disk diagnostics were discarded")
            })
            .ok_or_else(|| String::from("coalescing slot omitted its truncation diagnostic"))?;
        let expected_truncation_message = format!(
            "{MAX_RETAINED_DIAGNOSTICS} older disk diagnostics were discarded before delivery."
        );
        if truncation_diagnostic.message != expected_truncation_message {
            return Err(String::from(
                "coalescing slot reported an incorrect discarded diagnostic count",
            ));
        }
        let newest_diagnostic_message = format!(
            "distinct disk diagnostic {}",
            diagnostic_count.saturating_sub(1)
        );
        if !message
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == newest_diagnostic_message)
        {
            return Err(String::from(
                "coalescing slot did not retain the newest diagnostic",
            ));
        }

        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn coalescing_slot_retains_only_the_newest_snapshot() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("coalescing-slot").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let slot = CoalescingSlot::new();

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        let production_timings = WorkerTimings::production();
        assert_eq!(
            production_timings.debounce,
            super::super::super::constants::DEBOUNCE_INTERVAL
        );
        assert_eq!(
            production_timings.poll,
            Some(super::super::super::constants::POLL_INTERVAL)
        );
        assert_eq!(
            production_timings.retry,
            super::super::super::constants::RETRY_INTERVAL
        );

        for index in 0..SNAPSHOT_BURST_COUNT {
            slot.publish(DiskWorkerMessage {
                delivery:              DiskDelivery::Snapshot(DiskSnapshot {
                    source_path: paths.user_keymap().to_path_buf(),
                    contents:    UserKeymapContents::Read(Arc::from(
                        index.to_string().into_bytes(),
                    )),
                }),
                diagnostics:           Vec::new(),
                discarded_diagnostics: 0,
            });
        }

        let message = slot
            .take()
            .ok_or_else(|| String::from("coalescing slot has no newest snapshot"))?;
        let DiskDelivery::Snapshot(snapshot) = message.delivery else {
            return Err(String::from("coalescing slot has no snapshot"));
        };
        let newest_snapshot = SNAPSHOT_BURST_COUNT.saturating_sub(1).to_string();
        if snapshot.source_path != paths.user_keymap()
            || !matches!(
                &snapshot.contents,
                UserKeymapContents::Read(contents)
                    if contents.as_ref() == newest_snapshot.as_bytes()
            )
        {
            return Err(String::from(
                "coalescing slot did not retain the newest snapshot",
            ));
        }
        if slot.take().is_some() {
            return Err(String::from(
                "coalescing slot retained more than one message",
            ));
        }

        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }
}
