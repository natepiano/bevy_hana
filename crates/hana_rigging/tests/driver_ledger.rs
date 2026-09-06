//! Behaviour coverage for `DriverLedger` — the one type a driver embeds so that every kernel-issued
//! authority it is handed is retained, matched and finished in one place instead of in three
//! hand-written tables per driver.
//!
//! Every assertion here is written against the public verbs rather than the module internals, and
//! every authority the ledger holds is one the kernel issued through a real `App`: the ledger is
//! reached only through the five `EndpointDriver` callbacks and its own public verbs. Nothing in
//! this file constructs an `AttemptCompletion`, a `SessionLease` or a
//! `SessionDatumArrivalEvidence`, because none of them can be constructed — that is the property
//! the ledger exists to preserve, and a test that faked one would be testing a different type.
//!
//! Two consequences shape the fixture. The ledger lives behind an `Arc<Mutex<..>>` inside
//! [`LedgerDriver`], which is what lets a test call a verb from outside a callback — the same shape
//! a real driver uses when a per-frame system credits flow against a session the kernel established
//! several frames earlier. And [`PanelPlacement`] deliberately does not implement `Clone`: the
//! specification says no `Clone` bound exists on `Configuration` or `Applied`, and a configuration
//! that cannot be cloned is what turns that sentence into something the compiler checks.
//!
//! The flow arms are asserted through the kernel's published `ContinuousFlowView` rather than
//! through the verb's return value alone. A `credit_flow` that answered `CreditedToSession` while
//! crediting nothing would pass a return-value test and leave an operator reading a session that
//! says it is awaiting a first datum it already received.
//!
//! # Two vocabularies, kept apart
//!
//! *Record* is the ledger's word for the driver's own per-attempt and per-session value —
//! [`PanelAttemptWork`] and [`PanelSessionWork`] here, a restore preparation and an established
//! window in the clerestory driver. Neither is `Clone` or `Copy`, so every hand-back this file
//! asserts is a move the compiler proved rather than a copy the ledger might also have kept.
//!
//! *View* is this file's word for a `Copy` mirror of one answer the ledger gave —
//! [`EstablishmentView`], [`FinishView`], [`PanelAttemptView`] and the rest. The mirrors exist
//! because the generic answers carry a driver's records and therefore cannot be `Copy` or
//! `PartialEq`, and because a callback's answer has to survive in a `Vec` until a test reads it.
//! Every mirror is built by moving or borrowing the real answer, so a mirror that names a record
//! is a record the ledger really handed over.

use std::error::Error;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::ecs::reflect::ReflectComponent;
use bevy::platform::time::Instant;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::World;
use bevy::time::Real;
use bevy::time::Time;
use bevy::time::TimeUpdateStrategy;
use hana_rigging::prelude::Applied;
use hana_rigging::prelude::ApplyContext;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::AttemptFinish;
use hana_rigging::prelude::AttemptInvalidation;
use hana_rigging::prelude::AttemptLookup;
use hana_rigging::prelude::AttemptRecord;
use hana_rigging::prelude::AttemptRecordMut;
use hana_rigging::prelude::AttemptRef;
use hana_rigging::prelude::AttemptSearch;
use hana_rigging::prelude::AttemptSucceeded;
use hana_rigging::prelude::AuthoritativeReporterCoverage;
use hana_rigging::prelude::BegunAttempt;
use hana_rigging::prelude::BindingAuthoring;
use hana_rigging::prelude::BindingPolicy;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::CancelledAttempt;
use hana_rigging::prelude::ConfigurationChange;
use hana_rigging::prelude::ContinuousFlowExpectation;
use hana_rigging::prelude::ContinuousFlowView;
use hana_rigging::prelude::CoveredDeviceIdentitySpace;
use hana_rigging::prelude::CurrentRoleRecord;
use hana_rigging::prelude::CurrentRoleRecordMut;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceTransport;
use hana_rigging::prelude::DiscardedSession;
use hana_rigging::prelude::DiscoveryCadence;
use hana_rigging::prelude::DriverAbortReason;
use hana_rigging::prelude::DriverCleanupRoleEntity;
use hana_rigging::prelude::DriverLedger;
use hana_rigging::prelude::DriverRecords;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointDriverRegistration;
use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::EstablishedContext;
use hana_rigging::prelude::EstablishedFlowView;
use hana_rigging::prelude::Establishing;
use hana_rigging::prelude::Establishment;
use hana_rigging::prelude::EstablishmentRefusal;
use hana_rigging::prelude::FirstDatumTimeout;
use hana_rigging::prelude::FlowCredit;
use hana_rigging::prelude::HeldSession;
use hana_rigging::prelude::HeldSessionMut;
use hana_rigging::prelude::InFlightAttempt;
use hana_rigging::prelude::InFlightAttemptMut;
use hana_rigging::prelude::LossReport;
use hana_rigging::prelude::MaximumDatumGap;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::ReleasedLease;
use hana_rigging::prelude::ReporterActivation;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterHealth;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingPlugin;
use hana_rigging::prelude::RoleAttemptRecord;
use hana_rigging::prelude::RoleAttemptRecordMut;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleRecords;
use hana_rigging::prelude::RoleRecordsMut;
use hana_rigging::prelude::RoleSessionRecord;
use hana_rigging::prelude::RoleSessionRecordMut;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::SessionLookup;
use hana_rigging::prelude::SessionRecord;
use hana_rigging::prelude::SessionRecordMut;
use hana_rigging::prelude::SessionRef;
use hana_rigging::prelude::SessionReleaseCause;
use hana_rigging::prelude::TargetResolution;
use hana_rigging::prelude::TargetResolutionContext;
use hana_rigging::prelude::TransportObservation;
use hana_rigging::prelude::register_binding;
use hana_rigging_scripted::ScriptedDevice;
use hana_rigging_scripted::ScriptedReporter;
use hana_rigging_scripted::ScriptedScan;
use hana_rigging_scripted::advance_reporter;
use hana_rigging_scripted::reported_key;
use hana_rigging_scripted::scan;

/// Identity space every scripted panel in this suite is named in.
const PANEL_SCHEME: &str = "usb-serial";

/// Slot every scripted role asks its driver for.
const REQUESTED_SLOT: u32 = 1;

/// How far the fixture advances the frame clock per update.
///
/// Small enough that no flow bound in this suite can expire while a test is stepping frames, and
/// non-zero so the kernel's frame clock reads as measurable rather than not-yet-advanced.
const FRAME_STEP: Duration = Duration::from_millis(1);

/// Flow bounds generous enough that nothing in this suite stalls for want of stepping frames.
///
/// A stall would be indistinguishable from a credit the ledger dropped, so the bounds are set well
/// past any frame budget a test here spends.
const FLOW_BOUND: Duration = Duration::from_secs(30);

/// How many frames a fixture spends waiting for one kernel-driven condition.
const FRAME_CEILING: usize = 16;

/// Scans that keep reporting the panel before a replug script takes it away.
///
/// The kernel asks a reporter for runs of its own, so a departure has to sit far enough back in
/// the script that establishing the first session cannot reach it.
const REPLUG_PRESENT_PREFIX: usize = 8;

/// Placement the scripted role asks its driver for.
///
/// Deliberately not `Clone`: the ledger keeps nothing from `Applied`, so no verb may require one.
#[derive(Component, Debug, PartialEq, Reflect)]
#[reflect(Component)]
struct PanelPlacement {
    slot: u32,
}

/// The driver's own record for one attempt, held by the ledger for the whole of the attempt.
///
/// It stands in for what the three shipped drivers really keep: the window driver's restore
/// preparation and the window it left markers on, the screen and camera kernels' hardware opened
/// inside `start_apply` and filled from a worker thread. Its life starts at `begin_attempt`,
/// before any lease exists, which is the whole reason the record lives on the attempt slot rather
/// than on the lease.
///
/// Deliberately neither `Clone` nor `Copy`. A record is hardware the driver owns, and a hand-back a
/// test could satisfy with a copy would not prove the ledger let go of it.
#[derive(Debug, PartialEq, Eq)]
struct PanelAttemptWork {
    /// Which `start_apply` opened this record, so a record handed back names where it came from.
    opened_by: u32,
    /// Whether the scripted hardware this record stands for is still usable.
    hardware:  PanelHardware,
}

/// Whether the hardware one attempt record stands for is still usable.
///
/// The window driver's `CompletionWindowLifetime` in miniature. `abort_window` walks every record
/// the ledger holds and marks the ones naming its window as ended, and `establish` later reads that
/// mark as the judgement it gives `establish_lease`. A ledger that would not let a driver reach a
/// record in place could not carry that mark from the abort to the establishment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelHardware {
    /// The attempt's hardware is open and an establishment may convert it into a session.
    Open,
    /// The attempt's hardware ended while the attempt was still filed.
    Ended,
}

/// The driver's own record for one established session.
///
/// Built from [`PanelAttemptWork`] by the conversion `establish_lease` takes, so the session
/// carries which attempt opened the hardware it now owns — the fact a predecessor hand-back has
/// to carry for a driver to end the right device work rather than the successor's.
#[derive(Debug, PartialEq, Eq)]
struct PanelSessionWork {
    /// The attempt record this session was converted from.
    opened_by:     u32,
    /// How many times a per-frame sweep has polled this session through `sessions_mut`.
    ///
    /// The six per-frame loops the shipped kernels run over every open session, reduced to a
    /// counter: a sweep that could not reach a record in place could not credit anything.
    frames_polled: u32,
    /// What the driver froze into this record when the kernel released its session.
    ///
    /// The camera's frozen picture in miniature: its `release_session` suspends the stream, writes
    /// the last thing it knew through `session_record_mut`, and leaves the record retained, so a
    /// successor inherits a picture rather than a blank panel.
    suspended:     SuspendedPicture,
}

/// What one session left in its record when its release suspended it.
///
/// A mark only a release could have written, which is what makes an adopted record traceable to
/// the session that was really released rather than to a default a ledger could have fabricated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SuspendedPicture {
    /// The session was never suspended, so it left nothing behind for a successor.
    StillStreaming,
    /// The release froze the last frame this session had polled, and a successor that adopts the
    /// record adopts that frame with it.
    FrozenAtFrame(u32),
}

/// The record types this suite's driver keeps in its ledger.
///
/// Naming both associated types with real values rather than `()` is what makes the suite exercise
/// the generic ledger instead of the `NoRecords` default the kernels keep.
struct PanelRecords;

impl DriverRecords for PanelRecords {
    type Attempt = PanelAttemptWork;
    type Session = PanelSessionWork;
}

/// A readable copy of one [`PanelAttemptWork`] the ledger held or handed back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PanelAttemptView {
    opened_by: u32,
    hardware:  PanelHardware,
}

impl PanelAttemptView {
    const fn of(work: &PanelAttemptWork) -> Self {
        Self {
            opened_by: work.opened_by,
            hardware:  work.hardware,
        }
    }

    /// The view of a record opened by that `start_apply` whose hardware is still open.
    const fn open(opened_by: u32) -> Self {
        Self {
            opened_by,
            hardware: PanelHardware::Open,
        }
    }

    /// The view of a record opened by that `start_apply` whose hardware has since ended.
    const fn ended(opened_by: u32) -> Self {
        Self {
            opened_by,
            hardware: PanelHardware::Ended,
        }
    }
}

/// A readable copy of one [`PanelSessionWork`] the ledger held or handed back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PanelSessionView {
    opened_by:     u32,
    frames_polled: u32,
    suspended:     SuspendedPicture,
}

impl PanelSessionView {
    const fn of(work: &PanelSessionWork) -> Self {
        Self {
            opened_by:     work.opened_by,
            frames_polled: work.frames_polled,
            suspended:     work.suspended,
        }
    }

    /// The view of a session converted from that attempt record and never polled.
    const fn unpolled(opened_by: u32) -> Self {
        Self {
            opened_by,
            frames_polled: 0,
            suspended: SuspendedPicture::StillStreaming,
        }
    }

    /// The view of a session polled that many times and then frozen there by its own release.
    ///
    /// The two marks together are what no other record in a suite can carry: a fresh session has
    /// polled nothing and been suspended by nothing, and a successor's has its own `opened_by`.
    const fn frozen(opened_by: u32, frames_polled: u32) -> Self {
        Self {
            opened_by,
            frames_polled,
            suspended: SuspendedPicture::FrozenAtFrame(frames_polled),
        }
    }
}

/// Turn the attempt record the kernel accepted into the session record the driver keeps.
///
/// The one conversion this suite gives `establish_lease`, written once so every establishment in
/// the file converts identically and a session's `opened_by` is always traceable to the
/// `start_apply` that opened its hardware.
const fn into_session_work(work: PanelAttemptWork) -> PanelSessionWork {
    PanelSessionWork {
        opened_by:     work.opened_by,
        frames_polled: 0,
        suspended:     SuspendedPicture::StillStreaming,
    }
}

/// One observation of the scripted panel's transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelObservation {
    /// A datum reached a consumer.
    Datum,
    /// The transport was alive and carried nothing to present.
    Activity,
    /// Nothing was observed.
    Quiet,
}

/// The scripted panel's transport, so `credit_flow` has a real classification to route.
struct PanelTransport;

impl DeviceTransport for PanelTransport {
    type Observation = PanelObservation;

    fn classify(observation: &Self::Observation) -> TransportObservation {
        match observation {
            PanelObservation::Datum => TransportObservation::DatumDelivered,
            PanelObservation::Activity => TransportObservation::ActivityWithoutDatum,
            PanelObservation::Quiet => TransportObservation::Quiet,
        }
    }
}

/// What the driver reports about its own hardware at establishment.
///
/// A mirror of [`Establishing`] rather than the value itself, so the script can be re-read across
/// several establishments without depending on which derives the kernel type carries.
#[derive(Clone, Debug)]
enum EstablishingScript {
    /// The hardware is live.
    Live,
    /// The hardware died between the queued completion and this callback.
    Ended(DeviceAccessError),
}

impl EstablishingScript {
    fn build(&self) -> Establishing {
        match self {
            Self::Live => Establishing::Live,
            Self::Ended(error) => Establishing::Ended(error.clone()),
        }
    }
}

/// Whether the driver cancels the attempt inside `established` before establishing.
#[derive(Clone, Copy, Debug)]
enum EstablishedCancel {
    BeforeEstablish,
    Never,
}

/// Whether `release_session` hands the release through to the ledger.
#[derive(Clone, Copy, Debug)]
enum TeardownRelease {
    Released,
    Withheld,
}

/// What the scripted driver's callbacks do with the ledger this run.
///
/// Each field exists to reach one race the specification names from outside the crate, where the
/// kernel is the only thing that can hand a driver a context.
#[derive(Clone, Debug)]
struct LedgerScript {
    /// What `established` tells the ledger about the hardware.
    establishing:                EstablishingScript,
    /// Cancel the attempt inside `established` before establishing.
    ///
    /// This is the only way an external test can present the ledger with an establishment whose
    /// slot is not `CompletionQueued`: the kernel will not call `established` without having
    /// accepted a queued success first.
    established_cancel:          EstablishedCancel,
    /// Whether `release_session` hands the release through to the ledger.
    ///
    /// A driver that has not yet released is what leaves a `Holding` predecessor in place for the
    /// next establishment to find.
    teardown_release:            TeardownRelease,
    /// What `cancel_apply` reports into the ledger when the kernel ends an in-flight attempt.
    attempt_teardown:            AttemptTeardownReport,
    /// Observations credited inside `start_apply`, before any lease exists.
    credit_before_establishment: Vec<PanelObservation>,
    /// What the driver does with the session record `release_lease` leaves retained.
    after_release:               RetentionScript,
}

/// What the driver does with the session record a release leaves behind.
///
/// `release_lease` always retains, so this is the choice every driver makes right after it: the
/// screen keeps nothing past a release and discards at once, while the camera keeps a frozen
/// picture and the window driver keeps a hidden window for a successor to establish over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetentionScript {
    /// Discard the retained record in the same call, the shape a driver that keeps nothing uses.
    DiscardAtOnce,
    /// Leave the record retained for a successor to establish over or a later discard to end.
    ///
    /// The window driver's shape: it keeps a hidden window and writes nothing more into the
    /// record, because the window is the whole of what it kept.
    KeepRetained,
    /// Suspend the session into the record the release just retained, then keep it.
    ///
    /// The camera's shape, and the only one that writes into a record after its release: the
    /// stream closes, the last picture it had goes into the record through `session_record_mut`,
    /// and the successor establishes over a picture instead of a blank panel.
    SuspendAndKeep,
}

/// What the driver tells its ledger when the kernel ends an attempt that is still in flight.
///
/// The kernel has one path that ends an in-flight attempt without ever calling `cancel_apply` — a
/// role entity despawned outside the kernel — and no scripted reporter can produce it. The two
/// non-cancelling reports reproduce the two ledger states that path leaves behind, which is what
/// gives the role's next `start_apply` an abandoned slot to supersede.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptTeardownReport {
    /// The cancellation reaches the ledger, which is what an undisturbed kernel teardown looks
    /// like: the slot returns to idle and nothing is left for a successor to displace.
    Cancelled,
    /// Nothing reaches the ledger, leaving the slot `Applying` for an attempt the kernel has
    /// already abandoned.
    Withheld,
    /// The driver's device work landed in the same frame the kernel ended the attempt, so it
    /// reports the success it really had. The ledger queues a completion the kernel will never
    /// establish, leaving the slot `CompletionQueued` and just as abandoned.
    SucceededInstead,
}

impl Default for LedgerScript {
    fn default() -> Self {
        Self {
            establishing:                EstablishingScript::Live,
            established_cancel:          EstablishedCancel::Never,
            teardown_release:            TeardownRelease::Released,
            attempt_teardown:            AttemptTeardownReport::Cancelled,
            credit_before_establishment: Vec::new(),
            after_release:               RetentionScript::DiscardAtOnce,
        }
    }
}

/// Endpoint driver whose whole kernel-authority bookkeeping is one [`DriverLedger`].
///
/// The five methods are what the specification says a driver becomes once the ledger holds its
/// authorities: a hardware call — here, a scripted one — plus one ledger verb.
struct LedgerDriver {
    state: Arc<Mutex<LedgerDriverState>>,
}

/// Test-side handle onto the same ledger the registered driver holds.
///
/// A real driver reaches its ledger from a per-frame system; a test reaches it from here. Both are
/// outside a callback, which is the point: only `begin_attempt` and `establish_lease` need one.
#[derive(Clone)]
struct LedgerControl {
    state: Arc<Mutex<LedgerDriverState>>,
}

struct LedgerDriverState {
    ledger:         DriverLedger<PanelPlacement, PanelRecords>,
    script:         LedgerScript,
    /// Stamped onto the next attempt record the driver opens.
    ///
    /// The record's identity has to come from the driver, not from the ledger: a driver opens its
    /// hardware inside `start_apply` and hands the record over in the same call, before any
    /// [`AttemptRef`] exists to key it by.
    next_serial:    u32,
    /// What each `begin_attempt` answered, in dispatch order.
    ///
    /// Recorded beside `started` rather than derived from it, because the answer carries what
    /// `started` cannot: the attempt this one displaced, which is the only place a driver can
    /// learn that a per-attempt record of its own is now orphaned.
    begins:         Vec<BegunAttemptView>,
    /// Attempts the ledger began, in dispatch order.
    started:        Vec<AttemptRef>,
    /// What each `established` callback answered.
    establishments: Vec<EstablishmentView>,
    /// What each kernel cancellation answered.
    cancellations:  Vec<CancelView>,
    /// What each success the driver reported from inside a teardown answered.
    successes:      Vec<AttemptSucceeded>,
    /// What each kernel release answered, for the runs that pass releases through.
    releases:       Vec<ReleaseView>,
    /// What each discard of a retained session record answered.
    ///
    /// Recorded beside the releases because the two are one act split in half: a release retains,
    /// and only a discard ends the driver's record of that session.
    discards:       Vec<DiscardView>,
    /// How many times the kernel has torn an attempt or a session down.
    ///
    /// A script that withholds its release still counts here, which is what lets a test watch for
    /// a departure the kernel drove without also deciding what the driver did about it.
    teardowns:      usize,
}

/// A readable copy of one [`Establishment`], recorded as the callback returns it.
///
/// The two hand-back arms name the record they were given rather than dropping it, because a
/// predecessor's record is the driver's own hardware and an establishment that answered the right
/// arm while handing back the wrong record would leave the driver ending the successor's device
/// work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EstablishmentView {
    Established(SessionRef),
    EstablishedOverUnreleased {
        session:     SessionRef,
        predecessor: PanelSessionView,
    },
    EstablishedOverRetained {
        session:  SessionRef,
        retained: PanelSessionView,
    },
    RefusedNoQueuedCompletion,
    RefusedSessionEnded(PanelAttemptView),
}

impl EstablishmentView {
    const fn of(establishment: &Establishment<PanelAttemptWork, PanelSessionWork>) -> Self {
        match establishment {
            Establishment::Established { session } => Self::Established(*session),
            Establishment::EstablishedOverUnreleased {
                session,
                predecessor,
            } => Self::EstablishedOverUnreleased {
                session:     *session,
                predecessor: PanelSessionView::of(predecessor),
            },
            Establishment::EstablishedOverRetained { session, retained } => {
                Self::EstablishedOverRetained {
                    session:  *session,
                    retained: PanelSessionView::of(retained),
                }
            },
            Establishment::Refused(EstablishmentRefusal::NoQueuedCompletion) => {
                Self::RefusedNoQueuedCompletion
            },
            Establishment::Refused(EstablishmentRefusal::SessionEnded { record, .. }) => {
                Self::RefusedSessionEnded(PanelAttemptView::of(record))
            },
        }
    }
}

/// A readable copy of one [`CancelledAttempt`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancelView {
    Applying(PanelAttemptView),
    CompletionQueued(PanelAttemptView),
    WrongRole,
    Unknown,
}

impl CancelView {
    const fn of(cancelled: &CancelledAttempt<PanelAttemptWork>) -> Self {
        match cancelled {
            CancelledAttempt::Applying { record } => Self::Applying(PanelAttemptView::of(record)),
            CancelledAttempt::CompletionQueued { record } => {
                Self::CompletionQueued(PanelAttemptView::of(record))
            },
            CancelledAttempt::WrongRole => Self::WrongRole,
            CancelledAttempt::Unknown => Self::Unknown,
        }
    }
}

/// A readable copy of one [`DiscardedSession`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiscardView {
    Discarded(PanelSessionView),
    NothingRetained,
}

impl DiscardView {
    const fn of(discarded: &DiscardedSession<PanelSessionWork>) -> Self {
        match discarded {
            DiscardedSession::Discarded(record) => Self::Discarded(PanelSessionView::of(record)),
            DiscardedSession::NothingRetained => Self::NothingRetained,
        }
    }
}

/// A readable copy of one [`ReleasedLease`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReleaseView {
    Released,
    OtherSessionEstablished,
    NotEstablished,
}

impl ReleaseView {
    const fn of(released: ReleasedLease) -> Self {
        match released {
            ReleasedLease::Released => Self::Released,
            ReleasedLease::OtherSessionEstablished => Self::OtherSessionEstablished,
            ReleasedLease::NotEstablished => Self::NotEstablished,
        }
    }
}

/// A readable copy of one [`AttemptFinish`] — the answer a failure or an abort gets.
///
/// `Finished` names the record it was handed, which is what separates this answer from
/// [`AttemptSucceeded::Succeeded`]: a success leaves the record in the queued slot for
/// establishment to convert, and only a non-success gives it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FinishView {
    Finished(PanelAttemptView),
    AlreadyFinished,
    AttemptUnknown,
}

impl FinishView {
    const fn of(finish: &AttemptFinish<PanelAttemptWork>) -> Self {
        match finish {
            AttemptFinish::Finished { record } => Self::Finished(PanelAttemptView::of(record)),
            AttemptFinish::AlreadyFinished => Self::AlreadyFinished,
            AttemptFinish::AttemptUnknown => Self::AttemptUnknown,
        }
    }
}

/// A readable copy of one [`FlowCredit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CreditView {
    CreditedToSession,
    RetainedForEstablishment,
    SessionEnding,
    UnknownRole,
}

impl CreditView {
    const fn of(credit: FlowCredit) -> Self {
        match credit {
            FlowCredit::CreditedToSession => Self::CreditedToSession,
            FlowCredit::RetainedForEstablishment => Self::RetainedForEstablishment,
            FlowCredit::SessionEnding => Self::SessionEnding,
            FlowCredit::UnknownRole => Self::UnknownRole,
        }
    }
}

/// A readable copy of one [`ConfigurationChange`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigurationView {
    Applied,
    SessionEnding,
    NotEstablished,
}

impl ConfigurationView {
    const fn of(change: ConfigurationChange) -> Self {
        match change {
            ConfigurationChange::Applied => Self::Applied,
            ConfigurationChange::SessionEnding => Self::SessionEnding,
            ConfigurationChange::NotEstablished => Self::NotEstablished,
        }
    }
}

/// A readable copy of one [`LossReport`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LossView {
    Reported,
    AlreadyReported,
    NotEstablished,
}

impl LossView {
    const fn of(report: LossReport) -> Self {
        match report {
            LossReport::Reported => Self::Reported,
            LossReport::AlreadyReported => Self::AlreadyReported,
            LossReport::NotEstablished => Self::NotEstablished,
        }
    }
}

/// A readable copy of one [`BegunAttempt`].
///
/// `Superseding` names the displaced record as well as the displaced reference, because the
/// reference alone is what the ledger could already say before records existed: the record is the
/// hardware the abandoned attempt left open, and this answer is the only occasion a driver has to
/// take it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BegunAttemptView {
    Fresh(AttemptRef),
    Superseding {
        attempt:    AttemptRef,
        superseded: AttemptRef,
        displaced:  PanelAttemptView,
    },
}

impl BegunAttemptView {
    const fn of(begun: &BegunAttempt<PanelAttemptWork>) -> Self {
        match begun {
            BegunAttempt::Fresh(attempt) => Self::Fresh(*attempt),
            BegunAttempt::Superseding {
                attempt,
                superseded,
                displaced,
            } => Self::Superseding {
                attempt:    *attempt,
                superseded: *superseded,
                displaced:  PanelAttemptView::of(displaced),
            },
        }
    }
}

/// A readable copy of one [`AttemptRecord`], for the by-attempt read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptRecordView {
    Applying(PanelAttemptView),
    CompletionQueued(PanelAttemptView),
    AttemptUnknown,
}

impl AttemptRecordView {
    const fn of(record: &AttemptRecord<'_, PanelAttemptWork>) -> Self {
        match record {
            AttemptRecord::Applying(work) => Self::Applying(PanelAttemptView::of(work)),
            AttemptRecord::CompletionQueued(work) => {
                Self::CompletionQueued(PanelAttemptView::of(work))
            },
            AttemptRecord::AttemptUnknown => Self::AttemptUnknown,
        }
    }

    /// The same reading taken from the mutable read, so one assertion covers both.
    const fn of_mut(record: &AttemptRecordMut<'_, PanelAttemptWork>) -> Self {
        match record {
            AttemptRecordMut::Applying(work) => Self::Applying(PanelAttemptView::of(work)),
            AttemptRecordMut::CompletionQueued(work) => {
                Self::CompletionQueued(PanelAttemptView::of(work))
            },
            AttemptRecordMut::AttemptUnknown => Self::AttemptUnknown,
        }
    }
}

/// A readable copy of one [`SessionRecord`], for the by-role read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionRecordView {
    Holding(PanelSessionView),
    LossReported(PanelSessionView),
    Retained(PanelSessionView),
    NoSession,
}

impl SessionRecordView {
    const fn of(record: &SessionRecord<'_, PanelSessionWork>) -> Self {
        match record {
            SessionRecord::Holding(work) => Self::Holding(PanelSessionView::of(work)),
            SessionRecord::LossReported(work) => Self::LossReported(PanelSessionView::of(work)),
            SessionRecord::Retained(work) => Self::Retained(PanelSessionView::of(work)),
            SessionRecord::NoSession => Self::NoSession,
        }
    }

    /// The same reading taken from the mutable read, so one assertion covers both.
    const fn of_mut(record: &SessionRecordMut<'_, PanelSessionWork>) -> Self {
        match record {
            SessionRecordMut::Holding(work) => Self::Holding(PanelSessionView::of(work)),
            SessionRecordMut::LossReported(work) => Self::LossReported(PanelSessionView::of(work)),
            SessionRecordMut::Retained(work) => Self::Retained(PanelSessionView::of(work)),
            SessionRecordMut::NoSession => Self::NoSession,
        }
    }
}

/// A readable copy of the attempt half of one role-keyed two-slot view.
///
/// This is deliberately distinct from [`AttemptRecordView`]: an empty role slot is ordinary and
/// says `NoRecord`, while an attempt-keyed lookup that misses says the named attempt is unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoleAttemptRecordView {
    Applying(PanelAttemptView),
    CompletionQueued(PanelAttemptView),
    NoRecord,
}

/// A readable copy of the session half of one role-keyed two-slot view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoleSessionRecordView {
    Holding(PanelSessionView),
    LossReported(PanelSessionView),
    Retained(PanelSessionView),
    NoRecord,
}

/// Both driver records the ledger can hold for one role at the same time.
///
/// An owned view lets the tests compare the shared and mutable APIs after their borrows end. It is
/// also the assertion shape that prevents a replacement attempt from hiding its retained
/// predecessor: the two records have distinct fields and neither lookup can stand in for the
/// other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RoleRecordsView {
    attempt: RoleAttemptRecordView,
    session: RoleSessionRecordView,
}

/// The one record a simple role lookup treats as current.
///
/// Unlike [`RoleRecordsView`], this intentionally chooses one slot. Keeping the view separate is
/// what makes the precedence assertion visible: an in-flight replacement is current while its
/// predecessor remains independently visible in the two-slot view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurrentRoleRecordView {
    Applying(PanelAttemptView),
    CompletionQueued(PanelAttemptView),
    Holding(PanelSessionView),
    LossReported(PanelSessionView),
    Retained(PanelSessionView),
    NoRecord,
}

impl CurrentRoleRecordView {
    const fn of(record: CurrentRoleRecord<'_, PanelAttemptWork, PanelSessionWork>) -> Self {
        match record {
            CurrentRoleRecord::Applying(work) => Self::Applying(PanelAttemptView::of(work)),
            CurrentRoleRecord::CompletionQueued(work) => {
                Self::CompletionQueued(PanelAttemptView::of(work))
            },
            CurrentRoleRecord::Holding(work) => Self::Holding(PanelSessionView::of(work)),
            CurrentRoleRecord::LossReported(work) => Self::LossReported(PanelSessionView::of(work)),
            CurrentRoleRecord::Retained(work) => Self::Retained(PanelSessionView::of(work)),
            CurrentRoleRecord::NoRecord => Self::NoRecord,
        }
    }

    const fn of_mut(record: CurrentRoleRecordMut<'_, PanelAttemptWork, PanelSessionWork>) -> Self {
        match record {
            CurrentRoleRecordMut::Applying(work) => Self::Applying(PanelAttemptView::of(work)),
            CurrentRoleRecordMut::CompletionQueued(work) => {
                Self::CompletionQueued(PanelAttemptView::of(work))
            },
            CurrentRoleRecordMut::Holding(work) => Self::Holding(PanelSessionView::of(work)),
            CurrentRoleRecordMut::LossReported(work) => {
                Self::LossReported(PanelSessionView::of(work))
            },
            CurrentRoleRecordMut::Retained(work) => Self::Retained(PanelSessionView::of(work)),
            CurrentRoleRecordMut::NoRecord => Self::NoRecord,
        }
    }
}

impl RoleRecordsView {
    const fn of(records: RoleRecords<'_, PanelAttemptWork, PanelSessionWork>) -> Self {
        let attempt = match records.attempt {
            RoleAttemptRecord::Applying(work) => {
                RoleAttemptRecordView::Applying(PanelAttemptView::of(work))
            },
            RoleAttemptRecord::CompletionQueued(work) => {
                RoleAttemptRecordView::CompletionQueued(PanelAttemptView::of(work))
            },
            RoleAttemptRecord::NoRecord => RoleAttemptRecordView::NoRecord,
        };
        let session = match records.session {
            RoleSessionRecord::Holding(work) => {
                RoleSessionRecordView::Holding(PanelSessionView::of(work))
            },
            RoleSessionRecord::LossReported(work) => {
                RoleSessionRecordView::LossReported(PanelSessionView::of(work))
            },
            RoleSessionRecord::Retained(work) => {
                RoleSessionRecordView::Retained(PanelSessionView::of(work))
            },
            RoleSessionRecord::NoRecord => RoleSessionRecordView::NoRecord,
        };
        Self { attempt, session }
    }

    const fn of_mut(records: RoleRecordsMut<'_, PanelAttemptWork, PanelSessionWork>) -> Self {
        let attempt = match records.attempt {
            RoleAttemptRecordMut::Applying(work) => {
                RoleAttemptRecordView::Applying(PanelAttemptView::of(work))
            },
            RoleAttemptRecordMut::CompletionQueued(work) => {
                RoleAttemptRecordView::CompletionQueued(PanelAttemptView::of(work))
            },
            RoleAttemptRecordMut::NoRecord => RoleAttemptRecordView::NoRecord,
        };
        let session = match records.session {
            RoleSessionRecordMut::Holding(work) => {
                RoleSessionRecordView::Holding(PanelSessionView::of(work))
            },
            RoleSessionRecordMut::LossReported(work) => {
                RoleSessionRecordView::LossReported(PanelSessionView::of(work))
            },
            RoleSessionRecordMut::Retained(work) => {
                RoleSessionRecordView::Retained(PanelSessionView::of(work))
            },
            RoleSessionRecordMut::NoRecord => RoleSessionRecordView::NoRecord,
        };
        Self { attempt, session }
    }
}

/// A readable copy of one [`InFlightAttempt`] or [`InFlightAttemptMut`] — the attempt iterators'
/// item.
///
/// Separate from [`AttemptRecordView`] because the two answers are not the same shape: a by-key
/// read can be asked about an attempt the ledger never held, and an iterator item is always a
/// record the ledger is holding right now. Neither match below has an arm for an absent record
/// because neither item type has one to write.
///
/// One view mirrors both walks, which is what lets a test say the shared and mutable attempt walks
/// name the same attempts carrying the same records — the assertion the two session walks already
/// carry, owed to the attempt pair for the same reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InFlightAttemptView {
    Applying(PanelAttemptView),
    CompletionQueued(PanelAttemptView),
}

impl InFlightAttemptView {
    const fn of(record: &InFlightAttempt<'_, PanelAttemptWork>) -> Self {
        match record {
            InFlightAttempt::Applying(work) => Self::Applying(PanelAttemptView::of(work)),
            InFlightAttempt::CompletionQueued(work) => {
                Self::CompletionQueued(PanelAttemptView::of(work))
            },
        }
    }

    const fn of_mut(record: &InFlightAttemptMut<'_, PanelAttemptWork>) -> Self {
        match record {
            InFlightAttemptMut::Applying(work) => Self::Applying(PanelAttemptView::of(work)),
            InFlightAttemptMut::CompletionQueued(work) => {
                Self::CompletionQueued(PanelAttemptView::of(work))
            },
        }
    }
}

/// A readable copy of one [`HeldSession`] or [`HeldSessionMut`] — the session iterators' item.
///
/// One view for both readers, for the same reason [`SessionRecordView`] reads both by-role
/// readers: the shared and mutable walks have one job between them, and a suite that read only one
/// would let the other yield a different set of roles. Absence has no arm here because a role
/// carrying no record is not an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeldSessionView {
    Holding(PanelSessionView),
    LossReported(PanelSessionView),
    Retained(PanelSessionView),
}

impl HeldSessionView {
    const fn of(record: &HeldSession<'_, PanelSessionWork>) -> Self {
        match record {
            HeldSession::Holding(work) => Self::Holding(PanelSessionView::of(work)),
            HeldSession::LossReported(work) => Self::LossReported(PanelSessionView::of(work)),
            HeldSession::Retained(work) => Self::Retained(PanelSessionView::of(work)),
        }
    }

    /// The same reading taken from the mutable walk, so one assertion covers both.
    const fn of_mut(record: &HeldSessionMut<'_, PanelSessionWork>) -> Self {
        match record {
            HeldSessionMut::Holding(work) => Self::Holding(PanelSessionView::of(work)),
            HeldSessionMut::LossReported(work) => Self::LossReported(PanelSessionView::of(work)),
            HeldSessionMut::Retained(work) => Self::Retained(PanelSessionView::of(work)),
        }
    }
}

/// A readable copy of one [`AttemptLookup`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptLookupView {
    Applying(AttemptRef),
    CompletionQueued(AttemptRef),
    Idle,
}

impl AttemptLookupView {
    const fn of(lookup: &AttemptLookup) -> Self {
        match lookup {
            AttemptLookup::Applying(attempt) => Self::Applying(*attempt),
            AttemptLookup::CompletionQueued(attempt) => Self::CompletionQueued(*attempt),
            AttemptLookup::Idle => Self::Idle,
        }
    }
}

/// A readable copy of one [`SessionLookup`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionLookupView {
    Holding(SessionRef),
    LossReported(SessionRef),
    NotEstablished,
}

impl SessionLookupView {
    const fn of(lookup: &SessionLookup) -> Self {
        match lookup {
            SessionLookup::Holding(session) => Self::Holding(*session),
            SessionLookup::LossReported(session) => Self::LossReported(*session),
            SessionLookup::NotEstablished => Self::NotEstablished,
        }
    }
}

impl LedgerDriver {
    fn new(script: LedgerScript) -> (Self, LedgerControl) {
        let state = Arc::new(Mutex::new(LedgerDriverState {
            ledger: DriverLedger::new(),
            script,
            next_serial: 0,
            begins: Vec::new(),
            started: Vec::new(),
            establishments: Vec::new(),
            cancellations: Vec::new(),
            successes: Vec::new(),
            releases: Vec::new(),
            discards: Vec::new(),
            teardowns: 0,
        }));
        (
            Self {
                state: Arc::clone(&state),
            },
            LedgerControl { state },
        )
    }
}

impl LedgerControl {
    fn with_state<Answer>(&self, read: impl FnOnce(&mut LedgerDriverState) -> Answer) -> Answer {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        read(&mut state)
    }

    fn begins(&self) -> Vec<BegunAttemptView> { self.with_state(|state| state.begins.clone()) }

    fn started(&self) -> Vec<AttemptRef> { self.with_state(|state| state.started.clone()) }

    fn establishments(&self) -> Vec<EstablishmentView> {
        self.with_state(|state| state.establishments.clone())
    }

    fn successes(&self) -> Vec<AttemptSucceeded> {
        self.with_state(|state| state.successes.clone())
    }

    fn cancellations(&self) -> Vec<CancelView> {
        self.with_state(|state| state.cancellations.clone())
    }

    fn releases(&self) -> Vec<ReleaseView> { self.with_state(|state| state.releases.clone()) }

    fn discards(&self) -> Vec<DiscardView> { self.with_state(|state| state.discards.clone()) }

    fn teardowns(&self) -> usize { self.with_state(|state| state.teardowns) }

    fn succeed(&self, attempt: AttemptRef) -> AttemptSucceeded {
        self.with_state(|state| state.ledger.succeed_attempt(attempt, Applied::AsDispatched))
    }

    fn fail(&self, attempt: AttemptRef) -> FinishView {
        self.with_state(|state| {
            FinishView::of(&state.ledger.fail_attempt(attempt, access_error("failed")))
        })
    }

    fn abort(&self, attempt: AttemptRef) -> FinishView {
        self.with_state(|state| {
            FinishView::of(
                &state
                    .ledger
                    .abort_attempt(attempt, DriverAbortReason::OperationEnded),
            )
        })
    }

    fn credit(&self, role: &RoleKey, observation: PanelObservation, at: Instant) -> CreditView {
        self.with_state(|state| {
            CreditView::of(
                state
                    .ledger
                    .credit_flow::<PanelTransport>(role, &observation, at),
            )
        })
    }

    fn change_configuration(&self, role: &RoleKey, slot: u32) -> ConfigurationView {
        self.with_state(|state| {
            ConfigurationView::of(
                state
                    .ledger
                    .change_configuration(role, PanelPlacement { slot }),
            )
        })
    }

    fn report_loss(&self, role: &RoleKey) -> LossView {
        self.with_state(|state| {
            LossView::of(
                state
                    .ledger
                    .report_loss(role, access_error("session ended")),
            )
        })
    }

    fn cancel(&self, role: &RoleKey, attempt: AttemptRef) -> CancelView {
        self.with_state(|state| CancelView::of(&state.ledger.cancel_attempt(role, attempt)))
    }

    fn release(&self, role: &RoleKey, session: SessionRef) -> ReleaseView {
        self.with_state(|state| ReleaseView::of(state.ledger.release_lease(role, session)))
    }

    fn discard_retained(&self, role: &RoleKey) -> DiscardView {
        self.with_state(|state| DiscardView::of(&state.ledger.discard_retained(role)))
    }

    fn attempt_of(&self, role: &RoleKey) -> AttemptLookupView {
        self.with_state(|state| AttemptLookupView::of(&state.ledger.attempt_of(role)))
    }

    fn session_of(&self, role: &RoleKey) -> SessionLookupView {
        self.with_state(|state| SessionLookupView::of(&state.ledger.session_of(role)))
    }

    /// Read the driver's own record for one attempt, through both the shared and mutable readers.
    ///
    /// Both are read here rather than in two verbs because the pair has one job and a suite that
    /// exercised only the shared one would let the mutable reader answer a different slot.
    fn attempt_record(&self, attempt: AttemptRef) -> AttemptRecordView {
        self.with_state(|state| {
            let shared = AttemptRecordView::of(&state.ledger.attempt_record(attempt));
            let mutable = AttemptRecordView::of_mut(&state.ledger.attempt_record_mut(attempt));
            assert_eq!(
                shared, mutable,
                "the shared and mutable attempt readers disagreed about the same slot"
            );

            shared
        })
    }

    /// Read the driver's own record for one role's session, through both readers.
    fn session_record(&self, role: &RoleKey) -> SessionRecordView {
        self.with_state(|state| {
            let shared = SessionRecordView::of(&state.ledger.session_record(role));
            let mutable = SessionRecordView::of_mut(&state.ledger.session_record_mut(role));
            assert_eq!(
                shared, mutable,
                "the shared and mutable session readers disagreed about the same role"
            );

            shared
        })
    }

    /// Read both record slots for one role through the shared and mutable resolvers.
    fn role_records(&self, role: &RoleKey) -> RoleRecordsView {
        self.with_state(|state| {
            let shared = RoleRecordsView::of(state.ledger.record_of(role));
            let mutable = RoleRecordsView::of_mut(state.ledger.record_of_mut(role));
            assert_eq!(
                shared, mutable,
                "the shared and mutable role resolvers disagreed about the same two slots"
            );

            shared
        })
    }

    /// Read the current record for one role through the shared and mutable convenience views.
    fn current_role_record(&self, role: &RoleKey) -> CurrentRoleRecordView {
        self.with_state(|state| {
            let shared = CurrentRoleRecordView::of(state.ledger.record_of(role).current());
            let mutable = CurrentRoleRecordView::of_mut(state.ledger.record_of_mut(role).current());
            assert_eq!(
                shared, mutable,
                "the shared and mutable current-record views disagreed about slot precedence"
            );

            shared
        })
    }

    /// Walk every role once through the shared two-slot view.
    fn all_role_records(&self) -> Vec<(String, RoleRecordsView)> {
        self.with_state(|state| {
            let mut records = state
                .ledger
                .records()
                .map(|(role, records)| (role.to_string(), RoleRecordsView::of(records)))
                .collect::<Vec<_>>();
            records.sort_by(|(left, _), (right, _)| left.cmp(right));
            records
        })
    }

    /// Walk every role once through the mutable two-slot view, without changing either record.
    fn all_role_records_mut(&self) -> Vec<(String, RoleRecordsView)> {
        self.with_state(|state| {
            let mut records = Vec::new();
            state.ledger.for_each_record_mut(|role, role_records| {
                records.push((role.to_string(), RoleRecordsView::of_mut(role_records)));
            });
            records.sort_by(|(left, _), (right, _)| left.cmp(right));
            records
        })
    }

    /// Mark every attempt record the ledger holds whose hardware this window abort ended.
    ///
    /// The window driver's `abort_window` in miniature: it arrives keyed by neither an attempt nor
    /// a role, walks every open attempt, and marks the ones its predicate names. Answers how many
    /// records it reached, so a walk that silently reached nothing cannot pass.
    fn end_hardware_opened_by(&self, opened_by: u32) -> usize {
        self.with_state(|state| {
            let mut ended = 0;
            for (_, _, record) in state.ledger.attempts_mut() {
                let work = match record {
                    InFlightAttemptMut::Applying(work)
                    | InFlightAttemptMut::CompletionQueued(work) => work,
                };
                if work.opened_by == opened_by {
                    work.hardware = PanelHardware::Ended;
                    ended += 1;
                }
            }

            ended
        })
    }

    /// Every open attempt the ledger holds, named by role and read as a view.
    ///
    /// Sorted by attempt reference so a test can assert the whole set without depending on the
    /// hash order the ledger's maps happen to have.
    fn open_attempts(&self) -> Vec<(AttemptRef, String, InFlightAttemptView)> {
        self.with_state(|state| {
            let mut attempts: Vec<_> = state
                .ledger
                .attempts_mut()
                .map(|(attempt, role, record)| {
                    (
                        attempt,
                        role.to_string(),
                        InFlightAttemptView::of_mut(&record),
                    )
                })
                .collect();
            attempts.sort_by_key(|(attempt, ..)| attempt.get());

            attempts
        })
    }

    /// The same walk taken immutably, which is the only one a driver holding a `&World` can take.
    ///
    /// The camera kernel's shape: it proves a device has no session at all before it readdresses
    /// one, and that reading runs where no `&mut` of the ledger can be had. A shared walk that
    /// named a different set of attempts than the mutable one would leave that proof and the
    /// per-frame sweeps disagreeing about what the driver is holding.
    fn open_attempts_shared(&self) -> Vec<(AttemptRef, String, InFlightAttemptView)> {
        self.with_state(|state| {
            let mut attempts: Vec<_> = state
                .ledger
                .attempts()
                .map(|(attempt, role, record)| {
                    (attempt, role.to_string(), InFlightAttemptView::of(&record))
                })
                .collect();
            attempts.sort_by_key(|(attempt, ..)| attempt.get());

            attempts
        })
    }

    /// Every role whose session record the ledger still holds, read as a view.
    fn open_sessions(&self) -> Vec<(String, HeldSessionView)> {
        self.with_state(|state| {
            let mut sessions: Vec<_> = state
                .ledger
                .sessions()
                .map(|(role, record)| (role.to_string(), HeldSessionView::of(&record)))
                .collect();
            sessions.sort_by(|(left, _), (right, _)| left.cmp(right));

            sessions
        })
    }

    /// The same walk taken mutably, read rather than written.
    ///
    /// The mutable walk is what the per-frame sweeps really use, and everywhere else in this file
    /// it is exercised through the counter it increments. Reading it as a view is what lets one
    /// assertion say the two walks name the same roles carrying the same records.
    fn open_sessions_mut(&self) -> Vec<(String, HeldSessionView)> {
        self.with_state(|state| {
            let mut sessions: Vec<_> = state
                .ledger
                .sessions_mut()
                .map(|(role, record)| (role.to_string(), HeldSessionView::of_mut(&record)))
                .collect();
            sessions.sort_by(|(left, _), (right, _)| left.cmp(right));

            sessions
        })
    }

    /// Poll every session the driver still holds a live lease for, crediting one frame to each.
    ///
    /// The per-frame sweeps the shipped kernels run, reduced to a counter: only a `Holding` record
    /// is polled, because a session whose loss was reported and one kept after release are both
    /// records the driver keeps for teardown rather than streams it is still reading.
    fn poll_holding_sessions(&self) -> usize {
        self.with_state(|state| {
            let mut polled = 0;
            for (_, record) in state.ledger.sessions_mut() {
                match record {
                    HeldSessionMut::Holding(work) => {
                        work.frames_polled += 1;
                        polled += 1;
                    },
                    HeldSessionMut::LossReported(_) | HeldSessionMut::Retained(_) => {},
                }
            }

            polled
        })
    }

    /// Find the attempt whose record one `start_apply` opened.
    ///
    /// The lookup that arrives keyed by neither an attempt nor a role — the screen kernel's job
    /// outcome channel — and the only reason `find_attempt` exists.
    fn find_attempt_opened_by(&self, opened_by: u32) -> AttemptSearch {
        self.with_state(|state| {
            state
                .ledger
                .find_attempt(|work| work.opened_by == opened_by)
        })
    }
}

impl EndpointDriver for LedgerDriver {
    type Configuration = PanelPlacement;
    type Target = ();

    fn resolve_target(
        &mut self,
        _: &mut World,
        _: &TargetResolutionContext<'_>,
        _: &Self::Configuration,
    ) -> TargetResolution<Self::Target> {
        TargetResolution::Reached(())
    }

    fn start_apply(
        &mut self,
        world: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        _: &Self::Configuration,
        (): Self::Target,
    ) {
        let observed_at = frame_instant(world);
        let role = context.target().role().clone();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let opened_by = state.next_serial;
        state.next_serial += 1;
        let begun = state.ledger.begin_attempt(
            context,
            PanelAttemptWork {
                opened_by,
                hardware: PanelHardware::Open,
            },
        );
        let attempt = begun.attempt();
        state.begins.push(BegunAttemptView::of(&begun));
        state.started.push(attempt);
        let pre_establishment = state.script.credit_before_establishment.clone();
        for observation in pre_establishment {
            state
                .ledger
                .credit_flow::<PanelTransport>(&role, &observation, observed_at);
        }
        drop(state);
    }

    fn established(&mut self, _: &mut World, context: EstablishedContext<'_, Self::Configuration>) {
        let role = context.role().clone();
        let attempt = context.attempt();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(
            state.script.established_cancel,
            EstablishedCancel::BeforeEstablish
        ) {
            let cancelled = state.ledger.cancel_attempt(&role, attempt);
            state.cancellations.push(CancelView::of(&cancelled));
        }
        // A driver reads its own record before establishing, because `establish_lease` spends the
        // queued completion the judgement is about. This is the window driver's shape: the record's
        // lifetime mark, set by an abort that ran while the attempt was queued, is the judgement.
        let marked = matches!(
            state.ledger.attempt_record(attempt),
            AttemptRecord::CompletionQueued(work) if work.hardware == PanelHardware::Ended
        );
        let hardware = if marked {
            Establishing::Ended(access_error("the abort ended this attempt's hardware"))
        } else {
            state.script.establishing.build()
        };
        let establishment = state
            .ledger
            .establish_lease(context, hardware, into_session_work);
        state
            .establishments
            .push(EstablishmentView::of(&establishment));
        drop(state);
    }

    fn cancel_apply(
        &mut self,
        _: &mut World,
        role: &RoleKey,
        _: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        _: AttemptInvalidation,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.teardowns += 1;
        match state.script.attempt_teardown {
            AttemptTeardownReport::Cancelled => {
                let cancelled = state.ledger.cancel_attempt(role, attempt);
                state.cancellations.push(CancelView::of(&cancelled));
            },
            AttemptTeardownReport::Withheld => {},
            AttemptTeardownReport::SucceededInstead => {
                let succeeded = state.ledger.succeed_attempt(attempt, Applied::AsDispatched);
                state.successes.push(succeeded);
            },
        }
    }

    fn release_session(
        &mut self,
        _: &mut World,
        role: &RoleKey,
        _: DriverCleanupRoleEntity,
        session: SessionRef,
        _: SessionReleaseCause,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.teardowns += 1;
        if matches!(state.script.teardown_release, TeardownRelease::Withheld) {
            return;
        }
        let released = state.ledger.release_lease(role, session);
        state.releases.push(ReleaseView::of(released));
        // A release always retains, so what the driver does next is what determines whether
        // anything of its own outlives the session. `DiscardAtOnce` is the driver that
        // keeps nothing.
        if matches!(released, ReleasedLease::Released) {
            match state.script.after_release {
                RetentionScript::DiscardAtOnce => {
                    let discarded = state.ledger.discard_retained(role);
                    state.discards.push(DiscardView::of(&discarded));
                },
                RetentionScript::KeepRetained => {},
                RetentionScript::SuspendAndKeep => {
                    // The camera's suspend, in the same call as the release it follows: this is
                    // the only window in which the record is the driver's to write and not yet the
                    // successor's to adopt. The write is deliberately conditional on the arm
                    // rather than assumed — a release that stopped retaining would leave the
                    // record untouched, which the caller's first assertion reads as the failure it
                    // is.
                    if let SessionRecordMut::Retained(work) = state.ledger.session_record_mut(role)
                    {
                        work.suspended = SuspendedPicture::FrozenAtFrame(work.frames_polled);
                    }
                },
            }
        }
        drop(state);
    }
}

fn access_error(detail: &str) -> DeviceAccessError {
    DeviceAccessError::Transport {
        detail: detail.to_owned(),
    }
}

fn frame_instant(world: &World) -> Instant {
    let time = world.resource::<Time<Real>>();
    time.last_update().unwrap_or_else(|| time.startup())
}

/// Whether the fixture's binding is evaluated for continuous data arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlowMonitoring {
    /// The role has no data axis; nothing publishes a continuous-flow reading.
    Unmonitored,
    /// The role is evaluated for continuous data arrival, so credits are observable.
    Continuous,
}

/// One scripted panel, one role, and the ledger-backed driver that binds them.
struct LedgerFixture {
    app:      App,
    control:  LedgerControl,
    driver:   EndpointDriverRegistration<PanelPlacement>,
    role:     RoleKey,
    reporter: ReporterId,
    device:   DeviceKey,
    now:      Instant,
}

impl LedgerFixture {
    /// Build the app, register the panel, and stop as soon as an attempt is in flight.
    fn applying(
        name: &str,
        monitoring: FlowMonitoring,
        script: LedgerScript,
    ) -> Result<(Self, AttemptRef), Box<dyn Error>> {
        let device = panel_key(name)?;
        let scans = vec![scan![ScriptedDevice::present(device.clone())]];
        Self::applying_with_scans(
            name,
            device,
            scans,
            monitoring,
            RecoveryPolicy::Forget,
            script,
        )
    }

    /// Build the same fixture over a reporter that can also take the panel away and bring it back.
    ///
    /// The departure scans are opt-in, and they sit behind a run of scans that keep reporting the
    /// panel: the kernel asks a reporter for more runs than a test does, so a departure written
    /// directly behind the opening scan can be spent before the role is even established.
    /// [`LedgerFixture::depart`] drains the prefix by asking for runs until the kernel answers the
    /// departure with a teardown.
    ///
    /// This opener keeps the default [`RecoveryPolicy::Forget`], so the panel returning reopens
    /// nothing and the departure is the whole story.
    fn applying_replug(
        name: &str,
        monitoring: FlowMonitoring,
        script: LedgerScript,
    ) -> Result<(Self, AttemptRef), Box<dyn Error>> {
        Self::applying_replug_with(name, monitoring, RecoveryPolicy::Forget, script)
    }

    fn applying_replug_with(
        name: &str,
        monitoring: FlowMonitoring,
        recovery: RecoveryPolicy,
        script: LedgerScript,
    ) -> Result<(Self, AttemptRef), Box<dyn Error>> {
        let device = panel_key(name)?;
        let mut scans: Vec<ScriptedScan> = (0..REPLUG_PRESENT_PREFIX)
            .map(|_| scan![ScriptedDevice::present(device.clone())])
            .collect();
        scans.push(scan![]);
        scans.push(scan![ScriptedDevice::present(device.clone())]);
        Self::applying_with_scans(name, device, scans, monitoring, recovery, script)
    }

    fn applying_with_scans(
        name: &str,
        device: DeviceKey,
        scans: Vec<ScriptedScan>,
        monitoring: FlowMonitoring,
        recovery: RecoveryPolicy,
        script: LedgerScript,
    ) -> Result<(Self, AttemptRef), Box<dyn Error>> {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(RiggingPlugin)
            .register_device_scheme(SchemeName::new(PANEL_SCHEME)?);
        let now = app.world().resource::<Time<Real>>().startup() + Duration::from_secs(1);
        app.insert_resource(TimeUpdateStrategy::ManualInstant(now));
        let reporter = app.add_device_reporter(
            ScriptedReporter::new(scans),
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                panel_coverage()?,
                Duration::from_secs(10),
            ),
        );
        let (ledger_driver, control) = LedgerDriver::new(script);
        let driver = app.add_endpoint_driver(ledger_driver);
        let role = RoleKey::new(name)?;
        register_binding(
            app.world_mut(),
            BindingAuthoring::new(
                role.clone(),
                DeviceEndpoint {
                    device: device.clone(),
                    id:     EndpointId::Whole,
                },
                driver,
                PanelPlacement {
                    slot: REQUESTED_SLOT,
                },
                binding_policy(monitoring, recovery)?,
            ),
        )?;
        advance_reporter(&mut app, reporter)?;
        let mut fixture = Self {
            app,
            control,
            driver,
            role,
            reporter,
            device,
            now,
        };
        let attempt = fixture.wait_for_new_attempt(0)?;
        Ok((fixture, attempt))
    }

    /// Build the same fixture and carry it through to one established session.
    fn established(
        name: &str,
        monitoring: FlowMonitoring,
        script: LedgerScript,
    ) -> Result<(Self, SessionRef), Box<dyn Error>> {
        Self::established_from(Self::applying(name, monitoring, script)?)
    }

    /// Carry a replug-capable fixture through to one established session.
    ///
    /// This is the opener that asks for [`RecoveryPolicy::ReapplyOnReturn`]: the default forgets a
    /// departed device's configuration, so the panel coming back would reopen nothing and the
    /// second attempt these tests are about would never be handed out.
    fn established_replug(
        name: &str,
        monitoring: FlowMonitoring,
        script: LedgerScript,
    ) -> Result<(Self, SessionRef), Box<dyn Error>> {
        Self::established_from(Self::applying_replug_with(
            name,
            monitoring,
            RecoveryPolicy::ReapplyOnReturn,
            script,
        )?)
    }

    /// Build the app with two panels bound to two roles through one driver, both applying.
    ///
    /// The iterators and `find_attempt` only say anything with more than one role in flight: an
    /// iterator that answered for the role it happened to be asked about, or a search that returned
    /// the only record it held, would pass every single-role assertion in this file. The companion
    /// role is handed back beside the fixture rather than stored in it, because everything the
    /// fixture itself does — the published status, the flow reading, the departure script — belongs
    /// to the primary role.
    ///
    /// The primary role is registered and dispatched first, so its record is always `opened_by: 0`
    /// and the companion's `opened_by: 1`.
    fn applying_pair(
        name: &str,
    ) -> Result<(Self, AttemptRef, RoleKey, AttemptRef), Box<dyn Error>> {
        let device = panel_key(name)?;
        let companion_name = format!("{name}-companion");
        let companion_device = panel_key(&companion_name)?;
        let scans = vec![scan![
            ScriptedDevice::present(device.clone()),
            ScriptedDevice::present(companion_device.clone()),
        ]];
        let (mut fixture, attempt) = Self::applying_with_scans(
            name,
            device,
            scans,
            FlowMonitoring::Unmonitored,
            RecoveryPolicy::Forget,
            LedgerScript::default(),
        )?;
        let companion = RoleKey::new(&companion_name)?;
        register_binding(
            fixture.app.world_mut(),
            BindingAuthoring::new(
                companion.clone(),
                DeviceEndpoint {
                    device: companion_device,
                    id:     EndpointId::Whole,
                },
                fixture.driver,
                PanelPlacement {
                    slot: REQUESTED_SLOT + 1,
                },
                binding_policy(FlowMonitoring::Unmonitored, RecoveryPolicy::Forget)?,
            ),
        )?;
        let companion_attempt = fixture.wait_for_new_attempt(1)?;
        Ok((fixture, attempt, companion, companion_attempt))
    }

    /// Carry both roles of a pair fixture through to their own established sessions.
    fn established_pair(
        name: &str,
    ) -> Result<(Self, SessionRef, RoleKey, SessionRef), Box<dyn Error>> {
        let (mut fixture, attempt, companion, companion_attempt) = Self::applying_pair(name)?;
        assert_eq!(
            fixture.control.succeed(attempt),
            AttemptSucceeded::Succeeded
        );
        assert_eq!(
            fixture.control.succeed(companion_attempt),
            AttemptSucceeded::Succeeded
        );
        let session = fixture.wait_for_session()?;
        let companion_session = fixture.wait_for_session_of(&companion)?;
        Ok((fixture, session, companion, companion_session))
    }

    fn established_from(
        applying: (Self, AttemptRef),
    ) -> Result<(Self, SessionRef), Box<dyn Error>> {
        let (mut fixture, attempt) = applying;
        assert_eq!(
            fixture.control.succeed(attempt),
            AttemptSucceeded::Succeeded
        );
        let session = fixture.wait_for_session()?;
        Ok((fixture, session))
    }

    /// Advance the frame clock one step and run one update.
    fn update(&mut self) {
        self.now += FRAME_STEP;
        self.app
            .insert_resource(TimeUpdateStrategy::ManualInstant(self.now));
        self.app.update();
    }

    /// Run frames until the ledger has begun one more attempt than `already_started`.
    fn wait_for_new_attempt(
        &mut self,
        already_started: usize,
    ) -> Result<AttemptRef, Box<dyn Error>> {
        for _ in 0..FRAME_CEILING {
            if let Some(attempt) = self.control.started().get(already_started).copied() {
                return Ok(attempt);
            }
            self.update();
        }
        Err(format!("role `{}` was never handed an attempt", self.role).into())
    }

    /// Run frames until the ledger holds a session for the fixture's own role.
    fn wait_for_session(&mut self) -> Result<SessionRef, Box<dyn Error>> {
        let role = self.role.clone();
        self.wait_for_session_of(&role)
    }

    /// Run frames until the ledger holds a session for the named role.
    ///
    /// Named rather than assumed, because one ledger serves every role its driver is registered
    /// for and the pair fixtures wait on the companion as often as on the fixture's own role.
    fn wait_for_session_of(&mut self, role: &RoleKey) -> Result<SessionRef, Box<dyn Error>> {
        for _ in 0..FRAME_CEILING {
            if let SessionLookupView::Holding(session) = self.control.session_of(role) {
                return Ok(session);
            }
            self.update();
        }
        Err(format!("role `{role}` never established a session").into())
    }

    /// Run frames until every callback the current change implies has run.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.update();
        }
    }

    /// Read the role's published status.
    fn role_status(&self) -> Result<&RoleStatusView, Box<dyn Error>> {
        let entity = self
            .app
            .world()
            .resource::<Bindings>()
            .role_entity(&self.role)?;
        Ok(self
            .app
            .world()
            .get::<RoleStatus>(entity)
            .ok_or("the registered role has no RoleStatus component")?
            .view())
    }

    /// Read the role's published continuous-flow reading.
    fn flow(&self) -> Result<ContinuousFlowView, Box<dyn Error>> {
        let RoleStatusView::Established { flow, .. } = self.role_status()? else {
            return Err("the role is not established, so it publishes no flow reading".into());
        };
        let EstablishedFlowView::Continuous(continuous) = flow else {
            return Err("the role's binding is not monitored for continuous flow".into());
        };
        Ok(*continuous)
    }

    /// Read the slot the kernel has accepted as this role's last known good placement.
    fn accepted_slot(&self) -> Result<u32, Box<dyn Error>> {
        Ok(self
            .app
            .world()
            .resource::<Bindings>()
            .binding(&self.role)?
            .last_known_good()?
            .downcast_ref::<PanelPlacement>()
            .ok_or("the accepted placement had the wrong concrete type")?
            .slot)
    }

    /// Take the panel away.
    ///
    /// A departure is what invalidates an in-flight attempt and releases an established session,
    /// and it is the only such event a scripted reporter can produce. The panel is returned inside
    /// the kernel's departure grace, so the role reacquires rather than retiring.
    fn depart(&mut self) -> Result<(), Box<dyn Error>> {
        let before = self.control.teardowns();
        for _ in 0..=REPLUG_PRESENT_PREFIX {
            advance_reporter(&mut self.app, self.reporter)?;
            self.settle();
            if self.control.teardowns() > before {
                return Ok(());
            }
        }
        Err(format!("role `{}` never saw the panel depart", self.role).into())
    }

    /// Plug the panel back in, which reopens the role for another apply.
    fn returns(&mut self) -> Result<(), Box<dyn Error>> {
        advance_reporter(&mut self.app, self.reporter)?;
        self.settle();
        Ok(())
    }
}

fn binding_policy(
    monitoring: FlowMonitoring,
    recovery: RecoveryPolicy,
) -> Result<BindingPolicy, Box<dyn Error>> {
    let policy = BindingPolicy::new(
        recovery,
        RetryOn::NewRevision,
        OnAbort::default(),
        OnSessionLoss::default(),
        ApplyDeadline::ProcessDefault,
    );
    Ok(match monitoring {
        FlowMonitoring::Unmonitored => policy,
        FlowMonitoring::Continuous => policy.with_continuous_flow(ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(FLOW_BOUND)?,
            MaximumDatumGap::new(FLOW_BOUND)?,
        )),
    })
}

fn panel_coverage() -> Result<ReporterCoverage, Box<dyn Error>> {
    Ok(ReporterCoverage::EstablishesAbsence(
        AuthoritativeReporterCoverage::one(CoveredDeviceIdentitySpace::ReportedScheme {
            kind:   DeviceKind::ControlSurface,
            scheme: SchemeName::new(PANEL_SCHEME)?,
        }),
    ))
}

fn panel_key(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
    Ok(reported_key(
        DeviceKind::ControlSurface,
        PANEL_SCHEME,
        value,
    )?)
}

/// A role the ledger has never been told about, for the readers and the unknown-role arms.
fn unbound_role() -> Result<RoleKey, Box<dyn Error>> {
    Ok(RoleKey::new("a-role-the-ledger-never-saw")?)
}

/// Beginning an attempt files it under the role, and only finishing it moves the slot.
///
/// `attempt_of` is what a driver reads before deciding whether it may start hardware work, so a
/// slot that stayed `Idle` through an in-flight apply would let a driver open a second device
/// operation the kernel never authorized.
#[test]
fn an_attempt_the_ledger_began_is_applying_until_it_is_finished() -> Result<(), Box<dyn Error>> {
    let (fixture, attempt) = LedgerFixture::applying(
        "attempt-applying",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Applying(attempt)
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::NotEstablished
    );
    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::CompletionQueued(attempt)
    );
    Ok(())
}

/// A queued success becomes a session the ledger holds and the kernel publishes.
///
/// The two readings are asserted together because either alone would pass while the other was
/// wrong: a ledger that filed a lease the kernel never established, or a kernel establishment whose
/// lease the ledger dropped, both leave a role no driver can act on.
#[test]
fn a_succeeded_attempt_establishes_a_session_the_ledger_then_holds() -> Result<(), Box<dyn Error>> {
    let (fixture, session) = LedgerFixture::established(
        "establishes-session",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.establishments(),
        vec![EstablishmentView::Established(session)]
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::Holding(session)
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Idle
    );
    assert!(matches!(
        fixture.role_status()?,
        RoleStatusView::Established { session: published, .. } if *published == session
    ));
    Ok(())
}

/// A failed attempt finishes and returns the role to idle without establishing anything.
#[test]
fn a_failed_attempt_returns_the_role_to_idle() -> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt) = LedgerFixture::applying(
        "attempt-failed",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.fail(attempt),
        FinishView::Finished(PanelAttemptView::open(0))
    );
    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::AttemptUnknown,
        "a record handed back to the driver must not still be readable in the ledger"
    );
    fixture.settle();

    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Idle
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::NotEstablished
    );
    assert!(fixture.control.establishments().is_empty());
    assert!(!matches!(
        fixture.role_status()?,
        RoleStatusView::Established { .. }
    ));
    Ok(())
}

/// An aborted attempt finishes and returns the role to idle without establishing anything.
#[test]
fn an_aborted_attempt_returns_the_role_to_idle() -> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt) = LedgerFixture::applying(
        "attempt-aborted",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.abort(attempt),
        FinishView::Finished(PanelAttemptView::open(0))
    );
    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::AttemptUnknown
    );
    fixture.settle();

    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Idle
    );
    assert!(fixture.control.establishments().is_empty());
    Ok(())
}

/// Hardware failing after the driver already reported success changes nothing.
///
/// This is the common case, not a defect: a device answers after the kernel has already accepted a
/// completion. `AlreadyFinished` is the runtime reading of the one-use rule, and a ledger that let
/// the second call through would finish an authority the kernel had already consumed.
#[test]
fn a_failure_after_a_success_changes_nothing() -> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt) = LedgerFixture::applying(
        "failure-after-success",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(fixture.control.fail(attempt), FinishView::AlreadyFinished);
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::CompletionQueued(attempt)
    );

    let session = fixture.wait_for_session()?;
    assert_eq!(
        fixture.control.establishments(),
        vec![EstablishmentView::Established(session)]
    );
    Ok(())
}

/// Aborting after a success is refused for the same reason a failure is.
#[test]
fn an_abort_after_a_success_changes_nothing() -> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt) = LedgerFixture::applying(
        "abort-after-success",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(fixture.control.abort(attempt), FinishView::AlreadyFinished);
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::CompletionQueued(attempt)
    );

    let session = fixture.wait_for_session()?;
    assert_eq!(
        fixture.control.establishments(),
        vec![EstablishmentView::Established(session)]
    );
    Ok(())
}

/// An establishment whose slot no longer holds a queued completion is refused, not filed.
///
/// The driver cancels the attempt inside `established`, which is the one way an external caller can
/// present the ledger with this race. A ledger that filed the lease anyway would hold a session the
/// kernel has no attempt for, and the role would never be released.
#[test]
fn establishment_is_refused_when_no_completion_is_queued_for_the_attempt()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        established_cancel: EstablishedCancel::BeforeEstablish,
        ..LedgerScript::default()
    };
    let (mut fixture, attempt) =
        LedgerFixture::applying("no-queued-completion", FlowMonitoring::Unmonitored, script)?;

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    fixture.settle();

    assert_eq!(
        fixture.control.establishments(),
        vec![EstablishmentView::RefusedNoQueuedCompletion]
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::NotEstablished
    );
    // The refusal carries no record because the cancellation that caused it already took one.
    // A `NoQueuedCompletion` arm that also handed a record back would hand back a second copy of
    // hardware the driver had already unwound.
    assert_eq!(
        fixture.control.cancellations(),
        vec![CancelView::CompletionQueued(PanelAttemptView::open(0))]
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::NoSession
    );
    Ok(())
}

/// A driver that watched its hardware die between completion and establishment refuses the lease.
///
/// Only the driver holds this fact, so the ledger cannot infer it. The lease reports the driver's
/// own error; the kernel acts on the loss through `OnSessionLoss` alone, so the error's class is
/// not observable from outside and this test asserts the refusal and the role's session reading.
#[test]
fn establishment_is_refused_when_the_driver_reports_its_hardware_ended()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        establishing: EstablishingScript::Ended(access_error("the panel went away")),
        ..LedgerScript::default()
    };
    let (mut fixture, attempt) =
        LedgerFixture::applying("session-ended", FlowMonitoring::Unmonitored, script)?;

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    fixture.settle();

    assert_eq!(
        fixture.control.establishments(),
        vec![EstablishmentView::RefusedSessionEnded(
            PanelAttemptView::open(0)
        )]
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::NotEstablished
    );
    Ok(())
}

/// A replacement lease arriving before the driver released the previous one is still filed.
///
/// The kernel has already replaced that session, so a report from the predecessor would arrive
/// stale. Dropping it silently and answering `EstablishedOverUnreleased` is what tells the driver
/// its own bookkeeping fell behind without losing the session it now has.
#[test]
fn a_replacement_lease_is_filed_over_an_unreleased_predecessor() -> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        teardown_release: TeardownRelease::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, first) = LedgerFixture::established_replug(
        "unreleased-predecessor",
        FlowMonitoring::Unmonitored,
        script,
    )?;

    fixture.depart()?;
    fixture.returns()?;
    let replacement = fixture.wait_for_new_attempt(1)?;
    assert_eq!(
        fixture.control.succeed(replacement),
        AttemptSucceeded::Succeeded
    );
    fixture.settle();

    assert!(
        fixture.control.releases().is_empty(),
        "the script withheld its release, which is what leaves the predecessor in place"
    );
    let SessionLookupView::Holding(second) = fixture.control.session_of(&fixture.role) else {
        return Err("the replacement apply did not leave the ledger holding a session".into());
    };
    assert_ne!(second, first);
    assert_eq!(
        fixture.control.establishments(),
        vec![
            EstablishmentView::Established(first),
            EstablishmentView::EstablishedOverUnreleased {
                session:     second,
                predecessor: PanelSessionView::unpolled(0),
            },
        ]
    );
    Ok(())
}

/// A release naming a session a successor has already replaced cannot disturb the successor.
///
/// This is the behaviour `stale_session_release_cannot_disturb_a_ready_successor` pins in the
/// screen kernel, moved into the ledger: the `SessionRef` guard is what stops a late teardown from
/// tearing down the session that replaced it.
#[test]
fn a_stale_release_cannot_disturb_a_ready_successor() -> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        teardown_release: TeardownRelease::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, first) =
        LedgerFixture::established_replug("stale-release", FlowMonitoring::Unmonitored, script)?;

    fixture.depart()?;
    fixture.returns()?;
    let replacement = fixture.wait_for_new_attempt(1)?;
    assert_eq!(
        fixture.control.succeed(replacement),
        AttemptSucceeded::Succeeded
    );
    fixture.settle();
    let SessionLookupView::Holding(second) = fixture.control.session_of(&fixture.role) else {
        return Err("the replacement apply did not leave the ledger holding a session".into());
    };

    assert_eq!(
        fixture.control.release(&fixture.role, first),
        ReleaseView::OtherSessionEstablished
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::Holding(second)
    );
    Ok(())
}

/// A release naming the held session ends it and forgets everything retained for the role.
///
/// The retained evidence has to go with the session: a datum observed for the session that just
/// ended is not evidence about the next one, and the role record itself disappears once nothing is
/// applying and nothing is established.
#[test]
fn a_matching_release_ends_the_session_and_clears_the_role() -> Result<(), Box<dyn Error>> {
    let (mut fixture, session) = LedgerFixture::established(
        "matching-release",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;
    let now = fixture.now;

    assert_eq!(
        fixture.control.release(&fixture.role, session),
        ReleaseView::Released
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::NotEstablished
    );
    // The release retained the driver's record, and the role record stays while it is retained.
    // Only the discard takes the last thing the ledger held for this role.
    assert_eq!(
        fixture.control.discard_retained(&fixture.role),
        DiscardView::Discarded(PanelSessionView::unpolled(0))
    );
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Datum, now),
        CreditView::UnknownRole
    );
    fixture.settle();
    Ok(())
}

/// A release still matches a session whose loss the driver already reported.
///
/// The kernel releases every session it established, including one that ended by reporting loss, so
/// a ledger that only matched a `Holding` lease would leave the driver's hardware record alive with
/// nothing left to release it.
#[test]
fn a_release_after_a_reported_loss_matches_the_same_session() -> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        teardown_release: TeardownRelease::Withheld,
        ..LedgerScript::default()
    };
    let (fixture, session) =
        LedgerFixture::established("release-after-loss", FlowMonitoring::Unmonitored, script)?;

    assert_eq!(
        fixture.control.report_loss(&fixture.role),
        LossView::Reported
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::LossReported(session)
    );
    assert_eq!(
        fixture.control.release(&fixture.role, session),
        ReleaseView::Released
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::NotEstablished
    );
    Ok(())
}

/// Releasing a role the ledger holds no lease for is answered, not acted on.
///
/// A driver acts on its hardware only on `Released`, so this is the answer that stops a teardown
/// arriving for an attempt that never established from closing a device nothing opened.
#[test]
fn a_release_for_a_role_with_no_lease_is_answered_as_such() -> Result<(), Box<dyn Error>> {
    let (fixture, session) = LedgerFixture::established(
        "release-twice",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.release(&fixture.role, session),
        ReleaseView::Released
    );
    assert_eq!(
        fixture.control.release(&fixture.role, session),
        ReleaseView::NotEstablished
    );
    assert_eq!(
        fixture.control.release(&unbound_role()?, session),
        ReleaseView::NotEstablished
    );
    Ok(())
}

/// A reported loss moves the lease once and cannot be reported again.
///
/// The lease is one-use, so a second report has no authority left to spend. Answering
/// `AlreadyReported` rather than reporting twice is what keeps one hardware failure from publishing
/// two endings for one session.
#[test]
fn a_second_loss_report_is_answered_as_already_reported() -> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        teardown_release: TeardownRelease::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, session) =
        LedgerFixture::established("loss-twice", FlowMonitoring::Unmonitored, script)?;

    assert_eq!(
        fixture.control.report_loss(&fixture.role),
        LossView::Reported
    );
    assert_eq!(
        fixture.control.report_loss(&fixture.role),
        LossView::AlreadyReported
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::LossReported(session)
    );

    fixture.settle();
    assert!(!matches!(
        fixture.role_status()?,
        RoleStatusView::Established { session: published, .. } if *published == session
    ));
    Ok(())
}

/// Reporting loss for a role with no established session is answered, not acted on.
#[test]
fn a_loss_report_for_a_role_with_no_lease_is_answered_as_such() -> Result<(), Box<dyn Error>> {
    let (fixture, _attempt) = LedgerFixture::applying(
        "loss-before-establishment",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.report_loss(&fixture.role),
        LossView::NotEstablished
    );
    assert_eq!(
        fixture.control.report_loss(&unbound_role()?),
        LossView::NotEstablished
    );
    Ok(())
}

/// A configuration the device changed on its own reaches the kernel through the held lease.
///
/// The accepted value is read back from the binding rather than from the ledger, because the point
/// of the verb is that the kernel's own record moves: a ledger that answered `Applied` while
/// reporting nothing would leave a restore sending the stale placement.
#[test]
fn a_configuration_change_reaches_the_binding() -> Result<(), Box<dyn Error>> {
    let (mut fixture, _session) = LedgerFixture::established(
        "configuration-change",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;
    assert_eq!(fixture.accepted_slot()?, REQUESTED_SLOT);

    assert_eq!(
        fixture.control.change_configuration(&fixture.role, 3),
        ConfigurationView::Applied
    );
    fixture.settle();

    assert_eq!(fixture.accepted_slot()?, 3);
    Ok(())
}

/// A configuration change after the driver reported loss is refused.
///
/// The lease is gone, so there is no session to attribute the change to. Answering `SessionEnding`
/// rather than silently dropping it is what tells a driver its own record is behind the kernel's.
#[test]
fn a_configuration_change_after_a_reported_loss_is_refused() -> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        teardown_release: TeardownRelease::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, _session) = LedgerFixture::established(
        "configuration-after-loss",
        FlowMonitoring::Unmonitored,
        script,
    )?;

    assert_eq!(
        fixture.control.report_loss(&fixture.role),
        LossView::Reported
    );
    assert_eq!(
        fixture.control.change_configuration(&fixture.role, 4),
        ConfigurationView::SessionEnding
    );
    fixture.settle();

    assert_eq!(fixture.accepted_slot()?, REQUESTED_SLOT);
    Ok(())
}

/// A configuration change before any session exists is refused.
#[test]
fn a_configuration_change_before_establishment_is_refused() -> Result<(), Box<dyn Error>> {
    let (fixture, _attempt) = LedgerFixture::applying(
        "configuration-before-establishment",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.change_configuration(&fixture.role, 5),
        ConfigurationView::NotEstablished
    );
    assert_eq!(
        fixture.control.change_configuration(&unbound_role()?, 5),
        ConfigurationView::NotEstablished
    );
    Ok(())
}

/// A datum credited against a held lease starts the session flowing.
///
/// The published flow reading is the assertion, not the verb's answer: `CreditedToSession` from a
/// ledger that credited nothing is exactly the reading an operator would be misled by.
#[test]
fn a_datum_credited_to_a_live_session_starts_it_flowing() -> Result<(), Box<dyn Error>> {
    let (mut fixture, _session) = LedgerFixture::established(
        "credited-datum",
        FlowMonitoring::Continuous,
        LedgerScript::default(),
    )?;
    assert!(matches!(
        fixture.flow()?,
        ContinuousFlowView::AwaitingFirstDatum { .. }
    ));

    let now = fixture.now;
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Datum, now),
        CreditView::CreditedToSession
    );
    fixture.settle();

    assert!(matches!(
        fixture.flow()?,
        ContinuousFlowView::Flowing { .. }
    ));
    Ok(())
}

/// Transport activity credited against a held lease holds the session where it is.
///
/// Activity proves the transport is alive and presents nothing, so it can never be the arrival that
/// starts a session flowing. The verb still answers `CreditedToSession`, because the observation
/// was credited — what it bought is a separate question the kernel answers.
#[test]
fn activity_credited_to_a_live_session_cannot_start_it_flowing() -> Result<(), Box<dyn Error>> {
    let (mut fixture, _session) = LedgerFixture::established(
        "credited-activity",
        FlowMonitoring::Continuous,
        LedgerScript::default(),
    )?;

    let now = fixture.now;
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Activity, now),
        CreditView::CreditedToSession
    );
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Quiet, now),
        CreditView::CreditedToSession
    );
    fixture.settle();

    assert!(matches!(
        fixture.flow()?,
        ContinuousFlowView::AwaitingFirstDatum { .. }
    ));
    Ok(())
}

/// A datum observed before the lease existed is retained and credited at establishment.
///
/// This is the whole reason the ledger holds evidence per role rather than per session: a driver
/// whose device supplied the datum that made the session usable would otherwise establish a session
/// already awaiting a first datum it had received before the kernel issued the lease.
#[test]
fn a_datum_observed_before_establishment_is_credited_at_establishment() -> Result<(), Box<dyn Error>>
{
    let script = LedgerScript {
        credit_before_establishment: vec![PanelObservation::Datum],
        ..LedgerScript::default()
    };
    let (mut fixture, _session) =
        LedgerFixture::established("retained-datum", FlowMonitoring::Continuous, script)?;

    fixture.settle();
    assert!(matches!(
        fixture.flow()?,
        ContinuousFlowView::Flowing { .. }
    ));
    Ok(())
}

/// Activity observed before the lease existed leaves the session awaiting its first datum.
///
/// The counterpart of the retained-datum case, and what makes that one mean something: a ledger
/// that credited any pre-establishment observation as an arrival would pass the datum test on a
/// session that had presented nothing.
#[test]
fn activity_observed_before_establishment_leaves_the_session_awaiting_a_datum()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        credit_before_establishment: vec![PanelObservation::Activity],
        ..LedgerScript::default()
    };
    let (mut fixture, _session) =
        LedgerFixture::established("retained-activity", FlowMonitoring::Continuous, script)?;

    fixture.settle();
    assert!(matches!(
        fixture.flow()?,
        ContinuousFlowView::AwaitingFirstDatum { .. }
    ));
    Ok(())
}

/// Activity observed after a datum cannot displace the retained datum.
///
/// A retry inside one session polls the transport again, and a poll that carries nothing must not
/// cost the session the arrival it already had: the session would begin awaiting a first datum it
/// had received, and the operator would read a working device as stalled.
#[test]
fn activity_cannot_displace_a_datum_retained_before_establishment() -> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        credit_before_establishment: vec![
            PanelObservation::Datum,
            PanelObservation::Activity,
            PanelObservation::Quiet,
        ],
        ..LedgerScript::default()
    };
    let (mut fixture, _session) =
        LedgerFixture::established("retention-order", FlowMonitoring::Continuous, script)?;

    fixture.settle();
    assert!(matches!(
        fixture.flow()?,
        ContinuousFlowView::Flowing { .. }
    ));
    Ok(())
}

/// A datum retained under an attempt the ledger superseded never reaches the next session.
///
/// Evidence belongs to the attempt that saw it. The kernel ends one in-flight attempt without
/// calling `cancel_apply` — a role entity despawned outside the kernel while the attempt was
/// applying — and the role's next `start_apply` is what supersedes the abandoned slot. A ledger
/// that superseded the slot and kept the evidence would spend the first attempt's arrival on the
/// successor's lease, and the successor would start its first-datum bound from a frame it never
/// saw: a device that has delivered nothing since the replug reads as flowing, and the flow judge
/// that exists to catch exactly that never fires.
///
/// The script withholds the cancellation rather than despawning the role entity, because that is
/// the ledger state the despawn path leaves and it is the one a scripted reporter can reach. What
/// is asserted is the same either way: the successor's session starts from silence.
#[test]
fn a_datum_retained_under_a_superseded_attempt_never_reaches_the_next_session()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        attempt_teardown: AttemptTeardownReport::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, abandoned) = LedgerFixture::applying_replug_with(
        "superseded-evidence",
        FlowMonitoring::Continuous,
        RecoveryPolicy::ReapplyOnReturn,
        script,
    )?;

    let now = fixture.now;
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Datum, now),
        CreditView::RetainedForEstablishment
    );

    fixture.depart()?;
    fixture.returns()?;
    let successor = fixture.wait_for_new_attempt(1)?;
    assert_ne!(
        successor, abandoned,
        "the panel's return should be dispatched as a new attempt, not the abandoned one"
    );
    assert_eq!(
        fixture.control.succeed(successor),
        AttemptSucceeded::Succeeded
    );
    fixture.wait_for_session()?;
    fixture.settle();

    assert!(matches!(
        fixture.flow()?,
        ContinuousFlowView::AwaitingFirstDatum { .. }
    ));
    Ok(())
}

/// The same retained datum reaches the session of the attempt that saw it.
///
/// The control for the supersession case above, and what makes it mean something: the two runs
/// share a fixture, a script and a credit, and differ only in whether an attempt was superseded
/// between the arrival and the lease. Without this, a ledger that had simply stopped retaining
/// evidence credited from outside a callback would pass the supersession assertion while losing
/// every first datum a driver ever observed early.
#[test]
fn a_datum_retained_under_the_attempt_that_establishes_reaches_its_session()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        attempt_teardown: AttemptTeardownReport::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, attempt) = LedgerFixture::applying_replug_with(
        "unsuperseded-evidence",
        FlowMonitoring::Continuous,
        RecoveryPolicy::ReapplyOnReturn,
        script,
    )?;

    let now = fixture.now;
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Datum, now),
        CreditView::RetainedForEstablishment
    );

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    fixture.wait_for_session()?;
    fixture.settle();

    assert!(matches!(
        fixture.flow()?,
        ContinuousFlowView::Flowing { .. }
    ));
    Ok(())
}

/// Every pre-establishment observation is answered as retained, whatever it carried.
///
/// The answer names where the observation went, not what it was worth: a driver reads it to know
/// that no lease exists yet, and the classification is the kernel's business.
#[test]
fn an_observation_credited_before_establishment_is_answered_as_retained()
-> Result<(), Box<dyn Error>> {
    let (fixture, _attempt) = LedgerFixture::applying(
        "retained-answer",
        FlowMonitoring::Continuous,
        LedgerScript::default(),
    )?;
    let now = fixture.now;

    for observation in [
        PanelObservation::Datum,
        PanelObservation::Activity,
        PanelObservation::Quiet,
    ] {
        assert_eq!(
            fixture.control.credit(&fixture.role, observation, now),
            CreditView::RetainedForEstablishment
        );
    }
    Ok(())
}

/// A credit against a session whose loss was reported is answered without crediting anything.
///
/// The lease has already been spent reporting the loss, so there is nothing left to observe
/// through. Answering `SessionEnding` is what tells a driver still polling its hardware that the
/// kernel has moved on.
#[test]
fn a_credit_after_a_reported_loss_is_answered_as_session_ending() -> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        teardown_release: TeardownRelease::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, _session) =
        LedgerFixture::established("credit-after-loss", FlowMonitoring::Continuous, script)?;

    assert_eq!(
        fixture.control.report_loss(&fixture.role),
        LossView::Reported
    );
    let now = fixture.now;
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Datum, now),
        CreditView::SessionEnding
    );
    fixture.settle();
    Ok(())
}

/// A credit for a role the ledger holds nothing for names that, rather than inventing a record.
#[test]
fn a_credit_for_an_unknown_role_is_answered_as_such() -> Result<(), Box<dyn Error>> {
    let (fixture, _attempt) = LedgerFixture::applying(
        "credit-unknown-role",
        FlowMonitoring::Continuous,
        LedgerScript::default(),
    )?;
    let now = fixture.now;

    assert_eq!(
        fixture
            .control
            .credit(&unbound_role()?, PanelObservation::Datum, now),
        CreditView::UnknownRole
    );
    Ok(())
}

/// A kernel cancellation of an in-flight apply names the slot it found.
///
/// The device departs while the attempt is applying, which is the ordinary way the kernel
/// invalidates one. The driver drops its retained completion because the kernel has already ended
/// the attempt; finishing it afterwards would stamp a result on an attempt that no longer exists.
#[test]
fn a_cancelled_applying_attempt_is_answered_as_applying() -> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt) = LedgerFixture::applying_replug(
        "cancel-applying",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    fixture.depart()?;
    for _ in 0..FRAME_CEILING {
        if !fixture.control.cancellations().is_empty() {
            break;
        }
        fixture.update();
    }

    assert_eq!(
        fixture.control.cancellations(),
        vec![CancelView::Applying(PanelAttemptView::open(0))]
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Idle
    );
    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::AttemptUnknown
    );
    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::AttemptUnknown
    );
    Ok(())
}

/// Cancelling an attempt whose success is already queued names that slot.
#[test]
fn a_cancelled_completion_queued_attempt_is_answered_as_completion_queued()
-> Result<(), Box<dyn Error>> {
    let (fixture, attempt) = LedgerFixture::applying(
        "cancel-completion-queued",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(
        fixture.control.cancel(&fixture.role, attempt),
        CancelView::CompletionQueued(PanelAttemptView::open(0))
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Idle
    );
    Ok(())
}

/// A cancellation naming the wrong role for a live attempt is refused.
///
/// Two roles run their own attempts through one ledger, so the role is what scopes the lookup. A
/// ledger that cancelled by attempt alone would let one role's teardown drop another role's
/// retained completion.
#[test]
fn a_cancel_naming_another_roles_attempt_is_answered_as_wrong_role() -> Result<(), Box<dyn Error>> {
    let (fixture, attempt) = LedgerFixture::applying(
        "cancel-wrong-role",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;
    let other = RoleKey::new("cancel-wrong-role-other")?;

    assert_eq!(
        fixture.control.cancel(&other, attempt),
        CancelView::WrongRole
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Applying(attempt)
    );
    Ok(())
}

/// A cancellation for an attempt the ledger never indexed is refused.
#[test]
fn a_cancel_for_an_attempt_the_ledger_does_not_hold_is_answered_as_unknown()
-> Result<(), Box<dyn Error>> {
    let (fixture, attempt) = LedgerFixture::applying(
        "cancel-unknown",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.cancel(&fixture.role, attempt),
        CancelView::Applying(PanelAttemptView::open(0))
    );
    assert_eq!(
        fixture.control.cancel(&fixture.role, attempt),
        CancelView::Unknown
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Idle
    );
    Ok(())
}

/// The readers answer for a role the ledger has never held anything for.
///
/// Absence is a variant of each lookup rather than an optional, so a driver reading either one gets
/// a state it can act on instead of a `None` it has to interpret.
#[test]
fn the_readers_answer_idle_and_not_established_for_an_unknown_role() -> Result<(), Box<dyn Error>> {
    let (fixture, _attempt) = LedgerFixture::applying(
        "unknown-role-readers",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;
    let unknown = unbound_role()?;

    assert_eq!(
        fixture.control.attempt_of(&unknown),
        AttemptLookupView::Idle
    );
    assert_eq!(
        fixture.control.session_of(&unknown),
        SessionLookupView::NotEstablished
    );
    Ok(())
}

/// The ledger keeps the driver registration and reporter reachable for the whole fixture lifetime.
///
/// A compile-only assertion that the fixture's retained handles are the typed ones the kernel
/// issued: the driver registration is parameterized by the configuration, so a binding authored for
/// another configuration cannot be routed through it.
#[test]
fn the_fixture_routes_the_binding_through_the_typed_driver() -> Result<(), Box<dyn Error>> {
    let (fixture, _attempt) = LedgerFixture::applying(
        "typed-registration",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert!(
        fixture
            .app
            .world()
            .resource::<Bindings>()
            .is_routed_by(&fixture.role, fixture.driver)?
    );
    assert!(
        fixture
            .app
            .world()
            .iter_entities()
            .filter_map(|entity| entity.get::<ReporterHealth>())
            .any(|health| health.belongs_to(fixture.reporter))
    );
    assert_eq!(fixture.device.kind, DeviceKind::ControlSurface);
    Ok(())
}

/// An attempt begun on an idle role answers a fresh slot and is the attempt the ledger indexes.
///
/// This is the answer every ordinary apply gets, and it is what makes the superseding answer worth
/// reading: a driver that had to treat both answers alike could not tell an opening attempt from
/// one that displaced a record it still holds, and the whole point of the answer is that it can.
#[test]
fn an_attempt_begun_on_an_idle_role_supersedes_nothing() -> Result<(), Box<dyn Error>> {
    let (fixture, attempt) = LedgerFixture::applying(
        "fresh-slot",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.begins(),
        vec![BegunAttemptView::Fresh(attempt)]
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Applying(attempt)
    );
    Ok(())
}

/// An attempt begun over an abandoned applying slot names the attempt it displaced.
///
/// The kernel has one path that ends an in-flight attempt without calling `cancel_apply` — a role
/// entity despawned outside the kernel while the attempt was applying — so the role's next
/// `start_apply` is the first moment anything learns the earlier attempt is over. A driver keyed by
/// [`AttemptRef`], as the window driver is, has a record filed under the abandoned reference and no
/// other occasion to drop it: an answer that named only the new attempt would leave that record
/// resident for the process's life, and a later answer about the role would find two.
///
/// The abandoned reference is asserted gone from the index as well as named, because naming it
/// while still indexing it would let a driver prune its record and the ledger still route a
/// finish to work the kernel had already ended.
#[test]
fn an_attempt_begun_over_an_abandoned_applying_slot_names_what_it_superseded()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        attempt_teardown: AttemptTeardownReport::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, abandoned) = LedgerFixture::applying_replug_with(
        "supersede-applying",
        FlowMonitoring::Unmonitored,
        RecoveryPolicy::ReapplyOnReturn,
        script,
    )?;

    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Applying(abandoned)
    );

    fixture.depart()?;
    fixture.returns()?;
    let successor = fixture.wait_for_new_attempt(1)?;

    assert_eq!(
        fixture.control.begins(),
        vec![
            BegunAttemptView::Fresh(abandoned),
            BegunAttemptView::Superseding {
                attempt:    successor,
                superseded: abandoned,
                displaced:  PanelAttemptView::open(0),
            },
        ]
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Applying(successor)
    );
    assert_eq!(
        fixture.control.cancel(&fixture.role, abandoned),
        CancelView::Unknown
    );
    assert_eq!(fixture.control.fail(abandoned), FinishView::AttemptUnknown);
    Ok(())
}

/// An attempt begun over an abandoned queued completion names it, and the completion is gone.
///
/// The same abandonment can arrive one step later. The driver's device work landed in the frame
/// the kernel ended the attempt, so the driver reported the success it really had: the ledger
/// queued a completion for an establishment that can never come, and the role's next attempt
/// supersedes a `CompletionQueued` slot rather than an `Applying` one. The driver's record under
/// the abandoned reference is just as orphaned, and the answer is the only place it learns so.
///
/// The finish that answered `Finished` when the completion was queued is repeated after the
/// supersession and must answer `AttemptUnknown`, not `AlreadyFinished`: those two are what
/// separate a completion the ledger dropped from one it is still holding.
#[test]
fn an_attempt_begun_over_an_abandoned_queued_completion_names_what_it_superseded()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        attempt_teardown: AttemptTeardownReport::SucceededInstead,
        ..LedgerScript::default()
    };
    let (mut fixture, abandoned) = LedgerFixture::applying_replug_with(
        "supersede-queued",
        FlowMonitoring::Unmonitored,
        RecoveryPolicy::ReapplyOnReturn,
        script,
    )?;

    fixture.depart()?;
    fixture.returns()?;
    let successor = fixture.wait_for_new_attempt(1)?;

    assert_eq!(
        fixture.control.successes(),
        vec![AttemptSucceeded::Succeeded]
    );
    assert_eq!(
        fixture.control.begins(),
        vec![
            BegunAttemptView::Fresh(abandoned),
            BegunAttemptView::Superseding {
                attempt:    successor,
                superseded: abandoned,
                displaced:  PanelAttemptView::open(0),
            },
        ]
    );
    assert_eq!(
        fixture.control.attempt_of(&fixture.role),
        AttemptLookupView::Applying(successor)
    );
    assert_eq!(
        fixture.control.succeed(abandoned),
        AttemptSucceeded::AttemptUnknown
    );
    Ok(())
}

/// The driver's record for an attempt lives from `start_apply` until establishment converts it.
///
/// The record has to exist before any lease does, because the screen and camera kernels open their
/// hardware inside `start_apply` and a worker thread fills the record while the attempt is still
/// applying. A ledger that only held records for established sessions would leave every driver
/// keeping its own attempt-keyed table for exactly that window, the table the ledger exists to
/// make unnecessary.
#[test]
fn an_attempt_record_lives_from_start_apply_until_establishment_converts_it()
-> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt) = LedgerFixture::applying(
        "attempt-record-lifetime",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::Applying(PanelAttemptView::open(0))
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::NoSession
    );

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::CompletionQueued(PanelAttemptView::open(0)),
        "a success queues the completion and leaves the record where establishment reads it"
    );

    let session = fixture.wait_for_session()?;

    assert_eq!(
        fixture.control.establishments(),
        vec![EstablishmentView::Established(session)]
    );
    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::AttemptUnknown,
        "establishment spends the attempt record; nothing of it is left on the attempt side"
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Holding(PanelSessionView::unpolled(0)),
        "the session record is the attempt record the driver's own conversion produced"
    );
    Ok(())
}

/// The readers name a role and an attempt the ledger holds no record for.
///
/// Absence is a variant of each read rather than a `None`, for the same reason the lookups are:
/// a driver asking whether it still owns hardware gets a state it can act on.
#[test]
fn the_record_readers_answer_for_an_attempt_and_a_role_the_ledger_never_held()
-> Result<(), Box<dyn Error>> {
    let (fixture, attempt) = LedgerFixture::applying(
        "unknown-record-readers",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::Applying(PanelAttemptView::open(0))
    );
    assert_eq!(
        fixture.control.session_record(&unbound_role()?),
        SessionRecordView::NoSession
    );
    Ok(())
}

/// A cancelled queued completion hands its record back, so the driver can unwind opened hardware.
///
/// The cancelled-applying case has an attempt that never reported anything; this one has a driver
/// that already opened its device and told the kernel so. `CompletionQueued` naming the record is
/// what stops that device staying open for the life of the process.
#[test]
fn a_cancelled_queued_attempt_hands_its_record_back() -> Result<(), Box<dyn Error>> {
    let (fixture, attempt) = LedgerFixture::applying(
        "cancel-queued-record",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(
        fixture.control.cancel(&fixture.role, attempt),
        CancelView::CompletionQueued(PanelAttemptView::open(0))
    );
    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::AttemptUnknown
    );
    Ok(())
}

/// A release retains the driver's session record, and only a discard ends it.
///
/// The whole of a session's record life, in the order a driver lives it: the loss report leaves the
/// record where the release can still find it, the release keeps it rather than dropping it, and
/// the discard is the one verb that takes it. A ledger that dropped the record at release would
/// leave the camera with no frozen picture to hand a successor and the window driver with a hidden
/// window nothing owns.
#[test]
fn a_release_retains_the_session_record_until_the_driver_discards_it() -> Result<(), Box<dyn Error>>
{
    let script = LedgerScript {
        teardown_release: TeardownRelease::Withheld,
        ..LedgerScript::default()
    };
    let (fixture, session) =
        LedgerFixture::established("retained-record", FlowMonitoring::Unmonitored, script)?;

    assert_eq!(
        fixture.control.report_loss(&fixture.role),
        LossView::Reported
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::LossReported(PanelSessionView::unpolled(0)),
        "a reported loss spends the lease and leaves the driver's record in place"
    );

    assert_eq!(
        fixture.control.release(&fixture.role, session),
        ReleaseView::Released
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Retained(PanelSessionView::unpolled(0))
    );
    assert_eq!(
        fixture.control.session_of(&fixture.role),
        SessionLookupView::NotEstablished,
        "a retained record is the driver's, not a session the kernel still recognizes"
    );
    // A retained record is something the role still holds, so the role record outlives its
    // release. An observation credited in that window has somewhere to land — where a release
    // that pruned the role would have answered for a role the ledger had never heard of.
    let now = fixture.now;
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Datum, now),
        CreditView::RetainedForEstablishment
    );

    assert_eq!(
        fixture.control.discard_retained(&fixture.role),
        DiscardView::Discarded(PanelSessionView::unpolled(0))
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::NoSession
    );
    assert_eq!(
        fixture.control.discard_retained(&fixture.role),
        DiscardView::NothingRetained
    );
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Datum, now),
        CreditView::UnknownRole,
        "the discard took the last thing the ledger held, so the role record is gone with it"
    );
    Ok(())
}

/// Discarding a role whose session is live keeps the record the driver is still using.
///
/// `discard_retained` is how a driver that keeps nothing past a release ends its record, and a
/// retirement observer calls it outside the five contract methods with no idea what the role holds.
/// A discard that reached into a live session would take the hardware the driver is still reading.
#[test]
fn a_discard_cannot_take_the_record_of_a_session_the_role_still_holds() -> Result<(), Box<dyn Error>>
{
    let (fixture, _session) = LedgerFixture::established(
        "discard-live-session",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.discard_retained(&fixture.role),
        DiscardView::NothingRetained
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Holding(PanelSessionView::unpolled(0))
    );
    assert_eq!(
        fixture.control.discard_retained(&unbound_role()?),
        DiscardView::NothingRetained
    );
    Ok(())
}

/// A driver that discards on every release keeps nothing of a session the kernel ended.
///
/// The screen kernel's shape, driven through the kernel's own teardown rather than from a test:
/// the release answers `Released`, the discard in the same call hands the record back, and nothing
/// of that session survives into the role's next attempt.
#[test]
fn a_driver_that_discards_on_release_keeps_nothing_of_the_ended_session()
-> Result<(), Box<dyn Error>> {
    let (mut fixture, _session) = LedgerFixture::established_replug(
        "discard-on-release",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    fixture.depart()?;

    assert_eq!(fixture.control.releases(), vec![ReleaseView::Released]);
    assert_eq!(
        fixture.control.discards(),
        vec![DiscardView::Discarded(PanelSessionView::unpolled(0))]
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::NoSession
    );
    Ok(())
}

/// An establishment over a record the driver kept past its release hands that record back.
///
/// This is the arm that exists for the camera and the window driver: both keep something usable
/// after the kernel releases a session — a frozen picture, a hidden window — and both need it back
/// when a successor establishes over it. A ledger that answered `EstablishedOverUnreleased` here
/// would tell the driver its bookkeeping had fallen behind when it had done exactly what it meant
/// to, and one that answered `Established` would strand the kept record for the life of the
/// process.
#[test]
fn an_establishment_over_a_retained_record_hands_the_retained_record_back()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        after_release: RetentionScript::KeepRetained,
        ..LedgerScript::default()
    };
    let (mut fixture, first) = LedgerFixture::established_replug(
        "retained-predecessor",
        FlowMonitoring::Unmonitored,
        script,
    )?;

    fixture.depart()?;
    assert_eq!(fixture.control.releases(), vec![ReleaseView::Released]);
    assert!(
        fixture.control.discards().is_empty(),
        "the script keeps its record, which is what leaves a retained predecessor to establish over"
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Retained(PanelSessionView::unpolled(0))
    );

    fixture.returns()?;
    let replacement = fixture.wait_for_new_attempt(1)?;
    assert_eq!(
        fixture.control.succeed(replacement),
        AttemptSucceeded::Succeeded
    );
    fixture.settle();

    let SessionLookupView::Holding(second) = fixture.control.session_of(&fixture.role) else {
        return Err("the replacement apply did not leave the ledger holding a session".into());
    };
    assert_ne!(second, first);
    assert_eq!(
        fixture.control.establishments(),
        vec![
            EstablishmentView::Established(first),
            EstablishmentView::EstablishedOverRetained {
                session:  second,
                retained: PanelSessionView::unpolled(0),
            },
        ]
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Holding(PanelSessionView::unpolled(1)),
        "the successor's record is its own, converted from the attempt that established it"
    );
    Ok(())
}

/// A record the driver wrote into after its release is the record its successor is handed.
///
/// The camera's frozen picture, stated at the ledger rather than inside the kernel: the release
/// suspends the stream into the record it has just retained, and the successor's establishment
/// adopts that record with the suspend still written on it.
///
/// This is what [`an_establishment_over_a_retained_record_hands_the_retained_record_back`] cannot
/// say on its own. There the retained record's every field is what a fresh one would hold, so a
/// ledger that fabricated a default, or copied the record at release time and handed back the
/// copy, would answer the same arm and pass. The two marks here can only come from the session that
/// was really released — the frames its own sweeps polled, and the frame its own release froze —
/// and the driver writes the second of them through `session_record_mut` in the window between the
/// two, which is the only window a retained record is reachable at all. A ledger that lost either
/// mark would leave the camera presenting a picture the retained stream never produced.
#[test]
fn a_record_suspended_after_its_release_is_the_one_the_successor_adopts()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        after_release: RetentionScript::SuspendAndKeep,
        ..LedgerScript::default()
    };
    let (mut fixture, first) = LedgerFixture::established_replug(
        "suspended-picture",
        FlowMonitoring::Unmonitored,
        script,
    )?;

    // Two per-frame sweeps before the departure, so the record carries a count no fresh session
    // and no successor could have.
    assert_eq!(fixture.control.poll_holding_sessions(), 1);
    assert_eq!(fixture.control.poll_holding_sessions(), 1);

    fixture.depart()?;

    assert_eq!(fixture.control.releases(), vec![ReleaseView::Released]);
    assert!(
        fixture.control.discards().is_empty(),
        "the camera keeps its picture, so nothing was discarded inside the release"
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Retained(PanelSessionView::frozen(0, 2)),
        "the suspend wrote into the record the release had just retained, not beside it"
    );

    fixture.returns()?;
    let replacement = fixture.wait_for_new_attempt(1)?;
    assert_eq!(
        fixture.control.succeed(replacement),
        AttemptSucceeded::Succeeded
    );
    fixture.settle();

    let SessionLookupView::Holding(second) = fixture.control.session_of(&fixture.role) else {
        return Err("the replacement apply did not leave the ledger holding a session".into());
    };
    assert_ne!(second, first);
    assert_eq!(
        fixture.control.establishments(),
        vec![
            EstablishmentView::Established(first),
            EstablishmentView::EstablishedOverRetained {
                session:  second,
                retained: PanelSessionView::frozen(0, 2),
            },
        ],
        "the adopted record carries both marks of the session that was released"
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Holding(PanelSessionView::unpolled(1)),
        "the successor's own record took the slot the adopted one left"
    );
    assert_eq!(
        fixture.control.discard_retained(&fixture.role),
        DiscardView::NothingRetained,
        "the establishment handed the retained record over rather than keeping a copy of it"
    );
    Ok(())
}

/// A record marked in place through the attempt iterator is the one establishment reads.
///
/// The window driver's `abort_window` end to end: it arrives keyed by a window rather than by an
/// attempt or a role, walks every attempt the ledger holds, marks the ones its predicate names, and
/// the establishment that follows reads that mark as its judgement and refuses. A ledger whose
/// records could not be reached in place would have no way to carry a fact learned during the
/// abort into the callback that needs it, and the driver would establish a session over a window
/// that is already gone.
#[test]
fn a_record_marked_through_the_attempt_iterator_is_the_one_establishment_reads()
-> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt) = LedgerFixture::applying(
        "abort-through-the-iterator",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(
        fixture.control.end_hardware_opened_by(0),
        1,
        "the walk must reach the queued record, not only applying ones"
    );
    assert_eq!(
        fixture.control.attempt_record(attempt),
        AttemptRecordView::CompletionQueued(PanelAttemptView::ended(0))
    );

    fixture.settle();

    assert_eq!(
        fixture.control.establishments(),
        vec![EstablishmentView::RefusedSessionEnded(
            PanelAttemptView::ended(0)
        )],
        "the refusal hands back the record carrying the mark the abort left on it"
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::NoSession
    );
    Ok(())
}

/// The attempt iterator reaches every open attempt in the ledger, whichever role it belongs to.
///
/// One ledger serves every role its driver is registered for, and the per-frame loops that poll
/// open streams have no role to ask about — they sweep. An iterator that yielded idle slots, or
/// that stopped at the first role, would give those loops a different set every frame.
#[test]
fn the_attempt_iterator_reaches_every_open_attempt_across_roles() -> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt, companion, companion_attempt) =
        LedgerFixture::applying_pair("attempt-iterator")?;

    assert_eq!(
        fixture.control.open_attempts(),
        vec![
            (
                attempt,
                fixture.role.to_string(),
                InFlightAttemptView::Applying(PanelAttemptView::open(0)),
            ),
            (
                companion_attempt,
                companion.to_string(),
                InFlightAttemptView::Applying(PanelAttemptView::open(1)),
            ),
        ]
    );

    assert_eq!(
        fixture.control.succeed(companion_attempt),
        AttemptSucceeded::Succeeded
    );

    assert_eq!(
        fixture.control.open_attempts(),
        vec![
            (
                attempt,
                fixture.role.to_string(),
                InFlightAttemptView::Applying(PanelAttemptView::open(0)),
            ),
            (
                companion_attempt,
                companion.to_string(),
                InFlightAttemptView::CompletionQueued(PanelAttemptView::open(1)),
            ),
        ]
    );

    fixture.wait_for_session_of(&companion)?;

    assert_eq!(
        fixture.control.open_attempts(),
        vec![(
            attempt,
            fixture.role.to_string(),
            InFlightAttemptView::Applying(PanelAttemptView::open(0)),
        )],
        "an established attempt's slot is idle, and an idle slot is not an item"
    );
    Ok(())
}

/// The session iterator names every role carrying a record and what each one is carrying.
///
/// The three arms are not interchangeable to a sweeping driver: a held session is a stream it is
/// still reading, a loss-reported one is a teardown in progress, and a retained one is hardware it
/// kept past a release. A sweep that could not tell them apart would credit arrivals to a session
/// that had already ended.
#[test]
fn the_session_iterator_names_every_role_carrying_a_record() -> Result<(), Box<dyn Error>> {
    let (fixture, session, companion, _companion_session) =
        LedgerFixture::established_pair("session-iterator")?;

    assert_eq!(
        fixture.control.release(&fixture.role, session),
        ReleaseView::Released
    );
    assert_eq!(fixture.control.report_loss(&companion), LossView::Reported);

    assert_eq!(
        fixture.control.open_sessions(),
        vec![
            (
                fixture.role.to_string(),
                HeldSessionView::Retained(PanelSessionView::unpolled(0)),
            ),
            (
                companion.to_string(),
                HeldSessionView::LossReported(PanelSessionView::unpolled(1)),
            ),
        ]
    );

    assert_eq!(
        fixture.control.discard_retained(&fixture.role),
        DiscardView::Discarded(PanelSessionView::unpolled(0))
    );
    assert_eq!(
        fixture.control.open_sessions(),
        vec![(
            companion.to_string(),
            HeldSessionView::LossReported(PanelSessionView::unpolled(1)),
        )]
    );
    Ok(())
}

/// A per-frame sweep through the mutable session iterator credits only the roles still holding.
///
/// The six per-frame loops the shipped kernels run, in miniature. The counter is asserted through
/// the record itself rather than the sweep's own tally, because a `sessions_mut` that yielded the
/// right arms while handing out records the ledger did not keep would pass a tally test and lose
/// every frame it credited.
#[test]
fn a_sweep_through_the_session_iterator_credits_only_the_roles_still_holding()
-> Result<(), Box<dyn Error>> {
    let (fixture, session, companion, _companion_session) =
        LedgerFixture::established_pair("session-sweep")?;

    assert_eq!(fixture.control.poll_holding_sessions(), 2);
    assert_eq!(
        fixture.control.release(&fixture.role, session),
        ReleaseView::Released
    );
    assert_eq!(
        fixture.control.poll_holding_sessions(),
        1,
        "a retained record is not a stream the driver is still reading"
    );

    assert_eq!(
        fixture.control.open_sessions(),
        vec![
            (
                fixture.role.to_string(),
                HeldSessionView::Retained(PanelSessionView {
                    opened_by:     0,
                    frames_polled: 1,
                    suspended:     SuspendedPicture::StillStreaming,
                })
            ),
            (
                companion.to_string(),
                HeldSessionView::Holding(PanelSessionView {
                    opened_by:     1,
                    frames_polled: 2,
                    suspended:     SuspendedPicture::StillStreaming,
                })
            ),
        ]
    );
    Ok(())
}

/// The search names the role of the record it matched, for a lookup keyed by neither.
///
/// The screen kernel's job outcome channel arrives naming a job, not an attempt or a role, and this
/// is the one lookup that cannot be an `attempt_of` followed by an `attempt_record`. Answering the
/// role as well as the attempt is what lets the caller go on to read or finish the work.
#[test]
fn the_search_names_the_role_of_the_record_it_matched() -> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt, companion, companion_attempt) =
        LedgerFixture::applying_pair("attempt-search")?;

    assert_eq!(
        fixture.control.find_attempt_opened_by(0),
        AttemptSearch::Found {
            attempt,
            role: fixture.role.clone(),
        }
    );
    assert_eq!(
        fixture.control.find_attempt_opened_by(1),
        AttemptSearch::Found {
            attempt: companion_attempt,
            role:    companion,
        }
    );
    assert_eq!(
        fixture.control.find_attempt_opened_by(9),
        AttemptSearch::NoMatch
    );

    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(
        fixture.control.find_attempt_opened_by(0),
        AttemptSearch::Found {
            attempt,
            role: fixture.role.clone(),
        },
        "a queued completion is still an open attempt the search can find"
    );

    fixture.wait_for_session()?;

    assert_eq!(
        fixture.control.find_attempt_opened_by(0),
        AttemptSearch::NoMatch,
        "establishment spent the record, so the search has nothing left to match"
    );
    Ok(())
}

/// A driver that discards on its own release leaves the successor establishing fresh.
///
/// The release-then-discard pair driven by the kernel rather than by a test, read at the one place
/// the difference shows: the establishment that follows. `release_lease` retains rather than
/// prunes, so a driver that keeps nothing has to discard in the same call to be rid of its record;
/// one that released and stopped would hand the successor an
/// [`EstablishmentView::EstablishedOverRetained`] naming a record it meant to be gone, and act on
/// hardware it had already torn down. The successor answering `Established` is what says the
/// release really ended.
///
/// The role record itself is deliberately not asserted gone. A replug inside the kernel's
/// departure grace keeps the role and dispatches its next apply, so the ledger is still holding an
/// attempt slot for it while the discard runs — which is exactly why the retained record has to be
/// discarded rather than left for the role to be pruned. What a discard leaves behind for a role
/// with nothing else in flight is
/// [`a_release_retains_the_session_record_until_the_driver_discards_it`].
#[test]
fn a_driver_that_discards_on_release_establishes_its_successor_fresh() -> Result<(), Box<dyn Error>>
{
    let (mut fixture, first) = LedgerFixture::established_replug(
        "discard-then-reestablish",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;

    fixture.depart()?;

    assert_eq!(fixture.control.releases(), vec![ReleaseView::Released]);
    assert_eq!(
        fixture.control.discards(),
        vec![DiscardView::Discarded(PanelSessionView::unpolled(0))]
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::NoSession
    );
    assert_eq!(
        fixture.control.discard_retained(&fixture.role),
        DiscardView::NothingRetained,
        "a second discard has nothing to take and must change nothing"
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::NoSession
    );

    fixture.returns()?;
    let replacement = fixture.wait_for_new_attempt(1)?;
    assert_eq!(
        fixture.control.succeed(replacement),
        AttemptSucceeded::Succeeded
    );
    fixture.settle();

    let SessionLookupView::Holding(second) = fixture.control.session_of(&fixture.role) else {
        return Err("the replacement apply did not leave the ledger holding a session".into());
    };
    assert_ne!(second, first);
    assert_eq!(
        fixture.control.establishments(),
        vec![
            EstablishmentView::Established(first),
            EstablishmentView::Established(second),
        ],
        "the discard ran before the successor arrived, so its establishment has no retained \
         predecessor to be handed back"
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Holding(PanelSessionView::unpolled(1))
    );
    Ok(())
}

/// A retained record discarded before the successor arrives leaves it establishing fresh too.
///
/// The same pair split across two calls, which is what makes the state in between observable: the
/// record really is retained, a credit really is answered
/// [`CreditView::RetainedForEstablishment`] against it, and an establishment arriving in that
/// window really would be handed it back. Every one of those readings is right for a driver that
/// meant to keep something and wrong for one that did not, and the discard is the only verb that
/// ends them — which is why a driver keeping nothing must call it rather than rely on the release.
#[test]
fn a_retained_record_discarded_before_its_successor_leaves_it_establishing_fresh()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        after_release: RetentionScript::KeepRetained,
        ..LedgerScript::default()
    };
    let (mut fixture, first) = LedgerFixture::established_replug(
        "retained-then-discarded",
        FlowMonitoring::Unmonitored,
        script,
    )?;

    fixture.depart()?;

    assert_eq!(fixture.control.releases(), vec![ReleaseView::Released]);
    assert!(
        fixture.control.discards().is_empty(),
        "this script keeps its record, so nothing was discarded inside the release"
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::Retained(PanelSessionView::unpolled(0))
    );
    let now = fixture.now;
    assert_eq!(
        fixture
            .control
            .credit(&fixture.role, PanelObservation::Datum, now),
        CreditView::RetainedForEstablishment,
        "a released role whose record is still retained is a role the ledger still holds"
    );

    assert_eq!(
        fixture.control.discard_retained(&fixture.role),
        DiscardView::Discarded(PanelSessionView::unpolled(0))
    );
    assert_eq!(
        fixture.control.session_record(&fixture.role),
        SessionRecordView::NoSession
    );

    fixture.returns()?;
    let replacement = fixture.wait_for_new_attempt(1)?;
    assert_eq!(
        fixture.control.succeed(replacement),
        AttemptSucceeded::Succeeded
    );
    fixture.settle();

    let SessionLookupView::Holding(second) = fixture.control.session_of(&fixture.role) else {
        return Err("the replacement apply did not leave the ledger holding a session".into());
    };
    assert_eq!(
        fixture.control.establishments(),
        vec![
            EstablishmentView::Established(first),
            EstablishmentView::Established(second),
        ],
        "the retained record was discarded first, so the successor establishes over nothing"
    );
    Ok(())
}

/// A superseding attempt files its own record, and the displaced one leaves the ledger with it.
///
/// [`BegunAttemptView::Superseding`] names the record it hands back, and the record readers are
/// where that hand-back is proved rather than described: the successor's reference must read the
/// record its own `start_apply` opened, and the displaced reference must read nothing at all. A
/// `begin_attempt` that filed the successor's record under the abandoned reference, or that handed
/// back the successor's record while keeping the abandoned one, would answer the same
/// `Superseding` arm and leave the driver ending the wrong device work — or ending none, with the
/// abandoned record resident for the life of the process.
#[test]
fn a_superseding_attempt_files_its_own_record_over_the_one_it_displaced()
-> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        attempt_teardown: AttemptTeardownReport::Withheld,
        ..LedgerScript::default()
    };
    let (mut fixture, abandoned) = LedgerFixture::applying_replug_with(
        "supersede-records",
        FlowMonitoring::Unmonitored,
        RecoveryPolicy::ReapplyOnReturn,
        script,
    )?;

    assert_eq!(
        fixture.control.attempt_record(abandoned),
        AttemptRecordView::Applying(PanelAttemptView::open(0))
    );

    fixture.depart()?;
    fixture.returns()?;
    let successor = fixture.wait_for_new_attempt(1)?;

    assert_eq!(
        fixture.control.begins(),
        vec![
            BegunAttemptView::Fresh(abandoned),
            BegunAttemptView::Superseding {
                attempt:    successor,
                superseded: abandoned,
                displaced:  PanelAttemptView::open(0),
            },
        ],
        "the record handed back is the earlier attempt's, not the one this call filed"
    );
    assert_eq!(
        fixture.control.attempt_record(successor),
        AttemptRecordView::Applying(PanelAttemptView::open(1)),
        "the slot the supersession left holds the successor's own record"
    );
    assert_eq!(
        fixture.control.attempt_record(abandoned),
        AttemptRecordView::AttemptUnknown,
        "a record handed back to the driver must not still be readable under its old reference"
    );
    assert_eq!(
        fixture.control.open_attempts(),
        vec![(
            successor,
            fixture.role.to_string(),
            InFlightAttemptView::Applying(PanelAttemptView::open(1)),
        )],
        "the walk a driver sweeps every frame must reach the successor alone"
    );
    assert_eq!(
        fixture.control.find_attempt_opened_by(0),
        AttemptSearch::NoMatch
    );
    assert_eq!(
        fixture.control.find_attempt_opened_by(1),
        AttemptSearch::Found {
            attempt: successor,
            role:    fixture.role.clone(),
        }
    );
    Ok(())
}

/// The role-keyed resolver names every state a single occupied slot can hold.
///
/// Every reading is taken through both mutabilities so either half drifting from the other fails
/// at the state where it drifted. The paired-slot behavior has its own regression below so neither
/// concern has to hide inside one long test.
#[test]
fn role_record_resolvers_cover_every_single_slot_arm() -> Result<(), Box<dyn Error>> {
    let (mut states, attempt) = LedgerFixture::applying(
        "role-record-arms",
        FlowMonitoring::Unmonitored,
        LedgerScript::default(),
    )?;
    let unknown = unbound_role()?;
    assert_eq!(
        states.control.role_records(&unknown),
        RoleRecordsView {
            attempt: RoleAttemptRecordView::NoRecord,
            session: RoleSessionRecordView::NoRecord,
        },
        "an unknown role has two empty slots, not an inferred record"
    );
    assert_eq!(
        states.control.current_role_record(&unknown),
        CurrentRoleRecordView::NoRecord
    );
    assert_eq!(
        states.control.role_records(&states.role),
        RoleRecordsView {
            attempt: RoleAttemptRecordView::Applying(PanelAttemptView::open(0)),
            session: RoleSessionRecordView::NoRecord,
        }
    );
    assert_eq!(
        states.control.current_role_record(&states.role),
        CurrentRoleRecordView::Applying(PanelAttemptView::open(0))
    );

    assert_eq!(states.control.succeed(attempt), AttemptSucceeded::Succeeded);
    assert_eq!(
        states.control.role_records(&states.role),
        RoleRecordsView {
            attempt: RoleAttemptRecordView::CompletionQueued(PanelAttemptView::open(0)),
            session: RoleSessionRecordView::NoRecord,
        }
    );
    assert_eq!(
        states.control.current_role_record(&states.role),
        CurrentRoleRecordView::CompletionQueued(PanelAttemptView::open(0))
    );

    let session = states.wait_for_session()?;
    assert_eq!(
        states.control.role_records(&states.role),
        RoleRecordsView {
            attempt: RoleAttemptRecordView::NoRecord,
            session: RoleSessionRecordView::Holding(PanelSessionView::unpolled(0)),
        }
    );
    assert_eq!(
        states.control.current_role_record(&states.role),
        CurrentRoleRecordView::Holding(PanelSessionView::unpolled(0))
    );
    assert_eq!(states.control.report_loss(&states.role), LossView::Reported);
    assert_eq!(
        states.control.role_records(&states.role),
        RoleRecordsView {
            attempt: RoleAttemptRecordView::NoRecord,
            session: RoleSessionRecordView::LossReported(PanelSessionView::unpolled(0)),
        }
    );
    assert_eq!(
        states.control.current_role_record(&states.role),
        CurrentRoleRecordView::LossReported(PanelSessionView::unpolled(0))
    );
    assert_eq!(
        states.control.release(&states.role, session),
        ReleaseView::Released
    );
    assert_eq!(
        states.control.role_records(&states.role),
        RoleRecordsView {
            attempt: RoleAttemptRecordView::NoRecord,
            session: RoleSessionRecordView::Retained(PanelSessionView::unpolled(0)),
        }
    );
    assert_eq!(
        states.control.current_role_record(&states.role),
        CurrentRoleRecordView::Retained(PanelSessionView::unpolled(0))
    );
    Ok(())
}

/// The role-keyed walks visit a role once while exposing both of its occupied slots.
///
/// A by-attempt or attempt-first resolver cannot satisfy this sequence: once the replacement is
/// applying it must still return the predecessor retained in the session slot. Each walk must
/// return that two-slot role as one item, not two items produced by chaining the old attempt-only
/// and session-only walks.
#[test]
fn role_record_walks_visit_a_two_slot_role_once() -> Result<(), Box<dyn Error>> {
    let script = LedgerScript {
        after_release: RetentionScript::KeepRetained,
        ..LedgerScript::default()
    };
    let (mut replacement, _) = LedgerFixture::established_replug(
        "two-slot-role-records",
        FlowMonitoring::Unmonitored,
        script,
    )?;
    replacement.depart()?;
    let replacing = replacement.wait_for_new_attempt(1)?;
    let applying_over_retained = RoleRecordsView {
        attempt: RoleAttemptRecordView::Applying(PanelAttemptView::open(1)),
        session: RoleSessionRecordView::Retained(PanelSessionView::unpolled(0)),
    };
    assert_eq!(
        replacement.control.role_records(&replacement.role),
        applying_over_retained,
        "the replacement attempt and its retained predecessor must both be visible"
    );
    assert_eq!(
        replacement.control.current_role_record(&replacement.role),
        CurrentRoleRecordView::Applying(PanelAttemptView::open(1)),
        "the current-record view chooses the live replacement without erasing its predecessor"
    );
    let one_visit = vec![(replacement.role.to_string(), applying_over_retained)];
    assert_eq!(
        replacement.control.all_role_records(),
        one_visit,
        "the shared walk visits the two-slot role exactly once"
    );
    assert_eq!(
        replacement.control.all_role_records_mut(),
        one_visit,
        "the mutable walk mirrors the shared one without visiting either slot separately"
    );

    assert_eq!(
        replacement.control.succeed(replacing),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(
        replacement.control.role_records(&replacement.role),
        RoleRecordsView {
            attempt: RoleAttemptRecordView::CompletionQueued(PanelAttemptView::open(1)),
            session: RoleSessionRecordView::Retained(PanelSessionView::unpolled(0)),
        },
        "queueing the replacement completion must not hide or displace the retained predecessor"
    );
    assert_eq!(
        replacement.control.current_role_record(&replacement.role),
        CurrentRoleRecordView::CompletionQueued(PanelAttemptView::open(1)),
        "a queued replacement remains current over its retained predecessor"
    );
    Ok(())
}

/// Every arm the three walks can yield is an arm a record it is holding is really in.
///
/// The item types are the assertion the compiler makes here: the matches in
/// [`InFlightAttemptView::of`], [`HeldSessionView::of`] and [`HeldSessionView::of_mut`] name every
/// arm and nothing else, so an item type that could also mean "no record" would not compile
/// against them. That property is what the by-key readers deliberately do not have — they answer
/// about work the ledger may never have held — and mixing the two shapes is what left every walk
/// in every driver writing an arm that could not happen.
///
/// The body then walks all five arms through, in the order one role really reaches them, so the
/// types are proved against records the kernel filed rather than against an empty ledger. Each
/// walk is also compared with its own shared or mutable twin as well as with the records, because
/// a `sessions_mut` that named a different set of roles than `sessions` would leave the per-frame
/// sweeps and the read-only reports disagreeing about what the driver is holding — and the same
/// holds of `attempts` against `attempts_mut`, where the read-only side is the only one a driver
/// holding nothing but a `&World` can take.
#[test]
fn the_iterator_walks_yield_only_arms_a_held_record_is_really_in() -> Result<(), Box<dyn Error>> {
    let (mut fixture, attempt, companion, companion_attempt) =
        LedgerFixture::applying_pair("iterator-arms")?;

    assert_eq!(
        fixture.control.succeed(companion_attempt),
        AttemptSucceeded::Succeeded
    );
    assert_eq!(
        fixture.control.open_attempts(),
        vec![
            (
                attempt,
                fixture.role.to_string(),
                InFlightAttemptView::Applying(PanelAttemptView::open(0)),
            ),
            (
                companion_attempt,
                companion.to_string(),
                InFlightAttemptView::CompletionQueued(PanelAttemptView::open(1)),
            ),
        ],
        "both attempt arms, filed under the two roles that opened them"
    );
    assert_eq!(
        fixture.control.open_attempts_shared(),
        fixture.control.open_attempts(),
        "the shared attempt walk names the same attempts carrying the same records as the mutable \
         one"
    );

    let companion_session = fixture.wait_for_session_of(&companion)?;
    assert_eq!(
        fixture.control.succeed(attempt),
        AttemptSucceeded::Succeeded
    );
    let session = fixture.wait_for_session()?;
    assert!(
        fixture.control.open_attempts().is_empty(),
        "establishment spent both records, and an idle slot is not an item"
    );
    assert!(
        fixture.control.open_attempts_shared().is_empty(),
        "the shared walk skips an idle slot for the same reason the mutable one does"
    );
    let holding = vec![
        (
            fixture.role.to_string(),
            HeldSessionView::Holding(PanelSessionView::unpolled(0)),
        ),
        (
            companion.to_string(),
            HeldSessionView::Holding(PanelSessionView::unpolled(1)),
        ),
    ];
    assert_eq!(fixture.control.open_sessions(), holding);
    assert_eq!(fixture.control.open_sessions_mut(), holding);

    assert_eq!(
        fixture.control.release(&fixture.role, session),
        ReleaseView::Released
    );
    assert_eq!(
        fixture.control.report_loss(&companion),
        LossView::Reported,
        "the companion keeps its lease through the report, so its record stays reachable"
    );
    let expected = vec![
        (
            fixture.role.to_string(),
            HeldSessionView::Retained(PanelSessionView::unpolled(0)),
        ),
        (
            companion.to_string(),
            HeldSessionView::LossReported(PanelSessionView::unpolled(1)),
        ),
    ];
    assert_eq!(fixture.control.open_sessions(), expected);
    assert_eq!(fixture.control.open_sessions_mut(), expected);

    assert_eq!(
        fixture.control.release(&companion, companion_session),
        ReleaseView::Released
    );
    assert_eq!(
        fixture.control.discard_retained(&companion),
        DiscardView::Discarded(PanelSessionView::unpolled(1))
    );
    let retained_only = vec![(
        fixture.role.to_string(),
        HeldSessionView::Retained(PanelSessionView::unpolled(0)),
    )];
    assert_eq!(fixture.control.open_sessions(), retained_only);
    assert_eq!(
        fixture.control.open_sessions_mut(),
        retained_only,
        "a discarded record leaves both walks naming the same one role"
    );
    Ok(())
}
