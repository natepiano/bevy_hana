//! Events the kernel emits on the edge where a reconciled fact changed.
//!
//! Every event here fires on a change edge and never on a settled frame, so a consumer can treat
//! one arrival as one transition instead of debouncing a per-frame restatement.
//!
//! # Which events exist, and why
//!
//! The list is derived from the state axes the kernel mirrors onto an entity, not accumulated one
//! case at a time: `crate::Presence`, `crate::Claim`, `crate::IdentityVerdict`,
//! `crate::RecoveryPolicy`, `crate::RoleState`, and attempt completion each have exactly one
//! event. A mirrored axis with no event, or an event with no mirrored axis, is the defect this
//! derivation exists to prevent.
//!
//! Each transition event carries only the state moved *to*. The state moved *from* is still on the
//! entity when an observer runs, so duplicating it in the payload would let the two disagree.
//!
//! # Two axes deliberately outside the derivation
//!
//! `crate::WaitingWork` is not on the binding entity and `crate::IdentityDecisionOwed` is not on
//! the device entity, so neither is mirrored. A consumer reading only entities or the Bevy Remote
//! Protocol therefore cannot see that a role owes a restoration or that a human owes an identity
//! decision; both are read from the resources instead — `crate::Bindings::waiting_work` and
//! `crate::ReconciledDeviceState::decision_owed`. This is stated rather than left implicit so that
//! surfacing either one is a decision somebody makes, not a hole somebody finds.
//!
//! `IdentityQuestionRaised` and `IdentityQuestionExpired` are that decision, taken for the identity
//! debt and for nothing else. Every other axis the kernel reports is state an application can read
//! whenever it gets around to it, whereas an identity question exists to make a human act, and one
//! that is never noticed leaves a device unusable. A consumer that only polls
//! `crate::IdentityDecisions` cannot see a question that arrived and expired between two reads, and
//! a dialog it opened has no signal that the entry vanished underneath it.
//!
//! # Why some events target an entity and some are global
//!
//! An event is an `EntityEvent` only when an entity is guaranteed to exist to receive it. A role
//! whose device has never appeared, or whose binding was retired in the same frame its attempt
//! ended, has no valid `Entity` to name — so those cases are global `Event`s carrying the durable
//! `crate::DeviceKey` or `crate::RoleKey` instead.
//!
//! Per-unit facts (`crate::Presence`, `crate::Claim`, `crate::IdentityVerdict`) target the device
//! entity; per-role facts (`crate::RoleState`, `crate::RecoveryPolicy`, attempt completion) target
//! the binding entity. The split is not stylistic: a device departure can despawn the device entity
//! while an attempt is still finishing, so an `AttemptFinished` aimed at the device entity would
//! have nowhere to land.

use std::collections::VecDeque;

use bevy::ecs::event::EntityEvent;
use bevy::ecs::event::Event;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;

use crate::AttemptId;
use crate::AttemptOutcome;
use crate::Claim;
use crate::CompletedDiscoveryOutcome;
use crate::ConfiguredDeviceConnection;
use crate::DeviceAccessError;
use crate::DeviceId;
use crate::DeviceKey;
use crate::DeviceRevision;
use crate::DiscoveryBatchId;
use crate::DiscoveryProgress;
use crate::IdentityVerdict;
use crate::Presence;
use crate::RecoveryPolicy;
use crate::ReporterId;
use crate::RoleKey;
use crate::RoleState;
use crate::SchemeName;
use crate::StartupDiscoveryState;
use crate::devices::DeviceDeparture;

/// The capabilities two reporters disagree about for this device changed.
///
/// A co-reported unit whose reporters contradict each other about one capability stays drivable
/// for every capability they agree about, so the disagreement has to reach a diagnostic somehow:
/// nothing else in the kernel reports which capability went contested. Emitted on the change edge
/// only. An empty `capabilities` means the disagreement cleared and the device is fully drivable
/// again.
#[derive(Debug, EntityEvent, Reflect)]
pub struct CapabilitiesDisputed {
    /// Device entity whose contributors changed what they disagree about.
    #[event_target]
    pub device:       Entity,
    /// Reflected type paths of the disputed capability components, resolved from
    /// `crate::ReconciledDeviceState::disputed` through the type registry.
    ///
    /// Type paths rather than the `std::any::TypeId` values the kernel stores, because `TypeId`
    /// does not implement `Reflect` and so could not cross an event a Bevy Remote Protocol client
    /// reads — and the path is what a human reading a warning needs anyway.
    pub capabilities: Vec<String>,
}

/// One attempt reached a terminal outcome while its role still had a binding entity.
///
/// Targeted at the binding entity rather than the device entity because the binding outlives the
/// unit: an attempt that ends because the device departed still has somewhere to land. The role is
/// carried alongside the target so a consumer that observes the event does not have to read the
/// entity's components back to learn which role ended.
#[derive(Debug, EntityEvent, Reflect)]
pub struct AttemptFinished {
    /// Binding entity for the role this attempt ran for.
    #[event_target]
    pub binding: Entity,
    /// Application role the attempt ran for.
    pub role:    RoleKey,
    /// Registry-issued identifier of the attempt that ended.
    pub attempt: AttemptId,
    /// Terminal result the attempt ended with.
    pub outcome: AttemptOutcome,
}

/// Guarded report that an established local endpoint session ended while its device remained
/// present.
///
/// Integration code submits this value outside [`crate::EndpointDriver`] dispatch. The kernel
/// later compares the establishing attempt and both process-local device fields with the role's
/// current resolution at
/// [`crate::RiggingSystems::SessionLoss`], so a report retained from a replaced or rebound session
/// cannot move its successor.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub struct SessionLossReport {
    /// Stable application role whose established local session ended.
    pub role:                     RoleKey,
    /// Globally unique successful attempt that established the dead session.
    pub establishing_attempt:     AttemptId,
    /// Process-local device handle the dead session was established against.
    pub expected_device_id:       DeviceId,
    /// Per-device revision the dead session most recently observed as current.
    pub expected_device_revision: DeviceRevision,
    /// Typed failure supplied by the integration that owned the local session.
    pub error:                    DeviceAccessError,
}

impl SessionLossReport {
    /// Describe one dead established session using the exact device guard retained with it.
    #[must_use]
    pub const fn new(
        role: RoleKey,
        establishing_attempt: AttemptId,
        expected_device_id: DeviceId,
        expected_device_revision: DeviceRevision,
        error: DeviceAccessError,
    ) -> Self {
        Self {
            role,
            establishing_attempt,
            expected_device_id,
            expected_device_revision,
            error,
        }
    }
}

/// Inbox integrations use to hand established-session failures to the ordered kernel lifecycle.
///
/// Submission only retains a report. It deliberately does not mutate [`crate::Bindings`] or
/// [`crate::Devices`]; [`crate::RiggingPlugin`] drains the inbox at
/// [`crate::RiggingSystems::SessionLoss`] after application preparation and before apply dispatch.
#[derive(Default, Resource)]
pub struct SessionLossReports {
    pending: VecDeque<SessionLossReport>,
}

impl SessionLossReports {
    /// Retain a guarded established-session failure for ordered kernel processing.
    pub fn submit(&mut self, report: SessionLossReport) { self.pending.push_back(report); }

    pub(crate) fn drain(&mut self) -> impl Iterator<Item = SessionLossReport> + '_ {
        self.pending.drain(..)
    }
}

/// Why one submitted session-loss report was refused as stale or inapplicable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum SessionLossRefusal {
    /// No authored binding currently exists for the reported role.
    RoleAbsent,
    /// The role was retired before this lifecycle point processed the report.
    RoleRetired,
    /// A newly authored binding replaced the role the report came from.
    RoleReplaced,
    /// The current binding resolves to a different process-local device handle.
    DeviceRebound,
    /// The role has no established session to lose in its current lifecycle state.
    SessionNotEstablished,
    /// The resolved device changed after the dead session's retained guard was stamped.
    DeviceRevisionChanged,
    /// The report names a successful attempt replaced by the role's current established session.
    EstablishingAttemptReplaced,
}

/// Kernel decision made for one guarded established-session failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum SessionLossDisposition {
    /// [`crate::OnSessionLoss::Recreate`] moved the role into paced ordinary recovery.
    RecreateScheduled,
    /// [`crate::OnSessionLoss::ReportOnly`] exposed the loss without scheduling a replacement.
    ReportedOnly,
    /// The report did not describe the role and device revision currently established.
    Refused(SessionLossRefusal),
}

/// Observable result of processing one [`SessionLossReport`].
///
/// This is global rather than entity-targeted because retirement and replacement can remove the
/// binding entity before a stale report reaches the ordered lifecycle point.
#[derive(Debug, Event, Reflect)]
pub struct SessionLossProcessed {
    /// Guarded integration report the kernel evaluated.
    pub report:      SessionLossReport,
    /// Named accepted policy or refusal reason.
    pub disposition: SessionLossDisposition,
}

/// An attempt ended after its role was retired or replaced, so no binding entity remained.
///
/// Global rather than entity-targeted: retirement despawns the binding entity in the same frame the
/// kernel aborts the attempt, and an event addressed to a despawned entity reaches no observer at
/// all. The ending still has to be reportable, so it carries the `RoleKey` the entity would have
/// identified.
#[derive(Debug, Event, Reflect)]
pub struct RetiredRoleAttemptEnded {
    /// Application role whose binding was retired or replaced out from under the attempt.
    pub role:    RoleKey,
    /// Registry-issued identifier of the attempt that ended.
    pub attempt: AttemptId,
    /// Terminal result the attempt ended with, which is `crate::AttemptOutcome::Aborted` whenever
    /// the kernel rather than the driver ended it.
    pub outcome: AttemptOutcome,
}

/// An authored role left the kernel's binding set and no longer authorizes its endpoint.
///
/// This is global rather than entity-targeted because applying the retirement despawns the
/// binding entity in the same lifecycle stage. The role and endpoint remain useful to an
/// integration that must retire its own local session or visible representation without looking
/// up an entity that no longer exists.
#[derive(Debug, Event, Reflect)]
pub struct BindingRetired {
    /// Application role whose retained authorization was removed.
    pub role:     RoleKey,
    /// Exact durable device address that ceased to be authorized for this role.
    pub endpoint: crate::DeviceEndpoint,
}

/// A durable key entered the reconciled device set and now has a device entity behind it.
///
/// This is what lets an integration say "*my* Stream Deck came back" by observing one entity
/// instead of writing a global match arm over every device kind the process reports. It fires once
/// per spawn: a unit that goes absent without its key leaving the set keeps its entity and produces
/// a `PresenceChanged` instead, so a second `DeviceArrived` for the same entity never happens.
#[derive(Debug, EntityEvent, Reflect)]
pub struct DeviceArrived {
    /// Device entity the projection just spawned for this key.
    #[event_target]
    pub device: Entity,
    /// Durable name of the unit, carried so an observer can match an authored inventory entry
    /// without reading the entity back.
    pub key:    DeviceKey,
}

/// Reachability for this unit moved to a different `crate::Presence` variant.
///
/// Compared by variant, never by value: `crate::Presence::Unreachable` carries an elapsed time that
/// grows on every scan, so a value comparison would emit this event at scan rate forever and defeat
/// the once-per-change rule the whole module is built on.
#[derive(Debug, EntityEvent, Reflect)]
pub struct PresenceChanged {
    /// Device entity whose reachability moved.
    #[event_target]
    pub device:   Entity,
    /// Reachability the unit moved *to*. The prior value is still on the entity while an observer
    /// runs, so it is deliberately not duplicated here.
    pub presence: Presence,
}

/// Exclusive ownership of this unit changed hands, or the permission gating it did.
///
/// Separate from `PresenceChanged` because a camera can be plugged in and fully present while
/// another process owns its capture stream: a consumer that treated the two as one axis would show
/// a contended camera as missing hardware.
#[derive(Debug, EntityEvent, Reflect)]
pub struct ClaimChanged {
    /// Device entity whose exclusive ownership moved.
    #[event_target]
    pub device: Entity,
    /// Ownership state the unit moved *to*.
    pub claim:  Claim,
}

/// The kernel reached a different conclusion about whether this unit is the one its key names.
///
/// This is the event a `crate::IdentityVerdict::Displaced` conclusion reaches a consumer through: a
/// unit that moved to the port a departed one occupied is drivable for nothing until a human
/// resolves it, and nothing else reports that the conclusion changed.
#[derive(Debug, EntityEvent, Reflect)]
pub struct IdentityChanged {
    /// Device entity whose identity conclusion moved.
    #[event_target]
    pub device:  Entity,
    /// Conclusion the kernel moved *to*.
    pub verdict: IdentityVerdict,
}

/// The kernel added a question to `crate::IdentityDecisions` that only a human can settle.
///
/// A stated exception to the derivation above: `crate::IdentityDecisionOwed` has no mirrored axis,
/// and this event exists because a question nobody notices leaves a device unusable for the life of
/// the process. What an application does with it is its own — a notification that expands into the
/// register, an attention marker on the mesh representing that hardware.
///
/// Global rather than entity-targeted because it names two sides at once: the role's binding entity
/// may not exist while its device is absent, and the candidate's device entity is not what the
/// operator is being asked about.
#[derive(Debug, Event, Reflect)]
pub struct IdentityQuestionRaised {
    /// Application role whose saved key the candidate may replace.
    pub role:      RoleKey,
    /// Durable key of the unit that arrived into the attachment the saved one left. With `role` it
    /// names the register entry, so an observer can correlate this arrival with the
    /// `IdentityQuestionExpired` that cancels it.
    pub candidate: DeviceKey,
}

/// A standing identity question went away without being answered.
///
/// The other half of the stated exception `IdentityQuestionRaised` documents. It fires when the
/// candidate device departs or the role is retired, and never for an entry an answer removed: a
/// dialog the operator is looking at needs to know the question underneath it is gone, and an
/// application that answered already knows.
#[derive(Debug, Event, Reflect)]
pub struct IdentityQuestionExpired {
    /// Application role the expired question was about.
    pub role:      RoleKey,
    /// Durable key of the candidate whose question expired.
    pub candidate: DeviceKey,
}

/// A role's lifecycle state moved.
///
/// Emitted from `crate::RiggingSystems::Apply` beside `AttemptFinished`, not from the entity
/// mirror. The mirror refreshes at the top of `crate::RiggingSystems::Reconcile` while the apply
/// systems write `crate::RoleState` a full set later and can move one role
/// `Applying → Waiting → Applying` inside a single frame; a mirror-derived event would arrive one
/// frame late and collapse both transitions into one, leaving a consumer unable to count attempts
/// from events.
#[derive(Debug, EntityEvent, Reflect)]
pub struct RoleStateChanged {
    /// Binding entity for the role whose state moved.
    #[event_target]
    pub binding: Entity,
    /// Application role whose state moved, carried so an observer does not have to read the
    /// entity's components back to learn which role this is.
    pub role:    RoleKey,
    /// Lifecycle state the role moved *to*.
    pub state:   RoleState,
}

/// The retention rule applied when this role's device departs was re-authored.
///
/// Exists because the once-per-change rule is derived from the mirrored component set and
/// `crate::RecoveryPolicy` is in it; without this event the derivation would have a hole. A user
/// interface that shows what happens to a role on unplug reads it to stay current when application
/// code re-registers the binding with a different policy.
#[derive(Debug, EntityEvent, Reflect)]
pub struct RecoveryPolicyChanged {
    /// Binding entity for the role whose retention rule moved.
    #[event_target]
    pub binding:  Entity,
    /// Application role whose retention rule moved.
    pub role:     RoleKey,
    /// Retention rule the role moved *to*.
    pub recovery: RecoveryPolicy,
}

/// Application request to re-apply a role's saved configuration now.
///
/// This is what clears the `crate::WaitingWork::ApplicationRequestOwed` that a departure recorded,
/// and it is the kernel's replacement for clerestory's `RestoreWindow`. It is a *request from* the
/// application, not a report to it: the kernel observes it and answers according to the role's
/// `crate::RecoveryPolicy`.
///
/// It clears the owed request only for `crate::RecoveryPolicy::ReapplyOnRequest`. For
/// `crate::RecoveryPolicy::Retain` the kernel refuses — that policy promises the kernel remembers
/// and reports but never touches the device, and honouring a request here would break the promise
/// through the front door. For `crate::RecoveryPolicy::Forget` it refuses because the saved value
/// was already dropped at the departure and there is nothing left to re-apply.
#[derive(Debug, EntityEvent, Reflect)]
pub struct ReapplyConfiguration {
    /// Binding entity for the role whose saved configuration should be re-applied.
    #[event_target]
    pub binding: Entity,
}

/// One device stopped being usable, and which of the two ways it stopped by.
///
/// Global rather than entity-targeted because one of the two causes despawns the device entity in
/// the same frame, leaving an entity-addressed event with nowhere to land — and a consumer of that
/// cause has no entity left to read the key back from, which is why the durable
/// `crate::DeviceKey` travels in the payload.
///
/// The cause travels too rather than being discarded: both causes make every
/// `crate::RecoveryPolicy::ReapplyOnReturn` role owe its restoration, but only
/// `crate::DeviceDeparture::KeyLeftTheSet` retires the handle and despawns the entity. A consumer
/// that must tell "unplugged" from "still enumerated but not present" can do it from this payload
/// alone.
#[derive(Debug, Event, Reflect)]
pub struct DeviceDeparted {
    /// Durable name of the unit that left service.
    pub key:       DeviceKey,
    /// Which of the two departures this was.
    pub departure: DeviceDeparture,
}

/// Application request to retire a role and stop everything the kernel is doing for it.
///
/// Global rather than entity-targeted so an application can retire a role it never saw a binding
/// entity for — a role registered and retired inside one frame has no entity yet. Replaces
/// clerestory's `CancelWindowRecovery`.
#[derive(Debug, Event, Reflect)]
pub struct RetireRole {
    /// Application role to retire.
    pub role: RoleKey,
}

/// A registered role has no live device behind its endpoint and is waiting for one.
///
/// Global for the reason the interval itself exists: during it there is no device entity to address
/// and the binding entity may not have been spawned yet. Mirrors clerestory's
/// `WindowRecoveryPending`, and is what a user interface shows a "waiting for display" state from.
#[derive(Debug, Event, Reflect)]
pub struct RoleAwaiting {
    /// Application role with no live device behind its endpoint.
    pub role: RoleKey,
}

/// A registered role's endpoint resolved to a live device again.
///
/// The closing edge of `RoleAwaiting`, and global for the same reason: it is the transition out of
/// the interval where no entity could carry it. Mirrors clerestory's `WindowRecoveryAvailable`.
#[derive(Debug, Event, Reflect)]
pub struct RoleAvailable {
    /// Application role whose endpoint now resolves to a live device.
    pub role: RoleKey,
}

/// The kernel reached a different conclusion about whether an authored inventory key is connected.
///
/// Global because an authored key with no live unit behind it has no device entity: this is exactly
/// the event that lets a user interface list a configured-but-absent camera without inventing a
/// placeholder entity for it. Once a live unit is identified, the device-targeted events carry its
/// detailed presence, claim, and verdict transitions instead.
#[derive(Debug, Event, Reflect)]
pub struct ConfiguredDeviceConnectionChanged {
    /// Authored inventory key whose conclusion moved.
    pub key:        DeviceKey,
    /// Conclusion the key moved *to*.
    pub connection: ConfiguredDeviceConnection,
}

/// A reporter named a device under a `crate::SchemeName` no `RiggingAppExt` call registered.
///
/// The record is rejected at the ingest boundary and produces no device, no mirrored component, and
/// therefore no other event — so without this one a typo in a reporter's scheme name is completely
/// silent, and a reporter author debugging a device that never appears has nothing to look at.
/// Fires once per scheme, on the first record rejected under it; the scheme also stays readable
/// from `crate::Devices::unregistered_schemes`.
#[derive(Debug, Event, Reflect)]
pub struct UnregisteredSchemeReported {
    /// Identity scheme the record named, which no registration matched.
    pub scheme: SchemeName,
}

/// One reporter's running discovery job reported movement, and where that leaves its batch.
///
/// Global because a discovery run belongs to a reporter, not to any device: the run is what
/// decides which devices exist, so at the moment it is running there may be no entity for it to
/// address. Suppressed until the run has been going for `crate::DiscoveryLimits::progress_after`,
/// so a scan that finishes quickly produces no progress traffic and an application does not flash a
/// spinner for a run that was over before a human could read it.
///
/// The reporter's own report and the batch counts ride the same event because they are read from
/// one recorded transition: splitting them into two events made a consumer correlate two callbacks
/// that could never arrive apart, and left the aggregate free to disagree with the report that
/// produced it. A progress indicator reads the four counts, since one reporter's `Measured` count
/// says nothing about whether the application can proceed; a per-reporter view reads `reporter` and
/// `progress`. Neither needs a second observer.
#[derive(Debug, Event, Reflect)]
pub struct DiscoveryProgressChanged {
    /// Batch the run belongs to, shared by every reporter that became due in the same pass.
    pub batch:     DiscoveryBatchId,
    /// Reporter whose own job reported this.
    pub reporter:  ReporterId,
    /// What the job reported, including the explicitly uncountable case.
    pub progress:  DiscoveryProgress,
    /// Reporters in this batch that have finished, whether they succeeded or failed.
    pub completed: usize,
    /// Reporters the batch queued in the first place.
    pub total:     usize,
    /// Reporters in this batch whose job is enumerating hardware right now.
    pub running:   usize,
    /// Reporters in this batch still waiting for a job slot.
    pub queued:    usize,
}

/// One reporter's discovery run reached a terminal outcome and the kernel accepted it.
///
/// Global for the same reason as `DiscoveryProgressChanged`, and carrying
/// `crate::CompletedDiscoveryOutcome` rather than the retained
/// `crate::LastDiscoveryOutcome`: a run that just ended cannot be in the never-completed state, and
/// a consumer should not have to write an arm for a case this event can never carry.
#[derive(Debug, Event, Reflect)]
pub struct DiscoveryFinished {
    /// Batch that supplied the run.
    pub batch:    DiscoveryBatchId,
    /// Reporter whose run ended.
    pub reporter: ReporterId,
    /// How it ended, and how long it took.
    pub outcome:  CompletedDiscoveryOutcome,
}

/// The required-before-ready startup gate moved.
///
/// Global because it is a statement about the process rather than about any one device, and it is
/// the edge form of `crate::DiscoveryStatus::startup`: reading that field tells a system what the
/// gate is right now, while an application that wants to show "waiting for displays", "displays
/// failed", or "ready" needs the transition itself.
#[derive(Debug, Event, Reflect)]
pub struct StartupDiscoveryChanged {
    /// Gate state moved *to*. The state moved from is still on
    /// `crate::DiscoveryStatus::startup` until this event is delivered, so carrying it here would
    /// let the two disagree.
    pub state: StartupDiscoveryState,
}
