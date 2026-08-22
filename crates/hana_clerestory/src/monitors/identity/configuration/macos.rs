use std::ffi::c_void;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

use bevy::log::error;

use super::MonitorConfigurationGenerationTracker;
use super::constants::CORE_GRAPHICS_SUCCESS;
use super::constants::MACOS_BEGIN_CONFIGURATION_FLAG;
use crate::monitors::identity::MonitorIdentificationError;
use crate::monitors::identity::OperatingSystemQueryError;

type CoreGraphicsDisplayReconfigurationCallback =
    Option<unsafe extern "C-unwind" fn(u32, u32, *mut c_void)>;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C-unwind" {
    fn CGDisplayRegisterReconfigurationCallback(
        callback: CoreGraphicsDisplayReconfigurationCallback,
        user_info: *mut c_void,
    ) -> i32;
    fn CGDisplayRemoveReconfigurationCallback(
        callback: CoreGraphicsDisplayReconfigurationCallback,
        user_info: *mut c_void,
    ) -> i32;
}

pub(super) struct MacOsConfigurationNotification {
    registration: DisplayReconfigurationRegistration<NativeCoreGraphics>,
}

impl MacOsConfigurationNotification {
    pub(super) fn register(
        tracker: Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<Self, MonitorIdentificationError> {
        DisplayReconfigurationRegistration::register(NativeCoreGraphics, tracker)
            .map(|registration| Self { registration })
    }

    pub(super) fn unregister(&mut self) {
        if let Err(error) = self.registration.unregister() {
            error!("[MonitorConfiguration] {error}");
        }
    }
}

trait CoreGraphicsApi {
    fn register(
        &self,
        callback: CoreGraphicsDisplayReconfigurationCallback,
        user_info: *mut c_void,
    ) -> i32;

    fn remove(
        &self,
        callback: CoreGraphicsDisplayReconfigurationCallback,
        user_info: *mut c_void,
    ) -> i32;
}

struct NativeCoreGraphics;

impl CoreGraphicsApi for NativeCoreGraphics {
    fn register(
        &self,
        callback: CoreGraphicsDisplayReconfigurationCallback,
        user_info: *mut c_void,
    ) -> i32 {
        unsafe { CGDisplayRegisterReconfigurationCallback(callback, user_info) }
    }

    fn remove(
        &self,
        callback: CoreGraphicsDisplayReconfigurationCallback,
        user_info: *mut c_void,
    ) -> i32 {
        unsafe { CGDisplayRemoveReconfigurationCallback(callback, user_info) }
    }
}

struct DisplayReconfigurationRegistration<A>
where
    A: CoreGraphicsApi,
{
    api:     A,
    context: Option<DisplayReconfigurationContext>,
}

impl<A> DisplayReconfigurationRegistration<A>
where
    A: CoreGraphicsApi,
{
    fn register(
        api: A,
        tracker: Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<Self, MonitorIdentificationError> {
        let context = DisplayReconfigurationContext(tracker);
        let result = api.register(Some(display_reconfigured), context.user_info());
        if result != CORE_GRAPHICS_SUCCESS {
            return Err(OperatingSystemQueryError::ConfigurationNotificationRegistration.into());
        }
        Ok(Self {
            api,
            context: Some(context),
        })
    }

    fn unregister(&mut self) -> Result<(), MonitorIdentificationError> {
        let Some(context) = self.context.take() else {
            return Ok(());
        };
        let result = self
            .api
            .remove(Some(display_reconfigured), context.user_info());
        if result == CORE_GRAPHICS_SUCCESS {
            return Ok(());
        }
        display_reconfiguration_quarantine().retain(context);
        Err(OperatingSystemQueryError::ConfigurationNotificationRemoval.into())
    }
}

struct DisplayReconfigurationContext(Arc<MonitorConfigurationGenerationTracker>);

impl DisplayReconfigurationContext {
    fn user_info(&self) -> *mut c_void { Arc::as_ptr(&self.0).cast_mut().cast() }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TestRegistrationState {
    Registered,
    #[default]
    Removed,
}

#[derive(Default)]
struct DisplayReconfigurationQuarantine {
    contexts: Mutex<Vec<DisplayReconfigurationContext>>,
}

impl DisplayReconfigurationQuarantine {
    fn retain(&self, context: DisplayReconfigurationContext) {
        self.contexts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(context);
    }

    #[cfg(test)]
    fn reclaim_if_removed(
        &self,
        user_info: *mut c_void,
        registration: TestRegistrationState,
    ) -> Option<DisplayReconfigurationContext> {
        if registration == TestRegistrationState::Registered {
            return None;
        }
        let mut contexts = self
            .contexts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let position = contexts
            .iter()
            .position(|context| context.user_info() == user_info)?;
        Some(contexts.swap_remove(position))
    }
}

fn display_reconfiguration_quarantine() -> &'static DisplayReconfigurationQuarantine {
    static QUARANTINE: OnceLock<DisplayReconfigurationQuarantine> = OnceLock::new();
    QUARANTINE.get_or_init(DisplayReconfigurationQuarantine::default)
}

unsafe extern "C-unwind" fn display_reconfigured(_: u32, flags: u32, user_info: *mut c_void) {
    if flags & MACOS_BEGIN_CONFIGURATION_FLAG != 0 || user_info.is_null() {
        return;
    }
    let tracker = unsafe { &*user_info.cast::<MonitorConfigurationGenerationTracker>() };
    tracker.advance();
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fmt::Formatter;

    use super::*;
    use crate::monitors::identity::configuration::MonitorConfigurationState;

    const CORE_GRAPHICS_FAILURE: i32 = 1;

    #[test]
    fn callback_ignores_begin_and_advances_after_completion() {
        let tracker = Arc::new(MonitorConfigurationGenerationTracker::default());
        let user_info = Arc::as_ptr(&tracker).cast_mut().cast();

        unsafe {
            display_reconfigured(0, MACOS_BEGIN_CONFIGURATION_FLAG, user_info);
        }
        assert_eq!(
            tracker.state(),
            MonitorConfigurationState::Ready(0_u64.into())
        );

        unsafe {
            display_reconfigured(0, 0, user_info);
        }
        assert_eq!(
            tracker.state(),
            MonitorConfigurationState::Ready(1_u64.into())
        );
    }

    #[test]
    fn registration_failure_releases_context() {
        let fake = FakeCoreGraphics::new(CORE_GRAPHICS_FAILURE, CORE_GRAPHICS_SUCCESS);
        let tracker = Arc::new(MonitorConfigurationGenerationTracker::default());
        let tracker_weak = Arc::downgrade(&tracker);

        let result = DisplayReconfigurationRegistration::register(fake, tracker);

        assert!(result.is_err());
        assert!(tracker_weak.upgrade().is_none());
    }

    #[test]
    fn successful_removal_releases_matching_context() {
        let fake = FakeCoreGraphics::new(CORE_GRAPHICS_SUCCESS, CORE_GRAPHICS_SUCCESS);
        let tracker = Arc::new(MonitorConfigurationGenerationTracker::default());
        let tracker_weak = Arc::downgrade(&tracker);
        let mut registration =
            DisplayReconfigurationRegistration::register(fake.clone(), Arc::clone(&tracker))
                .expect("fake registration should succeed");
        drop(tracker);
        let registered = fake.registered_entry();

        registration
            .unregister()
            .expect("fake removal should succeed");

        assert_eq!(fake.removed_entry(), registered);
        assert!(tracker_weak.upgrade().is_none());
    }

    #[test]
    fn failed_removal_quarantines_live_context_until_fake_unregisters() {
        let fake = FakeCoreGraphics::new(CORE_GRAPHICS_SUCCESS, CORE_GRAPHICS_FAILURE);
        let tracker = Arc::new(MonitorConfigurationGenerationTracker::default());
        let tracker_weak = Arc::downgrade(&tracker);
        let mut registration =
            DisplayReconfigurationRegistration::register(fake.clone(), Arc::clone(&tracker))
                .expect("fake registration should succeed");
        drop(tracker);
        let registered = fake.registered_entry();

        let error = registration
            .unregister()
            .expect_err("scripted removal should fail");

        assert_eq!(
            error,
            OperatingSystemQueryError::ConfigurationNotificationRemoval.into()
        );
        assert!(tracker_weak.upgrade().is_some());
        assert!(
            display_reconfiguration_quarantine()
                .reclaim_if_removed(registered.user_info(), fake.registration_state())
                .is_none()
        );

        fake.invoke_registered_callback(0);
        assert_eq!(
            tracker_weak
                .upgrade()
                .expect("quarantined tracker should remain alive")
                .state(),
            MonitorConfigurationState::Ready(1_u64.into())
        );

        fake.declare_unregistered();
        let context = display_reconfiguration_quarantine()
            .reclaim_if_removed(registered.user_info(), fake.registration_state())
            .expect("fake registration is removed, so its context is reclaimable");
        drop(context);
        assert!(tracker_weak.upgrade().is_none());
    }

    #[derive(Clone)]
    struct FakeCoreGraphics {
        state: Arc<Mutex<FakeCoreGraphicsState>>,
    }

    impl FakeCoreGraphics {
        fn new(registration_result: i32, removal_result: i32) -> Self {
            Self {
                state: Arc::new(Mutex::new(FakeCoreGraphicsState {
                    registration_result,
                    removal_result,
                    ..Default::default()
                })),
            }
        }

        fn registered_entry(&self) -> FakeCallbackEntry {
            self.state
                .lock()
                .expect("fake Core Graphics state should not be poisoned")
                .registered
                .expect("fake should contain a registered callback")
        }

        fn removed_entry(&self) -> FakeCallbackEntry {
            self.state
                .lock()
                .expect("fake Core Graphics state should not be poisoned")
                .removed
                .expect("fake should contain a removed callback")
        }

        fn registration_state(&self) -> TestRegistrationState {
            self.state
                .lock()
                .expect("fake Core Graphics state should not be poisoned")
                .registration_state
        }

        fn invoke_registered_callback(&self, flags: u32) {
            let entry = self.registered_entry();
            let Some(callback) = entry.callback else {
                return;
            };
            unsafe { callback(0, flags, entry.user_info()) };
        }

        fn declare_unregistered(&self) {
            self.state
                .lock()
                .expect("fake Core Graphics state should not be poisoned")
                .registration_state = TestRegistrationState::Removed;
        }
    }

    impl CoreGraphicsApi for FakeCoreGraphics {
        fn register(
            &self,
            callback: CoreGraphicsDisplayReconfigurationCallback,
            user_info: *mut c_void,
        ) -> i32 {
            let mut state = self
                .state
                .lock()
                .expect("fake Core Graphics state should not be poisoned");
            state.registered = Some(FakeCallbackEntry::new(callback, user_info));
            state.registration_state = if state.registration_result == CORE_GRAPHICS_SUCCESS {
                TestRegistrationState::Registered
            } else {
                TestRegistrationState::Removed
            };
            state.registration_result
        }

        fn remove(
            &self,
            callback: CoreGraphicsDisplayReconfigurationCallback,
            user_info: *mut c_void,
        ) -> i32 {
            let mut state = self
                .state
                .lock()
                .expect("fake Core Graphics state should not be poisoned");
            state.removed = Some(FakeCallbackEntry::new(callback, user_info));
            if state.removal_result == CORE_GRAPHICS_SUCCESS {
                state.registration_state = TestRegistrationState::Removed;
            }
            state.removal_result
        }
    }

    #[derive(Clone, Copy)]
    struct FakeCallbackEntry {
        callback:  CoreGraphicsDisplayReconfigurationCallback,
        user_info: usize,
    }

    impl FakeCallbackEntry {
        fn new(
            callback: CoreGraphicsDisplayReconfigurationCallback,
            user_info: *mut c_void,
        ) -> Self {
            Self {
                callback,
                user_info: user_info.expose_provenance(),
            }
        }

        fn user_info(self) -> *mut c_void { std::ptr::with_exposed_provenance_mut(self.user_info) }
    }

    impl PartialEq for FakeCallbackEntry {
        fn eq(&self, other: &Self) -> bool {
            callbacks_equal(self.callback, other.callback) && self.user_info == other.user_info
        }
    }

    impl Eq for FakeCallbackEntry {}

    impl std::fmt::Debug for FakeCallbackEntry {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("FakeCallbackEntry")
                .field("has_callback", &self.callback.is_some())
                .field("user_info", &self.user_info)
                .finish()
        }
    }

    fn callbacks_equal(
        left: CoreGraphicsDisplayReconfigurationCallback,
        right: CoreGraphicsDisplayReconfigurationCallback,
    ) -> bool {
        match (left, right) {
            (Some(left), Some(right)) => std::ptr::fn_addr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
    }

    #[derive(Default)]
    struct FakeCoreGraphicsState {
        registered:          Option<FakeCallbackEntry>,
        registration_result: i32,
        registration_state:  TestRegistrationState,
        removal_result:      i32,
        removed:             Option<FakeCallbackEntry>,
    }
}
