use std::io::Error;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::mpsc::Sender;
#[cfg(test)]
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;
#[cfg(test)]
use std::time::Duration;

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

    /// Block until the worker publishes a delivery, then take it.
    #[cfg(test)]
    pub(super) fn await_message(&self) -> DiskWorkerMessage { self.slot.take_blocking() }

    /// Take a delivery published within `quiet`, reporting `None` when the worker stayed silent.
    #[cfg(test)]
    pub(super) fn take_message_within(&self, quiet: Duration) -> Option<DiskWorkerMessage> {
        self.slot.take_within(quiet)
    }

    /// Block until the worker's own activity satisfies `reached`.
    ///
    /// This is how a test waits for the worker to arm a watch, notice a change, or attempt a
    /// read: the worker announces each of those, so the wait ends when it did the thing rather
    /// than when a chosen interval expired.
    #[cfg(test)]
    pub(super) fn wait_for_activity(&self, reached: impl FnMut(&WorkerActivity) -> bool) {
        self.status.wait_until(reached);
    }

    /// Read the worker's activity as it stands, for an assertion about an exact count.
    #[cfg(test)]
    pub(super) fn activity<T>(&self, of: impl FnOnce(&WorkerActivity) -> T) -> T {
        self.status.read(of)
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
    slot: Arc<CoalescingSlotState>,
}

/// The single retained delivery, and the signal that one has arrived.
///
/// The condvar is what a test waits on. Sampling the slot on a timer instead would let a
/// machine that is merely busy read as a worker that published nothing, which is a
/// different fact entirely.
struct CoalescingSlotState {
    message:   Mutex<Option<DiskWorkerMessage>>,
    published: Condvar,
}

impl CoalescingSlot {
    pub(super) fn new() -> Self {
        Self {
            slot: Arc::new(CoalescingSlotState {
                message:   Mutex::new(None),
                published: Condvar::new(),
            }),
        }
    }

    pub(super) fn publish(&self, mut message: DiskWorkerMessage) {
        let mut message_slot = self.message_slot();

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
        drop(message_slot);
        self.slot.published.notify_all();
    }

    pub(super) fn take(&self) -> Option<DiskWorkerMessage> {
        Some(Self::retained(self.message_slot().take()?))
    }

    /// Block until the worker has published a delivery, then take it.
    ///
    /// This is the arrival's own signal, so returning from it means the worker published and
    /// never means the machine was quick enough. A worker that publishes nothing parks the
    /// caller here; nextest's `slow-timeout` in `.config/nextest.toml` is what ends that.
    #[cfg(test)]
    pub(super) fn take_blocking(&self) -> DiskWorkerMessage {
        let mut message_slot = self.message_slot();

        loop {
            if let Some(message) = message_slot.take() {
                return Self::retained(message);
            }

            message_slot = self
                .slot
                .published
                .wait(message_slot)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Block until the worker publishes, or report that `quiet` passed without one.
    ///
    /// The only negative assertion in the worker's tests, and the one place a duration decides
    /// an outcome: absence over an interval cannot be established any other way. It is safe
    /// where a deadline is not, because a slow machine makes this wait quieter rather than
    /// louder -- the failure it reports is a delivery that happened, never one that was late.
    #[cfg(test)]
    pub(super) fn take_within(&self, quiet: Duration) -> Option<DiskWorkerMessage> {
        let (mut message_slot, _) = self
            .slot
            .published
            .wait_timeout_while(self.message_slot(), quiet, |message_slot| {
                message_slot.is_none()
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        Some(Self::retained(message_slot.take()?))
    }

    /// Append the count of diagnostics the slot dropped while coalescing, when it dropped any.
    fn retained(mut message: DiskWorkerMessage) -> DiskWorkerMessage {
        if message.discarded_diagnostics > 0 {
            message.diagnostics.push(discarded_diagnostics_diagnostic(
                message.discarded_diagnostics,
            ));
        }

        message
    }

    fn message_slot(&self) -> MutexGuard<'_, Option<DiskWorkerMessage>> {
        self.slot
            .message
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// What the worker thread has done so far, and the signal that it has done more.
///
/// A test waits on the condvar below rather than sampling these fields on a timer. Sampling
/// decides a failure from how busy the machine is; waking from the condvar means the worker
/// itself moved. The lock is taken only when the worker arms or drops a watch and once per
/// read attempt, so it never sits on a hot path.
#[derive(Default)]
pub(super) struct WorkerStatus {
    activity: Mutex<WorkerActivity>,
    changed:  Condvar,
}

/// The worker activity a [`WorkerStatus`] retains.
#[derive(Default)]
pub(super) struct WorkerActivity {
    /// Whether a watcher is currently armed on the keymap directory.
    pub(super) watching:               bool,
    #[cfg(test)]
    pub(super) dirty_notifications:    usize,
    #[cfg(test)]
    pub(super) not_found_observations: usize,
    #[cfg(test)]
    pub(super) read_attempts:          usize,
}

impl WorkerStatus {
    pub(super) fn set_watching(&self, watching: bool) {
        self.record(|activity| activity.watching = watching);
    }

    #[cfg(test)]
    pub(super) fn record_dirty_notification(&self) {
        self.record(|activity| activity.dirty_notifications += 1);
    }

    #[cfg(test)]
    pub(super) fn record_not_found_observation(&self) {
        self.record(|activity| activity.not_found_observations += 1);
    }

    #[cfg(test)]
    pub(super) fn record_read_attempt(&self) {
        self.record(|activity| activity.read_attempts += 1);
    }

    /// Block until the worker's activity satisfies `reached`.
    ///
    /// The wait has no deadline, for the reason [`CoalescingSlot::take_blocking`] gives: an
    /// activity that never arrives is a stuck worker, and nextest's `slow-timeout` owns that.
    #[cfg(test)]
    pub(super) fn wait_until(&self, mut reached: impl FnMut(&WorkerActivity) -> bool) {
        let mut activity = self.lock_activity();

        while !reached(&activity) {
            activity = self
                .changed
                .wait(activity)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }

        drop(activity);
    }

    #[cfg(test)]
    pub(super) fn read<T>(&self, of: impl FnOnce(&WorkerActivity) -> T) -> T {
        of(&self.lock_activity())
    }

    fn record(&self, change: impl FnOnce(&mut WorkerActivity)) {
        let mut activity = self.lock_activity();
        change(&mut activity);
        drop(activity);
        self.changed.notify_all();
    }

    fn lock_activity(&self) -> MutexGuard<'_, WorkerActivity> {
        self.activity
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
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

/// Whether a delivered snapshot carries `expected_contents`, where `None` stands for the
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
