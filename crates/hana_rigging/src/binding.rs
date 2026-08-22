use std::collections::HashMap;
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::time::Duration;

use bevy::ecs::entity::Entity;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::reflect::ReflectResource;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::With;
use thiserror::Error;

use crate::ApplyPermit;
use crate::AttemptId;
use crate::AttemptOutcome;
use crate::BindingRetired;
use crate::CaptureOutcome;
use crate::DeviceEndpoint;
use crate::DeviceKey;
use crate::DeviceRevisionLookup;
use crate::DriverId;
use crate::LastKnownGoodConfiguration;
use crate::OnAbort;
use crate::OnSessionLoss;
use crate::RecoveryPolicy;
use crate::RetryOn;
use crate::RiggingLimits;
use crate::RoleKey;
use crate::RoleState;
use crate::attempt::RetryGate;
use crate::reconcile::FrameClockReading;

const CONSECUTIVE_FAILURE_LIMIT: u32 = 3;
const DEFAULT_PENDING_TRANSITION_CAPACITY: usize = 4_096;

/// One authored role binding, including its durable endpoint and driver-specific configuration.
///
/// A `Binding` keeps the application role separate from the device entity that may currently
/// represent `endpoint.device`. This lets a window, camera slot, or panel key retain its authored
/// configuration while the physical unit is absent, without treating a process-local `DeviceId`
/// as durable identity.
#[derive(Reflect)]
pub struct Binding {
    /// Application role that remains stable while devices leave and return.
    pub role:            RoleKey,
    /// Durable device key and provider-defined part that this role exclusively owns in v1.
    pub endpoint:        DeviceEndpoint,
    /// Registered endpoint driver that receives this role's erased configuration.
    pub driver:          DriverId,
    /// Retention rule applied when the device supplying this endpoint departs.
    pub recovery:        RecoveryPolicy,
    /// Retry rule applied after an endpoint driver reports a recoverable failure.
    pub retry:           RetryOn,
    /// Response selected when an in-flight operation is abandoned by a new device report.
    pub on_abort:        OnAbort,
    /// Response selected when a still-present endpoint loses its local session.
    pub on_loss:         OnSessionLoss,
    /// Current role lifecycle state; `Bindings` is its sole live writer after registration.
    pub state:           RoleState,
    /// Authored driver target that describes what the application wants to reach.
    pub requested:       RequestedConfiguration,
    /// Driver value a safe readback most recently proved was on this endpoint.
    pub last_known_good: LastKnownGoodConfiguration,
    /// How long an attempt for this role may run before the kernel abandons it.
    ///
    /// Authored per binding because one process drives endpoints with genuinely different costs: a
    /// window move lands in milliseconds while opening a screen-capture stream can take seconds,
    /// and a single process-wide bound either abandons the capture or lets the window hang.
    /// The default keeps the process-wide value, so a binding that has no reason to differ
    /// says nothing.
    pub apply_deadline:  ApplyDeadline,
}

/// How long one role's attempts may run, and whether the binding chose that itself.
///
/// A named enum rather than an optional `std::time::Duration` because the two cases lead to
/// different behaviour when `crate::RiggingLimits::apply_deadline` is later retuned: a
/// `Self::ProcessDefault` binding follows the new value and a `Self::Authored` one deliberately
/// does not.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Reflect)]
pub enum ApplyDeadline {
    /// Use `crate::RiggingLimits::apply_deadline`, the bound every role shares.
    ///
    /// The default, so that adding this field asked nothing of a binding whose endpoint has no
    /// reason to be timed differently from the rest of the process.
    #[default]
    ProcessDefault,
    /// Use this role's own bound instead of the process-wide one.
    Authored(Duration),
}

impl ApplyDeadline {
    /// Resolve the authored choice against the process-wide bound.
    ///
    /// The result names which of the two supplied the value rather than returning a bare duration,
    /// so a caller reading a stamped attempt back can tell a role that chose five seconds from one
    /// that inherited five seconds from the process.
    #[must_use]
    pub(crate) const fn resolve(self, rigging_limits: &RiggingLimits) -> ApplyDeadlineLookup {
        match self {
            Self::ProcessDefault => {
                ApplyDeadlineLookup::ProcessDefault(rigging_limits.apply_deadline)
            },
            Self::Authored(apply_deadline) => ApplyDeadlineLookup::Authored(apply_deadline),
        }
    }
}

/// Which bound an attempt was stamped with, and where it came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
pub(crate) enum ApplyDeadlineLookup {
    /// The binding authored nothing, so the attempt carries the process-wide bound.
    ProcessDefault(Duration),
    /// The binding authored its own bound, which a later change to
    /// `crate::RiggingLimits::apply_deadline` will not move.
    Authored(Duration),
}

impl ApplyDeadlineLookup {
    /// The bound itself, once the caller no longer needs to know which side supplied it.
    #[must_use]
    pub(crate) const fn duration(self) -> Duration {
        match self {
            Self::ProcessDefault(apply_deadline) | Self::Authored(apply_deadline) => apply_deadline,
        }
    }
}

/// Authored driver configuration held without exposing the concrete configuration type to the
/// kernel.
///
/// The concrete configuration remains owned by the endpoint driver. `RequestedConfiguration`
/// exists because a display placement and a camera format can share a `DeviceKey` while requiring
/// unrelated driver types and routing rules.
#[derive(Reflect)]
pub struct RequestedConfiguration(
    #[reflect(ignore, default = "default_erased_configuration")] Box<dyn Reflect>,
);

impl RequestedConfiguration {
    /// Erase one driver-specific value while retaining it as authored role intent.
    #[must_use]
    pub fn new(configuration: impl Reflect) -> Self { Self(Box::new(configuration)) }

    fn as_reflect(&self) -> &dyn Reflect { self.0.as_ref() }
}

fn default_erased_configuration() -> Box<dyn Reflect> { Box::new(()) }

/// Configuration currently available to an offline UI or authoring workflow.
///
/// A proven value takes precedence because it describes the endpoint state a safe readback
/// observed. When no readback has succeeded, the authored request remains useful for presenting
/// the role's intended value without fabricating endpoint evidence.
pub enum AvailableConfiguration<'a> {
    /// A safe readback established this value on the endpoint.
    LastKnownGood(&'a dyn Reflect),
    /// No readback established a value, so this is the authored target instead.
    Requested(&'a dyn Reflect),
}

/// Identity of one installed binding, minted by `Bindings` each time a role's binding is
/// registered, replaced, or readdressed.
///
/// A `crate::RoleKey` is the role's durable name and survives replacement; this value does not.
/// Each attempt is stamped with the generation that dispatched it, so an attempt ending can be
/// told apart from the binding currently installed under the same role name: an ending whose
/// generation is not the current one belongs to a superseded binding and may not touch the
/// current binding's retry gates or failure counts.
///
/// Reflection sees the counter value opaquely for the same reason as `crate::AttemptId`: a
/// dynamic tuple struct must not be able to fabricate a generation the kernel never minted.
/// Minting starts at 1 by the same convention, so a defaulted value names no installed binding.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Reflect)]
#[reflect(opaque)]
pub struct BindingGeneration(u64);

/// Role records, endpoint ownership, and bounded lifecycle handoff owned by the kernel.
///
/// `Bindings` retains authored intent even when no live device entity exists. Its private reverse
/// indexes make duplicate endpoint ownership unavailable through checked registration methods,
/// while `PendingBindingTransitions` retains only the next frame's lifecycle work.
#[derive(Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct Bindings {
    #[reflect(ignore, default = "default_bindings_by_role")]
    by_role:                   HashMap<RoleKey, Binding>,
    #[reflect(ignore, default = "default_owner_by_endpoint")]
    owner_by_endpoint:         HashMap<DeviceEndpoint, RoleKey>,
    #[reflect(ignore, default = "default_roles_by_device")]
    roles_by_device:           HashMap<DeviceKey, Vec<RoleKey>>,
    #[reflect(ignore, default = "default_configuration_readability")]
    configuration_readability: HashMap<RoleKey, ConfigurationReadability>,
    #[reflect(ignore, default = "default_waiting_work")]
    waiting_work:              HashMap<RoleKey, WaitingWork>,
    /// Which configuration the in-flight apply on each role draws from, so the attempt that
    /// settles a `WaitingWork::RestorationOwed` debt is the restoration and nothing else.
    ///
    /// A role can owe a restoration while an ordinary apply is in flight: the debt is recorded
    /// from `crate::RecoveryPolicy` and `LastKnownGoodConfiguration` against any role whose device
    /// departed, including one that already minted a requested apply. Reading "the outcome reached
    /// `RoleState::Ready`" as "the restoration ran" would clear a debt nothing paid and leave the
    /// returning device holding the wrong configuration.
    #[reflect(ignore, default = "default_applying_source")]
    applying_source:           HashMap<RoleKey, ApplyConfigurationSource>,
    /// Globally unique attempt that established each role's current Ready session.
    ///
    /// Device identity and revision can remain unchanged while a replacement session succeeds.
    /// Retaining the successful `AttemptId` lets a delayed loss from the replaced session be
    /// refused without depending on frame-local binding-transition history.
    #[reflect(ignore, default = "default_establishing_attempts")]
    establishing_attempts:     HashMap<RoleKey, AttemptId>,
    /// Consecutive failed attempts per role, reset only by a successful attempt.
    ///
    /// Two failures followed by a third is three, not a fresh start: the count follows the role's
    /// run of failures, which is why it is kernel state and not something a driver reporting
    /// arrival evidence could keep.
    #[reflect(ignore, default = "default_failure_counts")]
    attempt_failures:          HashMap<RoleKey, u32>,
    /// Consecutive failed safe readbacks per role, reset by the first successful one.
    ///
    /// Without it a driver whose readback is permanently broken is dispatched at frame rate
    /// forever, because a failed readback leaves `LastKnownGoodConfiguration::NotEstablished` and
    /// re-qualifies the role on the next pass.
    #[reflect(ignore, default = "default_failure_counts")]
    capture_failures:          HashMap<RoleKey, u32>,
    /// What each role that failed is waiting for before another attempt may be dispatched.
    #[reflect(ignore, default = "default_retry_gates")]
    retry_gates:               HashMap<RoleKey, RetryGate>,
    /// How each stopped role's endpoint last read, so a reacquisition can be told from a device
    /// that never left.
    ///
    /// A role the kernel stopped after three failures only gets another attempt once its device
    /// has actually gone and come back; without the previous reading, a device that stayed present
    /// the whole time would look like a return on every frame and the stop would mean nothing.
    #[reflect(ignore, default = "default_stopped_role_endpoints")]
    stopped_role_endpoints:    HashMap<RoleKey, EndpointAvailability>,
    /// The generation of each role's currently installed binding. Written beside every
    /// `by_role` insert and removed on retirement, so an equality check against an attempt's
    /// stamped generation answers "was this attempt dispatched for the binding standing now?"
    /// without consulting the frame-delayed transition pipeline.
    #[reflect(ignore, default = "default_generation_by_role")]
    generation_by_role:        HashMap<RoleKey, BindingGeneration>,
    #[reflect(ignore, default = "default_generation_counter")]
    next_generation:           u64,
    #[reflect(ignore, default = "PendingBindingTransitions::default")]
    pending_transitions:       PendingBindingTransitions,
    #[reflect(ignore, default = "default_transition_sequence")]
    next_transition_sequence:  u64,
}

fn default_bindings_by_role() -> HashMap<RoleKey, Binding> { HashMap::new() }

fn default_owner_by_endpoint() -> HashMap<DeviceEndpoint, RoleKey> { HashMap::new() }

fn default_roles_by_device() -> HashMap<DeviceKey, Vec<RoleKey>> { HashMap::new() }

fn default_establishing_attempts() -> HashMap<RoleKey, AttemptId> { HashMap::new() }

fn default_configuration_readability() -> HashMap<RoleKey, ConfigurationReadability> {
    HashMap::new()
}

fn default_waiting_work() -> HashMap<RoleKey, WaitingWork> { HashMap::new() }

fn default_applying_source() -> HashMap<RoleKey, ApplyConfigurationSource> { HashMap::new() }

fn default_failure_counts() -> HashMap<RoleKey, u32> { HashMap::new() }

fn default_retry_gates() -> HashMap<RoleKey, RetryGate> { HashMap::new() }

fn default_generation_by_role() -> HashMap<RoleKey, BindingGeneration> { HashMap::new() }

const fn default_generation_counter() -> u64 { 0 }

fn default_stopped_role_endpoints() -> HashMap<RoleKey, EndpointAvailability> { HashMap::new() }

/// What a waiting role is owed, distinct from why it is waiting.
///
/// Stored rather than derived: the attempt systems select requested intent versus a restore from
/// this value, and configuration capture is suppressed on `Self::RestorationOwed` instead of being
/// re-derived from recovery policy and attempt history at each of those call sites, where the two
/// derivations would eventually disagree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum WaitingWork {
    /// Nothing is owed. The role is waiting for usable, authorized hardware.
    #[default]
    Nothing,
    /// A last-known-good restoration is owed and runs as soon as the role is authorized. Capture is
    /// suppressed until it completes, because reading a value back before the owed one has been
    /// reapplied would record the endpoint's current state as the last one known to work.
    RestorationOwed,
    /// The device departed and this role's `crate::RecoveryPolicy` does not reapply on return, so
    /// the kernel starts nothing until application code acts. An established-session loss under
    /// `crate::OnSessionLoss::ReportOnly` uses the same hold: the device is still present, but the
    /// application explicitly asked the kernel not to open a replacement.
    ///
    /// Reconciliation records this on departure for `crate::RecoveryPolicy::{Retain,
    /// ReapplyOnRequest, Forget}`. It is what makes those three differ from `ReapplyOnReturn`:
    /// without it a departed role returns to `Nothing`, reaches `WaitingRole::Hardware`, and
    /// has its authored request dispatched automatically — which is the one thing `Retain`
    /// promises never happens.
    ///
    /// A role's *first* apply is unaffected: a newly registered binding has no recorded work, so it
    /// answers `Nothing` and reaches `WaitingRole::Hardware` as before. This state is recorded
    /// only after a departure or a report-only session loss.
    ApplicationRequestOwed,
}

/// Whether the kernel may still ask a driver to read one role's endpoint configuration back.
///
/// A named state rather than membership in a set of unreadable roles: at every lookup the reader
/// learns what the absent case means. A display API that reports geometry without exposing the
/// current window arrangement declines permanently, and re-asking it every reconcile pass would
/// call a driver forever for an answer that cannot change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub(crate) enum ConfigurationReadability {
    /// No driver has declined a safe readback for this role, so capture stays eligible.
    #[default]
    Readable,
    /// A driver reported `CaptureOutcome::NotReadable`, which is permanent for the endpoint. This
    /// is the retained reason later captures are suppressed.
    PermanentlyUnreadable,
}

const fn default_transition_sequence() -> u64 { 0 }

/// Monotonic order attached to lifecycle handoff entries.
///
/// This sequence orders changes submitted before a frame drain; it is not a historical log and
/// consumers cannot use it to recover work after the bounded handoff releases an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Reflect)]
#[reflect(opaque)]
pub(crate) struct BindingTransitionSequence(u64);

/// One accepted binding lifecycle change awaiting the next frame's internal processing.
#[derive(Debug, PartialEq, Eq, Reflect)]
pub(crate) enum BindingTransition {
    /// A newly registered role needs a binding entity during the next lifecycle stage.
    Registered {
        /// Ordering number for this accepted operation.
        sequence: BindingTransitionSequence,
        /// Authored role whose binding was registered.
        role:     RoleKey,
    },
    /// A replacement displaced a prior binding whose in-flight work is handled later.
    Replaced {
        /// Ordering number for this accepted operation.
        sequence: BindingTransitionSequence,
        /// Authored role whose binding was replaced.
        role:     RoleKey,
    },
    /// A retired role needs entity cleanup and any later attempt-abort processing.
    Retired {
        /// Ordering number for this accepted operation.
        sequence: BindingTransitionSequence,
        /// Authored role whose binding was retired.
        role:     RoleKey,
        /// Exact address the retired role had authorized before its indexes were removed.
        endpoint: DeviceEndpoint,
    },
}

struct PendingBindingTransitions {
    capacity: NonZeroUsize,
    queue:    VecDeque<BindingTransition>,
}

impl Default for PendingBindingTransitions {
    fn default() -> Self {
        Self {
            capacity: NonZeroUsize::new(DEFAULT_PENDING_TRANSITION_CAPACITY)
                .unwrap_or(NonZeroUsize::MIN),
            queue:    VecDeque::new(),
        }
    }
}

impl PendingBindingTransitions {
    fn has_capacity(&self) -> bool { self.queue.len() < self.capacity.get() }

    fn push(&mut self, binding_transition: BindingTransition) {
        self.queue.push_back(binding_transition);
    }
}

impl Bindings {
    /// Register one authored role after confirming that no prior role owns its endpoint.
    ///
    /// # Errors
    ///
    /// Returns `BindingError` when the role already exists, another role owns the endpoint, or
    /// the bounded lifecycle handoff cannot retain this registration without dropping work.
    pub fn register(&mut self, mut binding: Binding) -> Result<(), BindingError> {
        if self.by_role.contains_key(&binding.role) {
            return Err(BindingError::RoleAlreadyBound { role: binding.role });
        }
        if let Some(owner) = self.owner_by_endpoint.get(&binding.endpoint) {
            return Err(BindingError::EndpointAlreadyOwned {
                endpoint: binding.endpoint,
                owner:    owner.clone(),
            });
        }
        let reserved_transition = self.reserve_transition()?;

        binding.state = RoleState::Waiting;
        let role = binding.role.clone();
        let endpoint = binding.endpoint.clone();
        let device_key = endpoint.device.clone();
        self.owner_by_endpoint.insert(endpoint, role.clone());
        self.roles_by_device
            .entry(device_key)
            .or_default()
            .push(role.clone());
        self.by_role.insert(role.clone(), binding);
        let generation = self.mint_generation();
        self.generation_by_role.insert(role.clone(), generation);
        self.enqueue(BindingTransitionKind::Registered, role, reserved_transition);

        Ok(())
    }

    /// Replace one binding only after proving its new endpoint is not owned by another role.
    ///
    /// The old endpoint remains owned until the proposed endpoint and transition handoff both
    /// pass their checks, so an error cannot leave a role unbound or corrupt either reverse index.
    ///
    /// # Errors
    ///
    /// Returns `BindingError` when no old binding exists, a different role owns the proposed
    /// endpoint, or the transition handoff has no space for this replacement.
    pub fn replace(&mut self, mut binding: Binding) -> Result<Binding, BindingError> {
        let old_binding =
            self.by_role
                .get(&binding.role)
                .ok_or_else(|| BindingError::RoleNotBound {
                    role: binding.role.clone(),
                })?;
        if let Some(owner) = self.owner_by_endpoint.get(&binding.endpoint)
            && owner != &binding.role
        {
            return Err(BindingError::EndpointAlreadyOwned {
                endpoint: binding.endpoint,
                owner:    owner.clone(),
            });
        }
        let reserved_transition = self.reserve_transition()?;

        binding.state = RoleState::Waiting;
        let role = binding.role.clone();
        let old_endpoint = old_binding.endpoint.clone();
        let new_endpoint = binding.endpoint.clone();
        let displaced = self
            .by_role
            .insert(role.clone(), binding)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        let generation = self.mint_generation();
        self.generation_by_role.insert(role.clone(), generation);

        if old_endpoint != new_endpoint {
            let new_device_key = new_endpoint.device.clone();
            self.owner_by_endpoint.remove(&old_endpoint);
            self.remove_role_from_device(&old_endpoint.device, &role);
            self.owner_by_endpoint.insert(new_endpoint, role.clone());
            self.roles_by_device
                .entry(new_device_key)
                .or_default()
                .push(role.clone());
        }
        self.configuration_readability.remove(&role);
        self.waiting_work.remove(&role);
        self.applying_source.remove(&role);
        self.establishing_attempts.remove(&role);
        self.attempt_failures.remove(&role);
        self.capture_failures.remove(&role);
        self.retry_gates.remove(&role);
        self.stopped_role_endpoints.remove(&role);
        self.enqueue(BindingTransitionKind::Replaced, role, reserved_transition);

        Ok(displaced)
    }

    /// Move one role's endpoint onto an adopted device key, keeping the endpoint's own address.
    ///
    /// The adoption path for `crate::IdentityDecisions`: a human has decided the unit that arrived
    /// into the departed one's attachment *is* the unit this role should address, and the durable
    /// key indexed in `Self::owner_by_endpoint` and `Self::roles_by_device` has to move with that
    /// decision. Rewriting only the saved key elsewhere would leave the role resolving
    /// `crate::DeviceResolution::NotResolved` for good.
    ///
    /// This applies `Self::replace`'s ownership rule without asking for a whole replacement
    /// `Binding`: the value is not `Clone`, so an adoption that had to hand one over could not keep
    /// the role's authored request and last-known-good configuration. Everything else follows
    /// `Self::replace` — the role goes back to `crate::RoleState::Waiting` and its per-role failure
    /// and readability history is dropped, because that history describes the unit the role is no
    /// longer addressing.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::EndpointAlreadyOwned` when another role already holds the adopted
    /// endpoint, `BindingError::RoleNotBound` when the role was retired, and
    /// `BindingError::PendingTransitionCapacityReached` when the transition handoff is full.
    pub(crate) fn readdress(
        &mut self,
        role: &RoleKey,
        device: DeviceKey,
    ) -> Result<(), BindingError> {
        let binding = self
            .by_role
            .get(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        let old_endpoint = binding.endpoint.clone();
        let new_endpoint = DeviceEndpoint {
            device,
            id: old_endpoint.id.clone(),
        };
        if let Some(owner) = self.owner_by_endpoint.get(&new_endpoint)
            && owner != role
        {
            return Err(BindingError::EndpointAlreadyOwned {
                endpoint: new_endpoint,
                owner:    owner.clone(),
            });
        }
        if old_endpoint == new_endpoint {
            return Ok(());
        }
        let reserved_transition = self.reserve_transition()?;

        let binding = self
            .by_role
            .get_mut(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        binding.endpoint = new_endpoint.clone();
        binding.state = RoleState::Waiting;
        let new_device_key = new_endpoint.device.clone();
        self.owner_by_endpoint.remove(&old_endpoint);
        self.remove_role_from_device(&old_endpoint.device, role);
        self.owner_by_endpoint.insert(new_endpoint, role.clone());
        self.roles_by_device
            .entry(new_device_key)
            .or_default()
            .push(role.clone());
        self.configuration_readability.remove(role);
        self.waiting_work.remove(role);
        self.applying_source.remove(role);
        self.establishing_attempts.remove(role);
        self.attempt_failures.remove(role);
        self.capture_failures.remove(role);
        self.retry_gates.remove(role);
        self.stopped_role_endpoints.remove(role);
        let generation = self.mint_generation();
        self.generation_by_role.insert(role.clone(), generation);
        self.enqueue(
            BindingTransitionKind::Replaced,
            role.clone(),
            reserved_transition,
        );

        Ok(())
    }

    /// Checks whether every role on `saved` can move to `adopted` as one operation.
    ///
    /// Camera clones use different endpoint parts on one physical device. Checking the
    /// complete device address prevents an application from moving one clone while a
    /// conflicting destination or a full lifecycle handoff leaves its siblings behind.
    /// This method changes no binding, index, lifecycle state, or transition sequence.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::EndpointAlreadyOwned` when a destination endpoint belongs to a
    /// role that is not moving, `BindingError::PendingTransitionCapacityReached` when all role
    /// replacements do not fit together, or `BindingError::TransitionSequenceExhausted` when
    /// the complete replacement set cannot receive unique transition identities.
    pub fn validate_device_readdress(
        &self,
        saved: &DeviceKey,
        adopted: &DeviceKey,
    ) -> Result<(), BindingError> {
        self.prepare_device_readdress(saved, adopted).map(|_| ())
    }

    /// Moves every role on `saved` to the corresponding endpoint on `adopted` atomically.
    ///
    /// Each role keeps its endpoint part, driver, requested configuration, and last-known-good
    /// configuration. The roles return to `RoleState::Waiting`, and their device-specific
    /// attempt, retry, and readability state is cleared just as it is for `Self::readdress`.
    /// A device with no roles is a successful no-op.
    ///
    /// # Errors
    ///
    /// Returns the same preflight errors as `Self::validate_device_readdress`. Every check and
    /// every transition reservation completes before the first binding or index is changed.
    pub fn readdress_device(
        &mut self,
        saved: &DeviceKey,
        adopted: DeviceKey,
    ) -> Result<(), BindingError> {
        let prepared = self.prepare_device_readdress(saved, &adopted)?;
        if prepared.is_empty() {
            return Ok(());
        }
        let roles = prepared
            .iter()
            .map(|readdress| readdress.role.clone())
            .collect::<Vec<_>>();
        for readdress in prepared {
            let Some(binding) = self.by_role.get_mut(&readdress.role) else {
                continue;
            };
            binding.endpoint = readdress.adopted_endpoint.clone();
            binding.state = RoleState::Waiting;
            self.owner_by_endpoint.remove(&readdress.saved_endpoint);
            self.owner_by_endpoint
                .insert(readdress.adopted_endpoint, readdress.role.clone());
            self.configuration_readability.remove(&readdress.role);
            self.waiting_work.remove(&readdress.role);
            self.applying_source.remove(&readdress.role);
            self.establishing_attempts.remove(&readdress.role);
            self.attempt_failures.remove(&readdress.role);
            self.capture_failures.remove(&readdress.role);
            self.retry_gates.remove(&readdress.role);
            self.stopped_role_endpoints.remove(&readdress.role);
            let generation = self.mint_generation();
            self.generation_by_role
                .insert(readdress.role.clone(), generation);
            self.enqueue(
                BindingTransitionKind::Replaced,
                readdress.role,
                readdress.reserved_transition,
            );
        }
        self.roles_by_device.remove(saved);
        self.roles_by_device
            .entry(adopted)
            .or_default()
            .extend(roles);
        Ok(())
    }

    fn prepare_device_readdress(
        &self,
        saved: &DeviceKey,
        adopted: &DeviceKey,
    ) -> Result<Vec<PreparedDeviceReaddress>, BindingError> {
        if saved == adopted {
            return Ok(Vec::new());
        }
        let roles = self.roles_by_device.get(saved).cloned().unwrap_or_default();
        for role in &roles {
            let binding = self
                .by_role
                .get(role)
                .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
            let adopted_endpoint = DeviceEndpoint {
                device: adopted.clone(),
                id:     binding.endpoint.id.clone(),
            };
            if let Some(owner) = self.owner_by_endpoint.get(&adopted_endpoint)
                && owner != role
            {
                return Err(BindingError::EndpointAlreadyOwned {
                    endpoint: adopted_endpoint,
                    owner:    owner.clone(),
                });
            }
        }
        let reservations = self.reserve_transitions(roles.len())?;
        roles
            .into_iter()
            .zip(reservations)
            .map(|(role, reserved_transition)| {
                let binding = self
                    .by_role
                    .get(&role)
                    .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
                Ok(PreparedDeviceReaddress {
                    adopted_endpoint: DeviceEndpoint {
                        device: adopted.clone(),
                        id:     binding.endpoint.id.clone(),
                    },
                    saved_endpoint: binding.endpoint.clone(),
                    role,
                    reserved_transition,
                })
            })
            .collect()
    }

    /// Report which other role, if any, already holds the endpoint an adoption would move `role`
    /// onto.
    ///
    /// `crate::IdentityDecisions` caches this answer on each standing question, because application
    /// code answering a question holds only that resource and an adoption that quietly took an
    /// endpoint from another role is the outcome the register must never produce. A role that is
    /// unbound, or that already owns the endpoint itself, reads as `EndpointOwner::Unowned`:
    /// neither is a conflict an operator has to resolve.
    pub(crate) fn candidate_endpoint_owner(
        &self,
        role: &RoleKey,
        candidate: &DeviceKey,
    ) -> EndpointOwner {
        let Some(binding) = self.by_role.get(role) else {
            return EndpointOwner::Unowned;
        };
        let candidate_endpoint = DeviceEndpoint {
            device: candidate.clone(),
            id:     binding.endpoint.id.clone(),
        };

        self.owner_by_endpoint
            .get(&candidate_endpoint)
            .filter(|owner| *owner != role)
            .map_or(EndpointOwner::Unowned, |owner| {
                EndpointOwner::OwnedBy(owner.clone())
            })
    }

    /// Retire an authored role and remove every ownership index entry that selected it.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::PendingTransitionCapacityReached` when this effective retirement
    /// cannot be retained for later lifecycle processing. Retiring a role that is already absent
    /// succeeds with `RetirementOutcome::AlreadyUnbound` and creates no transition.
    pub fn retire(&mut self, role: &RoleKey) -> Result<RetirementOutcome, BindingError> {
        if !self.by_role.contains_key(role) {
            return Ok(RetirementOutcome::AlreadyUnbound);
        }
        let reserved_transition = self.reserve_transition()?;

        let mut binding = self
            .by_role
            .remove(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        self.owner_by_endpoint.remove(&binding.endpoint);
        self.remove_role_from_device(&binding.endpoint.device, role);
        self.configuration_readability.remove(role);
        self.waiting_work.remove(role);
        self.applying_source.remove(role);
        self.establishing_attempts.remove(role);
        self.attempt_failures.remove(role);
        self.capture_failures.remove(role);
        self.retry_gates.remove(role);
        self.stopped_role_endpoints.remove(role);
        self.generation_by_role.remove(role);
        binding.state = RoleState::Retired;
        self.enqueue(
            BindingTransitionKind::Retired(binding.endpoint.clone()),
            role.clone(),
            reserved_transition,
        );

        Ok(RetirementOutcome::Retired(binding))
    }

    /// Read the generation of the binding currently installed under `role`.
    ///
    /// `None` means the role has no binding at all — retired or never registered. Callers
    /// compare the answer with an attempt's stamped `BindingGeneration` to decide whether the
    /// attempt was dispatched for the binding standing now.
    pub(crate) fn generation(&self, role: &RoleKey) -> Option<BindingGeneration> {
        self.generation_by_role.get(role).copied()
    }

    /// Issue the next binding generation from the process-lifetime counter.
    ///
    /// Never reused while the process runs, so an attempt stamped under a superseded binding can
    /// never collide with a later installation under the same role name.
    const fn mint_generation(&mut self) -> BindingGeneration {
        self.next_generation += 1;
        BindingGeneration(self.next_generation)
    }

    /// Borrow one binding without exposing a mutable path around its role lifecycle views.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::RoleNotBound` when the role has no retained authored binding.
    pub fn binding(&self, role: &RoleKey) -> Result<&Binding, BindingError> {
        self.by_role
            .get(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })
    }

    /// Iterate every retained role whose endpoint names `device_key`.
    ///
    /// Several roles can address different endpoints of one device, so callers receive every
    /// role instead of a convenient but unsafe first match.
    pub fn roles_for(&self, device_key: &DeviceKey) -> impl Iterator<Item = &RoleKey> {
        self.roles_by_device.get(device_key).into_iter().flatten()
    }

    /// Select the one lifecycle view whose methods are valid for this stored role state.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::RoleNotBound` when the requested application role has no binding.
    pub(crate) fn role_view(&mut self, role: &RoleKey) -> Result<RoleView<'_>, BindingError> {
        let configuration_readability = &mut self.configuration_readability;
        let applying_source = &mut self.applying_source;
        let establishing_attempts = &mut self.establishing_attempts;
        let waiting_work = self.waiting_work.get(role).copied().unwrap_or_default();
        let binding = self
            .by_role
            .get_mut(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;

        Ok(match binding.state {
            RoleState::Waiting => RoleView::Waiting(match waiting_work {
                WaitingWork::Nothing => WaitingRole::Hardware(RequestingRole {
                    binding,
                    applying_source,
                }),
                WaitingWork::RestorationOwed => WaitingRole::Restoration(RestoringRole {
                    binding,
                    applying_source,
                }),
                WaitingWork::ApplicationRequestOwed => WaitingRole::ApplicationRequest,
            }),
            RoleState::Ready => RoleView::Ready(ReadyRole {
                binding,
                configuration_readability,
                capture_failures: &mut self.capture_failures,
            }),
            RoleState::Applying(_) => RoleView::Applying(ApplyingRole {
                binding,
                waiting_work: &mut self.waiting_work,
                applying_source,
                establishing_attempts,
            }),
            RoleState::StoppedAfterRepeatedFailures => RoleView::StoppedAfterRepeatedFailures,
            RoleState::Retired => RoleView::Retired,
        })
    }

    /// Read what one role is owed while it waits.
    ///
    /// Answers `WaitingWork::Nothing` for a role nobody has recorded work against, including one
    /// that is not waiting at all: owing a restoration is something the kernel records, so an
    /// unrecorded role owes nothing.
    #[must_use]
    pub fn waiting_work(&self, role: &RoleKey) -> WaitingWork {
        self.waiting_work.get(role).copied().unwrap_or_default()
    }

    /// Read the globally unique attempt that established this role's current Ready session.
    pub(crate) fn establishing_attempt(&self, role: &RoleKey) -> EstablishingAttemptLookup {
        self.establishing_attempts.get(role).copied().map_or(
            EstablishingAttemptLookup::NotEstablished,
            EstablishingAttemptLookup::EstablishedBy,
        )
    }

    /// Return a role whose device departed to `RoleState::Waiting`.
    ///
    /// Without this the work `Self::set_waiting_work` records is unreachable: `WaitingWork` is only
    /// ever consulted through `RoleView::Waiting`, so a role left in `RoleState::Ready` after its
    /// unit left never reaches `WaitingRole::Restoration` or `WaitingRole::ApplicationRequest`, and
    /// every `crate::RecoveryPolicy` variant behaves identically — the departed unit's return
    /// applies nothing at all.
    ///
    /// Only `RoleState::Ready` moves, because it is the one state whose meaning the departure
    /// falsified: the role no longer has a present usable unit. `RoleState::Applying` is ended by
    /// the abort path, which writes `RoleState::Waiting` itself;
    /// `RoleState::StoppedAfterRepeatedFailures` is re-armed by
    /// `Self::observe_stopped_role_endpoint`, which needs the departure to stay visible for one
    /// more pass; and `RoleState::Retired` never reactivates.
    pub(crate) fn await_departed_device(&mut self, role: &RoleKey) {
        if let Some(binding) = self.by_role.get_mut(role)
            && binding.state == RoleState::Ready
        {
            binding.state = RoleState::Waiting;
            self.establishing_attempts.remove(role);
        }
    }

    /// Record what one role is owed while it waits.
    pub(crate) fn set_waiting_work(&mut self, role: &RoleKey, waiting_work: WaitingWork) {
        match waiting_work {
            WaitingWork::Nothing => {
                self.waiting_work.remove(role);
            },
            WaitingWork::RestorationOwed | WaitingWork::ApplicationRequestOwed => {
                self.waiting_work.insert(role.clone(), waiting_work);
            },
        }
    }

    /// Discard the configuration this role last applied successfully.
    ///
    /// A role whose `crate::RecoveryPolicy` is `crate::RecoveryPolicy::Forget` keeps no saved value
    /// across a departure, so the value is dropped at the departure rather than left to be read by
    /// a later restore.
    /// Clear a role's owed application request by turning it into the restoration it asked for.
    ///
    /// Only `crate::RecoveryPolicy::ReapplyOnRequest` reaches here; the caller enforces that,
    /// because the refusal for the other policies is a statement about the policy rather than about
    /// the binding. A role with no saved value falls back to `WaitingWork::Nothing`, which lets its
    /// authored request dispatch: the application asked for the endpoint to be driven, and the only
    /// thing left to drive it with is what the application authored.
    pub(crate) fn request_reapply(&mut self, role: &RoleKey) {
        let established = self.by_role.get(role).is_some_and(|binding| {
            matches!(
                binding.last_known_good,
                LastKnownGoodConfiguration::Known(_)
            )
        });
        let waiting_work = if established {
            WaitingWork::RestorationOwed
        } else {
            WaitingWork::Nothing
        };
        self.set_waiting_work(role, waiting_work);
    }

    /// Discard a role's captured configuration because its endpoint no longer holds that value.
    ///
    /// A driver's safe readback runs only while `LastKnownGoodConfiguration` is
    /// `NotEstablished`, so a configuration changed outside the kernel — a user dragging a
    /// window, an external tool retuning a device — stays stale forever unless the owning
    /// application reports the drift. Forgetting reopens the safe-capture opportunity; the
    /// capture pass itself re-checks every dispatch condition (role readiness, nothing owed,
    /// device presence), so calling this is always safe and at worst a no-op.
    pub fn forget_last_known_good(&mut self, role: &RoleKey) {
        if let Some(binding) = self.by_role.get_mut(role) {
            binding.last_known_good = LastKnownGoodConfiguration::NotEstablished;
        }
    }

    /// Record how one attempt ended and escalate or clear this role's run of failures.
    ///
    /// `AttemptOutcome::Aborted` is terminal: it never counts toward escalation, because an attempt
    /// the kernel abandoned lost its authorization rather than its device. It still closes a retry
    /// gate, and that gate is what makes "terminal" true — the abort systems, the poll, and the
    /// dispatch all run inside one `crate::RiggingSystems::Apply` chain, so a role left ungated
    /// would be restarted later in the very frame that abandoned it, against the conditions that
    /// just invalidated it. The gate is stamped with the device revision that invalidated the
    /// attempt, so under `crate::RetryOn::NewRevision` the change that caused the abort cannot also
    /// open the retry.
    ///
    /// A success clears the count outright — that is what "self-clears on recovery" means.
    pub(crate) fn record_attempt_ending(
        &mut self,
        role: &RoleKey,
        ended_generation: BindingGeneration,
        outcome: AttemptOutcome,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) {
        // An ending may only affect the binding generation that dispatched its attempt. A stale
        // ending — the role was replaced or retired while the attempt was in flight — must not
        // install a retry gate, count a failure, or clear a run against the binding standing now:
        // `Self::replace` deliberately clears that history, and an ending that landed afterwards
        // would silently re-pollute it.
        if self.generation(role) != Some(ended_generation) {
            return;
        }
        match outcome {
            AttemptOutcome::Succeeded | AttemptOutcome::Substituted => {
                self.attempt_failures.remove(role);
                self.retry_gates.remove(role);
                self.stopped_role_endpoints.remove(role);
            },
            AttemptOutcome::Aborted => {
                if let Some(binding) = self.by_role.get(role) {
                    let retry_gate = RetryGate::from_policy(binding.retry, device_revision, now);
                    self.retry_gates.insert(role.clone(), retry_gate);
                }
            },
            AttemptOutcome::Failed(_) => {
                let consecutive = self
                    .attempt_failures
                    .get(role)
                    .copied()
                    .unwrap_or_default()
                    .saturating_add(1);
                self.attempt_failures.insert(role.clone(), consecutive);
                if consecutive >= CONSECUTIVE_FAILURE_LIMIT {
                    self.retry_gates.remove(role);
                    if let Some(binding) = self.by_role.get_mut(role) {
                        binding.state = RoleState::StoppedAfterRepeatedFailures;
                    }
                } else if let Some(binding) = self.by_role.get(role) {
                    let retry_gate = RetryGate::from_policy(binding.retry, device_revision, now);
                    self.retry_gates.insert(role.clone(), retry_gate);
                }
            },
        }
    }

    /// Apply an established role's session-loss policy after the caller validates its exact
    /// process-local device handle and revision.
    ///
    /// `Recreate` uses the ordinary waiting path and installs the role's normal retry gate before
    /// apply selection can run. A readable last-known-good value is restored; otherwise the
    /// retained authored request is the only configuration available. `ReportOnly` leaves the
    /// role waiting on application action and installs no retry that could open a replacement.
    pub(crate) fn apply_session_loss(
        &mut self,
        role: &RoleKey,
        device_revision: crate::DeviceRevision,
        now: FrameClockReading,
    ) -> SessionLossApplication {
        let Some(binding) = self.by_role.get(role) else {
            return SessionLossApplication::BindingAbsent;
        };
        let on_loss = binding.on_loss;
        let retry = binding.retry;
        let has_last_known_good = matches!(
            binding.last_known_good,
            LastKnownGoodConfiguration::Known(_)
        );
        if let Some(binding) = self.by_role.get_mut(role) {
            binding.state = RoleState::Waiting;
        }
        self.applying_source.remove(role);
        self.establishing_attempts.remove(role);
        match on_loss {
            OnSessionLoss::Recreate => {
                self.set_waiting_work(
                    role,
                    if has_last_known_good {
                        WaitingWork::RestorationOwed
                    } else {
                        WaitingWork::Nothing
                    },
                );
                self.retry_gates.insert(
                    role.clone(),
                    RetryGate::from_policy(
                        retry,
                        DeviceRevisionLookup::Retained(device_revision),
                        now,
                    ),
                );
            },
            OnSessionLoss::ReportOnly => {
                self.set_waiting_work(role, WaitingWork::ApplicationRequestOwed);
                self.retry_gates.remove(role);
            },
        }
        SessionLossApplication::Applied(on_loss)
    }

    /// Pace the next dispatch for a role whose apply never reached a working driver.
    ///
    /// An unregistered driver and a configuration-contract mismatch are not attempt failures — no
    /// attempt ran, nothing touched the device — so they neither escalate the role nor clear its
    /// run. They still have to be paced: the role stays `crate::RoleState::Waiting`, so without a
    /// gate the kernel would re-dispatch and be re-refused on every frame for the life of the
    /// binding, which is the same unbounded retry `crate::RetryOn` exists to stop.
    pub(crate) fn record_dispatch_refused(
        &mut self,
        role: &RoleKey,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) {
        if let Some(binding) = self.by_role.get(role) {
            let retry_gate = RetryGate::from_policy(binding.retry, device_revision, now);
            self.retry_gates.insert(role.clone(), retry_gate);
        }
    }

    /// Read what one role is waiting for before another attempt may be dispatched.
    pub(crate) fn retry_pacing(&self, role: &RoleKey) -> RetryPacing {
        self.retry_gates
            .get(role)
            .copied()
            .map_or(RetryPacing::Ready, RetryPacing::AwaitingGate)
    }

    /// Dispatch for a role the kernel stopped after three consecutive failures.
    ///
    /// The explicit half of the rule: the other way out is a successful attempt after
    /// reacquisition. Restarting clears the failure count so the role gets a full run again rather
    /// than stopping on its next single failure.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::RoleNotBound` when the role has no retained binding, and
    /// `BindingError::RoleNotStopped` when it was never stopped, so a mistaken restart cannot
    /// silently cancel an in-flight attempt.
    pub fn restart_after_repeated_failures(&mut self, role: &RoleKey) -> Result<(), BindingError> {
        let binding = self
            .by_role
            .get_mut(role)
            .ok_or_else(|| BindingError::RoleNotBound { role: role.clone() })?;
        if binding.state != RoleState::StoppedAfterRepeatedFailures {
            return Err(BindingError::RoleNotStopped { role: role.clone() });
        }
        binding.state = RoleState::Waiting;
        self.attempt_failures.remove(role);
        self.retry_gates.remove(role);
        self.stopped_role_endpoints.remove(role);

        Ok(())
    }

    /// Record how a stopped role's endpoint reads this frame and re-arm it once its device returns.
    ///
    /// This is the other half of the escalation rule: a role stopped after three consecutive
    /// failures leaves that state on an explicit
    /// `Self::restart_after_repeated_failures`, or on a successful attempt after reacquisition.
    /// Reacquisition is what this method watches for — the endpoint has to have gone
    /// `EndpointAvailability::Gone` and come back before another attempt is dispatched, so a wedged
    /// device that never leaves is not retried at frame rate. The failure count is deliberately
    /// left standing, so the returning device gets exactly one more attempt: it succeeds and clears
    /// the run, or it fails and the role stops again without a fourth dispatch.
    ///
    /// Roles in any other state are ignored, so a caller can pass every registered role.
    pub(crate) fn observe_stopped_role_endpoint(
        &mut self,
        role: &RoleKey,
        endpoint_availability: EndpointAvailability,
    ) {
        if self
            .by_role
            .get(role)
            .is_none_or(|binding| binding.state != RoleState::StoppedAfterRepeatedFailures)
        {
            return;
        }
        let previous = self
            .stopped_role_endpoints
            .insert(role.clone(), endpoint_availability);
        if previous != Some(EndpointAvailability::Gone)
            || endpoint_availability != EndpointAvailability::Available
        {
            return;
        }
        self.stopped_role_endpoints.remove(role);
        self.retry_gates.remove(role);
        if let Some(binding) = self.by_role.get_mut(role) {
            binding.state = RoleState::Waiting;
        }
    }

    /// Read whether the kernel may still dispatch a safe readback for one role.
    ///
    /// Separate from `Self::configuration_readability`, which records a driver's permanent refusal:
    /// a run of read failures is transient and clears on the first readback that succeeds.
    #[must_use]
    pub(crate) fn capture_dispatch(&self, role: &RoleKey) -> CaptureDispatch {
        if self.capture_failures.get(role).copied().unwrap_or_default() >= CONSECUTIVE_FAILURE_LIMIT
        {
            CaptureDispatch::SuspendedAfterRepeatedFailures
        } else {
            CaptureDispatch::Eligible
        }
    }

    /// Read whether a driver has permanently declined to read one role's endpoint back.
    ///
    /// A role that has never been asked reads `Readable`. This is the read-only half of the state
    /// `ReadyRole::record_capture` writes, so the safe-capture pass can find out whether a frame
    /// has any work before it takes mutable access to `Bindings`.
    #[must_use]
    pub(crate) fn configuration_readability(&self, role: &RoleKey) -> ConfigurationReadability {
        self.configuration_readability
            .get(role)
            .copied()
            .unwrap_or_default()
    }

    /// Iterate every role that currently has a retained binding.
    pub(crate) fn registered_roles(&self) -> impl Iterator<Item = &RoleKey> { self.by_role.keys() }

    /// Return the value an offline authoring interface can show for one retained role.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::RoleNotBound` when the role was never registered or was retired.
    pub fn configuration_for(
        &self,
        role: &RoleKey,
    ) -> Result<AvailableConfiguration<'_>, BindingError> {
        let binding = self.binding(role)?;
        Ok(match &binding.last_known_good {
            LastKnownGoodConfiguration::Known(configuration) => {
                AvailableConfiguration::LastKnownGood(configuration.as_ref())
            },
            LastKnownGoodConfiguration::NotEstablished => {
                AvailableConfiguration::Requested(binding.requested.as_reflect())
            },
        })
    }

    /// Change the bounded lifecycle handoff capacity without discarding already accepted work.
    ///
    /// # Errors
    ///
    /// Returns `BindingCapacityError` when the requested capacity is smaller than the number of
    /// lifecycle transitions currently awaiting the next frame drain.
    pub fn set_pending_transition_capacity(
        &mut self,
        capacity: NonZeroUsize,
    ) -> Result<(), BindingCapacityError> {
        let pending = self.pending_transitions.queue.len();
        if capacity.get() < pending {
            return Err(BindingCapacityError::BelowPendingCount { capacity, pending });
        }

        self.pending_transitions.capacity = capacity;
        Ok(())
    }

    fn has_pending_transitions(&self) -> bool { !self.pending_transitions.queue.is_empty() }

    pub(crate) fn pending_transitions(
        &self,
    ) -> impl DoubleEndedIterator<Item = &BindingTransition> + ExactSizeIterator {
        self.pending_transitions.queue.iter()
    }

    fn take_pending_transitions(&mut self) -> VecDeque<BindingTransition> {
        std::mem::take(&mut self.pending_transitions.queue)
    }

    fn reserve_transition(&self) -> Result<ReservedBindingTransition, BindingError> {
        if !self.pending_transitions.has_capacity() {
            return Err(BindingError::PendingTransitionCapacityReached);
        }
        let next_transition_sequence = self
            .next_transition_sequence
            .checked_add(1)
            .ok_or(BindingError::TransitionSequenceExhausted)?;

        Ok(ReservedBindingTransition {
            sequence: BindingTransitionSequence(self.next_transition_sequence),
            next_transition_sequence,
        })
    }

    fn reserve_transitions(
        &self,
        count: usize,
    ) -> Result<Vec<ReservedBindingTransition>, BindingError> {
        let Some(pending) = self.pending_transitions.queue.len().checked_add(count) else {
            return Err(BindingError::PendingTransitionCapacityReached);
        };
        if pending > self.pending_transitions.capacity.get() {
            return Err(BindingError::PendingTransitionCapacityReached);
        }
        let mut reservations = Vec::with_capacity(count);
        let mut sequence = self.next_transition_sequence;
        for _ in 0..count {
            let Some(next_transition_sequence) = sequence.checked_add(1) else {
                return Err(BindingError::TransitionSequenceExhausted);
            };
            reservations.push(ReservedBindingTransition {
                sequence: BindingTransitionSequence(sequence),
                next_transition_sequence,
            });
            sequence = next_transition_sequence;
        }
        Ok(reservations)
    }

    fn enqueue(
        &mut self,
        binding_transition_kind: BindingTransitionKind,
        role: RoleKey,
        reserved_transition: ReservedBindingTransition,
    ) {
        let ReservedBindingTransition {
            sequence,
            next_transition_sequence,
        } = reserved_transition;
        self.next_transition_sequence = next_transition_sequence;
        let binding_transition = match binding_transition_kind {
            BindingTransitionKind::Registered => BindingTransition::Registered { sequence, role },
            BindingTransitionKind::Replaced => BindingTransition::Replaced { sequence, role },
            BindingTransitionKind::Retired(endpoint) => BindingTransition::Retired {
                sequence,
                role,
                endpoint,
            },
        };
        self.pending_transitions.push(binding_transition);
    }

    fn remove_role_from_device(&mut self, device_key: &DeviceKey, role: &RoleKey) {
        let remove_device_entry = if let Some(roles) = self.roles_by_device.get_mut(device_key) {
            roles.retain(|stored_role| stored_role != role);
            roles.is_empty()
        } else {
            false
        };
        if remove_device_entry {
            self.roles_by_device.remove(device_key);
        }
    }
}

enum BindingTransitionKind {
    Registered,
    Replaced,
    Retired(DeviceEndpoint),
}

struct ReservedBindingTransition {
    sequence:                 BindingTransitionSequence,
    next_transition_sequence: u64,
}

struct PreparedDeviceReaddress {
    role:                RoleKey,
    saved_endpoint:      DeviceEndpoint,
    adopted_endpoint:    DeviceEndpoint,
    reserved_transition: ReservedBindingTransition,
}

/// Result of retiring one role while distinguishing a repeated request from an effective change.
pub enum RetirementOutcome {
    /// The retained binding was removed and marked retired before it was returned.
    Retired(Binding),
    /// No binding remained for this role, so no lifecycle work was added.
    AlreadyUnbound,
}

/// Recoverable failure from a checked binding operation or state-issued request.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BindingError {
    /// The submitted role already has an authored binding whose endpoint ownership must remain.
    #[error("role `{role}` is already bound")]
    RoleAlreadyBound {
        /// Existing application role that rejected a second binding record.
        role: RoleKey,
    },
    /// An operation required a binding for this role, but none remains registered.
    #[error("role `{role}` is not bound")]
    RoleNotBound {
        /// Application role that did not select a retained binding record.
        role: RoleKey,
    },
    /// A different role already owns the proposed endpoint, so two drivers cannot race it.
    #[error("endpoint `{endpoint:?}` is already owned by role `{owner}`")]
    EndpointAlreadyOwned {
        /// Endpoint the operation proposed for a second role.
        endpoint: DeviceEndpoint,
        /// Retained role whose binding already owns `endpoint`.
        owner:    RoleKey,
    },
    /// The lifecycle handoff is full, so accepting another authored mutation would lose work.
    #[error("pending binding transition capacity has been reached")]
    PendingTransitionCapacityReached,
    /// The next lifecycle handoff sequence cannot advance without reusing a prior transition id.
    #[error("binding transition sequence is exhausted")]
    TransitionSequenceExhausted,
    /// An apply of authored intent requires the permit that authorizes in-service use.
    #[error("requested configuration requires an in-service apply permit")]
    RequestedConfigurationRequiresInServicePermit,
    /// A restore from observed endpoint state requires the restore-only authorization purpose.
    #[error("last-known-good configuration requires a restore-only apply permit")]
    LastKnownGoodConfigurationRequiresRestoreOnlyPermit,
    /// A restore was requested before a safe readback established a value to restore.
    #[error("role `{role}` has no last-known-good configuration")]
    LastKnownGoodNotEstablished {
        /// Role whose configuration remains authored intent rather than endpoint evidence.
        role: RoleKey,
    },
    /// The configured device is offline, so passive discovery may observe it but no driver call
    /// may be issued for its endpoint.
    #[error("configured device `{device_key:?}` is offline")]
    ConfiguredDeviceOffline {
        /// Durable key whose authored offline mode blocks operational requests.
        device_key: DeviceKey,
    },
    /// A restart was requested for a role the kernel had not stopped, which would have cancelled
    /// whatever that role was doing instead.
    #[error("role `{role}` was not stopped after repeated failures")]
    RoleNotStopped {
        /// Role whose lifecycle state is not `crate::RoleState::StoppedAfterRepeatedFailures`.
        role: RoleKey,
    },
    /// A driver previously established that this endpoint cannot provide a safe configuration.
    #[error("role `{role}` has no readable endpoint configuration")]
    ConfigurationNotReadable {
        /// Role whose driver returned `CaptureOutcome::NotReadable`.
        role: RoleKey,
    },
}

/// Failure from attempting to lower lifecycle handoff capacity below already pending work.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BindingCapacityError {
    /// The requested capacity cannot retain the transitions already waiting for a frame drain.
    #[error("pending transition count {pending} exceeds requested capacity {capacity}")]
    BelowPendingCount {
        /// Capacity the caller requested for future lifecycle changes.
        capacity: NonZeroUsize,
        /// Number of transitions that must remain retained before the next drain.
        pending:  usize,
    },
}

/// State-selected access to one binding; only the contained view exposes valid operations.
pub(crate) enum RoleView<'a> {
    /// The role has no usable endpoint and may start a newly authorized operation.
    Waiting(WaitingRole<'a>),
    /// The role has reached its target and may ask for a safe configuration readback.
    Ready(ReadyRole<'a>),
    /// The role has an in-flight driver operation and may poll, abort, or finish it.
    Applying(ApplyingRole<'a>),
    /// Three consecutive attempts failed, so nothing is dispatched until the role is restarted or
    /// a later attempt succeeds.
    StoppedAfterRepeatedFailures,
    /// The role was retired, so no driver operation can be issued.
    Retired,
}

/// Whether a Ready role retains the exact attempt that established its current session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EstablishingAttemptLookup {
    /// This globally unique attempt established the session the role currently considers Ready.
    EstablishedBy(AttemptId),
    /// No successful attempt currently establishes a session for this role.
    NotEstablished,
}

/// Result of applying a session-loss policy after its guards were selected.
pub(crate) enum SessionLossApplication {
    /// The binding still existed and its authored policy was applied.
    Applied(OnSessionLoss),
    /// The binding was absent when mutation was attempted.
    BindingAbsent,
}

/// A role with no usable endpoint operation in progress, resolved by what it is owed.
///
/// The stored `WaitingWork` picks the arm, so only the request that is actually owed exists on the
/// value a caller holds: a role owing a restoration has no requested-apply method to reach for, and
/// a role owing nothing has no restore method. Restating that rule with a runtime check at every
/// call site is what this enum removes.
pub(crate) enum WaitingRole<'a> {
    /// Nothing is owed: the role waits for usable, authorized hardware and may start an apply from
    /// its authored request.
    Hardware(RequestingRole<'a>),
    /// A last-known-good restoration is owed and runs as soon as the role is authorized.
    Restoration(RestoringRole<'a>),
    /// The role's `crate::RecoveryPolicy` refused an automatic reapply after its device departed.
    /// It carries no view because there is nothing to mint: the kernel waits for application code.
    ApplicationRequest,
}

/// View of a waiting role that owes nothing and may reach for its authored target.
pub(crate) struct RequestingRole<'a> {
    binding:         &'a mut Binding,
    applying_source: &'a mut HashMap<RoleKey, ApplyConfigurationSource>,
}

impl<'a> RequestingRole<'a> {
    /// Start an authorized apply from the binding's authored requested configuration.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::ConfiguredDeviceOffline` when inventory marks this durable device
    /// offline, which prevents the request before a driver can observe it.
    pub(crate) fn start_requested_apply(
        self,
        attempt: AttemptId,
        permit: ApplyPermit,
        hardware_inventory: &HardwareInventory,
    ) -> Result<StartApplyRequest<'a>, BindingError> {
        // A restore-only permit may drive authored intent in exactly one case: no safe readback has
        // established anything to restore, so applying the request is the only way a `RestoreOnly`
        // device ever reaches a state a later capture can read back. Once a value is established,
        // authored intent needs the in-service permit again.
        if !permit.allows_in_service_use()
            && !matches!(
                self.binding.last_known_good,
                LastKnownGoodConfiguration::NotEstablished
            )
        {
            return Err(BindingError::RequestedConfigurationRequiresInServicePermit);
        }
        hardware_inventory.ensure_operational(&self.binding.endpoint.device)?;
        self.applying_source.insert(
            self.binding.role.clone(),
            ApplyConfigurationSource::Requested,
        );
        Ok(StartApplyRequest {
            binding: self.binding,
            configuration_source: ApplyConfigurationSource::Requested,
            attempt,
            permit,
        })
    }
}

/// View of a waiting role that owes a restoration of the value a safe readback established.
pub(crate) struct RestoringRole<'a> {
    binding:         &'a mut Binding,
    applying_source: &'a mut HashMap<RoleKey, ApplyConfigurationSource>,
}

impl<'a> RestoringRole<'a> {
    /// Start an authorized restore from the value a safe readback previously established.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::LastKnownGoodNotEstablished` until a successful safe readback has
    /// supplied an endpoint value, or `BindingError::ConfiguredDeviceOffline` for offline
    /// inventory that may be discovered passively but may not receive output.
    pub(crate) fn start_last_known_good_restore(
        self,
        attempt: AttemptId,
        permit: ApplyPermit,
        hardware_inventory: &HardwareInventory,
    ) -> Result<StartApplyRequest<'a>, BindingError> {
        if permit.allows_in_service_use() {
            return Err(BindingError::LastKnownGoodConfigurationRequiresRestoreOnlyPermit);
        }
        hardware_inventory.ensure_operational(&self.binding.endpoint.device)?;
        self.binding.last_known_good.as_reflect().map_err(|_| {
            BindingError::LastKnownGoodNotEstablished {
                role: self.binding.role.clone(),
            }
        })?;
        self.applying_source.insert(
            self.binding.role.clone(),
            ApplyConfigurationSource::LastKnownGood,
        );
        Ok(StartApplyRequest {
            binding: self.binding,
            configuration_source: ApplyConfigurationSource::LastKnownGood,
            attempt,
            permit,
        })
    }
}

/// View of a role that has a usable endpoint and no in-flight driver operation.
pub struct ReadyRole<'a> {
    binding:                   &'a mut Binding,
    configuration_readability: &'a mut HashMap<RoleKey, ConfigurationReadability>,
    capture_failures:          &'a mut HashMap<RoleKey, u32>,
}

impl<'a> ReadyRole<'a> {
    /// Mint the only capture request accepted by driver dispatch.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::ConfigurationNotReadable` after the driver permanently declined a
    /// safe readback, or `BindingError::ConfiguredDeviceOffline` before any driver call for an
    /// offline configured endpoint.
    pub(crate) fn capture_request(
        self,
        hardware_inventory: &HardwareInventory,
    ) -> Result<CaptureRequest<'a>, BindingError> {
        hardware_inventory.ensure_operational(&self.binding.endpoint.device)?;
        if self
            .configuration_readability
            .get(&self.binding.role)
            .copied()
            .unwrap_or_default()
            == ConfigurationReadability::PermanentlyUnreadable
        {
            return Err(BindingError::ConfigurationNotReadable {
                role: self.binding.role.clone(),
            });
        }

        Ok(CaptureRequest {
            role:     &self.binding.role,
            driver:   self.binding.driver,
            endpoint: &self.binding.endpoint,
        })
    }

    /// Record what one safe driver readback established without treating an apply target as proof.
    pub(crate) fn record_capture(
        &mut self,
        capture_outcome: CaptureOutcome<LastKnownGoodConfiguration>,
    ) {
        match capture_outcome {
            CaptureOutcome::Read(last_known_good) => {
                // `Bindings` is a resource, so assigning an equal configuration would still mark it
                // changed and make a settled frame look like it carried new evidence.
                if !self
                    .binding
                    .last_known_good
                    .holds_same_value(&last_known_good)
                {
                    self.binding.last_known_good = last_known_good;
                }
                self.capture_failures.remove(&self.binding.role);
            },
            CaptureOutcome::NotReadable => {
                if self
                    .configuration_readability
                    .get(&self.binding.role)
                    .copied()
                    .unwrap_or_default()
                    != ConfigurationReadability::PermanentlyUnreadable
                {
                    self.configuration_readability.insert(
                        self.binding.role.clone(),
                        ConfigurationReadability::PermanentlyUnreadable,
                    );
                }
            },
            CaptureOutcome::ReadFailed(_) => {
                let consecutive = self
                    .capture_failures
                    .get(&self.binding.role)
                    .copied()
                    .unwrap_or_default()
                    .saturating_add(1);
                self.capture_failures
                    .insert(self.binding.role.clone(), consecutive);
            },
        }
    }
}

/// View of a role whose driver operation is in flight.
pub(crate) struct ApplyingRole<'a> {
    binding:               &'a mut Binding,
    waiting_work:          &'a mut HashMap<RoleKey, WaitingWork>,
    applying_source:       &'a mut HashMap<RoleKey, ApplyConfigurationSource>,
    establishing_attempts: &'a mut HashMap<RoleKey, AttemptId>,
}

impl<'a> ApplyingRole<'a> {
    /// Mint the only poll request accepted by driver dispatch for this in-flight attempt.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::ConfiguredDeviceOffline` when inventory switched this device to
    /// offline before the next poll, preventing the driver from continuing the operation.
    pub(crate) fn poll_request(
        self,
        hardware_inventory: &HardwareInventory,
    ) -> Result<PollRequest<'a>, BindingError> {
        hardware_inventory.ensure_operational(&self.binding.endpoint.device)?;
        let RoleState::Applying(attempt) = self.binding.state else {
            return Err(BindingError::RoleNotBound {
                role: self.binding.role.clone(),
            });
        };
        Ok(PollRequest {
            role: &self.binding.role,
            driver: self.binding.driver,
            endpoint: &self.binding.endpoint,
            attempt,
        })
    }

    /// Stop the in-flight operation and return the role to the waiting lifecycle state.
    ///
    /// The abandoned attempt stops being the one that could settle a restoration debt, so the next
    /// dispatch decides that again from the request it mints.
    pub(crate) fn abort(&mut self) {
        self.binding.state = RoleState::Waiting;
        self.establishing_attempts.remove(&self.binding.role);
        self.take_applying_source();
    }

    /// Take the record of which configuration the ending apply was dispatched from.
    ///
    /// Taken rather than read, because the record describes an apply that is over: leaving it
    /// behind would let the next ending on this role read a source no dispatch of its own recorded.
    fn take_applying_source(&mut self) -> ApplySourceLookup {
        self.applying_source.remove(&self.binding.role).map_or(
            ApplySourceLookup::NotDispatched,
            ApplySourceLookup::Dispatched,
        )
    }

    /// Finish the in-flight operation, making only a successful apply ready for safe readback.
    ///
    /// A `WaitingWork::RestorationOwed` debt is settled by the restoration and by nothing else: a
    /// role whose device departed while an ordinary apply was in flight owes a restoration that
    /// apply never performed, so reading `RoleState::Ready` as proof would leave the returning
    /// device holding the requested value with the debt gone. An outcome that returns the role to
    /// `RoleState::Waiting` leaves the debt for the next pass to dispatch again.
    pub(crate) fn finish(&mut self, attempt_outcome: AttemptOutcome) {
        let establishing_attempt = match self.binding.state {
            RoleState::Applying(attempt) => EstablishingAttemptLookup::EstablishedBy(attempt),
            RoleState::Waiting
            | RoleState::Ready
            | RoleState::StoppedAfterRepeatedFailures
            | RoleState::Retired => EstablishingAttemptLookup::NotEstablished,
        };
        self.binding.state = match &attempt_outcome {
            AttemptOutcome::Succeeded | AttemptOutcome::Substituted => RoleState::Ready,
            AttemptOutcome::Failed(_) | AttemptOutcome::Aborted => RoleState::Waiting,
        };
        match (attempt_outcome, establishing_attempt) {
            (AttemptOutcome::Succeeded, EstablishingAttemptLookup::EstablishedBy(attempt)) => {
                self.establishing_attempts
                    .insert(self.binding.role.clone(), attempt);
            },
            (AttemptOutcome::Succeeded, EstablishingAttemptLookup::NotEstablished)
            | (
                AttemptOutcome::Failed(_) | AttemptOutcome::Aborted | AttemptOutcome::Substituted,
                _,
            ) => {
                self.establishing_attempts.remove(&self.binding.role);
            },
        }
        let restoration_completed = self.take_applying_source().restored_last_known_good();
        if self.binding.state == RoleState::Ready
            && restoration_completed
            && self
                .waiting_work
                .get(&self.binding.role)
                .copied()
                .unwrap_or_default()
                != WaitingWork::Nothing
        {
            self.waiting_work
                .insert(self.binding.role.clone(), WaitingWork::Nothing);
        }
    }
}

/// Whether the kernel may still ask a driver to read one role's endpoint configuration back.
///
/// The transient half of readback eligibility. A driver whose readback keeps failing is stopped
/// after three consecutive attempts instead of being dispatched once per frame for as long as the
/// binding lives, and the first successful readback resumes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub(crate) enum CaptureDispatch {
    /// Fewer than three consecutive readbacks have failed, so capture stays eligible.
    #[default]
    Eligible,
    /// Three consecutive readbacks failed, so the kernel stops asking until one succeeds.
    SuspendedAfterRepeatedFailures,
}

/// Whether one role's durable endpoint currently resolves to a device the kernel may drive.
///
/// A named reading rather than a `bool`, because it is the reacquisition signal a stopped role
/// waits on: `Gone` covers an endpoint that resolves to nothing and one whose device is retained
/// but no longer present, and both are the same fact for that decision — the unit the role was
/// failing against is not the unit it would be dispatched against next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EndpointAvailability {
    /// The endpoint resolves to a device the current reconcile pass reads as present.
    Available,
    /// The endpoint resolves to nothing, or to a device that is no longer present.
    Gone,
}

/// Which configuration source the apply now ending drew from.
///
/// A named result rather than a bare `Option`: absence means "no dispatch recorded a source for
/// this role", which is not the same as "the role restored its last-known-good value", and reading
/// it as the latter would settle a restoration debt nothing paid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplySourceLookup {
    /// No in-flight apply recorded a configuration source for this role.
    NotDispatched,
    /// The in-flight apply was dispatched from this source.
    Dispatched(ApplyConfigurationSource),
}

impl ApplySourceLookup {
    /// Report whether the ending apply was the last-known-good restoration a debt is settled by.
    const fn restored_last_known_good(self) -> bool {
        matches!(
            self,
            Self::Dispatched(ApplyConfigurationSource::LastKnownGood)
        )
    }
}

/// What one role is waiting for before another attempt may be dispatched after a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryPacing {
    /// Nothing paces this role: it has not failed, or its last attempt succeeded.
    Ready,
    /// The role failed and this gate has not opened yet.
    AwaitingGate(RetryGate),
}

impl RetryPacing {
    /// Report whether an attempt may be dispatched for this role on this frame.
    pub(crate) fn permits_dispatch(
        self,
        device_revision: DeviceRevisionLookup,
        now: FrameClockReading,
    ) -> bool {
        match self {
            Self::Ready => true,
            Self::AwaitingGate(retry_gate) => retry_gate.opened(device_revision, now),
        }
    }
}

/// State-issued permission to ask one driver for a safe endpoint configuration readback.
///
/// Its fields stay private so application code cannot choose a `DriverId` and endpoint without a
/// `ReadyRole` proving that the binding reached the state where capture is meaningful.
pub(crate) struct CaptureRequest<'a> {
    pub(crate) role:     &'a RoleKey,
    pub(crate) driver:   DriverId,
    pub(crate) endpoint: &'a DeviceEndpoint,
}

/// State-issued permission to start one asynchronous driver apply.
///
/// The request borrows the selected configuration source, so a later mutation cannot replace the
/// driver's target between lifecycle authorization and erased driver dispatch.
pub struct StartApplyRequest<'a> {
    pub(crate) binding:              &'a mut Binding,
    pub(crate) configuration_source: ApplyConfigurationSource,
    pub(crate) attempt:              AttemptId,
    pub(crate) permit:               ApplyPermit,
}

/// Configuration source paired with the authorization purpose that permits its dispatch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ApplyConfigurationSource {
    Requested,
    LastKnownGood,
}

impl ApplyConfigurationSource {
    pub(crate) fn configuration(self, binding: &Binding) -> Result<&dyn Reflect, BindingError> {
        match self {
            Self::Requested => Ok(binding.requested.as_reflect()),
            Self::LastKnownGood => binding.last_known_good.as_reflect().map_err(|_| {
                BindingError::LastKnownGoodNotEstablished {
                    role: binding.role.clone(),
                }
            }),
        }
    }
}

/// State-issued permission to poll one in-flight driver apply.
///
/// The request retains the role and endpoint even though the driver trait polls by attempt id,
/// so reconciliation can compare the token with current resolution before dispatch.
pub(crate) struct PollRequest<'a> {
    pub(crate) role:     &'a RoleKey,
    pub(crate) driver:   DriverId,
    pub(crate) endpoint: &'a DeviceEndpoint,
    pub(crate) attempt:  AttemptId,
}

/// Whether authored inventory permits driver operations for a configured device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum ConfiguredDeviceMode {
    /// The kernel may later ask a driver to capture, apply, and poll this device's endpoints.
    Managed,
    /// Reporters may enumerate the device passively, but no driver operation may touch it.
    Offline,
}

/// Which role already holds one endpoint, kept as a named state rather than an absent role.
///
/// Read by `crate::IdentityDecisions` before it records an adoption, where "nobody owns it" and
/// "another role owns it" lead to opposite answers for the operator.
#[derive(Clone, Debug, Default, PartialEq, Eq, Reflect)]
pub(crate) enum EndpointOwner {
    /// No other role owns the endpoint, so an adoption may move onto it.
    #[default]
    Unowned,
    /// This role owns the endpoint, so an adoption would have to take it away and does not.
    OwnedBy(RoleKey),
}

/// Authored device inventory entry that exists independently of reporter activation and entities.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub struct ConfiguredDevice {
    /// Durable identity the application authored without creating a live device entity.
    pub key:  DeviceKey,
    /// Operational rule that leaves passive connection evidence visible when offline.
    pub mode: ConfiguredDeviceMode,
}

/// Connectivity conclusion retained for one authored device without changing its operational mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum ConfiguredDeviceConnection {
    /// No enabled reporter completed a discovery capable of observing this authored key.
    NotObserved,
    /// Current passive reporter evidence contains this authored key.
    Present,
    /// Relevant complete reporter evidence omitted this key.
    Absent,
    /// Relevant evidence expired or discovery failed before absence could be established.
    Unreachable,
}

/// Authored durable hardware inventory and passive connectivity conclusions.
///
/// Adding an entry does not register, enable, or run a reporter, and it never creates a live
/// `DeviceId`. Reconciliation updates `ConfiguredDeviceConnection` from retained reporter
/// evidence while this resource keeps offline operation separate from connection visibility.
#[derive(Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct HardwareInventory {
    #[reflect(ignore, default = "default_configured_devices")]
    configured:  HashMap<DeviceKey, ConfiguredDevice>,
    #[reflect(ignore, default = "default_configured_device_connections")]
    connections: HashMap<DeviceKey, ConfiguredDeviceConnection>,
}

fn default_configured_devices() -> HashMap<DeviceKey, ConfiguredDevice> { HashMap::new() }

fn default_configured_device_connections() -> HashMap<DeviceKey, ConfiguredDeviceConnection> {
    HashMap::new()
}

impl HardwareInventory {
    /// Retain one authored device without enabling reporters or creating a device entity.
    pub fn configure(&mut self, configured_device: ConfiguredDevice) {
        let device_key = configured_device.key.clone();
        self.configured
            .insert(device_key.clone(), configured_device);
        self.connections
            .entry(device_key)
            .or_insert(ConfiguredDeviceConnection::NotObserved);
    }

    /// Move one authored entry and its connection conclusion onto an adopted durable key.
    ///
    /// Called with the binding rewrite in `Bindings::readdress`, because the two are keyed the same
    /// way: an adoption that moved the binding and left inventory holding the old key would leave
    /// the authored operation mode attached to a unit nothing addresses any more.
    ///
    /// A saved key nobody authored has nothing to move, which is not a failure — inventory records
    /// the application's decisions, and having made none is not one.
    pub(crate) fn readdress(&mut self, saved: &DeviceKey, candidate: DeviceKey) {
        let Some(mut configured_device) = self.configured.remove(saved) else {
            return;
        };
        let connection = self
            .connections
            .remove(saved)
            .unwrap_or(ConfiguredDeviceConnection::NotObserved);
        configured_device.key = candidate.clone();
        self.configured.insert(candidate.clone(), configured_device);
        self.connections.insert(candidate, connection);
    }

    /// Borrow one configured device and its authored operation mode.
    ///
    /// # Errors
    ///
    /// Returns `HardwareInventoryError::DeviceNotConfigured` when no authored entry uses this
    /// durable key.
    pub fn configured_device(
        &self,
        device_key: &DeviceKey,
    ) -> Result<&ConfiguredDevice, HardwareInventoryError> {
        self.configured
            .get(device_key)
            .ok_or_else(|| HardwareInventoryError::DeviceNotConfigured {
                device_key: device_key.clone(),
            })
    }

    /// Read the passive connection conclusion retained for one authored device.
    ///
    /// # Errors
    ///
    /// Returns `HardwareInventoryError::DeviceNotConfigured` for a key that application code did
    /// not author into this inventory.
    pub fn connection(
        &self,
        device_key: &DeviceKey,
    ) -> Result<ConfiguredDeviceConnection, HardwareInventoryError> {
        self.configured_device(device_key)?;
        self.connections.get(device_key).copied().ok_or_else(|| {
            HardwareInventoryError::DeviceNotConfigured {
                device_key: device_key.clone(),
            }
        })
    }

    /// Iterate every durable key application code authored into this inventory.
    ///
    /// Reconciliation walks these rather than the reported device set: an authored unit that no
    /// reporter has ever named still has a connection conclusion to record, and it is exactly the
    /// case a walk over live evidence would miss.
    pub(crate) fn configured_keys(&self) -> impl Iterator<Item = &DeviceKey> {
        self.configured.keys()
    }

    /// Record what current reporter evidence says about one authored device's connectivity.
    ///
    /// Connection is separate from `ConfiguredDeviceMode`: learning that an offline unit is plugged
    /// in neither enables a reporter nor authorizes anything to drive it.
    ///
    /// # Errors
    ///
    /// Returns `HardwareInventoryError::DeviceNotConfigured` for a key that application code did
    /// not author into this inventory.
    pub(crate) fn set_connection(
        &mut self,
        device_key: &DeviceKey,
        connection: ConfiguredDeviceConnection,
    ) -> Result<(), HardwareInventoryError> {
        self.configured_device(device_key)?;
        self.connections.insert(device_key.clone(), connection);
        Ok(())
    }

    /// Report whether an endpoint's durable device may receive driver traffic at all.
    ///
    /// Callers that must decide before taking mutable access — the safe-capture pass reads this to
    /// learn whether a frame has work before it borrows `Bindings` mutably — need the same answer
    /// the typed role views enforce, and a second copy of the offline rule would let the two drift.
    ///
    /// # Errors
    ///
    /// Returns `BindingError::ConfiguredDeviceOffline` for a device inventory marks offline.
    pub(crate) fn ensure_operational(&self, device_key: &DeviceKey) -> Result<(), BindingError> {
        match self.configured.get(device_key) {
            Some(ConfiguredDevice {
                mode: ConfiguredDeviceMode::Offline,
                ..
            }) => Err(BindingError::ConfiguredDeviceOffline {
                device_key: device_key.clone(),
            }),
            Some(ConfiguredDevice {
                mode: ConfiguredDeviceMode::Managed,
                ..
            })
            | None => Ok(()),
        }
    }
}

/// Process-local entity that carries one registered role's mirrored lifecycle state.
///
/// The entity exists for as long as the role is registered, which is longer than any device that
/// fills it: a projector that is unplugged mid-show leaves its role's policy, state, and later its
/// configuration mirror addressable, so a panel does not lose the row it was drawing. `RoleKey`,
/// `RecoveryPolicy`, and `RoleState` sit on this entity rather than on the device entity because a
/// Stream Deck with `"key/3"`, `"dial/1"`, and `"strip"` bound has one of each per role, and a
/// single component per unit would keep only whichever role was written last.
///
/// `Bindings` stays authoritative. The components here are refreshed from it on every reconcile, so
/// a Bevy Remote Protocol write to the mirrored `RecoveryPolicy` is overwritten on the next frame
/// instead of quietly changing what the kernel will do to live hardware.
#[derive(Debug, Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct BindingEntities {
    by_role: HashMap<RoleKey, Entity>,
}

impl BindingEntities {
    /// Find the entity carrying one registered role's mirrored state.
    #[must_use]
    pub fn entity(&self, role: &RoleKey) -> BindingEntityLookup {
        self.by_role
            .get(role)
            .map_or(BindingEntityLookup::Unregistered, |entity| {
                BindingEntityLookup::Registered(*entity)
            })
    }

    /// How many registered roles currently have a binding entity.
    #[must_use]
    pub fn count(&self) -> usize { self.by_role.len() }
}

/// Result of asking which entity carries one role's mirrored binding state.
///
/// A named result rather than an optional entity, because the absent case means the role was never
/// registered or has been retired — not that its device is missing. A caller that read "no entity"
/// as "offline" would wait forever for a role nothing will ever spawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BindingEntityLookup {
    /// No registered binding uses this role, so nothing mirrors its lifecycle state.
    Unregistered,
    /// The role is registered, and this entity carries its mirrored policy and state for as long as
    /// the registration lasts.
    Registered(Entity),
}

/// The binding entity's current link to the live device entity its endpoint resolves to.
///
/// Present only while the durable `DeviceEndpoint` names a device the kernel currently retains, so
/// its absence is exactly "this role has no live hardware right now". It is a relationship rather
/// than a second ownership map because Bevy then maintains `ResolvedBindings` on the device side
/// for free, and replacing the link moves the binding between reverse collections with no
/// bookkeeping that could drift from the authored record in `Bindings`.
///
/// It deliberately omits `linked_spawn`: despawning a departed device must remove this link and
/// nothing else. Despawning the binding entity would erase the role's retained policy and
/// configuration, which is the state that makes a returning unit recoverable at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Component, Reflect)]
#[relationship(relationship_target = ResolvedBindings)]
#[reflect(Component, PartialEq)]
pub struct ResolvedToDevice(Entity);

impl ResolvedToDevice {
    /// Link one binding entity to the device entity its durable endpoint currently resolves to.
    pub(crate) const fn new(device: Entity) -> Self { Self(device) }

    /// Read the device entity this link currently points at.
    #[must_use]
    pub const fn device(self) -> Entity { self.0 }
}

/// Every binding entity whose endpoint currently resolves to this live device entity.
///
/// Bevy maintains this collection from `ResolvedToDevice`, so a tool holding a device entity can
/// walk to every role using that unit — the Stream Deck's three bound endpoints, or a display
/// shared by two window roles — without the kernel keeping a second entity index that could
/// disagree with the authored record. It reports live resolution only; `Bindings::roles_for`
/// remains authoritative for durable ownership and for roles whose device is absent, and the
/// relationship cannot enforce endpoint uniqueness because it targets the whole device rather than
/// one endpoint of it.
#[derive(Debug, Component, Reflect)]
#[relationship_target(relationship = ResolvedToDevice)]
#[reflect(Component)]
pub struct ResolvedBindings(Vec<Entity>);

/// Every binding transition accepted before this frame's drain, in the order they were accepted.
///
/// One drain per frame moves `Bindings::take_pending_transitions` in here so the binding-entity
/// stage, the attempt aborts, and the public event stage all read one identical ordered
/// list. Reading `Bindings` directly from three stages would let the first drain hide the
/// registration from the other two. Entries are never removed one at a time: the event stage clears
/// the whole batch once it has emitted this frame's transitions, and `drain_binding_transitions`
/// replaces the contents wholesale on the next frame regardless, so a missing `clear` cannot strand
/// entries past the frame that produced them.
#[derive(Debug, Default, Resource)]
pub(crate) struct BindingTransitionBatch {
    transitions: Vec<BindingTransition>,
}

impl BindingTransitionBatch {
    /// Read this frame's accepted transitions in the order `Bindings` sequenced them.
    pub(crate) fn transitions(&self) -> &[BindingTransition] { &self.transitions }

    /// Drop this frame's transitions once the last consumer has read them.
    pub(crate) fn clear(&mut self) { self.transitions.clear(); }
}

/// Move every binding transition accepted since the last frame into this frame's shared batch.
///
/// Registration and retirement are application work, not discovery work, so this runs whether or
/// not a reporter completed a scan: it is ordered only `before` reconciliation, which returns early
/// on a settled frame and would otherwise strand an accepted transition until the next scan landed.
/// Operations submitted after this system runs stay in `Bindings` and are drained next frame.
///
/// A frame with nothing to move leaves `Bindings` untouched rather than taking an empty queue
/// through `ResMut`, so change detection on the resource still means "an authored operation was
/// accepted" for a once-per-change event stage or a Bevy Remote Protocol resource watch.
pub(crate) fn drain_binding_transitions(
    mut bindings: ResMut<Bindings>,
    mut binding_transition_batch: ResMut<BindingTransitionBatch>,
) {
    if bindings.has_pending_transitions() {
        binding_transition_batch.transitions = bindings.take_pending_transitions().into();
    } else if !binding_transition_batch.transitions.is_empty() {
        binding_transition_batch.clear();
    }
}

/// Spawn, refresh, and despawn the entity that mirrors each registered role's lifecycle state.
///
/// Retirement despawns the entity, which also removes any `ResolvedToDevice` link without touching
/// the device entity on the other side. Mirrors are written only when the authored value differs,
/// so a settled frame reports no component change and once-per-change events stay derivable.
///
/// The mirror refresh runs before this frame's transitions because `Commands::spawn` is deferred:
/// an entity registered in the loop below is not queryable until the schedule applies its commands,
/// so refreshing afterwards would read every new entity as one that no longer exists.
pub(crate) fn project_binding_entities(
    mut commands: Commands,
    binding_transition_batch: Res<BindingTransitionBatch>,
    bindings: Res<Bindings>,
    mut binding_entities: ResMut<BindingEntities>,
    mut mirrors: Query<(&mut RecoveryPolicy, &mut RoleState), With<RoleKey>>,
    live_entities: Query<()>,
) {
    binding_entities.by_role.retain(|role, entity| {
        let Ok(binding) = bindings.binding(role) else {
            return true;
        };
        let Ok((mut recovery_policy, mut role_state)) = mirrors.get_mut(*entity) else {
            // A despawn from outside the kernel leaves a mapping that would keep promising a live
            // entity carrying mirrored state, so it is dropped. An entity that is still alive but
            // lost a mirrored component — a Bevy Remote Protocol *remove* rather than a write — is
            // a different case: the role is still registered, so the components are re-inserted
            // and the mapping stays. Dropping it there would permanently un-index the role, while
            // the type doc promises a remote write is repaired on the next frame.
            if live_entities.get(*entity).is_ok() {
                commands
                    .entity(*entity)
                    .insert((role.clone(), binding.recovery, binding.state));

                return true;
            }

            return false;
        };
        if *recovery_policy != binding.recovery {
            *recovery_policy = binding.recovery;
        }
        if *role_state != binding.state {
            *role_state = binding.state;
        }

        true
    });

    for binding_transition in binding_transition_batch.transitions() {
        match binding_transition {
            BindingTransition::Registered { role, .. } => {
                let Ok(binding) = bindings.binding(role) else {
                    continue;
                };
                let entity = commands
                    .spawn((role.clone(), binding.recovery, binding.state))
                    .id();
                binding_entities.by_role.insert(role.clone(), entity);
            },
            // A replacement keeps the role registered and its entity alive; the refresh above
            // writes the `RoleState::Waiting` that `Bindings::replace` already stored.
            BindingTransition::Replaced { .. } => {},
            BindingTransition::Retired { role, endpoint, .. } => {
                if let Some(entity) = binding_entities.by_role.remove(role) {
                    commands.entity(entity).despawn();
                }
                commands.trigger(BindingRetired {
                    role:     role.clone(),
                    endpoint: endpoint.clone(),
                });
            },
        }
    }
}

/// Failure from reading or updating an authored inventory key that does not exist.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HardwareInventoryError {
    /// The requested durable key has no authored inventory record in this application.
    #[error("device `{device_key:?}` is not configured")]
    DeviceNotConfigured {
        /// Key that did not select a `ConfiguredDevice` inventory entry.
        device_key: DeviceKey,
    },
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::any::TypeId;
    use std::error::Error;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::sync::Mutex;

    use bevy::app::App;
    use bevy::app::Update;
    use bevy::ecs::change_detection::DetectChanges;
    use bevy::ecs::entity::Entity;
    use bevy::ecs::observer::On;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::ecs::relationship::Relationship;
    use bevy::ecs::relationship::RelationshipTarget;
    use bevy::ecs::schedule::IntoScheduleConfigs;
    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::World;

    use super::ApplyDeadline;
    use super::AvailableConfiguration;
    use super::Binding;
    use super::BindingCapacityError;
    use super::BindingEntities;
    use super::BindingEntityLookup;
    use super::BindingError;
    use super::BindingTransition;
    use super::BindingTransitionBatch;
    use super::BindingTransitionSequence;
    use super::Bindings;
    use super::ConfiguredDevice;
    use super::ConfiguredDeviceConnection;
    use super::ConfiguredDeviceMode;
    use super::HardwareInventory;
    use super::RequestedConfiguration;
    use super::ResolvedBindings;
    use super::ResolvedToDevice;
    use super::RetirementOutcome;
    use super::RoleView;
    use super::WaitingRole;
    use super::WaitingWork;
    use super::drain_binding_transitions;
    use super::project_binding_entities;
    use crate::ApplyPermit;
    use crate::AttemptId;
    use crate::AttemptOutcome;
    use crate::AttemptProgress;
    use crate::BindingRetired;
    use crate::CaptureOutcome;
    use crate::DeviceAccessError;
    use crate::DeviceEndpoint;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DeviceRevisionLookup;
    use crate::DriverContractError;
    use crate::EndpointDriver;
    use crate::EndpointId;
    use crate::LastKnownGoodConfiguration;
    use crate::OnAbort;
    use crate::OnSessionLoss;
    use crate::PartName;
    use crate::RecoveryPolicy;
    use crate::RetryOn;
    use crate::RiggingPlugin;
    use crate::RoleKey;
    use crate::RoleState;
    use crate::reconcile::FrameClockReading;
    use crate::registration::DriverId;
    use crate::registration::Drivers;
    use crate::scheme::AuthoredId;

    #[derive(Component, Reflect)]
    struct TestConfiguration(u8);

    struct RecordingDriver {
        applied_configurations: Arc<Mutex<Vec<u8>>>,
    }

    impl EndpointDriver for RecordingDriver {
        type Configuration = TestConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            CaptureOutcome::Read(TestConfiguration(7))
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
            configuration: &Self::Configuration,
            _: AttemptId,
            _: ApplyPermit,
        ) {
            self.applied_configurations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(configuration.0);
        }

        fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
            AttemptProgress::Pending
        }
    }

    #[derive(Debug, Default, PartialEq, Eq)]
    struct DriverCallLog {
        captures:               usize,
        applied_configurations: Vec<u8>,
        polls:                  usize,
    }

    struct CallCountingDriver {
        driver_call_log: Arc<Mutex<DriverCallLog>>,
    }

    impl EndpointDriver for CallCountingDriver {
        type Configuration = TestConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            self.driver_call_log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .captures += 1;
            CaptureOutcome::Read(TestConfiguration(7))
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
            configuration: &Self::Configuration,
            _: AttemptId,
            _: ApplyPermit,
        ) {
            self.driver_call_log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .applied_configurations
                .push(configuration.0);
        }

        fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
            self.driver_call_log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .polls += 1;
            AttemptProgress::Pending
        }
    }

    #[derive(Component, Reflect)]
    struct MismatchedConfiguration;

    struct MismatchedDriver;

    impl EndpointDriver for MismatchedDriver {
        type Configuration = MismatchedConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            CaptureOutcome::NotReadable
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
            _: &Self::Configuration,
            _: AttemptId,
            _: ApplyPermit,
        ) {
        }

        fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
            AttemptProgress::Pending
        }
    }

    #[test]
    fn duplicate_role_and_endpoint_registration_preserve_the_first_binding()
    -> Result<(), Box<dyn Error>> {
        let endpoint = display_endpoint("studio-display")?;
        let first_role = RoleKey::new("primary-window")?;
        let second_role = RoleKey::new("secondary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(first_role.clone(), endpoint.clone()))?;

        assert!(matches!(
            bindings.register(binding(first_role.clone(), display_endpoint("other-display")?)),
            Err(BindingError::RoleAlreadyBound { role }) if role == first_role
        ));
        assert!(matches!(
            bindings.register(binding(second_role, endpoint)),
            Err(BindingError::EndpointAlreadyOwned { owner, .. }) if owner == first_role
        ));
        assert_eq!(
            bindings.roles_for(&device_key("studio-display")?).count(),
            1
        );
        assert!(bindings.binding(&first_role).is_ok());

        Ok(())
    }

    #[test]
    fn failed_replace_keeps_each_existing_reverse_index() -> Result<(), Box<dyn Error>> {
        let first_role = RoleKey::new("primary-window")?;
        let second_role = RoleKey::new("secondary-window")?;
        let first_endpoint = display_endpoint("studio-display")?;
        let second_endpoint = display_endpoint("edit-display")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(first_role.clone(), first_endpoint.clone()))?;
        bindings.register(binding(second_role.clone(), second_endpoint.clone()))?;

        assert!(matches!(
            bindings.replace(binding(first_role.clone(), second_endpoint.clone())),
            Err(BindingError::EndpointAlreadyOwned { owner, .. }) if owner == second_role
        ));
        assert_eq!(bindings.binding(&first_role)?.endpoint, first_endpoint);
        assert_eq!(bindings.binding(&second_role)?.endpoint, second_endpoint);
        assert_eq!(
            bindings.roles_for(&device_key("studio-display")?).count(),
            1
        );
        assert_eq!(bindings.roles_for(&device_key("edit-display")?).count(), 1);

        Ok(())
    }

    #[test]
    fn successful_replace_releases_only_its_old_endpoint() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let old_endpoint = display_endpoint("studio-display")?;
        let new_endpoint = display_endpoint("edit-display")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), old_endpoint.clone()))?;

        let displaced = bindings.replace(binding(role.clone(), new_endpoint.clone()))?;

        assert_eq!(displaced.endpoint, old_endpoint);
        assert_eq!(bindings.binding(&role)?.endpoint, new_endpoint);
        assert_eq!(
            bindings.roles_for(&device_key("studio-display")?).count(),
            0
        );
        assert_eq!(bindings.roles_for(&device_key("edit-display")?).count(), 1);

        Ok(())
    }

    #[test]
    fn retirement_is_idempotent_and_removes_every_index() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let endpoint = display_endpoint("studio-display")?;
        let device_key = endpoint.device.clone();
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), endpoint))?;

        let retirement = bindings.retire(&role)?;

        assert!(matches!(
            retirement,
            RetirementOutcome::Retired(Binding {
                state: RoleState::Retired,
                ..
            })
        ));
        assert!(matches!(
            bindings.retire(&role)?,
            RetirementOutcome::AlreadyUnbound
        ));
        assert!(matches!(
            bindings.binding(&role),
            Err(BindingError::RoleNotBound { .. })
        ));
        assert_eq!(bindings.roles_for(&device_key).count(), 0);

        Ok(())
    }

    #[test]
    fn one_device_can_serve_several_roles_at_distinct_endpoints() -> Result<(), Box<dyn Error>> {
        let device_key = device_key("control-panel")?;
        let first_role = RoleKey::new("cut")?;
        let second_role = RoleKey::new("fade")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(
            first_role,
            DeviceEndpoint {
                device: device_key.clone(),
                id:     EndpointId::Part(crate::PartName::new("key/1")?),
            },
        ))?;
        bindings.register(binding(
            second_role,
            DeviceEndpoint {
                device: device_key.clone(),
                id:     EndpointId::Part(crate::PartName::new("key/2")?),
            },
        ))?;

        assert_eq!(bindings.roles_for(&device_key).count(), 2);

        Ok(())
    }

    #[test]
    fn transitions_are_monotonic_and_hold_only_lifecycle_metadata() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;
        bindings.replace(binding(role.clone(), display_endpoint("edit-display")?))?;
        let _ = bindings.retire(&role)?;
        let _ = bindings.retire(&role)?;

        let transitions = bindings.take_pending_transitions();
        let sequences = transitions
            .iter()
            .map(|binding_transition| match binding_transition {
                BindingTransition::Registered {
                    sequence,
                    role: transition_role,
                }
                | BindingTransition::Replaced {
                    sequence,
                    role: transition_role,
                }
                | BindingTransition::Retired {
                    sequence,
                    role: transition_role,
                    ..
                } => {
                    assert_eq!(transition_role, &role);
                    sequence.0
                },
            })
            .collect::<Vec<_>>();

        assert_eq!(sequences, vec![0, 1, 2]);

        Ok(())
    }

    #[test]
    fn configured_transition_capacity_keeps_register_replace_and_retire_atomic()
    -> Result<(), Box<dyn Error>> {
        let first_role = RoleKey::new("primary-window")?;
        let second_role = RoleKey::new("secondary-window")?;
        let third_role = RoleKey::new("tertiary-window")?;
        let first_endpoint = display_endpoint("studio-display")?;
        let second_endpoint = display_endpoint("edit-display")?;
        let third_endpoint = display_endpoint("presentation-display")?;
        let replacement_endpoint = display_endpoint("replacement-display")?;
        let mut bindings = Bindings::default();
        let capacity = NonZeroUsize::new(2).ok_or("nonzero capacity")?;
        bindings.set_pending_transition_capacity(capacity)?;
        bindings.register(binding(first_role.clone(), first_endpoint.clone()))?;
        bindings.register(binding(second_role.clone(), second_endpoint.clone()))?;

        assert_eq!(
            bindings.register(binding(third_role.clone(), third_endpoint.clone())),
            Err(BindingError::PendingTransitionCapacityReached)
        );
        assert!(matches!(
            bindings.replace(binding(first_role.clone(), replacement_endpoint.clone())),
            Err(BindingError::PendingTransitionCapacityReached)
        ));
        assert!(matches!(
            bindings.retire(&first_role),
            Err(BindingError::PendingTransitionCapacityReached)
        ));
        assert_eq!(bindings.binding(&first_role)?.endpoint, first_endpoint);
        assert_eq!(bindings.binding(&second_role)?.endpoint, second_endpoint);
        assert!(matches!(
            bindings.binding(&third_role),
            Err(BindingError::RoleNotBound { .. })
        ));
        assert_eq!(
            bindings.owner_by_endpoint.get(&first_endpoint),
            Some(&first_role)
        );
        assert_eq!(
            bindings.owner_by_endpoint.get(&second_endpoint),
            Some(&second_role)
        );
        assert!(!bindings.owner_by_endpoint.contains_key(&third_endpoint));
        assert!(
            !bindings
                .owner_by_endpoint
                .contains_key(&replacement_endpoint)
        );
        assert_eq!(
            bindings.roles_by_device.get(&first_endpoint.device),
            Some(&vec![first_role.clone()])
        );
        assert_eq!(
            bindings.roles_by_device.get(&second_endpoint.device),
            Some(&vec![second_role])
        );
        assert!(
            !bindings
                .roles_by_device
                .contains_key(&third_endpoint.device)
        );
        assert!(
            !bindings
                .roles_by_device
                .contains_key(&replacement_endpoint.device)
        );
        assert_eq!(
            bindings.set_pending_transition_capacity(NonZeroUsize::MIN),
            Err(BindingCapacityError::BelowPendingCount {
                capacity: NonZeroUsize::MIN,
                pending:  2,
            })
        );

        Ok(())
    }

    #[test]
    fn device_readdress_moves_all_endpoint_parts_together() -> Result<(), Box<dyn Error>> {
        let saved = device_key("saved-camera")?;
        let adopted = device_key("adopted-camera")?;
        let first_role = RoleKey::new("camera/first-clone")?;
        let second_role = RoleKey::new("camera/second-clone")?;
        let first_part = EndpointId::Part(PartName::new("tool/1")?);
        let second_part = EndpointId::Part(PartName::new("tool/2")?);
        let mut bindings = Bindings::default();
        bindings.register(binding(
            first_role.clone(),
            DeviceEndpoint {
                device: saved.clone(),
                id:     first_part.clone(),
            },
        ))?;
        bindings.register(binding(
            second_role.clone(),
            DeviceEndpoint {
                device: saved.clone(),
                id:     second_part.clone(),
            },
        ))?;

        bindings.validate_device_readdress(&saved, &adopted)?;
        bindings.readdress_device(&saved, adopted.clone())?;

        assert_eq!(
            bindings.binding(&first_role)?.endpoint,
            DeviceEndpoint {
                device: adopted.clone(),
                id:     first_part,
            }
        );
        assert_eq!(
            bindings.binding(&second_role)?.endpoint,
            DeviceEndpoint {
                device: adopted.clone(),
                id:     second_part,
            }
        );
        assert_eq!(
            bindings.roles_for(&saved).collect::<Vec<_>>(),
            Vec::<&RoleKey>::new()
        );
        assert_eq!(bindings.roles_for(&adopted).count(), 2);
        Ok(())
    }

    #[test]
    fn device_readdress_capacity_refusal_changes_no_role_or_index() -> Result<(), Box<dyn Error>> {
        let saved = device_key("saved-camera")?;
        let adopted = device_key("adopted-camera")?;
        let first_role = RoleKey::new("camera/first-clone")?;
        let second_role = RoleKey::new("camera/second-clone")?;
        let first_endpoint = DeviceEndpoint {
            device: saved.clone(),
            id:     EndpointId::Part(PartName::new("tool/1")?),
        };
        let second_endpoint = DeviceEndpoint {
            device: saved.clone(),
            id:     EndpointId::Part(PartName::new("tool/2")?),
        };
        let mut bindings = Bindings::default();
        bindings.register(binding(first_role.clone(), first_endpoint.clone()))?;
        bindings.register(binding(second_role.clone(), second_endpoint.clone()))?;
        bindings.set_pending_transition_capacity(
            NonZeroUsize::new(3).ok_or("nonzero transition capacity")?,
        )?;

        assert_eq!(
            bindings.validate_device_readdress(&saved, &adopted),
            Err(BindingError::PendingTransitionCapacityReached)
        );
        assert_eq!(
            bindings.readdress_device(&saved, adopted.clone()),
            Err(BindingError::PendingTransitionCapacityReached)
        );
        assert_eq!(bindings.binding(&first_role)?.endpoint, first_endpoint);
        assert_eq!(bindings.binding(&second_role)?.endpoint, second_endpoint);
        assert_eq!(bindings.roles_for(&saved).count(), 2);
        assert_eq!(bindings.roles_for(&adopted).count(), 0);
        Ok(())
    }

    #[test]
    fn default_transition_capacity_rejects_another_registration_without_index_mutation()
    -> Result<(), Box<dyn Error>> {
        let mut bindings = Bindings::default();

        for index in 0..super::DEFAULT_PENDING_TRANSITION_CAPACITY {
            let role = RoleKey::new(format!("default-capacity-role-{index}"))?;
            let endpoint = display_endpoint(&format!("default-capacity-device-{index}"))?;
            bindings.register(binding(role, endpoint))?;
        }

        let overflow_role = RoleKey::new("default-capacity-overflow")?;
        let overflow_endpoint = display_endpoint("default-capacity-overflow-device")?;
        assert_eq!(
            bindings.register(binding(overflow_role.clone(), overflow_endpoint.clone())),
            Err(BindingError::PendingTransitionCapacityReached)
        );
        assert!(matches!(
            bindings.binding(&overflow_role),
            Err(BindingError::RoleNotBound { .. })
        ));
        assert!(!bindings.owner_by_endpoint.contains_key(&overflow_endpoint));
        assert!(
            !bindings
                .roles_by_device
                .contains_key(&overflow_endpoint.device)
        );
        assert_eq!(
            bindings.pending_transitions.queue.len(),
            super::DEFAULT_PENDING_TRANSITION_CAPACITY
        );

        Ok(())
    }

    #[test]
    fn transition_sequence_exhaustion_keeps_all_binding_indexes_unchanged()
    -> Result<(), Box<dyn Error>> {
        let first_role = RoleKey::new("last-sequence-role")?;
        let first_endpoint = display_endpoint("last-sequence-device")?;
        let second_role = RoleKey::new("exhausted-sequence-role")?;
        let second_endpoint = display_endpoint("exhausted-sequence-device")?;
        let mut bindings = Bindings {
            next_transition_sequence: u64::MAX - 1,
            ..Default::default()
        };

        bindings.register(binding(first_role.clone(), first_endpoint.clone()))?;
        assert!(matches!(
            bindings.pending_transitions.queue.front(),
            Some(BindingTransition::Registered { sequence, role })
                if sequence.0 == u64::MAX - 1 && role == &first_role
        ));
        assert_eq!(bindings.next_transition_sequence, u64::MAX);

        assert_eq!(
            bindings.register(binding(second_role.clone(), second_endpoint.clone())),
            Err(BindingError::TransitionSequenceExhausted)
        );
        assert_eq!(bindings.binding(&first_role)?.endpoint, first_endpoint);
        assert!(matches!(
            bindings.binding(&second_role),
            Err(BindingError::RoleNotBound { .. })
        ));
        assert_eq!(
            bindings.owner_by_endpoint.get(&first_endpoint),
            Some(&first_role)
        );
        assert!(!bindings.owner_by_endpoint.contains_key(&second_endpoint));
        assert_eq!(
            bindings.roles_by_device.get(&first_endpoint.device),
            Some(&vec![first_role])
        );
        assert!(
            !bindings
                .roles_by_device
                .contains_key(&second_endpoint.device)
        );
        assert_eq!(bindings.next_transition_sequence, u64::MAX);

        Ok(())
    }

    #[test]
    fn in_service_apply_keeps_requested_and_readback_configuration_distinct()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        let hardware_inventory = HardwareInventory::default();
        let applied_configurations = Arc::new(Mutex::new(Vec::new()));
        let mut drivers = Drivers::new();
        let driver = drivers.add(RecordingDriver {
            applied_configurations: Arc::clone(&applied_configurations),
        });
        assert_eq!(driver, DriverId(0));
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.last_known_good =
            LastKnownGoodConfiguration::known(TestConfiguration(1));
        bindings.register(configured_binding)?;

        assert!(matches!(bindings.role_view(&role)?, RoleView::Waiting(_)));
        assert!(matches!(
            match bindings.role_view(&role)? {
                RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                    .start_requested_apply(
                        AttemptId::default(),
                        ApplyPermit::restore_only(),
                        &hardware_inventory,
                    ),
                _ => return Err("new binding must select waiting view".into()),
            },
            Err(BindingError::RequestedConfigurationRequiresInServicePermit)
        ));
        {
            let apply_request = match bindings.role_view(&role)? {
                RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                    .start_requested_apply(
                        AttemptId::default(),
                        ApplyPermit::in_service(),
                        &hardware_inventory,
                    )?,
                _ => return Err("new binding must select waiting view".into()),
            };
            assert_eq!(
                drivers.start_apply(&mut World::new(), apply_request),
                Ok(())
            );
        }
        assert_eq!(
            *applied_configurations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![3]
        );
        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => {
                applying_role.finish(AttemptOutcome::Succeeded);
            },
            _ => return Err("apply request must select applying view".into()),
        }
        match bindings.role_view(&role)? {
            RoleView::Ready(mut ready_role) => {
                ready_role.record_capture(CaptureOutcome::Read(LastKnownGoodConfiguration::known(
                    TestConfiguration(7),
                )));
            },
            _ => return Err("successful apply must select ready view".into()),
        }

        match bindings.configuration_for(&role)? {
            AvailableConfiguration::LastKnownGood(configuration) => {
                assert_eq!(
                    configuration
                        .as_any()
                        .downcast_ref::<TestConfiguration>()
                        .map(|test_configuration| test_configuration.0),
                    Some(7)
                );
            },
            AvailableConfiguration::Requested(_) => {
                return Err("safe readback must take precedence over requested intent".into());
            },
        }

        Ok(())
    }

    #[test]
    fn restore_only_apply_uses_last_known_good_configuration() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let applied_configurations = Arc::new(Mutex::new(Vec::new()));
        let mut drivers = Drivers::new();
        let driver = drivers.add(RecordingDriver {
            applied_configurations: Arc::clone(&applied_configurations),
        });
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.driver = driver;
        configured_binding.last_known_good =
            LastKnownGoodConfiguration::known(TestConfiguration(7));
        let mut bindings = Bindings::default();
        bindings.register(configured_binding)?;
        bindings.set_waiting_work(&role, WaitingWork::RestorationOwed);

        assert!(matches!(
            match bindings.role_view(&role)? {
                RoleView::Waiting(WaitingRole::Restoration(restoring_role)) => restoring_role
                    .start_last_known_good_restore(
                        AttemptId::default(),
                        ApplyPermit::in_service(),
                        &hardware_inventory,
                    ),
                _ => return Err("registered binding must select waiting view".into()),
            },
            Err(BindingError::LastKnownGoodConfigurationRequiresRestoreOnlyPermit)
        ));

        let apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Restoration(restoring_role)) => restoring_role
                .start_last_known_good_restore(
                    AttemptId::default(),
                    ApplyPermit::restore_only(),
                    &hardware_inventory,
                )?,
            _ => return Err("restore authorization failure must retain waiting state".into()),
        };
        assert_eq!(
            drivers.start_apply(&mut World::new(), apply_request),
            Ok(())
        );
        assert_eq!(
            *applied_configurations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![7]
        );
        assert!(matches!(bindings.role_view(&role)?, RoleView::Applying(_)));

        Ok(())
    }

    #[test]
    fn dropped_start_apply_request_leaves_the_binding_waiting() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        let _ = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?,
            _ => return Err("registered binding must select waiting view".into()),
        };

        assert!(matches!(bindings.role_view(&role)?, RoleView::Waiting(_)));

        Ok(())
    }

    #[test]
    fn unregistered_driver_dispatch_leaves_the_binding_waiting() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;
        let apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?,
            _ => return Err("registered binding must select waiting view".into()),
        };

        let start_apply_result = Drivers::new().start_apply(&mut World::new(), apply_request);

        assert!(matches!(
            start_apply_result,
            Err(DriverContractError::DriverNotRegistered { .. })
        ));
        assert!(matches!(bindings.role_view(&role)?, RoleView::Waiting(_)));

        Ok(())
    }

    #[test]
    fn type_mismatch_dispatch_leaves_the_binding_waiting() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut bindings = Bindings::default();
        let mut drivers = Drivers::new();
        let driver = drivers.add(MismatchedDriver);
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.driver = driver;
        bindings.register(configured_binding)?;
        let apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?,
            _ => return Err("registered binding must select waiting view".into()),
        };

        let start_apply_result = drivers.start_apply(&mut World::new(), apply_request);

        assert!(matches!(
            start_apply_result,
            Err(DriverContractError::ConfigurationTypeMismatch { .. })
        ));
        assert!(matches!(bindings.role_view(&role)?, RoleView::Waiting(_)));

        Ok(())
    }

    #[test]
    fn substituted_apply_returns_the_role_to_ready_for_safe_readback() -> Result<(), Box<dyn Error>>
    {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut drivers = Drivers::new();
        let driver = drivers.add(RecordingDriver {
            applied_configurations: Arc::new(Mutex::new(Vec::new())),
        });
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.driver = driver;
        let mut bindings = Bindings::default();
        bindings.register(configured_binding)?;
        let apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?,
            _ => return Err("registered binding must select waiting view".into()),
        };
        drivers.start_apply(&mut World::new(), apply_request)?;

        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => {
                applying_role.finish(AttemptOutcome::Substituted);
            },
            _ => return Err("dispatched apply must select applying view".into()),
        }

        assert!(matches!(bindings.role_view(&role)?, RoleView::Ready(_)));

        Ok(())
    }

    #[test]
    fn aborting_an_dispatched_apply_returns_the_role_to_waiting() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut drivers = Drivers::new();
        let driver = drivers.add(RecordingDriver {
            applied_configurations: Arc::new(Mutex::new(Vec::new())),
        });
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.driver = driver;
        let mut bindings = Bindings::default();
        bindings.register(configured_binding)?;
        let apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?,
            _ => return Err("registered binding must select waiting view".into()),
        };
        drivers.start_apply(&mut World::new(), apply_request)?;

        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => applying_role.abort(),
            _ => return Err("dispatched apply must select applying view".into()),
        }

        assert!(matches!(bindings.role_view(&role)?, RoleView::Waiting(_)));

        Ok(())
    }

    #[test]
    fn a_completed_restoration_settles_the_debt_and_a_failed_one_keeps_it()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut drivers = Drivers::new();
        let driver = drivers.add(RecordingDriver {
            applied_configurations: Arc::new(Mutex::new(Vec::new())),
        });
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.driver = driver;
        configured_binding.last_known_good =
            LastKnownGoodConfiguration::known(TestConfiguration(7));
        let mut bindings = Bindings::default();
        bindings.register(configured_binding)?;
        bindings.set_waiting_work(&role, WaitingWork::RestorationOwed);

        let restore_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Restoration(restoring_role)) => restoring_role
                .start_last_known_good_restore(
                    AttemptId::default(),
                    ApplyPermit::restore_only(),
                    &hardware_inventory,
                )?,
            _ => return Err("a role owing a restoration selects the restoring view".into()),
        };
        drivers.start_apply(&mut World::new(), restore_request)?;
        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => {
                applying_role.finish(AttemptOutcome::Failed(DeviceAccessError::Contended {
                    detail: String::from("another owner holds the display"),
                }));
            },
            _ => return Err("a dispatched restore selects the applying view".into()),
        }

        // The restoration did not land, so the endpoint still does not hold the saved value and the
        // debt is what dispatches the next attempt.
        assert_eq!(bindings.waiting_work(&role), WaitingWork::RestorationOwed);

        let retried_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Restoration(restoring_role)) => restoring_role
                .start_last_known_good_restore(
                    AttemptId::default(),
                    ApplyPermit::restore_only(),
                    &hardware_inventory,
                )?,
            _ => return Err("a failed restore leaves the role owing one".into()),
        };
        drivers.start_apply(&mut World::new(), retried_request)?;
        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => {
                applying_role.finish(AttemptOutcome::Succeeded);
            },
            _ => return Err("a dispatched restore selects the applying view".into()),
        }

        // The saved value is on the endpoint again, so nothing is owed: the role is a safe readback
        // opportunity once more rather than one that re-restores on every later pass.
        assert_eq!(bindings.waiting_work(&role), WaitingWork::Nothing);
        assert!(matches!(bindings.role_view(&role)?, RoleView::Ready(_)));

        Ok(())
    }

    #[test]
    fn an_ordinary_apply_completing_under_an_owed_restoration_leaves_the_debt_owed()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut drivers = Drivers::new();
        let driver = drivers.add(RecordingDriver {
            applied_configurations: Arc::new(Mutex::new(Vec::new())),
        });
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.driver = driver;
        configured_binding.last_known_good =
            LastKnownGoodConfiguration::known(TestConfiguration(7));
        let mut bindings = Bindings::default();
        bindings.register(configured_binding)?;

        // The role owes nothing yet, so what it mints is the authored request.
        let apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?,
            _ => return Err("a role owing nothing selects the requesting view".into()),
        };
        drivers.start_apply(&mut World::new(), apply_request)?;

        // The device departs mid-flight, which is what records the debt: the recovery rule reads
        // policy and last-known-good, not role state, so it lands on a role already applying.
        bindings.set_waiting_work(&role, WaitingWork::RestorationOwed);
        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => {
                applying_role.finish(AttemptOutcome::Succeeded);
            },
            _ => return Err("a dispatched requested apply selects the applying view".into()),
        }

        // The apply that landed carried the authored request, so the saved value is still not back
        // on the endpoint and the restoration is still what the next authorized pass must run.
        assert_eq!(bindings.waiting_work(&role), WaitingWork::RestorationOwed);

        Ok(())
    }

    #[test]
    fn read_failure_retains_prior_value_and_not_readable_stops_future_requests()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        let hardware_inventory = HardwareInventory::default();
        let mut drivers = Drivers::new();
        let driver = drivers.add(RecordingDriver {
            applied_configurations: Arc::new(Mutex::new(Vec::new())),
        });
        assert_eq!(driver, DriverId(0));
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.state = RoleState::Ready;
        configured_binding.last_known_good =
            LastKnownGoodConfiguration::known(TestConfiguration(7));
        bindings.register(configured_binding)?;

        match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => {
                let apply_request = requesting_role.start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?;
                assert_eq!(
                    drivers.start_apply(&mut World::new(), apply_request),
                    Ok(())
                );
            },
            _ => return Err("registration resets role state to waiting".into()),
        }
        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => {
                applying_role.finish(AttemptOutcome::Succeeded);
            },
            _ => return Err("requested operation must select applying view".into()),
        }
        match bindings.role_view(&role)? {
            RoleView::Ready(mut ready_role) => {
                ready_role.record_capture(CaptureOutcome::ReadFailed(DeviceAccessError::Absent {
                    detail: String::from("test departure"),
                }));
                ready_role.record_capture(CaptureOutcome::NotReadable);
            },
            _ => return Err("successful operation must select ready view".into()),
        }

        match bindings.configuration_for(&role)? {
            AvailableConfiguration::LastKnownGood(configuration) => assert_eq!(
                configuration
                    .as_any()
                    .downcast_ref::<TestConfiguration>()
                    .map(|test_configuration| test_configuration.0),
                Some(7)
            ),
            AvailableConfiguration::Requested(_) => {
                return Err("read failure must retain prior known value".into());
            },
        }
        match bindings.role_view(&role)? {
            RoleView::Ready(ready_role) => assert!(matches!(
                ready_role.capture_request(&hardware_inventory),
                Err(BindingError::ConfigurationNotReadable { .. })
            )),
            _ => return Err("readability test requires ready view".into()),
        }

        Ok(())
    }

    #[test]
    fn stored_waiting_work_selects_the_only_request_a_waiting_role_is_owed()
    -> Result<(), Box<dyn Error>> {
        let owing_nothing = RoleKey::new("primary-window")?;
        let owing_restoration = RoleKey::new("secondary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut bindings = Bindings::default();
        bindings.register(binding(
            owing_nothing.clone(),
            display_endpoint("studio-display")?,
        ))?;
        let mut restoring_binding =
            binding(owing_restoration.clone(), display_endpoint("edit-display")?);
        restoring_binding.last_known_good = LastKnownGoodConfiguration::known(TestConfiguration(7));
        bindings.register(restoring_binding)?;
        bindings.set_waiting_work(&owing_restoration, WaitingWork::RestorationOwed);

        // `RequestingRole` carries no restore method and `RestoringRole` carries no requested-apply
        // method, so selecting the arm is what removes the wrong call rather than refusing it.
        match bindings.role_view(&owing_nothing)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => {
                requesting_role.start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?;
            },
            _ => return Err("a role owing nothing waits for hardware".into()),
        }
        match bindings.role_view(&owing_restoration)? {
            RoleView::Waiting(WaitingRole::Restoration(restoring_role)) => {
                restoring_role.start_last_known_good_restore(
                    AttemptId::default(),
                    ApplyPermit::restore_only(),
                    &hardware_inventory,
                )?;
            },
            _ => return Err("a role owing a restoration waits for that restoration".into()),
        }

        Ok(())
    }

    #[test]
    fn offline_waiting_role_cannot_mint_apply_requests() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let endpoint = display_endpoint("studio-display")?;
        let driver_call_log = Arc::new(Mutex::new(DriverCallLog::default()));
        let mut drivers = Drivers::new();
        let driver = drivers.add(CallCountingDriver {
            driver_call_log: Arc::clone(&driver_call_log),
        });
        let mut bindings = Bindings::default();
        let mut configured_binding = binding(role.clone(), endpoint.clone());
        configured_binding.driver = driver;
        bindings.register(configured_binding)?;
        let mut hardware_inventory = HardwareInventory::default();
        hardware_inventory.configure(ConfiguredDevice {
            key:  endpoint.device.clone(),
            mode: ConfiguredDeviceMode::Offline,
        });

        match bindings.configuration_for(&role)? {
            AvailableConfiguration::Requested(configuration) => assert_eq!(
                configuration
                    .as_any()
                    .downcast_ref::<TestConfiguration>()
                    .map(|test_configuration| test_configuration.0),
                Some(3)
            ),
            AvailableConfiguration::LastKnownGood(_) => {
                return Err("no safe readback has established a configuration".into());
            },
        }
        assert_eq!(
            hardware_inventory.connection(&endpoint.device)?,
            ConfiguredDeviceConnection::NotObserved
        );
        hardware_inventory.set_connection(&endpoint.device, ConfiguredDeviceConnection::Present)?;
        assert_eq!(
            hardware_inventory.connection(&endpoint.device)?,
            ConfiguredDeviceConnection::Present
        );
        match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => assert!(matches!(
                requesting_role.start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                ),
                Err(BindingError::ConfiguredDeviceOffline { .. })
            )),
            _ => return Err("registered offline binding must remain waiting".into()),
        }
        assert!(matches!(bindings.role_view(&role)?, RoleView::Waiting(_)));
        bindings.set_waiting_work(&role, WaitingWork::RestorationOwed);
        match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Restoration(restoring_role)) => assert!(matches!(
                restoring_role.start_last_known_good_restore(
                    AttemptId::default(),
                    ApplyPermit::restore_only(),
                    &hardware_inventory,
                ),
                Err(BindingError::ConfiguredDeviceOffline { .. })
            )),
            _ => return Err("offline requested-apply refusal must retain waiting state".into()),
        }
        assert!(matches!(bindings.role_view(&role)?, RoleView::Waiting(_)));
        assert_eq!(
            *driver_call_log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            DriverCallLog::default()
        );

        Ok(())
    }

    #[test]
    fn offline_inventory_blocks_ready_capture_and_applying_poll_requests()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let endpoint = display_endpoint("studio-display")?;
        let driver_call_log = Arc::new(Mutex::new(DriverCallLog::default()));
        let mut drivers = Drivers::new();
        let driver = drivers.add(CallCountingDriver {
            driver_call_log: Arc::clone(&driver_call_log),
        });
        let mut bindings = Bindings::default();
        let mut configured_binding = binding(role.clone(), endpoint.clone());
        configured_binding.driver = driver;
        bindings.register(configured_binding)?;
        let mut hardware_inventory = HardwareInventory::default();
        hardware_inventory.configure(ConfiguredDevice {
            key:  endpoint.device.clone(),
            mode: ConfiguredDeviceMode::Managed,
        });
        let start_apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?,
            _ => return Err("managed binding must select waiting state before apply".into()),
        };
        drivers.start_apply(&mut World::new(), start_apply_request)?;
        assert!(matches!(bindings.role_view(&role)?, RoleView::Applying(_)));

        hardware_inventory.configure(ConfiguredDevice {
            key:  endpoint.device.clone(),
            mode: ConfiguredDeviceMode::Offline,
        });
        match bindings.role_view(&role)? {
            RoleView::Applying(applying_role) => assert!(matches!(
                applying_role.poll_request(&hardware_inventory),
                Err(BindingError::ConfiguredDeviceOffline { .. })
            )),
            _ => return Err("started apply must select applying state".into()),
        }
        assert!(matches!(bindings.role_view(&role)?, RoleView::Applying(_)));
        assert_eq!(
            *driver_call_log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            DriverCallLog {
                captures:               0,
                applied_configurations: vec![3],
                polls:                  0,
            }
        );

        hardware_inventory.configure(ConfiguredDevice {
            key:  endpoint.device.clone(),
            mode: ConfiguredDeviceMode::Managed,
        });
        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => {
                applying_role.finish(AttemptOutcome::Succeeded);
            },
            _ => return Err("applying role must remain finishable after poll refusal".into()),
        }
        assert!(matches!(bindings.role_view(&role)?, RoleView::Ready(_)));

        hardware_inventory.configure(ConfiguredDevice {
            key:  endpoint.device,
            mode: ConfiguredDeviceMode::Offline,
        });
        match bindings.role_view(&role)? {
            RoleView::Ready(ready_role) => assert!(matches!(
                ready_role.capture_request(&hardware_inventory),
                Err(BindingError::ConfiguredDeviceOffline { .. })
            )),
            _ => return Err("successful apply must select ready state".into()),
        }
        assert!(matches!(bindings.role_view(&role)?, RoleView::Ready(_)));
        assert_eq!(
            *driver_call_log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            DriverCallLog {
                captures:               0,
                applied_configurations: vec![3],
                polls:                  0,
            }
        );

        Ok(())
    }

    #[test]
    fn binding_inventory_and_reflected_configuration_types_register_automatically() {
        let app = App::new();
        let world = app.world();
        let type_registry = world.resource::<AppTypeRegistry>().read();

        for type_id in [
            TypeId::of::<Bindings>(),
            TypeId::of::<HardwareInventory>(),
            TypeId::of::<Binding>(),
            TypeId::of::<ConfiguredDevice>(),
        ] {
            assert!(type_registry.contains(type_id));
        }
        drop(type_registry);
    }

    fn binding(role: RoleKey, endpoint: DeviceEndpoint) -> Binding {
        Binding {
            role,
            endpoint,
            driver: DriverId(0),
            recovery: RecoveryPolicy::default(),
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::Ready,
            requested: RequestedConfiguration::new(TestConfiguration(3)),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline: ApplyDeadline::ProcessDefault,
        }
    }

    fn display_endpoint(value: &str) -> Result<DeviceEndpoint, Box<dyn Error>> {
        Ok(DeviceEndpoint {
            device: device_key(value)?,
            id:     EndpointId::Whole,
        })
    }

    fn device_key(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new(value)?,
            },
        })
    }

    // --- binding entities, the frame batch, and the resolved-device relationship ---

    /// Build an app with the kernel plugin and one authored role bound to a fresh endpoint.
    fn app_with_role(role: &str) -> Result<(App, RoleKey), Box<dyn Error>> {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let role = RoleKey::new(role)?;
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(test_binding(role.clone(), endpoint_named(role.as_str())?))?;

        Ok((app, role))
    }

    fn endpoint_named(value: &str) -> Result<DeviceEndpoint, Box<dyn Error>> {
        Ok(DeviceEndpoint {
            device: DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored {
                    value: AuthoredId::new(value)?,
                },
            },
            id:     EndpointId::Whole,
        })
    }

    fn test_binding(role: RoleKey, endpoint: DeviceEndpoint) -> Binding {
        Binding {
            role,
            endpoint,
            driver: DriverId(0),
            recovery: RecoveryPolicy::Forget,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::default(),
            requested: RequestedConfiguration::new(()),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline: ApplyDeadline::ProcessDefault,
        }
    }

    fn registered_entity(app: &App, role: &RoleKey) -> Entity {
        match app.world().resource::<BindingEntities>().entity(role) {
            BindingEntityLookup::Registered(entity) => entity,
            BindingEntityLookup::Unregistered => {
                panic!("role `{role}` has no binding entity")
            },
        }
    }

    #[derive(Default, Resource)]
    struct ObservedBindingRetirements(Vec<(RoleKey, DeviceEndpoint)>);

    fn observe_binding_retired(
        binding_retired: On<BindingRetired>,
        mut observed: ResMut<ObservedBindingRetirements>,
    ) {
        observed.0.push((
            binding_retired.role.clone(),
            binding_retired.endpoint.clone(),
        ));
    }

    #[test]
    fn registration_spawns_one_binding_entity_per_role_with_no_reporter_running()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        let second_role = RoleKey::new("window/inspector")?;
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(test_binding(
                second_role.clone(),
                endpoint_named(second_role.as_str())?,
            ))?;

        app.update();

        let binding_entities = app.world().resource::<BindingEntities>();
        assert_eq!(binding_entities.count(), 2);
        assert_ne!(
            registered_entity(&app, &role),
            registered_entity(&app, &second_role)
        );
        assert_eq!(
            binding_entities.entity(&RoleKey::new("window/never-registered")?),
            BindingEntityLookup::Unregistered
        );

        Ok(())
    }

    #[test]
    fn a_binding_entity_outlives_every_frame_in_which_its_role_has_no_device()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);

        for _ in 0..4 {
            app.update();
        }

        assert_eq!(registered_entity(&app, &role), entity);
        assert!(app.world().get_entity(entity).is_ok());
        assert_eq!(
            app.world().get::<RoleState>(entity),
            Some(&RoleState::Waiting)
        );

        Ok(())
    }

    #[test]
    fn a_binding_entity_stripped_of_its_mirrors_is_repaired_and_stays_indexed()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);

        // A Bevy Remote Protocol *remove* takes the mirrored components off a live entity, which is
        // what separates this from a despawn: the role is still registered.
        app.world_mut()
            .entity_mut(entity)
            .remove::<(RoleKey, RecoveryPolicy, RoleState)>();
        app.update();

        assert_eq!(registered_entity(&app, &role), entity);
        assert_eq!(app.world().get::<RoleKey>(entity), Some(&role));
        assert_eq!(
            app.world().get::<RecoveryPolicy>(entity),
            Some(&RecoveryPolicy::Forget)
        );
        assert_eq!(
            app.world().get::<RoleState>(entity),
            Some(&RoleState::Waiting)
        );

        Ok(())
    }

    #[test]
    fn retirement_despawns_the_binding_entity_on_a_frame_with_no_reporter_completion()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);
        app.world_mut().resource_mut::<Bindings>().retire(&role)?;

        app.update();

        assert_eq!(
            app.world().resource::<BindingEntities>().entity(&role),
            BindingEntityLookup::Unregistered
        );
        assert!(app.world().get_entity(entity).is_err());

        Ok(())
    }

    #[test]
    fn retirement_announces_the_role_and_endpoint_when_its_queued_transition_applies()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.init_resource::<ObservedBindingRetirements>()
            .add_observer(observe_binding_retired);
        app.update();
        let endpoint = app
            .world()
            .resource::<Bindings>()
            .binding(&role)?
            .endpoint
            .clone();

        app.world_mut().resource_mut::<Bindings>().retire(&role)?;
        assert!(
            app.world()
                .resource::<ObservedBindingRetirements>()
                .0
                .is_empty()
        );

        app.update();

        assert_eq!(
            app.world().resource::<ObservedBindingRetirements>().0,
            vec![(role, endpoint)]
        );
        Ok(())
    }

    #[test]
    fn one_drain_moves_every_pending_transition_in_sequence_and_later_work_waits_a_frame()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        let late_role = RoleKey::new("window/late")?;
        app.world_mut().resource_mut::<Bindings>().retire(&role)?;
        // The batch lives only inside the frame that drained it, so the sequences have to be read
        // from a system rather than from the world once the frame has ended.
        app.init_resource::<ObservedBatches>().add_systems(
            Update,
            observe_batch
                .after(project_binding_entities)
                .before(crate::reconcile::reconcile),
        );

        app.update();

        let sequences: Vec<u64> = app.world().resource::<ObservedBatches>().0[0]
            .iter()
            .map(|sequence| sequence.0)
            .collect();
        assert_eq!(sequences, vec![0, 1]);
        assert!(
            app.world_mut()
                .resource_mut::<Bindings>()
                .take_pending_transitions()
                .is_empty()
        );

        // Submitted after this frame's drain: it stays in `Bindings` until the next frame.
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(test_binding(
                late_role.clone(),
                endpoint_named(late_role.as_str())?,
            ))?;
        assert_eq!(
            app.world().resource::<BindingEntities>().entity(&late_role),
            BindingEntityLookup::Unregistered
        );

        app.update();

        assert_eq!(app.world().resource::<ObservedBatches>().0[1].len(), 1);
        assert!(matches!(
            app.world().resource::<BindingEntities>().entity(&late_role),
            BindingEntityLookup::Registered(_)
        ));

        Ok(())
    }

    #[derive(Default, Resource)]
    struct FramesWithChangedBindings(usize);

    fn count_frames_with_changed_bindings(
        bindings: Res<Bindings>,
        mut frames_with_changed_bindings: ResMut<FramesWithChangedBindings>,
    ) {
        if bindings.is_changed() {
            frames_with_changed_bindings.0 += 1;
        }
    }

    #[test]
    fn a_frame_with_no_submitted_binding_operation_leaves_bindings_unchanged()
    -> Result<(), Box<dyn Error>> {
        let (mut app, _) = app_with_role("window/main")?;
        app.init_resource::<FramesWithChangedBindings>()
            .add_systems(
                Update,
                count_frames_with_changed_bindings.after(drain_binding_transitions),
            );

        app.update();

        assert_eq!(app.world().resource::<FramesWithChangedBindings>().0, 1);

        for _ in 0..3 {
            app.update();
        }

        // The drain took nothing on those frames, so it never asked `Bindings` for mutable access.
        assert_eq!(app.world().resource::<FramesWithChangedBindings>().0, 1);

        Ok(())
    }

    #[derive(Default, Resource)]
    struct ObservedBatches(Vec<Vec<BindingTransitionSequence>>);

    fn observe_batch(
        binding_transition_batch: Res<BindingTransitionBatch>,
        mut observed_batches: ResMut<ObservedBatches>,
    ) {
        observed_batches.0.push(
            binding_transition_batch
                .transitions()
                .iter()
                .map(|binding_transition| match binding_transition {
                    BindingTransition::Registered { sequence, .. }
                    | BindingTransition::Replaced { sequence, .. }
                    | BindingTransition::Retired { sequence, .. } => *sequence,
                })
                .collect(),
        );
    }

    #[test]
    fn entity_lifecycle_attempts_and_events_observe_one_identical_ordered_batch()
    -> Result<(), Box<dyn Error>> {
        let (mut app, _) = app_with_role("window/main")?;
        // Three stand-ins for the binding-entity stage, the attempt aborts, and the event stage:
        // each reads the batch after the drain and none of them removes an entry.
        app.init_resource::<ObservedBatches>().add_systems(
            Update,
            (observe_batch, observe_batch, observe_batch)
                .chain()
                .after(project_binding_entities)
                .before(crate::reconcile::reconcile),
        );

        app.update();

        let observed_batches = app.world().resource::<ObservedBatches>();
        assert_eq!(observed_batches.0.len(), 3);
        assert!(
            observed_batches
                .0
                .iter()
                .all(|observed| observed == &observed_batches.0[0])
        );
        assert_eq!(observed_batches.0[0].len(), 1);
        // The batch survives every consumer inside the frame and is emptied by the clearing system
        // ordered after `crate::RiggingSystems::Apply`, so no later frame reads a stale transition.
        assert!(
            app.world()
                .resource::<BindingTransitionBatch>()
                .transitions()
                .is_empty()
        );

        Ok(())
    }

    #[test]
    fn a_reflection_write_to_the_mirrored_recovery_policy_is_overwritten_next_reconcile()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);
        assert_eq!(
            app.world().get::<RecoveryPolicy>(entity),
            Some(&RecoveryPolicy::Forget)
        );

        // What a Bevy Remote Protocol mutation does: write the mirrored component directly.
        *app.world_mut()
            .get_mut::<RecoveryPolicy>(entity)
            .expect("the binding entity mirrors its recovery policy") =
            RecoveryPolicy::ReapplyOnReturn;

        app.update();

        assert_eq!(
            app.world().get::<RecoveryPolicy>(entity),
            Some(&RecoveryPolicy::Forget)
        );
        assert_eq!(
            app.world().resource::<Bindings>().binding(&role)?.recovery,
            RecoveryPolicy::Forget
        );

        Ok(())
    }

    #[test]
    fn resolving_and_replacing_the_link_maintains_the_device_reverse_collection()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);
        let first_device = app.world_mut().spawn_empty().id();
        let second_device = app.world_mut().spawn_empty().id();

        app.world_mut()
            .entity_mut(entity)
            .insert(<ResolvedToDevice as Relationship>::from(first_device));

        assert_eq!(
            resolved_binding_entities(app.world(), first_device),
            vec![entity]
        );

        app.world_mut()
            .entity_mut(entity)
            .insert(<ResolvedToDevice as Relationship>::from(second_device));

        assert!(resolved_binding_entities(app.world(), first_device).is_empty());
        assert_eq!(
            resolved_binding_entities(app.world(), second_device),
            vec![entity]
        );

        Ok(())
    }

    fn resolved_binding_entities(world: &World, device: Entity) -> Vec<Entity> {
        world
            .get::<ResolvedBindings>(device)
            .map(|resolved_bindings| resolved_bindings.iter().collect())
            .unwrap_or_default()
    }

    #[test]
    fn despawning_a_live_device_removes_the_link_and_leaves_its_binding_entities_alive()
    -> Result<(), Box<dyn Error>> {
        let (mut app, role) = app_with_role("window/main")?;
        app.update();
        let entity = registered_entity(&app, &role);
        let device = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(entity)
            .insert(<ResolvedToDevice as Relationship>::from(device));

        app.world_mut().entity_mut(device).despawn();

        assert!(app.world().get_entity(entity).is_ok());
        assert!(app.world().get::<ResolvedToDevice>(entity).is_none());
        assert_eq!(
            app.world().resource::<BindingEntities>().entity(&role),
            BindingEntityLookup::Registered(entity)
        );
        assert!(app.world().resource::<Bindings>().binding(&role).is_ok());

        Ok(())
    }

    #[test]
    fn two_roles_on_one_device_share_a_reverse_collection_while_duplicates_stay_rejected()
    -> Result<(), Box<dyn Error>> {
        let device_key = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("stream-deck")?,
            },
        };
        let key_endpoint = DeviceEndpoint {
            device: device_key.clone(),
            id:     EndpointId::Part(PartName::new("key/3")?),
        };
        let dial_endpoint = DeviceEndpoint {
            device: device_key,
            id:     EndpointId::Part(PartName::new("dial/1")?),
        };
        let key_role = RoleKey::new("deck/key")?;
        let dial_role = RoleKey::new("deck/dial")?;
        let duplicate_role = RoleKey::new("deck/duplicate")?;
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        {
            let mut bindings = app.world_mut().resource_mut::<Bindings>();
            bindings.register(test_binding(key_role.clone(), key_endpoint.clone()))?;
            bindings.register(test_binding(dial_role.clone(), dial_endpoint))?;
            assert!(matches!(
                bindings.register(test_binding(duplicate_role, key_endpoint)),
                Err(BindingError::EndpointAlreadyOwned { .. })
            ));
        }

        app.update();

        let device = app.world_mut().spawn_empty().id();
        for role in [&key_role, &dial_role] {
            let entity = registered_entity(&app, role);
            app.world_mut()
                .entity_mut(entity)
                .insert(<ResolvedToDevice as Relationship>::from(device));
        }

        let resolved = resolved_binding_entities(app.world(), device);
        assert_eq!(resolved.len(), 2);
        assert!(resolved.contains(&registered_entity(&app, &key_role)));
        assert!(resolved.contains(&registered_entity(&app, &dial_role)));

        Ok(())
    }

    #[test]
    fn binding_entity_components_register_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();

        for type_id in [
            TypeId::of::<RoleKey>(),
            TypeId::of::<RecoveryPolicy>(),
            TypeId::of::<RoleState>(),
            TypeId::of::<ResolvedToDevice>(),
            TypeId::of::<ResolvedBindings>(),
        ] {
            assert!(type_registry.contains(type_id));
            assert!(
                type_registry
                    .get_type_data::<ReflectComponent>(type_id)
                    .is_some()
            );
        }

        drop(type_registry);
    }

    /// A different reading of the same role's device, which is what opens a gate waiting on one.
    fn changed(device_revision: crate::DeviceRevisionLookup) -> crate::DeviceRevisionLookup {
        match device_revision {
            DeviceRevisionLookup::Retired => {
                DeviceRevisionLookup::Retained(crate::DeviceRevision::default())
            },
            DeviceRevisionLookup::Retained(device_revision) => {
                DeviceRevisionLookup::Retained(device_revision.advanced())
            },
        }
    }

    fn blocked() -> DeviceAccessError {
        DeviceAccessError::Blocked {
            detail: "the platform refused access".to_owned(),
        }
    }

    fn measurable() -> FrameClockReading {
        FrameClockReading::Measurable(bevy::platform::time::Instant::now())
    }

    /// Read the generation the role's binding currently holds, as an ending dispatched under that
    /// binding would carry it.
    fn generation_now(bindings: &Bindings, role: &RoleKey) -> super::BindingGeneration {
        bindings
            .generation(role)
            .expect("the test registered a binding for this role")
    }

    #[test]
    fn an_aborted_attempt_gates_its_retry_without_counting_toward_escalation()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            AttemptOutcome::Aborted,
            device_revision,
            measurable(),
        );
        // The abort is terminal for this frame: the same revision that invalidated the attempt
        // cannot also open its retry, so the dispatch later in this very chain finds no work.
        assert!(
            !bindings
                .retry_pacing(&role)
                .permits_dispatch(device_revision, measurable())
        );
        assert!(
            bindings
                .retry_pacing(&role)
                .permits_dispatch(changed(device_revision), measurable())
        );
        // Three aborts in a row still leave the role dispatchable: only failures escalate.
        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                AttemptOutcome::Aborted,
                device_revision,
                measurable(),
            );
        }
        assert_eq!(bindings.binding(&role)?.state, RoleState::Waiting);

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            AttemptOutcome::Failed(blocked()),
            device_revision,
            measurable(),
        );
        let super::RetryPacing::AwaitingGate(retry_gate) = bindings.retry_pacing(&role) else {
            return Err("a failed attempt under RetryOn::NewRevision must install a gate".into());
        };
        assert!(!retry_gate.opened(device_revision, measurable()));
        assert!(retry_gate.opened(changed(device_revision), measurable()));

        Ok(())
    }

    #[test]
    fn an_interval_retry_policy_waits_on_the_clock_rather_than_on_a_new_revision()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.retry = RetryOn::Interval(std::time::Duration::from_hours(1));
        bindings.register(configured_binding)?;

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            AttemptOutcome::Failed(blocked()),
            device_revision,
            measurable(),
        );

        // A new revision does not shorten an interval: the two policies measure different things.
        assert!(
            !bindings
                .retry_pacing(&role)
                .permits_dispatch(changed(device_revision), measurable())
        );

        Ok(())
    }

    #[test]
    fn three_consecutive_failures_stop_dispatch_until_a_restart_or_a_success()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        let mut device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                AttemptOutcome::Failed(blocked()),
                device_revision,
                measurable(),
            );
            device_revision = changed(device_revision);
            assert_eq!(bindings.binding(&role)?.state, RoleState::Waiting);
        }
        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            AttemptOutcome::Failed(blocked()),
            device_revision,
            measurable(),
        );

        assert_eq!(
            bindings.binding(&role)?.state,
            RoleState::StoppedAfterRepeatedFailures
        );
        // A stopped role selects no waiting view, so no fourth attempt can be dispatched.
        assert!(matches!(
            bindings.role_view(&role)?,
            RoleView::StoppedAfterRepeatedFailures
        ));

        bindings.restart_after_repeated_failures(&role)?;

        assert_eq!(bindings.binding(&role)?.state, RoleState::Waiting);
        assert_eq!(bindings.retry_pacing(&role), super::RetryPacing::Ready);

        Ok(())
    }

    #[test]
    fn a_stopped_role_waits_for_its_device_to_leave_and_return_before_another_attempt()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;
        for _ in 0..3 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                AttemptOutcome::Failed(blocked()),
                device_revision,
                measurable(),
            );
        }
        assert_eq!(
            bindings.binding(&role)?.state,
            RoleState::StoppedAfterRepeatedFailures
        );

        // A device that never leaves is never retried, however many frames read it as available.
        for _ in 0..3 {
            bindings.observe_stopped_role_endpoint(&role, super::EndpointAvailability::Available);
        }
        assert_eq!(
            bindings.binding(&role)?.state,
            RoleState::StoppedAfterRepeatedFailures
        );

        bindings.observe_stopped_role_endpoint(&role, super::EndpointAvailability::Gone);
        assert_eq!(
            bindings.binding(&role)?.state,
            RoleState::StoppedAfterRepeatedFailures
        );

        bindings.observe_stopped_role_endpoint(&role, super::EndpointAvailability::Available);

        // Reacquired: one more attempt is dispatched, and the run of failures is still standing, so
        // a further failure stops the role again without a second dispatch.
        assert_eq!(bindings.binding(&role)?.state, RoleState::Waiting);
        assert_eq!(bindings.retry_pacing(&role), super::RetryPacing::Ready);

        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            AttemptOutcome::Succeeded,
            device_revision,
            measurable(),
        );

        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                AttemptOutcome::Failed(blocked()),
                device_revision,
                measurable(),
            );
        }
        // The success cleared the run, so two later failures are two and not five.
        assert_eq!(bindings.binding(&role)?.state, RoleState::Waiting);

        Ok(())
    }

    #[test]
    fn a_successful_attempt_clears_the_failures_counted_before_it() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        let device_revision = DeviceRevisionLookup::Retained(crate::DeviceRevision::default());
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                AttemptOutcome::Failed(blocked()),
                device_revision,
                measurable(),
            );
        }
        bindings.record_attempt_ending(
            &role,
            generation_now(&bindings, &role),
            AttemptOutcome::Succeeded,
            device_revision,
            measurable(),
        );
        for _ in 0..2 {
            bindings.record_attempt_ending(
                &role,
                generation_now(&bindings, &role),
                AttemptOutcome::Failed(blocked()),
                device_revision,
                measurable(),
            );
        }

        // Two failures after the recovery is two, not five: the count is consecutive.
        assert_eq!(bindings.binding(&role)?.state, RoleState::Waiting);

        Ok(())
    }

    #[test]
    fn a_restart_is_refused_for_a_role_that_was_never_stopped() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        assert!(matches!(
            bindings.restart_after_repeated_failures(&role),
            Err(BindingError::RoleNotStopped { .. })
        ));

        Ok(())
    }

    #[test]
    fn three_failed_readbacks_suspend_capture_until_one_succeeds() -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut drivers = Drivers::new();
        let driver = drivers.add(RecordingDriver {
            applied_configurations: Arc::new(Mutex::new(Vec::new())),
        });
        let mut bindings = Bindings::default();
        let mut configured_binding = binding(role.clone(), display_endpoint("studio-display")?);
        configured_binding.driver = driver;
        bindings.register(configured_binding)?;
        // Registration always starts a role waiting, so the ready state is reached the only way it
        // ever is: through one successful apply.
        let start_apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::in_service(),
                    &hardware_inventory,
                )?,
            _ => return Err("a new binding must select the waiting view".into()),
        };
        drivers.start_apply(&mut World::new(), start_apply_request)?;
        match bindings.role_view(&role)? {
            RoleView::Applying(mut applying_role) => {
                applying_role.finish(AttemptOutcome::Succeeded);
            },
            _ => return Err("a dispatched apply must select the applying view".into()),
        }

        for _ in 0..2 {
            match bindings.role_view(&role)? {
                RoleView::Ready(mut ready_role) => {
                    ready_role.record_capture(CaptureOutcome::ReadFailed(blocked()));
                },
                _ => return Err("a ready role must select the ready view".into()),
            }
            assert_eq!(
                bindings.capture_dispatch(&role),
                super::CaptureDispatch::Eligible
            );
        }
        match bindings.role_view(&role)? {
            RoleView::Ready(mut ready_role) => {
                ready_role.record_capture(CaptureOutcome::ReadFailed(blocked()));
            },
            _ => return Err("a ready role must select the ready view".into()),
        }

        assert_eq!(
            bindings.capture_dispatch(&role),
            super::CaptureDispatch::SuspendedAfterRepeatedFailures
        );

        match bindings.role_view(&role)? {
            RoleView::Ready(mut ready_role) => {
                ready_role.record_capture(CaptureOutcome::Read(LastKnownGoodConfiguration::known(
                    TestConfiguration(7),
                )));
            },
            _ => return Err("a ready role must select the ready view".into()),
        }

        assert_eq!(
            bindings.capture_dispatch(&role),
            super::CaptureDispatch::Eligible
        );

        Ok(())
    }

    #[test]
    fn a_restore_only_permit_drives_authored_intent_only_until_a_readback_establishes_one()
    -> Result<(), Box<dyn Error>> {
        let role = RoleKey::new("primary-window")?;
        let hardware_inventory = HardwareInventory::default();
        let mut bindings = Bindings::default();
        bindings.register(binding(role.clone(), display_endpoint("studio-display")?))?;

        // Nothing established: the restore-only permit is the only authorization a RestoreOnly
        // device ever offers, and refusing it here would leave that device permanently unapplied.
        let start_apply_request = match bindings.role_view(&role)? {
            RoleView::Waiting(WaitingRole::Hardware(requesting_role)) => requesting_role
                .start_requested_apply(
                    AttemptId::default(),
                    ApplyPermit::restore_only(),
                    &hardware_inventory,
                )?,
            _ => return Err("a new binding must select the waiting view".into()),
        };
        assert_eq!(
            start_apply_request.configuration_source,
            super::ApplyConfigurationSource::Requested
        );

        Ok(())
    }
}
