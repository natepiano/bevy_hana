use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::thread::JoinHandle;

use bevy::log::error;

use super::MonitorConfigurationGenerationTracker;
use super::constants::WINDOWS_NOTIFICATION_WAIT_MILLISECONDS;
use crate::monitors::identity::MonitorIdentificationError;
use crate::monitors::identity::OperatingSystemQueryError;

pub(super) struct WindowsListenerOwner {
    control:        Arc<WindowsListenerControl>,
    shutdown_event: Box<dyn WindowsListenerShutdownEvent>,
    thread:         Option<JoinHandle<()>>,
}

impl WindowsListenerOwner {
    pub(super) fn register<B, E>(
        tracker: Arc<MonitorConfigurationGenerationTracker>,
        backend: B,
        shutdown_event: E,
    ) -> Result<Self, OperatingSystemQueryError>
    where
        B: WindowsListenerBackend,
        E: WindowsListenerShutdownEvent,
    {
        let control = Arc::new(WindowsListenerControl::default());
        let thread_control = Arc::clone(&control);
        let (sender, receiver) = sync_channel(1);
        let thread = thread::Builder::new()
            .spawn(move || notification_thread(backend, tracker, thread_control, sender))
            .map_err(|_| OperatingSystemQueryError::ConfigurationNotificationRegistration)?;

        match receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                control,
                shutdown_event: Box::new(shutdown_event),
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            },
            Err(_) => {
                let _ = thread.join();
                Err(OperatingSystemQueryError::ConfigurationNotificationRegistration)
            },
        }
    }

    pub(super) fn shutdown(&mut self) -> Result<(), OperatingSystemQueryError> {
        self.control.stop();
        let _ = self.shutdown_event.signal();
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        thread
            .join()
            .map_err(|_| OperatingSystemQueryError::ConfigurationNotificationRemoval)
    }
}

impl Drop for WindowsListenerOwner {
    fn drop(&mut self) {
        if self.shutdown().is_err() {
            log_removal_error();
        }
    }
}

fn notification_thread<B>(
    backend: B,
    tracker: Arc<MonitorConfigurationGenerationTracker>,
    control: Arc<WindowsListenerControl>,
    sender: SyncSender<Result<(), OperatingSystemQueryError>>,
) where
    B: WindowsListenerBackend,
{
    let mut resources = WindowsListenerResources::new(backend, tracker);
    match resources.setup() {
        Ok(()) => {},
        Err(error) => {
            let _ = sender.send(Err(error));
            return;
        },
    }
    if sender.send(Ok(())).is_err() {
        return;
    }

    while control.is_listening() {
        match resources.wait_for_event(WINDOWS_NOTIFICATION_WAIT_MILLISECONDS) {
            #[cfg(target_os = "windows")]
            Ok(ListenerWait::Events) => {},
            Ok(ListenerWait::Timeout) => {},
            Ok(ListenerWait::Exit | ListenerWait::Stop) => break,
            Err(()) => {
                resources.fail_notification_stream();
                break;
            },
        }
    }
}

pub(super) trait WindowsListenerShutdownEvent: Send + Sync + 'static {
    fn signal(&self) -> Result<(), ()>;
}

pub(super) trait WindowsListenerBackend: Send + 'static {
    fn register_window_class(
        &mut self,
        tracker: &Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<(), OperatingSystemQueryError>;

    fn create_window(
        &mut self,
        tracker: &Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<(), OperatingSystemQueryError>;

    fn register_device_notification(
        &mut self,
        filter: DeviceNotificationFilter,
    ) -> Result<(), OperatingSystemQueryError>;

    fn wait_for_event(&mut self, timeout_milliseconds: u32) -> Result<ListenerWait, ()>;

    fn unregister_device_notification(&mut self) -> Result<(), ()>;

    fn destroy_window(&mut self) -> Result<(), ()>;

    fn unregister_window_class(&mut self) -> Result<(), ()>;

    fn release_tracker(&mut self, tracker: Arc<MonitorConfigurationGenerationTracker>) {
        drop(tracker);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowUserDataWrite {
    Replaced(isize),
    Empty,
    Failed,
}

impl WindowUserDataWrite {
    const ERROR_SUCCESS_CODE: u32 = 0;

    pub(super) const fn classify(previous: isize, last_error: u32) -> Self {
        match (previous, last_error) {
            (0, Self::ERROR_SUCCESS_CODE) => Self::Empty,
            (0, _) => Self::Failed,
            (previous, _) => Self::Replaced(previous),
        }
    }
}

struct WindowsListenerResources<B>
where
    B: WindowsListenerBackend,
{
    backend: Option<B>,
    stage:   ListenerSetupStage,
    tracker: Option<Arc<MonitorConfigurationGenerationTracker>>,
}

impl<B> WindowsListenerResources<B>
where
    B: WindowsListenerBackend,
{
    const fn new(backend: B, tracker: Arc<MonitorConfigurationGenerationTracker>) -> Self {
        Self {
            backend: Some(backend),
            stage:   ListenerSetupStage::Initial,
            tracker: Some(tracker),
        }
    }

    fn setup(&mut self) -> Result<(), OperatingSystemQueryError> {
        let tracker = self
            .tracker
            .as_ref()
            .ok_or(OperatingSystemQueryError::ConfigurationNotificationRegistration)?;
        let backend = self
            .backend
            .as_mut()
            .ok_or(OperatingSystemQueryError::ConfigurationNotificationRegistration)?;
        backend.register_window_class(tracker)?;
        self.stage = ListenerSetupStage::ClassRegistered;

        backend.create_window(tracker)?;
        self.stage = ListenerSetupStage::WindowCreated;

        backend.register_device_notification(DeviceNotificationFilter::MonitorInterface)?;
        self.stage = ListenerSetupStage::NotificationRegistered;
        Ok(())
    }

    fn fail_notification_stream(&self) {
        if let Some(tracker) = self.tracker.as_deref() {
            tracker.fail_notification_stream();
        }
    }

    fn wait_for_event(&mut self, timeout_milliseconds: u32) -> Result<ListenerWait, ()> {
        self.backend
            .as_mut()
            .ok_or(())?
            .wait_for_event(timeout_milliseconds)
    }

    fn cleanup(&mut self) {
        if self.stage == ListenerSetupStage::NotificationRegistered {
            if self
                .backend
                .as_mut()
                .is_some_and(|backend| backend.unregister_device_notification().is_err())
            {
                log_removal_error();
                self.quarantine_backend_and_tracker();
                return;
            }
            self.stage = ListenerSetupStage::WindowCreated;
        }

        if self.stage == ListenerSetupStage::WindowCreated {
            if self
                .backend
                .as_mut()
                .is_some_and(|backend| backend.destroy_window().is_err())
            {
                log_removal_error();
                self.quarantine_backend_and_tracker();
                return;
            }
            self.stage = ListenerSetupStage::ClassRegistered;
        }

        if self.stage == ListenerSetupStage::ClassRegistered {
            if self
                .backend
                .as_mut()
                .is_some_and(|backend| backend.unregister_window_class().is_err())
            {
                log_removal_error();
                self.release_tracker();
                self.quarantine_backend();
                return;
            }
            self.stage = ListenerSetupStage::Initial;
        }

        self.release_tracker();
        drop(self.backend.take());
    }

    fn release_tracker(&mut self) {
        if let Some(tracker) = self.tracker.take()
            && let Some(backend) = self.backend.as_mut()
        {
            backend.release_tracker(tracker);
        }
    }

    const fn quarantine_backend_and_tracker(&mut self) {
        let backend = self.backend.take();
        let tracker = self.tracker.take();
        std::mem::forget((backend, tracker));
    }

    const fn quarantine_backend(&mut self) { std::mem::forget(self.backend.take()); }
}

impl<B> Drop for WindowsListenerResources<B>
where
    B: WindowsListenerBackend,
{
    fn drop(&mut self) { self.cleanup(); }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListenerSetupStage {
    Initial,
    ClassRegistered,
    WindowCreated,
    NotificationRegistered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListenerWait {
    #[cfg(target_os = "windows")]
    Events,
    Timeout,
    Exit,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeviceNotificationFilter {
    MonitorInterface,
}

#[derive(Default)]
struct WindowsListenerControl(AtomicU8);

impl WindowsListenerControl {
    fn is_listening(&self) -> bool {
        self.0.load(Ordering::Acquire) == WindowsListenerStatus::Listening.to_raw()
    }

    fn stop(&self) {
        self.0
            .store(WindowsListenerStatus::Stopped.to_raw(), Ordering::Release);
    }
}

#[derive(Clone, Copy)]
enum WindowsListenerStatus {
    Listening,
    Stopped,
}

impl WindowsListenerStatus {
    const LISTENING: u8 = 0;
    const STOPPED: u8 = 1;

    const fn to_raw(self) -> u8 {
        match self {
            Self::Listening => Self::LISTENING,
            Self::Stopped => Self::STOPPED,
        }
    }
}

pub(super) fn log_removal_error() {
    let error = MonitorIdentificationError::from(
        OperatingSystemQueryError::ConfigurationNotificationRemoval,
    );
    error!("[MonitorConfiguration] {error}");
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::Weak;
    use std::sync::mpsc::Receiver;
    use std::sync::mpsc::RecvTimeoutError;
    use std::sync::mpsc::Sender;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use super::*;
    use crate::monitors::identity::configuration::MonitorConfigurationState;

    const COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);

    #[test]
    fn registration_uses_monitor_filter_and_cleans_up_failure() {
        let tracker = Arc::new(MonitorConfigurationGenerationTracker::default());
        let state = Arc::new(Mutex::new(FakeBackendState {
            failure: Some(SetupOperation::DeviceNotification),
            ..Default::default()
        }));
        let (shutdown_event, shutdown_receiver) = FakeShutdownEvent::new(Arc::clone(&state));
        let backend = FakeWindowsListenerBackend::new(Arc::clone(&state), shutdown_receiver);
        let (sender, receiver) = sync_channel(1);

        let registration = thread::spawn(move || {
            let result =
                WindowsListenerOwner::register(tracker, backend, shutdown_event).map(|_| ());
            sender
                .send(result)
                .expect("registration result receiver should remain connected");
        });
        let result = receiver
            .recv_timeout(COMPLETION_TIMEOUT)
            .expect("registration failure should complete before the timeout");
        registration
            .join()
            .expect("registration test thread should not panic");

        assert_eq!(
            result,
            Err(OperatingSystemQueryError::ConfigurationNotificationRegistration)
        );
        let state = state
            .lock()
            .expect("fake backend state should not be poisoned");
        assert_eq!(
            state.filter,
            Some(DeviceNotificationFilter::MonitorInterface)
        );
        assert_eq!(
            state.cleanup,
            [
                CleanupOperation::Window,
                CleanupOperation::WindowClass,
                CleanupOperation::Tracker
            ]
        );
        drop(state);
    }

    #[test]
    fn ordinary_stop_joins_and_cleans_up_once() {
        let mut listener = FakeListener::start(VecDeque::new());
        listener.wait_until_waiting();
        assert_eq!(listener.shutdown(SignalResult::Delivered), Ok(()));
        listener.assert_full_cleanup();
    }

    #[test]
    fn already_exited_listener_is_still_joined() {
        let mut listener = FakeListener::start(VecDeque::from([FakeWait::Exit]));
        listener.wait_until_complete();
        assert_eq!(listener.shutdown(SignalResult::Delivered), Ok(()));
        listener.assert_full_cleanup();
        assert!(listener.signal_attempts() > 0);
    }

    #[test]
    fn failed_signal_uses_timeout_fallback_and_joins() {
        let mut listener = FakeListener::start(VecDeque::new());
        listener.wait_until_waiting();
        assert_eq!(listener.shutdown(SignalResult::Failed), Ok(()));
        listener.assert_full_cleanup();
        assert!(listener.timeouts() > 0);
    }

    #[test]
    fn stream_failure_cleans_up_and_marks_tracker_unavailable() {
        let mut listener = FakeListener::start(VecDeque::from([FakeWait::StreamFailure]));
        listener.wait_until_complete();
        assert_eq!(
            listener
                .tracker
                .as_deref()
                .expect("test should retain its tracker owner")
                .state(),
            MonitorConfigurationState::Unavailable(
                OperatingSystemQueryError::ConfigurationNotificationStream.into()
            )
        );
        assert_eq!(listener.shutdown(SignalResult::Delivered), Ok(()));
        listener.assert_full_cleanup();
    }

    #[test]
    fn worker_panic_is_reported_after_unconditional_join() {
        let mut listener = FakeListener::start(VecDeque::from([FakeWait::Panic]));
        listener.wait_until_waiting();

        assert_eq!(
            listener.shutdown(SignalResult::Delivered),
            Err(OperatingSystemQueryError::ConfigurationNotificationRemoval)
        );
        listener.assert_full_cleanup();
    }

    #[test]
    fn notification_cleanup_failure_retains_all_native_ownership() {
        let mut listener = FakeListener::start_with_cleanup_failure(
            VecDeque::from([FakeWait::Exit]),
            CleanupOperation::DeviceNotification,
        );

        assert_eq!(listener.shutdown(SignalResult::Delivered), Ok(()));
        listener.assert_cleanup(&[CleanupOperation::DeviceNotification]);
        listener.assert_ownership(
            FakeOwnership::Owned,
            FakeOwnership::Owned,
            FakeOwnership::Owned,
        );
        listener.assert_callback_safe_after_failed_cleanup();
    }

    #[test]
    fn window_cleanup_failure_retains_window_tracker_and_class() {
        let mut listener = FakeListener::start_with_cleanup_failure(
            VecDeque::from([FakeWait::Exit]),
            CleanupOperation::Window,
        );

        assert_eq!(listener.shutdown(SignalResult::Delivered), Ok(()));
        listener.assert_cleanup(&[
            CleanupOperation::DeviceNotification,
            CleanupOperation::Window,
        ]);
        listener.assert_ownership(
            FakeOwnership::Released,
            FakeOwnership::Owned,
            FakeOwnership::Owned,
        );
        listener.assert_callback_safe_after_failed_cleanup();
    }

    #[test]
    fn window_user_data_write_classifies_nonzero_previous_value() {
        assert_eq!(
            WindowUserDataWrite::classify(23, WindowUserDataWrite::ERROR_SUCCESS_CODE),
            WindowUserDataWrite::Replaced(23)
        );
    }

    #[test]
    fn window_user_data_write_classifies_zero_with_success() {
        assert_eq!(
            WindowUserDataWrite::classify(0, WindowUserDataWrite::ERROR_SUCCESS_CODE),
            WindowUserDataWrite::Empty
        );
    }

    #[test]
    fn window_user_data_write_classifies_zero_with_error() {
        const ACCESS_DENIED_ERROR: u32 = 5;

        assert_eq!(
            WindowUserDataWrite::classify(0, ACCESS_DENIED_ERROR),
            WindowUserDataWrite::Failed
        );
    }

    struct FakeListener {
        completion: Receiver<()>,
        listener:   Option<WindowsListenerOwner>,
        state:      Arc<Mutex<FakeBackendState>>,
        tracker:    Option<Arc<MonitorConfigurationGenerationTracker>>,
        wait:       Receiver<()>,
    }

    impl FakeListener {
        fn start(waits: VecDeque<FakeWait>) -> Self { Self::start_with_state(waits, None) }

        fn start_with_cleanup_failure(
            waits: VecDeque<FakeWait>,
            cleanup_failure: CleanupOperation,
        ) -> Self {
            Self::start_with_state(waits, Some(cleanup_failure))
        }

        fn start_with_state(
            waits: VecDeque<FakeWait>,
            cleanup_failure: Option<CleanupOperation>,
        ) -> Self {
            let tracker = Arc::new(MonitorConfigurationGenerationTracker::default());
            let (completion_sender, completion) = sync_channel(1);
            let (wait_sender, wait) = sync_channel(1);
            let state = Arc::new(Mutex::new(FakeBackendState {
                cleanup_failure,
                completion: Some(completion_sender),
                wait_started: Some(wait_sender),
                waits,
                ..Default::default()
            }));
            let (shutdown_event, shutdown_receiver) = FakeShutdownEvent::new(Arc::clone(&state));
            let backend = FakeWindowsListenerBackend::new(Arc::clone(&state), shutdown_receiver);
            let listener =
                WindowsListenerOwner::register(Arc::clone(&tracker), backend, shutdown_event)
                    .expect("fake listener registration should succeed");
            Self {
                completion,
                listener: Some(listener),
                state,
                tracker: Some(tracker),
                wait,
            }
        }

        fn wait_until_waiting(&self) {
            self.wait
                .recv_timeout(COMPLETION_TIMEOUT)
                .expect("listener should enter its wait before the timeout");
        }

        fn wait_until_complete(&self) {
            self.completion
                .recv_timeout(COMPLETION_TIMEOUT)
                .expect("listener should clean up before the timeout");
        }

        fn shutdown(
            &mut self,
            signal_result: SignalResult,
        ) -> Result<(), OperatingSystemQueryError> {
            let mut listener = self.listener.take().expect("listener should be owned");
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .signal_result = signal_result;
            let (sender, receiver) = sync_channel(1);
            let shutdown = thread::spawn(move || {
                let result = listener.shutdown();
                sender
                    .send(result)
                    .expect("shutdown result receiver should remain connected");
            });
            let result = receiver
                .recv_timeout(COMPLETION_TIMEOUT)
                .expect("listener shutdown should complete before the timeout");
            shutdown
                .join()
                .expect("shutdown test thread should not panic");
            result
        }

        fn assert_full_cleanup(&self) {
            self.assert_cleanup(&[
                CleanupOperation::DeviceNotification,
                CleanupOperation::Window,
                CleanupOperation::WindowClass,
                CleanupOperation::Tracker,
            ]);
            self.assert_ownership(
                FakeOwnership::Released,
                FakeOwnership::Released,
                FakeOwnership::Released,
            );
            assert_eq!(
                self.state
                    .lock()
                    .expect("fake backend state should not be poisoned")
                    .window_releases,
                [ArcReleaseCounts {
                    before: 3,
                    after:  2,
                }]
            );
            assert_eq!(
                Arc::strong_count(
                    self.tracker
                        .as_ref()
                        .expect("test should retain its tracker owner")
                ),
                1
            );
        }

        fn assert_cleanup(&self, expected: &[CleanupOperation]) {
            assert_eq!(
                self.state
                    .lock()
                    .expect("fake backend state should not be poisoned")
                    .cleanup,
                expected
            );
        }

        fn assert_ownership(
            &self,
            device_notification: FakeOwnership,
            window: FakeOwnership,
            window_class: FakeOwnership,
        ) {
            let state = self
                .state
                .lock()
                .expect("fake backend state should not be poisoned");
            assert_eq!(state.device_notification, device_notification);
            assert_eq!(state.window, window);
            assert_eq!(state.window_class, window_class);
            drop(state);
        }

        fn assert_callback_safe_after_failed_cleanup(&mut self) {
            let external_tracker = self
                .tracker
                .take()
                .expect("test should retain its tracker owner");
            assert_eq!(Arc::strong_count(&external_tracker), 3);
            drop(external_tracker);
            let window_tracker = self
                .state
                .lock()
                .expect("fake backend state should not be poisoned")
                .window_tracker
                .clone()
                .expect("fake window should retain a tracker weak reference");
            assert_eq!(window_tracker.strong_count(), 2);
            let tracker = window_tracker
                .upgrade()
                .expect("failed destruction should retain the window tracker");

            tracker.advance();

            assert_eq!(
                tracker.state(),
                MonitorConfigurationState::Ready(1_u64.into())
            );
        }

        fn signal_attempts(&self) -> usize {
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .signal_attempts
        }

        fn timeouts(&self) -> usize {
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .timeouts
        }
    }

    struct FakeWindowsListenerBackend {
        shutdown:       Receiver<()>,
        state:          Arc<Mutex<FakeBackendState>>,
        window_tracker: Option<Arc<MonitorConfigurationGenerationTracker>>,
    }

    impl FakeWindowsListenerBackend {
        fn new(state: Arc<Mutex<FakeBackendState>>, shutdown: Receiver<()>) -> Self {
            Self {
                shutdown,
                state,
                window_tracker: None,
            }
        }

        fn should_fail(&self, operation: SetupOperation) -> Result<(), OperatingSystemQueryError> {
            if self
                .state
                .lock()
                .expect("fake backend state should not be poisoned")
                .failure
                == Some(operation)
            {
                Err(OperatingSystemQueryError::ConfigurationNotificationRegistration)
            } else {
                Ok(())
            }
        }

        fn record_cleanup(&self, operation: CleanupOperation) {
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .cleanup
                .push(operation);
        }

        fn cleanup_should_fail(&self, operation: CleanupOperation) -> bool {
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .cleanup_failure
                == Some(operation)
        }
    }

    impl WindowsListenerBackend for FakeWindowsListenerBackend {
        fn register_window_class(
            &mut self,
            _: &Arc<MonitorConfigurationGenerationTracker>,
        ) -> Result<(), OperatingSystemQueryError> {
            self.should_fail(SetupOperation::WindowClass)?;
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .window_class = FakeOwnership::Owned;
            Ok(())
        }

        fn create_window(
            &mut self,
            tracker: &Arc<MonitorConfigurationGenerationTracker>,
        ) -> Result<(), OperatingSystemQueryError> {
            self.should_fail(SetupOperation::Window)?;
            self.window_tracker = Some(Arc::clone(tracker));
            let mut state = self
                .state
                .lock()
                .expect("fake backend state should not be poisoned");
            state.window = FakeOwnership::Owned;
            state.window_tracker = Some(Arc::downgrade(tracker));
            drop(state);
            Ok(())
        }

        fn register_device_notification(
            &mut self,
            filter: DeviceNotificationFilter,
        ) -> Result<(), OperatingSystemQueryError> {
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .filter = Some(filter);
            self.should_fail(SetupOperation::DeviceNotification)?;
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .device_notification = FakeOwnership::Owned;
            Ok(())
        }

        fn wait_for_event(&mut self, timeout_milliseconds: u32) -> Result<ListenerWait, ()> {
            let (wait, wait_started) = {
                let mut state = self
                    .state
                    .lock()
                    .expect("fake backend state should not be poisoned");
                (state.waits.pop_front(), state.wait_started.take())
            };
            if let Some(wait_started) = wait_started {
                let _ = wait_started.send(());
            }
            match wait {
                Some(FakeWait::Exit) => Ok(ListenerWait::Exit),
                Some(FakeWait::Panic) => panic!("fake listener worker panic"),
                Some(FakeWait::StreamFailure) => Err(()),
                None => match self
                    .shutdown
                    .recv_timeout(Duration::from_millis(u64::from(timeout_milliseconds)))
                {
                    Ok(()) => Ok(ListenerWait::Stop),
                    Err(RecvTimeoutError::Timeout) => {
                        self.state
                            .lock()
                            .expect("fake backend state should not be poisoned")
                            .timeouts += 1;
                        Ok(ListenerWait::Timeout)
                    },
                    Err(RecvTimeoutError::Disconnected) => Err(()),
                },
            }
        }

        fn unregister_device_notification(&mut self) -> Result<(), ()> {
            self.record_cleanup(CleanupOperation::DeviceNotification);
            if self.cleanup_should_fail(CleanupOperation::DeviceNotification) {
                return Err(());
            }
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .device_notification = FakeOwnership::Released;
            Ok(())
        }

        fn destroy_window(&mut self) -> Result<(), ()> {
            self.record_cleanup(CleanupOperation::Window);
            if self.cleanup_should_fail(CleanupOperation::Window) {
                return Err(());
            }
            let window_tracker = self
                .window_tracker
                .take()
                .expect("fake window should own a tracker");
            let before = Arc::strong_count(&window_tracker);
            drop(window_tracker);
            let mut state = self
                .state
                .lock()
                .expect("fake backend state should not be poisoned");
            let after = state.window_tracker.as_ref().map_or(0, Weak::strong_count);
            state
                .window_releases
                .push(ArcReleaseCounts { before, after });
            state.window = FakeOwnership::Released;
            drop(state);
            Ok(())
        }

        fn unregister_window_class(&mut self) -> Result<(), ()> {
            self.record_cleanup(CleanupOperation::WindowClass);
            if self.cleanup_should_fail(CleanupOperation::WindowClass) {
                return Err(());
            }
            self.state
                .lock()
                .expect("fake backend state should not be poisoned")
                .window_class = FakeOwnership::Released;
            Ok(())
        }

        fn release_tracker(&mut self, tracker: Arc<MonitorConfigurationGenerationTracker>) {
            self.record_cleanup(CleanupOperation::Tracker);
            let completion = self
                .state
                .lock()
                .expect("fake backend state should not be poisoned")
                .completion
                .take();
            drop(tracker);
            if let Some(completion) = completion {
                let _ = completion.send(());
            }
        }
    }

    struct FakeShutdownEvent {
        shutdown: Sender<()>,
        state:    Arc<Mutex<FakeBackendState>>,
    }

    impl FakeShutdownEvent {
        fn new(state: Arc<Mutex<FakeBackendState>>) -> (Self, Receiver<()>) {
            let (shutdown, receiver) = channel();
            (Self { shutdown, state }, receiver)
        }
    }

    impl WindowsListenerShutdownEvent for FakeShutdownEvent {
        fn signal(&self) -> Result<(), ()> {
            let signal_result = {
                let mut state = self
                    .state
                    .lock()
                    .expect("fake backend state should not be poisoned");
                state.signal_attempts += 1;
                state.signal_result
            };
            match signal_result {
                SignalResult::Delivered => self.shutdown.send(()).map_err(|_| ()),
                SignalResult::Failed => Err(()),
            }
        }
    }

    #[derive(Default)]
    struct FakeBackendState {
        cleanup:             Vec<CleanupOperation>,
        cleanup_failure:     Option<CleanupOperation>,
        completion:          Option<SyncSender<()>>,
        device_notification: FakeOwnership,
        failure:             Option<SetupOperation>,
        filter:              Option<DeviceNotificationFilter>,
        signal_attempts:     usize,
        signal_result:       SignalResult,
        timeouts:            usize,
        wait_started:        Option<SyncSender<()>>,
        waits:               VecDeque<FakeWait>,
        window:              FakeOwnership,
        window_class:        FakeOwnership,
        window_releases:     Vec<ArcReleaseCounts>,
        window_tracker:      Option<Weak<MonitorConfigurationGenerationTracker>>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct ArcReleaseCounts {
        before: usize,
        after:  usize,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CleanupOperation {
        DeviceNotification,
        Window,
        WindowClass,
        Tracker,
    }

    #[derive(Clone, Copy)]
    enum FakeWait {
        Exit,
        Panic,
        StreamFailure,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SetupOperation {
        WindowClass,
        Window,
        DeviceNotification,
    }

    #[derive(Clone, Copy, Default)]
    enum SignalResult {
        #[default]
        Delivered,
        Failed,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    enum FakeOwnership {
        #[default]
        Absent,
        Owned,
        Released,
    }
}
