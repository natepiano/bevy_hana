use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result as FormatResult;
use std::num::NonZeroU32;
use std::time::Duration;
use std::time::Instant;

use bevy::ecs::reflect::ReflectComponent;
use bevy::log::info;
use bevy::log::warn;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;
use bevy::reflect::ReflectSerialize;
use serde::Serialize;

use crate::AuthoritativeReporterCoverage;
use crate::Bindings;
use crate::CapabilityProjectionFailure;
use crate::CoveredDeviceIdentitySpace;
use crate::DeviceAccessError;
use crate::DeviceKind;
use crate::DiscoveryBatchId;
use crate::DiscoveryCadence;
use crate::DiscoveryProgress;
use crate::ReporterActivation;
use crate::ReporterCoverage;
use crate::ReporterDeferral;
use crate::ReporterId;
use crate::ReporterRegistration;
use crate::RoleKey;
use crate::RoleStatusView;
use crate::SchemeName;
use crate::WaitingStatusView;
use crate::presence::RetainedSetChange;

/// A timestamp measured from the rigging runtime's process-local start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize)]
pub struct RiggingRuntimeTime(Duration);

impl RiggingRuntimeTime {
    /// Construct one diagnostic timestamp from elapsed process-runtime time.
    #[must_use]
    pub const fn from_elapsed(elapsed: Duration) -> Self { Self(elapsed) }

    /// Return the duration since the rigging runtime started.
    #[must_use]
    pub const fn elapsed(self) -> Duration { self.0 }

    const fn after(self, duration: Duration) -> Self { Self(self.0.saturating_add(duration)) }
}

/// The single process-local clock that produces rigging diagnostic timestamps.
#[derive(Resource, Clone, Copy)]
pub struct RiggingRuntimeClock {
    started_at: Instant,
}

impl RiggingRuntimeClock {
    pub(crate) const fn starting_at(started_at: Instant) -> Self { Self { started_at } }

    /// Convert an instant from this process into rigging runtime time.
    #[must_use]
    pub fn time_at(self, instant: Instant) -> RiggingRuntimeTime {
        RiggingRuntimeTime::from_elapsed(instant.saturating_duration_since(self.started_at))
    }

    /// Convert one retained runtime timestamp back to this process clock's instant.
    pub(crate) fn instant_at(self, runtime_time: RiggingRuntimeTime) -> Instant {
        self.started_at
            .checked_add(runtime_time.elapsed())
            .unwrap_or(self.started_at)
    }
}

/// Public diagnostic identity for one registered reporter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize)]
pub struct ReporterRef(u32);

impl ReporterRef {
    pub(crate) const fn from_reporter_id(reporter: ReporterId) -> Self { Self(reporter.0) }

    /// Return the reporter registry's process-local number.
    #[must_use]
    pub const fn get(self) -> u32 { self.0 }
}

/// Public diagnostic identity for one accepted discovery batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Reflect)]
#[reflect(opaque)]
#[reflect(Serialize)]
pub struct BatchRef(u64);

impl BatchRef {
    pub(crate) const fn from_batch_id(batch: DiscoveryBatchId) -> Self { Self(batch.0) }

    /// Return the reporter registry's process-local batch number.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

/// Whether startup waits for this reporter's first complete set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum StartupRequirement {
    /// Startup remains closed until this reporter supplies a complete set.
    Required,
    /// Application policy may leave this reporter disabled without blocking startup.
    Optional,
}

/// Milestone most recently emitted for one uninterrupted failure run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureLogMilestoneIndex {
    /// The run has not reached its first summary time.
    BeforeFirst,
    /// The run reached one minute.
    OneMinute,
    /// The run reached five minutes.
    FiveMinutes,
    /// The run reached fifteen minutes.
    FifteenMinutes,
    /// The run reached this many complete hours.
    Hour(u64),
}

impl FailureLogMilestoneIndex {
    fn reached_at(elapsed: Duration) -> Self {
        const ONE_MINUTE: Duration = Duration::from_secs(60);
        const FIVE_MINUTES: Duration = Duration::from_mins(5);
        const FIFTEEN_MINUTES: Duration = Duration::from_mins(15);
        const ONE_HOUR: Duration = Duration::from_hours(1);

        if elapsed >= ONE_HOUR {
            return Self::Hour(elapsed.as_secs() / ONE_HOUR.as_secs());
        }
        if elapsed >= FIFTEEN_MINUTES {
            return Self::FifteenMinutes;
        }
        if elapsed >= FIVE_MINUTES {
            return Self::FiveMinutes;
        }
        if elapsed >= ONE_MINUTE {
            return Self::OneMinute;
        }

        Self::BeforeFirst
    }
}

/// Whether a one-shot reporter diagnostic has already been emitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OneShotLogState {
    /// No diagnostic has been emitted.
    #[default]
    Pending,
    /// The diagnostic has been emitted.
    Emitted,
}

/// Error text and elapsed milestone retained across one uninterrupted failure run.
#[derive(Debug, PartialEq, Eq)]
struct FailureLogState {
    last_logged_error:     LoggedFailureError,
    last_logged_milestone: FailureLogMilestoneIndex,
}

impl Default for FailureLogState {
    fn default() -> Self {
        Self {
            last_logged_error:     LoggedFailureError::NoFailure,
            last_logged_milestone: FailureLogMilestoneIndex::BeforeFirst,
        }
    }
}

impl FailureLogState {
    fn decisions(
        &mut self,
        error: &DeviceAccessErrorView,
        failure_elapsed: Duration,
    ) -> FailureLogDecisions {
        let error_text = format!("{error:?}");
        let error = match &self.last_logged_error {
            LoggedFailureError::NoFailure => ReporterLogDecision::FirstFailure,
            LoggedFailureError::Text(previous) if previous != &error_text => {
                ReporterLogDecision::ChangedError
            },
            LoggedFailureError::Text(_) => ReporterLogDecision::NoLog,
        };
        self.last_logged_error = LoggedFailureError::Text(error_text);

        let reached = FailureLogMilestoneIndex::reached_at(failure_elapsed);
        let milestone = if reached == FailureLogMilestoneIndex::BeforeFirst
            || reached == self.last_logged_milestone
        {
            ReporterLogDecision::NoLog
        } else {
            self.last_logged_milestone = reached;
            ReporterLogDecision::FailureMilestone(reached)
        };

        FailureLogDecisions { error, milestone }
    }
}

/// Count, timing, and emitted diagnostics for one ordinary failure run.
#[derive(Debug, Default, PartialEq, Eq)]
enum FailureRunState {
    /// No ordinary failure run is active.
    #[default]
    Inactive,
    /// Ordinary failures have continued without an accepted non-failure outcome.
    Active {
        consecutive:      NonZeroU32,
        first_failure_at: RiggingRuntimeTime,
        log_state:        FailureLogState,
    },
}

/// Single owner of the state that starts, advances, and ends an ordinary reporter failure run.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReporterFailureRun {
    state:                 FailureRunState,
    #[cfg(test)]
    most_recent_decisions: FailureLogDecisions,
}

impl ReporterFailureRun {
    pub(crate) fn accept_failure(
        &mut self,
        error: DeviceAccessErrorView,
        failed_at: RiggingRuntimeTime,
        previous_success: PreviousSuccess,
    ) -> AcceptedFailureRun {
        let (consecutive, first_failure_at, mut log_state) = match std::mem::take(&mut self.state) {
            FailureRunState::Inactive => (NonZeroU32::MIN, failed_at, FailureLogState::default()),
            FailureRunState::Active {
                consecutive,
                first_failure_at,
                log_state,
            } => (
                NonZeroU32::new(consecutive.get().saturating_add(1)).unwrap_or(NonZeroU32::MAX),
                first_failure_at,
                log_state,
            ),
        };
        let failure_elapsed = failed_at
            .elapsed()
            .saturating_sub(first_failure_at.elapsed());
        let decisions = log_state.decisions(&error, failure_elapsed);
        self.state = FailureRunState::Active {
            consecutive,
            first_failure_at,
            log_state,
        };
        #[cfg(test)]
        {
            self.most_recent_decisions = decisions;
        }

        AcceptedFailureRun {
            status: FailureRunStatus {
                consecutive,
                first_failure_at,
                last_failure_at: failed_at,
                error,
                previous_success,
            },
            decisions,
        }
    }

    pub(crate) fn finish_as_unsupported(
        &mut self,
        error: &DeviceAccessErrorView,
    ) -> ReporterLogDecision {
        let mut log_state = match std::mem::take(&mut self.state) {
            FailureRunState::Inactive => FailureLogState::default(),
            FailureRunState::Active { log_state, .. } => log_state,
        };
        let decision = log_state.decisions(error, Duration::ZERO).error;
        #[cfg(test)]
        {
            self.most_recent_decisions = FailureLogDecisions::default();
        }
        decision
    }

    pub(crate) fn end(&mut self) {
        self.state = FailureRunState::Inactive;
        #[cfg(test)]
        {
            self.most_recent_decisions = FailureLogDecisions::default();
        }
    }

    #[cfg(test)]
    pub(crate) fn accept_failure_at_elapsed(
        &mut self,
        error: &DeviceAccessErrorView,
        failure_elapsed: Duration,
    ) -> FailureLogDecisions {
        self.accept_failure(
            error.clone(),
            RiggingRuntimeTime::from_elapsed(failure_elapsed),
            PreviousSuccess::Never,
        )
        .decisions
    }

    #[cfg(test)]
    pub(crate) const fn last_logged_milestone(&self) -> FailureLogMilestoneIndex {
        match &self.state {
            FailureRunState::Inactive => FailureLogMilestoneIndex::BeforeFirst,
            FailureRunState::Active { log_state, .. } => log_state.last_logged_milestone,
        }
    }

    #[cfg(test)]
    pub(crate) const fn most_recent_decisions(&self) -> FailureLogDecisions {
        self.most_recent_decisions
    }
}

/// Error text most recently emitted for the current failure run.
#[derive(Debug, PartialEq, Eq)]
enum LoggedFailureError {
    /// No failure in the current run has been emitted.
    NoFailure,
    /// This exact structured error text was emitted.
    Text(String),
}

/// Whether the reporter registry has accepted a successful set and what its count was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LastRetainedRecordCount {
    /// No successful complete set has been accepted.
    #[default]
    NoCompleteSet,
    /// The latest successful complete set retained this many records.
    Count(usize),
}

/// Whether an accepted success follows another success or ends a non-success outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SuccessfulReportContext {
    /// The preceding accepted outcome was a success or no outcome has completed.
    Ordinary,
    /// The preceding accepted outcome was a failure, unsupported result, or deferral.
    Recovery,
}

/// Typed reason a reporter log call emits or remains silent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ReporterLogDecision {
    /// The accepted outcome adds no reporter diagnostic.
    #[default]
    NoLog,
    /// This is the first failure in an uninterrupted run.
    FirstFailure,
    /// The failure's structured error text changed.
    ChangedError,
    /// The failure run reached a bounded summary milestone.
    FailureMilestone(FailureLogMilestoneIndex),
    /// The reporter supplied its first complete set.
    FirstSuccess,
    /// A complete set ended a failure or deferral.
    Recovery,
    /// A later complete set changed retained records.
    ChangedRecordSet,
    /// A nonempty retained set became empty.
    BecameEmpty,
    /// The first-complete-set deadline was crossed.
    FirstCompleteSetOverdue,
}

/// Private comparison state that turns accepted outcomes into edge and milestone diagnostics.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReporterLogState {
    last_retained_count:        LastRetainedRecordCount,
    first_complete_set_overdue: OneShotLogState,
    #[cfg(test)]
    most_recent_overdue:        ReporterLogDecision,
}

impl ReporterLogState {
    pub(crate) const fn accepted_success_decisions(
        &mut self,
        retained_set_change: RetainedSetChange,
        record_count: usize,
        context: SuccessfulReportContext,
    ) -> AcceptedSuccessLogDecisions {
        let previous_count = self.last_retained_count;
        let info = match (previous_count, context, retained_set_change) {
            (LastRetainedRecordCount::NoCompleteSet, _, _) => ReporterLogDecision::FirstSuccess,
            (_, SuccessfulReportContext::Recovery, _) => ReporterLogDecision::Recovery,
            (_, _, RetainedSetChange::Changed) => ReporterLogDecision::ChangedRecordSet,
            (LastRetainedRecordCount::Count(previous), _, RetainedSetChange::Unchanged)
                if previous != record_count =>
            {
                ReporterLogDecision::ChangedRecordSet
            },
            _ => ReporterLogDecision::NoLog,
        };
        let empty_warning = match previous_count {
            LastRetainedRecordCount::Count(previous) if previous > 0 && record_count == 0 => {
                ReporterLogDecision::BecameEmpty
            },
            LastRetainedRecordCount::NoCompleteSet | LastRetainedRecordCount::Count(_) => {
                ReporterLogDecision::NoLog
            },
        };

        self.last_retained_count = LastRetainedRecordCount::Count(record_count);

        AcceptedSuccessLogDecisions {
            info,
            empty_warning,
        }
    }

    pub(crate) const fn first_complete_set_overdue_decision(&mut self) -> ReporterLogDecision {
        let decision = match self.first_complete_set_overdue {
            OneShotLogState::Pending => {
                self.first_complete_set_overdue = OneShotLogState::Emitted;
                ReporterLogDecision::FirstCompleteSetOverdue
            },
            OneShotLogState::Emitted => ReporterLogDecision::NoLog,
        };
        #[cfg(test)]
        {
            self.most_recent_overdue = decision;
        }
        decision
    }

    pub(crate) const fn restart_first_complete_set_bound(&mut self) {
        self.first_complete_set_overdue = OneShotLogState::Pending;
        #[cfg(test)]
        {
            self.most_recent_overdue = ReporterLogDecision::NoLog;
        }
    }

    #[cfg(test)]
    pub(crate) const fn most_recent_overdue_decision(&self) -> ReporterLogDecision {
        self.most_recent_overdue
    }
}

/// Independent failure-edge and milestone decisions for one accepted failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FailureLogDecisions {
    pub(crate) error:     ReporterLogDecision,
    pub(crate) milestone: ReporterLogDecision,
}

/// Published failure-run data and log decisions produced by one accepted ordinary failure.
pub(crate) struct AcceptedFailureRun {
    pub(crate) status:    FailureRunStatus,
    pub(crate) decisions: FailureLogDecisions,
}

/// Information and empty-transition decisions for one accepted complete set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AcceptedSuccessLogDecisions {
    info:          ReporterLogDecision,
    empty_warning: ReporterLogDecision,
}

/// One or more authored roles sorted by their readable keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NonEmptyRoleKeys(Vec<RoleKey>);

impl Display for NonEmptyRoleKeys {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult {
        for (index, role) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            Display::fmt(role, formatter)?;
        }
        Ok(())
    }
}

/// Authored roles whose published reporter wait names one reporter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RolesWaitingOnReporter {
    /// No role is waiting on this reporter.
    NoneWaiting,
    /// At least one active authored role waits on this reporter.
    Waiting(NonEmptyRoleKeys),
}

impl Display for RolesWaitingOnReporter {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult {
        match self {
            Self::NoneWaiting => formatter.write_str("no role is waiting on this reporter"),
            Self::Waiting(roles) => Display::fmt(roles, formatter),
        }
    }
}

/// Find active authored roles whose readable status names `reporter`.
pub(crate) fn roles_waiting_on(
    bindings: &Bindings,
    reporter: ReporterRef,
) -> RolesWaitingOnReporter {
    let mut roles = bindings
        .registered_roles()
        .filter(|role| {
            matches!(
                bindings.projected_status(role),
                Ok(RoleStatusView::Waiting(WaitingStatusView::Reporter(wait)))
                    if wait.names_reporter(reporter)
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    roles.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    if roles.is_empty() {
        RolesWaitingOnReporter::NoneWaiting
    } else {
        RolesWaitingOnReporter::Waiting(NonEmptyRoleKeys(roles))
    }
}

/// Emit the edge and milestone diagnostics selected for one accepted ordinary failure.
pub(crate) fn log_reporter_failure(
    reporter_health: &ReporterHealth,
    decisions: FailureLogDecisions,
    roles_waiting: &RolesWaitingOnReporter,
) {
    log_reporter_failure_decision(reporter_health, decisions.error, roles_waiting);
    log_reporter_failure_decision(reporter_health, decisions.milestone, roles_waiting);
}

fn log_reporter_failure_decision(
    reporter_health: &ReporterHealth,
    decision: ReporterLogDecision,
    roles_waiting: &RolesWaitingOnReporter,
) {
    let ReporterOutcomeHealth::Failing {
        batch,
        duration,
        run,
    } = &reporter_health.outcome
    else {
        return;
    };
    let reason = match decision {
        ReporterLogDecision::FirstFailure => "first failure",
        ReporterLogDecision::ChangedError => "changed error",
        ReporterLogDecision::FailureMilestone(FailureLogMilestoneIndex::OneMinute) => {
            "one-minute failure milestone"
        },
        ReporterLogDecision::FailureMilestone(FailureLogMilestoneIndex::FiveMinutes) => {
            "five-minute failure milestone"
        },
        ReporterLogDecision::FailureMilestone(FailureLogMilestoneIndex::FifteenMinutes) => {
            "fifteen-minute failure milestone"
        },
        ReporterLogDecision::FailureMilestone(FailureLogMilestoneIndex::Hour(_)) => {
            "hourly failure milestone"
        },
        ReporterLogDecision::NoLog
        | ReporterLogDecision::FailureMilestone(FailureLogMilestoneIndex::BeforeFirst)
        | ReporterLogDecision::FirstSuccess
        | ReporterLogDecision::Recovery
        | ReporterLogDecision::ChangedRecordSet
        | ReporterLogDecision::BecameEmpty
        | ReporterLogDecision::FirstCompleteSetOverdue => return,
    };
    let failure_elapsed = run
        .last_failure_at
        .elapsed()
        .saturating_sub(run.first_failure_at.elapsed());
    warn!(
        "[hana_rigging] reporter {} ({}) {reason}: batch {}, duration {duration:?}, error {:?}, \
         consecutive {}, retained {}, failure run {failure_elapsed:?}, affected roles: {}",
        reporter_health.identity.reporter_ref.get(),
        reporter_health.identity.name,
        batch.get(),
        run.error,
        run.consecutive,
        reporter_health.retained_records,
        roles_waiting,
    );
}

/// Emit the information and empty-transition diagnostics selected for one accepted complete set.
pub(crate) fn log_reporter_success(
    reporter_health: &ReporterHealth,
    decisions: AcceptedSuccessLogDecisions,
    roles_waiting: &RolesWaitingOnReporter,
) {
    let ReporterOutcomeHealth::Succeeded {
        batch,
        duration,
        records,
        ..
    } = reporter_health.outcome
    else {
        return;
    };
    let reason = match decisions.info {
        ReporterLogDecision::FirstSuccess => "first complete set",
        ReporterLogDecision::Recovery => "recovery",
        ReporterLogDecision::ChangedRecordSet => "changed record set",
        ReporterLogDecision::NoLog
        | ReporterLogDecision::FirstFailure
        | ReporterLogDecision::ChangedError
        | ReporterLogDecision::FailureMilestone(_)
        | ReporterLogDecision::BecameEmpty
        | ReporterLogDecision::FirstCompleteSetOverdue => "",
    };
    if decisions.info != ReporterLogDecision::NoLog {
        info!(
            "[hana_rigging] reporter {} ({}) accepted {reason}: batch {}, duration {duration:?}, \
             records {records}",
            reporter_health.identity.reporter_ref.get(),
            reporter_health.identity.name,
            batch.get(),
        );
    }
    if decisions.empty_warning == ReporterLogDecision::BecameEmpty {
        warn!(
            "[hana_rigging] reporter {} ({}) retained record count changed from nonzero to zero: \
             batch {}, duration {duration:?}, roles waiting: {}",
            reporter_health.identity.reporter_ref.get(),
            reporter_health.identity.name,
            batch.get(),
            roles_waiting,
        );
    }
}

/// Emit the information diagnostic for entry into a typed reporter deferral.
pub(crate) fn log_reporter_deferral(
    reporter_health: &ReporterHealth,
    batch: BatchRef,
    duration: Duration,
    deferral: ReporterDeferral,
) {
    info!(
        "[hana_rigging] reporter {} ({}) deferred: batch {}, duration {duration:?}, {deferral:?}",
        reporter_health.identity.reporter_ref.get(),
        reporter_health.identity.name,
        batch.get(),
    );
}

/// Emit the one-shot diagnostic for a reporter's crossed first-complete-set deadline.
pub(crate) fn log_first_complete_set_overdue(
    reporter_health: &ReporterHealth,
    decision: ReporterLogDecision,
    roles_waiting: &RolesWaitingOnReporter,
) {
    if decision != ReporterLogDecision::FirstCompleteSetOverdue {
        return;
    }
    let FirstCompleteSetStatus::Waiting(WaitTiming::Overdue {
        since,
        deadline,
        crossed_at,
    }) = reporter_health.first_complete_set
    else {
        return;
    };
    warn!(
        "[hana_rigging] reporter {} ({}) exceeded its first complete set bound: waiting since \
         {:?}, deadline {:?}, crossed at {:?}, roles waiting: {}",
        reporter_health.identity.reporter_ref.get(),
        reporter_health.identity.name,
        since.elapsed(),
        deadline.elapsed(),
        crossed_at.elapsed(),
        roles_waiting,
    );
}

/// Emit an unsupported failure edge selected from the same private failure comparison state.
pub(crate) fn log_unsupported_reporter_failure(
    reporter_health: &ReporterHealth,
    batch: BatchRef,
    duration: Duration,
    error: &DeviceAccessErrorView,
    decision: ReporterLogDecision,
    roles_waiting: &RolesWaitingOnReporter,
) {
    let reason = match decision {
        ReporterLogDecision::FirstFailure => "first failure",
        ReporterLogDecision::ChangedError => "changed error",
        ReporterLogDecision::NoLog
        | ReporterLogDecision::FailureMilestone(_)
        | ReporterLogDecision::FirstSuccess
        | ReporterLogDecision::Recovery
        | ReporterLogDecision::ChangedRecordSet
        | ReporterLogDecision::BecameEmpty
        | ReporterLogDecision::FirstCompleteSetOverdue => return,
    };
    warn!(
        "[hana_rigging] reporter {} ({}) stopped after {reason}: batch {}, duration {duration:?}, \
         error {error:?}, retained {}, affected roles: {}",
        reporter_health.identity.reporter_ref.get(),
        reporter_health.identity.name,
        batch.get(),
        reporter_health.retained_records,
        roles_waiting,
    );
}

/// BRP-readable health for one registered reporter.
#[derive(Component, Clone, PartialEq, Reflect, Serialize)]
#[reflect(Component, PartialEq, Serialize)]
#[expect(
    clippy::derive_partial_eq_without_eq,
    reason = "the published component's required derive contract names PartialEq without Eq"
)]
pub struct ReporterHealth {
    /// Registry-issued identity and implementation name.
    identity:           ReporterIdentityView,
    /// Immutable scheduling and coverage policy declared at registration.
    registration:       ReporterRegistrationView,
    /// Whether the reporter has supplied its first accepted complete set.
    first_complete_set: FirstCompleteSetStatus,
    /// What the scheduler is doing with the reporter now.
    activity:           ReporterActivityView,
    /// How the most recently accepted discovery run ended.
    outcome:            ReporterOutcomeHealth,
    /// Number of discovery runs whose outcomes the registry has accepted.
    completed_runs:     u64,
    /// Number of records in the last accepted complete set.
    retained_records:   usize,
}

/// Registry identity projected without exposing the reporter authority handle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub struct ReporterIdentityView {
    /// Process-local diagnostic reference assigned in registration order.
    pub reporter_ref: ReporterRef,
    /// Rust implementation type captured when the reporter was registered.
    pub name:         String,
}

/// Registration policy projected into data-only diagnostic values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub struct ReporterRegistrationView {
    /// Whether startup needs this reporter's first complete set.
    pub requirement:              StartupRequirement,
    /// Identity spaces for which omission has meaning.
    pub coverage:                 ReporterCoverageView,
    /// Scheduling policy for later scans.
    pub cadence:                  DiscoveryCadenceView,
    /// Maximum wait before the first complete set becomes overdue.
    pub first_complete_set_bound: Duration,
}

/// Serializable projection of a reporter's checked absence coverage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum ReporterCoverageView {
    /// Omission from this reporter proves nothing about authored inventory.
    MatchingEvidenceOnly,
    /// Omission proves absence inside this completely enumerated identity space.
    EstablishesAbsence(CoveredDeviceIdentitySpaceView),
}

/// Serializable projection of one checked identity space.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum CoveredDeviceIdentitySpaceView {
    /// Every durable key of this device kind is enumerated.
    AllKeysOfKind {
        /// Completely enumerated physical kind.
        kind: DeviceKind,
    },
    /// Every reported value in this scheme and device kind is enumerated.
    ReportedScheme {
        /// Completely enumerated physical kind.
        kind:   DeviceKind,
        /// Completely enumerated provider identity scheme.
        scheme: SchemeName,
    },
    /// Every synthesized key of this device kind is enumerated.
    SynthesizedKeysOfKind {
        /// Completely enumerated physical kind.
        kind: DeviceKind,
    },
    /// Every authored key of this device kind is enumerated.
    AuthoredKeysOfKind {
        /// Completely enumerated physical kind.
        kind: DeviceKind,
    },
}

/// Serializable projection of reporter scheduling cadence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum DiscoveryCadenceView {
    /// Every run requires an application request.
    OnDemand,
    /// Notifications request runs, with a periodic retry for missed notifications.
    EventDriven {
        /// Longest interval between notification-independent scans.
        backstop: Duration,
    },
    /// Runs become due at a fixed minimum interval.
    Periodic {
        /// Minimum interval between submissions.
        interval: Duration,
    },
}

/// Whether the first complete set is still awaited or has arrived.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum FirstCompleteSetStatus {
    /// The reporter has not supplied a complete set yet.
    Waiting(WaitTiming),
    /// The reporter supplied its first complete set in this batch.
    Completed {
        /// Batch containing the first complete set.
        batch: BatchRef,
        /// Runtime time when its discovery run completed.
        at:    RiggingRuntimeTime,
    },
}

/// Timing state for work that has not completed yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum WaitTiming {
    /// The wait is still inside its declared bound.
    Bounded {
        /// Time when the bound began.
        since:    RiggingRuntimeTime,
        /// Time when the bound becomes overdue.
        deadline: RiggingRuntimeTime,
    },
    /// The wait crossed its declared bound.
    Overdue {
        /// Time when the bound began.
        since:      RiggingRuntimeTime,
        /// Time when the bound became overdue.
        deadline:   RiggingRuntimeTime,
        /// First collect pass that observed the crossed deadline.
        crossed_at: RiggingRuntimeTime,
    },
    /// No bound runs until application policy enables this reporter.
    Unbounded {
        /// Time when the reporter entered this wait.
        since: RiggingRuntimeTime,
    },
}

/// Current scheduling activity projected without authority handles or instants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum ReporterActivityView {
    /// Application policy disabled this optional reporter.
    Disabled,
    /// The reporter is waiting for its next due signal.
    Idle,
    /// The reporter has joined a batch and awaits preparation or admission.
    Queued {
        /// Batch awaiting work.
        batch: BatchRef,
    },
    /// A discovery job is running.
    Running {
        /// Batch being discovered.
        batch:      BatchRef,
        /// Runtime time when work began.
        started_at: RiggingRuntimeTime,
        /// Most recent progress sent by the job.
        progress:   DiscoveryProgressView,
    },
    /// An unsupported integration stopped until application control resumes it.
    Stopped {
        /// Runtime time when the reporter stopped.
        since:        RiggingRuntimeTime,
        /// Control input that may resume discovery.
        resumes_when: ReporterResume,
    },
}

/// Data-only projection of background discovery progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum DiscoveryProgressView {
    /// The job cannot count its remaining operations.
    Indeterminate,
    /// The job reports a completed count against a nonzero total.
    Measured {
        /// Completed operations.
        completed: u32,
        /// Total operations.
        total:     NonZeroU32,
    },
}

/// Application input that can resume an unsupported reporter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum ReporterResume {
    /// An explicit request or a later enable asks the reporter to run again.
    ExplicitRequestOrEnable,
}

/// Checked non-empty capability projection failures from one accepted complete set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub struct CapabilityProjectionFailures {
    entries: Vec<CapabilityProjectionFailure>,
}

impl CapabilityProjectionFailures {
    fn new(
        first: CapabilityProjectionFailure,
        remaining: impl IntoIterator<Item = CapabilityProjectionFailure>,
    ) -> Self {
        let mut entries = vec![first];
        entries.extend(remaining);
        Self { entries }
    }

    /// Borrow the failures in ascending reflected type-path order.
    #[must_use]
    pub fn entries(&self) -> &[CapabilityProjectionFailure] { &self.entries }
}

/// Whether every capability in an accepted complete set reached reflected component projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum CapabilityProjectionStatus {
    /// Every reported capability was retained for entity projection.
    AllProjected,
    /// At least one capability lacked reflected component registration and was dropped.
    Failed(CapabilityProjectionFailures),
}

impl CapabilityProjectionStatus {
    pub(crate) fn from_failures(mut failures: Vec<CapabilityProjectionFailure>) -> Self {
        failures.sort_by(|left, right| {
            capability_projection_failure_type_path(left)
                .cmp(capability_projection_failure_type_path(right))
        });
        failures.dedup();

        let mut failures = failures.into_iter();
        let Some(first) = failures.next() else {
            return Self::AllProjected;
        };
        Self::Failed(CapabilityProjectionFailures::new(first, failures))
    }

    fn extend_failures(&mut self, failures: impl IntoIterator<Item = CapabilityProjectionFailure>) {
        let mut combined = match self {
            Self::AllProjected => Vec::new(),
            Self::Failed(failures) => failures.entries.clone(),
        };
        combined.extend(failures);
        *self = Self::from_failures(combined);
    }
}

fn capability_projection_failure_type_path(failure: &CapabilityProjectionFailure) -> &str {
    match failure {
        CapabilityProjectionFailure::ApplicationTypeRegistryUnavailable { affected_type_path } => {
            affected_type_path
        },
        CapabilityProjectionFailure::ReflectComponentNotRegistered { type_path } => type_path,
    }
}

/// How the most recently accepted discovery run ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum ReporterOutcomeHealth {
    /// No discovery result has been accepted.
    NotCompleted,
    /// A prerequisite prevented a complete set.
    Deferred {
        /// Start of the uninterrupted wait for this prerequisite.
        since:    RiggingRuntimeTime,
        /// Typed prerequisite that blocked discovery.
        deferral: ReporterDeferral,
    },
    /// A complete set was accepted.
    Succeeded {
        /// Batch containing the accepted set.
        batch:                 BatchRef,
        /// Runtime time when discovery completed.
        completed_at:          RiggingRuntimeTime,
        /// Time spent discovering this set.
        duration:              Duration,
        /// Number of records in this set.
        records:               usize,
        /// Result of validating capability reflected-component registrations for this set.
        capability_projection: CapabilityProjectionStatus,
    },
    /// An ordinary failure retained the preceding complete set.
    Failing {
        /// Batch whose discovery run failed.
        batch:    BatchRef,
        /// Time spent on the failed run.
        duration: Duration,
        /// Current consecutive-failure run.
        run:      FailureRunStatus,
    },
    /// The platform contract is unavailable and the reporter stopped.
    Unsupported {
        /// Owned typed error safe for diagnostics.
        error: DeviceAccessErrorView,
        /// Runtime time when the reporter stopped.
        since: RiggingRuntimeTime,
    },
}

/// Detail retained across one uninterrupted run of ordinary discovery failures.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub struct FailureRunStatus {
    /// Number of failures since the last non-failure outcome.
    pub consecutive:      NonZeroU32,
    /// Scheduler time assigned to the first failure in this run.
    pub first_failure_at: RiggingRuntimeTime,
    /// Scheduler time assigned to the latest failure in this run.
    pub last_failure_at:  RiggingRuntimeTime,
    /// Owned typed error from the latest failed run.
    pub error:            DeviceAccessErrorView,
    /// Most recent successful complete set before this failure.
    pub previous_success: PreviousSuccess,
}

/// Whether a reporter has a preceding successful complete set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum PreviousSuccess {
    /// No complete set has ever succeeded.
    Never,
    /// A complete set most recently succeeded with this retained capability projection.
    At {
        /// Runtime time when the retained complete set succeeded.
        completed_at:          RiggingRuntimeTime,
        /// Capability projection validated while accepting that complete set.
        capability_projection: CapabilityProjectionStatus,
    },
}

/// Owned serializable projection of a classified device-access error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum DeviceAccessErrorView {
    /// Another owner holds exclusive access.
    Contended {
        /// Provider detail.
        detail: String,
    },
    /// Operating-system or device policy refused access.
    Blocked {
        /// Provider detail.
        detail: String,
    },
    /// The device is not reachable.
    Absent {
        /// Provider detail.
        detail: String,
    },
    /// Communication with the device or provider failed.
    Transport {
        /// Provider detail.
        detail: String,
    },
    /// The current platform has no implementation for this operation.
    Unsupported {
        /// Provider detail.
        detail: String,
    },
}

impl ReporterHealth {
    /// Whether this health component belongs to the registry-issued reporter handle.
    #[must_use]
    pub const fn belongs_to(&self, reporter: ReporterId) -> bool {
        self.identity.reporter_ref.0 == reporter.0
    }

    /// Borrow the reporter's registry identity.
    #[must_use]
    pub const fn identity(&self) -> &ReporterIdentityView { &self.identity }

    /// Borrow the reporter's immutable registration policy.
    #[must_use]
    pub const fn registration(&self) -> &ReporterRegistrationView { &self.registration }

    /// Borrow first-complete-set progress.
    #[must_use]
    pub const fn first_complete_set(&self) -> &FirstCompleteSetStatus { &self.first_complete_set }

    /// Borrow current scheduler activity.
    #[must_use]
    pub const fn activity(&self) -> &ReporterActivityView { &self.activity }

    /// Borrow the most recently accepted discovery outcome.
    #[must_use]
    pub const fn outcome(&self) -> &ReporterOutcomeHealth { &self.outcome }

    /// Return how many discovery outcomes the registry has accepted for this reporter.
    #[must_use]
    pub const fn completed_runs(&self) -> u64 { self.completed_runs }

    /// Borrow the retained complete-set record count.
    #[must_use]
    pub const fn retained_records(&self) -> &usize { &self.retained_records }

    pub(crate) const fn set_activity(&mut self, activity: ReporterActivityView) {
        self.activity = activity;
    }

    pub(crate) fn set_outcome(&mut self, outcome: ReporterOutcomeHealth) {
        self.outcome = outcome;
        self.completed_runs = self.completed_runs.saturating_add(1);
    }

    pub(crate) fn record_capability_projection_failures(
        &mut self,
        failures: impl IntoIterator<Item = CapabilityProjectionFailure>,
    ) {
        if let ReporterOutcomeHealth::Succeeded {
            capability_projection,
            ..
        } = &mut self.outcome
        {
            capability_projection.extend_failures(failures);
        }
    }

    pub(crate) const fn set_first_complete_set(&mut self, status: FirstCompleteSetStatus) {
        self.first_complete_set = status;
    }

    pub(crate) const fn set_retained_records(&mut self, retained_records: usize) {
        self.retained_records = retained_records;
    }
    pub(crate) fn at_registration(
        reporter: ReporterId,
        name: String,
        registration: &ReporterRegistration,
        registered_at: RiggingRuntimeTime,
    ) -> Self {
        let wait_timing = match registration.activation() {
            ReporterActivation::Enabled => WaitTiming::Bounded {
                since:    registered_at,
                deadline: registered_at.after(registration.first_complete_set_bound()),
            },
            ReporterActivation::Disabled => WaitTiming::Unbounded {
                since: registered_at,
            },
        };
        let activity = match registration.activation() {
            ReporterActivation::Enabled => ReporterActivityView::Idle,
            ReporterActivation::Disabled => ReporterActivityView::Disabled,
        };

        Self {
            identity: ReporterIdentityView {
                reporter_ref: ReporterRef::from_reporter_id(reporter),
                name,
            },
            registration: ReporterRegistrationView {
                requirement:              registration.requirement(),
                coverage:                 ReporterCoverageView::from(registration.coverage()),
                cadence:                  DiscoveryCadenceView::from(registration.cadence()),
                first_complete_set_bound: registration.first_complete_set_bound(),
            },
            first_complete_set: FirstCompleteSetStatus::Waiting(wait_timing),
            activity,
            outcome: ReporterOutcomeHealth::NotCompleted,
            completed_runs: 0,
            retained_records: 0,
        }
    }

    pub(crate) const fn begin_first_complete_set_bound(&mut self, started_at: RiggingRuntimeTime) {
        if !matches!(
            self.first_complete_set,
            FirstCompleteSetStatus::Waiting(WaitTiming::Unbounded { .. })
        ) {
            return;
        }
        self.first_complete_set = FirstCompleteSetStatus::Waiting(WaitTiming::Bounded {
            since:    started_at,
            deadline: started_at.after(self.registration.first_complete_set_bound),
        });
    }

    pub(crate) const fn transition_to_disabled(&mut self, disabled_at: RiggingRuntimeTime) {
        self.activity = ReporterActivityView::Disabled;
        if matches!(
            self.first_complete_set,
            FirstCompleteSetStatus::Waiting(
                WaitTiming::Bounded { .. } | WaitTiming::Overdue { .. }
            )
        ) {
            self.first_complete_set =
                FirstCompleteSetStatus::Waiting(WaitTiming::Unbounded { since: disabled_at });
        }
    }

    pub(crate) fn cross_first_complete_set_deadline(&mut self, now: RiggingRuntimeTime) -> bool {
        let FirstCompleteSetStatus::Waiting(WaitTiming::Bounded { since, deadline }) =
            self.first_complete_set
        else {
            return false;
        };
        if now.elapsed() < deadline.elapsed() {
            return false;
        }
        self.first_complete_set = FirstCompleteSetStatus::Waiting(WaitTiming::Overdue {
            since,
            deadline,
            crossed_at: now,
        });
        true
    }
}

impl From<&ReporterCoverage> for ReporterCoverageView {
    fn from(coverage: &ReporterCoverage) -> Self {
        match coverage {
            ReporterCoverage::MatchingEvidenceOnly => Self::MatchingEvidenceOnly,
            ReporterCoverage::EstablishesAbsence(authoritative) => {
                Self::EstablishesAbsence(CoveredDeviceIdentitySpaceView::from(authoritative))
            },
        }
    }
}

impl From<&AuthoritativeReporterCoverage> for CoveredDeviceIdentitySpaceView {
    fn from(coverage: &AuthoritativeReporterCoverage) -> Self {
        Self::from(coverage.identity_space())
    }
}

impl From<&CoveredDeviceIdentitySpace> for CoveredDeviceIdentitySpaceView {
    fn from(identity_space: &CoveredDeviceIdentitySpace) -> Self {
        match identity_space {
            CoveredDeviceIdentitySpace::AllKeysOfKind { kind } => {
                Self::AllKeysOfKind { kind: *kind }
            },
            CoveredDeviceIdentitySpace::ReportedScheme { kind, scheme } => Self::ReportedScheme {
                kind:   *kind,
                scheme: scheme.clone(),
            },
            CoveredDeviceIdentitySpace::SynthesizedKeysOfKind { kind } => {
                Self::SynthesizedKeysOfKind { kind: *kind }
            },
            CoveredDeviceIdentitySpace::AuthoredKeysOfKind { kind } => {
                Self::AuthoredKeysOfKind { kind: *kind }
            },
        }
    }
}

impl From<&DiscoveryCadence> for DiscoveryCadenceView {
    fn from(cadence: &DiscoveryCadence) -> Self {
        match cadence {
            DiscoveryCadence::OnDemand => Self::OnDemand,
            DiscoveryCadence::EventDriven { backstop } => Self::EventDriven {
                backstop: *backstop,
            },
            DiscoveryCadence::Periodic { interval } => Self::Periodic {
                interval: *interval,
            },
        }
    }
}

impl From<&DiscoveryProgress> for DiscoveryProgressView {
    fn from(progress: &DiscoveryProgress) -> Self {
        match progress {
            DiscoveryProgress::Indeterminate => Self::Indeterminate,
            DiscoveryProgress::Measured { completed, total } => Self::Measured {
                completed: *completed,
                total:     *total,
            },
        }
    }
}

impl From<&DeviceAccessError> for DeviceAccessErrorView {
    fn from(error: &DeviceAccessError) -> Self {
        match error {
            DeviceAccessError::Contended { detail } => Self::Contended {
                detail: detail.clone(),
            },
            DeviceAccessError::Blocked { detail } => Self::Blocked {
                detail: detail.clone(),
            },
            DeviceAccessError::Absent { detail } => Self::Absent {
                detail: detail.clone(),
            },
            DeviceAccessError::Transport { detail } => Self::Transport {
                detail: detail.clone(),
            },
            DeviceAccessError::Unsupported { detail } => Self::Unsupported {
                detail: detail.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;
    use std::time::Instant;

    use super::NonEmptyRoleKeys;
    use super::ReporterRef;
    use super::RiggingRuntimeClock;
    use super::RolesWaitingOnReporter;
    use super::roles_waiting_on;
    use crate::ApplyDeadline;
    use crate::Binding;
    use crate::Bindings;
    use crate::DeviceEndpoint;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::EndpointId;
    use crate::FlowExpectation;
    use crate::LastKnownGoodConfiguration;
    use crate::OnAbort;
    use crate::OnSessionLoss;
    use crate::RecoveryPolicy;
    use crate::ReportedId;
    use crate::ReporterId;
    use crate::RequestedConfiguration;
    use crate::RetryOn;
    use crate::RoleKey;
    use crate::SchemeName;
    use crate::binding::WaitTransition;
    use crate::registration::DriverId;

    #[test]
    fn roles_waiting_on_sorts_keys_and_each_wait_crosses_its_bound_once()
    -> Result<(), Box<dyn Error>> {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(30);
        let reporter = ReporterId(4);
        let device = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: SchemeName::new("reporter-summary")?,
                value:  ReportedId::new("display")?,
            },
        };
        let mut bindings = Bindings::default();
        bindings.install_role_status_clock(RiggingRuntimeClock::starting_at(now));
        let later = RoleKey::new("z-output")?;
        let earlier = RoleKey::new("a-output")?;
        for role in [&later, &earlier] {
            bindings.register(Binding {
                role:             role.clone(),
                endpoint:         DeviceEndpoint {
                    device: device.clone(),
                    id:     EndpointId::Part(crate::PartName::new(role.as_str())?),
                },
                driver:           DriverId(0),
                recovery:         RecoveryPolicy::default(),
                retry:            RetryOn::NewRevision,
                on_abort:         OnAbort::default(),
                on_loss:          OnSessionLoss::default(),
                requested:        RequestedConfiguration::new(String::from("test configuration")),
                last_known_good:  LastKnownGoodConfiguration::default(),
                apply_deadline:   ApplyDeadline::ProcessDefault,
                flow_expectation: FlowExpectation::NotMonitored,
            })?;
            assert_eq!(
                bindings.record_test_reporter_wait(role, device.clone(), reporter, now, deadline,),
                WaitTransition::ReasonChanged
            );
        }

        assert_eq!(
            bindings.record_test_reporter_wait(
                &earlier,
                device.clone(),
                reporter,
                deadline,
                deadline,
            ),
            WaitTransition::BoundCrossed
        );
        assert_eq!(
            bindings.record_test_reporter_wait(
                &earlier,
                device,
                reporter,
                deadline + Duration::from_secs(1),
                deadline,
            ),
            WaitTransition::Unchanged
        );

        assert_eq!(
            roles_waiting_on(&bindings, ReporterRef::from_reporter_id(reporter)),
            RolesWaitingOnReporter::Waiting(NonEmptyRoleKeys(vec![earlier, later]))
        );

        Ok(())
    }
}
