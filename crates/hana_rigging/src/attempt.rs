use std::time::Duration;

use bevy::platform::time::Instant;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;

use crate::ApplyPermit;
use crate::BindingGeneration;
use crate::DeviceAccessError;
use crate::DeviceEndpoint;
use crate::DeviceId;
use crate::DeviceRevision;
use crate::DeviceRevisionLookup;
use crate::RetryOn;
use crate::RoleKey;
use crate::reconcile::FrameClockReading;

/// Identifier issued from the attempt registry's monotonic counter.
///
/// An `AttemptId` is never reused while the process runs, so a delayed provider poll cannot
/// resolve a later attempt after the original attempt has finished.
///
/// Reflection sees the counter value opaquely, so a dynamic tuple struct cannot construct an
/// identifier the registry never issued: a reflected poll for a fabricated attempt would otherwise
/// resolve against an in-flight record belonging to a different role.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Reflect)]
#[reflect(opaque)]
pub struct AttemptId(u64);

impl AttemptId {
    /// Wrap the attempt registry's next counter value.
    ///
    /// Private to the crate because only `crate::Attempts` issues identifiers; a driver that could
    /// mint one would be claiming an authorization the kernel never granted.
    pub(crate) const fn new(value: u64) -> Self { Self(value) }

    /// Read back the counter value this identifier wraps.
    ///
    /// Crate-private and used by `crate::Attempts` alone, so the registry can reclaim an identifier
    /// it minted for a dispatch that never committed. Nothing outside the registry has a reason to
    /// see the number: comparing identifiers is what every other caller does.
    pub(crate) const fn value(self) -> u64 { self.0 }
}

/// One in-flight endpoint operation and the authorization it was started under.
///
/// The record exists so every poll can re-answer "is this still the unit the kernel authorized?"
/// without trusting the driver's own bookkeeping. Both re-checked fields are copied in at
/// authorization time rather than read live, because a driver that reported against a replaced unit
/// would otherwise look correct.
#[derive(Clone, Debug, Reflect)]
pub struct Attempt {
    /// Registry-issued identifier the driver echoes back on every poll and completion.
    pub id:                 AttemptId,
    /// Application role this operation is running for, so a completion reaches the binding entity
    /// that outlives the device.
    pub role:               RoleKey,
    /// Durable endpoint the operation addresses, retained so a poll can be compared with current
    /// resolution rather than with whatever the driver believes it opened.
    pub endpoint:           DeviceEndpoint,
    /// The generation of the binding this attempt was dispatched for, stamped at authorization
    /// time like the fields above. An attempt ending is only allowed to touch the binding
    /// generation named here: `crate::Bindings::record_attempt_ending` refuses a stale ending, so
    /// an attempt that outlives its binding cannot gate or fail-count the replacement.
    pub binding_generation: BindingGeneration,
    /// The authorisation this attempt was minted under. The kernel mints one value and both stores
    /// it here and hands it to the driver, so there is no second copy that could disagree.
    pub permit:             ApplyPermit,
    /// The identity this attempt was authorised against.
    /// Re-checked on every poll; a mismatch invalidates it.
    pub expected_device_id: DeviceId,
    /// This device's own revision at authorisation. A newer one invalidates.
    ///
    /// Per device rather than `crate::RiggingRevision`: the global counter advances on every pass
    /// in which any reporter completed a scan, so validating against it would let one
    /// reporter's routine scan abandon every attempt in the kernel, including attempts on
    /// devices that reporter never names.
    pub device_revision:    DeviceRevision,
    /// Bounds the attempt end to end, not per step.
    ///
    /// `bevy_platform::time::Instant` rather than `std::time::Instant` because this record is
    /// reflected and only the bevy type carries a `Reflect` impl; on native targets it is a
    /// re-export of the std type, so call sites are unchanged.
    pub deadline:           Instant,
}

/// Progress returned by a provider while the kernel polls an attempt.
///
/// Splitting the in-progress state from `AttemptOutcome` prevents a completion event from
/// carrying `Pending` as though the attempt had ended.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum AttemptProgress {
    /// The provider has not yet observed the requested configuration at its destination.
    Pending,
    /// The provider observed a terminal result for this attempt.
    Finished(AttemptOutcome),
}

/// Terminal result reported by a provider for one attempt.
///
/// `Aborted` is terminal and does not auto-retry because the kernel uses it when an attempt no
/// longer has authorization to continue, such as after a device departure or a safety gate
/// closes.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum AttemptOutcome {
    /// The device reached the requested configuration, including a provider-defined tolerance.
    Succeeded,
    /// The driver started but the device or platform rejected continued access.
    Failed(DeviceAccessError),
    /// The kernel stopped an attempt that must not continue or retry automatically.
    Aborted,
    /// The provider reached a different device or endpoint than the one the attempt addressed.
    Substituted,
}

/// Why an in-flight attempt may not continue.
///
/// Named rather than a flag because the reasons finish the attempt differently: an abandoned
/// attempt is `AttemptOutcome::Aborted` and never auto-retries, while a role that no longer exists
/// has no binding left to move. Only `Self::RevisionAdvanced` consults `crate::OnAbort` — a lost
/// claim makes reversion impossible and a deferred veto makes it unsafe.
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
}

/// The most recent attempt ending for a role, mirrored onto its binding entity.
///
/// `crate::RoleState` alone cannot separate a role that is healthily waiting from one whose last
/// attempt was aborted behind a retry gate that will never reopen. This component names the
/// ending's outcome and — when the kernel invalidated the attempt — the `AttemptInvalidation`
/// that says why, so a Bevy Remote Protocol query of the binding entity answers the question
/// without instrumenting the kernel. The ended `AttemptId` is deliberately not carried: a runtime
/// identifier registers no serialization, and one unserializable field makes the whole component
/// unreadable over the remote protocol.
#[derive(Component, Clone, PartialEq, Eq, Debug, Reflect)]
#[reflect(Component, PartialEq)]
pub struct LastAttemptEnding {
    /// The terminal result the attempt ended with.
    pub outcome:      AttemptOutcome,
    /// Why the kernel invalidated the attempt, for an `AttemptOutcome::Aborted` the abort pass
    /// produced; `None` for endings a driver reported.
    pub invalidation: Option<AttemptInvalidation>,
}

/// Where one attempt stands against its own deadline and the kernel's bounded overrun budget.
///
/// A named result rather than an overdue `bool`: a bare flag collapses "still inside its deadline",
/// "overdue but still converging", and "no such attempt" into one bit, and the abort branch cannot
/// tell a healthy attempt from a handle that names nothing. Being overdue is neither an error nor a
/// failure, so the two overdue variants are separate states rather than one error case.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
pub(crate) enum AttemptDeadlineStatus {
    /// The registry retains no attempt for this identifier, so it either finished or was never
    /// issued. A caller that read this as "not overdue" would keep polling a driver forever.
    NoSuchAttempt,
    /// The attempt is inside its own deadline, or the real-time clock has not advanced past
    /// application startup and no elapsed time exists to judge it against.
    WithinDeadline,
    /// The attempt passed its deadline and is still inside `crate::RiggingLimits::apply_overrun`.
    /// The kernel keeps polling: a projector that is genuinely converging deserves the slack.
    OverdueWithinOverrun {
        /// How far past its own deadline the attempt has run, which is the reading a diagnostic
        /// needs to tell a device that is nearly there from one that has barely started.
        past_deadline: Duration,
    },
    /// The attempt passed `deadline + crate::RiggingLimits::apply_overrun`, so the kernel stops
    /// asking and finishes it `AttemptOutcome::Aborted`.
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

/// The most recent configuration a safe readback established on one endpoint.
///
/// This enum distinguishes the absence of endpoint evidence from an established driver value.
/// `RequestedConfiguration` is application intent; it must never fill this value after an apply
/// because a driver can normalize or substitute the value that actually reached the hardware.
#[derive(Default, Reflect)]
pub enum LastKnownGoodConfiguration {
    /// No successful safe readback has established what is on this endpoint.
    #[default]
    NotEstablished,
    /// A safe readback established this driver-specific endpoint value.
    Known(#[reflect(ignore, default = "default_erased_configuration")] Box<dyn Reflect>),
}

impl LastKnownGoodConfiguration {
    /// Erase one value returned by a successful safe endpoint readback.
    #[must_use]
    pub fn known(configuration: impl Reflect) -> Self { Self::Known(Box::new(configuration)) }

    /// Report whether both sides hold the same established value, so a repeated readback of an
    /// unchanged endpoint can be dropped instead of rewriting lifecycle state.
    ///
    /// A configuration type whose reflection declines to compare values answers `false`, which
    /// keeps the newer readback.
    pub(crate) fn holds_same_value(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NotEstablished, Self::NotEstablished) => true,
            (Self::Known(held), Self::Known(captured)) => {
                held.reflect_partial_eq(captured.as_partial_reflect()) == Some(true)
            },
            _ => false,
        }
    }

    pub(crate) fn as_reflect(&self) -> Result<&dyn Reflect, LastKnownGoodConfigurationAccessError> {
        match self {
            Self::NotEstablished => Err(LastKnownGoodConfigurationAccessError::NotEstablished),
            Self::Known(configuration) => Ok(configuration.as_ref()),
        }
    }
}

/// Reason erased dispatch could not borrow a readback value from lifecycle state.
pub(crate) enum LastKnownGoodConfigurationAccessError {
    /// No driver readback proved an endpoint value for the binding yet.
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

    use super::AttemptId;
    use super::LastAttemptEnding;
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
        let type_id = TypeId::of::<LastAttemptEnding>();

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
    fn known_configuration_recovers_the_provider_value_after_erasure() {
        let last_known_good =
            LastKnownGoodConfiguration::known(ProviderConfiguration { frame_rate: 60 });

        let recovered = last_known_good.as_reflect().ok().and_then(|configuration| {
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
    fn runtime_reflection_cannot_construct_an_attempt_identifier_the_registry_never_issued() {
        let mut dynamic_attempt = DynamicTupleStruct::default();
        dynamic_attempt.insert(7_u64);

        assert!(AttemptId::from_reflect(&dynamic_attempt).is_none());
    }
}
