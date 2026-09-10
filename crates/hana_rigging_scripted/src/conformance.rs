//! One walk that proves any [`EndpointDriver`] against the real kernel lifecycle.
//!
//! Every device integration used to prove its driver against a harness of its own, so two drivers
//! could disagree about what the kernel promises and neither test would notice. The states that
//! matter are not the ones a driver author thinks to write down: a session released because its
//! device departed, an attempt cancelled because the binding was replaced under it, a cleanup
//! dispatched after the role entity is already gone. [`run`] walks one subject through all of them
//! in one [`App`] driven by the real [`RiggingPlugin`], and names the step it stopped at.
//!
//! The suite lives here rather than in `hana_rigging` because `hana_rigging_scripted` depends on
//! `hana_rigging`: a module there naming [`ScriptedReporter`] would be circular. This crate already
//! owns the hardware-free reporter and the advance helpers the walk needs, and already enables
//! `test-support`.
//!
//! # What the walk proves, and what it does not
//!
//! The walk asserts the kernel's own observables — [`RoleStatus`], [`KernelRolePresentation`], the
//! attempt endings the kernel publishes, and the entity [`Bindings`] retains — plus two readings of
//! the subject's own record. It proves *this driver, on this scan, reached every state the kernel
//! promised it and refused none of them*. It makes no impossibility claim: what a driver may not
//! spell at all stays with the `compile_fail` lane.
//!
//! Two readings have to come from the subject, and they are the two failures no kernel observable
//! can see.
//!
//! A driver that takes its [`SessionLease`] and drops it establishes exactly the session a driver
//! that files it does: the kernel issues the lease and publishes the establishment before it calls
//! `established`, and the lease has no `Drop`, so no kernel reading tells the two apart.
//! [`ConformanceSubject::session`] is that channel.
//!
//! A driver whose `cancel_apply` or `release_session` body is empty is likewise invisible from
//! outside: the kernel dispatches the call, records nothing about what the driver did with it, and
//! publishes the same status, presentation and ending either way. Every teardown step in the walk
//! would pass against a driver that ignores the cleanup contract outright.
//! [`ConformanceSubject::cleanups`] is the channel that makes those steps falsifiable, and it is
//! why each teardown asserts the cause and the [`DriverCleanupRoleEntity`] the kernel dispatched
//! rather than only that the role stopped being established.
//!
//! [`RiggingPlugin`]: hana_rigging::RiggingPlugin
//! [`RoleStatus`]: hana_rigging::RoleStatus
//! [`Bindings`]: hana_rigging::Bindings
//! [`SessionLease`]: hana_rigging::SessionLease

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::sync::Arc;
use std::time::Duration;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FromReflect;
use bevy::prelude::On;
use bevy::prelude::Reflect;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::time::TimeUpdateStrategy;
use hana_rigging::ApplyDeadline;
use hana_rigging::AttemptEndingView;
use hana_rigging::AttemptInvalidation;
use hana_rigging::AttemptInvalidationView;
use hana_rigging::AttemptRef;
use hana_rigging::AuthoritativeReporterCoverage;
use hana_rigging::BindingAuthoring;
use hana_rigging::BindingError;
use hana_rigging::BindingPolicy;
use hana_rigging::Bindings;
use hana_rigging::Capabilities;
use hana_rigging::ConnectedCause;
use hana_rigging::ContinuousFlowExpiryCause;
use hana_rigging::CoveredDeviceIdentitySpace;
use hana_rigging::DeviceEndpoint;
use hana_rigging::DeviceIdSource;
use hana_rigging::DeviceKey;
use hana_rigging::DiscoveryCadence;
use hana_rigging::DiscoveryProgress;
use hana_rigging::DriverCleanupRoleEntity;
use hana_rigging::EndpointDriverRegistration;
use hana_rigging::EndpointId;
use hana_rigging::FlowExpectation;
use hana_rigging::KernelRolePresentation;
use hana_rigging::LiveRoleChange;
use hana_rigging::LiveRoleChanged;
use hana_rigging::OnAbort;
use hana_rigging::OnSessionLoss;
use hana_rigging::Presence;
use hana_rigging::RecoveryPolicy;
use hana_rigging::RegistrationAttemptEnded;
use hana_rigging::ReportedAs;
use hana_rigging::ReporterActivation;
use hana_rigging::ReporterCoverage;
use hana_rigging::ReporterId;
use hana_rigging::ReporterRegistration;
use hana_rigging::RetireRole;
use hana_rigging::RetryOn;
use hana_rigging::RiggingAppExt;
use hana_rigging::RiggingLimits;
use hana_rigging::RiggingPlugin;
use hana_rigging::RoleKey;
use hana_rigging::RolePresentationView;
use hana_rigging::RoleStatus;
use hana_rigging::RoleStatusView;
use hana_rigging::RoleUnavailableCause;
use hana_rigging::SessionLookup;
use hana_rigging::SessionRef;
use hana_rigging::SessionReleaseCause;
use thiserror::Error;

use crate::CapabilityBuilder;
use crate::ScriptedAdvanceError;
use crate::ScriptedDevice;
use crate::ScriptedReporter;
use crate::ScriptedRunGate;
use crate::ScriptedScan;
use crate::advance_until_accepted;
use crate::advance_until_running;
use crate::install_scripted_io_task_pool;

/// Role every conformance walk authors, retires, and re-authors.
///
/// The walk owns the role rather than taking one from the subject: the retirement steps end a role
/// for good, and a subject that had named its own would be handed back a key it could no longer
/// use.
const CONFORMANCE_ROLE: &str = "conformance-subject";

/// How many frames one bounded wait spends before it reports the step unreachable.
///
/// Generous enough to cover the kernel's own `Collect -> RoleEntityRecovery -> Reconcile ->
/// Prepare -> SessionLoss -> Apply` ordering several times over, so a step that never lands is a
/// driver fact rather than a frame budget.
const STEP_FRAME_CEILING: usize = 32;

/// How many scripted scans one bounded wait admits before it reports the step unreachable.
///
/// One more than the longest run of same-state scans the script holds, so a step that has to walk
/// a whole state block still has a release in hand when it reaches its target state.
const SCAN_RELEASE_CEILING: usize = 6;

/// How many times [`script`] repeats each device state.
///
/// The walk's own minimum consumption is one release for the first arrival plus one whole state
/// block for each of its three device transitions, so the script has to hold more than four blocks'
/// worth to leave anything over. The surplus is what a discovery run the walk did not ask for
/// spends: `request_discovery_for_waiting_roles` asks the covering reporter for a run every time a
/// bound role's device stops being live, which the walk's own departure steps guarantee.
const SCAN_STATES_REPEATED: usize = 4;

/// Frame step for a subject whose sessions are never evaluated for data arrival.
///
/// Non-zero so the kernel's frame clock reads as measurable, and small enough that no kernel bound
/// in the walk can expire while a step is stepping frames.
const UNMONITORED_FRAME_STEP: Duration = Duration::from_millis(1);

/// How many frame steps must fit inside the tightest bound a continuous subject declared.
///
/// The walk drives the clock, so a step long enough to cross a subject's own flow bound would
/// stall a driver that is crediting every frame. Dividing the tightest declared bound by this keeps
/// the walk inside it while still letting the flow step cross it deliberately, and requiring the
/// division to have actually happened is what
/// [`ConformanceRefusal::FramePacingCrossesTheFlowBound`] checks.
const FRAME_STEPS_PER_FLOW_BOUND: u32 = 32;

/// Extra frames the flow step spends past a continuous subject's first-datum bound.
///
/// The stall has to become observable inside the step, or a silent driver would read as a step the
/// walk merely ran out of frames for.
const FLOW_OVERRUN_FRAMES: usize = 8;

/// The most frames the flow step will ever spend, whatever bounds a subject declared.
///
/// Frames needed to cross the first-datum bound scale with the ratio between that bound and the
/// datum gap, and both are free: a subject declaring a 33ms gap and a 5s startup allowance needs
/// thousands of updates, and legal bounds a few nanoseconds apart need hundreds of millions. A walk
/// that spends those is a hang rather than a result, so the walk refuses the subject up front
/// instead — see [`ConformanceRefusal::FlowBoundsExceedTheWalksFrameBudget`].
const FLOW_FRAME_CEILING: usize = STEP_FRAME_CEILING * 8;

/// A driver put through the conformance walk, and the facts only its author can supply.
///
/// The trait says nothing about hardware. Everything a walk needs is either a kernel observable it
/// reads for itself or one of these six answers.
///
/// Every reader takes `&World`. A shipped driver keeps its records in a world resource rather than
/// in the value the subject holds — [`ScreenDriver`](hana_rigging::EndpointDriver) files its lease
/// into a non-send resource, and the driver value itself lives inside the kernel's own registry —
/// so a reader with no world could be implemented by a purpose-written fake and by nothing that
/// ships.
///
/// # Writing `FLOW`
///
/// [`FlowExpectation`] is an associated const because it selects which presentation reading the
/// walk asserts, and a driver's data axis is a property of the driver rather than of one instance.
/// [`FirstDatumTimeout::new`] and [`MaximumDatumGap::new`] are `const fn` returning `Result`, and
/// `?`, `unwrap` and `expect` are all unavailable in a const initializer, so a continuous subject
/// spells its bounds like this:
///
/// ```ignore
/// const FLOW: FlowExpectation = FlowExpectation::Continuous(ContinuousFlowExpectation::new(
///     match FirstDatumTimeout::new(Duration::from_millis(200)) {
///         Ok(bound) => bound,
///         Err(_) => panic!("the first-datum bound must be non-zero"),
///     },
///     match MaximumDatumGap::new(Duration::from_millis(100)) {
///         Ok(bound) => bound,
///         Err(_) => panic!("the datum gap must be non-zero"),
///     },
/// ));
/// ```
///
/// The two bounds may not be arbitrarily far apart: the walk drives the frame clock by hand and
/// refuses a pair it cannot cross inside its own frame budget. See
/// [`ConformanceRefusal::FlowBoundsExceedTheWalksFrameBudget`].
///
/// A subject is `'static` because its capability declaration outlives the call that built it: the
/// scripted reporter holds the declaration for the life of the app and rebuilds it on every replay.
///
/// [`FirstDatumTimeout::new`]: hana_rigging::FirstDatumTimeout::new
/// [`MaximumDatumGap::new`]: hana_rigging::MaximumDatumGap::new
pub trait ConformanceSubject: 'static {
    /// The configuration this driver accepts, which is also what the walk authors bindings with.
    type Configuration: Reflect + FromReflect + Component;

    /// Whether this driver's sessions must keep receiving data, and within what bounds.
    ///
    /// The walk asserts the presentation this promises and nothing more, so a `NotMonitored` driver
    /// is proven on its own promises rather than on a streaming driver's.
    const FLOW: FlowExpectation;

    /// Add this driver to the app exactly as production adds it, and return its registration.
    ///
    /// The subject adds its own plugin, which is what registers the driver: `add_endpoint_driver`
    /// never deduplicates, and a driver holding a catalog cannot be registered twice. The walk
    /// therefore registers no driver of its own and routes every binding through what this returns.
    ///
    /// `reporter` is the walk's own scripted reporter, and it is the reporter the subject's driver
    /// has to read its capabilities from.
    /// [`required_capability`](hana_rigging::TargetResolutionContext::required_capability) matches
    /// on the pair of reporter and requirement, so a declaration published under the walk's
    /// reporter can only satisfy a driver that asks the walk's reporter for it. A shipped driver
    /// captures a `ReporterId` at plugin build; a subject wrapping one captures this instead.
    fn install(
        &mut self,
        app: &mut App,
        reporter: ReporterId,
    ) -> EndpointDriverRegistration<Self::Configuration>;

    /// The configuration the walk authors each of its bindings with.
    fn requested(&self) -> Self::Configuration;

    /// The capability components the scripted reporter publishes on the presented device.
    ///
    /// `device` is the key the walk bound to, because the only capability requirement in the
    /// workspace names device-specific data: a declaration that could not see the presented device
    /// could not be kept consistent with it.
    ///
    /// The declaration is published on the presented device alone. A caller who wrote a
    /// multi-device scan keeps whatever capabilities the other devices already carried.
    ///
    /// A driver that reads no capability answers [`CapabilityDeclaration::none`].
    fn declare(&mut self, device: &DeviceKey) -> CapabilityDeclaration;

    /// Attach whatever `resolve_target` waits on to the role entity the walk just authored.
    ///
    /// Called once for each binding the walk authors, before that binding's first frame — the
    /// replacement binding of [`ConformanceStep::ReplacementCancelledTheAttempt`] included. A
    /// driver whose target is not an entity — one that resolves straight to `Reached` — leaves this
    /// empty.
    fn attach_role(&mut self, world: &mut World, role_entity: Entity);

    /// What this driver's own record says it holds for `role`.
    ///
    /// A driver built on [`DriverLedger`](hana_rigging::DriverLedger) answers
    /// `self.ledger.session_of(role)` and nothing else. The walk reads this at establishment and
    /// after every teardown, because a lease the driver never filed is invisible to the kernel.
    fn session(&self, world: &World, role: &RoleKey) -> SessionLookup;

    /// Every cleanup call this driver has been handed for `role`, in the order it received them.
    ///
    /// Cumulative for the life of the driver and never drained: the walk reads the list at the
    /// start and the end of each teardown step and asserts what appeared between the two, so a
    /// reader that consumed its entries would hide a step's evidence from the step after it.
    ///
    /// A driver records one [`RecordedCleanup`] per `cancel_apply` and `release_session` it
    /// receives, carrying the arguments unchanged. Nothing in the kernel can see whether a driver
    /// acted on a cleanup, so this is what makes the walk's teardown half falsifiable at all.
    fn cleanups(&self, world: &World, role: &RoleKey) -> RecordedCleanups;
}

/// The capability components one subject's driver reads off its presented device.
///
/// A declaration that builds rather than a value that is held: [`Capabilities`] owns
/// `Box<dyn Reflect>`, which Bevy 0.19 cannot clone, and the scripted reporter replays its scan
/// any number of times, so what the reporter keeps has to build a fresh set on each replay.
#[derive(Clone)]
pub struct CapabilityDeclaration(CapabilityBuilder);

impl CapabilityDeclaration {
    /// Declare whatever this closure builds, rebuilt fresh on every replay of the scan.
    #[must_use]
    pub fn rebuilt_by(build: impl Fn() -> Capabilities + Send + Sync + 'static) -> Self {
        Self(Arc::new(build))
    }

    /// Declare nothing, which is what a driver that reads no capability answers.
    #[must_use]
    pub fn none() -> Self { Self::rebuilt_by(Capabilities::new) }

    /// Publish this declaration on one scripted device.
    fn published_on(&self, device: ScriptedDevice) -> ScriptedDevice {
        let build = Arc::clone(&self.0);
        device.with_capabilities(move || build())
    }
}

impl fmt::Debug for CapabilityDeclaration {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("CapabilityDeclaration(<rebuilt on each replay>)")
    }
}

/// One cleanup call the kernel dispatched to a driver, as the driver recorded it.
///
/// Carries the arguments the driver was handed rather than a verdict about them, because the walk
/// is what assigns which cause and which role-entity state each of its steps requires. A driver
/// records these unexamined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordedCleanup {
    /// `cancel_apply` reached the driver for an attempt the kernel would not let continue.
    AttemptCancelled {
        /// Whether the role entity was still spawned when the call ran.
        role_entity:  DriverCleanupRoleEntity,
        /// The attempt the kernel cancelled.
        attempt:      AttemptRef,
        /// Why the kernel would not let the attempt continue.
        invalidation: AttemptInvalidation,
    },
    /// `release_session` reached the driver for a session the kernel has ended.
    SessionReleased {
        /// Whether the role entity was still spawned when the call ran.
        role_entity: DriverCleanupRoleEntity,
        /// The session the kernel ended.
        session:     SessionRef,
        /// Why the kernel ended it.
        cause:       SessionReleaseCause,
    },
}

/// Every cleanup one driver has been handed for one role, oldest first.
///
/// Ordered rather than a set because two steps of the walk end a session for different reasons and
/// only position separates them, and cumulative rather than drained because each step reads the
/// tail its own frames produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordedCleanups {
    calls: Vec<RecordedCleanup>,
}

impl RecordedCleanups {
    /// Record every cleanup a driver received for one role, in the order it received them.
    #[must_use]
    pub const fn new(calls: Vec<RecordedCleanup>) -> Self { Self { calls } }

    /// Every cleanup this driver received, oldest first.
    #[must_use]
    pub fn calls(&self) -> &[RecordedCleanup] { &self.calls }

    /// How many cleanups this driver has received so far, which is where a step starts reading.
    #[must_use]
    pub const fn len(&self) -> usize { self.calls.len() }

    /// Report whether this driver has received no cleanup at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.calls.is_empty() }

    /// Everything recorded after `cursor`, which is one step's own share of the list.
    fn since(&self, cursor: usize) -> &[RecordedCleanup] {
        self.calls.get(cursor..).unwrap_or_default()
    }
}

impl FromIterator<RecordedCleanup> for RecordedCleanups {
    fn from_iter<Calls>(calls: Calls) -> Self
    where
        Calls: IntoIterator<Item = RecordedCleanup>,
    {
        Self::new(calls.into_iter().collect())
    }
}

/// The release cause one teardown step requires.
///
/// A named requirement rather than a [`SessionReleaseCause`] value because
/// [`SessionReleaseCause::DeviceUnavailable`] carries the availability conclusion that ended
/// authorization, and a step requires the departure, not one particular conclusion behind it.
#[derive(Clone, Copy, Debug)]
enum RequiredReleaseCause {
    /// The application retired the role.
    RoleRetired,
    /// The device that owned the established endpoint stopped authorizing work.
    DeviceUnavailable,
}

impl RequiredReleaseCause {
    const fn matches(self, cause: &SessionReleaseCause) -> bool {
        matches!(
            (self, cause),
            (Self::RoleRetired, SessionReleaseCause::RoleRetired)
                | (
                    Self::DeviceUnavailable,
                    SessionReleaseCause::DeviceUnavailable { .. }
                )
        )
    }

    const fn described(self) -> &'static str {
        match self {
            Self::RoleRetired => "release_session(RoleRetired)",
            Self::DeviceUnavailable => "release_session(DeviceUnavailable)",
        }
    }
}

/// One attempt ending the kernel published while the walk ran.
///
/// The kernel publishes an ending through one of two channels depending on whether the role entity
/// still exists, and the walk's removal steps end attempts whose entity is gone, so both are
/// collected into one list and read the same way.
#[derive(Clone, Debug)]
struct PublishedAttemptEnding {
    role:    RoleKey,
    attempt: AttemptRef,
    ending:  AttemptEndingView,
}

/// Every attempt ending the kernel published during one walk, in publication order.
#[derive(Resource, Default)]
struct PublishedAttemptEndings(Vec<PublishedAttemptEnding>);

/// Collect the ending of an attempt whose role entity is still live.
fn record_live_attempt_ending(
    event: On<LiveRoleChanged>,
    mut endings: ResMut<PublishedAttemptEndings>,
) {
    if let LiveRoleChange::AttemptEnded { attempt, ending } = &event.change {
        endings.0.push(PublishedAttemptEnding {
            role:    event.role.clone(),
            attempt: *attempt,
            ending:  ending.clone(),
        });
    }
}

/// Collect the ending of an attempt the kernel could not publish against a live role entity.
fn record_registration_attempt_ending(
    event: On<RegistrationAttemptEnded>,
    mut endings: ResMut<PublishedAttemptEndings>,
) {
    endings.0.push(PublishedAttemptEnding {
        role:    event.role.clone(),
        attempt: event.attempt,
        ending:  event.ending.clone(),
    });
}

/// One state the walk reached, in the order the kernel hands them out.
///
/// Named for the kernel reading that proves it rather than for the driver callback behind it: the
/// walk can only observe what the kernel publishes, and a step named for a callback would promise
/// evidence it does not hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConformanceStep {
    /// The reporter's scan reached the kernel and the presented device resolves to a device id.
    DeviceReported,
    /// The binding was accepted and the kernel retains a role entity for it.
    BindingAuthored,
    /// The role is applying, so `resolve_target` answered `Reached` and `start_apply` ran.
    AttemptIssued,
    /// The role has an established session, and the driver's own record holds it.
    SessionEstablished,
    /// The role presents the reading its [`ConformanceSubject::FLOW`] promised.
    FlowRead,
    /// The device departed, the session ended, and the driver was told why.
    SessionEndedOnDeparture,
    /// The departed role presents as disconnected during its departure grace.
    DisconnectedDuringGrace,
    /// The device returned and the role established a session the departed one did not name.
    ReestablishedOnReturn,
    /// Retirement ended the role, released the driver's session, and ended its record.
    RetirementEndedTheSession,
    /// A binding replaced while the role was applying cancelled the attempt under it.
    ///
    /// The role entity survives: a same-endpoint replacement reuses the displaced entity, so the
    /// cancellation reaches the driver with a live one. The attempt is what the replacement
    /// displaces.
    ReplacementCancelledTheAttempt,
    /// Retiring and despawning an established role in one frame released its session as removed.
    RemovedRoleEntityEndedTheSession,
    /// Retiring and despawning an applying role in one frame cancelled its attempt as removed.
    RemovedRoleEntityCancelledTheAttempt,
    /// A departure found the recorded role entity gone and still released the driver's session.
    StrandedRoleEntityEndedTheSession,
}

impl Display for ConformanceStep {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let described = match self {
            Self::DeviceReported => "the presented device reaching the kernel",
            Self::BindingAuthored => "the binding being accepted",
            Self::AttemptIssued => "an attempt reaching the driver",
            Self::SessionEstablished => "a session being established",
            Self::FlowRead => "the promised flow presentation",
            Self::SessionEndedOnDeparture => "the session ending when the device departed",
            Self::DisconnectedDuringGrace => "the departed role presenting as disconnected",
            Self::ReestablishedOnReturn => "a fresh session when the device returned",
            Self::RetirementEndedTheSession => "retirement ending the session",
            Self::ReplacementCancelledTheAttempt => "a replacement cancelling an applying attempt",
            Self::RemovedRoleEntityEndedTheSession => {
                "retirement ending a session whose role entity was removed in the same frame"
            },
            Self::RemovedRoleEntityCancelledTheAttempt => {
                "retirement ending an attempt whose role entity was removed in the same frame"
            },
            Self::StrandedRoleEntityEndedTheSession => {
                "a departure ending a session whose recorded role entity was already gone"
            },
        };
        formatter.write_str(described)
    }
}

/// Where one conformance walk stopped.
///
/// A walk can refuse before it takes its first step — on the scan it was handed, or on bounds the
/// subject declared that the walk cannot drive — and that refusal says nothing about the driver.
/// Reporting it as the first real step would tell a caller the presented device never reached the
/// kernel when the kernel was never started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConformanceStop {
    /// The walk refused before its first step, so no driver behaviour was observed at all.
    BeforeTheWalkBegan,
    /// The walk was walking this step.
    At(ConformanceStep),
}

impl Display for ConformanceStop {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeTheWalkBegan => formatter.write_str("the walk's own preconditions"),
            Self::At(step) => step.fmt(formatter),
        }
    }
}

/// Every step one walk reached, in the order it reached them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConformanceReport {
    steps: Vec<ConformanceStep>,
}

impl ConformanceReport {
    /// Every step this walk reached, in order.
    #[must_use]
    pub fn steps(&self) -> &[ConformanceStep] { &self.steps }

    /// Report whether this walk reached one named step.
    #[must_use]
    pub fn reached(&self, step: ConformanceStep) -> bool { self.steps.contains(&step) }

    fn record(&mut self, step: ConformanceStep) { self.steps.push(step); }
}

/// Why one conformance walk could go no further.
///
/// Separate from the stop it happened at so a caller can tell "this driver never established" from
/// "this scan could not start a walk at all" without parsing a message.
#[derive(Debug, Error)]
pub enum ConformanceRefusal {
    /// The supplied scan names no device the walk could bind to.
    #[error("the supplied scan names no keyed, present device to bind")]
    ScanNamesNoPresentedDevice,
    /// The presented device is named by something other than a value the unit reported.
    ///
    /// The walk registers the identity scheme and the reporter's coverage itself, and both are
    /// read off the presented key; a synthesized or authored key names neither.
    #[error("the presented device is not named by a reported identity value")]
    PresentedDeviceIsNotReported,
    /// The supplied scan is a deferral or a failure rather than a completed whole set.
    ///
    /// The walk builds the departure and return scans around the one it was given, and only a
    /// completed set establishes that a device is there to bind.
    #[error("the supplied scan is not a completed whole set")]
    ScanIsNotACompletedSet,
    /// The kernel refused a binding the walk authored.
    #[error("the kernel refused a conformance binding: {0}")]
    BindingRefused(#[from] BindingError),
    /// The scripted reporter did not complete a run the walk asked for.
    #[error("the scripted reporter did not complete a requested run: {0}")]
    ReporterStalled(#[from] ScriptedAdvanceError),
    /// The walk replayed every scan it scripted and the device can no longer change state.
    ///
    /// Its own for a reason: a spent script departs the presented device for good, so every later
    /// wait reads exactly like a driver that cannot re-establish. Reporting exhaustion as an
    /// unpublished kernel reading would blame the driver for the walk running out of scans.
    #[error(
        "the walk's script of {scripted} scans is spent after {accepted} accepted runs, so the \
         presented device can no longer return"
    )]
    ScriptedScansExhausted {
        /// How many whole-set runs the kernel has accepted from the walk's reporter.
        accepted: u64,
        /// How many scans the walk scripted.
        scripted: usize,
    },
    /// The walk's own frame pacing would cross a bound the subject declared.
    ///
    /// A conformance suite that can fail a conformant driver on its own pacing proves nothing, so
    /// the walk refuses to start rather than report a stall it caused itself. The walk divides the
    /// tightest declared bound by `FRAME_STEPS_PER_FLOW_BOUND` and floors the result at one
    /// nanosecond; this fires whenever that floor bit, which is every gap under roughly thirty-two
    /// nanoseconds.
    #[error(
        "the walk's frame step of {frame_step:?} does not divide the subject's maximum datum gap \
         of {maximum_datum_gap:?} into {FRAME_STEPS_PER_FLOW_BOUND} steps"
    )]
    FramePacingCrossesTheFlowBound {
        /// How far the walk would advance the clock per update.
        frame_step:        Duration,
        /// The bound the subject declared between consecutive data arrivals.
        maximum_datum_gap: Duration,
    },
    /// The subject's declared bounds are too far apart for a hand-driven clock to cross.
    ///
    /// The frame step has to stay inside the datum gap, so crossing the first-datum bound costs
    /// roughly `FRAME_STEPS_PER_FLOW_BOUND` frames for every multiple the first-datum bound is of
    /// the gap. A subject declaring a 33ms gap and a 5s startup allowance would spend several
    /// thousand updates in one step, and bounds a few nanoseconds apart would spend hundreds of
    /// millions — a hang rather than a result, so the walk refuses instead.
    #[error(
        "crossing a first-datum bound of {first_datum_timeout:?} at a frame step held under a \
         datum gap of {maximum_datum_gap:?} needs {frames_required} updates, and the walk spends \
         at most {frames_available}"
    )]
    FlowBoundsExceedTheWalksFrameBudget {
        /// The bound the subject declared for its session's first datum.
        first_datum_timeout: Duration,
        /// The bound the subject declared between consecutive data arrivals.
        maximum_datum_gap:   Duration,
        /// How many updates crossing the first-datum bound would take.
        frames_required:     usize,
        /// How many updates the walk will spend on one flow step.
        frames_available:    usize,
    },
    /// The kernel judged the subject's session stalled before it ever presented.
    ///
    /// The kernel's own verdict rather than the walk running out of frames: a session that crossed
    /// a bound it declared is a driver that promised continuous data and did not deliver it, and
    /// the cause names which bound was crossed.
    #[error("the kernel judged the session stalled before it presented: {cause:?}")]
    FlowStalledBeforeItPresented {
        /// Which of the subject's declared bounds the session crossed.
        cause: ContinuousFlowExpiryCause,
    },
    /// The kernel never published the reading this step is defined by.
    #[error("the kernel never published `{expected}`; it last read `{observed}`")]
    ObservableNeverReached {
        /// The kernel reading this step waits for.
        expected: &'static str,
        /// What the kernel was publishing when the wait ran out.
        observed: String,
    },
    /// The kernel reissued the reference of a session it had already ended.
    ///
    /// The return step exists to prove the role opened a new session rather than resumed the old
    /// one, and a reissued reference would make the two indistinguishable to anyone holding it.
    #[error("the returned session reused the departed reference {session:?}")]
    ReturnedSessionReusesTheDepartedReference {
        /// The reference the departed session had, and the returned one answered with.
        session: SessionRef,
    },
    /// The driver's own record disagrees with what the kernel published.
    ///
    /// One of the two failures the kernel cannot see for itself: a driver that dropped an authority
    /// the kernel had already issued to it.
    #[error("the driver's own record answered `{observed}` where `{expected}` was promised")]
    SubjectRecordContradictsTheKernel {
        /// What the kernel's own publication promises the driver is holding.
        expected: &'static str,
        /// What the driver's record answered instead.
        observed: String,
    },
    /// The kernel dispatched a cleanup the driver's own record never saw.
    ///
    /// The other failure the kernel cannot see: it dispatches `cancel_apply` and `release_session`
    /// and keeps no account of what the driver did with either, so a driver whose cleanup bodies
    /// are empty publishes exactly what a driver that tore its hardware down publishes.
    #[error("the driver recorded no `{expected}` for this step; it recorded `{observed}`")]
    CleanupNeverReachedTheSubject {
        /// The cleanup call, cause and role-entity state this step dispatched.
        expected: &'static str,
        /// What the driver recorded during the step instead.
        observed: String,
    },
}

/// One conformance walk that stopped before the driver had been proven.
#[derive(Debug, Error)]
#[error("conformance stopped at {stop}: {refusal}")]
pub struct ConformanceFailure {
    reached: ConformanceReport,
    stop:    ConformanceStop,
    #[source]
    refusal: Box<ConformanceRefusal>,
}

impl ConformanceFailure {
    /// Every step the walk reached before it stopped.
    #[must_use]
    pub const fn reached(&self) -> &ConformanceReport { &self.reached }

    /// Where the walk stopped, which may be before it began.
    #[must_use]
    pub const fn stop(&self) -> ConformanceStop { self.stop }

    /// Why it could go no further.
    ///
    /// Boxed inside the failure so the walk's own `Result`s stay small: every step returns one.
    #[must_use]
    pub fn refusal(&self) -> &ConformanceRefusal { &self.refusal }
}

/// Walk one endpoint driver through the kernel's whole lifecycle and report what it reached.
///
/// `scan` names the device set the driver's device lives in — one completed whole set holding at
/// least one keyed, present device. The walk builds the departure and return scans around it, so a
/// caller supplies the present set only and never an empty one. The subject's
/// [`declare`](ConformanceSubject::declare) is published on the first keyed, present device the
/// scan names; every other device keeps the capabilities the caller gave it.
///
/// # Errors
///
/// Returns [`ConformanceFailure`] naming where the walk stopped and why. A failure carries every
/// step reached before it, so a driver that establishes but never survives a replug is
/// distinguishable from one that never establishes at all, and a walk that refused before its first
/// step reports [`ConformanceStop::BeforeTheWalkBegan`] rather than borrowing a step's name.
pub fn run<Subject>(
    subject: Subject,
    scan: ScriptedScan,
) -> Result<ConformanceReport, ConformanceFailure>
where
    Subject: ConformanceSubject,
{
    let mut walk = Walk::open(subject, scan)?;
    walk.walk()?;
    Ok(walk.finish())
}

/// One conformance walk in progress: the app, the subject, and what has been proven so far.
struct Walk<Subject>
where
    Subject: ConformanceSubject,
{
    app:            App,
    subject:        Subject,
    gate:           ScriptedRunGate,
    reporter:       ReporterId,
    scripted_scans: usize,
    driver:         EndpointDriverRegistration<Subject::Configuration>,
    endpoint:       DeviceEndpoint,
    role:           RoleKey,
    report:         ConformanceReport,
}

impl<Subject> Walk<Subject>
where
    Subject: ConformanceSubject,
{
    /// Build the app the walk runs in, entirely from the supplied scan and the subject.
    #[expect(
        clippy::expect_used,
        reason = "`CONFORMANCE_ROLE` is a fixed const naming no control character, which with \
                  emptiness is all `RoleKey::new` rejects, so the walk's own role name cannot be \
                  refused and a public refusal variant for it would be surface no caller could \
                  produce"
    )]
    fn open(mut subject: Subject, scan: ScriptedScan) -> Result<Self, ConformanceFailure> {
        let report = ConformanceReport::default();
        let refuse = |refusal: ConformanceRefusal| ConformanceFailure {
            reached: ConformanceReport::default(),
            stop:    ConformanceStop::BeforeTheWalkBegan,
            refusal: Box::new(refusal),
        };

        let device = presented_device(&scan).map_err(refuse)?;
        let DeviceIdSource::Reported { scheme, .. } = &device.id else {
            return Err(refuse(ConformanceRefusal::PresentedDeviceIsNotReported));
        };
        let scheme = scheme.clone();
        let kind = device.kind;
        let role = RoleKey::new(CONFORMANCE_ROLE).expect("the conformance role name is valid");
        let frame_step = frame_step::<Subject>();
        refuse_undrivable_flow_bounds::<Subject>(frame_step).map_err(refuse)?;

        let declaration = subject.declare(&device);
        let scans = script(declared(&scan, &device, &declaration));
        let scripted_scans = scans.len();

        install_scripted_io_task_pool();
        let mut app = App::new();
        app.insert_resource(walk_limits())
            .add_plugins(MinimalPlugins)
            .add_plugins(RiggingPlugin)
            .register_device_scheme(scheme)
            .insert_resource(TimeUpdateStrategy::ManualDuration(frame_step))
            .init_resource::<PublishedAttemptEndings>()
            .add_observer(record_live_attempt_ending)
            .add_observer(record_registration_attempt_ending);

        let (scripted_reporter, gate) =
            ScriptedReporter::gated(scans, DiscoveryProgress::Indeterminate);
        let reporter = app.add_device_reporter(
            scripted_reporter,
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
                    CoveredDeviceIdentitySpace::AllKeysOfKind { kind },
                )),
                Duration::from_secs(10),
            ),
        );
        let driver = subject.install(&mut app, reporter);

        Ok(Self {
            app,
            subject,
            gate,
            reporter,
            scripted_scans,
            driver,
            endpoint: DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            role,
            report,
        })
    }

    /// The whole lifecycle, in the order the kernel hands it out.
    fn walk(&mut self) -> Result<(), ConformanceFailure> {
        self.device_reported()?;
        self.binding_authored()?;
        self.attempt_issued(ConformanceStep::AttemptIssued)?;
        let established = self.session_established()?;
        self.flow_read()?;
        self.session_ended_on_departure(established)?;
        self.disconnected_during_grace()?;
        let returned = self.reestablished_on_return(established)?;
        self.retirement_ended_the_session(returned)?;
        self.replacement_cancelled_the_attempt()?;
        self.removed_role_entity_ended_the_session()?;
        self.removed_role_entity_cancelled_the_attempt()?;
        self.stranded_role_entity_ended_the_session()
    }

    /// Return the finished report, leaving no manual clock behind for whatever the subject added.
    fn finish(mut self) -> ConformanceReport {
        self.app.world_mut().remove_resource::<TimeUpdateStrategy>();
        self.report
    }

    // ---- steps ----

    /// Admit scans until the kernel resolves the presented key to a device it retains.
    fn device_reported(&mut self) -> Result<(), ConformanceFailure> {
        let device = self.endpoint.device.clone();
        self.admit_scans_until(
            ConformanceStep::DeviceReported,
            "a resolved device",
            move |walk| walk.resolves(&device),
        )?;
        self.report.record(ConformanceStep::DeviceReported);
        Ok(())
    }

    /// Author the walk's binding and hand the subject its role entity.
    fn binding_authored(&mut self) -> Result<(), ConformanceFailure> {
        self.author_binding(ConformanceStep::BindingAuthored)?;
        self.report.record(ConformanceStep::BindingAuthored);
        Ok(())
    }

    /// Advance until the role is applying, which is the only frame a mid-flight step can act on.
    fn attempt_issued(&mut self, step: ConformanceStep) -> Result<(), ConformanceFailure> {
        self.advance_until(step, "RoleStatusView::Applying", |walk| {
            matches!(walk.role_status(), Some(RoleStatusView::Applying { .. }))
        })?;
        if step == ConformanceStep::AttemptIssued {
            self.report.record(step);
        }
        Ok(())
    }

    /// Advance until the kernel publishes a session, then check the driver actually kept it.
    fn session_established(&mut self) -> Result<SessionRef, ConformanceFailure> {
        let session = self.wait_for_session(ConformanceStep::SessionEstablished)?;
        self.require_held_session(ConformanceStep::SessionEstablished, session)?;
        self.report.record(ConformanceStep::SessionEstablished);
        Ok(session)
    }

    /// Assert the presentation this subject's declared flow promises, and no other.
    fn flow_read(&mut self) -> Result<(), ConformanceFailure> {
        let step = ConformanceStep::FlowRead;
        match Subject::FLOW {
            FlowExpectation::NotMonitored => {
                let expected = "Connected(FlowNotMonitored)";
                self.advance_until(step, expected, |walk| {
                    matches!(
                        walk.presentation(),
                        Some(RolePresentationView::Connected(
                            ConnectedCause::FlowNotMonitored
                        ))
                    )
                })?;
            },
            FlowExpectation::Continuous(_) => self.advance_until_presenting(step)?,
        }
        self.report.record(step);
        Ok(())
    }

    /// Take the device away and require the established session to be released as unavailable.
    fn session_ended_on_departure(
        &mut self,
        established: SessionRef,
    ) -> Result<(), ConformanceFailure> {
        let step = ConformanceStep::SessionEndedOnDeparture;
        let cursor = self.cleanup_cursor();
        self.admit_scans_until(step, "a role with no established session", |walk| {
            !matches!(walk.role_status(), Some(RoleStatusView::Established { .. }))
        })?;
        self.require_recorded_release(
            step,
            cursor,
            established,
            RequiredReleaseCause::DeviceUnavailable,
            RoleEntityAtCleanup::Live,
        )?;
        self.report.record(step);
        Ok(())
    }

    /// The departed role must say so to an operator, not merely stop being established.
    fn disconnected_during_grace(&mut self) -> Result<(), ConformanceFailure> {
        let step = ConformanceStep::DisconnectedDuringGrace;
        self.advance_until(step, "Disconnected", |walk| {
            matches!(
                walk.presentation(),
                Some(RolePresentationView::Disconnected)
            )
        })?;
        self.report.record(step);
        Ok(())
    }

    /// Bring the device back and require a session that is not the one that departed.
    fn reestablished_on_return(
        &mut self,
        departed: SessionRef,
    ) -> Result<SessionRef, ConformanceFailure> {
        let step = ConformanceStep::ReestablishedOnReturn;
        self.admit_scans_until(step, "a re-established session", |walk| {
            matches!(walk.role_status(), Some(RoleStatusView::Established { .. }))
        })?;
        let session = self.wait_for_session(step)?;
        if session == departed {
            return Err(self.refuse(
                step,
                ConformanceRefusal::ReturnedSessionReusesTheDepartedReference { session },
            ));
        }
        self.require_held_session(step, session)?;
        self.report.record(step);
        Ok(session)
    }

    /// Retire the role and require the driver's record to end with the kernel's.
    fn retirement_ended_the_session(
        &mut self,
        established: SessionRef,
    ) -> Result<(), ConformanceFailure> {
        let step = ConformanceStep::RetirementEndedTheSession;
        let cursor = self.cleanup_cursor();
        self.retire();
        self.app.update();
        self.require_retired_role(step)?;
        self.require_recorded_release(
            step,
            cursor,
            established,
            RequiredReleaseCause::RoleRetired,
            RoleEntityAtCleanup::Live,
        )?;
        self.require_ended_record(step)?;
        self.report.record(step);
        Ok(())
    }

    /// Replace the binding while the role is applying and require the attempt to be cancelled.
    ///
    /// The replacement keeps the same endpoint, so the successor binding applies against the device
    /// that is still there and the walk can carry on into the removal steps. It also reuses the
    /// displaced role entity, which is why this cancellation reaches the driver with a live one:
    /// the attempt is what a replacement displaces, never the entity.
    fn replacement_cancelled_the_attempt(&mut self) -> Result<(), ConformanceFailure> {
        let step = ConformanceStep::ReplacementCancelledTheAttempt;
        self.author_binding(step)?;
        self.attempt_issued(step)?;
        let displaced = self.applying_attempt(step)?;
        let cursor = self.cleanup_cursor();
        let endings = self.ending_cursor();
        let authoring = self.authoring();
        let successor = hana_rigging::replace_binding(self.app.world_mut(), authoring)
            .map_err(|error| self.refuse(step, ConformanceRefusal::BindingRefused(error)))?;
        self.attach_role(successor);
        self.app.update();
        self.advance_until(
            step,
            "a role no longer running the displaced attempt",
            |walk| {
                !matches!(
                    walk.role_status(),
                    Some(RoleStatusView::Applying { attempt, .. }) if *attempt == displaced
                )
            },
        )?;
        self.require_recorded_cancellation(
            step,
            cursor,
            displaced,
            AttemptInvalidation::BindingReplaced,
            RoleEntityAtCleanup::Live,
        )?;
        self.require_published_ending(
            step,
            endings,
            displaced,
            AttemptInvalidationView::BindingReplaced,
        )?;
        self.report.record(step);
        Ok(())
    }

    /// Retire an established role and despawn its entity in one frame.
    ///
    /// This is how an application takes a role entity away for good, and the only way the cleanup
    /// reaches a driver with no live entity to read: a plain despawn of a still-registered role is
    /// re-spawned by recovery and arrives live.
    fn removed_role_entity_ended_the_session(&mut self) -> Result<(), ConformanceFailure> {
        let step = ConformanceStep::RemovedRoleEntityEndedTheSession;
        let session = self.wait_for_session(step)?;
        self.require_held_session(step, session)?;
        let established = self.role_entity(step)?;
        let cursor = self.cleanup_cursor();
        self.retire();
        self.app.world_mut().entity_mut(established).despawn();
        self.app.update();
        self.require_retired_role(step)?;
        self.require_recorded_release(
            step,
            cursor,
            session,
            RequiredReleaseCause::RoleRetired,
            RoleEntityAtCleanup::Removed,
        )?;
        self.require_ended_record(step)?;
        self.report.record(step);
        Ok(())
    }

    /// The same removal against a role that is still applying rather than established.
    fn removed_role_entity_cancelled_the_attempt(&mut self) -> Result<(), ConformanceFailure> {
        let step = ConformanceStep::RemovedRoleEntityCancelledTheAttempt;
        self.author_binding(step)?;
        self.attempt_issued(step)?;
        let attempt = self.applying_attempt(step)?;
        let applying = self.role_entity(step)?;
        let cursor = self.cleanup_cursor();
        let endings = self.ending_cursor();
        self.retire();
        self.app.world_mut().entity_mut(applying).despawn();
        self.app.update();
        self.require_retired_role(step)?;
        self.require_recorded_cancellation(
            step,
            cursor,
            attempt,
            AttemptInvalidation::RoleRetired,
            RoleEntityAtCleanup::Removed,
        )?;
        self.require_published_ending(
            step,
            endings,
            attempt,
            AttemptInvalidationView::RoleRetired,
        )?;
        self.require_ended_record(step)?;
        self.report.record(step);
        Ok(())
    }

    /// Strand the recorded role entity, then take the device away.
    ///
    /// Taking [`RoleKey`] off the entity before despawning it is what strands the id the kernel
    /// records: the despawn hook reads the key off the entity to record the loss, so an entity
    /// without one goes with nothing noticing and recovery never re-spawns it. Every later cleanup
    /// has to ask the world rather than trust the record, and this is the step that proves it — the
    /// release arrives with [`DriverCleanupRoleEntity::Removed`] for an id [`Bindings`] still
    /// answers with.
    fn stranded_role_entity_ended_the_session(&mut self) -> Result<(), ConformanceFailure> {
        let step = ConformanceStep::StrandedRoleEntityEndedTheSession;
        self.author_binding(step)?;
        let session = self.wait_for_session(step)?;
        self.require_held_session(step, session)?;
        let established = self.role_entity(step)?;
        let cursor = self.cleanup_cursor();
        self.app
            .world_mut()
            .entity_mut(established)
            .remove::<RoleKey>();
        self.app.world_mut().entity_mut(established).despawn();
        self.app.update();

        let role = self.role.clone();
        self.admit_scans_until(step, "a driver record with no session", move |walk| {
            matches!(
                walk.subject.session(walk.app.world(), &role),
                SessionLookup::NotEstablished
            )
        })?;
        self.require_recorded_release(
            step,
            cursor,
            session,
            RequiredReleaseCause::DeviceUnavailable,
            RoleEntityAtCleanup::Removed,
        )?;
        self.report.record(step);
        Ok(())
    }

    // ---- shared machinery ----

    /// Author one binding for the walk's role and hand the subject the entity it reserved.
    fn author_binding(&mut self, step: ConformanceStep) -> Result<(), ConformanceFailure> {
        let authoring = self.authoring();
        let role_entity = hana_rigging::register_binding(self.app.world_mut(), authoring)
            .map_err(|error| self.refuse(step, ConformanceRefusal::BindingRefused(error)))?;
        self.attach_role(role_entity);
        Ok(())
    }

    /// The authoring every binding in the walk is built from.
    fn authoring(&self) -> BindingAuthoring<Subject::Configuration> {
        BindingAuthoring::new(
            self.role.clone(),
            self.endpoint.clone(),
            self.driver,
            self.subject.requested(),
            policy::<Subject>(),
        )
    }

    fn attach_role(&mut self, role_entity: Entity) {
        let world = self.app.world_mut();
        self.subject.attach_role(world, role_entity);
    }

    fn retire(&mut self) {
        let role = self.role.clone();
        self.app.world_mut().trigger(RetireRole { role });
    }

    /// Advance until the kernel publishes a session, and return its reference.
    fn wait_for_session(
        &mut self,
        step: ConformanceStep,
    ) -> Result<SessionRef, ConformanceFailure> {
        self.advance_until(step, "RoleStatusView::Established", |walk| {
            matches!(walk.role_status(), Some(RoleStatusView::Established { .. }))
        })?;
        match self.role_status() {
            Some(RoleStatusView::Established { session, .. }) => Ok(*session),
            _ => Err(self.refuse(
                step,
                ConformanceRefusal::ObservableNeverReached {
                    expected: "RoleStatusView::Established",
                    observed: self.observed_status(),
                },
            )),
        }
    }

    /// Cross a continuous subject's first-datum bound, watching for the kernel's stall verdict.
    ///
    /// The budget alone would prove nothing: a driver that credits nothing has not presented at
    /// frame zero either, so a step that only ran out of frames would pass with
    /// `judge_continuous_flow` deleted from the kernel. The stall reading is what makes the flow
    /// step a reading of the bound rather than of the walk's own patience.
    fn advance_until_presenting(
        &mut self,
        step: ConformanceStep,
    ) -> Result<(), ConformanceFailure> {
        for _ in 0..flow_frames::<Subject>() {
            match self.presentation() {
                Some(RolePresentationView::Presenting) => return Ok(()),
                Some(RolePresentationView::Connected(ConnectedCause::Stalled(cause))) => {
                    let cause = *cause;
                    return Err(self.refuse(
                        step,
                        ConformanceRefusal::FlowStalledBeforeItPresented { cause },
                    ));
                },
                _ => {},
            }
            self.app.update();
        }
        if matches!(self.presentation(), Some(RolePresentationView::Presenting)) {
            return Ok(());
        }
        Err(self.refuse(
            step,
            ConformanceRefusal::ObservableNeverReached {
                expected: "Presenting",
                observed: self.observed(),
            },
        ))
    }

    /// The driver must be holding the exact session the kernel just published.
    fn require_held_session(
        &self,
        step: ConformanceStep,
        session: SessionRef,
    ) -> Result<(), ConformanceFailure> {
        match self.subject.session(self.app.world(), &self.role) {
            SessionLookup::Holding(held) if held == session => Ok(()),
            other => Err(self.refuse(
                step,
                ConformanceRefusal::SubjectRecordContradictsTheKernel {
                    expected: "SessionLookup::Holding, matching the published session",
                    observed: format!("{other:?}"),
                },
            )),
        }
    }

    /// The driver's record must be over once the kernel has ended the role.
    fn require_ended_record(&self, step: ConformanceStep) -> Result<(), ConformanceFailure> {
        match self.subject.session(self.app.world(), &self.role) {
            SessionLookup::NotEstablished => Ok(()),
            other => Err(self.refuse(
                step,
                ConformanceRefusal::SubjectRecordContradictsTheKernel {
                    expected: "SessionLookup::NotEstablished",
                    observed: format!("{other:?}"),
                },
            )),
        }
    }

    /// The kernel must keep no role entity for a role it has retired.
    fn require_retired_role(&self, step: ConformanceStep) -> Result<(), ConformanceFailure> {
        if self
            .app
            .world()
            .resource::<Bindings>()
            .role_entity(&self.role)
            .is_err()
        {
            return Ok(());
        }
        Err(self.refuse(
            step,
            ConformanceRefusal::ObservableNeverReached {
                expected: "a role the kernel no longer retains",
                observed: String::from("a role the kernel still retains"),
            },
        ))
    }

    /// Where the driver's cleanup record stands before a step runs its frames.
    fn cleanup_cursor(&self) -> usize { self.subject.cleanups(self.app.world(), &self.role).len() }

    /// Where the kernel's published endings stand before a step runs its frames.
    fn ending_cursor(&self) -> usize {
        self.app
            .world()
            .resource::<PublishedAttemptEndings>()
            .0
            .len()
    }

    /// This step must have handed the driver `release_session` with the cause the kernel
    /// dispatched.
    fn require_recorded_release(
        &self,
        step: ConformanceStep,
        cursor: usize,
        session: SessionRef,
        cause: RequiredReleaseCause,
        role_entity: RoleEntityAtCleanup,
    ) -> Result<(), ConformanceFailure> {
        let recorded = self.subject.cleanups(self.app.world(), &self.role);
        let matched = recorded.since(cursor).iter().any(|call| match call {
            RecordedCleanup::SessionReleased {
                role_entity: recorded_entity,
                session: recorded_session,
                cause: recorded_cause,
            } => {
                *recorded_session == session
                    && cause.matches(recorded_cause)
                    && role_entity.matches(*recorded_entity)
            },
            RecordedCleanup::AttemptCancelled { .. } => false,
        });
        if matched {
            return Ok(());
        }
        Err(self.refuse(
            step,
            ConformanceRefusal::CleanupNeverReachedTheSubject {
                expected: cause.described(),
                observed: format!("{:?}", recorded.since(cursor)),
            },
        ))
    }

    /// This step must have handed the driver `cancel_apply` for the attempt the kernel displaced.
    fn require_recorded_cancellation(
        &self,
        step: ConformanceStep,
        cursor: usize,
        attempt: AttemptRef,
        invalidation: AttemptInvalidation,
        role_entity: RoleEntityAtCleanup,
    ) -> Result<(), ConformanceFailure> {
        let recorded = self.subject.cleanups(self.app.world(), &self.role);
        let matched = recorded.since(cursor).iter().any(|call| match call {
            RecordedCleanup::AttemptCancelled {
                role_entity: recorded_entity,
                attempt: recorded_attempt,
                invalidation: recorded_invalidation,
            } => {
                *recorded_attempt == attempt
                    && *recorded_invalidation == invalidation
                    && role_entity.matches(*recorded_entity)
            },
            RecordedCleanup::SessionReleased { .. } => false,
        });
        if matched {
            return Ok(());
        }
        Err(self.refuse(
            step,
            ConformanceRefusal::CleanupNeverReachedTheSubject {
                expected: match invalidation {
                    AttemptInvalidation::BindingReplaced => "cancel_apply(BindingReplaced)",
                    _ => "cancel_apply(RoleRetired)",
                },
                observed: format!("{:?}", recorded.since(cursor)),
            },
        ))
    }

    /// The kernel must have published the ending the cleanup was dispatched for.
    ///
    /// The driver's record says the call arrived; this says the kernel told everyone else the same
    /// thing, through whichever channel the role entity's fate left open.
    fn require_published_ending(
        &self,
        step: ConformanceStep,
        cursor: usize,
        attempt: AttemptRef,
        invalidation: AttemptInvalidationView,
    ) -> Result<(), ConformanceFailure> {
        let endings = self.app.world().resource::<PublishedAttemptEndings>();
        let published = endings.0.get(cursor..).unwrap_or_default();
        let matched = published.iter().any(|ending| {
            ending.role == self.role
                && ending.attempt == attempt
                && matches!(
                    &ending.ending,
                    AttemptEndingView::Invalidated(published) if *published == invalidation
                )
        });
        if matched {
            return Ok(());
        }
        Err(self.refuse(
            step,
            ConformanceRefusal::ObservableNeverReached {
                expected: "AttemptEndingView::Invalidated naming this step's cause",
                observed: format!("{published:?}"),
            },
        ))
    }

    /// The attempt the role is currently running, for the step that must see it displaced.
    fn applying_attempt(&self, step: ConformanceStep) -> Result<AttemptRef, ConformanceFailure> {
        match self.role_status() {
            Some(RoleStatusView::Applying { attempt, .. }) => Ok(*attempt),
            _ => Err(self.refuse(
                step,
                ConformanceRefusal::ObservableNeverReached {
                    expected: "RoleStatusView::Applying",
                    observed: self.observed(),
                },
            )),
        }
    }

    fn role_entity(&self, step: ConformanceStep) -> Result<Entity, ConformanceFailure> {
        self.app
            .world()
            .resource::<Bindings>()
            .role_entity(&self.role)
            .map_err(|error| self.refuse(step, ConformanceRefusal::BindingRefused(error)))
    }

    fn resolves(&self, device: &DeviceKey) -> bool {
        matches!(
            self.app
                .world()
                .resource::<hana_rigging::Devices>()
                .resolve(device),
            hana_rigging::DeviceResolution::Resolved(_)
        )
    }

    fn role_status(&self) -> Option<&RoleStatusView> {
        let entity = self
            .app
            .world()
            .resource::<Bindings>()
            .role_entity(&self.role)
            .ok()?;
        Some(self.app.world().get::<RoleStatus>(entity)?.view())
    }

    fn presentation(&self) -> Option<&RolePresentationView<RoleUnavailableCause>> {
        let entity = self
            .app
            .world()
            .resource::<Bindings>()
            .role_entity(&self.role)
            .ok()?;
        Some(
            self.app
                .world()
                .get::<KernelRolePresentation>(entity)?
                .view(),
        )
    }

    fn observed_status(&self) -> String {
        self.role_status().map_or_else(
            || String::from("no published role status"),
            |status| format!("{status:?}"),
        )
    }

    /// Both kernel readings of the role, so a failure names what was published rather than which.
    fn observed(&self) -> String {
        let presentation = self.presentation().map_or_else(
            || String::from("no published presentation"),
            |view| format!("{view:?}"),
        );
        format!("{} / {presentation}", self.observed_status())
    }

    /// Run frames until `reached` answers true, bounded by [`STEP_FRAME_CEILING`].
    fn advance_until(
        &mut self,
        step: ConformanceStep,
        expected: &'static str,
        reached: impl Fn(&Self) -> bool,
    ) -> Result<(), ConformanceFailure> {
        for _ in 0..STEP_FRAME_CEILING {
            if reached(self) {
                return Ok(());
            }
            self.app.update();
        }
        if reached(self) {
            return Ok(());
        }
        Err(self.refuse(
            step,
            ConformanceRefusal::ObservableNeverReached {
                expected,
                observed: self.observed(),
            },
        ))
    }

    /// Admit scripted scans one at a time until `reached` answers, bounded by the release ceiling.
    ///
    /// The reporter is gated, so no scan reaches the kernel that the walk did not admit. The script
    /// holds more scans than the walk's own steps consume, and what is left over is what a
    /// discovery run the walk did not ask for spends: the kernel asks the covering reporter for a
    /// run whenever a bound role's device stops being live. Past the script the reporter repeats
    /// its last scan, which departs the presented device for good, so a walk that has spent its
    /// script says so rather than waiting on a return that can never come.
    fn admit_scans_until(
        &mut self,
        step: ConformanceStep,
        expected: &'static str,
        reached: impl Fn(&Self) -> bool,
    ) -> Result<(), ConformanceFailure> {
        for _ in 0..SCAN_RELEASE_CEILING {
            self.require_unspent_script(step)?;
            advance_until_running(&mut self.app, self.reporter)
                .map_err(|error| self.refuse(step, ConformanceRefusal::ReporterStalled(error)))?;
            self.gate.release();
            advance_until_accepted(&mut self.app, self.reporter)
                .map_err(|error| self.refuse(step, ConformanceRefusal::ReporterStalled(error)))?;
            for _ in 0..STEP_FRAME_CEILING {
                if reached(self) {
                    return Ok(());
                }
                self.app.update();
            }
        }
        Err(self.refuse(
            step,
            ConformanceRefusal::ObservableNeverReached {
                expected,
                observed: self.observed(),
            },
        ))
    }

    /// The script must still hold a scan the walk has not replayed.
    fn require_unspent_script(&self, step: ConformanceStep) -> Result<(), ConformanceFailure> {
        let accepted = crate::completed_batches(&self.app, self.reporter)
            .map_err(|error| self.refuse(step, ConformanceRefusal::ReporterStalled(error)))?;
        if usize::try_from(accepted).unwrap_or(usize::MAX) < self.scripted_scans {
            return Ok(());
        }
        Err(self.refuse(
            step,
            ConformanceRefusal::ScriptedScansExhausted {
                accepted,
                scripted: self.scripted_scans,
            },
        ))
    }

    fn refuse(&self, step: ConformanceStep, refusal: ConformanceRefusal) -> ConformanceFailure {
        ConformanceFailure {
            reached: self.report.clone(),
            stop:    ConformanceStop::At(step),
            refusal: Box::new(refusal),
        }
    }
}

/// Whether a step's cleanup must arrive with the role entity still spawned.
///
/// Only the two steps that despawn the entity and the one that strands it see
/// [`DriverCleanupRoleEntity::Removed`]; a replacement reuses the displaced entity and a plain
/// retirement never despawns, so both of those arrive live. Requiring `Removed` where the kernel
/// correctly answers `Live` would fail a conformant kernel.
#[derive(Clone, Copy, Debug)]
enum RoleEntityAtCleanup {
    /// The role entity was still spawned when the kernel dispatched the cleanup.
    Live,
    /// The role entity was gone, so the driver had only entity-free work available.
    Removed,
}

impl RoleEntityAtCleanup {
    const fn matches(self, recorded: DriverCleanupRoleEntity) -> bool {
        matches!(
            (self, recorded),
            (Self::Live, DriverCleanupRoleEntity::Live(_))
                | (Self::Removed, DriverCleanupRoleEntity::Removed)
        )
    }
}

/// The policy every binding in the walk is authored with.
///
/// `ReapplyOnReturn` because the walk's replug step is the whole point of the departure — the
/// default forgets a departed device's configuration, so the return would reopen nothing.
/// `NewRevision` because the walk drives the clock by hand, and an interval retry gate would race
/// the manual frame step rather than the reported device set.
fn policy<Subject>() -> BindingPolicy
where
    Subject: ConformanceSubject,
{
    let policy = BindingPolicy::new(
        RecoveryPolicy::ReapplyOnReturn,
        RetryOn::NewRevision,
        OnAbort::default(),
        OnSessionLoss::default(),
        ApplyDeadline::ProcessDefault,
    );
    match Subject::FLOW {
        FlowExpectation::NotMonitored => policy,
        FlowExpectation::Continuous(expectation) => policy.with_continuous_flow(expectation),
    }
}

/// Kernel bounds the walk installs before the plugin, so its manual clock cannot trip one.
///
/// The defaults are wall-clock seconds. The walk drives time by hand in fractions of a
/// millisecond, so a default departure grace or apply deadline would be crossed or never reached
/// for reasons that say nothing about the driver under test.
const fn walk_limits() -> RiggingLimits {
    RiggingLimits {
        apply_deadline:  Duration::from_secs(600),
        apply_overrun:   Duration::from_secs(600),
        departure_grace: Duration::from_secs(600),
        report_grace:    Duration::from_secs(600),
    }
}

/// Refuse a continuous subject whose declared bounds the walk's own clock cannot drive.
///
/// Both refusals read the same arithmetic from opposite ends. The pacing one fires when the frame
/// step could not be divided out of the datum gap at all, because the one-nanosecond floor bit; the
/// budget one fires when it divided cleanly but the first-datum bound is so many gaps away that
/// crossing it would take more updates than the walk will ever spend.
fn refuse_undrivable_flow_bounds<Subject>(frame_step: Duration) -> Result<(), ConformanceRefusal>
where
    Subject: ConformanceSubject,
{
    let FlowExpectation::Continuous(expectation) = Subject::FLOW else {
        return Ok(());
    };
    let maximum_datum_gap = expectation.maximum_datum_gap().duration();
    if frame_step.saturating_mul(FRAME_STEPS_PER_FLOW_BOUND) > maximum_datum_gap {
        return Err(ConformanceRefusal::FramePacingCrossesTheFlowBound {
            frame_step,
            maximum_datum_gap,
        });
    }
    let frames_required = flow_frames::<Subject>();
    if frames_required > FLOW_FRAME_CEILING {
        return Err(ConformanceRefusal::FlowBoundsExceedTheWalksFrameBudget {
            first_datum_timeout: expectation.first_datum_timeout().duration(),
            maximum_datum_gap,
            frames_required,
            frames_available: FLOW_FRAME_CEILING,
        });
    }
    Ok(())
}

/// How far the walk advances the frame clock per update.
///
/// A continuous subject declares bounds the kernel enforces, and the walk owns the clock: a step
/// large enough to cross the tightest of those bounds would stall a driver that credits every
/// frame. A subject with no data axis has nothing to cross.
fn frame_step<Subject>() -> Duration
where
    Subject: ConformanceSubject,
{
    match Subject::FLOW {
        FlowExpectation::NotMonitored => UNMONITORED_FRAME_STEP,
        FlowExpectation::Continuous(expectation) => {
            let tightest = expectation
                .first_datum_timeout()
                .duration()
                .min(expectation.maximum_datum_gap().duration());
            (tightest / FRAME_STEPS_PER_FLOW_BOUND).max(Duration::from_nanos(1))
        },
    }
}

/// How many frames the flow step spends before it calls a continuous subject silent.
///
/// Enough to cross the subject's own first-datum bound and then some: a driver that never credits
/// has to be told apart from a step that merely ran out of frames. A subject whose bounds put this
/// past [`FLOW_FRAME_CEILING`] never starts a walk, so the flow step's own budget is always one the
/// walk agreed to spend.
fn flow_frames<Subject>() -> usize
where
    Subject: ConformanceSubject,
{
    let FlowExpectation::Continuous(expectation) = Subject::FLOW else {
        return STEP_FRAME_CEILING;
    };
    let step = frame_step::<Subject>();
    let bound = expectation.first_datum_timeout().duration();
    let frames = usize::try_from(bound.as_nanos() / step.as_nanos().max(1)).unwrap_or(usize::MAX);
    frames.saturating_add(FLOW_OVERRUN_FRAMES)
}

/// The scans the walk replays: the supplied set, an empty one, the set again, then empty again.
///
/// Each state is repeated [`SCAN_STATES_REPEATED`] times, which is more than the walk's own steps
/// consume. The surplus is what absorbs a discovery run the walk did not ask for; the walk checks
/// what is left before every release, so exhausting the script reports itself rather than reading
/// as a driver that cannot re-establish.
fn script(present: ScriptedScan) -> Vec<ScriptedScan> {
    let departed = ScriptedScan::Complete(Vec::new());
    let mut scans = Vec::with_capacity(SCAN_STATES_REPEATED * 4);
    for state in [&present, &departed, &present, &departed] {
        scans.extend(std::iter::repeat_n(state.clone(), SCAN_STATES_REPEATED));
    }
    scans
}

/// Rebuild one scan with the subject's capability declaration on the presented device.
///
/// The declaration is what `required_capability` resolves against, so it belongs to the driver
/// under test rather than to whoever wrote the scan. Only the presented device is rewritten: a
/// caller's multi-device scan keeps whatever the siblings were declared with.
fn declared(
    scan: &ScriptedScan,
    presented: &DeviceKey,
    declaration: &CapabilityDeclaration,
) -> ScriptedScan {
    let redeclare = |devices: &Vec<ScriptedDevice>| -> Vec<ScriptedDevice> {
        devices
            .iter()
            .map(|device| {
                if bindable_key(device).as_ref() == Some(presented) {
                    declaration.published_on(device.clone())
                } else {
                    device.clone()
                }
            })
            .collect()
    };
    match scan {
        ScriptedScan::Complete(devices) => ScriptedScan::Complete(redeclare(devices)),
        ScriptedScan::CompleteWithProjection(devices) => {
            ScriptedScan::CompleteWithProjection(redeclare(devices))
        },
        other => other.clone(),
    }
}

/// The keyed, present device one completed scan names first.
fn presented_device(scan: &ScriptedScan) -> Result<DeviceKey, ConformanceRefusal> {
    let devices = match scan {
        ScriptedScan::Complete(devices) | ScriptedScan::CompleteWithProjection(devices) => devices,
        ScriptedScan::Deferred(_) | ScriptedScan::Failed(_) | ScriptedScan::Unsupported { .. } => {
            return Err(ConformanceRefusal::ScanIsNotACompletedSet);
        },
    };
    devices
        .iter()
        .find_map(bindable_key)
        .ok_or(ConformanceRefusal::ScanNamesNoPresentedDevice)
}

/// The durable key one scripted device can be bound to, if it names one and is there.
///
/// Reading `ScriptedDevice`'s own fields is what keeps `run`'s signature at two arguments: the
/// walk derives the scheme, the coverage and the endpoint from the scan it was handed, and only a
/// module inside this crate can see them.
fn bindable_key(device: &ScriptedDevice) -> Option<DeviceKey> {
    match (&device.reported_as, device.presence) {
        (ReportedAs::Keyed(key), Presence::Present) => Some(key.clone()),
        _ => None,
    }
}
