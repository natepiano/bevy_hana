//! Application-observable edges emitted by the rigging kernel.
//!
//! A live attempt ending is queued at the authoritative lifecycle write and published after its
//! `LiveRoleChange::Status` edge. A displaced ending is published after replacement cleanup and
//! successor status publication. A retired ending is published after driver cleanup, role-entity
//! despawn, and `RetiredRoleChange::Retired`.
//!
//! `DiscoveryFinished` remains the discovery journal's public completion and deferral surface.

use bevy::ecs::event::EntityEvent;
use bevy::ecs::event::Event;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::reflect::ReflectSerialize;
use serde::Serialize;

use crate::AttemptEndingView;
use crate::AttemptRef;
use crate::CompletedDiscoveryOutcome;
use crate::DeviceEndpoint;
use crate::DeviceKey;
use crate::DiscoveryBatchId;
use crate::DiscoveryProgress;
use crate::IdentityVerdict;
use crate::KeyAvailability;
use crate::ReporterId;
use crate::RoleKey;
use crate::RoleStatusView;
use crate::StartupDiscoveryState;

/// Readable role status before one published change.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(opaque)]
#[reflect(Serialize)]
#[serde(transparent)]
pub struct RoleStatusBeforeChange(Box<RoleStatusView>);

impl RoleStatusBeforeChange {
    pub(crate) fn new(status: RoleStatusView) -> Self { Self(Box::new(status)) }

    /// Return the status before the change.
    #[must_use]
    pub fn view(&self) -> &RoleStatusView { &self.0 }
}

/// Readable role status after one published change.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(opaque)]
#[reflect(Serialize)]
#[serde(transparent)]
pub struct RoleStatusAfterChange(Box<RoleStatusView>);

impl RoleStatusAfterChange {
    pub(crate) fn new(status: RoleStatusView) -> Self { Self(Box::new(status)) }

    /// Return the status after the change.
    #[must_use]
    pub fn view(&self) -> &RoleStatusView { &self.0 }
}

/// One BRP-readable status transition for a role whose entity remains live.
#[derive(Debug, EntityEvent, Reflect)]
pub struct LiveRoleChanged {
    /// Binding entity whose status changed.
    #[event_target]
    pub binding: Entity,
    /// Stable authored role whose status changed.
    pub role:    RoleKey,
    /// Typed status transition.
    pub change:  LiveRoleChange,
}

/// State change published for a live role.
#[derive(Debug, Reflect)]
pub enum LiveRoleChange {
    /// The complete readable status changed.
    Status {
        /// Status before the authoritative write.
        from: RoleStatusBeforeChange,
        /// Status after the authoritative write.
        to:   RoleStatusAfterChange,
    },
    /// One attempt reached a terminal outcome for this live registration.
    AttemptEnded {
        /// Process-local correlation value for the attempt that ended.
        attempt: AttemptRef,
        /// Data-only terminal outcome accepted by the kernel.
        ending:  AttemptEndingView,
    },
}

/// One global transition for a role whose live entity no longer exists.
#[derive(Debug, Event, Reflect)]
pub struct RetiredRoleChanged {
    /// Stable authored role that was retired.
    pub role:     RoleKey,
    /// Durable endpoint released by retirement.
    pub endpoint: crate::DeviceEndpoint,
    /// Typed retirement transition.
    pub change:   RetiredRoleChange,
}

/// State change published after a role entity is retired.
#[derive(Debug, Reflect)]
pub enum RetiredRoleChange {
    /// The role left the registered binding set.
    Retired,
}

/// Binding-set state accompanying a global attempt ending whose role entity is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum EndedRegistrationLifetime {
    /// A replacement installed a successor under the same durable role.
    Displaced,
    /// The durable role left the binding set.
    Retired,
    /// The role entity is gone, but the registration could not be retired.
    ///
    /// When consumers receive this variant, the role is still present in [`crate::Bindings`] and
    /// no successor registration exists for it.
    RetirementBlocked,
}

/// One attempt ended without a live role entity available for targeted publication.
///
/// Global publication preserves the registration's durable facts without targeting a successor
/// entity or a despawned role entity.
#[derive(Debug, Event, Reflect)]
pub struct RegistrationAttemptEnded {
    /// Stable authored role the ended registration served.
    pub role:     RoleKey,
    /// Durable endpoint authorized by the ended registration.
    pub endpoint: DeviceEndpoint,
    /// Process-local correlation value for the attempt that ended.
    pub attempt:  AttemptRef,
    /// Data-only terminal outcome accepted by the kernel.
    pub ending:   AttemptEndingView,
    /// Binding-set state accompanying the global ending.
    pub lifetime: EndedRegistrationLifetime,
}

/// One retained device fact changed on an availability edge.
#[derive(Debug, Event, Reflect)]
pub enum DeviceChange {
    /// The kernel published a different availability conclusion for one durable key.
    Availability {
        /// Durable key whose availability changed.
        key:  DeviceKey,
        /// Availability before the transition.
        from: KeyAvailability,
        /// Availability after the transition.
        to:   KeyAvailability,
    },
}

/// A durable key entered the reconciled device set and now has a device entity behind it.
///
/// This is what lets an integration say "*my* Stream Deck came back" by observing one entity
/// instead of writing a global match arm over every device kind the process reports. It fires once
/// per spawn; a unit that goes absent without its key leaving the set keeps its entity, so a second
/// `DeviceArrived` for the same entity never happens.
#[derive(Debug, EntityEvent, Reflect)]
pub struct DeviceArrived {
    /// Device entity the projection just spawned for this key.
    #[event_target]
    pub device: Entity,
    /// Durable name of the unit, carried so an observer can match an authored inventory entry
    /// without reading the entity back.
    pub key:    DeviceKey,
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
    /// names the register entry for later reads from `crate::IdentityDecisions`.
    pub candidate: DeviceKey,
}

/// Application request to re-apply a role's saved configuration now.
///
/// This is what clears the `crate::WaitingWork::ReapplyRequestOwed` that a departure or a
/// report-only session loss recorded, and it is the kernel's replacement for clerestory's
/// `RestoreWindow`. It is a *request from* the application, not a report to it.
///
/// A role owing `crate::WaitingWork::RegistrationOwed` is the one refusal: its
/// `crate::RecoveryPolicy::Forget` dropped the saved value at the departure, so there is nothing to
/// re-apply and only registering a binding with a fresh configuration restarts it.
/// `crate::Bindings::waiting_work` identifies which application action clears the hold.
#[derive(Debug, EntityEvent, Reflect)]
pub struct ReapplyConfiguration {
    /// Binding entity for the role whose saved configuration should be re-applied.
    #[event_target]
    pub binding: Entity,
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

/// One reporter's running discovery job reported movement, and where that leaves its batch.
///
/// Global because a discovery run belongs to a reporter, not to any device: the run is what
/// decides which devices exist, so at the moment it is running there may be no entity for it to
/// address. Suppressed until the run has been going for `crate::DiscoveryLimits::progress_after`,
/// so a scan that finishes quickly produces no progress traffic and an application does not flash a
/// spinner for a run that was over before a human could read it.
///
/// The reporter's own report and the batch counts travel on the same event because they are read
/// from one recorded transition: splitting them into two events would make a consumer correlate two
/// callbacks that never arrive apart, and would let the aggregate disagree with the report that
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
    /// Reporters in this batch whose terminal outcome the kernel accepted.
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
