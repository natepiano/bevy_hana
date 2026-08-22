use std::sync::Arc;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Foundation::LRESULT;
use windows::Win32::Foundation::SetLastError;
use windows::Win32::Foundation::TRUE;
use windows::Win32::Foundation::WAIT_EVENT;
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Foundation::WAIT_TIMEOUT;
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::System::Threading::SetEvent;
use windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
use windows::Win32::UI::WindowsAndMessaging::CreateWindowExW;
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE;
use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
use windows::Win32::UI::WindowsAndMessaging::DispatchMessageW;
use windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA;
use windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;
use windows::Win32::UI::WindowsAndMessaging::HDEVNOTIFY;
use windows::Win32::UI::WindowsAndMessaging::MSG;
use windows::Win32::UI::WindowsAndMessaging::MWMO_INPUTAVAILABLE;
use windows::Win32::UI::WindowsAndMessaging::MsgWaitForMultipleObjectsEx;
use windows::Win32::UI::WindowsAndMessaging::PM_REMOVE;
use windows::Win32::UI::WindowsAndMessaging::PeekMessageW;
use windows::Win32::UI::WindowsAndMessaging::QS_ALLINPUT;
use windows::Win32::UI::WindowsAndMessaging::RegisterClassW;
use windows::Win32::UI::WindowsAndMessaging::RegisterDeviceNotificationW;
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW;
use windows::Win32::UI::WindowsAndMessaging::TranslateMessage;
use windows::Win32::UI::WindowsAndMessaging::UnregisterClassW;
use windows::Win32::UI::WindowsAndMessaging::UnregisterDeviceNotification;
use windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE;
use windows::Win32::UI::WindowsAndMessaging::WM_DEVICECHANGE;
use windows::Win32::UI::WindowsAndMessaging::WM_DISPLAYCHANGE;
use windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE;
use windows::Win32::UI::WindowsAndMessaging::WM_NCDESTROY;
use windows::Win32::UI::WindowsAndMessaging::WM_QUIT;
use windows::Win32::UI::WindowsAndMessaging::WNDCLASSW;
use windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPED;
use windows::core::PCWSTR;

use super::MonitorConfigurationGenerationTracker;
use super::constants::WINDOWS_NOTIFICATION_CLASS_PREFIX;
use super::windows_device_notification;
use super::windows_listener;
use super::windows_listener::DeviceNotificationFilter;
use super::windows_listener::ListenerWait;
use super::windows_listener::WindowUserDataWrite;
use super::windows_listener::WindowsListenerBackend;
use super::windows_listener::WindowsListenerOwner;
use super::windows_listener::WindowsListenerShutdownEvent;
use crate::monitors::identity::MonitorIdentificationError;
use crate::monitors::identity::OperatingSystemQueryError;

pub(super) struct WindowsConfigurationNotification {
    listener: WindowsListenerOwner,
}

impl WindowsConfigurationNotification {
    pub(super) fn register(
        tracker: Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<Self, MonitorIdentificationError> {
        let shutdown_event = NativeWindowsShutdownEvent::new()?;
        let backend = NativeWindowsListenerBackend::new(shutdown_event.handle());
        WindowsListenerOwner::register(tracker, backend, shutdown_event)
            .map(|listener| Self { listener })
            .map_err(Into::into)
    }

    pub(super) fn unregister(&mut self) {
        if self.listener.shutdown().is_err() {
            windows_listener::log_removal_error();
        }
    }
}

struct NativeWindowsListenerBackend {
    class_name:          Vec<u16>,
    device_notification: Option<usize>,
    instance:            Option<usize>,
    shutdown_event:      usize,
    window:              Option<usize>,
}

impl NativeWindowsListenerBackend {
    const fn new(shutdown_event: usize) -> Self {
        Self {
            class_name: Vec::new(),
            device_notification: None,
            instance: None,
            shutdown_event,
            window: None,
        }
    }
}

struct NativeWindowsShutdownEvent {
    handle: usize,
}

impl NativeWindowsShutdownEvent {
    fn new() -> Result<Self, OperatingSystemQueryError> {
        let handle = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map_err(|_| registration_error())?;
        Ok(Self {
            handle: handle.0.expose_provenance(),
        })
    }

    const fn handle(&self) -> usize { self.handle }
}

impl WindowsListenerShutdownEvent for NativeWindowsShutdownEvent {
    fn signal(&self) -> Result<(), ()> {
        unsafe { SetEvent(handle_from_address(self.handle)) }.map_err(|_| ())
    }
}

impl Drop for NativeWindowsShutdownEvent {
    fn drop(&mut self) {
        if unsafe { CloseHandle(handle_from_address(self.handle)) }.is_err() {
            windows_listener::log_removal_error();
        }
    }
}

struct WindowCreationContext<'a> {
    tracker: &'a Arc<MonitorConfigurationGenerationTracker>,
}

impl WindowsListenerBackend for NativeWindowsListenerBackend {
    fn register_window_class(
        &mut self,
        tracker: &Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<(), OperatingSystemQueryError> {
        let module = unsafe { GetModuleHandleW(None) }.map_err(|_| registration_error())?;
        let instance = HINSTANCE(module.0);
        self.class_name = format!(
            "{WINDOWS_NOTIFICATION_CLASS_PREFIX}{:p}",
            Arc::as_ptr(tracker)
        )
        .encode_utf16()
        .chain([0])
        .collect();
        let window_class = WNDCLASSW {
            hInstance: instance,
            lpfnWndProc: Some(configuration_window_proc),
            lpszClassName: PCWSTR(self.class_name.as_ptr()),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&raw const window_class) } == 0 {
            return Err(registration_error());
        }
        self.instance = Some(instance.0.expose_provenance());
        Ok(())
    }

    fn create_window(
        &mut self,
        tracker: &Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<(), OperatingSystemQueryError> {
        let Some(instance) = self.instance else {
            return Err(registration_error());
        };
        let instance = HINSTANCE(std::ptr::with_exposed_provenance_mut(instance));
        let creation_context = WindowCreationContext { tracker };
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(self.class_name.as_ptr()),
                PCWSTR::null(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                Some(std::ptr::from_ref(&creation_context).cast()),
            )
        }
        .map_err(|_| registration_error())?;
        let window_address = window.0.expose_provenance();
        self.window = Some(window_address);
        Ok(())
    }

    fn register_device_notification(
        &mut self,
        filter: DeviceNotificationFilter,
    ) -> Result<(), OperatingSystemQueryError> {
        let Some(window) = self.window else {
            return Err(registration_error());
        };
        let window = HWND(std::ptr::with_exposed_provenance_mut(window));
        let filter = match filter {
            DeviceNotificationFilter::MonitorInterface => {
                windows_device_notification::monitor_device_notification_filter()?
            },
        };
        let notification = unsafe {
            RegisterDeviceNotificationW(
                HANDLE(window.0),
                std::ptr::from_ref(&filter).cast(),
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        }
        .map_err(|_| registration_error())?;
        self.device_notification = Some(notification.0.expose_provenance());
        Ok(())
    }

    fn wait_for_event(&mut self, timeout_milliseconds: u32) -> Result<ListenerWait, ()> {
        let shutdown_events = [handle_from_address(self.shutdown_event)];
        let wait = unsafe {
            MsgWaitForMultipleObjectsEx(
                Some(&shutdown_events),
                timeout_milliseconds,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        let event_count = u32::try_from(shutdown_events.len()).map_err(|_| ())?;
        let message_queue_ready = WAIT_EVENT(WAIT_OBJECT_0.0 + event_count);
        match wait {
            WAIT_TIMEOUT => Ok(ListenerWait::Timeout),
            WAIT_OBJECT_0 => Ok(ListenerWait::Stop),
            result if result == message_queue_ready => Ok(dispatch_pending_messages()),
            _ => Err(()),
        }
    }

    fn unregister_device_notification(&mut self) -> Result<(), ()> {
        let Some(notification) = self.device_notification else {
            return Ok(());
        };
        let notification = HDEVNOTIFY(std::ptr::with_exposed_provenance_mut(notification));
        unsafe { UnregisterDeviceNotification(notification) }.map_err(|_| ())?;
        self.device_notification = None;
        Ok(())
    }

    fn destroy_window(&mut self) -> Result<(), ()> {
        let Some(window) = self.window else {
            return Ok(());
        };
        let window = HWND(std::ptr::with_exposed_provenance_mut(window));
        unsafe { DestroyWindow(window) }.map_err(|_| ())?;
        self.window = None;
        Ok(())
    }

    fn unregister_window_class(&mut self) -> Result<(), ()> {
        let Some(instance) = self.instance else {
            return Ok(());
        };
        let instance = HINSTANCE(std::ptr::with_exposed_provenance_mut(instance));
        unsafe { UnregisterClassW(PCWSTR(self.class_name.as_ptr()), Some(instance)) }
            .map_err(|_| ())?;
        self.class_name.clear();
        self.instance = None;
        Ok(())
    }
}

fn dispatch_pending_messages() -> ListenerWait {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        if message.message == WM_QUIT {
            return ListenerWait::Exit;
        }
        unsafe {
            let _ = TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
    ListenerWait::Events
}

const fn registration_error() -> OperatingSystemQueryError {
    OperatingSystemQueryError::ConfigurationNotificationRegistration
}

const fn handle_from_address(address: usize) -> HANDLE {
    HANDLE(std::ptr::with_exposed_provenance_mut(address))
}

fn configuration_message_result(message: u32) -> Option<LRESULT> {
    match message {
        WM_DISPLAYCHANGE => Some(LRESULT(0)),
        WM_DEVICECHANGE => Some(LRESULT(isize::from(TRUE.as_bool()))),
        _ => None,
    }
}

unsafe fn install_window_tracker(window: HWND, lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let create =
        unsafe { &*std::ptr::with_exposed_provenance::<CREATESTRUCTW>(lparam.0.cast_unsigned()) };
    if create.lpCreateParams.is_null() {
        return false;
    }
    let creation_context = unsafe { &*create.lpCreateParams.cast::<WindowCreationContext<'_>>() };
    let tracker = Arc::into_raw(Arc::clone(creation_context.tracker));
    let tracker_address = tracker.expose_provenance().cast_signed();
    match unsafe { write_window_user_data(window, tracker_address) } {
        WindowUserDataWrite::Empty => true,
        WindowUserDataWrite::Failed => {
            drop(unsafe { Arc::from_raw(tracker) });
            false
        },
        WindowUserDataWrite::Replaced(_) => {
            // `tracker` remains in `GWLP_USERDATA` for `WM_NCDESTROY`. The
            // previous value has unknown provenance and is quarantined.
            windows_listener::log_removal_error();
            false
        },
    }
}

unsafe fn release_window_tracker(window: HWND) {
    match unsafe { write_window_user_data(window, 0) } {
        WindowUserDataWrite::Replaced(tracker) => {
            let tracker = std::ptr::with_exposed_provenance::<MonitorConfigurationGenerationTracker>(
                tracker.cast_unsigned(),
            );
            drop(unsafe { Arc::from_raw(tracker) });
        },
        WindowUserDataWrite::Empty => {},
        WindowUserDataWrite::Failed => {
            // The raw `Arc` may remain in `GWLP_USERDATA`, so reconstructing it
            // could invalidate a pointer still referenced by the window. Its
            // strong reference is deliberately quarantined for process lifetime.
            windows_listener::log_removal_error();
        },
    }
}

unsafe fn write_window_user_data(window: HWND, value: isize) -> WindowUserDataWrite {
    unsafe {
        SetLastError(WIN32_ERROR(0));
    }
    let previous = unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, value) };
    if previous != 0 {
        return WindowUserDataWrite::classify(previous, ERROR_SUCCESS.0);
    }
    WindowUserDataWrite::classify(previous, unsafe { GetLastError() }.0)
}

unsafe fn process_configuration_message(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) {
    let tracker = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) };
    if tracker == 0 {
        return;
    }
    let tracker = unsafe {
        &*std::ptr::with_exposed_provenance::<MonitorConfigurationGenerationTracker>(
            tracker.cast_unsigned(),
        )
    };
    unsafe {
        windows_device_notification::process_configuration_message(
            message, wparam, lparam, tracker,
        );
    }
}

unsafe extern "system" fn configuration_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let installed = unsafe { install_window_tracker(window, lparam) };
            if !installed {
                return LRESULT(0);
            }
        },
        WM_NCDESTROY => {
            unsafe { release_window_tracker(window) };
            let _ = unsafe { DefWindowProcW(window, message, wparam, lparam) };
            return LRESULT(0);
        },
        WM_DISPLAYCHANGE | WM_DEVICECHANGE => {
            unsafe { process_configuration_message(window, message, wparam, lparam) };
            if let Some(result) = configuration_message_result(message) {
                return result;
            }
        },
        _ => {},
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_change_returns_zero() {
        assert_eq!(
            configuration_message_result(WM_DISPLAYCHANGE),
            Some(LRESULT(0))
        );
    }

    #[test]
    fn every_device_change_returns_true() {
        assert_eq!(
            configuration_message_result(WM_DEVICECHANGE),
            Some(LRESULT(isize::from(TRUE.as_bool())))
        );
    }
}
