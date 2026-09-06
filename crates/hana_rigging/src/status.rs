use std::num::NonZeroU32;
use std::time::Duration;

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::reflect::ReflectSerialize;
use serde::Serialize;

use crate::AttemptInvalidation;
use crate::AttemptRef;
use crate::BatchRef;
use crate::ClaimHolder;
use crate::DeviceAccessErrorView;
use crate::DeviceEndpoint;
use crate::DeviceId;
use crate::DeviceKey;
use crate::DriverContractFailureReport;
use crate::DriverId;
use crate::DriverOutcomeStatus;
use crate::DriverStopReason;
use crate::EstablishedFlowView;
use crate::PermissionGate;
use crate::Presence;
use crate::ReporterId;
use crate::ReporterRef;
use crate::RiggingRuntimeTime;
use crate::RoleKey;
use crate::WaitTiming;
use crate::devices::DeviceRevision;

/// BRP-readable state for one registered role.
#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "the published component contract requires PartialEq without promising Eq"
)]
#[derive(Component, Clone, PartialEq, Reflect, Serialize)]
#[reflect(opaque)]
#[reflect(Component, PartialEq, Serialize)]
pub struct RoleStatus(RoleStatusView);

impl RoleStatus {
    pub(crate) const fn from_view(view: RoleStatusView) -> Self { Self(view) }

    /// Borrow the complete role-state projection.
    #[must_use]
    pub const fn view(&self) -> &RoleStatusView { &self.0 }
}

/// Complete diagnostic state of one registered role.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum RoleStatusView {
    /// The role is waiting for the named condition.
    Waiting(WaitingStatusView),
    /// One endpoint operation is in flight.
    Applying {
        /// Process-local attempt reference.
        attempt:         AttemptRef,
        /// Runtime time at which driver work began.
        since:           RiggingRuntimeTime,
        /// Runtime time at which the attempt's ordinary bound ends.
        deadline:        RiggingRuntimeTime,
        /// Configuration supplied to the driver.
        source:          ApplySourceView,
        /// Registration-scoped evidence distinguishing the first apply from a re-apply.
        application_run: RegistrationApplicationRunView,
    },
    /// The role has an established driver session.
    Established {
        /// Runtime time at which the current session became established.
        since:   RiggingRuntimeTime,
        /// Process-local session reference.
        session: SessionRef,
        /// Relation between the accepted configuration and the dispatched value.
        applied: AppliedKind,
        /// Data-arrival state for the current session.
        flow:    EstablishedFlowView,
    },
    /// Automatic driver work has stopped.
    Stopped(StoppedStatusView),
}

/// Condition preventing a waiting role from starting driver work.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum WaitingStatusView {
    /// Hardware evidence must change before dispatch can proceed.
    Reporter(HardwareWait),
    /// Kernel retry pacing has not opened.
    KernelRetry {
        /// Retained timing for this wait.
        timing:   WaitTiming,
        /// Exact gate whose opening permits another dispatch.
        gate:     RetryGateView,
        /// Consecutive apply failures retained for the next attempt.
        failures: RoleApplyFailureRunView,
    },
    /// Application code must request another apply.
    ApplicationReapply {
        /// Retained timing for this wait.
        timing: WaitTiming,
    },
    /// Application code must enable at least one registered reporter.
    ApplicationReporterEnable {
        /// Registered reporters whose activation can end the wait.
        reporters: NonEmptyReporterRefs,
        /// Retained timing for this wait.
        timing:    WaitTiming,
    },
    /// Application code must register a replacement binding.
    NewRegistration {
        /// Retained timing for this wait.
        timing: WaitTiming,
    },
    /// An operator must answer an identity question.
    OperatorDecision {
        /// Authored role awaiting the answer.
        role:      RoleKey,
        /// Candidate key awaiting adjudication.
        candidate: DeviceKey,
        /// Retained timing for this wait.
        timing:    WaitTiming,
    },
    /// Another owner must release its device claim.
    ClaimRelease {
        /// Owner information supplied by the platform.
        holder: ClaimHolderView,
        /// Retained timing for this wait.
        timing: WaitTiming,
    },
    /// The driver registration or erased configuration contract must be repaired.
    DriverRepair {
        /// Owned failure from the erased driver boundary.
        error:  DriverContractFailureView,
        /// Retained timing for this wait.
        timing: WaitTiming,
        /// Retry schedule installed for the refused dispatch.
        retry:  RetryScheduleView,
    },
    /// Application code must register the reflected capability component.
    ApplicationCapabilityRegistrationRequired {
        /// Capability type whose projection cannot proceed.
        failure: CapabilityProjectionFailure,
        /// Retained timing for this wait.
        timing:  WaitTiming,
    },
    /// Application code must register a reporter covering this durable key.
    ApplicationReporterRegistrationRequired {
        /// Key for which no registered reporter establishes evidence.
        key:    DeviceKey,
        /// Retained timing for this wait.
        timing: WaitTiming,
    },
    /// Application code must return an authored device to managed mode.
    ApplicationDeviceEnableRequired {
        /// Offline device whose authored mode prevents driver work.
        key:    DeviceKey,
        /// Retained timing for this wait.
        timing: WaitTiming,
    },
    /// The user must grant the operating-system permission named by the provider.
    ApplicationPermissionRequired {
        /// Permission class preventing device access.
        gate:   PermissionGateView,
        /// Retained timing for this wait.
        timing: WaitTiming,
    },
    /// Application code must repair role configuration that could not construct a request.
    ApplicationBindingRepairRequired {
        /// Retained timing for this wait.
        timing: WaitTiming,
        /// Retry schedule installed for the refused dispatch.
        retry:  RetryScheduleView,
    },
    /// Application code must attach the target entity used by the endpoint driver.
    ApplicationTargetAttachmentRequired {
        /// Retained timing for this wait.
        timing: WaitTiming,
        /// Retry schedule installed for the refused dispatch.
        retry:  RetryScheduleView,
    },
    /// A kernel state inconsistency must be corrected before authorization can proceed.
    KernelStateRepairRequired {
        /// Retained timing for this wait.
        timing: WaitTiming,
    },
}

/// Consecutive apply failures retained while a role waits for another attempt.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum RoleApplyFailureRunView {
    /// No failed apply precedes the next attempt.
    Clear,
    /// This many consecutive applies failed before the current wait.
    Consecutive {
        /// Nonzero failed-apply count retained for escalation.
        failures:    NonZeroU32,
        /// Last terminal result in this consecutive failure run.
        last_ending: AttemptEndingView,
    },
}

/// Registration-scoped evidence for the apply currently in flight.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum RegistrationApplicationRunView {
    /// This is the first apply issued for the current binding registration.
    Initial,
    /// At least one earlier apply ended under the current binding registration.
    Reapplying {
        /// Ordinal of the current apply within this registration.
        applications: NonZeroU32,
        /// Ending of the apply immediately preceding this one.
        last_ending:  AttemptEndingView,
    },
}

/// Reporter evidence a role waits for before its endpoint can resolve or authorize.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum HardwareWait {
    /// Covering reporters have not supplied their first complete sets.
    AwaitingFirstReport {
        /// Durable key the role retains.
        key:       DeviceKey,
        /// Covering reporters that can establish the key's state.
        reporters: NonEmptyReporterRefs,
        /// Retained timing for this wait.
        timing:    WaitTiming,
    },
    /// Completed evidence cannot confirm the key's availability.
    Unconfirmed {
        /// Durable key the role retains.
        key:       DeviceKey,
        /// Why current evidence cannot confirm the key's availability.
        basis:     UnconfirmedBasis,
        /// Reporters contributing the current uncertainty.
        reporters: NonEmptyReporterRefs,
        /// Retained timing for this wait.
        timing:    WaitTiming,
    },
    /// Reporter evidence has expired or reports an unreachable unit.
    Unreachable {
        /// Durable key the role retains.
        key:       DeviceKey,
        /// Reporters contributing the current uncertainty.
        reporters: NonEmptyReporterRefs,
        /// Retained timing for this wait.
        timing:    WaitTiming,
    },
    /// Confirmed departure is inside the configured retirement grace period.
    DepartureGrace {
        /// Durable key the role retains.
        key:      DeviceKey,
        /// Runtime deadline at which grace ends.
        deadline: RiggingRuntimeTime,
        /// Reporter and batch that established departure.
        evidence: RetirementEvidence,
    },
    /// Reporter evidence established that the key is absent.
    Absent {
        /// Durable key the role retains.
        key:            DeviceKey,
        /// Reporter and batch that established absence.
        established_by: RetirementEvidence,
    },
}

impl HardwareWait {
    pub(crate) fn names_reporter(&self, reporter: ReporterRef) -> bool {
        match self {
            Self::AwaitingFirstReport { reporters, .. } | Self::Unreachable { reporters, .. } => {
                reporters.contains(reporter)
            },
            Self::Unconfirmed {
                basis, reporters, ..
            } => {
                reporters.contains(reporter)
                    || matches!(
                        basis,
                        UnconfirmedBasis::UncoveredAbsence {
                            reporter: deciding_reporter,
                            ..
                        } if *deciding_reporter == reporter
                    )
            },
            Self::DepartureGrace { evidence, .. } => evidence.reporter == reporter,
            Self::Absent { established_by, .. } => established_by.reporter == reporter,
        }
    }
}

/// Terminal role state with only the fields valid for each stopping cause.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum StoppedStatusView {
    /// Consecutive device-operation failures exhausted the automatic retry run.
    RepeatedFailures {
        /// Number of consecutive failed attempts.
        failures:     NonZeroU32,
        /// Last terminal attempt result in the run.
        last_ending:  AttemptEndingView,
        /// Actions that can resume this role.
        resumes_when: ResumeCondition,
    },
    /// The driver reported that the operation is unsupported.
    Unsupported {
        /// Owned driver reason supplied to the operator.
        reason:       DriverStopReasonView,
        /// Action that can resume this role.
        resumes_when: ResumeCondition,
    },
}

/// How a stopped role can return to waiting work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum ResumeCondition {
    /// Explicit restart or departure followed by exact reacquisition resumes the role.
    ExplicitRestartOrReacquisition,
    /// Only an explicit application restart resumes the role.
    ExplicitRestart,
}

/// Relation between an accepted configuration and the dispatched configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum AppliedKind {
    /// The driver accepted the dispatched configuration.
    AsDispatched,
    /// The driver established a different configuration and reported that fact.
    DiffersFromDispatched,
}

/// Configuration source supplied to one apply operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum ApplySourceView {
    /// The application-authored request was dispatched.
    Requested,
    /// The last configuration proved by readback was dispatched.
    LastKnownGood,
}

/// Process-local reference issued for one established session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Serialize)]
#[reflect(opaque)]
#[reflect(Serialize)]
pub struct SessionRef(u64);

impl SessionRef {
    pub(crate) const fn new(value: u64) -> Self { Self(value) }

    /// Return the process-local session number.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

/// Owned terminal result projected from one private attempt ending.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum AttemptEndingView {
    /// The endpoint driver reported this terminal result.
    Reported(AttemptOutcomeView),
    /// Kernel validation invalidated the attempt.
    Invalidated(AttemptInvalidationView),
    /// The erased driver boundary refused the typed operation.
    ContractFailed(DriverContractFailureView),
}

/// Owned projection of a driver-reported terminal result.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum AttemptOutcomeView {
    /// The dispatched configuration was established.
    Succeeded(AppliedKind),
    /// The operation failed with a classified device-access error.
    Failed(DeviceAccessErrorView),
    /// The kernel or driver abandoned the operation.
    Aborted,
}

/// Owned projection of a kernel attempt invalidation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum AttemptInvalidationView {
    /// The endpoint resolves to a different process-local device.
    DeviceChanged,
    /// The retained device revision advanced.
    RevisionAdvanced,
    /// The process lost its claim on the device.
    ClaimLost,
    /// The identity verdict stopped authorizing the unit.
    IdentityNoLongerConfirmed,
    /// The unit is no longer present.
    DeviceNotPresent,
    /// Authored inventory placed the device offline.
    InventoryWithdrewTheDevice,
    /// The attempt exceeded its deadline and overrun allowance.
    OverrunExhausted,
    /// Application code retired the role.
    RoleRetired,
    /// Application code replaced the binding.
    BindingReplaced,
    /// The erased driver boundary failed before any typed `EndpointDriver` method could run: the
    /// downcast found no registered driver for the id, or a driver or configuration type that
    /// differs from the one the dispatch functions were built for.
    DriverContractFailed,
}

/// Owned projection of a driver stop reason.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum DriverStopReasonView {
    /// The current platform cannot perform this operation.
    Unsupported {
        /// Provider detail naming the unsupported contract.
        detail: String,
    },
}

/// Owned projection of an erased driver contract failure.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum DriverContractFailureView {
    /// No driver registration owns the route.
    DriverNotRegistered {
        /// Process-local driver route.
        driver: DriverRef,
    },
    /// The erased entry held a different driver type.
    DriverTypeMismatch {
        /// Concrete driver type required by the erased function.
        expected_driver: String,
    },
    /// The retained configuration had the wrong concrete type.
    ConfigurationTypeMismatch {
        /// Concrete configuration type accepted by the driver.
        expected_configuration: String,
        /// Reflected type path of the retained value.
        received_configuration: String,
    },
    /// A restore request no longer had a checked last-known-good value.
    LastKnownGoodConfigurationUnavailable {
        /// Role whose restore value was unavailable.
        role: RoleKey,
    },
}

/// Process-local diagnostic reference for one registered driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Serialize)]
#[reflect(opaque)]
#[reflect(Serialize)]
pub struct DriverRef(u32);

impl DriverRef {
    /// Return the driver registry's process-local number.
    #[must_use]
    pub const fn get(self) -> u32 { self.0 }
}

/// Owned platform information about a competing device owner.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum ClaimHolderView {
    /// The platform supplied the owner's name.
    Named(String),
    /// The platform reported an owner without identifying it.
    Unidentified,
}

/// Owned platform permission class required before a device can be used.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum PermissionGateView {
    /// Camera capture permission is required.
    CameraAccess,
    /// Screen-recording permission is required.
    ScreenRecording,
    /// Input-monitoring permission is required.
    InputMonitoring,
    /// A provider-specific permission is required.
    Other {
        /// Provider detail identifying the permission.
        detail: String,
    },
}

/// Retry schedule retained after a failed or refused dispatch.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum RetryGateView {
    /// A different per-device revision reading opens the gate.
    DeviceRevisionChanged {
        /// Revision reading retained when the gate was installed.
        from: DeviceRevisionGateView,
    },
    /// Reaching this runtime time opens the gate.
    TimeReached {
        /// Runtime time at which dispatch becomes eligible.
        retry_at: RiggingRuntimeTime,
    },
}

/// Retry pacing retained beside a dispatch refusal cause.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum RetryScheduleView {
    /// No retry gate is installed.
    Ready,
    /// Another dispatch waits for this gate.
    Blocked(RetryGateView),
}

/// Device revision reading retained by a revision-based retry gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum DeviceRevisionGateView {
    /// The role's key resolved to no retained device.
    DeviceRetired,
    /// The role's device had this revision.
    Retained(DeviceRevisionView),
}

/// Capability projection failure visible to reporter and role diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum CapabilityProjectionFailure {
    /// The world has no application type registry resource.
    ApplicationTypeRegistryUnavailable {
        /// Reflected type path of a capability affected by the missing registry.
        affected_type_path: String,
    },
    /// The application type registry has no `ReflectComponent` entry for this type.
    ReflectComponentNotRegistered {
        /// Reflected type path of the component that could not be projected.
        type_path: String,
    },
}

/// Durable endpoint authored for one role.
#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "the published component contract requires PartialEq without promising Eq"
)]
#[derive(Component, Clone, PartialEq, Reflect, Serialize)]
#[reflect(Component, PartialEq, Serialize)]
pub struct RoleEndpoint(DeviceEndpoint);

impl RoleEndpoint {
    pub(crate) const fn new(endpoint: DeviceEndpoint) -> Self { Self(endpoint) }

    /// Borrow the role's durable endpoint.
    #[must_use]
    pub const fn endpoint(&self) -> &DeviceEndpoint { &self.0 }
}

/// BRP-readable identity, revision, and availability for one live device entity.
#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "the published component contract requires PartialEq without promising Eq"
)]
#[derive(Component, Clone, PartialEq, Reflect, Serialize)]
#[reflect(Component, PartialEq, Serialize)]
pub struct DeviceStatus {
    key:          DeviceKey,
    id:           DeviceRef,
    revision:     DeviceRevisionView,
    availability: KeyAvailability,
}

impl DeviceStatus {
    pub(crate) const fn new(
        key: DeviceKey,
        id: DeviceRef,
        revision: DeviceRevisionView,
        availability: KeyAvailability,
    ) -> Self {
        Self {
            key,
            id,
            revision,
            availability,
        }
    }

    /// Borrow the durable device key.
    #[must_use]
    pub const fn key(&self) -> &DeviceKey { &self.key }

    /// Borrow the process-local device reference.
    #[must_use]
    pub const fn id(&self) -> &DeviceRef { &self.id }

    /// Borrow the device revision projection.
    #[must_use]
    pub const fn revision(&self) -> &DeviceRevisionView { &self.revision }

    /// Borrow the current key availability.
    #[must_use]
    pub const fn availability(&self) -> &KeyAvailability { &self.availability }
}

/// Process-local diagnostic reference for one retained device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Serialize)]
#[reflect(opaque)]
#[reflect(Serialize)]
pub struct DeviceRef(u64);

impl DeviceRef {
    pub(crate) const fn from_device_id(device_id: DeviceId) -> Self { Self(device_id.get()) }

    /// Return the device registry's process-local number.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }

    /// Return the process-local device handle during the Phase 10-12 driver migrations.
    #[must_use]
    pub const fn device_id(self) -> DeviceId { DeviceId::new(self.0) }
}

/// Serializable projection of a retained per-device revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Reflect, Serialize)]
#[reflect(opaque)]
#[reflect(Serialize)]
pub struct DeviceRevisionView(u64);

impl DeviceRevisionView {
    pub(crate) const fn from_revision(device_revision: DeviceRevision) -> Self {
        Self(device_revision.get())
    }

    /// Return the number of accepted device changes.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

/// Availability conclusion published for one durable key.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum KeyAvailability {
    /// At least one reporter currently contributes present evidence.
    Present(PresentEvidence),
    /// Confirmed departure is inside the configured retirement grace period.
    DepartureGrace {
        /// Runtime time at which grace began.
        since:    RiggingRuntimeTime,
        /// Runtime time at which grace ends.
        deadline: RiggingRuntimeTime,
        /// Reporter and batch that established departure.
        evidence: RetirementEvidence,
    },
    /// Covering reporters exist and none has supplied a complete set.
    AwaitingFirstReport {
        /// Runtime time at which this availability began.
        since:     RiggingRuntimeTime,
        /// Covering reporters still awaited.
        reporters: NonEmptyReporterRefs,
    },
    /// Completed evidence does not confirm presence or absence.
    Unconfirmed {
        /// Runtime time at which this availability began.
        since: RiggingRuntimeTime,
        /// Why current evidence cannot confirm availability.
        basis: UnconfirmedBasis,
    },
    /// Retained evidence cannot currently establish reachability.
    Unreachable {
        /// Runtime time at which this availability began.
        since:     RiggingRuntimeTime,
        /// Reporters whose evidence contributed to the conclusion.
        reporters: NonEmptyReporterRefs,
    },
    /// Reporter evidence established absence.
    Absent {
        /// Runtime time at which this availability began.
        since:          RiggingRuntimeTime,
        /// Reporter and batch that established absence.
        established_by: RetirementEvidence,
    },
}

/// Evidence basis for an availability conclusion that confirms neither presence nor absence.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum UnconfirmedBasis {
    /// No fresh record or covering omission contributes evidence for the key.
    NoFreshEvidence,
    /// A reporter supplied an absent record but its coverage does not establish this key's absence.
    UncoveredAbsence {
        /// Reporter supplying the absent record.
        reporter: ReporterRef,
        /// Accepted complete-set batch carrying the record.
        batch:    BatchRef,
    },
}

/// An availability conclusion that cannot authorize device work.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum UnavailableKeyAvailability {
    /// Confirmed departure is inside the configured retirement grace period.
    DepartureGrace {
        /// Runtime time at which grace began.
        since:    RiggingRuntimeTime,
        /// Runtime time at which grace ends.
        deadline: RiggingRuntimeTime,
        /// Reporter and batch that established departure.
        evidence: RetirementEvidence,
    },
    /// Covering reporters exist and none has supplied a complete set.
    AwaitingFirstReport {
        /// Runtime time at which this availability began.
        since:     RiggingRuntimeTime,
        /// Covering reporters still awaited.
        reporters: NonEmptyReporterRefs,
    },
    /// Completed evidence confirms neither presence nor absence.
    Unconfirmed {
        /// Runtime time at which this availability began.
        since: RiggingRuntimeTime,
        /// Why current evidence cannot confirm availability.
        basis: UnconfirmedBasis,
    },
    /// Retained evidence cannot currently establish reachability.
    Unreachable {
        /// Runtime time at which this availability began.
        since:     RiggingRuntimeTime,
        /// Reporters whose evidence contributed to the conclusion.
        reporters: NonEmptyReporterRefs,
    },
    /// Reporter evidence established absence.
    Absent {
        /// Runtime time at which this availability began.
        since:          RiggingRuntimeTime,
        /// Reporter and batch that established absence.
        established_by: RetirementEvidence,
    },
}

impl TryFrom<KeyAvailability> for UnavailableKeyAvailability {
    type Error = KeyAvailability;

    fn try_from(availability: KeyAvailability) -> Result<Self, Self::Error> {
        match availability {
            KeyAvailability::Present(evidence) => Err(KeyAvailability::Present(evidence)),
            KeyAvailability::DepartureGrace {
                since,
                deadline,
                evidence,
            } => Ok(Self::DepartureGrace {
                since,
                deadline,
                evidence,
            }),
            KeyAvailability::AwaitingFirstReport { since, reporters } => {
                Ok(Self::AwaitingFirstReport { since, reporters })
            },
            KeyAvailability::Unconfirmed { since, basis } => Ok(Self::Unconfirmed { since, basis }),
            KeyAvailability::Unreachable { since, reporters } => {
                Ok(Self::Unreachable { since, reporters })
            },
            KeyAvailability::Absent {
                since,
                established_by,
            } => Ok(Self::Absent {
                since,
                established_by,
            }),
        }
    }
}

impl From<UnavailableKeyAvailability> for KeyAvailability {
    fn from(availability: UnavailableKeyAvailability) -> Self {
        match availability {
            UnavailableKeyAvailability::DepartureGrace {
                since,
                deadline,
                evidence,
            } => Self::DepartureGrace {
                since,
                deadline,
                evidence,
            },
            UnavailableKeyAvailability::AwaitingFirstReport { since, reporters } => {
                Self::AwaitingFirstReport { since, reporters }
            },
            UnavailableKeyAvailability::Unconfirmed { since, basis } => {
                Self::Unconfirmed { since, basis }
            },
            UnavailableKeyAvailability::Unreachable { since, reporters } => {
                Self::Unreachable { since, reporters }
            },
            UnavailableKeyAvailability::Absent {
                since,
                established_by,
            } => Self::Absent {
                since,
                established_by,
            },
        }
    }
}

/// Reporter contributions proving that a key is currently present.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub struct PresentEvidence {
    contributors: NonEmptyContributors,
}

impl PresentEvidence {
    pub(crate) const fn new(contributors: NonEmptyContributors) -> Self { Self { contributors } }

    /// Borrow the checked non-empty contributor list.
    #[must_use]
    pub const fn contributors(&self) -> &NonEmptyContributors { &self.contributors }
}

/// One or more reporter contributions, sorted by reporter reference.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub struct NonEmptyContributors(Vec<ContributorView>);

impl NonEmptyContributors {
    pub(crate) fn from_contributors(mut contributors: Vec<ContributorView>) -> Result<Self, ()> {
        contributors.sort_by_key(|contributor| contributor.reporter.get());
        if contributors.is_empty() {
            Err(())
        } else {
            Ok(Self(contributors))
        }
    }

    /// Borrow the sorted contributor list.
    #[must_use]
    pub fn as_slice(&self) -> &[ContributorView] { &self.0 }
}

/// One reporter record contributing to a device availability conclusion.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub struct ContributorView {
    /// Reporter that supplied the record.
    pub reporter: ReporterRef,
    /// Accepted complete-set batch carrying the record.
    pub batch:    BatchRef,
    /// Reachability stated by this reporter record.
    pub presence: PresenceView,
}

impl ContributorView {
    pub(crate) const fn new(
        reporter: ReporterRef,
        batch: BatchRef,
        presence: PresenceView,
    ) -> Self {
        Self {
            reporter,
            batch,
            presence,
        }
    }
}

/// Serializable reporter reachability carried only as contributor evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum PresenceView {
    /// The reporter observed the unit as present.
    Present,
    /// The reporter established that the unit is absent.
    Absent,
    /// The reporter could not establish reachability.
    Unreachable {
        /// Elapsed time reported for this unreachable interval.
        since: Duration,
    },
}

/// Reporter and accepted batch that established device retirement evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub struct RetirementEvidence {
    /// Reporter supplying the decisive evidence.
    pub reporter: ReporterRef,
    /// Accepted complete-set batch carrying that evidence.
    pub batch:    BatchRef,
}

impl RetirementEvidence {
    pub(crate) const fn new(reporter: ReporterRef, batch: BatchRef) -> Self {
        Self { reporter, batch }
    }
}

/// One or more reporter references, sorted by process-local reference.
#[derive(Clone, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub struct NonEmptyReporterRefs(Vec<ReporterRef>);

impl NonEmptyReporterRefs {
    pub(crate) fn from_first_and_rest(first: ReporterId, rest: &[ReporterId]) -> Self {
        let mut reporters = std::iter::once(first)
            .chain(rest.iter().copied())
            .map(ReporterRef::from_reporter_id)
            .collect::<Vec<_>>();
        reporters.sort_by_key(|reporter| reporter.get());
        reporters.dedup();
        Self(reporters)
    }

    pub(crate) fn from_reporter_ids(
        reporters: impl IntoIterator<Item = ReporterId>,
    ) -> Result<Self, ()> {
        let mut reporters = reporters
            .into_iter()
            .map(ReporterRef::from_reporter_id)
            .collect::<Vec<_>>();
        reporters.sort_by_key(|reporter| reporter.get());
        reporters.dedup();
        if reporters.is_empty() {
            Err(())
        } else {
            Ok(Self(reporters))
        }
    }

    /// Borrow the sorted reporter references.
    #[must_use]
    pub fn as_slice(&self) -> &[ReporterRef] { &self.0 }

    pub(crate) fn contains(&self, reporter: ReporterRef) -> bool { self.0.contains(&reporter) }
}

impl From<&crate::DriverOutcomeStatus> for AttemptOutcomeView {
    fn from(outcome: &crate::DriverOutcomeStatus) -> Self {
        match outcome {
            DriverOutcomeStatus::Succeeded(applied) => Self::Succeeded(*applied),
            DriverOutcomeStatus::Failed(error) => Self::Failed(DeviceAccessErrorView::from(error)),
            DriverOutcomeStatus::Aborted(_) => Self::Aborted,
        }
    }
}

impl From<AttemptInvalidation> for AttemptInvalidationView {
    fn from(invalidation: AttemptInvalidation) -> Self {
        match invalidation {
            AttemptInvalidation::DeviceChanged => Self::DeviceChanged,
            AttemptInvalidation::RevisionAdvanced => Self::RevisionAdvanced,
            AttemptInvalidation::ClaimLost => Self::ClaimLost,
            AttemptInvalidation::IdentityNoLongerConfirmed => Self::IdentityNoLongerConfirmed,
            AttemptInvalidation::DeviceNotPresent => Self::DeviceNotPresent,
            AttemptInvalidation::InventoryWithdrewTheDevice => Self::InventoryWithdrewTheDevice,
            AttemptInvalidation::OverrunExhausted => Self::OverrunExhausted,
            AttemptInvalidation::RoleRetired => Self::RoleRetired,
            AttemptInvalidation::BindingReplaced => Self::BindingReplaced,
            AttemptInvalidation::DriverContractFailed => Self::DriverContractFailed,
        }
    }
}

impl From<&DriverContractFailureReport> for DriverContractFailureView {
    fn from(failure: &DriverContractFailureReport) -> Self {
        match failure {
            DriverContractFailureReport::DriverNotRegistered { driver } => {
                Self::DriverNotRegistered {
                    driver: DriverRef::from(*driver),
                }
            },
            DriverContractFailureReport::DriverTypeMismatch { expected_driver } => {
                Self::DriverTypeMismatch {
                    expected_driver: expected_driver.clone(),
                }
            },
            DriverContractFailureReport::ConfigurationTypeMismatch {
                expected_configuration,
                received_configuration,
            } => Self::ConfigurationTypeMismatch {
                expected_configuration: expected_configuration.clone(),
                received_configuration: received_configuration.clone(),
            },
            DriverContractFailureReport::LastKnownGoodConfigurationUnavailable { role } => {
                Self::LastKnownGoodConfigurationUnavailable { role: role.clone() }
            },
        }
    }
}

impl From<DriverId> for DriverRef {
    fn from(driver: DriverId) -> Self { Self(driver.0) }
}

impl From<ClaimHolder> for ClaimHolderView {
    fn from(holder: ClaimHolder) -> Self {
        match holder {
            ClaimHolder::Named(name) => Self::Named(name),
            ClaimHolder::Unidentified => Self::Unidentified,
        }
    }
}

impl From<&DriverStopReason> for DriverStopReasonView {
    fn from(reason: &DriverStopReason) -> Self {
        match reason {
            DriverStopReason::Unsupported { detail } => Self::Unsupported {
                detail: detail.clone(),
            },
        }
    }
}

impl From<&PermissionGate> for PermissionGateView {
    fn from(gate: &PermissionGate) -> Self {
        match gate {
            PermissionGate::CameraAccess => Self::CameraAccess,
            PermissionGate::ScreenRecording => Self::ScreenRecording,
            PermissionGate::InputMonitoring => Self::InputMonitoring,
            PermissionGate::Other { detail } => Self::Other {
                detail: detail.clone(),
            },
        }
    }
}

impl From<Presence> for PresenceView {
    fn from(presence: Presence) -> Self {
        match presence {
            Presence::Present => Self::Present,
            Presence::Absent => Self::Absent,
            Presence::Unreachable { since } => Self::Unreachable { since },
        }
    }
}
