use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;

use bevy::log::error;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
use x11rb::protocol::randr::NotifyMask;
use x11rb::xcb_ffi::XCBConnection;

use super::MonitorConfigurationGenerationTracker;
use super::constants::X11_NOTIFICATION_POLL_INTERVAL;
use crate::monitors::identity::MonitorIdentificationError;
use crate::monitors::identity::OperatingSystemQueryError;

pub(super) struct X11ConfigurationNotification {
    control: Arc<ListenerControl>,
    thread:  Option<JoinHandle<()>>,
}

impl X11ConfigurationNotification {
    pub(super) fn register(
        tracker: Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<Self, MonitorIdentificationError> {
        let (connection, screen_number) = XCBConnection::connect(None)
            .map_err(|_| OperatingSystemQueryError::ConfigurationNotificationRegistration)?;
        let root = connection
            .setup()
            .roots
            .get(screen_number)
            .ok_or(OperatingSystemQueryError::ConfigurationNotificationRegistration)?
            .root;
        let mask = NotifyMask::SCREEN_CHANGE
            | NotifyMask::CRTC_CHANGE
            | NotifyMask::OUTPUT_CHANGE
            | NotifyMask::OUTPUT_PROPERTY
            | NotifyMask::PROVIDER_CHANGE
            | NotifyMask::RESOURCE_CHANGE;
        connection
            .randr_select_input(root, mask)
            .map_err(|_| OperatingSystemQueryError::ConfigurationNotificationRegistration)?
            .check()
            .map_err(|_| OperatingSystemQueryError::ConfigurationNotificationRegistration)?;

        let control = Arc::new(ListenerControl::default());
        let thread_control = Arc::clone(&control);
        let thread = thread::Builder::new()
            .spawn(move || listen(connection, tracker, thread_control))
            .map_err(|_| OperatingSystemQueryError::ConfigurationNotificationRegistration)?;
        Ok(Self {
            control,
            thread: Some(thread),
        })
    }

    pub(super) fn unregister(&mut self) {
        self.control.stop();
        let Some(thread) = self.thread.take() else {
            return;
        };
        thread.thread().unpark();
        if thread.join().is_err() {
            let error = MonitorIdentificationError::from(
                OperatingSystemQueryError::ConfigurationNotificationRemoval,
            );
            error!("[MonitorConfiguration] {error}");
        }
    }
}

fn listen(
    connection: XCBConnection,
    tracker: Arc<MonitorConfigurationGenerationTracker>,
    control: Arc<ListenerControl>,
) {
    while control.is_listening() {
        match connection.poll_for_event() {
            Ok(Some(Event::RandrNotify(_) | Event::RandrScreenChangeNotify(_))) => {
                tracker.advance();
            },
            Ok(Some(_)) => {},
            Ok(None) => thread::park_timeout(X11_NOTIFICATION_POLL_INTERVAL),
            Err(_) => {
                tracker.fail_notification_stream();
                return;
            },
        }
    }
}

#[derive(Default)]
struct ListenerControl(AtomicU8);

impl ListenerControl {
    fn is_listening(&self) -> bool {
        self.0.load(Ordering::Acquire) == ListenerStatus::Listening.to_raw()
    }

    fn stop(&self) {
        self.0
            .store(ListenerStatus::Stopped.to_raw(), Ordering::Release);
    }
}

#[derive(Clone, Copy)]
enum ListenerStatus {
    Listening,
    Stopped,
}

impl ListenerStatus {
    const LISTENING: u8 = 0;
    const STOPPED: u8 = 1;

    const fn to_raw(self) -> u8 {
        match self {
            Self::Listening => Self::LISTENING,
            Self::Stopped => Self::STOPPED,
        }
    }
}
