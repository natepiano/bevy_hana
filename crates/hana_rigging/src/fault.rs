use bevy::prelude::Reflect;

/// What the kernel does about a device failure during apply, decided by its class alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ApplyFailureDisposition {
    /// Reconciliation re-answers without retry pacing, spending retry budget, or counting a
    /// failure.
    Reconsider,
    /// Retry behind the pacing gate without counting a failure against the device.
    AwaitClearance,
    /// Count the failure, pace the retry, and surface the fault.
    Fault,
    /// End driver work without scheduling another apply.
    Stop(DriverStopReason),
}

/// Why a driver failure ends apply work without retrying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverStopReason {
    /// The driver cannot perform this operation on the current platform.
    Unsupported {
        /// Provider-supplied text naming the unsupported platform contract.
        detail: String,
    },
}

/// A classified device-access failure from discovery, capture, apply, or an established session.
///
/// [`DeviceAccessError`] keeps the kernel-readable class separate from the provider detail, so
/// retry policy can distinguish a contended camera from a transport failure without discarding
/// the operating-system message needed for diagnostics.
///
/// New hardware can bring failure classifications this crate cannot enumerate in advance, so
/// downstream matches must carry a wildcard arm and adding a variant must not be a breaking change.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
#[non_exhaustive]
pub enum DeviceAccessError {
    /// Another owner holds exclusive access, such as a camera open by a video-conferencing
    /// application.
    Contended {
        /// Provider-supplied text that identifies the contending operation or platform response.
        detail: String,
    },
    /// The operating system or device policy refuses access, such as a USB accessory denied by a
    /// platform permission gate.
    Blocked {
        /// Provider-supplied text that identifies the permission or policy response.
        detail: String,
    },
    /// The device is not reachable for the authorized operation, such as a disconnected display.
    Absent {
        /// Provider-supplied text that identifies the departure or missing-device response.
        detail: String,
    },
    /// Communication with the device or its provider failed for a reason outside the other
    /// access classes, such as a HID transport error.
    Transport {
        /// Provider-supplied text that identifies the transport or protocol response.
        detail: String,
    },
    /// The current platform has no implementation that can perform this operation.
    Unsupported {
        /// Provider-supplied text naming the unsupported platform contract.
        detail: String,
    },
}

impl DeviceAccessError {
    /// Report the [`ApplyFailureDisposition`] the kernel uses when this failure occurs during
    /// apply.
    #[must_use]
    pub(crate) fn apply_failure_disposition(&self) -> ApplyFailureDisposition {
        match self {
            Self::Contended { .. } | Self::Blocked { .. } => {
                ApplyFailureDisposition::AwaitClearance
            },
            Self::Absent { .. } => ApplyFailureDisposition::Reconsider,
            Self::Transport { .. } => ApplyFailureDisposition::Fault,
            Self::Unsupported { detail } => {
                ApplyFailureDisposition::Stop(DriverStopReason::Unsupported {
                    detail: detail.clone(),
                })
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;

    use super::ApplyFailureDisposition;
    use super::DeviceAccessError;
    use super::DriverStopReason;

    #[test]
    fn contended_apply_failures_await_clearance() {
        assert_eq!(
            DeviceAccessError::Contended {
                detail: String::new(),
            }
            .apply_failure_disposition(),
            ApplyFailureDisposition::AwaitClearance
        );
    }

    #[test]
    fn blocked_apply_failures_await_clearance() {
        assert_eq!(
            DeviceAccessError::Blocked {
                detail: String::new(),
            }
            .apply_failure_disposition(),
            ApplyFailureDisposition::AwaitClearance
        );
    }

    #[test]
    fn absent_apply_failures_reconsider() {
        assert_eq!(
            DeviceAccessError::Absent {
                detail: String::new(),
            }
            .apply_failure_disposition(),
            ApplyFailureDisposition::Reconsider
        );
    }

    #[test]
    fn transport_apply_failures_are_faults() {
        assert_eq!(
            DeviceAccessError::Transport {
                detail: String::new(),
            }
            .apply_failure_disposition(),
            ApplyFailureDisposition::Fault
        );
    }

    #[test]
    fn unsupported_apply_failures_stop_with_the_driver_reason() {
        let detail = String::from("the platform has no device identity contract");

        assert_eq!(
            DeviceAccessError::Unsupported {
                detail: detail.clone(),
            }
            .apply_failure_disposition(),
            ApplyFailureDisposition::Stop(DriverStopReason::Unsupported { detail })
        );
    }

    #[test]
    fn device_access_error_is_a_history_payload_not_a_component() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();
        let type_id = TypeId::of::<DeviceAccessError>();

        assert!(type_registry.contains(type_id));
        assert!(
            type_registry
                .get_type_data::<ReflectComponent>(type_id)
                .is_none()
        );

        drop(type_registry);
    }
}
