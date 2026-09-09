//! Driver-owned records kept for exactly as long as the kernel authorities that govern them.
//!
//! A role can hold an in-flight attempt and an older session at the same time. [`RoleRecords`]
//! exposes both slots together, [`DriverLedger::record_of`] resolves one named role without making
//! a driver repeat the ledger's storage rules, and [`DriverLedger::records`] and
//! [`DriverLedger::for_each_record_mut`] visit every role once without hiding either record.

use bevy::platform::collections::HashMap;
use bevy::platform::time::Instant;
use bevy::prelude::Reflect;

use crate::AttemptRef;
use crate::DeviceAccessError;
use crate::RoleKey;
use crate::SessionDatumArrivalEvidence;
use crate::SessionRef;
use crate::contract::Applied;
use crate::contract::ApplyContext;
use crate::contract::AttemptCompletion;
use crate::contract::DriverAbortReason;
use crate::contract::DriverCompletion;
use crate::contract::EstablishedContext;
use crate::contract::SessionLease;
use crate::transport::DeviceTransport;

/// The two record types one driver keeps beside the authorities the ledger holds for it.
///
/// A driver's hardware does not live in `hana_rigging` and never will, but the *bookkeeping* that
/// tracks it was, in every shipped driver, a second set of maps keyed by exactly what the ledger
/// already keys: [`AttemptRef`] and [`RoleKey`]. Two maps for one lifetime is two chances to leak,
/// and the leaks were real — a superseded attempt left restore markers on a window for the life of
/// the process, and a refused establishment retired hardware the driver could no longer find.
///
/// Naming the two records here lets the ledger carry them through the same states it already
/// tracks, so there is one lifetime instead of two and the driver's record is handed back at every
/// point the ledger ends its interest. The associated types are deliberately unbounded past
/// `'static`: a record holds whatever the driver's hardware needs, including a non-`Send` stream
/// handle, and `hana_rigging` never inspects it.
pub trait DriverRecords: 'static {
    /// What the driver keeps for one in-flight attempt, from `start_apply` until the attempt ends.
    type Attempt: 'static;
    /// What the driver keeps for one established session, until it discards it.
    type Session: 'static;
}

/// The records of a driver that keeps none, and the [`DriverLedger`] default.
///
/// The conformance fakes keep no hardware record for an attempt or session, so they use this type
/// rather than naming `()` twice at every call site. Shipped drivers whose work does carry
/// hardware, including the screen and camera kernels, name records that keep those handles inside
/// the ledger for the lifetime it already governs.
pub struct NoRecords;

impl DriverRecords for NoRecords {
    type Attempt = ();
    type Session = ();
}

/// What the ledger did with a success a driver reported for one attempt.
///
/// Deliberately not `#[must_use]`, and total in every state: hardware answering after the kernel
/// already ended the attempt is the ordinary case for a device that finishes on a worker thread,
/// not a defect a driver has to handle. A driver may report and move on.
///
/// Nothing is handed back. A success is the one terminal result that does not end the driver's
/// interest in the attempt — the record stays in the queued slot until
/// [`DriverLedger::establish_lease`] converts it into a session record — so unlike
/// [`AttemptFinish`] there is nothing here the driver has to take responsibility for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptSucceeded {
    /// The retained completion was consumed and the success was queued for the kernel; the
    /// attempt's record waits in the queued slot for establishment.
    Succeeded,
    /// A terminal result was already reported for this attempt, and nothing changed.
    AlreadyFinished,
    /// No in-flight attempt in this ledger carries that reference.
    AttemptUnknown,
}

/// What the ledger did with a non-success terminal result a driver reported for one attempt.
///
/// `#[must_use]`, because [`Self::Finished`] hands back the record for hardware the driver opened
/// and the kernel will never establish. A window record still naming a window that carries restore
/// markers must not be droppable in silence: dropping it is exactly the leak that outlives the
/// process.
///
/// Total in every state for the same reason [`AttemptSucceeded`] is: hardware answering after the
/// kernel moved on is ordinary, not a defect.
#[derive(Debug)]
#[must_use]
pub enum AttemptFinish<Attempt> {
    /// The retained completion was consumed, the result was queued for the kernel, and the
    /// attempt's record is handed back for the driver to unwind.
    Finished {
        /// What the driver was keeping for this attempt. The ledger's interest in it ends here.
        record: Attempt,
    },
    /// A terminal result was already reported for this attempt, and nothing changed.
    ///
    /// Answered for a failure or abort arriving after [`DriverLedger::succeed_attempt`], whose
    /// queued completion — and whose record — the ledger still holds for establishment. An attempt
    /// already returned to [`AttemptLookup::Idle`] by a failure or abort answers
    /// [`Self::AttemptUnknown`] instead: the ledger keeps no record of attempts it has fully
    /// retired.
    AlreadyFinished,
    /// No in-flight attempt in this ledger carries that reference.
    AttemptUnknown,
}

/// What the role's attempt slot held when the kernel issued a new attempt against it.
///
/// `#[must_use]` because [`Self::Superseding`] carries the displaced attempt's record, which is
/// hardware only the driver can end. The ledger prunes its own record of a superseded attempt and
/// hands the driver's back rather than dropping it, because the kernel has one cleanup path — a
/// role entity despawned outside the kernel while its attempt was applying — that ends an attempt
/// without ever calling `cancel_apply`. Before records, the driver's own attempt-keyed map was
/// left holding that entry for the life of the process. A driver that keeps nothing per attempt
/// uses [`NoRecords`] and takes [`Self::attempt`].
#[derive(Debug)]
#[must_use]
pub enum BegunAttempt<Attempt> {
    /// The role's slot was idle; this attempt superseded nothing.
    Fresh(AttemptRef),
    /// This attempt superseded one the kernel ended without calling `cancel_apply`.
    Superseding {
        /// The attempt this call began.
        attempt:    AttemptRef,
        /// The abandoned attempt, whose retained completion and datum evidence were dropped here.
        superseded: AttemptRef,
        /// The abandoned attempt's record, handed back whichever slot state it was in. Applying
        /// and queued are both possible: the kernel abandons an attempt at either point.
        displaced:  Attempt,
    },
}

impl<Attempt> BegunAttempt<Attempt> {
    /// The attempt this call began, whichever slot state it replaced.
    #[must_use]
    pub const fn attempt(&self) -> AttemptRef {
        match *self {
            Self::Fresh(attempt) | Self::Superseding { attempt, .. } => attempt,
        }
    }
}

/// The one establishment fact only the driver holds: whether its hardware is still live.
///
/// The kernel accepted a successful completion and issued a lease, but between the completion and
/// this callback the device can die — a captured window closes, a stream ends. Nothing in
/// `hana_rigging` can see that, so the driver states it here and the ledger reports the loss on the
/// lease it was just handed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Establishing {
    /// The driver's hardware is still usable for this session.
    Live,
    /// The driver's hardware ended before its session could be established.
    Ended(DeviceAccessError),
}

/// What the ledger did with the lease the kernel issued for one accepted completion.
///
/// `#[must_use]` because three of the four answers hand back a record the driver must end: a
/// refusal returns the attempt record whose hardware the kernel will never use, and an
/// establishment over a predecessor — unreleased or retained — returns that predecessor's session
/// record, which is device work still open for a session the kernel has already replaced.
#[derive(Debug)]
#[must_use]
pub enum Establishment<Attempt, Session> {
    /// The lease is filed under the role and the session is usable.
    Established {
        /// The session the kernel issued.
        session: SessionRef,
    },
    /// The lease is filed, and it displaced a predecessor the kernel never released.
    ///
    /// The predecessor's lease is dropped without a report: the kernel has already replaced that
    /// session, so a report against it would arrive stale.
    EstablishedOverUnreleased {
        /// The session the kernel issued.
        session:     SessionRef,
        /// The displaced session's record, for the driver to end.
        predecessor: Session,
    },
    /// The lease is filed, and it established over a record kept past
    /// [`DriverLedger::release_lease`].
    ///
    /// Distinct from [`Self::EstablishedOverUnreleased`] because the driver deliberately kept this
    /// one — the camera's frozen picture, the window driver's hidden window — and what it does
    /// with it differs: a successor commonly adopts the retained hardware rather than ending it.
    EstablishedOverRetained {
        /// The session the kernel issued.
        session:  SessionRef,
        /// The record kept since release, for the successor to adopt or end.
        retained: Session,
    },
    /// No session was filed; the lease reported its loss and is gone.
    Refused(EstablishmentRefusal<Attempt>),
}

/// Why an establishment filed no session.
#[derive(Debug)]
pub enum EstablishmentRefusal<Attempt> {
    /// The role held no queued completion for that attempt when the kernel established it.
    ///
    /// Reported on the lease as [`DeviceAccessError::Transport`], the class the shipped drivers
    /// report today. Nothing downstream reads the class of a loss reported at establishment: the
    /// attempt already ended as a success, and the kernel acts on the loss through its
    /// `OnSessionLoss` policy alone.
    ///
    /// Carries no record because there was no queued completion to spend, and so no record the
    /// ledger could have been holding for this establishment.
    NoQueuedCompletion,
    /// The driver stated its hardware had already ended, and that error was reported on the lease.
    ///
    /// The attempt's record comes back: the hardware behind it has to be retired, and a `Refused`
    /// arm handing back nothing was the leak that made this whole design necessary.
    SessionEnded {
        /// The error the ledger reported on the lease, so the driver can attribute its own
        /// teardown to the same cause.
        error:  DeviceAccessError,
        /// The attempt's record, whose hardware is now the driver's to retire.
        record: Attempt,
    },
}

/// Where one classified transport observation was credited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowCredit {
    /// The observation went to the role's established lease, which stamps its own arrival slot.
    CreditedToSession,
    /// No lease exists yet, so the observation was retained on the role for establishment.
    RetainedForEstablishment,
    /// The role's lease already reported its loss, so nothing can be credited against it.
    SessionEnding,
    /// This ledger holds no record for that role.
    ///
    /// A role has a record from [`DriverLedger::begin_attempt`] until its attempt is idle and both
    /// its lease and its retained session record are gone, so an observation between two attempts
    /// has nothing to attach to.
    UnknownRole,
}

/// What the ledger did with an external configuration change the driver observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigurationChange {
    /// The change was reported on the role's established lease.
    Applied,
    /// The role's lease already reported its loss, so it can report nothing further.
    SessionEnding,
    /// The role holds no lease.
    NotEstablished,
}

/// What the ledger did with a session loss the driver observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LossReport {
    /// The loss was reported on the lease, which the ledger replaced with its session reference.
    Reported,
    /// This session already reported a loss; a second report would say nothing new.
    AlreadyReported,
    /// The role holds no lease.
    NotEstablished,
}

/// What the retained completion was doing when the kernel cancelled its attempt.
///
/// `#[must_use]` because the two answers that name a slot hand back the record for work the driver
/// started and the kernel will never establish.
#[derive(Debug)]
#[must_use]
pub enum CancelledAttempt<Attempt> {
    /// The attempt was in flight; its retained completion was dropped unused.
    Applying {
        /// The attempt's record, whose hardware is now the driver's to unwind.
        record: Attempt,
    },
    /// The attempt had reported success; the queued completion the kernel refused is gone.
    CompletionQueued {
        /// The attempt's record, whose hardware is now the driver's to unwind.
        record: Attempt,
    },
    /// That attempt belongs to a different role than the one named.
    WrongRole,
    /// No in-flight attempt in this ledger carries that reference.
    Unknown,
}

/// Whether the named session was the one this role still held.
///
/// `#[must_use]` because it is the guard on hardware teardown: a driver ends its device work only
/// on [`Self::Released`]. A late release naming a session a successor already replaced answers
/// [`Self::OtherSessionEstablished`], and acting on it would tear down the successor's hardware.
///
/// Nothing is handed back. The released session's record moves to the ledger's retained slot and
/// stays there — a driver that keeps hardware past release, the camera's frozen picture or the
/// window driver's hidden window, needs it exactly there for the successor to find, and a driver
/// that keeps nothing calls [`DriverLedger::discard_retained`] at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum ReleasedLease {
    /// The role held that session; its lease and retained evidence are gone and its record is
    /// retained.
    Released,
    /// The role holds a different session, which this release does not disturb.
    OtherSessionEstablished,
    /// The role holds no lease.
    NotEstablished,
}

/// What the ledger did with the session record a role kept past its release.
///
/// `#[must_use]` because [`Self::Discarded`] is the last time the driver can reach hardware it has
/// been keeping since release: after this call the ledger holds nothing for that role.
#[derive(Debug)]
#[must_use]
pub enum DiscardedSession<Session> {
    /// The role's retained record is handed back and the ledger's interest in it has ended.
    Discarded(Session),
    /// The role kept nothing past a release — it holds a live session, or nothing at all.
    NothingRetained,
}

/// The attempt a role is running, as the ledger sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptLookup {
    /// An attempt is in flight and its completion is retained, unused.
    Applying(AttemptRef),
    /// An attempt reported success and its completion is queued for the kernel.
    CompletionQueued(AttemptRef),
    /// The role is running no attempt.
    Idle,
}

/// The session a role holds, as the ledger sees it.
///
/// A record kept past a release reads as [`Self::NotEstablished`]: the question this answers is
/// which kernel-issued session the role holds, and a retained record is the driver's hardware
/// outliving one, not a session. [`DriverLedger::session_record`] is how a retained record is seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionLookup {
    /// The role holds a usable lease for this session.
    Holding(SessionRef),
    /// The role's lease reported this session's loss and can report nothing further.
    LossReported(SessionRef),
    /// The role holds no lease.
    NotEstablished,
}

/// The driver's own record for one attempt, and which slot state it is in.
///
/// The state comes with the record rather than being a second lookup, because the two answers are
/// acted on differently: an applying attempt's hardware may still be opening, where a queued one's
/// is open and waiting for a kernel decision that may never come.
#[derive(Debug)]
pub enum AttemptRecord<'ledger, Attempt> {
    /// The attempt is in flight.
    Applying(&'ledger Attempt),
    /// The attempt reported success and waits for establishment.
    CompletionQueued(&'ledger Attempt),
    /// No in-flight attempt in this ledger carries that reference.
    AttemptUnknown,
}

/// [`AttemptRecord`] with the record mutable, for a driver amending its own bookkeeping in place.
///
/// A second type rather than a lifetime parameter on the first, because Rust has no way to be
/// generic over the mutability of a reference.
#[derive(Debug)]
pub enum AttemptRecordMut<'ledger, Attempt> {
    /// The attempt is in flight.
    Applying(&'ledger mut Attempt),
    /// The attempt reported success and waits for establishment.
    CompletionQueued(&'ledger mut Attempt),
    /// No in-flight attempt in this ledger carries that reference.
    AttemptUnknown,
}

/// The driver's own record for one role's session, and which state the ledger holds it in.
#[derive(Debug)]
pub enum SessionRecord<'ledger, Session> {
    /// The role holds a usable lease and this record is its session's.
    Holding(&'ledger Session),
    /// The role's lease reported its loss; the record is still the driver's to end.
    LossReported(&'ledger Session),
    /// The record was kept past [`DriverLedger::release_lease`] and no lease stands.
    Retained(&'ledger Session),
    /// The role has no session record at all.
    NoSession,
}

/// [`SessionRecord`] with the record mutable, for a driver amending its own bookkeeping in place.
#[derive(Debug)]
pub enum SessionRecordMut<'ledger, Session> {
    /// The role holds a usable lease and this record is its session's.
    Holding(&'ledger mut Session),
    /// The role's lease reported its loss; the record is still the driver's to end.
    LossReported(&'ledger mut Session),
    /// The record was kept past [`DriverLedger::release_lease`] and no lease stands.
    Retained(&'ledger mut Session),
    /// The role has no session record at all.
    NoSession,
}

/// One in-flight attempt's record as [`DriverLedger::attempts_mut`] hands it over, mutably.
///
/// A separate type from [`AttemptRecordMut`] because a walk has no absence to report: every item
/// *is* a record, and the roles with no attempt are skipped before an item exists. Absence is the
/// by-key lookups' answer — [`DriverLedger::attempt_record`] and
/// [`DriverLedger::attempt_record_mut`] keep [`AttemptRecord::AttemptUnknown`] because a caller
/// naming an attempt can name one the ledger never had. Sharing one type across both forced every
/// walk to write a dead arm, and the window driver's `abort_window` fused that dead arm with a
/// live one, which is exactly the mistake an unreachable variant invites.
#[derive(Debug)]
pub enum InFlightAttemptMut<'ledger, Attempt> {
    /// The attempt is in flight.
    Applying(&'ledger mut Attempt),
    /// The attempt reported success and waits for establishment.
    CompletionQueued(&'ledger mut Attempt),
}

/// One in-flight attempt's record as [`DriverLedger::attempts`] hands it over.
///
/// The immutable half of [`InFlightAttemptMut`], and a separate type for the reason that one
/// gives: every item of a walk *is* a record, so there is no absent arm to write. A driver whose
/// reading is a pure read takes this — the camera kernel proves a device has no session at all
/// before it readdresses one, and that check runs against a `&World` it can never borrow mutably.
#[derive(Debug)]
pub enum InFlightAttempt<'ledger, Attempt> {
    /// The attempt is in flight.
    Applying(&'ledger Attempt),
    /// The attempt reported success and waits for establishment.
    CompletionQueued(&'ledger Attempt),
}

/// One role's session record as [`DriverLedger::sessions`] hands it over.
///
/// A separate type from [`SessionRecord`] for the reason [`InFlightAttemptMut`] gives: an item of
/// a walk is always a record, so there is no `NoSession` to answer. Roles holding no session are
/// skipped before an item exists, and a caller that needs the absence reported asks
/// [`DriverLedger::session_record`] by role.
#[derive(Debug)]
pub enum HeldSession<'ledger, Session> {
    /// The role holds a usable lease and this record is its session's.
    Holding(&'ledger Session),
    /// The role's lease reported its loss; the record is still the driver's to end.
    LossReported(&'ledger Session),
    /// The record was kept past [`DriverLedger::release_lease`] and no lease stands.
    Retained(&'ledger Session),
}

/// [`HeldSession`] with the record mutable, for a driver amending its own bookkeeping in place.
///
/// A second type rather than a lifetime parameter on the first, because Rust has no way to be
/// generic over the mutability of a reference.
#[derive(Debug)]
pub enum HeldSessionMut<'ledger, Session> {
    /// The role holds a usable lease and this record is its session's.
    Holding(&'ledger mut Session),
    /// The role's lease reported its loss; the record is still the driver's to end.
    LossReported(&'ledger mut Session),
    /// The record was kept past [`DriverLedger::release_lease`] and no lease stands.
    Retained(&'ledger mut Session),
}

/// The driver's record in one role's attempt slot.
///
/// Unlike [`AttemptRecord`], this view starts from a role rather than an [`AttemptRef`], so an
/// empty slot is not an unknown attempt. It is the ordinary [`Self::NoRecord`] half of a
/// [`RoleRecords`] view whose session half may still be occupied.
#[derive(Debug)]
pub enum RoleAttemptRecord<'ledger, Attempt> {
    /// The role's attempt is in flight.
    Applying(&'ledger Attempt),
    /// The role's attempt reported success and waits for establishment.
    CompletionQueued(&'ledger Attempt),
    /// The role has no attempt record.
    NoRecord,
}

/// [`RoleAttemptRecord`] with an occupied record mutable.
#[derive(Debug)]
pub enum RoleAttemptRecordMut<'ledger, Attempt> {
    /// The role's attempt is in flight.
    Applying(&'ledger mut Attempt),
    /// The role's attempt reported success and waits for establishment.
    CompletionQueued(&'ledger mut Attempt),
    /// The role has no attempt record.
    NoRecord,
}

/// The driver's record in one role's session slot.
///
/// This role-keyed view is paired with [`RoleAttemptRecord`] so a replacement cannot hide the
/// predecessor session it is replacing.
#[derive(Debug)]
pub enum RoleSessionRecord<'ledger, Session> {
    /// The role holds a usable lease and this record is its session's.
    Holding(&'ledger Session),
    /// The role's lease reported its loss; the record still belongs to that ending session.
    LossReported(&'ledger Session),
    /// The record was kept after the role released its lease.
    Retained(&'ledger Session),
    /// The role has no session record.
    NoRecord,
}

/// [`RoleSessionRecord`] with an occupied record mutable.
#[derive(Debug)]
pub enum RoleSessionRecordMut<'ledger, Session> {
    /// The role holds a usable lease and this record is its session's.
    Holding(&'ledger mut Session),
    /// The role's lease reported its loss; the record still belongs to that ending session.
    LossReported(&'ledger mut Session),
    /// The record was kept after the role released its lease.
    Retained(&'ledger mut Session),
    /// The role has no session record.
    NoRecord,
}

/// The driver record that represents a role's current work.
///
/// A replacement attempt is current while its predecessor still occupies the session slot, so
/// attempt arms take precedence over session arms. [`RoleRecords`] remains available when a
/// caller must act on both instead of resolving to one.
#[derive(Debug)]
pub enum CurrentRoleRecord<'ledger, Attempt, Session> {
    /// The role's current work is an applying attempt.
    Applying(&'ledger Attempt),
    /// The role's current work reported success and waits for establishment.
    CompletionQueued(&'ledger Attempt),
    /// The role's current work holds a usable session lease.
    Holding(&'ledger Session),
    /// The role's current session reported its loss and awaits release.
    LossReported(&'ledger Session),
    /// The role's most recent session record was kept after release.
    Retained(&'ledger Session),
    /// The role has no driver record in either slot.
    NoRecord,
}

/// [`CurrentRoleRecord`] with an occupied record mutable.
#[derive(Debug)]
pub enum CurrentRoleRecordMut<'ledger, Attempt, Session> {
    /// The role's current work is an applying attempt.
    Applying(&'ledger mut Attempt),
    /// The role's current work reported success and waits for establishment.
    CompletionQueued(&'ledger mut Attempt),
    /// The role's current work holds a usable session lease.
    Holding(&'ledger mut Session),
    /// The role's current session reported its loss and awaits release.
    LossReported(&'ledger mut Session),
    /// The role's most recent session record was kept after release.
    Retained(&'ledger mut Session),
    /// The role has no driver record in either slot.
    NoRecord,
}

/// Both driver-record slots one role can occupy at the same time.
///
/// A replacement attempt does not displace the predecessor session merely by starting. Keeping
/// the slots in one view makes that two-record state visible to adoption, retirement, and device
/// re-keying code instead of letting an attempt-first lookup silently hide the predecessor.
#[derive(Debug)]
pub struct RoleRecords<'ledger, Attempt, Session> {
    /// The role's in-flight attempt record, if it has one.
    pub attempt: RoleAttemptRecord<'ledger, Attempt>,
    /// The role's established or retained session record, if it has one.
    pub session: RoleSessionRecord<'ledger, Session>,
}

impl<'ledger, Attempt, Session> RoleRecords<'ledger, Attempt, Session> {
    /// Resolve the record for the role's current work, preferring a replacement attempt over its
    /// predecessor session.
    #[must_use]
    pub const fn current(self) -> CurrentRoleRecord<'ledger, Attempt, Session> {
        match self.attempt {
            RoleAttemptRecord::Applying(record) => CurrentRoleRecord::Applying(record),
            RoleAttemptRecord::CompletionQueued(record) => {
                CurrentRoleRecord::CompletionQueued(record)
            },
            RoleAttemptRecord::NoRecord => match self.session {
                RoleSessionRecord::Holding(record) => CurrentRoleRecord::Holding(record),
                RoleSessionRecord::LossReported(record) => CurrentRoleRecord::LossReported(record),
                RoleSessionRecord::Retained(record) => CurrentRoleRecord::Retained(record),
                RoleSessionRecord::NoRecord => CurrentRoleRecord::NoRecord,
            },
        }
    }
}

/// [`RoleRecords`] with both occupied records mutable at once.
///
/// The fields are disjoint borrows from one role record. A driver can therefore amend a
/// replacement and its predecessor in one visit without running two walks or rebuilding the
/// ledger's slot precedence in its own state.
#[derive(Debug)]
pub struct RoleRecordsMut<'ledger, Attempt, Session> {
    /// The role's in-flight attempt record, if it has one.
    pub attempt: RoleAttemptRecordMut<'ledger, Attempt>,
    /// The role's established or retained session record, if it has one.
    pub session: RoleSessionRecordMut<'ledger, Session>,
}

impl<'ledger, Attempt, Session> RoleRecordsMut<'ledger, Attempt, Session> {
    /// Resolve the mutable record for the role's current work with the same precedence as
    /// [`RoleRecords::current`].
    #[must_use]
    pub const fn current(self) -> CurrentRoleRecordMut<'ledger, Attempt, Session> {
        match self.attempt {
            RoleAttemptRecordMut::Applying(record) => CurrentRoleRecordMut::Applying(record),
            RoleAttemptRecordMut::CompletionQueued(record) => {
                CurrentRoleRecordMut::CompletionQueued(record)
            },
            RoleAttemptRecordMut::NoRecord => match self.session {
                RoleSessionRecordMut::Holding(record) => CurrentRoleRecordMut::Holding(record),
                RoleSessionRecordMut::LossReported(record) => {
                    CurrentRoleRecordMut::LossReported(record)
                },
                RoleSessionRecordMut::Retained(record) => CurrentRoleRecordMut::Retained(record),
                RoleSessionRecordMut::NoRecord => CurrentRoleRecordMut::NoRecord,
            },
        }
    }
}

/// Which attempt, if any, carries a record matching a driver's predicate.
///
/// Answers [`DriverLedger::find_attempt`], the one lookup for work that arrives keyed by neither
/// the attempt nor the role — a worker thread's job identifier, say. A lookup that *does* have the
/// role is [`DriverLedger::attempt_of`] followed by [`DriverLedger::attempt_record`], never a scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptSearch {
    /// One attempt's record satisfied the predicate.
    Found {
        /// The matching attempt.
        attempt: AttemptRef,
        /// The role that attempt belongs to.
        role:    RoleKey,
    },
    /// No in-flight attempt's record satisfied the predicate.
    NoMatch,
}

/// The one-use authority a role's attempt is holding, and the driver's record for that attempt.
///
/// The kernel runs at most one attempt per role, so the slot lives on the role record and every
/// verb costs one hash. `Applying` retains the completion the driver was handed; `CompletionQueued`
/// records that it was spent on a success the kernel has not established yet, which is the state
/// [`DriverLedger::establish_lease`] requires and [`AttemptFinish::AlreadyFinished`] reports.
///
/// Both states carry the driver's record, not only the leased one: the screen and camera kernels
/// open their hardware inside `start_apply`, before any lease exists, and a worker thread fills
/// that record while the attempt is still applying.
enum AttemptSlot<Configuration, Attempt> {
    Idle,
    Applying {
        attempt:    AttemptRef,
        completion: AttemptCompletion<Configuration>,
        record:     Attempt,
    },
    CompletionQueued {
        attempt: AttemptRef,
        record:  Attempt,
    },
}

/// The session authority a role is holding, and the driver's record for that session.
///
/// `LossReported` keeps the [`SessionRef`] after the lease is consumed by
/// [`SessionLease::report_loss`], because a release naming that session still has to be matched:
/// the kernel releases a session it was told had ended.
///
/// `Retained` is the state with no kernel authority in it at all — the lease is gone and the
/// release is answered — and exists because a driver's hardware routinely outlives the session
/// that opened it. The camera keeps a frozen picture for the next session; the window driver keeps
/// a hidden window through a loss. Holding those in the ledger rather than in a second driver map
/// is what makes [`Establishment::EstablishedOverRetained`] able to hand the record to the
/// successor.
enum LeaseState<Configuration, Session> {
    NotEstablished,
    Holding {
        lease:  SessionLease<Configuration>,
        record: Session,
    },
    LossReported {
        session: SessionRef,
        record:  Session,
    },
    Retained(Session),
}

/// Everything `hana_rigging` issued for one role, plus the driver's own records for it.
///
/// `evidence` is the pre-establishment slot: observations classified before a lease existed fold
/// into it through [`SessionDatumArrivalEvidence::retain`], so the first arrival an attempt saw
/// survives every later observation on that same attempt rather than being displaced by it.
/// Establishment spends it through [`EstablishedContext::into_lease`].
///
/// It belongs to the attempt that saw it, and never outlives that attempt: a failure or abort
/// returns the slot to idle and [`DriverLedger::prune_finished_role`] takes the whole record,
/// and an attempt superseded by [`DriverLedger::begin_attempt`] has its evidence dropped there.
/// Either way the next attempt starts from silence unless it observes an arrival of its own.
struct RoleRecord<Configuration, Attempt, Session> {
    evidence: SessionDatumArrivalEvidence,
    attempt:  AttemptSlot<Configuration, Attempt>,
    lease:    LeaseState<Configuration, Session>,
}

impl<Configuration, Attempt, Session> RoleRecord<Configuration, Attempt, Session> {
    const fn new() -> Self {
        Self {
            evidence: SessionDatumArrivalEvidence::NoDatumObserved,
            attempt:  AttemptSlot::Idle,
            lease:    LeaseState::NotEstablished,
        }
    }

    /// A record holding neither kernel-issued authority nor a driver record is removable; evidence
    /// retained for an attempt that never became a session goes with it.
    ///
    /// A retained session record counts as held: dropping the role here would drop the driver's
    /// hardware silently, which is the whole thing [`LeaseState::Retained`] exists to prevent.
    const fn holds_nothing(&self) -> bool {
        matches!(self.attempt, AttemptSlot::Idle)
            && matches!(self.lease, LeaseState::NotEstablished)
    }
}

/// The role's queued completion, spent or absent.
///
/// A private answer rather than an `Option`, because the two states are read at exactly one call
/// site and the absent one is a named establishment refusal there.
enum QueuedCompletion<Attempt> {
    Spent(Attempt),
    NotQueued,
}

/// Every kernel-issued authority one endpoint driver is holding, the driver's own records for the
/// same work, and the races that come with them.
///
/// A driver embeds this in its own state and stores nothing `hana_rigging` issued: the retained
/// [`AttemptCompletion`] for each in-flight attempt, the [`SessionLease`] for each established
/// role, and the pre-establishment datum evidence each role has collected. What is left in the
/// driver is hardware — so the five [`EndpointDriver`](crate::EndpointDriver) methods become a
/// hardware call plus one ledger verb.
///
/// The `Records` parameter closes the last gap. A driver that keyed its own hardware by
/// [`AttemptRef`] or [`RoleKey`] was keeping a second table over the same lifetime the ledger
/// already tracks, and the two drifted apart at exactly the points the ledger has names for: a
/// superseded attempt, a refused establishment, a release the driver answered late. Handing those
/// records to the ledger makes the drift impossible to write — every verb that ends the ledger's
/// interest in a record hands it back, and the answers that do are `#[must_use]`. A driver with
/// nothing to carry takes the [`NoRecords`] default and pays nothing.
///
/// The three shipped drivers each grew their own version of this table, and with it their own
/// version of the same four races: a completion queued for an attempt the kernel never
/// established, a device that died between completion and establishment, a replacement lease
/// arriving before its predecessor was released, and a late release naming a session a successor
/// already replaced. Each verb below answers with a named enum rather than a bare option, so the
/// race a driver is in is stated at the call site instead of inferred from an absence.
///
/// The ledger carries no `Resource` or `Reflect` derive and imposes no `Send` or `Sync` bound of
/// its own — the screen driver keeps its state `NonSend` and the ledger does not object, and the
/// record types are unbounded past `'static` so a record may hold a non-`Send` stream handle.
/// Where the state lives is the driver's decision.
///
/// Both maps are keyed by process-local, operator-authored values, so
/// [`bevy::platform::collections::HashMap`]'s `FixedHasher` gives up nothing.
/// The ledger holds the authorities; a lookup borrows what it holds rather than
/// handing one out, and every outcome that names hardware the driver must still
/// end is `#[must_use]`:
///
/// ```
/// #![deny(unused_must_use)]
/// use bevy::prelude::Component;
/// use bevy::prelude::Reflect;
/// use hana_rigging::AttemptLookup;
/// use hana_rigging::DriverLedger;
/// use hana_rigging::RoleKey;
/// use hana_rigging::SessionLookup;
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn look_up(ledger: &DriverLedger<Configuration>, role: &RoleKey) {
///     let _: SessionLookup = ledger.session_of(role);
///     let _: AttemptLookup = ledger.attempt_of(role);
/// }
///
/// fn discard(ledger: &mut DriverLedger<Configuration>, role: &RoleKey) {
///     let discarded = ledger.discard_retained(role);
///     drop(discarded);
/// }
/// ```
///
/// The lookups above are what keep the cases below meaningful — a rename would
/// break them loudly rather than leaving these failing for an unrelated reason.
///
/// ```compile_fail,E0308
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::{AttemptCompletion, DriverLedger, RoleKey, SessionLease};
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn take_owned_authorities(ledger: &DriverLedger<Configuration>, role: &RoleKey) {
///     let _: SessionLease<Configuration> = ledger.session_of(role);
///     let _: AttemptCompletion<Configuration> = ledger.attempt_of(role);
/// }
/// ```
///
/// ```compile_fail
/// #![deny(unused_must_use)]
/// use bevy::prelude::{Component, Reflect};
/// use hana_rigging::{
///     AttemptRef, DriverAbortReason, DriverLedger, EstablishedContext, Establishing, RoleKey,
/// };
///
/// #[derive(Component, Reflect)]
/// struct Configuration;
///
/// fn establish_and_forget(
///     ledger: &mut DriverLedger<Configuration>,
///     context: EstablishedContext<'_, Configuration>,
/// ) {
///     ledger.establish_lease(context, Establishing::Live, |()| ());
/// }
///
/// fn abort_and_forget(ledger: &mut DriverLedger<Configuration>, attempt: AttemptRef) {
///     ledger.abort_attempt(attempt, DriverAbortReason::OperationEnded);
/// }
///
/// fn discard_and_forget(ledger: &mut DriverLedger<Configuration>, role: &RoleKey) {
///     ledger.discard_retained(role);
/// }
/// ```
pub struct DriverLedger<Configuration, Records: DriverRecords = NoRecords> {
    roles:    HashMap<RoleKey, RoleRecord<Configuration, Records::Attempt, Records::Session>>,
    /// Maps an in-flight attempt to the role whose slot holds it, so the three finish verbs —
    /// which receive only an [`AttemptRef`] — resolve their role in one hash. An entry lives
    /// exactly as long as the role's slot names that attempt.
    attempts: HashMap<AttemptRef, RoleKey>,
}

impl<Configuration, Records> DriverLedger<Configuration, Records>
where
    Records: DriverRecords,
{
    /// Create a ledger holding no attempt, session, record, or evidence.
    #[must_use]
    pub fn new() -> Self {
        Self {
            roles:    HashMap::default(),
            attempts: HashMap::default(),
        }
    }
}

impl<Configuration, Records> Default for DriverLedger<Configuration, Records>
where
    Records: DriverRecords,
{
    fn default() -> Self { Self::new() }
}

impl<Configuration, Records> DriverLedger<Configuration, Records>
where
    Configuration: Reflect,
    Records: DriverRecords,
{
    /// Retain the attempt's one-use completion and the driver's record for it, and report what
    /// they displaced.
    ///
    /// Call it from [`EndpointDriver::start_apply`](crate::EndpointDriver::start_apply) with
    /// whatever the driver would otherwise have filed under [`BegunAttempt::attempt`] in a map of
    /// its own.
    ///
    /// A slot that is not idle is superseded: its retained completion is dropped unused, its index
    /// entry removed, the role's retained datum evidence cleared, and its record handed back on
    /// [`BegunAttempt::Superseding`]. The record is handed back rather than dropped because the
    /// kernel runs one attempt per role but has a cleanup path — a role entity despawned outside
    /// the kernel while its attempt was applying — that ends the attempt with a warning and never
    /// calls `cancel_apply`. Whatever that attempt left on the hardware is only reachable through
    /// its record, and only the driver holds the handles to unwind it. The evidence goes with the
    /// completion for the same reason it goes with a pruned record: a datum observed under the
    /// abandoned attempt was seen by a session that never existed, and leaving it retained would
    /// let [`Self::establish_lease`] start the successor's flow bounds from an arrival that
    /// attempt never saw.
    ///
    /// # Panics
    ///
    /// Panics when the kernel reissues an [`AttemptRef`] this ledger already indexes. References
    /// are never reissued, so that state means the ledger's view of in-flight work has diverged
    /// from the kernel's and every later answer it gives would be about the wrong attempt.
    pub fn begin_attempt(
        &mut self,
        context: ApplyContext<'_, Configuration>,
        record: Records::Attempt,
    ) -> BegunAttempt<Records::Attempt> {
        let role = context.target().role().clone();
        let attempt = context.attempt();
        assert!(
            !self.attempts.contains_key(&attempt),
            "driver ledger already holds attempt {} reissued for role {role}",
            attempt.get()
        );
        let role_record = self
            .roles
            .entry(role.clone())
            .or_insert_with(RoleRecord::new);
        let displaced = std::mem::replace(
            &mut role_record.attempt,
            AttemptSlot::Applying {
                attempt,
                completion: context.into_completion(),
                record,
            },
        );
        let begun = match displaced {
            AttemptSlot::Applying {
                attempt: superseded,
                record: displaced,
                ..
            }
            | AttemptSlot::CompletionQueued {
                attempt: superseded,
                record: displaced,
            } => {
                role_record.evidence = SessionDatumArrivalEvidence::NoDatumObserved;
                self.attempts.remove(&superseded);
                BegunAttempt::Superseding {
                    attempt,
                    superseded,
                    displaced,
                }
            },
            AttemptSlot::Idle => BegunAttempt::Fresh(attempt),
        };
        self.attempts.insert(attempt, role);

        begun
    }

    /// Report that the attempt established a usable configuration.
    ///
    /// The attempt's record stays in the queued slot: the kernel has not established the session
    /// yet, and [`Self::establish_lease`] is what converts the record rather than the driver
    /// re-filing it.
    ///
    /// The ledger keeps nothing from `applied`. A driver that needs the readback — the window
    /// driver's settled geometry — copies it into its own record before calling, which is why
    /// neither `Configuration` nor [`Applied`] carries a `Clone` bound.
    pub fn succeed_attempt(
        &mut self,
        attempt: AttemptRef,
        applied: Applied<Configuration>,
    ) -> AttemptSucceeded {
        let Some(role) = self.attempts.get(&attempt).cloned() else {
            return AttemptSucceeded::AttemptUnknown;
        };
        let Some(role_record) = self.roles.get_mut(&role) else {
            return AttemptSucceeded::AttemptUnknown;
        };
        match std::mem::replace(&mut role_record.attempt, AttemptSlot::Idle) {
            AttemptSlot::Applying {
                attempt: applying,
                completion,
                record,
            } if applying == attempt => {
                completion.finish(DriverCompletion::Succeeded(applied));
                role_record.attempt = AttemptSlot::CompletionQueued { attempt, record };
                AttemptSucceeded::Succeeded
            },
            AttemptSlot::CompletionQueued {
                attempt: queued,
                record,
            } if queued == attempt => {
                role_record.attempt = AttemptSlot::CompletionQueued {
                    attempt: queued,
                    record,
                };
                AttemptSucceeded::AlreadyFinished
            },
            restored => {
                role_record.attempt = restored;
                AttemptSucceeded::AttemptUnknown
            },
        }
    }

    /// Report that device access failed after the attempt started, and take the record back.
    pub fn fail_attempt(
        &mut self,
        attempt: AttemptRef,
        error: DeviceAccessError,
    ) -> AttemptFinish<Records::Attempt> {
        self.finish_attempt(attempt, DriverCompletion::Failed(error))
    }

    /// Report that the driver's own work ended without a device-access failure, and take the
    /// record back.
    pub fn abort_attempt(
        &mut self,
        attempt: AttemptRef,
        reason: DriverAbortReason,
    ) -> AttemptFinish<Records::Attempt> {
        self.finish_attempt(attempt, DriverCompletion::Aborted(reason))
    }

    /// Take the kernel's lease for an accepted completion and settle the establishment races.
    ///
    /// Call it from [`EndpointDriver::established`](crate::EndpointDriver::established), after
    /// whatever unconditional bookkeeping the driver owes its own records — the screen driver's
    /// sweep of the work it had outstanding for the finished job — because that sweep must run on
    /// every path, including the two refusals.
    ///
    /// `into_session` is where the attempt's record becomes the session's: the ledger spends the
    /// queued completion, hands the driver its own attempt record, and files whatever comes back
    /// under the role. A driver whose two records are the same type returns it unchanged; the
    /// window driver narrows a whole restore preparation down to the window entity that survived
    /// it. The conversion runs only on the path that files a session, so a refusal hands the
    /// attempt record back untouched.
    ///
    /// The evidence the role retained through [`Self::credit_flow`] is spent here, so a session
    /// whose first datum arrived before the kernel issued its lease starts from that arrival
    /// instead of from silence it never had.
    pub fn establish_lease(
        &mut self,
        context: EstablishedContext<'_, Configuration>,
        hardware: Establishing,
        into_session: impl FnOnce(Records::Attempt) -> Records::Session,
    ) -> Establishment<Records::Attempt, Records::Session> {
        let role = context.role().clone();
        let attempt = context.attempt();
        let evidence = self.roles.get_mut(&role).map_or(
            SessionDatumArrivalEvidence::NoDatumObserved,
            |role_record| {
                std::mem::replace(
                    &mut role_record.evidence,
                    SessionDatumArrivalEvidence::NoDatumObserved,
                )
            },
        );
        let lease = context.into_lease(evidence);
        let session = lease.session_ref();

        let QueuedCompletion::Spent(record) = self.spend_queued_completion(&role, attempt) else {
            lease.report_loss(DeviceAccessError::Transport {
                detail: format!(
                    "attempt {} was accepted without its queued completion",
                    attempt.get()
                ),
            });
            self.prune_finished_role(&role);
            return Establishment::Refused(EstablishmentRefusal::NoQueuedCompletion);
        };

        if let Establishing::Ended(error) = hardware {
            lease.report_loss(error.clone());
            self.prune_finished_role(&role);
            return Establishment::Refused(EstablishmentRefusal::SessionEnded { error, record });
        }

        let session_record = into_session(record);
        // `spend_queued_completion` answered `Spent` only by reading this role's record, so the
        // entry is a restatement of that, not a second lookup that could fail.
        let role_record = self.roles.entry(role).or_insert_with(RoleRecord::new);
        match std::mem::replace(
            &mut role_record.lease,
            LeaseState::Holding {
                lease,
                record: session_record,
            },
        ) {
            LeaseState::NotEstablished => Establishment::Established { session },
            // The kernel has already replaced the predecessor session, so a report against it
            // would arrive stale; dropping the lease is the whole release. `LossReported` is
            // counted with `Holding` because it is just as unreleased — the driver's hardware
            // record for it is still open either way.
            LeaseState::Holding {
                record: predecessor,
                ..
            }
            | LeaseState::LossReported {
                record: predecessor,
                ..
            } => Establishment::EstablishedOverUnreleased {
                session,
                predecessor,
            },
            LeaseState::Retained(retained) => {
                Establishment::EstablishedOverRetained { session, retained }
            },
        }
    }

    /// Credit one classified transport observation against the role's flow.
    ///
    /// Call it where the datum reaches its consumer, not where it is polled: a datum discarded in
    /// between is [`TransportObservation::Quiet`](crate::TransportObservation::Quiet), because
    /// nobody saw it. A driver that must guard the credit with a fact only it holds — the camera's
    /// stream generation, the screen's current job — reads its own record first and then makes
    /// this one call.
    pub fn credit_flow<Transport>(
        &mut self,
        role: &RoleKey,
        observation: &Transport::Observation,
        observed_at: Instant,
    ) -> FlowCredit
    where
        Transport: DeviceTransport,
    {
        let Some(role_record) = self.roles.get_mut(role) else {
            return FlowCredit::UnknownRole;
        };
        match &mut role_record.lease {
            LeaseState::Holding { lease, .. } => {
                lease.observe::<Transport>(observation);
                FlowCredit::CreditedToSession
            },
            LeaseState::LossReported { .. } => FlowCredit::SessionEnding,
            LeaseState::Retained(_) | LeaseState::NotEstablished => {
                role_record
                    .evidence
                    .retain::<Transport>(observation, observed_at);
                FlowCredit::RetainedForEstablishment
            },
        }
    }

    /// Report a repeatable external configuration change against the role's session.
    pub fn change_configuration(
        &mut self,
        role: &RoleKey,
        configuration: Configuration,
    ) -> ConfigurationChange {
        let Some(role_record) = self.roles.get_mut(role) else {
            return ConfigurationChange::NotEstablished;
        };
        match &mut role_record.lease {
            LeaseState::Holding { lease, .. } => {
                lease.configuration_changed(configuration);
                ConfigurationChange::Applied
            },
            LeaseState::LossReported { .. } => ConfigurationChange::SessionEnding,
            LeaseState::Retained(_) | LeaseState::NotEstablished => {
                ConfigurationChange::NotEstablished
            },
        }
    }

    /// Report that the role's session lost its device access.
    ///
    /// The lease is consumed by the report, and the ledger keeps its [`SessionRef`] — and the
    /// driver's session record — so the release the kernel sends afterwards still matches and
    /// still has hardware to hand over.
    pub fn report_loss(&mut self, role: &RoleKey, error: DeviceAccessError) -> LossReport {
        let Some(role_record) = self.roles.get_mut(role) else {
            return LossReport::NotEstablished;
        };
        match std::mem::replace(&mut role_record.lease, LeaseState::NotEstablished) {
            LeaseState::Holding { lease, record } => {
                let session = lease.session_ref();
                lease.report_loss(error);
                role_record.lease = LeaseState::LossReported { session, record };
                LossReport::Reported
            },
            LeaseState::LossReported { session, record } => {
                role_record.lease = LeaseState::LossReported { session, record };
                LossReport::AlreadyReported
            },
            restored @ (LeaseState::Retained(_) | LeaseState::NotEstablished) => {
                role_record.lease = restored;
                LossReport::NotEstablished
            },
        }
    }

    /// Drop the retained completion for an attempt the kernel has ended, and take the record back.
    ///
    /// Call it from [`EndpointDriver::cancel_apply`](crate::EndpointDriver::cancel_apply), and
    /// from a retirement observer running outside the contract methods — no verb here is reserved
    /// to the trait. The completion is dropped unused: the kernel invalidated the attempt, so
    /// finishing it would report a result against work that no longer exists.
    pub fn cancel_attempt(
        &mut self,
        role: &RoleKey,
        attempt: AttemptRef,
    ) -> CancelledAttempt<Records::Attempt> {
        match self.attempts.get(&attempt) {
            None => return CancelledAttempt::Unknown,
            Some(indexed) if indexed != role => return CancelledAttempt::WrongRole,
            Some(_) => {},
        }
        let Some(role_record) = self.roles.get_mut(role) else {
            return CancelledAttempt::Unknown;
        };
        let cancelled = match std::mem::replace(&mut role_record.attempt, AttemptSlot::Idle) {
            AttemptSlot::Applying {
                attempt: applying,
                record,
                ..
            } if applying == attempt => CancelledAttempt::Applying { record },
            AttemptSlot::CompletionQueued {
                attempt: queued,
                record,
            } if queued == attempt => CancelledAttempt::CompletionQueued { record },
            restored => {
                role_record.attempt = restored;
                CancelledAttempt::Unknown
            },
        };
        if matches!(
            cancelled,
            CancelledAttempt::Applying { .. } | CancelledAttempt::CompletionQueued { .. }
        ) {
            self.attempts.remove(&attempt);
            self.prune_finished_role(role);
        }

        cancelled
    }

    /// Give back the role's lease when the kernel names the session the role still holds, keeping
    /// that session's record.
    ///
    /// Call it from
    /// [`EndpointDriver::release_session`](crate::EndpointDriver::release_session), or from a
    /// retirement observer, and act on hardware only for [`ReleasedLease::Released`]: a release
    /// naming a session that a successor already replaced answers
    /// [`ReleasedLease::OtherSessionEstablished`], and tearing down there would end the
    /// successor's device work.
    ///
    /// The session's record moves to the ledger's retained slot rather than coming back here,
    /// because the common case is that the driver keeps using it: the camera keeps a frozen
    /// picture for the successor, the window driver keeps a hidden window through a loss. A
    /// successor finds it on [`Establishment::EstablishedOverRetained`]; a driver that keeps
    /// nothing past release calls [`Self::discard_retained`] immediately after this.
    pub fn release_lease(&mut self, role: &RoleKey, session: SessionRef) -> ReleasedLease {
        let Some(role_record) = self.roles.get_mut(role) else {
            return ReleasedLease::NotEstablished;
        };
        match std::mem::replace(&mut role_record.lease, LeaseState::NotEstablished) {
            LeaseState::Holding { lease, record } if lease.session_ref() == session => {
                role_record.lease = LeaseState::Retained(record);
                role_record.evidence = SessionDatumArrivalEvidence::NoDatumObserved;
                ReleasedLease::Released
            },
            LeaseState::LossReported {
                session: reported,
                record,
            } if reported == session => {
                role_record.lease = LeaseState::Retained(record);
                role_record.evidence = SessionDatumArrivalEvidence::NoDatumObserved;
                ReleasedLease::Released
            },
            restored @ (LeaseState::Holding { .. } | LeaseState::LossReported { .. }) => {
                role_record.lease = restored;
                ReleasedLease::OtherSessionEstablished
            },
            restored @ (LeaseState::Retained(_) | LeaseState::NotEstablished) => {
                role_record.lease = restored;
                ReleasedLease::NotEstablished
            },
        }
    }

    /// End the ledger's interest in a session record the role kept past its release.
    ///
    /// The counterpart to [`Self::release_lease`]: a driver that keeps nothing after a release
    /// calls this at once and closes whatever comes back, and a driver that does keep something
    /// calls it when the reason to keep it has passed — a retirement observer, a successor that
    /// chose not to adopt. Until it is called, the role record stays in the ledger holding that
    /// record, which is what makes it findable through [`Self::session_record`] and what stops the
    /// ledger pruning the role out from under it.
    pub fn discard_retained(&mut self, role: &RoleKey) -> DiscardedSession<Records::Session> {
        let Some(role_record) = self.roles.get_mut(role) else {
            return DiscardedSession::NothingRetained;
        };
        let discarded = match std::mem::replace(&mut role_record.lease, LeaseState::NotEstablished)
        {
            LeaseState::Retained(record) => DiscardedSession::Discarded(record),
            restored => {
                role_record.lease = restored;
                DiscardedSession::NothingRetained
            },
        };
        if matches!(discarded, DiscardedSession::Discarded(_)) {
            self.prune_finished_role(role);
        }

        discarded
    }

    /// Read which attempt this role is running.
    #[must_use]
    pub fn attempt_of(&self, role: &RoleKey) -> AttemptLookup {
        match self.roles.get(role).map(|role_record| &role_record.attempt) {
            Some(AttemptSlot::Applying { attempt, .. }) => AttemptLookup::Applying(*attempt),
            Some(AttemptSlot::CompletionQueued { attempt, .. }) => {
                AttemptLookup::CompletionQueued(*attempt)
            },
            Some(AttemptSlot::Idle) | None => AttemptLookup::Idle,
        }
    }

    /// Read which session this role holds.
    #[must_use]
    pub fn session_of(&self, role: &RoleKey) -> SessionLookup {
        match self.roles.get(role).map(|role_record| &role_record.lease) {
            Some(LeaseState::Holding { lease, .. }) => SessionLookup::Holding(lease.session_ref()),
            Some(LeaseState::LossReported { session, .. }) => SessionLookup::LossReported(*session),
            Some(LeaseState::Retained(_) | LeaseState::NotEstablished) | None => {
                SessionLookup::NotEstablished
            },
        }
    }

    /// Read whether this ledger is holding anything at all, for any role.
    ///
    /// The per-role readers answer what one named role holds, which cannot catch work filed under
    /// a role nobody thought to name — an attempt keyed by the wrong role reads as empty from
    /// every role a test names. A startup test asserting that a driver issued nothing needs
    /// the whole-ledger answer, and this is it: no role holds an attempt slot, a lease, or a
    /// record retained past a release.
    ///
    /// Datum evidence a role collected for an attempt that never became a session does not count
    /// as held, for the same reason [`Self::release_lease`] and the finish verbs prune a role
    /// record that carries only evidence: the evidence belongs to the attempt, and the attempt is
    /// gone.
    #[must_use]
    pub fn holds_no_role(&self) -> bool { self.roles.values().all(RoleRecord::holds_nothing) }

    /// Read the driver's own record for one attempt, and which slot state holds it.
    #[must_use]
    pub fn attempt_record(&self, attempt: AttemptRef) -> AttemptRecord<'_, Records::Attempt> {
        let Some(role) = self.attempts.get(&attempt) else {
            return AttemptRecord::AttemptUnknown;
        };
        let Some(role_record) = self.roles.get(role) else {
            return AttemptRecord::AttemptUnknown;
        };
        match &role_record.attempt {
            AttemptSlot::Applying {
                attempt: applying,
                record,
                ..
            } if *applying == attempt => AttemptRecord::Applying(record),
            AttemptSlot::CompletionQueued {
                attempt: queued,
                record,
            } if *queued == attempt => AttemptRecord::CompletionQueued(record),
            AttemptSlot::Idle
            | AttemptSlot::Applying { .. }
            | AttemptSlot::CompletionQueued { .. } => AttemptRecord::AttemptUnknown,
        }
    }

    /// Amend the driver's own record for one attempt in place.
    #[must_use]
    pub fn attempt_record_mut(
        &mut self,
        attempt: AttemptRef,
    ) -> AttemptRecordMut<'_, Records::Attempt> {
        let Some(role) = self.attempts.get(&attempt) else {
            return AttemptRecordMut::AttemptUnknown;
        };
        let Some(role_record) = self.roles.get_mut(role) else {
            return AttemptRecordMut::AttemptUnknown;
        };
        match &mut role_record.attempt {
            AttemptSlot::Applying {
                attempt: applying,
                record,
                ..
            } if *applying == attempt => AttemptRecordMut::Applying(record),
            AttemptSlot::CompletionQueued {
                attempt: queued,
                record,
            } if *queued == attempt => AttemptRecordMut::CompletionQueued(record),
            AttemptSlot::Idle
            | AttemptSlot::Applying { .. }
            | AttemptSlot::CompletionQueued { .. } => AttemptRecordMut::AttemptUnknown,
        }
    }

    /// Read the driver's own record for one role's session, and which state holds it.
    #[must_use]
    pub fn session_record(&self, role: &RoleKey) -> SessionRecord<'_, Records::Session> {
        match self.roles.get(role).map(|role_record| &role_record.lease) {
            Some(LeaseState::Holding { record, .. }) => SessionRecord::Holding(record),
            Some(LeaseState::LossReported { record, .. }) => SessionRecord::LossReported(record),
            Some(LeaseState::Retained(record)) => SessionRecord::Retained(record),
            Some(LeaseState::NotEstablished) | None => SessionRecord::NoSession,
        }
    }

    /// Amend the driver's own record for one role's session in place.
    #[must_use]
    pub fn session_record_mut(&mut self, role: &RoleKey) -> SessionRecordMut<'_, Records::Session> {
        match self
            .roles
            .get_mut(role)
            .map(|role_record| &mut role_record.lease)
        {
            Some(LeaseState::Holding { record, .. }) => SessionRecordMut::Holding(record),
            Some(LeaseState::LossReported { record, .. }) => SessionRecordMut::LossReported(record),
            Some(LeaseState::Retained(record)) => SessionRecordMut::Retained(record),
            Some(LeaseState::NotEstablished) | None => SessionRecordMut::NoSession,
        }
    }

    /// Read both driver-record slots for one role.
    ///
    /// The attempt and session are independent slots: during replacement both can be occupied,
    /// and neither is an acceptable substitute for the other. An unknown role therefore answers
    /// two explicit `NoRecord` states instead of a bare absence.
    #[must_use]
    pub fn record_of(&self, role: &RoleKey) -> RoleRecords<'_, Records::Attempt, Records::Session> {
        let Some(role_record) = self.roles.get(role) else {
            return RoleRecords {
                attempt: RoleAttemptRecord::NoRecord,
                session: RoleSessionRecord::NoRecord,
            };
        };
        role_records(role_record)
    }

    /// Amend both driver-record slots for one role in place.
    ///
    /// This is the mutable counterpart to [`Self::record_of`]. Both references come from the same
    /// role visit, so a caller cannot accidentally resolve the attempt before a transition and the
    /// session after it.
    #[must_use]
    pub fn record_of_mut(
        &mut self,
        role: &RoleKey,
    ) -> RoleRecordsMut<'_, Records::Attempt, Records::Session> {
        let Some(role_record) = self.roles.get_mut(role) else {
            return RoleRecordsMut {
                attempt: RoleAttemptRecordMut::NoRecord,
                session: RoleSessionRecordMut::NoRecord,
            };
        };
        role_records_mut(role_record)
    }

    /// Visit every role once with both of its driver-record slots visible.
    ///
    /// A role with a replacement in flight and a predecessor session produces one item carrying
    /// both records. This is deliberately not the chained union of [`Self::attempts`] and
    /// [`Self::sessions`]: that union visits a two-slot role twice and invites either double work
    /// or an attempt-first filter that hides its predecessor.
    pub fn records(
        &self,
    ) -> impl Iterator<
        Item = (
            &RoleKey,
            RoleRecords<'_, Records::Attempt, Records::Session>,
        ),
    > {
        self.roles
            .iter()
            .map(|(role, role_record)| (role, role_records(role_record)))
    }

    /// Visit every role once with both of its driver-record slots mutable.
    ///
    /// A closure keeps the two disjoint mutable slot borrows inside one ledger traversal. It is
    /// the mutable counterpart to [`Self::records`] for adoption and retirement passes that must
    /// see a predecessor beside its replacement.
    pub fn for_each_record_mut(
        &mut self,
        mut visit: impl FnMut(&RoleKey, RoleRecordsMut<'_, Records::Attempt, Records::Session>),
    ) {
        for (role, role_record) in &mut self.roles {
            visit(role, role_records_mut(role_record));
        }
    }

    /// Walk every in-flight attempt's record, mutably.
    ///
    /// The window driver's `abort_window` is the shape this serves: work arrives naming a window
    /// entity, and every applying attempt whose preparation names that window has to end while
    /// every queued one has to be marked. Only the driver's own record can answer which those are,
    /// so the walk is over records rather than over roles.
    ///
    /// Roles whose slot is idle are skipped, so the item type is [`InFlightAttemptMut`], which has
    /// no absent arm at all.
    pub fn attempts_mut(
        &mut self,
    ) -> impl Iterator<
        Item = (
            AttemptRef,
            &RoleKey,
            InFlightAttemptMut<'_, Records::Attempt>,
        ),
    > {
        self.roles
            .iter_mut()
            .filter_map(|(role, role_record)| match &mut role_record.attempt {
                AttemptSlot::Applying {
                    attempt, record, ..
                } => Some((*attempt, role, InFlightAttemptMut::Applying(record))),
                AttemptSlot::CompletionQueued { attempt, record } => {
                    Some((*attempt, role, InFlightAttemptMut::CompletionQueued(record)))
                },
                AttemptSlot::Idle => None,
            })
    }

    /// Walk every in-flight attempt's record, immutably.
    ///
    /// The immutable half of [`Self::attempts_mut`], for the readings a driver makes where it
    /// holds no `&mut` of its own. Roles whose slot is idle are skipped, so the item type is
    /// [`InFlightAttempt`], which has no absent arm at all.
    pub fn attempts(
        &self,
    ) -> impl Iterator<Item = (AttemptRef, &RoleKey, InFlightAttempt<'_, Records::Attempt>)> {
        self.roles
            .iter()
            .filter_map(|(role, role_record)| match &role_record.attempt {
                AttemptSlot::Applying {
                    attempt, record, ..
                } => Some((*attempt, role, InFlightAttempt::Applying(record))),
                AttemptSlot::CompletionQueued { attempt, record } => {
                    Some((*attempt, role, InFlightAttempt::CompletionQueued(record)))
                },
                AttemptSlot::Idle => None,
            })
    }

    /// Walk every session record this ledger holds.
    ///
    /// The per-frame loops are what this serves: polling every open stream, crediting arrivals,
    /// and finding the roles that share one physical device. Roles carrying no session record are
    /// skipped, so the item type is [`HeldSession`], which has no absent arm at all.
    pub fn sessions(&self) -> impl Iterator<Item = (&RoleKey, HeldSession<'_, Records::Session>)> {
        self.roles
            .iter()
            .filter_map(|(role, role_record)| match &role_record.lease {
                LeaseState::Holding { record, .. } => Some((role, HeldSession::Holding(record))),
                LeaseState::LossReported { record, .. } => {
                    Some((role, HeldSession::LossReported(record)))
                },
                LeaseState::Retained(record) => Some((role, HeldSession::Retained(record))),
                LeaseState::NotEstablished => None,
            })
    }

    /// Walk every session record this ledger holds, mutably.
    ///
    /// Roles carrying no session record are skipped, so the item type is [`HeldSessionMut`], which
    /// has no absent arm at all.
    pub fn sessions_mut(
        &mut self,
    ) -> impl Iterator<Item = (&RoleKey, HeldSessionMut<'_, Records::Session>)> {
        self.roles
            .iter_mut()
            .filter_map(|(role, role_record)| match &mut role_record.lease {
                LeaseState::Holding { record, .. } => Some((role, HeldSessionMut::Holding(record))),
                LeaseState::LossReported { record, .. } => {
                    Some((role, HeldSessionMut::LossReported(record)))
                },
                LeaseState::Retained(record) => Some((role, HeldSessionMut::Retained(record))),
                LeaseState::NotEstablished => None,
            })
    }

    /// Find the in-flight attempt whose record satisfies a driver's predicate.
    ///
    /// This serves the one lookup that arrives keyed by neither the attempt nor the role — the
    /// screen kernel's job outcome channel, which carries a job identifier only the driver's own
    /// record holds. A lookup that has the role is [`Self::attempt_of`] followed by
    /// [`Self::attempt_record`], never a scan.
    #[must_use]
    pub fn find_attempt(
        &self,
        mut predicate: impl FnMut(&Records::Attempt) -> bool,
    ) -> AttemptSearch {
        for (role, role_record) in &self.roles {
            let (attempt, record) = match &role_record.attempt {
                AttemptSlot::Applying {
                    attempt, record, ..
                }
                | AttemptSlot::CompletionQueued { attempt, record } => (*attempt, record),
                AttemptSlot::Idle => continue,
            };
            if predicate(record) {
                return AttemptSearch::Found {
                    attempt,
                    role: role.clone(),
                };
            }
        }

        AttemptSearch::NoMatch
    }

    /// Spend the retained completion on one non-success terminal result and take the record back.
    ///
    /// A failure or abort returns the slot to `Idle` and drops the index entry, which is what
    /// makes a second failure answer [`AttemptFinish::AttemptUnknown`] where a failure after a
    /// success answers [`AttemptFinish::AlreadyFinished`].
    fn finish_attempt(
        &mut self,
        attempt: AttemptRef,
        completion: DriverCompletion<Configuration>,
    ) -> AttemptFinish<Records::Attempt> {
        let Some(role) = self.attempts.get(&attempt).cloned() else {
            return AttemptFinish::AttemptUnknown;
        };
        let Some(role_record) = self.roles.get_mut(&role) else {
            return AttemptFinish::AttemptUnknown;
        };
        let finish = match std::mem::replace(&mut role_record.attempt, AttemptSlot::Idle) {
            AttemptSlot::Applying {
                attempt: applying,
                completion: retained,
                record,
            } if applying == attempt => {
                retained.finish(completion);
                AttemptFinish::Finished { record }
            },
            AttemptSlot::CompletionQueued {
                attempt: queued,
                record,
            } if queued == attempt => {
                role_record.attempt = AttemptSlot::CompletionQueued {
                    attempt: queued,
                    record,
                };
                AttemptFinish::AlreadyFinished
            },
            restored => {
                role_record.attempt = restored;
                AttemptFinish::AttemptUnknown
            },
        };
        if self
            .roles
            .get(&role)
            .is_some_and(|role_record| matches!(role_record.attempt, AttemptSlot::Idle))
        {
            self.attempts.remove(&attempt);
            self.prune_finished_role(&role);
        }

        finish
    }

    /// Consume the role's queued completion for that exact attempt, handing back the record it
    /// carried.
    ///
    /// Establishment is the only caller: it is the moment the kernel accepts the success the
    /// completion carried, so the slot returns to idle whether or not the session survives.
    fn spend_queued_completion(
        &mut self,
        role: &RoleKey,
        attempt: AttemptRef,
    ) -> QueuedCompletion<Records::Attempt> {
        let Some(role_record) = self.roles.get_mut(role) else {
            return QueuedCompletion::NotQueued;
        };
        match std::mem::replace(&mut role_record.attempt, AttemptSlot::Idle) {
            AttemptSlot::CompletionQueued {
                attempt: queued,
                record,
            } if queued == attempt => {
                self.attempts.remove(&attempt);
                QueuedCompletion::Spent(record)
            },
            restored => {
                role_record.attempt = restored;
                QueuedCompletion::NotQueued
            },
        }
    }

    /// Remove a role record that holds neither kernel-issued authority nor a driver record.
    ///
    /// The retained evidence goes with it: a datum observed for an attempt that failed belonged to
    /// a session that never existed, and crediting it to the next attempt would start that session
    /// flowing on an arrival it never saw.
    fn prune_finished_role(&mut self, role: &RoleKey) {
        if self.roles.get(role).is_some_and(RoleRecord::holds_nothing) {
            self.roles.remove(role);
        }
    }
}

const fn role_records<Configuration, Attempt, Session>(
    role_record: &RoleRecord<Configuration, Attempt, Session>,
) -> RoleRecords<'_, Attempt, Session> {
    let attempt = match &role_record.attempt {
        AttemptSlot::Applying { record, .. } => RoleAttemptRecord::Applying(record),
        AttemptSlot::CompletionQueued { record, .. } => RoleAttemptRecord::CompletionQueued(record),
        AttemptSlot::Idle => RoleAttemptRecord::NoRecord,
    };
    let session = match &role_record.lease {
        LeaseState::Holding { record, .. } => RoleSessionRecord::Holding(record),
        LeaseState::LossReported { record, .. } => RoleSessionRecord::LossReported(record),
        LeaseState::Retained(record) => RoleSessionRecord::Retained(record),
        LeaseState::NotEstablished => RoleSessionRecord::NoRecord,
    };
    RoleRecords { attempt, session }
}

const fn role_records_mut<Configuration, Attempt, Session>(
    role_record: &mut RoleRecord<Configuration, Attempt, Session>,
) -> RoleRecordsMut<'_, Attempt, Session> {
    let attempt = match &mut role_record.attempt {
        AttemptSlot::Applying { record, .. } => RoleAttemptRecordMut::Applying(record),
        AttemptSlot::CompletionQueued { record, .. } => {
            RoleAttemptRecordMut::CompletionQueued(record)
        },
        AttemptSlot::Idle => RoleAttemptRecordMut::NoRecord,
    };
    let session = match &mut role_record.lease {
        LeaseState::Holding { record, .. } => RoleSessionRecordMut::Holding(record),
        LeaseState::LossReported { record, .. } => RoleSessionRecordMut::LossReported(record),
        LeaseState::Retained(record) => RoleSessionRecordMut::Retained(record),
        LeaseState::NotEstablished => RoleSessionRecordMut::NoRecord,
    };
    RoleRecordsMut { attempt, session }
}
