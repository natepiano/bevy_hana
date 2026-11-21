//! Dedicated filesystem worker that reads the user keymap and delivers each complete state to
//! the application thread.

use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use notify::RecommendedWatcher;

use super::channels;
use super::channels::CoalescingSlot;
use super::channels::DiskDelivery;
use super::channels::DiskSnapshot;
use super::channels::DiskWorkerChannels;
use super::channels::DiskWorkerMessage;
use super::channels::WorkerControl;
use super::channels::WorkerStatus;
use super::watch::WatchMode;
use super::watch::WatchNotifications;
use crate::Diagnostic;
use crate::disk::companion_files;
use crate::disk::constants::DEBOUNCE_INTERVAL;
use crate::disk::constants::POLL_INTERVAL;
use crate::disk::constants::RETRY_INTERVAL;
use crate::disk::paths::KeymapPaths;
use crate::keymap::UserKeymapContents;

pub(super) const USER_KEYMAP_STUB: &[u8] = br#"{
  "$schema": "./keymap.schema.json",
  "bindings": []
}"#;

/// Starts the dedicated disk worker for one configured application.
pub(crate) fn start_disk_worker(
    paths: &KeymapPaths,
    default_keymap: Vec<u8>,
    schema: Option<Vec<u8>>,
) -> DiskWorkerChannels {
    start_disk_worker_with(
        paths,
        default_keymap,
        schema,
        WorkerTimings::production(),
        WatchMode::Native,
        WorkerClock::System,
    )
}

pub(super) fn start_disk_worker_with(
    paths: &KeymapPaths,
    default_keymap: Vec<u8>,
    schema: Option<Vec<u8>>,
    worker_timings: WorkerTimings,
    watch_mode: WatchMode,
    clock: WorkerClock,
) -> DiskWorkerChannels {
    let slot = CoalescingSlot::new();
    let (control_sender, control_receiver) = mpsc::channel();
    let status = Arc::new(WorkerStatus::default());
    #[cfg(test)]
    let (first_read_release, first_read_hold) = mpsc::sync_channel(1);
    #[cfg(test)]
    let injected_notifications = watch_mode
        .is_injected()
        .then(|| Arc::new(Mutex::new(WatchNotifications::default())));
    let next_poll_at = worker_timings.poll.map(|poll| clock.now() + poll);
    let disk_worker = DiskWorker {
        paths: paths.clone(),
        default_keymap,
        schema,
        slot: slot.clone(),
        control_receiver,
        status: Arc::clone(&status),
        worker_timings,
        watch_mode,
        clock,
        watcher: None,
        #[cfg(test)]
        watcher_notifications: injected_notifications.clone(),
        #[cfg(not(test))]
        watcher_notifications: None,
        watcher_receiver: None,
        #[cfg(test)]
        first_read_hold,
        observed_keymap: ObservedKeymap::Unknown,
        dirty_at: None,
        missing_retry_at: None,
        next_poll_at,
        parent_exists: false,
        reported_read_error: None,
        reported_watch_error: None,
    };
    let join_handle = thread::spawn(move || disk_worker.run());

    DiskWorkerChannels {
        slot,
        control_sender,
        join_handle: Some(join_handle),
        status,
        #[cfg(test)]
        injected_notifications,
        #[cfg(test)]
        first_read_release,
    }
}

/// Builds a worker whose thread never starts, so a test can drive one method directly, paired
/// with the [`CoalescingSlot`] its diagnostics publish into.
#[cfg(all(test, unix))]
pub(super) fn watch_test_worker(paths: &KeymapPaths) -> (DiskWorker, CoalescingSlot) {
    let slot = CoalescingSlot::new();
    let (_, control_receiver) = mpsc::channel();
    let (_, first_read_hold) = mpsc::sync_channel(1);
    let disk_worker = DiskWorker {
        paths: paths.clone(),
        default_keymap: Vec::new(),
        schema: None,
        slot: slot.clone(),
        control_receiver,
        status: Arc::new(WorkerStatus::default()),
        worker_timings: WorkerTimings {
            debounce: DEBOUNCE_INTERVAL,
            poll:     None,
            retry:    RETRY_INTERVAL,
        },
        watch_mode: WatchMode::Native,
        clock: WorkerClock::System,
        watcher: None,
        watcher_notifications: None,
        watcher_receiver: None,
        first_read_hold,
        observed_keymap: ObservedKeymap::Unknown,
        dirty_at: None,
        missing_retry_at: None,
        next_poll_at: None,
        parent_exists: false,
        reported_read_error: None,
        reported_watch_error: None,
    };

    (disk_worker, slot)
}

pub(super) struct DiskWorker {
    paths:                            KeymapPaths,
    default_keymap:                   Vec<u8>,
    schema:                           Option<Vec<u8>>,
    slot:                             CoalescingSlot,
    control_receiver:                 Receiver<WorkerControl>,
    pub(super) status:                Arc<WorkerStatus>,
    pub(super) worker_timings:        WorkerTimings,
    pub(super) watch_mode:            WatchMode,
    pub(super) clock:                 WorkerClock,
    pub(super) watcher:               Option<RecommendedWatcher>,
    pub(super) watcher_notifications: Option<Arc<Mutex<WatchNotifications>>>,
    pub(super) watcher_receiver:      Option<Receiver<()>>,
    #[cfg(test)]
    first_read_hold:                  Receiver<()>,
    observed_keymap:                  ObservedKeymap,
    pub(super) dirty_at:              Option<Instant>,
    missing_retry_at:                 Option<Instant>,
    next_poll_at:                     Option<Instant>,
    pub(super) parent_exists:         bool,
    reported_read_error:              Option<String>,
    pub(super) reported_watch_error:  Option<String>,
}

impl DiskWorker {
    fn run(mut self) {
        let paths = self.paths.clone();
        self.report_diagnostics(companion_files::publish_companion_files(
            &paths,
            &self.default_keymap,
            self.schema.as_deref(),
        ));
        self.create_user_stub(&paths);
        self.parent_exists = paths.config_directory().is_dir();

        // The watcher is armed before the first read. A native watch only reports changes made
        // after it starts, so reading first would lose any edit that landed between that read and
        // `recreate_watcher` — recoverable only on the next `WorkerTimings::poll` audit, seconds
        // later. Arming first makes every edit either visible to this read or reported as an event.
        self.recreate_watcher(&paths);
        #[cfg(test)]
        self.hold_first_read();
        self.read_user_keymap(&paths, self.clock.now());

        match self.clock {
            WorkerClock::System => self.run_on_system_clock(&paths),
            #[cfg(test)]
            WorkerClock::Manual(_) => self.run_on_manual_clock(&paths),
        }

        self.status.set_watching(false);
    }

    fn run_on_system_clock(&mut self, paths: &KeymapPaths) {
        while self.process_control() {
            let now = self.clock.now();
            self.process_due_work(paths, now);

            let timeout = self.next_deadline(now).min(self.worker_timings.retry);

            match self.watcher_receiver.as_ref() {
                Some(watcher_receiver) => match watcher_receiver.recv_timeout(timeout) {
                    Ok(()) => self.handle_watcher_notifications(paths),
                    Err(RecvTimeoutError::Timeout) => {},
                    Err(RecvTimeoutError::Disconnected) => self.handle_watcher_failure(
                        paths,
                        String::from("The keymap watcher notification channel disconnected."),
                    ),
                },
                None => match self.control_receiver.recv_timeout(timeout) {
                    Ok(WorkerControl::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {},
                    #[cfg(test)]
                    Ok(WorkerControl::WatcherNotification | WorkerControl::AdvanceClock { .. }) => {
                    },
                },
            }
        }
    }

    #[cfg(test)]
    fn run_on_manual_clock(&mut self, paths: &KeymapPaths) {
        while let Ok(control) = self.control_receiver.recv() {
            match control {
                WorkerControl::Stop => break,
                WorkerControl::WatcherNotification => self.handle_watcher_notifications(paths),
                WorkerControl::AdvanceClock { elapsed, processed } => {
                    self.clock = WorkerClock::Manual(self.clock.now() + elapsed);
                    self.process_due_work(paths, self.clock.now());
                    let _ = processed.send(());
                },
            }
        }
    }

    /// Parks the worker between [`Self::recreate_watcher`] arming the watch and
    /// [`Self::read_user_keymap`] performing the first read, so a test can write into that window.
    #[cfg(test)]
    fn hold_first_read(&self) {
        if self.watch_mode.holds_first_read() {
            let _ = self.first_read_hold.recv();
        }
    }

    fn process_control(&self) -> bool {
        match self.control_receiver.try_recv() {
            Ok(WorkerControl::Stop) | Err(TryRecvError::Disconnected) => false,
            Err(TryRecvError::Empty) => true,
            #[cfg(test)]
            Ok(WorkerControl::WatcherNotification | WorkerControl::AdvanceClock { .. }) => true,
        }
    }

    fn process_due_work(&mut self, paths: &KeymapPaths, now: Instant) {
        let dirty_due = self.dirty_at.is_some_and(|dirty_at| dirty_at <= now);
        let retry_due = self
            .missing_retry_at
            .is_some_and(|missing_retry_at| missing_retry_at <= now);

        if dirty_due {
            self.dirty_at = None;
            self.read_user_keymap(paths, now);
        } else if retry_due {
            self.read_user_keymap(paths, now);
        }

        if self
            .next_poll_at
            .is_some_and(|next_poll_at| next_poll_at <= now)
        {
            self.next_poll_at = self.worker_timings.poll.map(|poll| now + poll);
            self.audit(paths, now);
        }
    }

    fn next_deadline(&self, now: Instant) -> Duration {
        [self.dirty_at, self.missing_retry_at, self.next_poll_at]
            .into_iter()
            .flatten()
            .map(|deadline| deadline.saturating_duration_since(now))
            .min()
            .unwrap_or(self.worker_timings.retry)
    }

    fn create_user_stub(&self, paths: &KeymapPaths) {
        if let Err(error) = fs::create_dir_all(paths.config_directory()) {
            self.report_diagnostics(vec![channels::disk_error_diagnostic(
                paths.config_directory(),
                "Could not create the keymap configuration directory",
                &error,
            )]);
            return;
        }

        let user_keymap = paths.user_keymap();
        let stub_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(user_keymap);

        match stub_file {
            Ok(mut stub_file) => {
                if let Err(error) = stub_file.write_all(USER_KEYMAP_STUB) {
                    self.report_diagnostics(vec![channels::disk_error_diagnostic(
                        user_keymap,
                        "Could not write the first-launch keymap stub",
                        &error,
                    )]);
                }
            },
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {},
            Err(error) => self.report_diagnostics(vec![channels::disk_error_diagnostic(
                user_keymap,
                "Could not create the first-launch keymap stub",
                &error,
            )]),
        }
    }

    pub(super) fn read_user_keymap(&mut self, paths: &KeymapPaths, now: Instant) {
        #[cfg(test)]
        self.status.record_read_attempt();

        match fs::read(paths.user_keymap()) {
            Ok(contents) => self.observe_contents(paths, Arc::from(contents)),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.observe_absence(paths, now);
                #[cfg(test)]
                self.status.record_not_found_observation();
            },
            Err(error) => {
                let message = error.to_string();

                if self.reported_read_error.as_deref() != Some(message.as_str()) {
                    self.report_diagnostics(vec![channels::disk_error_diagnostic(
                        paths.user_keymap(),
                        "Could not read the user keymap",
                        &error,
                    )]);
                    self.reported_read_error = Some(message);
                }

                self.missing_retry_at = Some(now + self.worker_timings.retry);
            },
        }
    }

    fn observe_contents(&mut self, paths: &KeymapPaths, contents: Arc<[u8]>) {
        let changed = !matches!(
            &self.observed_keymap,
            ObservedKeymap::Present(previous) if previous.as_ref() == contents.as_ref()
        );

        self.observed_keymap = ObservedKeymap::Present(Arc::clone(&contents));
        self.missing_retry_at = None;
        self.reported_read_error = None;

        if changed {
            self.publish_snapshot(paths, UserKeymapContents::Read(contents));
        }
    }

    fn observe_absence(&mut self, paths: &KeymapPaths, now: Instant) {
        if matches!(self.observed_keymap, ObservedKeymap::Missing) {
            return;
        }

        let Some(missing_retry_at) = self.missing_retry_at else {
            self.missing_retry_at = Some(now + self.worker_timings.retry);
            return;
        };

        if now < missing_retry_at {
            return;
        }

        if self.dirty_at.is_some() {
            self.missing_retry_at = Some(now + self.worker_timings.retry);
            return;
        }

        self.observed_keymap = ObservedKeymap::Missing;
        self.missing_retry_at = None;
        self.reported_read_error = None;
        self.publish_snapshot(paths, UserKeymapContents::Absent);
    }

    fn publish_snapshot(&self, paths: &KeymapPaths, contents: UserKeymapContents) {
        self.slot.publish(DiskWorkerMessage {
            delivery:              DiskDelivery::Snapshot(DiskSnapshot {
                source_path: paths.user_keymap().to_path_buf(),
                contents,
            }),
            diagnostics:           Vec::new(),
            discarded_diagnostics: 0,
        });
    }

    pub(super) fn report_diagnostics(&self, diagnostics: Vec<Diagnostic>) {
        if diagnostics.is_empty() {
            return;
        }

        self.slot.publish(DiskWorkerMessage {
            delivery: DiskDelivery::DiagnosticsOnly,
            diagnostics,
            discarded_diagnostics: 0,
        });
    }
}

pub(super) enum WorkerClock {
    System,
    #[cfg(test)]
    Manual(Instant),
}

impl WorkerClock {
    #[cfg(test)]
    pub(super) fn manual() -> Self { Self::Manual(Instant::now()) }

    pub(super) fn now(&self) -> Instant {
        match self {
            Self::System => Instant::now(),
            #[cfg(test)]
            Self::Manual(now) => *now,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct WorkerTimings {
    pub(super) debounce: Duration,
    pub(super) poll:     Option<Duration>,
    pub(super) retry:    Duration,
}

impl WorkerTimings {
    pub(super) const fn production() -> Self {
        Self {
            debounce: DEBOUNCE_INTERVAL,
            poll:     Some(POLL_INTERVAL),
            retry:    RETRY_INTERVAL,
        }
    }
}

enum ObservedKeymap {
    Unknown,
    Present(Arc<[u8]>),
    Missing,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests stop when their isolated disk-worker setup fails"
)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::DiskDelivery;
    use super::DiskWorkerChannels;
    use super::DiskWorkerMessage;
    use super::USER_KEYMAP_STUB;
    use super::UserKeymapContents;
    use super::WatchMode;
    use super::WorkerClock;
    use super::WorkerTimings;
    use super::start_disk_worker_with;
    use crate::disk::KeymapPathAvailability;
    use crate::disk::KeymapPaths;
    use crate::disk::paths::ENVIRONMENT_LOCK;
    use crate::disk::paths::TestDirectory;
    use crate::disk::paths::XdgConfigHome;
    use crate::disk::worker::channels;

    const DEFAULT_KEYMAP: &[u8] = b"{\"bindings\": []}";
    const DEBOUNCE_INTERVAL: Duration = Duration::from_millis(100);
    const ERROR_RETRY_WINDOWS: u32 = 6;
    const POLL_AUDIT_INTERVAL: Duration = Duration::from_millis(20);
    const QUIESCENCE_INTERVAL: Duration = Duration::from_millis(180);
    const RETRY_ASSERTION_MARGIN: Duration = RETRY_GRACE_INTERVAL
        .checked_div(2)
        .expect("halving a duration is always representable");
    const RETRY_BOUNDARY_STEP: Duration = Duration::from_millis(1);
    const RETRY_GRACE_INTERVAL: Duration = Duration::from_millis(1500);
    const RETRY_INTERVAL: Duration = Duration::from_millis(30);
    const TEST_APP_NAME: &str = "hana-rubric-worker-test";

    fn worker_timings() -> WorkerTimings {
        WorkerTimings {
            debounce: DEBOUNCE_INTERVAL,
            poll:     Some(POLL_AUDIT_INTERVAL),
            retry:    RETRY_INTERVAL,
        }
    }

    fn worker_timings_without_polling() -> WorkerTimings {
        WorkerTimings {
            debounce: DEBOUNCE_INTERVAL,
            poll:     None,
            retry:    RETRY_INTERVAL,
        }
    }

    fn worker_timings_for_retry_before_debounce() -> WorkerTimings {
        WorkerTimings {
            debounce: RETRY_INTERVAL.saturating_mul(2),
            poll:     None,
            retry:    RETRY_INTERVAL,
        }
    }

    fn worker_timings_with_retry_grace() -> WorkerTimings {
        WorkerTimings {
            debounce: DEBOUNCE_INTERVAL,
            poll:     None,
            retry:    RETRY_GRACE_INTERVAL,
        }
    }

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

    fn start_worker(watch_mode: WatchMode) -> DiskWorkerChannels {
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test keymap paths resolve");
        start_disk_worker_with(
            &paths,
            DEFAULT_KEYMAP.to_vec(),
            Some(b"{}".to_vec()),
            worker_timings(),
            watch_mode,
            WorkerClock::System,
        )
    }

    fn start_injected_worker(worker_timings: WorkerTimings) -> DiskWorkerChannels {
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test keymap paths resolve");
        start_disk_worker_with(
            &paths,
            DEFAULT_KEYMAP.to_vec(),
            Some(b"{}".to_vec()),
            worker_timings,
            WatchMode::Injected,
            WorkerClock::manual(),
        )
    }

    fn start_injected_worker_holding_first_read(
        worker_timings: WorkerTimings,
    ) -> DiskWorkerChannels {
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test keymap paths resolve");
        start_disk_worker_with(
            &paths,
            DEFAULT_KEYMAP.to_vec(),
            Some(b"{}".to_vec()),
            worker_timings,
            WatchMode::InjectedHoldingFirstRead,
            WorkerClock::manual(),
        )
    }

    /// Block until the worker publishes its next delivery.
    ///
    /// The worker's own publication ends this wait, so a slower machine changes how long the
    /// test takes and never whether it passes. A worker that publishes nothing parks here for
    /// nextest's `slow-timeout` to end, which is the right report: nothing arrived, rather than
    /// nothing arrived quickly enough.
    fn wait_for_message(worker: &DiskWorkerChannels) -> DiskWorkerMessage { worker.await_message() }

    fn wait_for_watcher(worker: &DiskWorkerChannels, expected: bool) {
        worker.wait_for_activity(|activity| activity.watching == expected);
    }

    fn expect_no_message_for(worker: &DiskWorkerChannels, quiet: Duration) -> Result<(), String> {
        worker.advance_clock(quiet)?;
        if worker.take_message().is_some() {
            return Err(String::from(
                "disk worker published unexpectedly during the quiet interval",
            ));
        }

        Ok(())
    }

    fn expect_no_confirmed_deletion(worker: &DiskWorkerChannels) -> Result<(), String> {
        if matches!(
            worker.take_message(),
            Some(DiskWorkerMessage {
                delivery: DiskDelivery::Snapshot(snapshot),
                ..
            }) if matches!(snapshot.contents, UserKeymapContents::Absent)
        ) {
            return Err(String::from(
                "disk worker published confirmed deletion before the retry grace expired",
            ));
        }

        Ok(())
    }

    fn wait_for_dirty_notifications(worker: &DiskWorkerChannels, expected: usize) {
        worker.wait_for_activity(|activity| activity.dirty_notifications >= expected);
    }

    fn wait_for_not_found_observations(worker: &DiskWorkerChannels, expected: usize) {
        worker.wait_for_activity(|activity| activity.not_found_observations >= expected);
    }

    #[test]
    fn watcher_edges_produce_one_debounced_read() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("rename-save").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let worker_timings = worker_timings_for_retry_before_debounce();
        let mut worker = start_injected_worker(worker_timings);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        wait_for_watcher(&worker, true);
        channels::assert_snapshot(
            &wait_for_message(&worker),
            paths.user_keymap(),
            Some(USER_KEYMAP_STUB),
        )?;
        let initial_read_attempts = worker.activity(|activity| activity.read_attempts);

        let saved_contents = b"{\"bindings\": [{\"bindings\": {}}]}";
        fs::write(paths.user_keymap(), saved_contents)
            .map_err(|error| format!("saved keymap write failed: {error}"))?;
        worker.inject_watcher_dirty()?;
        wait_for_dirty_notifications(&worker, 1);
        worker.advance_clock(worker_timings.retry)?;
        if worker.activity(|activity| activity.read_attempts) != initial_read_attempts {
            return Err(String::from(
                "the first watcher edge read before the final debounced edge",
            ));
        }

        worker.inject_watcher_dirty()?;
        wait_for_dirty_notifications(&worker, 2);
        worker.advance_clock(
            worker_timings
                .debounce
                .checked_sub(worker_timings.retry)
                .expect("the debounce interval must be at least the retry interval"),
        )?;
        if worker.activity(|activity| activity.read_attempts) != initial_read_attempts {
            return Err(String::from(
                "the final watcher edge read before its debounce interval expired",
            ));
        }

        worker.advance_clock(worker_timings.retry)?;
        worker.expect_snapshot(paths.user_keymap(), Some(saved_contents))?;
        if worker.activity(|activity| activity.read_attempts) != initial_read_attempts + 1 {
            return Err(String::from(
                "the debounced watcher edges performed more than one read",
            ));
        }
        expect_no_message_for(&worker, QUIESCENCE_INTERVAL)?;

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn an_edit_landing_before_the_first_read_is_observed_without_a_poll_audit() -> Result<(), String>
    {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("first-read-window").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let mut worker = start_injected_worker_holding_first_read(worker_timings_without_polling());

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        // The watch is armed before the first read, so this resolves while the worker is still
        // parked ahead of that read. Reading first would leave it unarmed until after the read.
        wait_for_watcher(&worker, true);

        let edited_contents = b"{\"bindings\": [{\"bindings\": {}}]}";
        fs::write(paths.user_keymap(), edited_contents)
            .map_err(|error| format!("first-read-window keymap write failed: {error}"))?;
        worker.release_first_read()?;

        // Polling is off and no watcher edge is injected, so only the first read can carry this
        // edit to the application thread.
        channels::assert_snapshot(
            &wait_for_message(&worker),
            paths.user_keymap(),
            Some(edited_contents),
        )?;

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn retry_before_debounce_does_not_confirm_absence() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("retry-before-debounce").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let worker_timings = worker_timings_without_polling();
        let mut worker = start_injected_worker(worker_timings);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        channels::assert_snapshot(
            &wait_for_message(&worker),
            paths.user_keymap(),
            Some(USER_KEYMAP_STUB),
        )?;

        fs::remove_file(paths.user_keymap())
            .map_err(|error| format!("user keymap removal failed: {error}"))?;
        worker.inject_watcher_dirty()?;
        wait_for_dirty_notifications(&worker, 1);
        worker.inject_watcher_failure()?;
        wait_for_not_found_observations(&worker, 1);
        expect_no_confirmed_deletion(&worker)?;

        let mut read_attempts = worker.activity(|activity| activity.read_attempts);
        let mut until_debounce = worker_timings.debounce;
        while until_debounce > worker_timings.retry {
            worker.advance_clock(worker_timings.retry)?;
            read_attempts += 1;
            assert_eq!(
                worker.activity(|activity| activity.read_attempts),
                read_attempts,
                "the worker must retry the read once while the dirty edge is pending",
            );
            expect_no_confirmed_deletion(&worker)?;
            until_debounce -= worker_timings.retry;
        }

        let restored_contents = b"{\"bindings\": [{\"bindings\": {}}]}";
        fs::write(paths.user_keymap(), restored_contents)
            .map_err(|error| format!("restored keymap write failed: {error}"))?;
        worker.advance_clock(until_debounce)?;
        worker.expect_snapshot(paths.user_keymap(), Some(restored_contents))?;
        expect_no_message_for(&worker, QUIESCENCE_INTERVAL)?;

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn transient_rename_absence_does_not_publish_deletion() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("transient-absence").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let worker_timings = worker_timings_with_retry_grace();
        let mut worker = start_injected_worker(worker_timings);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        channels::assert_snapshot(
            &wait_for_message(&worker),
            paths.user_keymap(),
            Some(USER_KEYMAP_STUB),
        )?;

        let temporary_keymap = paths.config_directory().join("keymap.jsonc.tmp");
        fs::rename(paths.user_keymap(), &temporary_keymap)
            .map_err(|error| format!("temporary keymap removal failed: {error}"))?;
        worker.inject_watcher_failure()?;
        wait_for_not_found_observations(&worker, 1);
        worker.advance_clock(
            worker_timings
                .retry
                .checked_sub(RETRY_ASSERTION_MARGIN)
                .expect("the retry interval must be at least RETRY_ASSERTION_MARGIN"),
        )?;
        expect_no_confirmed_deletion(&worker)?;
        fs::rename(&temporary_keymap, paths.user_keymap())
            .map_err(|error| format!("temporary keymap restoration failed: {error}"))?;

        let read_attempts_before_restore = worker.activity(|activity| activity.read_attempts);
        worker.advance_clock(worker_timings.retry + RETRY_ASSERTION_MARGIN)?;
        assert_eq!(
            worker.activity(|activity| activity.read_attempts),
            read_attempts_before_restore + 1,
            "the worker must retry the read once after the file returns",
        );
        expect_no_confirmed_deletion(&worker)?;

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn confirmed_absence_publishes_only_after_retry_grace() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("confirmed-absence").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let worker_timings = worker_timings_with_retry_grace();
        let mut worker = start_injected_worker(worker_timings);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        channels::assert_snapshot(
            &wait_for_message(&worker),
            paths.user_keymap(),
            Some(USER_KEYMAP_STUB),
        )?;

        fs::remove_file(paths.user_keymap())
            .map_err(|error| format!("user keymap removal failed: {error}"))?;
        worker.inject_watcher_failure()?;
        wait_for_not_found_observations(&worker, 1);
        worker.advance_clock(
            worker_timings
                .retry
                .checked_sub(RETRY_ASSERTION_MARGIN)
                .expect("the retry interval must be at least RETRY_ASSERTION_MARGIN"),
        )?;
        expect_no_confirmed_deletion(&worker)?;

        worker.advance_clock(RETRY_ASSERTION_MARGIN)?;
        worker.expect_snapshot(paths.user_keymap(), None)?;
        expect_no_message_for(&worker, QUIESCENCE_INTERVAL)?;

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn read_errors_retry_at_a_bounded_rate() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("read-errors").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let worker_timings = worker_timings_without_polling();
        let mut worker = start_injected_worker(worker_timings);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        channels::assert_snapshot(
            &wait_for_message(&worker),
            paths.user_keymap(),
            Some(USER_KEYMAP_STUB),
        )?;
        fs::remove_file(paths.user_keymap())
            .map_err(|error| format!("user keymap removal failed: {error}"))?;
        fs::create_dir(paths.user_keymap())
            .map_err(|error| format!("user keymap directory creation failed: {error}"))?;

        let read_attempts_before_error = worker.activity(|activity| activity.read_attempts);
        worker.inject_watcher_failure()?;
        worker.advance_clock(Duration::ZERO)?;
        let mut expected_read_attempts = read_attempts_before_error + 1;
        assert_eq!(
            worker.activity(|activity| activity.read_attempts),
            expected_read_attempts,
            "the watcher failure must cause one read",
        );

        for window in 1..=ERROR_RETRY_WINDOWS {
            worker.advance_clock(
                worker_timings
                    .retry
                    .checked_sub(RETRY_BOUNDARY_STEP)
                    .expect("the retry interval must be at least RETRY_BOUNDARY_STEP"),
            )?;
            assert_eq!(
                worker.activity(|activity| activity.read_attempts),
                expected_read_attempts,
                "the worker retried before the deadline in window {window}",
            );

            worker.advance_clock(RETRY_BOUNDARY_STEP)?;
            expected_read_attempts += 1;
            assert_eq!(
                worker.activity(|activity| activity.read_attempts),
                expected_read_attempts,
                "the worker must retry once at the deadline in window {window}",
            );
        }

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn stub_is_created_once_and_never_overwritten() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory = TestDirectory::new("stub").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let mut first_worker = start_worker(WatchMode::Native);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        let _ = wait_for_message(&first_worker);
        if fs::read(paths.user_keymap())
            .map_err(|error| format!("first-launch stub read failed: {error}"))?
            != USER_KEYMAP_STUB
        {
            return Err(String::from(
                "first-launch stub bytes differ from the required document",
            ));
        }
        first_worker.shutdown();

        let custom_contents = b"{\"bindings\": [{\"bindings\": {}}]}";
        fs::write(paths.user_keymap(), custom_contents)
            .map_err(|error| format!("custom keymap write failed: {error}"))?;
        let mut second_worker = start_worker(WatchMode::Native);

        channels::assert_snapshot(
            &wait_for_message(&second_worker),
            paths.user_keymap(),
            Some(custom_contents),
        )?;
        if fs::read(paths.user_keymap())
            .map_err(|error| format!("second-launch keymap read failed: {error}"))?
            != custom_contents
        {
            return Err(String::from(
                "second launch overwrote the existing user keymap",
            ));
        }

        second_worker.shutdown();

        fs::write(paths.user_keymap(), [])
            .map_err(|error| format!("empty keymap write failed: {error}"))?;
        let mut third_worker = start_worker(WatchMode::Native);
        channels::assert_snapshot(
            &wait_for_message(&third_worker),
            paths.user_keymap(),
            Some(&[]),
        )?;
        if !fs::read(paths.user_keymap())
            .map_err(|error| format!("empty keymap read failed: {error}"))?
            .is_empty()
        {
            return Err(String::from(
                "third launch overwrote the existing empty user keymap",
            ));
        }
        third_worker.shutdown();

        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }
}
