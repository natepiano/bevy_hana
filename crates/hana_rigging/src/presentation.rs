//! One derived operator vocabulary for every device kind.
//!
//! Role status, key availability, and session flow are three separate kernel axes. Each device
//! integration used to fold them into its own words from its own private evidence, so two device
//! kinds could disagree about the same situation. [`derive_role_presentation`] is the single
//! ordered function that folds them, and [`RolePresentation`] is the only way its answer reaches an
//! entity: the wrapper's constructor is crate-private, so an integration can widen the
//! [`RolePresentationView::Unavailable`] payload through [`RolePresentation::map_unavailable`] but
//! can never select a different variant.

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::reflect::ReflectSerialize;
use serde::Serialize;

use crate::ContinuousFlowExpiryCause;
use crate::ContinuousFlowView;
use crate::EstablishedFlowView;
use crate::KeyAvailability;
use crate::RoleStatusView;
use crate::StoppedStatusView;
use crate::WaitingStatusView;

/// Operator-facing state of one registered role, derived by the kernel.
///
/// Opaque for the same reason [`crate::RoleStatus`] is: Rust cannot give a public enum private
/// variants, so without the wrapper any caller could insert
/// [`RolePresentationView::Presenting`] directly and the shared vocabulary would be a convention
/// rather than a kernel projection.
#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "the published component contract requires PartialEq without promising Eq"
)]
/// The type is public and nameable, because an application reads one:
///
/// ```
/// fn presented<Cause>(_: &hana_rigging::RolePresentation<Cause>) {}
/// ```
///
/// Its wrapped view is private, so the kernel is the only writer. The signature
/// above is what keeps this case meaningful — a rename would break it loudly
/// rather than leaving this one failing for an unrelated reason:
///
/// ```compile_fail,E0423
/// use hana_rigging::{KernelRolePresentation, RolePresentation, RolePresentationView};
///
/// let _: KernelRolePresentation = RolePresentation(RolePresentationView::Presenting);
/// ```
#[derive(Component, Clone, PartialEq, Reflect, Serialize)]
#[reflect(opaque)]
#[reflect(Component, PartialEq, Serialize)]
#[reflect(where UnavailableCause: Clone + PartialEq + Serialize + Send + Sync + 'static)]
pub struct RolePresentation<UnavailableCause>(RolePresentationView<UnavailableCause>);

impl<UnavailableCause> RolePresentation<UnavailableCause> {
    pub(crate) const fn from_view(view: RolePresentationView<UnavailableCause>) -> Self {
        Self(view)
    }

    /// Borrow the derived operator state.
    #[must_use]
    pub const fn view(&self) -> &RolePresentationView<UnavailableCause> { &self.0 }

    /// Widen the unavailable payload without disturbing the kernel's chosen variant.
    ///
    /// The kernel classifies into [`RoleUnavailableCause`]; an integration whose device kind has
    /// causes of its own converts into a wider payload here. `widen` never sees the variant, so an
    /// integration cannot re-decide whether a role is unavailable at all.
    #[must_use]
    pub fn map_unavailable<Widened>(
        self,
        widen: impl FnOnce(UnavailableCause) -> Widened,
    ) -> RolePresentation<Widened> {
        RolePresentation(match self.0 {
            RolePresentationView::Scanning => RolePresentationView::Scanning,
            RolePresentationView::Disconnected => RolePresentationView::Disconnected,
            RolePresentationView::Connecting => RolePresentationView::Connecting,
            RolePresentationView::Connected(cause) => RolePresentationView::Connected(cause),
            RolePresentationView::Presenting => RolePresentationView::Presenting,
            RolePresentationView::Unavailable(cause) => {
                RolePresentationView::Unavailable(widen(cause))
            },
        })
    }
}

/// The six words every device kind presents a registered role with.
///
/// Wording may vary per device kind; this derivation may not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
#[reflect(where UnavailableCause: Serialize)]
pub enum RolePresentationView<UnavailableCause> {
    /// The reporter has not answered yet.
    Scanning,
    /// The device is not attached.
    Disconnected,
    /// An apply is in progress, or the kernel will retry without anyone's help.
    Connecting,
    /// The role is bound, but hana is making no claim that data is moving.
    Connected(ConnectedCause),
    /// The role is bound and data is arriving within the gap budget.
    Presenting,
    /// The kernel cannot currently authorize this role.
    Unavailable(UnavailableCause),
}

/// Why a bound role carries no claim that data is moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum ConnectedCause {
    /// The session is established and its first datum is still inside its budget.
    AwaitingFirstDatum,
    /// The session crossed one of its configured flow bounds.
    Stalled(ContinuousFlowExpiryCause),
    /// The binding declared no flow expectation, so arrival is never evaluated.
    FlowNotMonitored,
}

/// Kernel classification of a role the kernel cannot currently authorize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize)]
#[reflect(Serialize)]
pub enum RoleUnavailableCause {
    /// Completed reporter evidence confirms neither presence nor absence of the key.
    IdentityUnconfirmed,
    /// Retained reporter evidence cannot establish that the device is reachable.
    DeviceUnreachable,
    /// An operator must answer an identity question before work can proceed.
    OperatorDecisionRequired,
    /// Another owner holds the platform claim on this device.
    ClaimHeldElsewhere,
    /// A person must grant an operating-system permission.
    PermissionRequired,
    /// Application code must enable a registered reporter.
    ReporterNotEnabled,
    /// Application code must register a reporter covering this key.
    ReporterRegistrationRequired,
    /// Application code must register the reflected capability component.
    CapabilityRegistrationRequired,
    /// Application code must return an authored device to managed mode.
    DeviceEnableRequired,
    /// Application code must request another apply.
    ReapplyRequired,
    /// Application code must repair the role's binding.
    BindingRepairRequired,
    /// Application code must attach the target entity the endpoint driver uses.
    TargetNotAttached,
    /// Consecutive device-operation failures exhausted the automatic retry run.
    RepeatedFailures,
    /// The driver reported that this operation is unsupported on this platform.
    Unsupported,
    /// Automatic recovery for this role was abandoned.
    RecoveryAbandoned,
}

/// The presentation the kernel itself derives, before any integration widens its cause.
pub type KernelRolePresentation = RolePresentation<RoleUnavailableCause>;

/// Fold role status, key availability, and session flow into one operator state.
///
/// The arms are ordered and the first match wins. A stopped role is terminal, so availability
/// cannot change it. A live session is authoritative over whatever a reporter last said, so the
/// established arm never reads availability. An apply genuinely is in flight, so it presents as
/// [`RolePresentationView::Connecting`] even under a departed device — the kernel ends that attempt
/// on its own next pass. Only a waiting role reads availability first, because telling an operator
/// "Connecting" while the cable is out is the failure this vocabulary exists to remove.
pub(crate) fn derive_role_presentation(
    status: &RoleStatusView,
    availability: KeyAvailability,
) -> RolePresentationView<RoleUnavailableCause> {
    match status {
        RoleStatusView::Stopped(stopped) => RolePresentationView::Unavailable(match stopped {
            StoppedStatusView::RepeatedFailures { .. } => RoleUnavailableCause::RepeatedFailures,
            StoppedStatusView::Unsupported { .. } => RoleUnavailableCause::Unsupported,
        }),
        RoleStatusView::Established { flow, .. } => established_presentation(*flow),
        RoleStatusView::Applying { .. } => RolePresentationView::Connecting,
        RoleStatusView::Waiting(waiting) => waiting_presentation(waiting, availability),
    }
}

/// Derive a live session's presentation from its flow state alone.
///
/// [`ContinuousFlowView::Flowing`] presents as [`RolePresentationView::Presenting`] because flow
/// means transport data reached hana, which is exactly the claim this vocabulary makes. Whether a
/// frame was published downstream is a consumer-side question the kernel holds no evidence for.
const fn established_presentation(
    flow: EstablishedFlowView,
) -> RolePresentationView<RoleUnavailableCause> {
    match flow {
        EstablishedFlowView::NotMonitored => {
            RolePresentationView::Connected(ConnectedCause::FlowNotMonitored)
        },
        EstablishedFlowView::Continuous(ContinuousFlowView::AwaitingFirstDatum { .. }) => {
            RolePresentationView::Connected(ConnectedCause::AwaitingFirstDatum)
        },
        EstablishedFlowView::Continuous(ContinuousFlowView::Flowing { .. }) => {
            RolePresentationView::Presenting
        },
        EstablishedFlowView::Continuous(ContinuousFlowView::Stalled { cause, .. }) => {
            RolePresentationView::Connected(ConnectedCause::Stalled(cause))
        },
    }
}

/// Derive a waiting role's presentation, reading availability before the wait family.
fn waiting_presentation(
    waiting: &WaitingStatusView,
    availability: KeyAvailability,
) -> RolePresentationView<RoleUnavailableCause> {
    match availability {
        KeyAvailability::Absent { .. } | KeyAvailability::DepartureGrace { .. } => {
            RolePresentationView::Disconnected
        },
        KeyAvailability::AwaitingFirstReport { .. } => RolePresentationView::Scanning,
        KeyAvailability::Unconfirmed { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::IdentityUnconfirmed)
        },
        KeyAvailability::Unreachable { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::DeviceUnreachable)
        },
        KeyAvailability::Present(_) => present_key_wait_presentation(waiting),
    }
}

/// Split a wait under a present key on the single question of whether anyone must act.
///
/// Automatic retry and kernel repair are [`RolePresentationView::Connecting`]. A wait that does not
/// resolve until a person or the application acts is [`RolePresentationView::Unavailable`], because
/// "connecting" tells an operator to wait when nothing proceeds without them.
const fn present_key_wait_presentation(
    waiting: &WaitingStatusView,
) -> RolePresentationView<RoleUnavailableCause> {
    match waiting {
        WaitingStatusView::Reporter(_)
        | WaitingStatusView::KernelRetry { .. }
        | WaitingStatusView::NewRegistration { .. }
        | WaitingStatusView::DriverRepair { .. }
        | WaitingStatusView::KernelStateRepairRequired { .. } => RolePresentationView::Connecting,
        WaitingStatusView::OperatorDecision { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::OperatorDecisionRequired)
        },
        WaitingStatusView::ClaimRelease { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::ClaimHeldElsewhere)
        },
        WaitingStatusView::ApplicationPermissionRequired { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::PermissionRequired)
        },
        WaitingStatusView::ApplicationReapply { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::ReapplyRequired)
        },
        WaitingStatusView::ApplicationReporterEnable { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::ReporterNotEnabled)
        },
        WaitingStatusView::ApplicationReporterRegistrationRequired { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::ReporterRegistrationRequired)
        },
        WaitingStatusView::ApplicationCapabilityRegistrationRequired { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::CapabilityRegistrationRequired)
        },
        WaitingStatusView::ApplicationDeviceEnableRequired { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::DeviceEnableRequired)
        },
        WaitingStatusView::ApplicationBindingRepairRequired { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::BindingRepairRequired)
        },
        WaitingStatusView::ApplicationTargetAttachmentRequired { .. } => {
            RolePresentationView::Unavailable(RoleUnavailableCause::TargetNotAttached)
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::num::NonZeroU32;
    use std::time::Duration;
    use std::time::Instant;

    use super::ConnectedCause;
    use super::RolePresentationView;
    use super::RoleUnavailableCause;
    use super::derive_role_presentation;
    use crate::AppliedKind;
    use crate::ApplySourceView;
    use crate::AttemptEndingView;
    use crate::AttemptInvalidationView;
    use crate::AttemptRef;
    use crate::AuthoredId;
    use crate::BatchRef;
    use crate::CapabilityProjectionFailure;
    use crate::ClaimHolderView;
    use crate::ContinuousFlowExpectation;
    use crate::ContinuousFlowExpiryCause;
    use crate::ContinuousFlowView;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DriverContractFailureView;
    use crate::DriverStopReasonView;
    use crate::EstablishedFlowView;
    use crate::FirstDatumTimeout;
    use crate::FlowExpectation;
    use crate::HardwareWait;
    use crate::KeyAvailability;
    use crate::MaximumDatumGap;
    use crate::NonEmptyContributors;
    use crate::NonEmptyReporterRefs;
    use crate::PermissionGateView;
    use crate::PresenceView;
    use crate::PresentEvidence;
    use crate::RegistrationApplicationRunView;
    use crate::ReporterId;
    use crate::ReporterRef;
    use crate::ResumeCondition;
    use crate::RetirementEvidence;
    use crate::RetryGateView;
    use crate::RetryScheduleView;
    use crate::RiggingRuntimeClock;
    use crate::RiggingRuntimeTime;
    use crate::RoleApplyFailureRunView;
    use crate::RoleKey;
    use crate::RoleStatusView;
    use crate::SessionRef;
    use crate::StoppedStatusView;
    use crate::UnconfirmedBasis;
    use crate::WaitTiming;
    use crate::WaitingStatusView;
    use crate::discovery::DiscoveryBatchId;
    use crate::flow::EstablishedFlow;
    use crate::flow::EstablishedFlowExpiry;

    /// The presentation every waiting role reads from availability alone, before its wait family.
    ///
    /// `None` names the one availability that hands the decision to the wait family instead.
    const AVAILABILITY_VERDICTS: [(&str, Option<RolePresentationView<RoleUnavailableCause>>); 6] = [
        ("Absent", Some(RolePresentationView::Disconnected)),
        ("DepartureGrace", Some(RolePresentationView::Disconnected)),
        ("AwaitingFirstReport", Some(RolePresentationView::Scanning)),
        (
            "Unconfirmed",
            Some(RolePresentationView::Unavailable(
                RoleUnavailableCause::IdentityUnconfirmed,
            )),
        ),
        (
            "Unreachable",
            Some(RolePresentationView::Unavailable(
                RoleUnavailableCause::DeviceUnreachable,
            )),
        ),
        ("Present", None),
    ];

    /// Every wait family under a present key, with the presentation it must derive.
    const PRESENT_KEY_WAIT_VERDICTS: [(&str, RolePresentationView<RoleUnavailableCause>); 15] = [
        ("Reporter", RolePresentationView::Connecting),
        ("KernelRetry", RolePresentationView::Connecting),
        ("NewRegistration", RolePresentationView::Connecting),
        ("DriverRepair", RolePresentationView::Connecting),
        (
            "KernelStateRepairRequired",
            RolePresentationView::Connecting,
        ),
        (
            "OperatorDecision",
            RolePresentationView::Unavailable(RoleUnavailableCause::OperatorDecisionRequired),
        ),
        (
            "ClaimRelease",
            RolePresentationView::Unavailable(RoleUnavailableCause::ClaimHeldElsewhere),
        ),
        (
            "ApplicationPermissionRequired",
            RolePresentationView::Unavailable(RoleUnavailableCause::PermissionRequired),
        ),
        (
            "ApplicationReapply",
            RolePresentationView::Unavailable(RoleUnavailableCause::ReapplyRequired),
        ),
        (
            "ApplicationReporterEnable",
            RolePresentationView::Unavailable(RoleUnavailableCause::ReporterNotEnabled),
        ),
        (
            "ApplicationReporterRegistrationRequired",
            RolePresentationView::Unavailable(RoleUnavailableCause::ReporterRegistrationRequired),
        ),
        (
            "ApplicationCapabilityRegistrationRequired",
            RolePresentationView::Unavailable(RoleUnavailableCause::CapabilityRegistrationRequired),
        ),
        (
            "ApplicationDeviceEnableRequired",
            RolePresentationView::Unavailable(RoleUnavailableCause::DeviceEnableRequired),
        ),
        (
            "ApplicationBindingRepairRequired",
            RolePresentationView::Unavailable(RoleUnavailableCause::BindingRepairRequired),
        ),
        (
            "ApplicationTargetAttachmentRequired",
            RolePresentationView::Unavailable(RoleUnavailableCause::TargetNotAttached),
        ),
    ];

    /// Name one availability conclusion.
    ///
    /// Exhaustive so a new [`KeyAvailability`] variant cannot reach production without a row in
    /// `AVAILABILITY_VERDICTS`.
    const fn availability_label(availability: &KeyAvailability) -> &'static str {
        match availability {
            KeyAvailability::Present(_) => "Present",
            KeyAvailability::DepartureGrace { .. } => "DepartureGrace",
            KeyAvailability::AwaitingFirstReport { .. } => "AwaitingFirstReport",
            KeyAvailability::Unconfirmed { .. } => "Unconfirmed",
            KeyAvailability::Unreachable { .. } => "Unreachable",
            KeyAvailability::Absent { .. } => "Absent",
        }
    }

    /// Name one wait family.
    ///
    /// Exhaustive so a new [`WaitingStatusView`] variant cannot reach production without a row in
    /// `PRESENT_KEY_WAIT_VERDICTS`.
    const fn wait_family_label(waiting: &WaitingStatusView) -> &'static str {
        match waiting {
            WaitingStatusView::Reporter(_) => "Reporter",
            WaitingStatusView::KernelRetry { .. } => "KernelRetry",
            WaitingStatusView::NewRegistration { .. } => "NewRegistration",
            WaitingStatusView::DriverRepair { .. } => "DriverRepair",
            WaitingStatusView::KernelStateRepairRequired { .. } => "KernelStateRepairRequired",
            WaitingStatusView::OperatorDecision { .. } => "OperatorDecision",
            WaitingStatusView::ClaimRelease { .. } => "ClaimRelease",
            WaitingStatusView::ApplicationPermissionRequired { .. } => {
                "ApplicationPermissionRequired"
            },
            WaitingStatusView::ApplicationReapply { .. } => "ApplicationReapply",
            WaitingStatusView::ApplicationReporterEnable { .. } => "ApplicationReporterEnable",
            WaitingStatusView::ApplicationReporterRegistrationRequired { .. } => {
                "ApplicationReporterRegistrationRequired"
            },
            WaitingStatusView::ApplicationCapabilityRegistrationRequired { .. } => {
                "ApplicationCapabilityRegistrationRequired"
            },
            WaitingStatusView::ApplicationDeviceEnableRequired { .. } => {
                "ApplicationDeviceEnableRequired"
            },
            WaitingStatusView::ApplicationBindingRepairRequired { .. } => {
                "ApplicationBindingRepairRequired"
            },
            WaitingStatusView::ApplicationTargetAttachmentRequired { .. } => {
                "ApplicationTargetAttachmentRequired"
            },
        }
    }

    fn runtime_time(seconds: u64) -> RiggingRuntimeTime {
        RiggingRuntimeTime::from_elapsed(Duration::from_secs(seconds))
    }

    fn sample_reporter() -> ReporterRef { ReporterRef::from_reporter_id(ReporterId(1)) }

    fn sample_batch() -> BatchRef { BatchRef::from_batch_id(DiscoveryBatchId(1)) }

    fn sample_evidence() -> RetirementEvidence {
        RetirementEvidence::new(sample_reporter(), sample_batch())
    }

    fn sample_reporters() -> NonEmptyReporterRefs {
        NonEmptyReporterRefs::from_first_and_rest(ReporterId(1), &[])
    }

    fn sample_timing() -> WaitTiming {
        WaitTiming::Bounded {
            since:    runtime_time(0),
            deadline: runtime_time(1),
        }
    }

    fn sample_device_key() -> Result<DeviceKey, Box<dyn Error>> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("truth-table-device")?,
            },
        })
    }

    fn sample_present_evidence() -> Result<PresentEvidence, Box<dyn Error>> {
        let contributors =
            NonEmptyContributors::from_contributors(vec![crate::ContributorView::new(
                sample_reporter(),
                sample_batch(),
                PresenceView::Present,
            )])
            .map_err(|()| "a single contributor is not empty")?;
        Ok(PresentEvidence::new(contributors))
    }

    /// One sample of every [`KeyAvailability`] variant, labelled to match `AVAILABILITY_VERDICTS`.
    fn every_availability() -> Result<Vec<KeyAvailability>, Box<dyn Error>> {
        Ok(vec![
            KeyAvailability::Present(sample_present_evidence()?),
            KeyAvailability::DepartureGrace {
                since:    runtime_time(0),
                deadline: runtime_time(1),
                evidence: sample_evidence(),
            },
            KeyAvailability::AwaitingFirstReport {
                since:     runtime_time(0),
                reporters: sample_reporters(),
            },
            KeyAvailability::Unconfirmed {
                since: runtime_time(0),
                basis: UnconfirmedBasis::NoFreshEvidence,
            },
            KeyAvailability::Unreachable {
                since:     runtime_time(0),
                reporters: sample_reporters(),
            },
            KeyAvailability::Absent {
                since:          runtime_time(0),
                established_by: sample_evidence(),
            },
        ])
    }

    /// One sample of every [`WaitingStatusView`] family, labelled by `wait_family_label`.
    fn every_wait_family() -> Result<Vec<WaitingStatusView>, Box<dyn Error>> {
        Ok(vec![
            WaitingStatusView::Reporter(HardwareWait::AwaitingFirstReport {
                key:       sample_device_key()?,
                reporters: sample_reporters(),
                timing:    sample_timing(),
            }),
            WaitingStatusView::KernelRetry {
                timing:   sample_timing(),
                gate:     RetryGateView::TimeReached {
                    retry_at: runtime_time(2),
                },
                failures: RoleApplyFailureRunView::Clear,
            },
            WaitingStatusView::NewRegistration {
                timing: sample_timing(),
            },
            WaitingStatusView::DriverRepair {
                error:  DriverContractFailureView::DriverTypeMismatch {
                    expected_driver: "TruthTableDriver".to_owned(),
                },
                timing: sample_timing(),
                retry:  RetryScheduleView::Ready,
            },
            WaitingStatusView::KernelStateRepairRequired {
                timing: sample_timing(),
            },
            WaitingStatusView::OperatorDecision {
                role:      RoleKey::new("truth-table-role")?,
                candidate: sample_device_key()?,
                timing:    sample_timing(),
            },
            WaitingStatusView::ClaimRelease {
                holder: ClaimHolderView::Unidentified,
                timing: sample_timing(),
            },
            WaitingStatusView::ApplicationPermissionRequired {
                gate:   PermissionGateView::ScreenRecording,
                timing: sample_timing(),
            },
            WaitingStatusView::ApplicationReapply {
                timing: sample_timing(),
            },
            WaitingStatusView::ApplicationReporterEnable {
                reporters: sample_reporters(),
                timing:    sample_timing(),
            },
            WaitingStatusView::ApplicationReporterRegistrationRequired {
                key:    sample_device_key()?,
                timing: sample_timing(),
            },
            WaitingStatusView::ApplicationCapabilityRegistrationRequired {
                failure: CapabilityProjectionFailure::ReflectComponentNotRegistered {
                    type_path: "truth_table::Capability".to_owned(),
                },
                timing:  sample_timing(),
            },
            WaitingStatusView::ApplicationDeviceEnableRequired {
                key:    sample_device_key()?,
                timing: sample_timing(),
            },
            WaitingStatusView::ApplicationBindingRepairRequired {
                timing: sample_timing(),
                retry:  RetryScheduleView::Ready,
            },
            WaitingStatusView::ApplicationTargetAttachmentRequired {
                timing: sample_timing(),
                retry:  RetryScheduleView::Ready,
            },
        ])
    }

    /// One sample of both [`StoppedStatusView`] variants, with the cause each must derive.
    fn every_stopped_status()
    -> Result<Vec<(StoppedStatusView, RoleUnavailableCause)>, Box<dyn Error>> {
        let failures = NonZeroU32::new(3).ok_or("three is not zero")?;
        Ok(vec![
            (
                StoppedStatusView::RepeatedFailures {
                    failures,
                    last_ending: AttemptEndingView::Invalidated(
                        AttemptInvalidationView::OverrunExhausted,
                    ),
                    resumes_when: ResumeCondition::ExplicitRestartOrReacquisition,
                },
                RoleUnavailableCause::RepeatedFailures,
            ),
            (
                StoppedStatusView::Unsupported {
                    reason:       DriverStopReasonView::Unsupported {
                        detail: "this platform has no such endpoint".to_owned(),
                    },
                    resumes_when: ResumeCondition::ExplicitRestart,
                },
                RoleUnavailableCause::Unsupported,
            ),
        ])
    }

    fn established_with(flow: EstablishedFlowView) -> RoleStatusView {
        RoleStatusView::Established {
            since: runtime_time(0),
            session: SessionRef::new(1),
            applied: AppliedKind::AsDispatched,
            flow,
        }
    }

    fn applying_status() -> RoleStatusView {
        RoleStatusView::Applying {
            attempt:         AttemptRef::new(1),
            since:           runtime_time(0),
            deadline:        runtime_time(1),
            source:          ApplySourceView::Requested,
            application_run: RegistrationApplicationRunView::Initial,
        }
    }

    /// One sample of every flow state, with the presentation a live session must derive.
    fn every_flow_state() -> [(
        EstablishedFlowView,
        RolePresentationView<RoleUnavailableCause>,
    ); 4] {
        [
            (
                EstablishedFlowView::NotMonitored,
                RolePresentationView::Connected(ConnectedCause::FlowNotMonitored),
            ),
            (
                EstablishedFlowView::Continuous(ContinuousFlowView::AwaitingFirstDatum {
                    deadline: runtime_time(5),
                }),
                RolePresentationView::Connected(ConnectedCause::AwaitingFirstDatum),
            ),
            (
                EstablishedFlowView::Continuous(ContinuousFlowView::Flowing {
                    flowing_since: runtime_time(1),
                }),
                RolePresentationView::Presenting,
            ),
            (
                EstablishedFlowView::Continuous(ContinuousFlowView::Stalled {
                    since: runtime_time(6),
                    cause: ContinuousFlowExpiryCause::FirstDatumOverdue,
                }),
                RolePresentationView::Connected(ConnectedCause::Stalled(
                    ContinuousFlowExpiryCause::FirstDatumOverdue,
                )),
            ),
        ]
    }

    fn verdict_for(
        label: &str,
    ) -> Result<RolePresentationView<RoleUnavailableCause>, Box<dyn Error>> {
        AVAILABILITY_VERDICTS
            .iter()
            .find(|(availability_label, _)| *availability_label == label)
            .and_then(|(_, verdict)| *verdict)
            .ok_or_else(|| format!("`{label}` decides nothing on its own").into())
    }

    #[test]
    fn every_availability_variant_has_a_truth_table_row() -> Result<(), Box<dyn Error>> {
        let sampled = every_availability()?
            .iter()
            .map(availability_label)
            .collect::<BTreeSet<_>>();
        let tabulated = AVAILABILITY_VERDICTS
            .iter()
            .map(|(label, _)| *label)
            .collect::<BTreeSet<_>>();

        assert_eq!(sampled, tabulated);
        Ok(())
    }

    #[test]
    fn every_wait_family_has_a_truth_table_row() -> Result<(), Box<dyn Error>> {
        let sampled = every_wait_family()?
            .iter()
            .map(wait_family_label)
            .collect::<BTreeSet<_>>();
        let tabulated = PRESENT_KEY_WAIT_VERDICTS
            .iter()
            .map(|(label, _)| *label)
            .collect::<BTreeSet<_>>();

        assert_eq!(sampled, tabulated);
        Ok(())
    }

    #[test]
    fn a_stopped_role_presents_its_stop_cause_under_every_availability()
    -> Result<(), Box<dyn Error>> {
        for availability in every_availability()? {
            for (stopped, cause) in every_stopped_status()? {
                let status = RoleStatusView::Stopped(stopped.clone());

                assert_eq!(
                    derive_role_presentation(&status, availability.clone()),
                    RolePresentationView::Unavailable(cause),
                    "stopped `{stopped:?}` under `{}`",
                    availability_label(&availability)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn a_live_session_presents_its_flow_under_every_availability() -> Result<(), Box<dyn Error>> {
        for availability in every_availability()? {
            for (flow, expected) in every_flow_state() {
                let status = established_with(flow);

                assert_eq!(
                    derive_role_presentation(&status, availability.clone()),
                    expected,
                    "established `{flow:?}` under `{}`",
                    availability_label(&availability)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn an_apply_in_flight_presents_as_connecting_under_every_availability()
    -> Result<(), Box<dyn Error>> {
        for availability in every_availability()? {
            assert_eq!(
                derive_role_presentation(&applying_status(), availability.clone()),
                RolePresentationView::Connecting,
                "applying under `{}`",
                availability_label(&availability)
            );
        }
        Ok(())
    }

    #[test]
    fn a_wait_under_a_present_key_splits_on_who_must_act() -> Result<(), Box<dyn Error>> {
        let availability = KeyAvailability::Present(sample_present_evidence()?);
        for waiting in every_wait_family()? {
            let label = wait_family_label(&waiting);
            let (_, expected) = PRESENT_KEY_WAIT_VERDICTS
                .iter()
                .find(|(row, _)| *row == label)
                .ok_or_else(|| format!("`{label}` has no truth-table row"))?;

            assert_eq!(
                derive_role_presentation(
                    &RoleStatusView::Waiting(waiting.clone()),
                    availability.clone()
                ),
                *expected,
                "waiting `{label}` under a present key"
            );
        }
        Ok(())
    }

    #[test]
    fn a_wait_reads_availability_before_its_family() -> Result<(), Box<dyn Error>> {
        for availability in every_availability()? {
            let label = availability_label(&availability);
            if label == "Present" {
                continue;
            }
            let expected = verdict_for(label)?;
            for waiting in every_wait_family()? {
                assert_eq!(
                    derive_role_presentation(
                        &RoleStatusView::Waiting(waiting.clone()),
                        availability.clone()
                    ),
                    expected,
                    "waiting `{}` under `{label}`",
                    wait_family_label(&waiting)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn the_two_stall_causes_present_as_distinct_connected_payloads() {
        let first_datum_overdue = established_with(EstablishedFlowView::Continuous(
            ContinuousFlowView::Stalled {
                since: runtime_time(5),
                cause: ContinuousFlowExpiryCause::FirstDatumOverdue,
            },
        ));
        let maximum_gap_exceeded = established_with(EstablishedFlowView::Continuous(
            ContinuousFlowView::Stalled {
                since: runtime_time(5),
                cause: ContinuousFlowExpiryCause::MaximumDatumGapExceeded,
            },
        ));
        let availability = KeyAvailability::AwaitingFirstReport {
            since:     runtime_time(0),
            reporters: sample_reporters(),
        };

        let first = derive_role_presentation(&first_datum_overdue, availability.clone());
        let second = derive_role_presentation(&maximum_gap_exceeded, availability);

        assert_eq!(
            first,
            RolePresentationView::Connected(ConnectedCause::Stalled(
                ContinuousFlowExpiryCause::FirstDatumOverdue
            ))
        );
        assert_eq!(
            second,
            RolePresentationView::Connected(ConnectedCause::Stalled(
                ContinuousFlowExpiryCause::MaximumDatumGapExceeded
            ))
        );
        assert_ne!(first, second);
    }

    #[test]
    fn an_unmonitored_expectation_can_never_publish_a_continuous_flow() {
        let established_at = Instant::now();
        let flow = EstablishedFlow::new(FlowExpectation::NotMonitored, established_at);

        assert_eq!(
            flow.view(RiggingRuntimeClock::starting_at(established_at)),
            EstablishedFlowView::NotMonitored
        );
    }

    #[test]
    fn a_continuous_expectation_can_never_publish_an_unmonitored_flow() -> Result<(), Box<dyn Error>>
    {
        let established_at = Instant::now();
        let expectation = ContinuousFlowExpectation::new(
            FirstDatumTimeout::new(Duration::from_secs(5))?,
            MaximumDatumGap::new(Duration::from_secs(3))?,
        );
        let mut flow =
            EstablishedFlow::new(FlowExpectation::Continuous(expectation), established_at);
        let runtime_clock = RiggingRuntimeClock::starting_at(established_at);

        assert!(matches!(
            flow.view(runtime_clock),
            EstablishedFlowView::Continuous(_)
        ));
        flow.record_datum_arrival(established_at + Duration::from_secs(1));
        assert!(matches!(
            flow.view(runtime_clock),
            EstablishedFlowView::Continuous(_)
        ));
        assert_eq!(
            flow.evaluate_expiry(established_at + Duration::from_secs(60)),
            EstablishedFlowExpiry::Crossed(ContinuousFlowExpiryCause::MaximumDatumGapExceeded)
        );
        assert!(matches!(
            flow.view(runtime_clock),
            EstablishedFlowView::Continuous(_)
        ));
        Ok(())
    }

    #[test]
    fn map_unavailable_widens_only_the_unavailable_payload() {
        #[derive(Debug, PartialEq)]
        enum WiderCause {
            Kernel(RoleUnavailableCause),
        }

        let unavailable = super::RolePresentation::from_view(RolePresentationView::Unavailable(
            RoleUnavailableCause::PermissionRequired,
        ));
        let presenting = super::RolePresentation::from_view(
            RolePresentationView::<RoleUnavailableCause>::Presenting,
        );

        assert_eq!(
            unavailable.map_unavailable(WiderCause::Kernel).view(),
            &RolePresentationView::Unavailable(WiderCause::Kernel(
                RoleUnavailableCause::PermissionRequired
            ))
        );
        assert_eq!(
            presenting.map_unavailable(WiderCause::Kernel).view(),
            &RolePresentationView::Presenting
        );
    }
}
