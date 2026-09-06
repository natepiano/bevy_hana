use std::time::Duration;

use bevy::platform::time::Instant;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::reflect::ReflectSerialize;
use serde::Serialize;
use thiserror::Error;

use crate::DeviceAccessError;
use crate::DriverContractError;
use crate::DriverId;
use crate::RetryOn;
use crate::RoleKey;
use crate::devices::DeviceRevisionLookup;
use crate::reconcile::FrameClockReading;

/// Public diagnostic identity for one process-local apply attempt.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize)]
pub struct AttemptRef(u64);

impl AttemptRef {
    /// Wrap the attempt registry's next counter value.
    ///
    /// Private to the crate because only `crate::Attempts` issues identifiers; a driver that could
    /// fabricate one would be claiming an authorization the kernel never granted.
    pub(crate) const fn new(value: u64) -> Self { Self(value) }

    /// Return the attempt registry's process-local number.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

/// Terminal result reported through one typed attempt completion.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum DriverOutcomeStatus {
    /// The driver established a configuration and named its relation to the dispatched value.
    Succeeded(crate::AppliedKind),
    /// The device or platform rejected continued access.
    Failed(DeviceAccessError),
    /// The driver ended a started operation for a typed client-owned reason.
    Aborted(crate::DriverAbortReason),
}

/// Why an in-flight attempt may not continue.
///
/// Named rather than a flag because the reasons finish the attempt differently: a revision advance
/// authorizes a successor against the new revision, while a role that no longer exists has no
/// binding left to move. Only `Self::RevisionAdvanced` consults `crate::OnAbort` — a lost claim
/// makes reversion impossible and a deferred veto makes it unsafe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum AttemptInvalidation {
    /// The device the attempt was authorized against is no longer the one the endpoint resolves to,
    /// so continuing would land an authorized configuration on a different unit.
    DeviceChanged,
    /// This device's own reconciled state moved under the attempt in a way no other check names —
    /// its endpoint set, what its reporters disagree about, where it hangs. Narrow by construction:
    /// every reason with a consequence of its own is a variant of its own, and this one is what is
    /// left. It is also the only reason that consults `crate::OnAbort`.
    RevisionAdvanced,
    /// The device's `crate::Claim` stopped permitting this process to use it, so whatever the
    /// attempt was doing has already stopped reaching the hardware.
    ClaimLost,
    /// The device's `crate::IdentityVerdict` no longer identifies the unit the attempt was
    /// authorized against, so continuing would drive hardware the kernel can no longer name.
    IdentityNoLongerConfirmed,
    /// The device stopped being `crate::Presence::Present`. Checked ahead of
    /// `Self::RevisionAdvanced` so a freshness lease expiring reads as a departure: the lease
    /// rewrites presence with no scan behind it, and calling that an ordinary state move would send
    /// whoever reads the ending looking for a change no reporter made.
    DeviceNotPresent,
    /// The device is still present, but the application switched its authored inventory entry to
    /// `crate::ConfiguredDeviceMode::Offline`, so no driver operation may touch it. Separate from
    /// `Self::DeviceNotPresent` because nothing about the hardware changed: reporting a withdrawal
    /// as a departure would send whoever reads the ending looking for a cable.
    InventoryWithdrewTheDevice,
    /// The attempt ran past `deadline + crate::RiggingLimits::apply_overrun`, so the kernel stopped
    /// asking a device that was never going to converge.
    OverrunExhausted,
    /// The application retired the role while its attempt was in flight, so there is no binding
    /// left for the ending to move.
    RoleRetired,
    /// The role's binding was replaced while this attempt was in flight: the attempt's stamped
    /// `crate::BindingGeneration` no longer matches the installed binding's. The replacement's own
    /// dispatch carries the new generation, so it can never select itself here.
    BindingReplaced,
    /// The erased driver boundary failed before any typed [`EndpointDriver`](crate::EndpointDriver)
    /// method could run for this attempt: the downcast found no registered driver for the id, or a
    /// driver or configuration type that differs from the one the dispatch functions were built
    /// for. No driver callback ran, so nothing was started that needs undoing.
    DriverContractFailed,
}

/// The most recent terminal result for a role, mirrored onto its binding entity.
///
/// Each variant admits one producer: a driver reports a [`DriverOutcomeStatus`], the kernel
/// supplies an [`AttemptInvalidation`], and erased dispatch supplies a
/// [`DriverContractFailureReport`]. Keeping those producers separate prevents a reported ending
/// from carrying an invalidation and prevents an invalidated ending from omitting its reason.
#[derive(Component, Clone, PartialEq, Eq, Debug, Reflect)]
#[reflect(Component, PartialEq)]
pub enum AttemptEnding {
    /// The endpoint driver reported this terminal outcome.
    Reported(DriverOutcomeStatus),
    /// Kernel validation ended the attempt for this reason before another driver poll.
    Invalidated(AttemptInvalidation),
    /// The erased driver boundary could not complete the typed driver call.
    ContractFailed(DriverContractFailureReport),
}

/// Data-only report of one erased driver contract failure.
///
/// The report owns all diagnostic text so it can be retained in [`AttemptEnding`] without
/// borrowing the driver registry or carrying a type-erased error value.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum DriverContractFailureReport {
    /// No endpoint driver registration owns the process-local route.
    DriverNotRegistered {
        /// Route that failed to select a registered driver.
        driver: DriverId,
    },
    /// The erased registry entry did not contain the concrete driver type its functions require.
    DriverTypeMismatch {
        /// Concrete driver type required by the erased function.
        expected_driver: String,
    },
    /// The retained configuration did not match the registered driver's configuration type.
    ConfigurationTypeMismatch {
        /// Concrete configuration type the driver accepts.
        expected_configuration: String,
        /// Reflected type path of the retained value.
        received_configuration: String,
    },
    /// A restore request no longer had its checked last-known-good value.
    LastKnownGoodConfigurationUnavailable {
        /// Role whose restore value was unavailable.
        role: RoleKey,
    },
}

impl From<DriverContractError> for DriverContractFailureReport {
    fn from(error: DriverContractError) -> Self {
        match error {
            DriverContractError::DriverNotRegistered { driver_id } => {
                Self::DriverNotRegistered { driver: driver_id }
            },
            DriverContractError::DriverTypeMismatch { expected_driver } => {
                Self::DriverTypeMismatch {
                    expected_driver: expected_driver.to_owned(),
                }
            },
            DriverContractError::ConfigurationTypeMismatch {
                expected_configuration,
                received_configuration,
            } => Self::ConfigurationTypeMismatch {
                expected_configuration: expected_configuration.to_owned(),
                received_configuration,
            },
            DriverContractError::LastKnownGoodConfigurationUnavailable { role } => {
                Self::LastKnownGoodConfigurationUnavailable { role }
            },
        }
    }
}

/// Where one attempt stands against its own deadline and the kernel's bounded overrun budget.
///
/// A named result rather than an overdue `bool`: a bare flag collapses "still inside its deadline",
/// "overdue but still converging", and "no such attempt" into one bit, and the abort branch cannot
/// tell a healthy attempt from a handle that names nothing. Being overdue is neither an error nor a
/// failure, so the two overdue variants are separate states rather than one error case.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
enum AttemptDeadlineStatus {
    /// The registry retains no attempt for this identifier, so it either finished or was never
    /// issued. A caller that read this as "not overdue" would keep polling a driver forever.
    NoSuchAttempt,
    /// The attempt is inside its own deadline, or the real-time clock has not advanced past
    /// application startup and no elapsed time exists to judge it against.
    WithinDeadline,
    /// The attempt passed its deadline and is still inside `crate::RiggingLimits::apply_overrun`.
    /// The kernel keeps polling through that budget, so a projector still converging can finish.
    OverdueWithinOverrun {
        /// How far past its own deadline the attempt has run, which is the reading a diagnostic
        /// needs to tell a device that is nearly there from one that has barely started.
        past_deadline: Duration,
    },
    /// The attempt passed `deadline + crate::RiggingLimits::apply_overrun`, so the kernel stops
    /// asking and records [`AttemptInvalidation::OverrunExhausted`].
    OverrunExhausted {
        /// How far past its own deadline the attempt ran before the kernel abandoned it.
        past_deadline: Duration,
    },
}

/// What one role is waiting for before another apply may be dispatched after a failure.
///
/// Stored rather than recomputed from `crate::RetryOn` at each dispatch, because the two paced
/// policies measure different things: one counts completed scans and the other counts real time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryGate {
    /// `crate::RetryOn::NewRevision` — no attempt runs until this role's own device changes, so a
    /// permanently unavailable display cannot issue attempts against an unchanged report while a
    /// scan naming some other device cannot wake it either.
    ///
    /// The whole reading is stamped, not just the counter: a key that leaves the reported set and
    /// returns receives a freshly issued handle whose revision restarts, so an ordering comparison
    /// would leave the gate shut for the rest of the process. Two readings that differ is what
    /// "this device changed" means, and a departure and a return each produce one.
    AwaitingRevision(DeviceRevisionLookup),
    /// `crate::RetryOn::Interval` — no attempt runs before this instant, so a camera another
    /// application holds open is retried at a cadence rather than at frame rate.
    AwaitingInstant(Instant),
}

impl RetryGate {
    /// Build the gate one failed role waits behind from its authored retry policy.
    ///
    /// An interval policy on a frame whose real-time clock has not advanced falls back to the
    /// revision gate: with no clock reading there is no instant to wait until, and waiting for the
    /// device's next change is the conservative pacing of the two.
    pub(crate) fn from_policy(
        retry: RetryOn,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) -> Self {
        match (retry, now) {
            (RetryOn::Interval(interval), FrameClockReading::Measurable(now)) => {
                Self::AwaitingInstant(now + interval)
            },
            (RetryOn::Interval(_) | RetryOn::NewRevision, _) => {
                Self::AwaitingRevision(device_revision)
            },
        }
    }

    /// Report whether the paced wait this gate describes has elapsed.
    pub(crate) fn opened(
        self,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) -> bool {
        match self {
            Self::AwaitingRevision(failed_at) => device_revision != failed_at,
            Self::AwaitingInstant(retry_at) => match now {
                FrameClockReading::Measurable(now) => now >= retry_at,
                FrameClockReading::NotYetAdvanced => false,
            },
        }
    }
}

/// The most recent configuration established by an accepted driver completion.
#[derive(Default, Reflect)]
pub enum LastKnownGoodConfiguration {
    /// No accepted success has established what is on this endpoint.
    #[default]
    NotEstablished,
    /// The accepted value matches the application-authored request.
    MatchesRequested,
    /// The driver established this value instead of the dispatched configuration.
    DiffersFromDispatched(
        #[reflect(ignore, default = "default_erased_configuration")] Box<dyn Reflect>,
    ),
}

impl LastKnownGoodConfiguration {
    pub(crate) const fn is_established(&self) -> bool { !matches!(self, Self::NotEstablished) }

    pub(crate) fn as_reflect<'a>(
        &'a self,
        requested: &'a dyn Reflect,
    ) -> Result<&'a dyn Reflect, LastKnownGoodConfigurationAccessError> {
        match self {
            Self::NotEstablished => Err(LastKnownGoodConfigurationAccessError::NotEstablished),
            Self::MatchesRequested => Ok(requested),
            Self::DiffersFromDispatched(configuration) => Ok(configuration.as_ref()),
        }
    }
}

/// Reason erased dispatch could not borrow a readback value from lifecycle state.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LastKnownGoodConfigurationAccessError {
    /// No accepted success established an endpoint value for the binding yet.
    #[error("no accepted driver success has established a configuration")]
    NotEstablished,
}

fn default_erased_configuration() -> Box<dyn Reflect> { Box::new(()) }

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::prelude::Reflect;
    use bevy::reflect::FromReflect;
    use bevy::reflect::ReflectFromReflect;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;

    use super::AttemptEnding;
    use super::AttemptRef;
    use super::LastKnownGoodConfiguration;

    #[derive(Debug, PartialEq, Eq, Reflect)]
    struct ProviderConfiguration {
        frame_rate: u32,
    }

    #[derive(Reflect)]
    struct LastKnownGoodConfigurationRecord {
        last_known_good_configuration: LastKnownGoodConfiguration,
    }

    /// The component exists for Bevy Remote Protocol diagnosis, so its registration is the
    /// behavior under test: an unregistered component would compile and insert but answer no
    /// remote query.
    #[test]
    fn the_last_attempt_ending_component_registers_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();
        let type_id = TypeId::of::<AttemptEnding>();

        assert!(type_registry.contains(type_id));
        assert!(
            type_registry
                .get_type_data::<ReflectComponent>(type_id)
                .is_some()
        );

        drop(type_registry);
    }

    #[test]
    fn last_known_good_configuration_allows_its_enclosing_record_to_reflect() {
        fn assert_from_reflect<T: FromReflect>() {}

        assert_from_reflect::<LastKnownGoodConfigurationRecord>();

        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();
        let type_id = TypeId::of::<LastKnownGoodConfigurationRecord>();

        assert!(type_registry.contains(type_id));
        assert!(
            type_registry
                .get_type_data::<ReflectFromReflect>(type_id)
                .is_some()
        );

        drop(type_registry);
    }

    #[test]
    fn differing_configuration_recovers_the_provider_value_after_erasure() {
        let last_known_good =
            LastKnownGoodConfiguration::DiffersFromDispatched(Box::new(ProviderConfiguration {
                frame_rate: 60,
            }));

        let requested = ProviderConfiguration { frame_rate: 30 };
        let recovered = last_known_good
            .as_reflect(&requested)
            .ok()
            .and_then(|configuration| {
                configuration
                    .as_any()
                    .downcast_ref::<ProviderConfiguration>()
            });

        assert_eq!(recovered, Some(&ProviderConfiguration { frame_rate: 60 }));
    }

    #[test]
    fn last_known_good_configuration_defaults_to_not_established() {
        assert!(matches!(
            LastKnownGoodConfiguration::default(),
            LastKnownGoodConfiguration::NotEstablished
        ));
    }

    #[test]
    fn runtime_reflection_cannot_construct_an_attempt_reference_the_registry_never_issued() {
        let mut dynamic_attempt = DynamicTupleStruct::default();
        dynamic_attempt.insert(7_u64);

        assert!(AttemptRef::from_reflect(&dynamic_attempt).is_none());
    }
}
