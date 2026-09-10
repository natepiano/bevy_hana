use std::any::Any;
use std::any::type_name;
use std::collections::VecDeque;
use std::fmt::Formatter;
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Weak;

use bevy::platform::time::Instant;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::reflect::FromReflect;
use bevy::reflect::TypePath;
use thiserror::Error;

use crate::AttemptInvalidation;
use crate::AttemptRef;
use crate::CapabilityProjectionFailure;
use crate::CapabilityProjectionStatus;
use crate::CapabilityRequirement;
use crate::DeviceAccessError;
use crate::DeviceEndpoint;
use crate::DeviceRef;
use crate::DeviceScan;
use crate::DriverCleanupRoleEntity;
use crate::DriverId;
use crate::PreviousSuccess;
use crate::ReporterHealth;
use crate::ReporterId;
use crate::ReporterOutcomeHealth;
use crate::RoleKey;
use crate::SessionDatumArrivalEvidence;
use crate::SessionRef;
use crate::capabilities::ProjectedCapabilityTypes;
use crate::devices::DeviceRevision;
use crate::registration::ApplyPermit;
use crate::transport::DeviceTransport;
use crate::transport::TransportObservation;

/// Typed process-local registration for one endpoint-driver instance.
///
/// The configuration marker prevents a binding authored for one driver configuration from being
/// routed through a driver that accepts another configuration type. Only
/// [`RiggingAppExt::add_endpoint_driver`](crate::RiggingAppExt::add_endpoint_driver) constructs a
/// registration, so the contained process-local route cannot be fabricated by application code.
pub struct EndpointDriverRegistration<Configuration> {
    driver_id: DriverId,
    marker:    PhantomData<fn() -> Configuration>,
}

impl<Configuration> std::fmt::Debug for EndpointDriverRegistration<Configuration> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("EndpointDriverRegistration")
            .field(&self.driver_id)
            .finish()
    }
}

impl<Configuration> PartialEq for EndpointDriverRegistration<Configuration> {
    fn eq(&self, other: &Self) -> bool { self.driver_id == other.driver_id }
}

impl<Configuration> Eq for EndpointDriverRegistration<Configuration> {}

impl<Configuration> std::hash::Hash for EndpointDriverRegistration<Configuration> {
    fn hash<Hasher>(&self, state: &mut Hasher)
    where
        Hasher: std::hash::Hasher,
    {
        std::hash::Hash::hash(&self.driver_id, state);
    }
}

impl<Configuration> EndpointDriverRegistration<Configuration> {
    pub(crate) const fn new(driver_id: DriverId) -> Self {
        Self {
            driver_id,
            marker: PhantomData,
        }
    }

    pub(crate) const fn driver_id(self) -> DriverId { self.driver_id }
}

impl<Configuration> Clone for EndpointDriverRegistration<Configuration> {
    fn clone(&self) -> Self { *self }
}

impl<Configuration> Copy for EndpointDriverRegistration<Configuration> {}

/// Reports the whole current set of hardware devices from an integration crate.
///
/// This trait is an external extension point: a monitor integration reports display records while
/// a camera integration reports camera records, and the kernel must not name either type. An
/// implementation prepares work on the main thread without receiving the application [`World`],
/// then hands one owned enumeration job to the kernel. Kernel dispatch temporarily removes the
/// reporter registry while [`Self::discover`] and a returned [`MainThreadDiscoveryJob`] run. The
/// main-thread job may read `!Send` integration state, but it must not mutate kernel-owned
/// resources, device entities, binding entities, or their kernel components; returning
/// [`DeviceScan`] is the only reporting path. A successful scan may retain a
/// [`ReportAcceptanceProjection`](crate::ReportAcceptanceProjection) that the kernel invokes only
/// when it accepts that exact result. The projection may mutate only integration-owned state and
/// remains subject to the same prohibition on kernel authority.
///
/// An implementation panic is fatal, and the kernel does not catch it. Continuing after a
/// partially prepared discovery run would leave the reporter's device set and progress partly
/// built, with nothing recording which parts completed.
/// A reporter states what work it wants done; the kernel decides where that
/// work runs. [`Self::discover`] therefore takes no `World`:
///
/// ```
/// use hana_rigging::prelude::DeviceReporter;
/// use hana_rigging::prelude::DeviceScan;
/// use hana_rigging::prelude::DiscoveryJob;
/// use hana_rigging::prelude::DiscoveryWork;
///
/// struct BackgroundReporter;
///
/// impl DeviceReporter for BackgroundReporter {
///     fn discover(&mut self) -> DiscoveryWork {
///         DiscoveryWork::Background(DiscoveryJob::new(|_| DeviceScan::Complete(Vec::new())))
///     }
/// }
/// ```
///
/// A world-taking `discover` would let a reporter reach live state from
/// whichever thread its scan ran on. The implementation above is what keeps the
/// cases below meaningful — a rename would break it loudly rather than leaving
/// these failing for an unrelated reason.
///
/// ```compile_fail,E0050
/// use bevy::prelude::World;
/// use hana_rigging::{DeviceReporter, DiscoveryWork};
///
/// struct Reporter;
///
/// impl DeviceReporter for Reporter {
///     fn discover(&mut self, _: &mut World) -> DiscoveryWork { todo!() }
/// }
/// ```
///
/// The earlier `scan` entry point that did is gone, not deprecated:
///
/// ```compile_fail,E0407
/// use bevy::prelude::World;
/// use hana_rigging::{DeviceReporter, DeviceScan};
///
/// struct Reporter;
///
/// impl DeviceReporter for Reporter {
///     fn scan(&mut self, _: &mut World) -> DeviceScan { DeviceScan::Complete(Vec::new()) }
/// }
/// ```
pub trait DeviceReporter: Send + Sync + 'static {
    /// Prepare one complete discovery run when the kernel schedules this reporter.
    ///
    /// This method runs on the main thread, receives no [`World`], and must return without
    /// blocking. A main-thread-only reporter returns a [`MainThreadDiscoveryJob`] that the kernel
    /// immediately invokes with [`World`]. A reporter that needs a slow probe captures sendable
    /// data it already owns and returns [`DiscoveryWork::Background`].
    fn discover(&mut self) -> DiscoveryWork;
}

/// Work a [`DeviceReporter`] prepared after the kernel selected one discovery opportunity.
pub enum DiscoveryWork {
    /// Owned work the kernel immediately invokes with [`World`] on the main thread.
    Immediate(MainThreadDiscoveryJob),
    /// Owned enumeration that Bevy may run on `IoTaskPool` without access to [`World`].
    Background(DiscoveryJob),
}

/// Owned device enumeration that the kernel immediately runs on the main thread.
///
/// This is the only discovery job that receives the application [`World`]. Its closure may read
/// `!Send` resources and must return an owned whole-set [`DeviceScan`] before scheduler admission
/// continues. The kernel never stores this job in reporter runtime state or submits it to
/// `IoTaskPool`.
pub struct MainThreadDiscoveryJob(Box<dyn FnOnce(&mut World) -> DeviceScan + 'static>);

impl MainThreadDiscoveryJob {
    /// Store one main-thread discovery closure for immediate synchronous execution.
    #[must_use]
    pub fn new(run: impl FnOnce(&mut World) -> DeviceScan + 'static) -> Self { Self(Box::new(run)) }

    pub(crate) fn run(self, world: &mut World) -> DeviceScan { self.0(world) }
}

/// Owned device enumeration that the kernel can move onto Bevy's I/O task pool.
///
/// The closure cannot receive a [`World`], [`Devices`](crate::Devices), bindings, or a reporter
/// identifier. It can only send descriptive progress and return an owned [`DeviceScan`], leaving
/// kernel mutation, accepted integration-state projection, and reporter revision assignment on the
/// main thread.
pub struct DiscoveryJob(
    Mutex<Box<dyn FnOnce(DiscoveryProgressSender) -> DeviceScan + Send + 'static>>,
);

impl DiscoveryJob {
    /// Store one sendable discovery closure for a later `IoTaskPool` submission.
    #[must_use]
    /// The closure runs on an I/O worker, so it is sendable, it receives a
    /// progress sender rather than the world, and it returns an owned scan:
    ///
    /// ```
    /// use hana_rigging::DeviceScan;
    /// use hana_rigging::DiscoveryJob;
    ///
    /// fn job() -> DiscoveryJob { DiscoveryJob::new(|_| DeviceScan::Complete(Vec::new())) }
    /// ```
    ///
    /// The job above is what keeps the cases below meaningful — a rename would
    /// break it loudly rather than leaving these failing for an unrelated
    /// reason.
    ///
    /// ```compile_fail,E0277
    /// use std::rc::Rc;
    ///
    /// use hana_rigging::{DeviceScan, DiscoveryJob};
    ///
    /// let non_send_state = Rc::new(());
    /// let _ = DiscoveryJob::new(move |_| {
    ///     let _ = Rc::strong_count(&non_send_state);
    ///     DeviceScan::Complete(Vec::new())
    /// });
    /// ```
    ///
    /// ```compile_fail,E0631
    /// use bevy::prelude::World;
    /// use hana_rigging::{DeviceScan, DiscoveryJob};
    ///
    /// let _ = DiscoveryJob::new(|_: &mut World| DeviceScan::Complete(Vec::new()));
    /// ```
    ///
    /// ```compile_fail,E0308
    /// use hana_rigging::{DeviceScan, DiscoveryJob};
    ///
    /// let device_scan = DeviceScan::Complete(Vec::new());
    /// let _ = DiscoveryJob::new(move |_| &device_scan);
    /// ```
    pub fn new(run: impl FnOnce(DiscoveryProgressSender) -> DeviceScan + Send + 'static) -> Self {
        Self(Mutex::new(Box::new(run)))
    }

    pub(crate) fn run(self, discovery_progress_sender: DiscoveryProgressSender) -> DeviceScan {
        let run = self
            .0
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        run(discovery_progress_sender)
    }
}

/// Send-only handle a background discovery job uses to describe its current work.
///
/// The sender carries no reporter identity. The scheduler associates each update with the reporter
/// and discovery batch that submitted the job. Unread updates coalesce in one latest-value slot, so
/// sending replaces any progress the scheduler has not collected yet.
#[derive(Clone)]
pub struct DiscoveryProgressSender(Weak<DiscoveryProgressMailbox>);

struct DiscoveryProgressMailbox {
    pending: Mutex<PendingDiscoveryProgress>,
}

pub(crate) struct DiscoveryProgressReceiver(Arc<DiscoveryProgressMailbox>);

pub(crate) enum PendingDiscoveryProgress {
    NoUpdate,
    Latest(DiscoveryProgress),
}

impl DiscoveryProgressSender {
    pub(crate) fn scheduler_mailbox() -> (Self, DiscoveryProgressReceiver) {
        let discovery_progress_mailbox = Arc::new(DiscoveryProgressMailbox {
            pending: Mutex::new(PendingDiscoveryProgress::NoUpdate),
        });
        (
            Self(Arc::downgrade(&discovery_progress_mailbox)),
            DiscoveryProgressReceiver(discovery_progress_mailbox),
        )
    }

    /// Replace any unread progress with the job's latest observed progress.
    ///
    /// Sending uses constant storage and briefly synchronizes on the single-slot mailbox lock
    /// while replacing unread progress. It neither queues earlier updates nor waits for the
    /// scheduler to collect them.
    ///
    /// # Errors
    ///
    /// Returns [`DiscoveryProgressSendError::SchedulerStopped`] after the kernel has stopped
    /// retaining this job, such as during application shutdown.
    pub fn send(
        &self,
        discovery_progress: DiscoveryProgress,
    ) -> Result<(), DiscoveryProgressSendError> {
        let discovery_progress_mailbox = self
            .0
            .upgrade()
            .ok_or(DiscoveryProgressSendError::SchedulerStopped)?;
        *discovery_progress_mailbox
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            PendingDiscoveryProgress::Latest(discovery_progress);

        Ok(())
    }
}

impl DiscoveryProgressReceiver {
    pub(crate) fn take_latest(&self) -> PendingDiscoveryProgress {
        let mut pending = self
            .0
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut *pending, PendingDiscoveryProgress::NoUpdate)
    }
}

/// Progress state a background discovery job can report without exposing a bare optional count.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum DiscoveryProgress {
    /// The job has no count of its remaining device operations.
    Indeterminate,
    /// The job counted device operations, such as opening three of eight camera descriptors.
    Measured {
        /// Number of completed device operations from this discovery run.
        completed: u32,
        /// Total device operations in this discovery run; `NonZeroU32` rules out zero.
        total:     NonZeroU32,
    },
}

/// Failure returned when a background job sends progress after scheduler retention ended.
#[derive(Debug, Error)]
pub enum DiscoveryProgressSendError {
    /// The scheduler released this job's progress receiver during application shutdown.
    #[error("discovery scheduler stopped receiving background progress")]
    SchedulerStopped,
}

/// Drives one endpoint after the kernel authorizes an apply.
///
/// The kernel removes the driver registry during every callback. Driver implementations must not
/// access that registry or mutate kernel-owned device and role components through the supplied
/// [`World`]. A role [`Entity`] in a context is valid only for that callback; drivers retain the
/// [`RoleKey`] because role-entity recovery may replace the entity between callbacks.
///
/// # Author guide
///
/// A device reaches the kernel through two facing halves. The application authors a binding and
/// the policy the kernel applies to it; the driver implements this trait, five thin hardware calls
/// each paired with one verb on a [`DriverLedger`](crate::DriverLedger) it stores in its own
/// state. Neither half names the other's vocabulary: the application never holds a hardware
/// handle, and the driver never reads a policy.
///
/// The worked example at the end of this page is one compiled driver for an imaginary shutter,
/// facing the binding an application would author for it. Both halves are organized as the
/// decisions their author has to make, and every decision below names the answer for a device that
/// has nothing particular to say.
///
/// ## Decisions the application makes
///
/// - **Must data keep arriving?** [`FlowExpectation`](crate::FlowExpectation). `Continuous` has the
///   kernel end a session that goes quiet past its bounds. `NotMonitored` — the answer for a device
///   with no data axis at all, such as a window — means no length of silence ends anything.
/// - **What makes another attempt worth making?** [`RetryOn`](crate::RetryOn). `NewRevision`, the
///   answer whenever the obstacle would be visible in the reported device set, retries only after
///   that set advances. `Interval` is for an obstacle that clears silently, such as another
///   application letting go of a camera.
/// - **How long may one attempt run?** [`ApplyDeadline`](crate::ApplyDeadline). `ProcessDefault`,
///   the answer for a device with no unusual timing, follows the process-wide bound and moves with
///   it. `Authored` pins this role to its own bound and deliberately stays put when the
///   process-wide one is retuned.
/// - **The session died but the device is still here.** [`OnSessionLoss`](crate::OnSessionLoss).
///   `Recreate`, the answer for anything that can simply be reopened, has the kernel start another
///   attempt behind the retry gate. `ReportOnly` stops and waits for an application decision.
/// - **The device left.** [`RecoveryPolicy`](crate::RecoveryPolicy). `Forget`, the answer whenever
///   nothing should come back on its own, discards the saved configuration. `ReapplyOnRequest`
///   keeps it until the application asks for it; `ReapplyOnReturn` puts it back as soon as
///   reconciliation verifies the returning physical unit.
///
/// [`OnAbort`](crate::OnAbort) completes the set and is read only when a revision advance
/// abandons an attempt: `LeaveAsIs` for a device whose partial state is harmless, `Revert` for one
/// that is visibly broken half-applied.
///
/// ## Decisions the driver makes
///
/// Each method is one hardware call and one ledger verb. The ledger holds every authority the
/// kernel issued — the one-use [`AttemptCompletion`], the [`SessionLease`], and the datum evidence
/// retained before any lease exists — so no driver writes the establishment races a second time.
/// When the hardware call needs a stream or handle to survive the callback, the driver names
/// records that keep it inside the ledger beside that authority; the ledger governs the record's
/// lifetime but never inspects its contents. Hardware state whose lifetime is not role-bound, such
/// as a screen image retained per device across sessions, remains in the driver.
///
/// - [`Self::resolve_target`] looks the live handle up. It is the one method holding no authority,
///   so the ledger is only ever read here. A target it cannot reach is
///   [`TargetResolution::Deferred`] naming the state change that would fix it, never a failure —
///   nothing has started yet.
/// - [`Self::start_apply`] starts the work and returns. `begin_attempt` takes the one-use
///   completion off the context, so no driver stores an [`AttemptCompletion`] itself.
/// - [`Self::established`] states the one fact only the driver holds — whether the thing is still
///   live — and `establish_lease` settles every race behind it.
/// - [`Self::cancel_apply`] undoes started work. `cancel_attempt` drops the retained completion;
///   the answer says whether there was anything to undo.
/// - [`Self::release_session`] closes the handle, and only when `release_lease` answers `Released`:
///   any other answer means the named session is not the one this driver still holds.
///
/// Off the trait, wherever the hardware actually answers: `succeed_attempt`, `fail_attempt`, or
/// `abort_attempt` when an attempt ends; `credit_flow` where a datum reaches its consumer;
/// `change_configuration` when the device changes itself; `report_loss` when access ends.
///
/// ### Choosing the failure class
///
/// [`DeviceAccessError`] is the whole of what a driver says about a failure, and its class alone
/// selects what follows.
///
/// - `Contended` and `Blocked` retry behind the pacing gate and count no failure against the
///   device: someone else holds it, or policy refuses, and neither is the device malfunctioning.
/// - `Absent` has reconciliation re-answer. Nothing is paced, nothing is counted, and no retry
///   budget is spent — the device is simply not there to attempt.
/// - `Transport` is counted, paced, and surfaced as a fault; three consecutively stop the role.
/// - `Unsupported` ends driver work with no further attempt scheduled.
///
/// ### Reading the release cause
///
/// [`SessionReleaseCause`] says what the hardware needs, not what the kernel would prefer.
///
/// - `RoleRetired` and `BindingReplaced`: this session is over for good. Close the handle and drop
///   the record.
/// - `ReplacementApply`: the same role is applying again. Close this session's handle but keep
///   whatever the next session inherits, such as a retained image.
/// - `DeviceUnavailable` and `ReportedLoss`: the handle is already dead — the device stopped
///   authorizing work, or this driver reported the loss itself. Only the record needs clearing.
/// - `FlowStalled`: the kernel saw silence past the flow expectation. The handle may well still be
///   open, and closing it is the driver's job here.
///
/// ### What a driver cannot do
///
/// - **Finish an attempt it was not handed.** `AttemptFinish::AttemptUnknown`, and there is no path
///   from a ledger back to an owned [`AttemptCompletion`].
/// - **Finish twice.** [`AttemptCompletion::finish`] consumes the authority, and a failure or abort
///   arriving after a success answers `AttemptFinish::AlreadyFinished` and changes nothing.
///   Hardware answering after the kernel already moved on is the common case, not a defect.
/// - **Testify that a datum arrived.** A driver classifies its own transport through
///   [`DeviceTransport::classify`](crate::DeviceTransport::classify) and nothing more; what that
///   classification permits belongs to the kernel.
/// - **Transition a session.** Establishment, loss, and expiry are the kernel's; the driver reports
///   facts and reads named outcomes.
///
/// The ledger imposes no `Send` bound of its own. A driver whose state must live in a `NonSend`
/// resource — the screen kernel's does — stores one exactly as a `Resource` driver does.
///
/// ```
/// use std::time::Duration;
///
/// use bevy::platform::time::Instant;
/// use bevy::prelude::Component;
/// use bevy::prelude::Entity;
/// use bevy::prelude::Reflect;
/// use bevy::prelude::World;
/// use hana_rigging::prelude::*;
///
/// // ---- The application's half: one binding, and the policy the kernel applies to it. ----
///
/// /// How far open the shutter should be. The driver owns this type; the kernel only reflects it.
/// #[derive(Clone, Component, Reflect)]
/// struct ShutterOpening {
///     percent: u8,
/// }
///
/// fn author_the_house_left_shutter(
///     world: &mut World,
///     driver: EndpointDriverRegistration<ShutterOpening>,
/// ) -> Result<Entity, Box<dyn std::error::Error>> {
///     let policy = BindingPolicy::new(
///         // The unit left: put the saved opening back once the returning one is verified.
///         RecoveryPolicy::ReapplyOnReturn,
///         // Another console can hold this universe, and letting go of it reports nothing.
///         RetryOn::Interval(Duration::from_secs(2)),
///         // A half-moved shutter is not visibly broken, so an abandoned move stays where it is.
///         OnAbort::LeaveAsIs,
///         // The bus dropped with the shutter still present: reopen it without asking.
///         OnSessionLoss::Recreate,
///         // This bus is slower than the process-wide bound and must stay so if that is retuned.
///         ApplyDeadline::Authored(Duration::from_secs(5)),
///     )
///     // Status frames must keep arriving, or the kernel ends the session as stalled.
///     .with_continuous_flow(ContinuousFlowExpectation::new(
///         FirstDatumTimeout::new(Duration::from_secs(3))?,
///         MaximumDatumGap::new(Duration::from_secs(1))?,
///     ));
///
///     let shutter = DeviceKey::reported(
///         DeviceKind::DmxUniverse,
///         SchemeName::new("dmx-artnet")?,
///         ReportedId::new("universe-1")?,
///     );
///
///     Ok(register_binding(
///         world,
///         BindingAuthoring::new(
///             RoleKey::new("shutter:house-left")?,
///             DeviceEndpoint::whole(shutter),
///             driver,
///             ShutterOpening { percent: 0 },
///             policy,
///         ),
///     )?)
/// }
///
/// // ---- The driver's half: five hardware calls, each with one ledger verb. ----
///
/// /// The address this driver drives, resolved fresh before every attempt.
/// struct ShutterAddress;
///
/// /// What one read of the shutter's status bus carried.
/// enum ShutterStatus {
///     Position(u8),
///     KeepAlive,
///     Nothing,
/// }
///
/// /// The one flow judgment this device makes for itself. Everything downstream is the kernel's.
/// struct ShutterBusTransport;
///
/// impl DeviceTransport for ShutterBusTransport {
///     type Observation = ShutterStatus;
///
///     fn classify(observation: &Self::Observation) -> TransportObservation {
///         match observation {
///             ShutterStatus::Position(_) => TransportObservation::DatumDelivered,
///             ShutterStatus::KeepAlive => TransportObservation::ActivityWithoutDatum,
///             ShutterStatus::Nothing => TransportObservation::Quiet,
///         }
///     }
/// }
///
/// /// One open shutter: the driver's own hardware, and the record the ledger carries for it.
/// struct ShutterBus {
///     wanted: ShutterOpening,
/// }
///
/// impl ShutterBus {
///     fn opening_to(_: ShutterAddress, wanted: ShutterOpening) -> Self { Self { wanted } }
///
///     /// Whether the bus is still up: the one fact `established` needs and the kernel cannot see.
///     fn is_live(&self) -> bool { self.wanted.percent <= 100 }
///
///     /// Close the handle for good.
///     fn close(self) {}
///
///     /// Close this session's handle, keeping what the next session inherits.
///     fn close_keeping_retained_frame(self) {}
/// }
///
/// /// What this driver keeps for one attempt and for one session: the bus, either way. Naming
/// /// both records lets the ledger carry the bus through supersede, cancel, refusal and release,
/// /// so the driver keeps no attempt- or role-keyed map of its own and cannot strand a handle.
/// struct ShutterRecords;
///
/// impl DriverRecords for ShutterRecords {
///     type Attempt = ShutterBus;
///     type Session = ShutterBus;
/// }
///
/// /// One ledger is the whole of this driver's bookkeeping: the kernel's authorities and the
/// /// driver's own hardware records, carried over one lifetime instead of two.
/// #[derive(Default)]
/// struct ShutterDriver {
///     ledger: DriverLedger<ShutterOpening, ShutterRecords>,
/// }
///
/// impl EndpointDriver for ShutterDriver {
///     type Configuration = ShutterOpening;
///     type Target = ShutterAddress;
///
///     fn resolve_target(
///         &mut self,
///         _world: &mut World,
///         context: &TargetResolutionContext<'_>,
///         _requested: &Self::Configuration,
///     ) -> TargetResolution<Self::Target> {
///         self.address_on_the_bus(context.endpoint())
///     }
///
///     fn start_apply(
///         &mut self,
///         _world: &mut World,
///         context: ApplyContext<'_, Self::Configuration>,
///         requested: &Self::Configuration,
///         target: Self::Target,
///     ) {
///         // The ledger retains the one-use completion AND the bus this attempt opened, so the
///         // two share one lifetime and cannot drift apart.
///         match self
///             .ledger
///             .begin_attempt(context, ShutterBus::opening_to(target, requested.clone()))
///         {
///             BegunAttempt::Fresh(_) => {},
///             // The kernel abandoned the previous attempt without calling `cancel_apply`. Its
///             // bus is handed back here or it is never reachable again.
///             BegunAttempt::Superseding { displaced, .. } => displaced.close(),
///         }
///     }
///
///     fn established(
///         &mut self,
///         _world: &mut World,
///         context: EstablishedContext<'_, Self::Configuration>,
///     ) {
///         let role = context.role().clone();
///         let closed = DeviceAccessError::Absent {
///             detail: format!("shutter bus for role {role} closed before establishment"),
///         };
///         // The one fact the kernel cannot see, read off the record the ledger is already
///         // holding for this attempt.
///         let hardware = match self.ledger.attempt_record(context.attempt()) {
///             AttemptRecord::Applying(bus) | AttemptRecord::CompletionQueued(bus)
///                 if bus.is_live() =>
///             {
///                 Establishing::Live
///             },
///             AttemptRecord::Applying(_)
///             | AttemptRecord::CompletionQueued(_)
///             | AttemptRecord::AttemptUnknown => Establishing::Ended(closed),
///         };
///         // The attempt's bus becomes the session's bus, unchanged. A driver whose two records
///         // differ narrows one into the other here.
///         match self.ledger.establish_lease(context, hardware, |bus| bus) {
///             Establishment::Established { .. } => {},
///             // The role's previous session was never released. The ledger dropped that lease
///             // unreported and handed back the bus it was still holding for it.
///             Establishment::EstablishedOverUnreleased { predecessor, .. } => predecessor.close(),
///             // A bus deliberately kept past a release; this successor inherits its frame.
///             Establishment::EstablishedOverRetained { retained, .. } => {
///                 retained.close_keeping_retained_frame();
///             },
///             // The ledger already reported on the lease and handed the bus back with the
///             // reason, so the driver can close what it opened.
///             Establishment::Refused(EstablishmentRefusal::SessionEnded { record, .. }) => {
///                 record.close();
///             },
///             // Nothing was queued, so the ledger held no bus for this establishment either.
///             Establishment::Refused(EstablishmentRefusal::NoQueuedCompletion) => {},
///         }
///     }
///
///     fn cancel_apply(
///         &mut self,
///         _world: &mut World,
///         role: &RoleKey,
///         _role_entity: DriverCleanupRoleEntity,
///         attempt: AttemptRef,
///         _cause: AttemptInvalidation,
///     ) {
///         match self.ledger.cancel_attempt(role, attempt) {
///             CancelledAttempt::Applying { record }
///             | CancelledAttempt::CompletionQueued { record } => record.close(),
///             // Another role's attempt, or one this driver never started: touch no hardware.
///             CancelledAttempt::WrongRole | CancelledAttempt::Unknown => {},
///         }
///     }
///
///     fn release_session(
///         &mut self,
///         _world: &mut World,
///         role: &RoleKey,
///         _role_entity: DriverCleanupRoleEntity,
///         session: SessionRef,
///         cause: SessionReleaseCause,
///     ) {
///         // The release moves the bus to the ledger's retained slot rather than handing it back,
///         // so a successor can inherit it. What happens next is entirely about the cause.
///         if !matches!(
///             self.ledger.release_lease(role, session),
///             ReleasedLease::Released
///         ) {
///             return;
///         }
///         match cause {
///             // The same role is applying again: leave the bus retained, and the successor takes
///             // it on `Establishment::EstablishedOverRetained`.
///             SessionReleaseCause::ReplacementApply => {},
///             // Over for good, either way; and silence past the flow expectation, where the
///             // handle may still be open. Take the bus back out and close it.
///             SessionReleaseCause::RoleRetired
///             | SessionReleaseCause::BindingReplaced
///             | SessionReleaseCause::FlowStalled => match self.ledger.discard_retained(role) {
///                 DiscardedSession::Discarded(bus) => bus.close(),
///                 DiscardedSession::NothingRetained => {},
///             },
///             // The handle is already dead; letting the record go is the whole of it.
///             SessionReleaseCause::DeviceUnavailable { .. }
///             | SessionReleaseCause::ReportedLoss => {
///                 self.ledger.discard_retained(role);
///             },
///         }
///     }
/// }
///
/// impl ShutterDriver {
///     /// The hardware lookup, kept off the trait so `resolve_target` stays one line.
///     fn address_on_the_bus(
///         &self,
///         endpoint: &DeviceEndpoint,
///     ) -> TargetResolution<ShutterAddress> {
///         // The whole universe is addressable as soon as the bus is open. A driver that cannot
///         // reach its target names the state change that would make it reachable; nothing has
///         // started, so there is no failure to report.
///         match endpoint.id {
///             EndpointId::Whole => TargetResolution::Reached(ShutterAddress),
///             EndpointId::Part(_) => {
///                 TargetResolution::Deferred(TargetWait::ApplicationRoleAttachmentRequired)
///             },
///         }
///     }
///
///     /// The shutter reached its position. Any readback the driver needs is copied into its own
///     /// record first: the ledger keeps nothing from `applied`, which is why the kernel puts no
///     /// `Clone` bound on `Configuration`.
///     fn shutter_settled(&mut self, attempt: AttemptRef, settled: ShutterOpening) {
///         match self
///             .ledger
///             .succeed_attempt(attempt, Applied::DiffersFromDispatched(settled))
///         {
///             // The bus stays in the ledger's queued slot, waiting for establishment.
///             AttemptSucceeded::Succeeded => {},
///             // The kernel ended the attempt while the shutter was still moving. Expected.
///             AttemptSucceeded::AlreadyFinished | AttemptSucceeded::AttemptUnknown => {},
///         }
///     }
///
///     /// The bus failed mid-move. The class is the whole message.
///     fn shutter_failed(&mut self, attempt: AttemptRef, detail: String) {
///         match self
///             .ledger
///             .fail_attempt(attempt, DeviceAccessError::Transport { detail })
///         {
///             // A non-success finish is the ledger's last word on this attempt, so the bus comes
///             // back with it. `#[must_use]` is what stops this arm being written as `let _`.
///             AttemptFinish::Finished { record } => record.close(),
///             AttemptFinish::AlreadyFinished | AttemptFinish::AttemptUnknown => {},
///         }
///     }
///
///     /// The move ended without reaching its arrival condition and without a device failure.
///     fn shutter_move_ended(&mut self, attempt: AttemptRef) {
///         match self
///             .ledger
///             .abort_attempt(attempt, DriverAbortReason::OperationEnded)
///         {
///             AttemptFinish::Finished { record } => record.close(),
///             AttemptFinish::AlreadyFinished | AttemptFinish::AttemptUnknown => {},
///         }
///     }
///
///     /// A status frame reached its consumer — never where it was polled off the bus, because a
///     /// frame discarded in between was presented to nobody.
///     fn shutter_reported(&mut self, role: &RoleKey, status: &ShutterStatus, at: Instant) {
///         match self
///             .ledger
///             .credit_flow::<ShutterBusTransport>(role, status, at)
///         {
///             // Credited against the live session, or retained until one exists.
///             FlowCredit::CreditedToSession | FlowCredit::RetainedForEstablishment => {},
///             // The session is on its way out, or this role is not one the ledger holds.
///             FlowCredit::SessionEnding | FlowCredit::UnknownRole => {},
///         }
///     }
///
///     /// Someone moved the shutter by hand.
///     fn shutter_moved_itself(&mut self, role: &RoleKey, opening: ShutterOpening) {
///         let _ = self.ledger.change_configuration(role, opening);
///     }
///
///     /// The bus dropped while a session stood.
///     fn shutter_bus_dropped(&mut self, role: &RoleKey, detail: String) {
///         let _ = self
///             .ledger
///             .report_loss(role, DeviceAccessError::Transport { detail });
///     }
///
///     /// What the ledger is holding for a role. Neither reader hands out an authority.
///     fn attempt_for(&self, role: &RoleKey) -> AttemptLookup { self.ledger.attempt_of(role) }
///
///     /// The session side of the same question.
///     fn session_for(&self, role: &RoleKey) -> SessionLookup { self.ledger.session_of(role) }
/// }
///
/// fn main() {}
/// ```
pub trait EndpointDriver: Send + Sync + 'static {
    /// Driver-specific configuration mirrored on the role entity after an accepted success.
    type Configuration: Reflect + FromReflect + Component;

    /// The live handle this driver must hold before it can touch its device.
    type Target: 'static;

    /// Resolve the driven target, or report why it cannot be reached.
    fn resolve_target(
        &mut self,
        world: &mut World,
        context: &TargetResolutionContext<'_>,
        requested: &Self::Configuration,
    ) -> TargetResolution<Self::Target>;

    /// Start an absolute apply and retain or finish the supplied one-use completion.
    ///
    /// The driver reports every hardware failure through its ledger — `fail_attempt` and the
    /// [`DeviceAccessError`] classes — so there is no second failure channel here to disagree with
    /// it. Returning nothing is what makes that the only channel.
    fn start_apply(
        &mut self,
        world: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        requested: &Self::Configuration,
        target: Self::Target,
    );

    /// Accept the lease for a successful completion before the kernel publishes establishment.
    ///
    /// The named [`Establishment`](crate::Establishment) outcome of `establish_lease` is the whole
    /// answer, so this method has nothing left to return; see [`Self::start_apply`] for why the
    /// driver has one failure channel and not two.
    fn established(
        &mut self,
        world: &mut World,
        context: EstablishedContext<'_, Self::Configuration>,
    );

    /// Cancel client work for an attempt whose kernel guards no longer hold.
    ///
    /// `cause` says why the kernel invalidated the attempt, and a driver may branch on it: an
    /// attempt abandoned by a revision advance and one displaced by a replacement binding can want
    /// different hardware treatment. A driver with nothing to distinguish ignores it.
    ///
    /// `role_entity` may be [`DriverCleanupRoleEntity::Removed`]. A despawn alone does not produce
    /// it: a role entity despawned while its binding stays registered is re-spawned by role-entity
    /// recovery, which runs immediately before kernel cleanup, and arrives here as `Live`.
    /// `Removed` means the entity is gone at the moment the binding is replaced or retired, or
    /// that the id the kernel recorded no longer exists. Handle it exactly as `Live` for the
    /// driver's own records and hardware, and skip only the work that needs the entity. To
    /// reproduce it in a test: retire the role, despawn its entity, run one update.
    fn cancel_apply(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        cause: AttemptInvalidation,
    );

    /// Release client session state before its role entity is retired or replaced.
    ///
    /// `role_entity` may be [`DriverCleanupRoleEntity::Removed`], under the same precondition and
    /// with the same obligation as in [`Self::cancel_apply`]: the entity is gone because the
    /// binding was replaced or retired with it already despawned, or because the recorded id no
    /// longer exists — never merely because something despawned it — and the handle still has to
    /// be closed even though the entity is gone.
    fn release_session(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    );
}

/// Result of resolving the live target an endpoint driver needs before it can apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetResolution<Target> {
    /// The target is present and may be driven.
    Reached(Target),
    /// Target resolution named the state change required before another attempt can start.
    Deferred(TargetWait),
}

/// State change that can make a deferred target reachable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetWait {
    /// A reporter must produce usable evidence for the target capability.
    Reporter {
        /// Registry-issued reporter responsible for the target evidence.
        reporter: ReporterId,
        /// Reporter-classified reason the target is not currently reachable.
        error:    DeviceAccessError,
    },
    /// Application code must attach the role's driven entity.
    ApplicationRoleAttachmentRequired,
    /// Application code must restore reflected-component registration.
    ApplicationCapabilityRegistrationRequired {
        /// Correlated reporter projection failure naming the required type.
        failure: CapabilityProjectionFailure,
    },
}

/// Process-local device facts validated before target resolution.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedDeviceContext {
    entity:   Entity,
    id:       DeviceRef,
    #[cfg(feature = "test-support")]
    revision: DeviceRevision,
}

impl ResolvedDeviceContext {
    #[cfg(feature = "test-support")]
    pub(crate) const fn new(entity: Entity, id: DeviceRef, revision: DeviceRevision) -> Self {
        Self {
            entity,
            id,
            revision,
        }
    }

    #[cfg(not(feature = "test-support"))]
    pub(crate) const fn new(entity: Entity, id: DeviceRef, _revision: DeviceRevision) -> Self {
        Self { entity, id }
    }

    /// Return the current device entity for this callback.
    #[must_use]
    pub const fn entity(&self) -> Entity { self.entity }

    /// Return the process-local diagnostic device reference.
    #[must_use]
    pub const fn id(&self) -> DeviceRef { self.id }

    /// Return the device revision validated for this resolution.
    #[must_use]
    #[cfg(feature = "test-support")]
    pub const fn revision(&self) -> DeviceRevision { self.revision }
}

/// Borrowed role and device facts supplied before an attempt is issued.
pub struct TargetResolutionContext<'a> {
    role:        &'a RoleKey,
    role_entity: Entity,
    endpoint:    &'a DeviceEndpoint,
    device:      ResolvedDeviceContext,
}

impl<'a> TargetResolutionContext<'a> {
    pub(crate) const fn new(
        role: &'a RoleKey,
        role_entity: Entity,
        endpoint: &'a DeviceEndpoint,
        device: ResolvedDeviceContext,
    ) -> Self {
        Self {
            role,
            role_entity,
            endpoint,
            device,
        }
    }

    /// Borrow the durable role key. The accompanying entity is valid only for this callback.
    #[must_use]
    pub const fn role(&self) -> &RoleKey { self.role }

    /// Return the role entity repaired immediately before this callback.
    #[must_use]
    pub const fn role_entity(&self) -> Entity { self.role_entity }

    /// Borrow the durable endpoint selected by the binding.
    #[must_use]
    pub const fn endpoint(&self) -> &DeviceEndpoint { self.endpoint }

    /// Borrow the validated process-local device facts.
    #[must_use]
    pub const fn device(&self) -> &ResolvedDeviceContext { &self.device }

    /// Borrow a capability only while its reporter's accepted projection still supports it.
    ///
    /// # Errors
    ///
    /// Returns the requirement when the reporter's accepted projection does not currently include
    /// the requested capability.
    pub fn required_capability<'world, C>(
        &self,
        world: &'world World,
        reporter: ReporterId,
    ) -> Result<&'world C, RequiredCapabilityUnavailable>
    where
        C: Component + Reflect + TypePath,
    {
        let requirement = CapabilityRequirement::of::<C>();
        let projected = world
            .get::<ProjectedCapabilityTypes>(self.device.entity)
            .is_some_and(|projected| projected.includes(reporter, &requirement));
        let reporter_supports_projection = reporter_capability_projection(world, reporter)
            .is_some_and(|projection| {
                matches!(projection, CapabilityProjectionStatus::AllProjected)
            });
        if projected && reporter_supports_projection {
            return world
                .get::<C>(self.device.entity)
                .ok_or(RequiredCapabilityUnavailable {
                    reporter,
                    requirement,
                });
        }
        Err(RequiredCapabilityUnavailable {
            reporter,
            requirement,
        })
    }
}

/// Kernel-owned signal that one reporter did not project a required capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredCapabilityUnavailable {
    reporter:    ReporterId,
    requirement: CapabilityRequirement,
}

impl RequiredCapabilityUnavailable {
    /// Correlate this requirement with the reporter's latest projection result.
    #[must_use]
    pub fn target_wait(&self, world: &World) -> TargetWait {
        let failure = reporter_capability_projection(world, self.reporter).and_then(|projection| {
            let CapabilityProjectionStatus::Failed(failures) = projection else {
                return None;
            };
            failures.entries().iter().find(|failure| match failure {
                CapabilityProjectionFailure::ApplicationTypeRegistryUnavailable {
                    affected_type_path,
                } => affected_type_path == self.requirement.type_path(),
                CapabilityProjectionFailure::ReflectComponentNotRegistered { type_path } => {
                    type_path == self.requirement.type_path()
                },
            })
        });
        failure.cloned().map_or_else(
            || TargetWait::Reporter {
                reporter: self.reporter,
                error:    DeviceAccessError::Absent {
                    detail: format!(
                        "required capability `{}` is not projected",
                        self.requirement.type_path()
                    ),
                },
            },
            |failure| TargetWait::ApplicationCapabilityRegistrationRequired { failure },
        )
    }
}

fn reporter_capability_projection(
    world: &World,
    reporter: ReporterId,
) -> Option<&CapabilityProjectionStatus> {
    let reporter_ref = crate::ReporterRef::from_reporter_id(reporter);
    world.iter_entities().find_map(|entity| {
        let health = entity.get::<ReporterHealth>()?;
        if health.identity().reporter_ref != reporter_ref {
            return None;
        }
        match health.outcome() {
            ReporterOutcomeHealth::Succeeded {
                capability_projection,
                ..
            } => Some(capability_projection),
            ReporterOutcomeHealth::Failing { run, .. } => {
                let PreviousSuccess::At {
                    capability_projection,
                    ..
                } = &run.previous_success
                else {
                    return None;
                };
                Some(capability_projection)
            },
            ReporterOutcomeHealth::NotCompleted
            | ReporterOutcomeHealth::Deferred { .. }
            | ReporterOutcomeHealth::Unsupported { .. } => None,
        }
    })
}

/// Successful apply result and its relation to the dispatched configuration.
pub enum Applied<Configuration> {
    /// The driver established the dispatched configuration.
    AsDispatched,
    /// The driver established this configuration instead of the dispatched value.
    DiffersFromDispatched(Configuration),
}

/// Terminal result sent through an [`AttemptCompletion`].
pub enum DriverCompletion<Configuration> {
    /// The driver established a usable configuration.
    Succeeded(Applied<Configuration>),
    /// Device access failed after the attempt started.
    Failed(DeviceAccessError),
    /// Client-owned work ended for a driver-specific reason.
    Aborted(DriverAbortReason),
}

/// Why a driver ended a started operation without reporting a device-access failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum DriverAbortReason {
    /// The client operation ended before reaching its arrival condition.
    OperationEnded,
}

/// Why the kernel asks a driver to release an established session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionReleaseCause {
    /// Application code retired the role.
    RoleRetired,
    /// Application code replaced the binding under the same role key.
    BindingReplaced,
    /// The role is starting another apply.
    ReplacementApply,
    /// The device that owned the established endpoint stopped authorizing work.
    DeviceUnavailable {
        /// Unavailable conclusion that ended authorization.
        availability: crate::UnavailableKeyAvailability,
    },
    /// The session lease reported loss of device access.
    ReportedLoss,
    /// The kernel observed silence beyond the session's continuous-flow expectation.
    FlowStalled,
}

/// One-use, typed authority for reporting an attempt result.
///
/// The authority belongs in driver storage keyed by [`AttemptRef`], never on the role entity. A
/// role-scoped [`SessionLease`] is instead keyed by [`RoleKey`]. Role recovery may replace the
/// entity while either authority remains valid.
///
/// One attempt reports one result, and the result carries the configuration
/// the driver was registered for:
///
/// ```
/// use bevy::prelude::Component;
/// use bevy::prelude::Reflect;
/// use hana_rigging::Applied;
/// use hana_rigging::AttemptCompletion;
/// use hana_rigging::DriverCompletion;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn finish_once(completion: AttemptCompletion<Configuration>) {
///     completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
/// }
/// ```
///
/// [`Self::finish`] takes the authority by value, so a second report has
/// nothing left to report with. The single report above is what keeps the
/// cases below meaningful — a rename would break it loudly rather than leaving
/// these failing for an unrelated reason.
///
/// ```compile_fail,E0382
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::{Applied, AttemptCompletion, DriverCompletion};
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn finish_twice(completion: AttemptCompletion<Configuration>) {
///     completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
///     completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
/// }
/// ```
///
/// A result describing some other configuration type is not this attempt's:
///
/// ```compile_fail,E0308
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::{Applied, AttemptCompletion, DriverCompletion};
///
/// #[derive(Component, Reflect)]
/// struct ExpectedConfiguration;
///
/// #[derive(Component, Reflect)]
/// struct WrongConfiguration;
///
/// fn finish_with_wrong_configuration(completion: AttemptCompletion<ExpectedConfiguration>) {
///     completion.finish(DriverCompletion::Succeeded(Applied::DiffersFromDispatched(
///         WrongConfiguration,
///     )));
/// }
/// ```
///
/// The kernel issues the authority, so a driver cannot assemble one:
///
/// ```compile_fail
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::AttemptCompletion;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// let _ = AttemptCompletion::<Configuration> {};
/// ```
///
/// … nor duplicate one it holds:
///
/// ```compile_fail,E0308
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::AttemptCompletion;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn clone_authority(completion: &AttemptCompletion<Configuration>) {
///     let _: AttemptCompletion<Configuration> = completion.clone();
/// }
/// ```
///
/// … nor write one down to be revived later:
///
/// ```compile_fail,E0277
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::AttemptCompletion;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn serialize_authority(completion: &AttemptCompletion<Configuration>) {
///     let _ = serde_json::to_string(completion);
/// }
/// ```
pub struct AttemptCompletion<Configuration> {
    role:          RoleKey,
    attempt:       AttemptRef,
    frame_instant: Arc<Mutex<Instant>>,
    reports:       Arc<Mutex<DriverReportMailbox>>,
    marker:        PhantomData<fn(Configuration)>,
}

impl<Configuration> AttemptCompletion<Configuration>
where
    Configuration: Reflect,
{
    pub(crate) fn new(role: RoleKey, attempt: AttemptRef, reports: &DriverReports) -> Self {
        Self {
            role,
            attempt,
            frame_instant: Arc::clone(&reports.frame_instant),
            reports: Arc::clone(&reports.mailbox),
            marker: PhantomData,
        }
    }

    /// Consume this authority and queue exactly one terminal result.
    ///
    /// The frame instant is read and the result queued under one held frame-instant lock, which
    /// the kernel's own frame-instant update also takes. That is what stops a driver finishing on a
    /// worker thread from stamping its completion with a frame the kernel has already left: a
    /// completion stamped one frame early is accepted by the overrun check that should refuse it.
    pub fn finish(self, completion: DriverCompletion<Configuration>) {
        let completion = match completion {
            DriverCompletion::Succeeded(Applied::AsDispatched) => {
                ErasedDriverCompletion::Succeeded(ErasedApplied::AsDispatched)
            },
            DriverCompletion::Succeeded(Applied::DiffersFromDispatched(configuration)) => {
                ErasedDriverCompletion::Succeeded(ErasedApplied::DiffersFromDispatched(Box::new(
                    configuration,
                )))
            },
            DriverCompletion::Failed(error) => ErasedDriverCompletion::Failed(error),
            DriverCompletion::Aborted(reason) => ErasedDriverCompletion::Aborted(reason),
        };
        let frame_instant = self
            .frame_instant
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .completions
            .push_back(QueuedCompletion {
                role: self.role,
                attempt: self.attempt,
                completed_at: *frame_instant,
                completion,
            });
    }
}

/// Typed authority for one established driver session.
///
/// The lease belongs in driver storage keyed by [`RoleKey`], never on the role entity. The entity
/// supplied to [`EndpointDriver::established`] is valid only for that callback.
///
/// A lease reports what its session did:
///
/// ```
/// use bevy::prelude::Component;
/// use bevy::prelude::Reflect;
/// use hana_rigging::DeviceAccessError;
/// use hana_rigging::SessionLease;
/// use hana_rigging::SessionRef;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn correlate(lease: &SessionLease<Configuration>) -> SessionRef { lease.session_ref() }
///
/// fn reconfigure(lease: &mut SessionLease<Configuration>) {
///     lease.configuration_changed(Configuration);
/// }
///
/// fn end(lease: SessionLease<Configuration>, error: DeviceAccessError) {
///     lease.report_loss(error);
/// }
/// ```
///
/// The kernel issues it, so a driver cannot assemble one. The reports above are
/// what keep the cases below meaningful — a rename would break them loudly
/// rather than leaving these failing for an unrelated reason.
///
/// ```compile_fail
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::SessionLease;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// let _ = SessionLease::<Configuration> {};
/// ```
///
/// … nor duplicate one it holds:
///
/// ```compile_fail,E0308
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::SessionLease;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn clone_authority(lease: &SessionLease<Configuration>) {
///     let _: SessionLease<Configuration> = lease.clone();
/// }
/// ```
///
/// … nor write one down to be revived later:
///
/// ```compile_fail,E0277
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::SessionLease;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn serialize_authority(lease: &SessionLease<Configuration>) {
///     let _ = serde_json::to_string(lease);
/// }
/// ```
///
/// A driver states what its transport carried; it never credits an arrival. The
/// lease exposes no method that would, and the evidence a lease holds cannot be
/// spelled from outside the kernel either:
///
/// ```compile_fail
/// use hana_rigging::SessionLease;
///
/// fn assert_an_arrival<Configuration>(lease: &mut SessionLease<Configuration>) {
///     lease.record_datum_arrival();
/// }
/// ```
///
/// ```compile_fail,E0603
/// use std::time::Instant;
///
/// use hana_rigging::SessionDatumArrivalEvidence;
///
/// fn spell_an_arrival(observed_at: Instant) -> SessionDatumArrivalEvidence {
///     SessionDatumArrivalEvidence::ObservedAt(observed_at)
/// }
/// ```
pub struct SessionLease<Configuration> {
    role:           RoleKey,
    session:        SessionRef,
    frame_instant:  Arc<Mutex<Instant>>,
    reports:        Arc<Mutex<DriverReportMailbox>>,
    datum_arrivals: Weak<SessionDatumArrivalMailbox>,
    marker:         PhantomData<fn(Configuration)>,
}

#[derive(Debug)]
struct SessionDatumArrivalMailbox {
    pending: Mutex<SessionDatumArrivalEvidence>,
}

#[derive(Debug)]
pub(crate) struct SessionDatumArrivalReceiver(Arc<SessionDatumArrivalMailbox>);

impl<Configuration> SessionLease<Configuration>
where
    Configuration: Reflect,
{
    pub(crate) fn new(
        role: RoleKey,
        session: SessionRef,
        reports: &DriverReports,
    ) -> (Self, SessionDatumArrivalReceiver) {
        let datum_arrivals = SessionDatumArrivalReceiver::without_sender();
        (
            Self {
                role,
                session,
                frame_instant: Arc::clone(&reports.frame_instant),
                reports: Arc::clone(&reports.mailbox),
                datum_arrivals: Arc::downgrade(&datum_arrivals.0),
                marker: PhantomData,
            },
            datum_arrivals,
        )
    }

    /// Report a repeatable external configuration change for this exact session.
    pub fn configuration_changed(&mut self, configuration: Configuration) {
        self.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .session_reports
            .push_back(QueuedSessionReport {
                role:    self.role.clone(),
                session: self.session,
                report:  ErasedSessionReport::ConfigurationChanged(Box::new(configuration)),
            });
    }

    /// Consume the lease and report that its device access ended.
    pub fn report_loss(self, error: DeviceAccessError) {
        self.reports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .session_reports
            .push_back(QueuedSessionReport {
                role:    self.role,
                session: self.session,
                report:  ErasedSessionReport::Loss(error),
            });
    }

    /// Return the copyable correlation value for this lease.
    #[must_use]
    pub const fn session_ref(&self) -> SessionRef { self.session }
}

impl<Configuration> SessionLease<Configuration> {
    /// Record that this exact session supplied a datum during the current kernel frame.
    ///
    /// Uncollected arrivals coalesce in one latest-value slot owned by the exact session. After
    /// the kernel releases or replaces that session, its old lease can no longer write testimony.
    ///
    /// The slot never shares the contended report mailbox, but the write is made under the same
    /// held frame-instant lock [`AttemptCompletion::finish`] uses, so testimony cannot be credited
    /// against a frame the kernel has already left.
    fn record_datum_arrival(&self) {
        let Some(datum_arrivals) = self.datum_arrivals.upgrade() else {
            return;
        };
        let frame_instant = self
            .frame_instant
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *datum_arrivals
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            SessionDatumArrivalEvidence::ObservedAt(*frame_instant);
    }

    /// Record that this exact session showed transport activity carrying no datum.
    ///
    /// Same slot and same locking as [`Self::record_datum_arrival`]. The two are separate calls
    /// because only one of them can start a session flowing: a driver reporting traffic it cannot
    /// present has no way to spell a datum it never received.
    fn record_transport_activity(&self) {
        let Some(datum_arrivals) = self.datum_arrivals.upgrade() else {
            return;
        };
        let frame_instant = self
            .frame_instant
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *datum_arrivals
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            SessionDatumArrivalEvidence::TransportActivityAt(*frame_instant);
    }

    /// Credit an arrival the caller observed at a stated moment rather than this kernel frame.
    ///
    /// A driver's stated moment is testimony about the past, so the current kernel frame instant
    /// is its ceiling. An instant beyond that frame would make the expiry judge's elapsed-silence
    /// measurement saturate at zero and hold the session `Flowing` until real time overtook the
    /// claimed moment — silence the kernel could not see. A moment earlier than the frame is
    /// credited as stated: that is a real observation made before the lease existed, and ageing it
    /// correctly is the whole point of the retained-observation path.
    ///
    /// The ceiling is read under the same held frame-instant lock [`AttemptCompletion::finish`]
    /// takes, so it cannot be a frame the kernel has already left.
    fn record_observed_datum_arrival(&self, observed_at: Instant) {
        let Some(datum_arrivals) = self.datum_arrivals.upgrade() else {
            return;
        };
        let frame_instant = self
            .frame_instant
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *datum_arrivals
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            SessionDatumArrivalEvidence::ObservedAt(observed_at.min(*frame_instant));
    }

    /// Credit one transport observation against this session's flow, classified by the driver
    /// that owns it.
    ///
    /// This is the only way a driver moves the flow axis. There is deliberately no method that
    /// simply asserts an arrival: a bare assertion is what let a device report itself flowing
    /// having delivered no picture, and it kept reappearing at whichever call site was written
    /// next. Here the device-specific judgment is confined to one
    /// [`DeviceTransport::classify`] implementation, and every transition that follows from it
    /// belongs to the kernel.
    ///
    /// Call it where the datum reaches its consumer, not where it is polled: a datum discarded in
    /// between must be classified [`TransportObservation::Quiet`], because nobody saw it.
    pub fn observe<Transport>(&mut self, observation: &Transport::Observation)
    where
        Transport: DeviceTransport,
    {
        match Transport::classify(observation) {
            TransportObservation::DatumDelivered => self.record_datum_arrival(),
            TransportObservation::ActivityWithoutDatum => self.record_transport_activity(),
            TransportObservation::Quiet => {},
        }
    }
}

impl SessionDatumArrivalReceiver {
    #[must_use]
    fn without_sender() -> Self {
        Self(Arc::new(SessionDatumArrivalMailbox {
            pending: Mutex::new(SessionDatumArrivalEvidence::NoDatumObserved),
        }))
    }

    pub(crate) fn has_pending(&self) -> bool {
        *self
            .0
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            != SessionDatumArrivalEvidence::NoDatumObserved
    }

    pub(crate) fn take_latest(&self) -> SessionDatumArrivalEvidence {
        let mut pending = self
            .0
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut *pending, SessionDatumArrivalEvidence::NoDatumObserved)
    }
}

/// Context supplied after target resolution issues one attempt and completion.
pub struct ApplyContext<'a, Configuration> {
    target:     TargetResolutionContext<'a>,
    deadline:   Instant,
    permit:     ApplyPermit,
    attempt:    AttemptRef,
    completion: AttemptCompletion<Configuration>,
}

impl<'a, Configuration> ApplyContext<'a, Configuration> {
    pub(crate) const fn new(
        target: TargetResolutionContext<'a>,
        deadline: Instant,
        permit: ApplyPermit,
        attempt: AttemptRef,
        completion: AttemptCompletion<Configuration>,
    ) -> Self {
        Self {
            target,
            deadline,
            permit,
            attempt,
            completion,
        }
    }

    /// Borrow the target resolution facts used to issue this attempt.
    #[must_use]
    pub const fn target(&self) -> &TargetResolutionContext<'a> { &self.target }

    /// Return the frame-clock deadline for the apply.
    #[must_use]
    pub const fn deadline(&self) -> Instant { self.deadline }

    /// Return the kernel-issued device authorization.
    #[must_use]
    #[cfg(feature = "test-support")]
    pub const fn permit(&self) -> ApplyPermit { self.permit }

    /// Return the process-local attempt reference.
    #[must_use]
    pub const fn attempt(&self) -> AttemptRef { self.attempt }

    /// Consume the context and return its one-use completion authority.
    #[must_use]
    pub fn into_completion(self) -> AttemptCompletion<Configuration> {
        let Self {
            completion, permit, ..
        } = self;
        let _ = permit;
        completion
    }
}

/// Context supplied when the kernel accepts a successful completion.
pub struct EstablishedContext<'a, Configuration> {
    role:        &'a RoleKey,
    role_entity: Entity,
    attempt:     AttemptRef,
    lease:       SessionLease<Configuration>,
}

impl<'a, Configuration> EstablishedContext<'a, Configuration> {
    pub(crate) const fn new(
        role: &'a RoleKey,
        role_entity: Entity,
        attempt: AttemptRef,
        lease: SessionLease<Configuration>,
    ) -> Self {
        Self {
            role,
            role_entity,
            attempt,
            lease,
        }
    }

    /// Borrow the durable role key for this session.
    #[must_use]
    pub const fn role(&self) -> &RoleKey { self.role }

    /// Return the role entity repaired immediately before this callback.
    #[must_use]
    pub const fn role_entity(&self) -> Entity { self.role_entity }

    /// Return the attempt whose success created this session.
    #[must_use]
    pub const fn attempt(&self) -> AttemptRef { self.attempt }

    /// Consume the context and return the one-use session lease.
    ///
    /// `datum_arrival` states what the driver already observed for this session before the kernel
    /// issued the lease. A driver whose device supplied the datum that made the session usable
    /// reports the moment it saw it, and that moment is what the kernel credits — a driver with
    /// nothing to report says so.
    #[must_use]
    pub fn into_lease(
        self,
        datum_arrival: SessionDatumArrivalEvidence,
    ) -> SessionLease<Configuration> {
        match datum_arrival {
            SessionDatumArrivalEvidence::ObservedAt(observed_at) => {
                self.lease.record_observed_datum_arrival(observed_at);
            },
            // A session opened on traffic alone has delivered nothing to present, so it begins
            // awaiting its first datum exactly as one that saw nothing at all.
            SessionDatumArrivalEvidence::TransportActivityAt(_)
            | SessionDatumArrivalEvidence::NoDatumObserved => {},
        }
        self.lease
    }
}

pub(crate) enum ErasedApplied {
    AsDispatched,
    DiffersFromDispatched(Box<dyn Reflect>),
}

pub(crate) enum ErasedDriverCompletion {
    Succeeded(ErasedApplied),
    Failed(DeviceAccessError),
    Aborted(DriverAbortReason),
}

pub(crate) struct QueuedCompletion {
    pub(crate) role:         RoleKey,
    pub(crate) attempt:      AttemptRef,
    pub(crate) completed_at: Instant,
    pub(crate) completion:   ErasedDriverCompletion,
}

pub(crate) enum ErasedSessionReport {
    ConfigurationChanged(Box<dyn Reflect>),
    Loss(DeviceAccessError),
}

pub(crate) struct QueuedSessionReport {
    pub(crate) role:    RoleKey,
    pub(crate) session: SessionRef,
    pub(crate) report:  ErasedSessionReport,
}

struct DriverReportMailbox {
    completions:     VecDeque<QueuedCompletion>,
    session_reports: VecDeque<QueuedSessionReport>,
}

#[derive(Clone, Resource)]
pub(crate) struct DriverReports {
    frame_instant: Arc<Mutex<Instant>>,
    mailbox:       Arc<Mutex<DriverReportMailbox>>,
}

impl DriverReports {
    pub(crate) fn starting_at(frame_instant: Instant) -> Self {
        Self {
            frame_instant: Arc::new(Mutex::new(frame_instant)),
            mailbox:       Arc::new(Mutex::new(DriverReportMailbox {
                completions:     VecDeque::new(),
                session_reports: VecDeque::new(),
            })),
        }
    }

    pub(crate) fn set_frame_instant(&self, frame_instant: Instant) {
        *self
            .frame_instant
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = frame_instant;
    }

    pub(crate) fn drain_completions(&self) -> VecDeque<QueuedCompletion> {
        std::mem::take(
            &mut self
                .mailbox
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .completions,
        )
    }

    pub(crate) fn drain_session_reports(&self) -> VecDeque<QueuedSessionReport> {
        std::mem::take(
            &mut self
                .mailbox
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .session_reports,
        )
    }

    pub(crate) fn refuse_attempt(&self, attempt: AttemptRef) {
        self.mailbox
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .completions
            .retain(|queued| queued.attempt != attempt);
    }
}

/// Failure at the erased driver boundary before a typed `EndpointDriver` method can run.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DriverContractError {
    /// A binding addressed a [`DriverId`](crate::DriverId) that no registration call issued during
    /// this process.
    #[error("endpoint driver `{driver_id:?}` is not registered")]
    DriverNotRegistered {
        /// The process-local route that did not select a registered endpoint driver.
        driver_id: crate::DriverId,
    },
    /// A driver registry entry's erased value differs from the concrete driver type installed in
    /// its dispatch functions.
    ///
    /// Registration constructs the value and functions together, but erased dispatch keeps this
    /// internal contract failure recoverable so one invalid entry does not terminate the app.
    #[error("endpoint driver registry entry expected concrete driver `{expected_driver}`")]
    DriverTypeMismatch {
        /// The concrete [`EndpointDriver`] type required by the installed dispatch function.
        expected_driver: &'static str,
    },
    /// A binding or capture value reached a driver whose `Configuration` type differs from the
    /// concrete value selected by a state-issued driver request.
    ///
    /// This remains recoverable because two drivers can serve one device while accepting distinct
    /// configuration types; terminating the app would turn one authored routing error into loss
    /// of every endpoint.
    #[error(
        "endpoint driver expected configuration `{expected_configuration}` but received `{received_configuration}`"
    )]
    ConfigurationTypeMismatch {
        /// The concrete [`EndpointDriver::Configuration`] type the registered driver accepts.
        expected_configuration: &'static str,
        /// The reflected type path stored in the supplied requested or established value.
        received_configuration: String,
    },
    /// A state-issued restore request no longer has its checked readback value at dispatch time.
    ///
    /// Normal request ownership prevents this result: the request retains the binding borrow until
    /// dispatch completes. It remains recoverable so a future kernel caller cannot commit an
    /// applying state after a malformed internal request.
    #[error("state-issued apply request for role `{role}` has no last-known-good configuration")]
    LastKnownGoodConfigurationUnavailable {
        /// Binding role whose restore request no longer selected a safe readback value.
        role: crate::RoleKey,
    },
}

type ErasedDriver = dyn Any + Send + Sync;
pub(crate) type ErasedTarget = Box<dyn Any>;
type ResolveTargetFunction = fn(
    &mut ErasedDriver,
    &mut World,
    &TargetResolutionContext<'_>,
    &dyn Reflect,
) -> Result<TargetResolution<ErasedTarget>, DriverContractError>;
type StartApplyFunction = fn(
    &mut ErasedDriver,
    &mut World,
    TargetResolutionContext<'_>,
    Instant,
    ApplyPermit,
    AttemptRef,
    &DriverReports,
    &dyn Reflect,
    ErasedTarget,
) -> Result<(), DriverContractError>;
type EstablishedFunction = fn(
    &mut ErasedDriver,
    &mut World,
    &RoleKey,
    Entity,
    AttemptRef,
    SessionRef,
    &DriverReports,
) -> Result<SessionDatumArrivalReceiver, DriverContractError>;
type CancelApplyFunction = fn(
    &mut ErasedDriver,
    &mut World,
    &RoleKey,
    DriverCleanupRoleEntity,
    AttemptRef,
    AttemptInvalidation,
) -> Result<(), DriverContractError>;
type ReleaseSessionFunction = fn(
    &mut ErasedDriver,
    &mut World,
    &RoleKey,
    DriverCleanupRoleEntity,
    SessionRef,
    SessionReleaseCause,
) -> Result<(), DriverContractError>;

/// Erased driver value and the typed functions the kernel routes by `DriverId`.
pub(crate) struct DriverEntry {
    driver:          Box<ErasedDriver>,
    resolve_target:  ResolveTargetFunction,
    start_apply:     StartApplyFunction,
    established:     EstablishedFunction,
    cancel_apply:    CancelApplyFunction,
    release_session: ReleaseSessionFunction,
}

impl DriverEntry {
    pub(crate) fn new<Driver>(driver: Driver) -> Self
    where
        Driver: EndpointDriver,
    {
        Self {
            driver:          Box::new(driver),
            resolve_target:  resolve_driver_target::<Driver>,
            start_apply:     start_driver_apply::<Driver>,
            established:     establish_driver_session::<Driver>,
            cancel_apply:    cancel_driver_apply::<Driver>,
            release_session: release_driver_session::<Driver>,
        }
    }

    pub(crate) fn resolve_target(
        &mut self,
        world: &mut World,
        context: &TargetResolutionContext<'_>,
        configuration: &dyn Reflect,
    ) -> Result<TargetResolution<ErasedTarget>, DriverContractError> {
        (self.resolve_target)(self.driver.as_mut(), world, context, configuration)
    }

    pub(crate) fn start_apply(
        &mut self,
        world: &mut World,
        context: TargetResolutionContext<'_>,
        deadline: Instant,
        permit: ApplyPermit,
        attempt: AttemptRef,
        reports: &DriverReports,
        configuration: &dyn Reflect,
        target: ErasedTarget,
    ) -> Result<(), DriverContractError> {
        (self.start_apply)(
            self.driver.as_mut(),
            world,
            context,
            deadline,
            permit,
            attempt,
            reports,
            configuration,
            target,
        )
    }

    pub(crate) fn established(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: Entity,
        attempt: AttemptRef,
        session: SessionRef,
        reports: &DriverReports,
    ) -> Result<SessionDatumArrivalReceiver, DriverContractError> {
        (self.established)(
            self.driver.as_mut(),
            world,
            role,
            role_entity,
            attempt,
            session,
            reports,
        )
    }

    pub(crate) fn cancel_apply(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        cause: AttemptInvalidation,
    ) -> Result<(), DriverContractError> {
        (self.cancel_apply)(
            self.driver.as_mut(),
            world,
            role,
            role_entity,
            attempt,
            cause,
        )
    }

    pub(crate) fn release_session(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        role_entity: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    ) -> Result<(), DriverContractError> {
        (self.release_session)(
            self.driver.as_mut(),
            world,
            role,
            role_entity,
            session,
            cause,
        )
    }
}

fn resolve_driver_target<Driver>(
    driver: &mut ErasedDriver,
    world: &mut World,
    context: &TargetResolutionContext<'_>,
    configuration: &dyn Reflect,
) -> Result<TargetResolution<ErasedTarget>, DriverContractError>
where
    Driver: EndpointDriver,
{
    let driver = typed_driver_mut::<Driver>(driver)?;
    let configuration = typed_configuration::<Driver>(configuration)?;
    Ok(match driver.resolve_target(world, context, configuration) {
        TargetResolution::Reached(target) => TargetResolution::Reached(Box::new(target)),
        TargetResolution::Deferred(wait) => TargetResolution::Deferred(wait),
    })
}

fn start_driver_apply<Driver>(
    driver: &mut ErasedDriver,
    world: &mut World,
    context: TargetResolutionContext<'_>,
    deadline: Instant,
    permit: ApplyPermit,
    attempt: AttemptRef,
    reports: &DriverReports,
    configuration: &dyn Reflect,
    target: ErasedTarget,
) -> Result<(), DriverContractError>
where
    Driver: EndpointDriver,
{
    let driver = typed_driver_mut::<Driver>(driver)?;
    let configuration = typed_configuration::<Driver>(configuration)?;
    let target = target.downcast::<Driver::Target>().map_err(|_| {
        DriverContractError::DriverTypeMismatch {
            expected_driver: type_name::<Driver>(),
        }
    })?;
    let completion = AttemptCompletion::new(context.role().clone(), attempt, reports);
    driver.start_apply(
        world,
        ApplyContext::new(context, deadline, permit, attempt, completion),
        configuration,
        *target,
    );

    Ok(())
}

fn establish_driver_session<Driver>(
    driver: &mut ErasedDriver,
    world: &mut World,
    role: &RoleKey,
    role_entity: Entity,
    attempt: AttemptRef,
    session: SessionRef,
    reports: &DriverReports,
) -> Result<SessionDatumArrivalReceiver, DriverContractError>
where
    Driver: EndpointDriver,
{
    let (lease, datum_arrivals) = SessionLease::new(role.clone(), session, reports);
    typed_driver_mut::<Driver>(driver)?.established(
        world,
        EstablishedContext::new(role, role_entity, attempt, lease),
    );
    Ok(datum_arrivals)
}

fn cancel_driver_apply<Driver>(
    driver: &mut ErasedDriver,
    world: &mut World,
    role: &RoleKey,
    role_entity: DriverCleanupRoleEntity,
    attempt: AttemptRef,
    cause: AttemptInvalidation,
) -> Result<(), DriverContractError>
where
    Driver: EndpointDriver,
{
    typed_driver_mut::<Driver>(driver)?.cancel_apply(world, role, role_entity, attempt, cause);
    Ok(())
}

fn release_driver_session<Driver>(
    driver: &mut ErasedDriver,
    world: &mut World,
    role: &RoleKey,
    role_entity: DriverCleanupRoleEntity,
    session: SessionRef,
    cause: SessionReleaseCause,
) -> Result<(), DriverContractError>
where
    Driver: EndpointDriver,
{
    typed_driver_mut::<Driver>(driver)?.release_session(world, role, role_entity, session, cause);
    Ok(())
}

fn typed_configuration<Driver>(
    configuration: &dyn Reflect,
) -> Result<&Driver::Configuration, DriverContractError>
where
    Driver: EndpointDriver,
{
    configuration.as_any().downcast_ref().ok_or_else(|| {
        DriverContractError::ConfigurationTypeMismatch {
            expected_configuration: type_name::<Driver::Configuration>(),
            received_configuration: configuration.reflect_type_path().to_owned(),
        }
    })
}

fn typed_driver_mut<Driver>(driver: &mut ErasedDriver) -> Result<&mut Driver, DriverContractError>
where
    Driver: EndpointDriver,
{
    driver
        .downcast_mut::<Driver>()
        .ok_or_else(|| DriverContractError::DriverTypeMismatch {
            expected_driver: type_name::<Driver>(),
        })
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::num::NonZeroU32;
    use std::rc::Rc;
    use std::sync::PoisonError;
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::Reflect;
    use bevy::prelude::World;

    use super::Applied;
    use super::AttemptCompletion;
    use super::DiscoveryProgressSender;
    use super::DriverCompletion;
    use super::DriverReports;
    use super::EstablishedContext;
    use super::MainThreadDiscoveryJob;
    use super::PendingDiscoveryProgress;
    use super::SessionDatumArrivalEvidence;
    use super::SessionDatumArrivalReceiver;
    use super::SessionLease;
    use crate::AttemptRef;
    use crate::DeviceScan;
    use crate::DiscoveryProgress;
    use crate::DiscoveryProgressSendError;
    use crate::RoleKey;
    use crate::RoleKeyError;
    use crate::SessionRef;

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct TestConfiguration;

    struct MainThreadDiscoverySource {
        name: Rc<str>,
    }

    #[test]
    fn main_thread_discovery_job_reads_non_send_resource_and_returns_owned_whole_set() {
        let mut world = World::new();
        world.insert_non_send(MainThreadDiscoverySource {
            name: Rc::from("monitor-api"),
        });
        let main_thread_discovery_job = MainThreadDiscoveryJob::new(|world| {
            let main_thread_discovery_source = world.non_send::<MainThreadDiscoverySource>();
            assert_eq!(main_thread_discovery_source.name.as_ref(), "monitor-api");

            DeviceScan::Complete(Vec::new())
        });

        let device_scan = main_thread_discovery_job.run(&mut world);

        assert!(matches!(
            device_scan,
            DeviceScan::Complete(device_records) if device_records.is_empty()
        ));
    }

    #[test]
    fn progress_mailbox_coalesces_to_latest_measured_and_indeterminate_updates() {
        const UPDATE_COUNT: u32 = 1_000;

        let (discovery_progress_sender, discovery_progress_receiver) =
            DiscoveryProgressSender::scheduler_mailbox();
        let total = NonZeroU32::new(UPDATE_COUNT).unwrap_or(NonZeroU32::MIN);
        for completed in 0..UPDATE_COUNT {
            assert!(
                discovery_progress_sender
                    .send(DiscoveryProgress::Measured { completed, total })
                    .is_ok(),
                "scheduler must retain the progress receiver"
            );
        }

        assert!(matches!(
            discovery_progress_receiver.take_latest(),
            PendingDiscoveryProgress::Latest(DiscoveryProgress::Measured {
                completed,
                total,
            }) if completed == UPDATE_COUNT - 1 && total.get() == UPDATE_COUNT
        ));
        assert!(matches!(
            discovery_progress_receiver.take_latest(),
            PendingDiscoveryProgress::NoUpdate
        ));

        for completed in 0..UPDATE_COUNT {
            assert!(
                discovery_progress_sender
                    .send(DiscoveryProgress::Measured { completed, total })
                    .is_ok(),
                "scheduler must retain the progress receiver"
            );
        }
        assert!(
            discovery_progress_sender
                .send(DiscoveryProgress::Indeterminate)
                .is_ok(),
            "scheduler must retain the progress receiver"
        );
        assert!(matches!(
            discovery_progress_receiver.take_latest(),
            PendingDiscoveryProgress::Latest(DiscoveryProgress::Indeterminate)
        ));
    }

    #[test]
    fn progress_sender_reports_stopped_after_scheduler_releases_receiver() {
        let (discovery_progress_sender, discovery_progress_receiver) =
            DiscoveryProgressSender::scheduler_mailbox();
        drop(discovery_progress_receiver);

        assert!(matches!(
            discovery_progress_sender.send(DiscoveryProgress::Indeterminate),
            Err(DiscoveryProgressSendError::SchedulerStopped)
        ));
    }

    #[test]
    fn session_datum_arrivals_coalesce_to_the_latest_kernel_frame() -> Result<(), RoleKeyError> {
        let first_frame = Instant::now();
        let second_frame = first_frame + Duration::from_secs(1);
        let reports = DriverReports::starting_at(first_frame);
        let (lease, datum_arrivals) = SessionLease::<TestConfiguration>::new(
            RoleKey::new("coalescing-testimony")?,
            SessionRef::new(1),
            &reports,
        );

        assert!(!datum_arrivals.has_pending());
        lease.record_datum_arrival();
        assert!(datum_arrivals.has_pending());
        reports.set_frame_instant(second_frame);
        lease.record_datum_arrival();
        assert!(datum_arrivals.has_pending());

        assert_eq!(
            datum_arrivals.take_latest(),
            SessionDatumArrivalEvidence::ObservedAt(second_frame)
        );
        assert!(!datum_arrivals.has_pending());
        assert_eq!(
            datum_arrivals.take_latest(),
            SessionDatumArrivalEvidence::NoDatumObserved
        );
        Ok(())
    }

    #[test]
    fn datum_arrival_receiver_without_sender_permanently_reports_no_datum() {
        let datum_arrivals = SessionDatumArrivalReceiver::without_sender();

        assert!(!datum_arrivals.has_pending());
        assert_eq!(
            datum_arrivals.take_latest(),
            SessionDatumArrivalEvidence::NoDatumObserved
        );
        assert!(!datum_arrivals.has_pending());
    }

    #[test]
    fn released_session_lease_cannot_write_to_a_dropped_receiver() -> Result<(), RoleKeyError> {
        let reports = DriverReports::starting_at(Instant::now());
        let (lease, datum_arrivals) = SessionLease::<TestConfiguration>::new(
            RoleKey::new("stale-testimony")?,
            SessionRef::new(1),
            &reports,
        );
        drop(datum_arrivals);

        lease.record_datum_arrival();
        Ok(())
    }

    #[test]
    fn a_retained_observation_beyond_the_kernel_frame_is_credited_as_that_frame()
    -> Result<(), RoleKeyError> {
        let frame = Instant::now();
        let reports = DriverReports::starting_at(frame);
        let role = RoleKey::new("testimony-from-the-future")?;
        let (lease, datum_arrivals) =
            SessionLease::<TestConfiguration>::new(role.clone(), SessionRef::new(1), &reports);
        let established =
            EstablishedContext::new(&role, Entity::PLACEHOLDER, AttemptRef::new(1), lease);

        drop(
            established.into_lease(SessionDatumArrivalEvidence::ObservedAt(
                frame + Duration::from_secs(30),
            )),
        );

        assert_eq!(
            datum_arrivals.take_latest(),
            SessionDatumArrivalEvidence::ObservedAt(frame),
            "an observation stated past the kernel frame is credited as that frame, so elapsed \
             silence is measured from a moment the kernel has actually reached"
        );
        Ok(())
    }

    #[test]
    fn a_retained_observation_before_the_kernel_frame_is_credited_as_stated()
    -> Result<(), RoleKeyError> {
        let observed_at = Instant::now();
        let frame = observed_at + Duration::from_millis(250);
        let reports = DriverReports::starting_at(frame);
        let role = RoleKey::new("testimony-from-before-the-lease")?;
        let (lease, datum_arrivals) =
            SessionLease::<TestConfiguration>::new(role.clone(), SessionRef::new(1), &reports);
        let established =
            EstablishedContext::new(&role, Entity::PLACEHOLDER, AttemptRef::new(1), lease);

        drop(established.into_lease(SessionDatumArrivalEvidence::ObservedAt(observed_at)));

        assert_eq!(
            datum_arrivals.take_latest(),
            SessionDatumArrivalEvidence::ObservedAt(observed_at),
            "an observation the driver made before the lease existed keeps its stated moment, so \
             the silence that accrued while the session was being established still counts"
        );
        Ok(())
    }

    #[test]
    fn a_completion_carries_the_frame_instant_in_effect_when_it_was_queued()
    -> Result<(), RoleKeyError> {
        /// Pause between probes, so probing does not starve the finishing thread of the lock it is
        /// trying to take.
        const PROBE_INTERVAL: Duration = Duration::from_millis(1);

        let first_frame = Instant::now();
        let second_frame = first_frame + Duration::from_secs(1);
        let reports = DriverReports::starting_at(first_frame);
        let completion = AttemptCompletion::<TestConfiguration>::new(
            RoleKey::new("contended-completion")?,
            AttemptRef::new(1),
            &reports,
        );

        // Holding the mailbox parks `finish` between reading the frame instant and queueing under
        // it, which is the only window where the two locks can be observed apart.
        let held_mailbox = reports
            .mailbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let finishing = thread::spawn(move || {
            completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
        });

        // A `finish` parked on the mailbox still holds the frame instant. Released after the read
        // instead, the frame-instant lock would stay free for as long as this loop probes.
        //
        // There is no deadline. How soon the spawned thread reaches the lock is the machine's
        // business, so a deadline decides from how busy the machine is that a thread still on its
        // way had never held it. What is definitive is that thread returning: this test holds the
        // mailbox, so a correct `finish` cannot get past it, and a finished thread is one that let
        // the frame instant go without ever queueing. A `finish` that released the frame instant
        // and then parked on the mailbox trips no such signal and parks here for nextest's
        // `slow-timeout` to end, which reports it as stuck.
        loop {
            let frame_instant_held = reports.frame_instant.try_lock().is_err();
            if frame_instant_held {
                break;
            }
            assert!(
                !finishing.is_finished(),
                "finish left the frame instant free while its completion was still unqueued, so \
                 the kernel could leave the frame that completion is stamped with"
            );
            thread::sleep(PROBE_INTERVAL);
        }

        // The kernel now attempts to leave the frame; it blocks until the completion is queued.
        let advancing = thread::spawn({
            let reports = reports.clone();
            move || reports.set_frame_instant(second_frame)
        });
        drop(held_mailbox);

        assert!(
            finishing.join().is_ok(),
            "the finishing thread must not panic"
        );
        assert!(
            advancing.join().is_ok(),
            "the frame-advancing thread must not panic"
        );

        let completions = reports.drain_completions();
        assert_eq!(completions.len(), 1, "finish queues exactly one completion");
        assert!(
            completions
                .front()
                .is_some_and(|queued| queued.completed_at == first_frame),
            "the queued completion carries the frame instant in effect when it was enqueued"
        );
        assert_eq!(
            *reports
                .frame_instant
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            second_frame,
            "the contending frame advance must have landed, or nothing was contended"
        );
        Ok(())
    }

    #[test]
    fn driver_configuration_registers_reflect_component_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();
        let type_id = TypeId::of::<TestConfiguration>();

        assert!(type_registry.contains(type_id));
        assert!(
            type_registry
                .get_type_data::<ReflectComponent>(type_id)
                .is_some()
        );

        drop(type_registry);
    }
}
