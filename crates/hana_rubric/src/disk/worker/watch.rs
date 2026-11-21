use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
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
            self.dirty_at = Some(self.clock.now() + self.worker_timings.debounce);
            #[cfg(test)]
            self.status.record_dirty_notification();
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

        self.read_user_keymap(paths, self.clock.now());
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
        self.status.set_watching(false);

        if self.watch_mode.is_injected() {
            if paths.config_directory().is_dir() {
                self.status.set_watching(true);
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
        self.status.set_watching(true);

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
    use std::sync::mpsc;
    use std::time::Duration;

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
    use crate::disk::worker::channels::DiskWorkerChannels;
    use crate::disk::worker::runtime;
    use crate::disk::worker::runtime::WorkerClock;
    use crate::disk::worker::runtime::WorkerTimings;

    const DEFAULT_KEYMAP: &[u8] = b"{\"bindings\": []}";
    const DEBOUNCE_INTERVAL: Duration = Duration::from_millis(100);
    const POLL_AUDIT_INTERVAL: Duration = Duration::from_millis(20);
    const RETRY_INTERVAL: Duration = Duration::from_millis(30);
    const TEST_APP_NAME: &str = "hana-rubric-watch-test";

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

    fn start_worker(
        worker_timings: WorkerTimings,
        watch_mode: WatchMode,
        clock: WorkerClock,
    ) -> DiskWorkerChannels {
        let paths = KeymapPathAvailability::for_app_name(TEST_APP_NAME)
            .into_resolved()
            .expect("test keymap paths resolve");
        runtime::start_disk_worker_with(
            &paths,
            DEFAULT_KEYMAP.to_vec(),
            Some(b"{}".to_vec()),
            worker_timings,
            watch_mode,
            clock,
        )
    }

    /// Every wait below ends on the worker's own signal rather than on a deadline, so a busy
    /// machine changes how long a test takes and never whether it passes. A signal that never
    /// arrives parks the test for nextest's `slow-timeout` in `.config/nextest.toml`, which
    /// reports the worker as stuck -- the fact a deadline here would have obscured.
    fn wait_for_message(worker: &DiskWorkerChannels) { drop(worker.await_message()); }

    fn wait_for_snapshot(
        worker: &DiskWorkerChannels,
        path: &Path,
        expected_contents: Option<&[u8]>,
    ) {
        while channels::assert_snapshot(&worker.await_message(), path, expected_contents).is_err() {
        }
    }

    fn wait_for_watcher(worker: &DiskWorkerChannels, expected: bool) {
        worker.wait_for_activity(|activity| activity.watching == expected);
    }

    fn wait_for_dirty_notifications(worker: &DiskWorkerChannels, expected: usize) {
        worker.wait_for_activity(|activity| activity.dirty_notifications >= expected);
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
        let mut worker = start_worker(worker_timings, WatchMode::Injected, WorkerClock::manual());

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        wait_for_message(&worker);
        wait_for_watcher(&worker, true);

        fs::remove_dir_all(paths.config_directory())
            .map_err(|error| format!("configuration directory removal failed: {error}"))?;
        worker.inject_watcher_failure()?;
        wait_for_watcher(&worker, false);
        fs::create_dir_all(paths.config_directory())
            .map_err(|error| format!("configuration directory recreation failed: {error}"))?;
        worker.inject_watcher_failure()?;
        wait_for_watcher(&worker, true);
        worker.advance_clock(worker_timings.retry)?;
        worker.expect_snapshot(paths.user_keymap(), None)?;
        let read_attempts_before_write = worker.activity(|activity| activity.read_attempts);
        fs::write(paths.user_keymap(), b"{\"bindings\": []}")
            .map_err(|error| format!("recreated user keymap write failed: {error}"))?;
        worker.inject_watcher_dirty()?;
        wait_for_dirty_notifications(&worker, 1);
        worker.advance_clock(worker_timings.debounce)?;

        worker.expect_snapshot(paths.user_keymap(), Some(b"{\"bindings\": []}"))?;
        if worker.activity(|activity| activity.read_attempts) != read_attempts_before_write + 1 {
            return Err(String::from(
                "the recreated watcher snapshot did not come from its injected watcher edge",
            ));
        }

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    /// Wait for the operating system to deliver an event for a write to the watched directory.
    fn wait_for_native_notification() -> Result<(), String> {
        let directory = TestDirectory::new("notification-probe").map_err(|error| {
            format!("native notification probe directory creation failed: {error}")
        })?;
        let (sender, receiver) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let _ = sender.send(event);
        })
        .map_err(|error| format!("native notification probe watcher creation failed: {error}"))?;
        watcher
            .watch(directory.path(), RecursiveMode::NonRecursive)
            .map_err(|error| format!("native notification probe watch failed: {error}"))?;

        fs::write(directory.path().join("probe.jsonc"), b"{}")
            .map_err(|error| format!("native notification probe write failed: {error}"))?;
        receiver
            .recv()
            .map_err(|error| format!("native notification probe channel closed: {error}"))?
            .map_err(|error| format!("native notification probe reported an error: {error}"))?;
        Ok(())
    }

    #[test]
    fn a_native_watch_carries_an_edit_without_a_poll_audit() -> Result<(), String> {
        wait_for_native_notification()?;

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
        let mut worker = start_worker(worker_timings, WatchMode::Native, WorkerClock::System);

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
        );
        wait_for_watcher(&worker, true);

        // The native watch must deliver the edit. If the edit never arrives, nextest's
        // `slow-timeout` ends the test.
        let edited_contents = b"{\"bindings\": [{\"bindings\": {}}]}";
        fs::write(paths.user_keymap(), edited_contents)
            .map_err(|error| format!("native watch keymap write failed: {error}"))?;
        wait_for_snapshot(&worker, paths.user_keymap(), Some(edited_contents));

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
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
        let mut worker = start_worker(worker_timings, WatchMode::PollOnly, WorkerClock::manual());

        assert!(
            paths
                .config_directory()
                .starts_with(temporary_directory.path())
        );
        wait_for_message(&worker);
        fs::write(paths.user_keymap(), b"{\"bindings\": [{\"bindings\": {}}]}")
            .map_err(|error| format!("polled user keymap write failed: {error}"))?;
        worker.advance_clock(POLL_AUDIT_INTERVAL)?;

        worker.expect_snapshot(
            paths.user_keymap(),
            Some(b"{\"bindings\": [{\"bindings\": {}}]}"),
        )?;

        worker.shutdown();
        drop(xdg_config_home);
        drop(environment_lock);
        Ok(())
    }

    /// Keymap paths reached through a symlink. These tests create it with
    /// `std::os::unix::fs::symlink`, which Windows does not have.
    #[cfg(unix)]
    mod symlink {
        use std::fs;

        use super::DEBOUNCE_INTERVAL;
        use super::ENVIRONMENT_LOCK;
        use super::RETRY_INTERVAL;
        use super::TestDirectory;
        use super::WatchMode;
        use super::WorkerClock;
        use super::WorkerTimings;
        use super::XdgConfigHome;
        use super::isolated_paths;
        use super::runtime;
        use super::start_worker;
        use super::wait_for_native_notification;
        use super::wait_for_snapshot;
        use super::wait_for_watcher;

        const MISSING_SYMLINK_TARGET: &str = "missing-keymap-target/user-keymap.jsonc";

        /// A symlinked keymap file whose target directory is missing fails the same way on every
        /// watcher recreation, and `recreate_watcher` clears the reported watch error before it
        /// reports that failure. The report must still be compared against what was reported last.
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

        /// A dotfile manager links one tracked file into an otherwise untracked directory, so the
        /// keymap path is a symlink to a file the configuration directory does not contain. An
        /// editor then saves by renaming a temporary file over that target, which replaces
        /// the file a file-only watch was placed on.
        #[test]
        fn a_native_watch_carries_edits_to_a_symlinked_keymap_file() -> Result<(), String> {
            wait_for_native_notification()?;

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
            let mut worker = start_worker(worker_timings, WatchMode::Native, WorkerClock::System);

            wait_for_snapshot(
                &worker,
                paths.user_keymap(),
                Some(runtime::USER_KEYMAP_STUB),
            );
            wait_for_watcher(&worker, true);

            // Both saves assert delivery and nothing about how quickly it arrived, for the reason
            // given on the direct-edit test above.
            let edited_contents = b"{\"bindings\": [{\"bindings\": {}}]}";
            fs::write(paths.user_keymap(), edited_contents)
                .map_err(|error| format!("symlinked keymap write failed: {error}"))?;
            wait_for_snapshot(&worker, paths.user_keymap(), Some(edited_contents));

            let renamed_contents = b"{\"bindings\": [{\"context\": \"resting\"}]}";
            let staged_keymap = keymap_store.join("tracked-keymap.jsonc.tmp");
            fs::write(&staged_keymap, renamed_contents)
                .map_err(|error| format!("staged keymap write failed: {error}"))?;
            fs::rename(&staged_keymap, &linked_keymap)
                .map_err(|error| format!("rename-over-target save failed: {error}"))?;
            wait_for_snapshot(&worker, paths.user_keymap(), Some(renamed_contents));

            worker.shutdown();
            drop(xdg_config_home);
            drop(environment_lock);
            Ok(())
        }
    }
}
