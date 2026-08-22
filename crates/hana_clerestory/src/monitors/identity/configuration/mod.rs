mod constants;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
mod windows_device_notification;
#[cfg(any(test, target_os = "windows"))]
mod windows_listener;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;

use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use bevy::prelude::Resource;
use bevy::prelude::error;

#[cfg(target_os = "macos")]
use self::macos::MacOsConfigurationNotification;
#[cfg(target_os = "windows")]
use self::windows::WindowsConfigurationNotification;
#[cfg(all(unix, not(target_os = "macos")))]
use self::x11::X11ConfigurationNotification;
use super::MonitorIdentificationError;
#[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
use super::OperatingSystemQueryError;
use crate::Platform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::monitors) struct MonitorConfigurationGeneration(u64);

impl From<u64> for MonitorConfigurationGeneration {
    fn from(value: u64) -> Self { Self(value) }
}

impl MonitorConfigurationGeneration {
    #[cfg(feature = "monitor-probe")]
    pub const fn get(self) -> u64 { self.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::monitors) enum MonitorConfigurationState {
    Ready(MonitorConfigurationGeneration),
    Unavailable(MonitorIdentificationError),
}

#[cfg(feature = "monitor-probe")]
impl MonitorConfigurationState {
    pub const fn probe_fields(self) -> (&'static str, Option<u64>) {
        match self {
            Self::Ready(generation) => ("ready", Some(generation.get())),
            Self::Unavailable(_) => ("unavailable", None),
        }
    }
}

struct MonitorConfigurationGenerationTracker {
    generation: AtomicU64,
    status:     AtomicU8,
}

impl Default for MonitorConfigurationGenerationTracker {
    fn default() -> Self {
        Self {
            generation: AtomicU64::default(),
            status:     AtomicU8::new(ConfigurationTrackerStatus::Ready.to_raw()),
        }
    }
}

impl MonitorConfigurationGenerationTracker {
    pub(super) fn advance(&self) {
        if self
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1)
            })
            .is_err()
        {
            self.status.store(
                ConfigurationTrackerStatus::GenerationExhausted.to_raw(),
                Ordering::Release,
            );
        }
    }

    #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
    pub(super) fn fail_notification_stream(&self) {
        let _ = self.status.compare_exchange(
            ConfigurationTrackerStatus::Ready.to_raw(),
            ConfigurationTrackerStatus::NotificationFailed.to_raw(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn state(&self) -> MonitorConfigurationState {
        match ConfigurationTrackerStatus::from_raw(self.status.load(Ordering::Acquire)) {
            ConfigurationTrackerStatus::GenerationExhausted => {
                MonitorConfigurationState::Unavailable(
                    MonitorIdentificationError::ConfigurationGenerationExhausted,
                )
            },
            #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
            ConfigurationTrackerStatus::NotificationFailed => {
                MonitorConfigurationState::Unavailable(
                    OperatingSystemQueryError::ConfigurationNotificationStream.into(),
                )
            },
            ConfigurationTrackerStatus::Ready => {
                MonitorConfigurationState::Ready(self.generation.load(Ordering::Acquire).into())
            },
        }
    }
}

#[derive(Clone, Copy)]
enum ConfigurationTrackerStatus {
    Ready,
    GenerationExhausted,
    #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
    NotificationFailed,
}

impl ConfigurationTrackerStatus {
    const GENERATION_EXHAUSTED: u8 = 1;
    #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
    const NOTIFICATION_FAILED: u8 = 2;
    const READY: u8 = 0;

    const fn from_raw(raw: u8) -> Self {
        match raw {
            Self::GENERATION_EXHAUSTED => Self::GenerationExhausted,
            #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
            Self::NOTIFICATION_FAILED => Self::NotificationFailed,
            _ => Self::Ready,
        }
    }

    const fn to_raw(self) -> u8 {
        match self {
            Self::Ready => Self::READY,
            Self::GenerationExhausted => Self::GENERATION_EXHAUSTED,
            #[cfg(any(test, target_os = "windows", all(unix, not(target_os = "macos"))))]
            Self::NotificationFailed => Self::NOTIFICATION_FAILED,
        }
    }
}

#[derive(Resource)]
pub(in crate::monitors) struct MonitorConfiguration {
    tracker:      Arc<MonitorConfigurationGenerationTracker>,
    registration: Result<ConfigurationNotificationRegistration, MonitorIdentificationError>,
}

impl MonitorConfiguration {
    pub(in crate::monitors) fn register(platform: Platform) -> Self {
        let tracker = Arc::new(MonitorConfigurationGenerationTracker::default());
        let registration = ConfigurationNotificationRegistration::register(platform, &tracker);
        if let Err(error) = registration {
            error!("[MonitorConfiguration] stable monitor identity disabled: {error}");
        }
        Self {
            tracker,
            registration,
        }
    }

    pub(in crate::monitors) fn state(&self) -> MonitorConfigurationState {
        self.registration.as_ref().map_or_else(
            |error| MonitorConfigurationState::Unavailable(*error),
            |_| self.tracker.state(),
        )
    }
}

enum ConfigurationNotificationRegistration {
    #[cfg(target_os = "macos")]
    MacOs(MacOsConfigurationNotification),
    #[cfg(target_os = "windows")]
    Windows(WindowsConfigurationNotification),
    #[cfg(all(unix, not(target_os = "macos")))]
    X11(X11ConfigurationNotification),
    Wayland,
}

impl ConfigurationNotificationRegistration {
    fn register(
        platform: Platform,
        tracker: &Arc<MonitorConfigurationGenerationTracker>,
    ) -> Result<Self, MonitorIdentificationError> {
        match platform {
            #[cfg(target_os = "macos")]
            Platform::MacOs => {
                MacOsConfigurationNotification::register(Arc::clone(tracker)).map(Self::MacOs)
            },
            #[cfg(target_os = "windows")]
            Platform::Windows => {
                WindowsConfigurationNotification::register(Arc::clone(tracker)).map(Self::Windows)
            },
            #[cfg(all(unix, not(target_os = "macos")))]
            Platform::X11 => {
                X11ConfigurationNotification::register(Arc::clone(tracker)).map(Self::X11)
            },
            Platform::Wayland => Ok(Self::Wayland),
            _ => Err(MonitorIdentificationError::StablePhysicalIdentityUnavailable),
        }
    }
}

impl Drop for ConfigurationNotificationRegistration {
    fn drop(&mut self) {
        match self {
            #[cfg(target_os = "macos")]
            Self::MacOs(notification) => notification.unregister(),
            #[cfg(target_os = "windows")]
            Self::Windows(notification) => notification.unregister(),
            #[cfg(all(unix, not(target_os = "macos")))]
            Self::X11(notification) => notification.unregister(),
            Self::Wayland => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_generation_advances_monotonically() {
        let tracker = MonitorConfigurationGenerationTracker::default();

        assert_eq!(
            tracker.state(),
            MonitorConfigurationState::Ready(0_u64.into())
        );
        tracker.advance();
        tracker.advance();

        assert_eq!(
            tracker.state(),
            MonitorConfigurationState::Ready(2_u64.into())
        );
    }

    #[test]
    fn configuration_generation_exhaustion_is_checked() {
        let tracker = MonitorConfigurationGenerationTracker {
            generation: AtomicU64::new(u64::MAX),
            ..Default::default()
        };

        tracker.advance();

        assert_eq!(
            tracker.state(),
            MonitorConfigurationState::Unavailable(
                MonitorIdentificationError::ConfigurationGenerationExhausted
            )
        );
    }
}
