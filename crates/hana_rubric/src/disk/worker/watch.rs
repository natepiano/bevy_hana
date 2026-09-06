use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
#[cfg(test)]
use std::sync::mpsc::SyncSender;
use std::time::Instant;

use notify::Event;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::Watcher;

use super::channels;
use super::runtime::DiskWorker;
use crate::DiagnosticOrigin;
use crate::disk::paths::KeymapPaths;

#[derive(Default)]
pub(super) struct WatchNotifications {
    pub(super) dirty:          bool,
    pub(super) health_message: Option<String>,
}

#[cfg(test)]
pub(super) struct TestWatcher {
    pub(super) watcher_notifications: Arc<Mutex<WatchNotifications>>,
    pub(super) watcher_sender:        SyncSender<()>,
}

#[derive(Clone, Copy)]
pub(super) enum WatchMode {
    Native,
    #[cfg(test)]
    PollOnly,
    #[cfg(test)]
    Injected,
    /// [`Self::Injected`], with the worker parked between arming its watcher and its first read
    /// until `DiskWorkerChannels::release_first_read` lets it go.
    #[cfg(test)]
    InjectedHoldingFirstRead,
}

impl WatchMode {
    pub(super) const fn is_native(self) -> bool {
        match self {
            Self::Native => true,
            #[cfg(test)]
            Self::PollOnly => false,
            #[cfg(test)]
            Self::Injected | Self::InjectedHoldingFirstRead => false,
        }
    }

    #[cfg(test)]
    pub(super) const fn is_injected(self) -> bool {
        matches!(self, Self::Injected | Self::InjectedHoldingFirstRead)
    }

    #[cfg(test)]
    pub(super) const fn holds_first_read(self) -> bool {
        matches!(self, Self::InjectedHoldingFirstRead)
    }

    #[cfg(not(test))]
    pub(super) const fn is_injected(self) -> bool {
        match self {
            Self::Native => false,
        }
    }
}

impl DiskWorker {
    pub(super) fn handle_watcher_notifications(&mut self, paths: &KeymapPaths) {
        let Some(watcher_notifications) = &self.watcher_notifications else {
            return;
        };
        let mut watcher_notifications = watcher_notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let health_message = watcher_notifications.health_message.take();
        let dirty = std::mem::take(&mut watcher_notifications.dirty);
        drop(watcher_notifications);

        if let Some(health_message) = health_message {
            self.handle_watcher_failure(paths, health_message);
        } else if dirty {
            self.dirty_at = Some(Instant::now() + self.worker_timings.debounce);
            #[cfg(test)]
            self.status
                .dirty_notifications
                .fetch_add(1, Ordering::Release);
        }
    }

    pub(super) fn handle_watcher_failure(&mut self, paths: &KeymapPaths, message: String) {
        if self.reported_watch_error.as_deref() != Some(message.as_str()) {
            self.report_diagnostics(vec![channels::disk_diagnostic(
                DiagnosticOrigin::KeymapDirectory(paths.config_directory().to_path_buf()),
                &message,
            )]);
            self.reported_watch_error = Some(message);
        }

        self.read_user_keymap(paths, Instant::now());
        self.recreate_watcher(paths);
    }

    pub(super) fn audit(&mut self, paths: &KeymapPaths, now: Instant) {
        let parent_exists = paths.config_directory().is_dir();

        if self.parent_exists != parent_exists {
            self.parent_exists = parent_exists;
            self.handle_watcher_failure(
                paths,
                String::from(
                    "The keymap configuration directory changed and its watch was recreated.",
                ),
            );
        }

        self.read_user_keymap(paths, now);

        if self.watcher.is_none() && parent_exists {
            self.recreate_watcher(paths);
        }
    }

    pub(super) fn recreate_watcher(&mut self, paths: &KeymapPaths) {
        self.watcher = None;
        self.status.watching.store(false, Ordering::Release);

        if self.watch_mode.is_injected() {
            if paths.config_directory().is_dir() {
                self.status.watching.store(true, Ordering::Release);
            } else {
                self.report_watcher_setup_failure(
                    paths.config_directory(),
                    String::from("The keymap configuration directory is unavailable for watching."),
                );
            }
            return;
        }

        self.watcher_notifications = None;
        self.watcher_receiver = None;

        if !self.watch_mode.is_native() {
            return;
        }

        if !paths.config_directory().is_dir() {
            self.report_watcher_setup_failure(
                paths.config_directory(),
                String::from("The keymap configuration directory is unavailable for watching."),
            );
            return;
        }

        let user_keymap_watch = resolve_user_keymap_watch(paths);
        let callback_keymap_watch = user_keymap_watch.clone();
        let watch_notifications = Arc::new(Mutex::new(WatchNotifications::default()));
        let callback_notifications = Arc::clone(&watch_notifications);
        let (watcher_sender, watcher_receiver) = mpsc::sync_channel(1);
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let mut notifications = callback_notifications
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let notify_worker = match event {
                Ok(event) if event.need_rescan() => {
                    notifications.health_message = Some(String::from(
                        "The keymap watcher requested a complete rescan.",
                    ));
                    true
                },
                Ok(event)
                    if event
                        .paths
                        .iter()
                        .any(|event_path| callback_keymap_watch.reports(event_path)) =>
                {
                    notifications.dirty = true;
                    true
                },
                Ok(_) => false,
                Err(error) => {
                    notifications.health_message =
                        Some(format!("The keymap watcher reported an error: {error}"));
                    true
                },
            };
            drop(notifications);

            if notify_worker {
                let _ = watcher_sender.try_send(());
            }
        });

        let mut watcher = match watcher {
            Ok(watcher) => watcher,
            Err(error) => {
                self.report_watcher_setup_failure(
                    paths.config_directory(),
                    format!("Could not create the keymap watcher: {error}"),
                );
                return;
            },
        };

        if let Err(error) = watcher.watch(paths.config_directory(), RecursiveMode::NonRecursive) {
            self.report_watcher_setup_failure(
                paths.config_directory(),
                format!("Could not watch the keymap configuration directory: {error}"),
            );
            return;
        }
        let symlink_parent_watch = watch_symlink_target_parent(&mut watcher, &user_keymap_watch);

        let previously_reported_watch_error = self.reported_watch_error.take();

        self.watcher = Some(watcher);
        self.watcher_notifications = Some(watch_notifications);
        self.watcher_receiver = Some(watcher_receiver);
        self.status.watching.store(true, Ordering::Release);

        if let SymlinkParentWatch::Unwatched {
            target_parent,
            message,
        } = symlink_parent_watch
        {
            // The symlink-parent failure survives every recreation, so it is compared against what
            // `report_watcher_setup_failure` last reported rather than against the cleared field.
            self.reported_watch_error = previously_reported_watch_error;
            self.report_watcher_setup_failure(&target_parent, message);
        }
    }

    fn report_watcher_setup_failure(&mut self, path: &Path, message: String) {
        if self.reported_watch_error.as_deref() != Some(message.as_str()) {
            self.report_diagnostics(vec![channels::disk_diagnostic(
                DiagnosticOrigin::KeymapDirectory(path.to_path_buf()),
                &message,
            )]);
            self.reported_watch_error = Some(message);
        }
    }
}

/// How [`KeymapPaths::user_keymap`] sits relative to the watched configuration directory.
///
/// Each variant carries every path form [`Self::reports`] accepts as an event for that keymap
/// file, and `watch_symlink_target_parent` places a second watch only for
/// [`Self::ThroughSymlink`], reporting the outcome as a [`SymlinkParentWatch`].
#[derive(Clone)]
enum UserKeymapWatch {
    /// The configuration directory did not canonicalize, so only the configured path is known.
    UnresolvedPath { configured: PathBuf },
    /// The keymap file itself lives in the watched configuration directory.
    WithinConfigDirectory {
        configured: PathBuf,
        resolved:   PathBuf,
    },
    /// The keymap file is a symlink to a file in another directory.
    ///
    /// An editor saves by writing a temporary file and renaming it over the target, which replaces
    /// the file the watch was placed on. A watch on `target_parent` reports that rename, so edits
    /// keep arriving after the first save.
    ThroughSymlink {
        configured:     PathBuf,
        resolved:       PathBuf,
        symlink_target: PathBuf,
        target_parent:  PathBuf,
    },
}

impl UserKeymapWatch {
    fn reports(&self, event_path: &Path) -> bool {
        match self {
            Self::UnresolvedPath { configured } => event_path == configured,
            Self::WithinConfigDirectory {
                configured,
                resolved,
            } => event_path == configured || event_path == resolved,
            Self::ThroughSymlink {
                configured,
                resolved,
                symlink_target,
                ..
            } => event_path == configured || event_path == resolved || event_path == symlink_target,
        }
    }
}

/// Whether the directory holding a symlinked keymap file is watched alongside the keymap.
enum SymlinkParentWatch {
    /// The keymap file is not a symlink out of the configuration directory.
    NotNeeded,
    /// The directory holding the symlink target is watched, so a rename-over save is reported.
    Established,
    /// The directory holding the symlink target is not watched, and why.
    Unwatched {
        target_parent: PathBuf,
        message:       String,
    },
}

/// Also watches the directory a symlinked keymap file resolves into.
///
/// Skipped when the platform's recommended watcher is a polling one: a network or virtual mount
/// would then be walked on every interval, so the directory is left
/// [`SymlinkParentWatch::Unwatched`] with the reason recorded and the poll audit carries a
/// rename-over save instead.
fn watch_symlink_target_parent(
    watcher: &mut RecommendedWatcher,
    user_keymap_watch: &UserKeymapWatch,
) -> SymlinkParentWatch {
    let UserKeymapWatch::ThroughSymlink { target_parent, .. } = user_keymap_watch else {
        return SymlinkParentWatch::NotNeeded;
    };

    if matches!(
        notify::RecommendedWatcher::kind(),
        notify::WatcherKind::PollWatcher
    ) {
        return SymlinkParentWatch::Unwatched {
            target_parent: target_parent.clone(),
            message:       String::from(
                "The keymap file is a symlink and this platform watches only by polling, so a \
                 save that renames over the symlink target is carried by the poll audit instead.",
            ),
        };
    }

    match watcher.watch(target_parent, RecursiveMode::NonRecursive) {
        Ok(()) => SymlinkParentWatch::Established,
        Err(error) => SymlinkParentWatch::Unwatched {
            target_parent: target_parent.clone(),
            message:       format!(
                "Could not watch the directory holding the keymap symlink target: {error}"
            ),
        },
    }
}

/// Every path form the operating system may use when it reports an event for the user keymap.
///
/// macOS delivers `notify::Event` paths fully resolved, so an event for
/// [`KeymapPaths::user_keymap`] does not equal that path whenever the configuration directory is
/// reached through a symlink — the ordinary case under a dotfile manager, and the form `TMPDIR`
/// always takes. Accepting the resolved form as well keeps the native watch delivering an edit
/// within [`WorkerTimings::debounce`] rather than leaving it to the much later
/// [`WorkerTimings::poll`] audit.
///
/// The keymap file may also be a symlink of its own, which a dotfile manager creates when it
/// links one tracked file into an otherwise untracked directory. Resolving it costs one
/// `read_link` that fails on the ordinary case.
///
/// [`WorkerTimings::debounce`]: super::runtime::WorkerTimings::debounce
/// [`WorkerTimings::poll`]: super::runtime::WorkerTimings::poll
fn resolve_user_keymap_watch(paths: &KeymapPaths) -> UserKeymapWatch {
    let configured = paths.user_keymap().to_path_buf();
    let Some((config_directory, file_name)) = paths
        .config_directory()
        .canonicalize()
        .ok()
        .zip(paths.user_keymap().file_name())
    else {
        return UserKeymapWatch::UnresolvedPath { configured };
    };
    let resolved = config_directory.join(file_name);

    let Ok(link_target) = resolved.read_link() else {
        return UserKeymapWatch::WithinConfigDirectory {
            configured,
            resolved,
        };
    };
    let absolute_target = if link_target.is_absolute() {
        link_target
    } else {
        config_directory.join(link_target)
    };
    let symlink_target = absolute_target
        .canonicalize()
        .unwrap_or_else(|_| absolute_target.clone());
    let target_parent = symlink_target
        .parent()
        .map_or_else(|| config_directory.clone(), Path::to_path_buf);

    if target_parent == config_directory {
        return UserKeymapWatch::WithinConfigDirectory {
            configured,
            resolved,
        };
    }

    UserKeymapWatch::ThroughSymlink {
        configured,
        resolved,
        symlink_target,
        target_parent,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests stop when their isolated disk-worker setup fails"
)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::OnceLock;
    use std::sync::mpsc;
    use std::sync::mpsc::RecvTimeoutError;
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;

    use notify::Event;
    use notify::RecursiveMode;
    use notify::Watcher;

    use super::WatchMode;
    use crate::disk::KeymapPathAvailability;
    use crate::disk::KeymapPaths;
    use crate::disk::paths::ENVIRONMENT_LOCK;
    use crate::disk::paths::TestDirectory;
    use crate::disk::paths::XdgConfigHome;
    use crate::disk::worker::channels;
    use crate::disk::worker::channels::DiskDelivery;
    use crate::disk::worker::channels::DiskWorkerChannels;
    use crate::disk::worker::runtime;
    use crate::disk::worker::runtime::WorkerTimings;

    const DEFAULT_KEYMAP: &[u8] = b"{\"bindings\": []}";
    const DEBOUNCE_INTERVAL: Duration = Duration::from_millis(100);
    const NATIVE_WATCH_DEADLINE: Duration = Duration::from_millis(500);
    const POLL_AUDIT_INTERVAL: Duration = Duration::from_millis(20);
    const RETRY_INTERVAL: Duration = Duration::from_millis(30);
    const MISSING_SYMLINK_TARGET: &str = "missing-keymap-target/user-keymap.jsonc";
    const NOTIFICATION_PROBE_DEADLINE: Duration = Duration::from_secs(2);
    const NOTIFICATION_PROBE_INTERVAL: Duration = Duration::from_millis(50);
    const TEST_APP_NAME: &str = "hana-rubric-watch-test";
    const TEST_TIMEOUT: Duration = Duration::from_secs(3);
    const WAIT_INTERVAL: Duration = Duration::from_millis(5);

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

    fn start_worker(worker_timings: WorkerTimings, watch_mode: WatchMode) -> DiskWorkerChannels {
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test keymap paths resolve");
        runtime::start_disk_worker_with(
            &paths,
            DEFAULT_KEYMAP.to_vec(),
            Some(b"{}".to_vec()),
            worker_timings,
            watch_mode,
        )
    }

    fn wait_for_message(worker: &DiskWorkerChannels) -> Result<(), String> {
        let deadline = Instant::now() + TEST_TIMEOUT;

        while Instant::now() < deadline {
            if worker.take_message().is_some() {
                return Ok(());
            }
            thread::sleep(WAIT_INTERVAL);
        }

        Err(String::from(
            "disk worker did not publish a message before the test timeout",
        ))
    }

    fn wait_for_snapshot(
        worker: &DiskWorkerChannels,
        path: &Path,
        expected_contents: Option<&[u8]>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + TEST_TIMEOUT;

        while Instant::now() < deadline {
            if let Some(message) = worker.take_message()
                && matches!(
                    &message.delivery,
                    DiskDelivery::Snapshot(snapshot)
                        if snapshot.source_path == path
                            && channels::contents_match(&snapshot.contents, expected_contents)
                )
            {
                return Ok(());
            }
            thread::sleep(WAIT_INTERVAL);
        }

        Err(String::from(
            "disk worker did not publish the requested snapshot",
        ))
    }

    fn wait_for_watcher(worker: &DiskWorkerChannels, expected: bool) -> Result<(), String> {
        let deadline = Instant::now() + TEST_TIMEOUT;

        while Instant::now() < deadline {
            if worker.is_watching() == expected {
                return Ok(());
            }
            thread::sleep(WAIT_INTERVAL);
        }

        Err(format!("disk worker watch state did not become {expected}"))
    }

    fn wait_for_dirty_notifications(
        worker: &DiskWorkerChannels,
        expected: usize,
    ) -> Result<(), String> {
        let deadline = Instant::now() + TEST_TIMEOUT;

        while Instant::now() < deadline {
            if worker.dirty_notifications() >= expected {
                return Ok(());
            }
            thread::sleep(WAIT_INTERVAL);
        }

        Err(String::from(
            "disk worker did not handle the injected watcher notification",
        ))
    }

    /// A symlinked keymap file whose target directory is missing fails the same way on every
    /// watcher recreation, and `recreate_watcher` clears the reported watch error before it
    /// reports that failure. The report must still be compared against what was reported last.
    #[cfg(unix)]
    #[test]
    fn a_repeated_symlink_parent_failure_is_reported_once() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("symlink-parent-failure").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        fs::create_dir_all(paths.config_directory())
            .map_err(|error| format!("configuration directory creation failed: {error}"))?;
        std::os::unix::fs::symlink(
            temporary_directory.path().join(MISSING_SYMLINK_TARGET),
            paths.user_keymap(),
        )
        .map_err(|error| format!("user keymap symlink creation failed: {error}"))?;

        let (mut disk_worker, slot) = runtime::watch_test_worker(&paths);
        disk_worker.recreate_watcher(&paths);
        disk_worker.recreate_watcher(&paths);

        let symlink_failures = slot
            .take()
            .ok_or_else(|| {
                String::from("the symlinked keymap target did not produce a watch diagnostic")
            })?
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.contains("symlink"))
            .count();
        assert_eq!(symlink_failures, 1);

        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    #[test]
    fn recreated_parent_directory_reestablishes_the_watch() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("recreated-parent").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let worker_timings = WorkerTimings {
            debounce: DEBOUNCE_INTERVAL,
            poll:     None,
            retry:    RETRY_INTERVAL,
        };
        let mut worker = start_worker(worker_timings, WatchMode::Injected);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        wait_for_message(&worker)?;
        wait_for_watcher(&worker, true)?;

        fs::remove_dir_all(paths.config_directory())
            .map_err(|error| format!("configuration directory removal failed: {error}"))?;
        worker.inject_watcher_failure()?;
        wait_for_watcher(&worker, false)?;
        fs::create_dir_all(paths.config_directory())
            .map_err(|error| format!("configuration directory recreation failed: {error}"))?;
        worker.inject_watcher_failure()?;
        wait_for_watcher(&worker, true)?;
        wait_for_snapshot(&worker, paths.user_keymap(), None)?;
        let read_attempts_before_write = worker.read_attempts();
        fs::write(paths.user_keymap(), b"{\"bindings\": []}")
            .map_err(|error| format!("recreated user keymap write failed: {error}"))?;
        worker.inject_watcher_dirty()?;
        wait_for_dirty_notifications(&worker, 1)?;

        wait_for_snapshot(&worker, paths.user_keymap(), Some(b"{\"bindings\": []}"))?;
        if worker.read_attempts() != read_attempts_before_write + 1 {
            return Err(String::from(
                "the recreated watcher snapshot did not come from its injected watcher edge",
            ));
        }

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    /// Whether this machine's filesystem-notification service actually delivers events.
    ///
    /// [`WatchMode::Native`] is the production path and the only mode that depends on that
    /// service. Some execution environments deny it without denying the filesystem, so every
    /// native watch silently never fires while ordinary reads and writes keep working. The
    /// difference is measured rather than assumed: a platform or environment-variable check would
    /// give up this coverage on the machine that needs it most.
    enum NativeNotificationDelivery {
        /// A watcher armed on a directory reported a file written inside it.
        Delivering,
        /// Nothing was reported, so a native watch cannot be observed here.
        Silent { reason: String },
    }

    fn native_notification_delivery() -> &'static NativeNotificationDelivery {
        static DELIVERY: OnceLock<NativeNotificationDelivery> = OnceLock::new();

        DELIVERY.get_or_init(probe_native_notification_delivery)
    }

    fn probe_native_notification_delivery() -> NativeNotificationDelivery {
        let directory = match TestDirectory::new("notification-probe") {
            Ok(directory) => directory,
            Err(error) => {
                return NativeNotificationDelivery::Silent {
                    reason: format!("a directory to watch could not be created: {error}"),
                };
            },
        };
        let (sender, receiver) = mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |event: notify::Result<Event>| {
            let _ = sender.send(event);
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                return NativeNotificationDelivery::Silent {
                    reason: format!("the notification service refused a watcher: {error}"),
                };
            },
        };
        if let Err(error) = watcher.watch(directory.path(), RecursiveMode::NonRecursive) {
            return NativeNotificationDelivery::Silent {
                reason: format!("the notification service refused to watch a directory: {error}"),
            };
        }

        // The write repeats because arming a watch is not instantaneous on every backend: a single
        // write can land before the service is listening and be reported by nobody.
        let probe_file = directory.path().join("probe.jsonc");
        let deadline = Instant::now() + NOTIFICATION_PROBE_DEADLINE;
        let mut writes = 0_u32;
        while Instant::now() < deadline {
            writes += 1;
            if let Err(error) = fs::write(&probe_file, writes.to_string()) {
                return NativeNotificationDelivery::Silent {
                    reason: format!("the watched file could not be written: {error}"),
                };
            }
            match receiver.recv_timeout(NOTIFICATION_PROBE_INTERVAL) {
                Ok(Ok(_)) => return NativeNotificationDelivery::Delivering,
                Ok(Err(error)) => {
                    return NativeNotificationDelivery::Silent {
                        reason: format!("the notification service reported an error: {error}"),
                    };
                },
                Err(RecvTimeoutError::Timeout) => {},
                Err(RecvTimeoutError::Disconnected) => {
                    return NativeNotificationDelivery::Silent {
                        reason: String::from("the notification service closed its event channel"),
                    };
                },
            }
        }

        NativeNotificationDelivery::Silent {
            reason: format!(
                "the notification service accepted a watch and then reported nothing for a file written {writes} times inside the directory it was watching",
            ),
        }
    }

    /// Reports whether `test` can observe a native watch on this machine, naming the environment
    /// when it cannot so the skip is never read as a disk-worker defect.
    ///
    /// The reason is printed rather than swallowed, and `.config/nextest.toml` shows these two
    /// tests' output even when they pass, because a silent pass would be indistinguishable from
    /// real coverage.
    fn native_watch_is_observable(test: &str) -> bool {
        match native_notification_delivery() {
            NativeNotificationDelivery::Delivering => true,
            NativeNotificationDelivery::Silent { reason } => {
                eprintln!(
                    "SKIPPED {test}: this environment denies the filesystem-notification service, \
                     so a native watch cannot be observed. The disk worker was never exercised and \
                     is not implicated: {reason}",
                );
                false
            },
        }
    }

    #[test]
    fn a_native_watch_carries_an_edit_without_a_poll_audit() -> Result<(), String> {
        if !native_watch_is_observable("a_native_watch_carries_an_edit_without_a_poll_audit") {
            return Ok(());
        }

        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("native-watch").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        // The temporary directory is left in its symlinked form, not canonicalized. A configuration
        // directory reached through a symlink is the ordinary case on macOS and under a dotfile
        // manager, and it is precisely the case a path-equality watch check fails.
        let worker_timings = WorkerTimings {
            debounce: DEBOUNCE_INTERVAL,
            poll:     None,
            retry:    RETRY_INTERVAL,
        };
        let mut worker = start_worker(worker_timings, WatchMode::Native);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        // Waiting for the stub proves the first read has already happened, so nothing but the
        // native watch can carry the edit written below.
        wait_for_snapshot(
            &worker,
            paths.user_keymap(),
            Some(runtime::USER_KEYMAP_STUB),
        )?;
        wait_for_watcher(&worker, true)?;

        let edited_contents = b"{\"bindings\": [{\"bindings\": {}}]}";
        let edited_at = Instant::now();
        fs::write(paths.user_keymap(), edited_contents)
            .map_err(|error| format!("native watch keymap write failed: {error}"))?;
        wait_for_snapshot(&worker, paths.user_keymap(), Some(edited_contents))?;
        let latency = edited_at.elapsed();

        if latency > NATIVE_WATCH_DEADLINE {
            return Err(format!(
                "the native watch delivered an edit after {latency:?}, past the {NATIVE_WATCH_DEADLINE:?} deadline",
            ));
        }

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    /// A dotfile manager links one tracked file into an otherwise untracked directory, so the
    /// keymap path is a symlink to a file the configuration directory does not contain. An editor
    /// then saves by renaming a temporary file over that target, which replaces the file a
    /// file-only watch was placed on.
    #[cfg(unix)]
    #[test]
    fn a_native_watch_carries_edits_to_a_symlinked_keymap_file() -> Result<(), String> {
        if !native_watch_is_observable("a_native_watch_carries_edits_to_a_symlinked_keymap_file") {
            return Ok(());
        }

        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("symlinked-keymap").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let keymap_store = temporary_directory.path().join("keymap-store");
        let linked_keymap = keymap_store.join("tracked-keymap.jsonc");

        fs::create_dir_all(paths.config_directory())
            .map_err(|error| format!("configuration directory setup failed: {error}"))?;
        fs::create_dir_all(&keymap_store)
            .map_err(|error| format!("keymap store setup failed: {error}"))?;
        fs::write(&linked_keymap, runtime::USER_KEYMAP_STUB)
            .map_err(|error| format!("linked keymap setup failed: {error}"))?;
        std::os::unix::fs::symlink(&linked_keymap, paths.user_keymap())
            .map_err(|error| format!("keymap symlink setup failed: {error}"))?;

        let worker_timings = WorkerTimings {
            debounce: DEBOUNCE_INTERVAL,
            poll:     None,
            retry:    RETRY_INTERVAL,
        };
        let mut worker = start_worker(worker_timings, WatchMode::Native);

        wait_for_snapshot(
            &worker,
            paths.user_keymap(),
            Some(runtime::USER_KEYMAP_STUB),
        )?;
        wait_for_watcher(&worker, true)?;

        let edited_contents = b"{\"bindings\": [{\"bindings\": {}}]}";
        let edited_at = Instant::now();
        fs::write(paths.user_keymap(), edited_contents)
            .map_err(|error| format!("symlinked keymap write failed: {error}"))?;
        wait_for_snapshot(&worker, paths.user_keymap(), Some(edited_contents))?;
        assert_within_native_deadline("a direct edit", edited_at.elapsed())?;

        let renamed_contents = b"{\"bindings\": [{\"context\": \"resting\"}]}";
        let staged_keymap = keymap_store.join("tracked-keymap.jsonc.tmp");
        let renamed_at = Instant::now();
        fs::write(&staged_keymap, renamed_contents)
            .map_err(|error| format!("staged keymap write failed: {error}"))?;
        fs::rename(&staged_keymap, &linked_keymap)
            .map_err(|error| format!("rename-over-target save failed: {error}"))?;
        wait_for_snapshot(&worker, paths.user_keymap(), Some(renamed_contents))?;
        assert_within_native_deadline("a rename-over-target save", renamed_at.elapsed())?;

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    fn assert_within_native_deadline(save_form: &str, latency: Duration) -> Result<(), String> {
        if latency > NATIVE_WATCH_DEADLINE {
            return Err(format!(
                "the native watch delivered {save_form} after {latency:?}, past the {NATIVE_WATCH_DEADLINE:?} deadline",
            ));
        }

        Ok(())
    }

    #[test]
    fn polling_audit_detects_suppressed_native_events() -> Result<(), String> {
        let environment_lock = ENVIRONMENT_LOCK
            .lock()
            .expect("environment lock is available");
        let temporary_directory =
            TestDirectory::new("polling-audit").expect("temporary directory exists");
        let xdg_config_home = XdgConfigHome::set(temporary_directory.path());
        let paths = isolated_paths(&temporary_directory)?;
        let worker_timings = WorkerTimings {
            debounce: DEBOUNCE_INTERVAL,
            poll:     Some(POLL_AUDIT_INTERVAL),
            retry:    RETRY_INTERVAL,
        };
        let mut worker = start_worker(worker_timings, WatchMode::PollOnly);

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        wait_for_message(&worker)?;
        fs::write(paths.user_keymap(), b"{\"bindings\": [{\"bindings\": {}}]}")
            .map_err(|error| format!("polled user keymap write failed: {error}"))?;

        wait_for_snapshot(
            &worker,
            paths.user_keymap(),
            Some(b"{\"bindings\": [{\"bindings\": {}}]}"),
        )?;

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }
}
