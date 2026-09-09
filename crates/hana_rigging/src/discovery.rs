use std::num::NonZeroUsize;
use std::time::Duration;

use bevy::ecs::reflect::ReflectResource;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;
use bevy::reflect::ReflectSerialize;
use bevy::tasks::IoTaskPool;
use serde::Serialize;
use thiserror::Error;

use crate::DeviceAccessError;
use crate::DeviceIdSource;
use crate::DeviceKey;
use crate::DeviceKind;
use crate::DiscoveryProgress;
use crate::ReporterId;
use crate::SchemeName;
use crate::StartupRequirement;

const DEFAULT_MAX_COMPLETIONS_PER_FRAME: usize = 2;
const DEFAULT_MAX_CONCURRENT_JOBS: usize = 2;
const DEFAULT_PROGRESS_AFTER: Duration = Duration::from_millis(500);

/// Scheduling policy for when a reporter becomes eligible for one discovery run.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub enum DiscoveryCadence {
    /// The application must request every run, such as probing a USB bus from a refresh button.
    OnDemand,
    /// An operating-system notification marks the reporter dirty, while `backstop` finds missed
    /// notifications such as an unplug event delivered while the application was suspended.
    EventDriven {
        /// Longest time the kernel waits before retrying after no dirty notification arrives.
        backstop: Duration,
    },
    /// The reporter may run once during each interval, such as refreshing a remote camera list.
    Periodic {
        /// Minimum time between one reporter's submitted discovery jobs.
        interval: Duration,
    },
}

/// A prerequisite that prevented a reporter from enumerating its complete set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Reflect)]
#[reflect(Serialize)]
pub enum ReporterDeferral {
    /// Monitor topology has not completed the observation that display discovery depends on.
    WaitingForTopology,
}

/// Whether an optional reporter is currently permitted to discover hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum ReporterActivation {
    /// The reporter may become due under its cadence and receives one initial run.
    Enabled,
    /// The reporter stays out of due lists until application code explicitly enables it.
    Disabled,
}

/// Whether a successful complete report can establish absence for authored inventory identity.
///
/// A reporter may contribute live matching evidence without being able to enumerate every device
/// in an identity space. This distinction prevents a camera-only or partial display report from
/// treating omitted authored hardware as absent.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub enum ReporterCoverage {
    /// This reporter can contribute live records and matching evidence, but omission proves
    /// nothing about an authored device.
    MatchingEvidenceOnly,
    /// A fresh successful complete report can establish absence in the declared identity spaces.
    EstablishesAbsence(AuthoritativeReporterCoverage),
}

impl ReporterCoverage {
    /// Report whether this reporter's omission of one durable key would prove the unit is gone.
    pub(crate) fn establishes_absence_for(&self, device_key: &DeviceKey) -> bool {
        match self {
            Self::MatchingEvidenceOnly => false,
            Self::EstablishesAbsence(authoritative_reporter_coverage) => {
                authoritative_reporter_coverage.covers(device_key)
            },
        }
    }
}

/// The one identity space a reporter completely enumerates on a successful scan.
///
/// Absence authority always names an identity space: omission proves a unit is gone only inside
/// a space the reporter enumerates completely, so authority with no named space would look
/// enabled while proving nothing. The single space is a fact of the type, not a rule checked at
/// construction.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
#[reflect(opaque)]
pub struct AuthoritativeReporterCoverage {
    identity_space: CoveredDeviceIdentitySpace,
}

impl AuthoritativeReporterCoverage {
    /// Construct absence authority for one complete durable identity space.
    #[must_use]
    pub const fn one(covered_device_identity_space: CoveredDeviceIdentitySpace) -> Self {
        Self {
            identity_space: covered_device_identity_space,
        }
    }

    fn covers(&self, device_key: &DeviceKey) -> bool { self.identity_space.covers(device_key) }

    pub(crate) const fn identity_space(&self) -> &CoveredDeviceIdentitySpace {
        &self.identity_space
    }
}

/// One durable identity space a successful complete reporter scan can enumerate.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub enum CoveredDeviceIdentitySpace {
    /// The reporter enumerates every durable key of this physical device kind.
    AllKeysOfKind {
        /// Physical kind completely enumerated by this reporter.
        kind: DeviceKind,
    },
    /// The reporter enumerates every unit-reported value in this registered identity scheme.
    ReportedScheme {
        /// Physical kind separated from unrelated users of the same scheme name.
        kind:   DeviceKind,
        /// Registered identity space fully enumerated by this reporter.
        scheme: SchemeName,
    },
    /// The reporter enumerates synthesized durable keys of this physical device kind.
    SynthesizedKeysOfKind {
        /// Physical kind whose synthesized identity records are complete in this report.
        kind: DeviceKind,
    },
    /// The reporter enumerates operator-authored durable keys of this physical device kind.
    AuthoredKeysOfKind {
        /// Physical kind whose authored inventory identities are complete in this report.
        kind: DeviceKind,
    },
}

impl CoveredDeviceIdentitySpace {
    fn covers(&self, device_key: &DeviceKey) -> bool {
        match self {
            Self::AllKeysOfKind { kind } => device_key.kind == *kind,
            Self::ReportedScheme { kind, scheme } => {
                device_key.kind == *kind
                    && matches!(&device_key.id, DeviceIdSource::Reported { scheme: key_scheme, .. } if key_scheme == scheme)
            },
            Self::SynthesizedKeysOfKind { kind } => {
                device_key.kind == *kind
                    && matches!(&device_key.id, DeviceIdSource::Synthesized { .. })
            },
            Self::AuthoredKeysOfKind { kind } => {
                device_key.kind == *kind
                    && matches!(&device_key.id, DeviceIdSource::Authored { .. })
            },
        }
    }
}

/// Registration policy that separates startup requirements from reporter activation.
///
/// Required reporters always start enabled because startup cannot become ready without their first
/// successful whole-set report. Optional reporters use `ReporterActivation` instead, so authored
/// offline inventory never causes the kernel to search for hardware.
#[derive(Clone, Reflect)]
#[reflect(opaque)]
pub struct ReporterRegistration {
    cadence:                  DiscoveryCadence,
    startup_requirement:      StartupRequirement,
    activation:               ReporterActivation,
    coverage:                 ReporterCoverage,
    first_complete_set_bound: Duration,
}

impl ReporterRegistration {
    /// Require an enabled reporter's first successful complete result before hardware is ready.
    #[must_use]
    pub const fn required(
        cadence: DiscoveryCadence,
        coverage: ReporterCoverage,
        first_complete_set_bound: Duration,
    ) -> Self {
        Self {
            cadence,
            startup_requirement: StartupRequirement::Required,
            activation: ReporterActivation::Enabled,
            coverage,
            first_complete_set_bound,
        }
    }

    /// Register hardware that application policy may enable or leave offline independently.
    #[must_use]
    pub const fn optional(
        cadence: DiscoveryCadence,
        activation: ReporterActivation,
        coverage: ReporterCoverage,
        first_complete_set_bound: Duration,
    ) -> Self {
        Self {
            cadence,
            startup_requirement: StartupRequirement::Optional,
            activation,
            coverage,
            first_complete_set_bound,
        }
    }

    pub(crate) const fn activation(&self) -> ReporterActivation { self.activation }

    pub(crate) const fn requirement(&self) -> StartupRequirement { self.startup_requirement }

    pub(crate) const fn cadence(&self) -> &DiscoveryCadence { &self.cadence }

    pub(crate) const fn coverage(&self) -> &ReporterCoverage { &self.coverage }

    pub(crate) const fn first_complete_set_bound(&self) -> Duration {
        self.first_complete_set_bound
    }
}

/// Runtime control surface for reporter activation and explicit discovery triggers.
///
/// The resource records user intent only. `RiggingSystems::Collect` applies that intent before it
/// builds the frame's due list, which prevents an application from submitting one reporter twice
/// by issuing several requests in one update. Reflection exposes this resource opaquely so dynamic
/// values cannot bypass its checked control methods or mutate invariant-bearing reporter records.
#[derive(Clone, Default, Resource, Reflect)]
#[reflect(Resource, opaque)]
pub struct DiscoveryControl {
    reporters: Vec<ReporterControl>,
}

#[derive(Clone)]
struct ReporterControl {
    reporter:            ReporterId,
    startup_requirement: StartupRequirement,
    activation:          ReporterActivation,
    request:             DiscoveryRequest,
    dirty:               DiscoveryDirtyState,
}

#[derive(Clone, Copy)]
pub(crate) enum DiscoveryRequest {
    NotRequested,
    Requested,
}

#[derive(Clone, Copy)]
pub(crate) enum DiscoveryDirtyState {
    Clean,
    Dirty,
}

impl DiscoveryControl {
    pub(crate) fn register(&mut self, reporter: ReporterId, registration: &ReporterRegistration) {
        self.reporters.push(ReporterControl {
            reporter,
            startup_requirement: registration.requirement(),
            activation: registration.activation(),
            request: DiscoveryRequest::Requested,
            dirty: DiscoveryDirtyState::Clean,
        });
    }

    /// Enable an optional reporter and request exactly one initial discovery run.
    ///
    /// # Errors
    ///
    /// Returns `DiscoveryControlError::ReporterNotRegistered` when `reporter` was not issued by
    /// this process's reporter registry.
    pub fn enable(&mut self, reporter: ReporterId) -> Result<(), DiscoveryControlError> {
        let reporter_control = self.find_mut(reporter)?;
        reporter_control.activation = ReporterActivation::Enabled;
        reporter_control.request = DiscoveryRequest::Requested;

        Ok(())
    }

    /// Disable an optional reporter and cancel its queued work without cancelling a running job.
    ///
    /// # Errors
    ///
    /// Returns `DiscoveryControlError::RequiredReporterCannotBeDisabled` when startup depends on
    /// `reporter`, or `DiscoveryControlError::ReporterNotRegistered` for an unknown identifier.
    pub fn disable(&mut self, reporter: ReporterId) -> Result<(), DiscoveryControlError> {
        let reporter_control = self.find_mut(reporter)?;
        if reporter_control.startup_requirement == StartupRequirement::Required {
            return Err(DiscoveryControlError::RequiredReporterCannotBeDisabled { reporter });
        }

        reporter_control.activation = ReporterActivation::Disabled;
        reporter_control.request = DiscoveryRequest::NotRequested;
        reporter_control.dirty = DiscoveryDirtyState::Clean;

        Ok(())
    }

    /// Request one run for a reporter whose cadence would otherwise leave it idle.
    ///
    /// # Errors
    ///
    /// Returns `DiscoveryControlError::ReporterNotRegistered` when `reporter` was not issued by
    /// this process's reporter registry.
    pub fn request(&mut self, reporter: ReporterId) -> Result<(), DiscoveryControlError> {
        self.find_mut(reporter)?.request = DiscoveryRequest::Requested;

        Ok(())
    }

    /// Mark a reporter dirty after an integration observed a device-configuration notification.
    ///
    /// # Errors
    ///
    /// Returns `DiscoveryControlError::ReporterNotRegistered` when `reporter` was not issued by
    /// this process's reporter registry.
    pub fn mark_dirty(&mut self, reporter: ReporterId) -> Result<(), DiscoveryControlError> {
        self.find_mut(reporter)?.dirty = DiscoveryDirtyState::Dirty;

        Ok(())
    }

    pub(crate) fn activation(&self, reporter: ReporterId) -> ReporterActivation {
        self.reporters
            .iter()
            .find(|reporter_control| reporter_control.reporter == reporter)
            .map_or(ReporterActivation::Disabled, |reporter_control| {
                reporter_control.activation
            })
    }

    pub(crate) fn take_request(&mut self, reporter: ReporterId) -> DiscoveryRequest {
        self.take(
            reporter,
            |reporter_control| &mut reporter_control.request,
            DiscoveryRequest::NotRequested,
        )
    }

    pub(crate) fn take_dirty(&mut self, reporter: ReporterId) -> DiscoveryDirtyState {
        self.take(
            reporter,
            |reporter_control| &mut reporter_control.dirty,
            DiscoveryDirtyState::Clean,
        )
    }

    fn find_mut(
        &mut self,
        reporter: ReporterId,
    ) -> Result<&mut ReporterControl, DiscoveryControlError> {
        self.reporters
            .iter_mut()
            .find(|reporter_control| reporter_control.reporter == reporter)
            .ok_or(DiscoveryControlError::ReporterNotRegistered { reporter })
    }

    fn take<State: Copy>(
        &mut self,
        reporter: ReporterId,
        select: impl FnOnce(&mut ReporterControl) -> &mut State,
        default: State,
    ) -> State {
        self.reporters
            .iter_mut()
            .find(|reporter_control| reporter_control.reporter == reporter)
            .map_or(default, |reporter_control| {
                std::mem::replace(select(reporter_control), default)
            })
    }
}

/// Rejected reporter-control operation that leaves kernel scheduling state unchanged.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DiscoveryControlError {
    /// Application code addressed an identifier that no `add_device_reporter` call issued.
    #[error("device reporter `{reporter:?}` is not registered")]
    ReporterNotRegistered {
        /// Process-local reporter handle that did not select a registry entry.
        reporter: ReporterId,
    },
    /// Application code attempted to disable a reporter needed for startup readiness.
    #[error("required device reporter `{reporter:?}` cannot be disabled")]
    RequiredReporterCannotBeDisabled {
        /// Process-local reporter handle whose required registration remains enabled.
        reporter: ReporterId,
    },
}

/// Process-local label shared by every reporter due during one collect pass.
///
/// The registry creates batch ids because a reporter cannot choose an identifier that overlaps a
/// co-reporter's discovery opportunity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Reflect)]
#[reflect(opaque)]
pub struct DiscoveryBatchId(pub(crate) u64);

impl DiscoveryBatchId {
    /// Return the registry-issued batch number for diagnostics and progress presentation.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

/// Capacity and user-interface timing limits for discovery collection.
///
/// `max_concurrent_jobs` reserves one I/O worker for unrelated file and network operations when
/// `IoTaskPool` has at least two threads. A one-thread pool runs one discovery job because
/// reserving its only worker would disable discovery entirely.
///
/// Both capacities are `NonZeroUsize`, so a limit of none cannot be authored:
///
/// ```
/// use std::num::NonZeroUsize;
///
/// use hana_rigging::DiscoveryLimits;
///
/// fn limits() -> DiscoveryLimits {
///     let mut limits = DiscoveryLimits::default();
///     limits.set_max_concurrent_jobs(NonZeroUsize::MIN);
///     limits.set_max_completions_per_frame(NonZeroUsize::MIN);
///     limits
/// }
/// ```
///
/// The authoring above is what keeps this case meaningful — a rename would
/// break it loudly rather than leaving this one failing for an unrelated
/// reason:
///
/// ```compile_fail,E0308
/// use hana_rigging::DiscoveryLimits;
///
/// let mut discovery_limits = DiscoveryLimits::default();
/// discovery_limits.set_max_concurrent_jobs(0);
/// discovery_limits.set_max_completions_per_frame(0);
/// ```
#[derive(Clone, Resource, Reflect)]
#[reflect(Resource)]
pub struct DiscoveryLimits {
    max_concurrent_jobs:       NonZeroUsize,
    max_completions_per_frame: NonZeroUsize,
    progress_after:            Duration,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            max_concurrent_jobs:       NonZeroUsize::new(DEFAULT_MAX_CONCURRENT_JOBS)
                .unwrap_or(NonZeroUsize::MIN),
            max_completions_per_frame: NonZeroUsize::new(DEFAULT_MAX_COMPLETIONS_PER_FRAME)
                .unwrap_or(NonZeroUsize::MIN),
            progress_after:            DEFAULT_PROGRESS_AFTER,
        }
    }
}

impl DiscoveryLimits {
    /// Return the number of whole reporter results the kernel may accept in one update.
    #[must_use]
    pub(crate) const fn max_completions_per_frame(&self) -> NonZeroUsize {
        self.max_completions_per_frame
    }

    /// Return how long a running job must last before progress becomes UI-visible.
    #[must_use]
    pub(crate) const fn progress_after(&self) -> Duration { self.progress_after }

    /// Return the capacity available after preserving one I/O-pool thread when possible.
    ///
    /// # Errors
    ///
    /// Returns `DiscoverySchedulerError::IoTaskPoolUnavailable` until Bevy's `TaskPoolPlugin`
    /// initializes the global I/O pool. This kernel never installs a pool with application-wide
    /// settings of its own.
    pub(crate) fn effective_max_concurrent_jobs(
        &self,
    ) -> Result<NonZeroUsize, DiscoverySchedulerError> {
        let io_task_pool =
            IoTaskPool::try_get().ok_or(DiscoverySchedulerError::IoTaskPoolUnavailable)?;
        Ok(effective_discovery_job_capacity(
            self.max_concurrent_jobs,
            io_task_pool.thread_num(),
        ))
    }

    /// Change the number of background jobs a later admission pass may submit.
    #[cfg(feature = "test-support")]
    pub const fn set_max_concurrent_jobs(&mut self, max_concurrent_jobs: NonZeroUsize) {
        self.max_concurrent_jobs = max_concurrent_jobs;
    }

    /// Change the number of completed whole sets a later collect pass may accept.
    #[cfg(feature = "test-support")]
    pub const fn set_max_completions_per_frame(&mut self, max_completions_per_frame: NonZeroUsize) {
        self.max_completions_per_frame = max_completions_per_frame;
    }

    /// Change when UI event integration may treat a running job as long-running.
    #[cfg(feature = "test-support")]
    pub const fn set_progress_after(&mut self, progress_after: Duration) {
        self.progress_after = progress_after;
    }
}

fn effective_discovery_job_capacity(
    configured: NonZeroUsize,
    io_thread_count: usize,
) -> NonZeroUsize {
    let pool_capacity = io_thread_count.saturating_sub(1).max(1);
    let effective = configured.get().min(pool_capacity);

    NonZeroUsize::new(effective).unwrap_or(NonZeroUsize::MIN)
}

/// Scheduler failure that is separate from a reporter's device-access failure.
#[derive(Clone, Debug, PartialEq, Eq, Error, Reflect)]
pub(crate) enum DiscoverySchedulerError {
    /// A background reporter became due before Bevy initialized the global I/O task pool.
    #[error("Bevy IoTaskPool is not initialized")]
    IoTaskPoolUnavailable,
}

/// Current scheduler health retained for diagnostics while reporters are registered.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum DiscoverySchedulerState {
    /// Background work can use Bevy's I/O pool when a reporter becomes due.
    Available,
    /// A due background reporter could not start because the application has not installed the
    /// pool.
    Failed {
        /// Named initialization failure that application diagnostics can render directly.
        error: DiscoverySchedulerError,
    },
}

/// Latest retained state for all registered reporters and startup discovery.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub(crate) struct DiscoveryStatus {
    /// Whether every required reporter has produced the successful whole set startup needs.
    pub(crate) startup:   StartupDiscoveryState,
    /// Whether the scheduler can start background work on the application's I/O task pool.
    pub(crate) scheduler: DiscoverySchedulerState,
    reporters:            Vec<ReporterStatusRecord>,
}

#[derive(Reflect)]
struct ReporterStatusRecord {
    reporter: ReporterId,
    status:   ReporterDiscoveryStatus,
}

impl Default for DiscoveryStatus {
    fn default() -> Self {
        Self {
            startup:   StartupDiscoveryState::Ready,
            scheduler: DiscoverySchedulerState::Available,
            reporters: Vec::new(),
        }
    }
}

impl DiscoveryStatus {
    pub(crate) fn register(&mut self, reporter: ReporterId, registration: &ReporterRegistration) {
        let activity = match registration.activation() {
            ReporterActivation::Enabled => ReporterActivity::Idle,
            ReporterActivation::Disabled => ReporterActivity::Disabled,
        };
        self.reporters.push(ReporterStatusRecord {
            reporter,
            status: ReporterDiscoveryStatus {
                activity,
                last_outcome: LastDiscoveryOutcome::NotCompleted,
                completed_batches: 0,
            },
        });
        if registration.requirement() == StartupRequirement::Required {
            self.startup = StartupDiscoveryState::Discovering;
        }
    }

    /// Read retained activity and outcome for one registry-issued reporter identifier.
    ///
    /// # Errors
    ///
    /// Returns `DiscoveryStatusError::ReporterNotRegistered` when `reporter` has no status entry.
    pub(crate) fn reporter_status(
        &self,
        reporter: ReporterId,
    ) -> Result<&ReporterDiscoveryStatus, DiscoveryStatusError> {
        self.reporters
            .iter()
            .find(|reporter_status_record| reporter_status_record.reporter == reporter)
            .map(|reporter_status_record| &reporter_status_record.status)
            .ok_or(DiscoveryStatusError::ReporterNotRegistered { reporter })
    }

    pub(crate) fn reporter_status_mut(
        &mut self,
        reporter: ReporterId,
    ) -> Result<&mut ReporterDiscoveryStatus, DiscoveryStatusError> {
        self.reporters
            .iter_mut()
            .find(|reporter_status_record| reporter_status_record.reporter == reporter)
            .map(|reporter_status_record| &mut reporter_status_record.status)
            .ok_or(DiscoveryStatusError::ReporterNotRegistered { reporter })
    }
}

/// Retained outcome lookup failure for application user interfaces and diagnostics.
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum DiscoveryStatusError {
    /// Application code addressed an identifier that no `add_device_reporter` call issued.
    #[error("device reporter `{reporter:?}` has no discovery status")]
    ReporterNotRegistered {
        /// Process-local reporter handle that did not select a retained status record.
        reporter: ReporterId,
    },
}

/// Per-reporter activity and the previous accepted outcome retained across later runs.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub(crate) struct ReporterDiscoveryStatus {
    /// What the scheduler is doing with this reporter right now.
    pub(crate) activity:          ReporterActivity,
    /// Most recent accepted whole-set result or failure, preserved while the reporter runs again.
    pub(crate) last_outcome:      LastDiscoveryOutcome,
    /// Number of discovery runs this reporter has finished accepting.
    pub(crate) completed_batches: u64,
}

/// Current scheduler activity for one reporter without bare optional progress fields.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum ReporterActivity {
    /// Application policy disabled this optional reporter, so it cannot enter a due list.
    Disabled,
    /// The reporter is eligible only when its cadence, dirty flag, or explicit request says so.
    Idle,
    /// The reporter is in a logical batch awaiting main-thread preparation or job admission.
    Queued {
        /// Registry-issued batch shared by every reporter due in the same collect pass.
        batch: DiscoveryBatchId,
    },
    /// An I/O task is enumerating hardware while the kernel retains its own mutable state.
    Running {
        /// Registry-issued batch that submitted this job.
        batch:    DiscoveryBatchId,
        /// Time since the kernel submitted this owned background job.
        elapsed:  Duration,
        /// Job-reported progress, including the explicitly indeterminate case.
        progress: DiscoveryProgress,
    },
}

/// Previous whole-set result or failure that stays visible while reporter activity changes.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub(crate) enum LastDiscoveryOutcome {
    /// No whole-set result has been accepted since this reporter was registered.
    NotCompleted,
    /// The reporter is waiting for a prerequisite and retains its preceding complete set.
    Deferred {
        /// Batch whose discovery run reached the prerequisite.
        batch:    DiscoveryBatchId,
        /// Time when the current uninterrupted deferral began.
        since:    crate::RiggingRuntimeTime,
        /// Typed prerequisite that must complete before discovery can produce a complete set.
        deferral: ReporterDeferral,
    },
    /// The reporter's complete set was accepted and its registry revision advanced.
    Succeeded {
        /// Batch that supplied the accepted complete set.
        batch:    DiscoveryBatchId,
        /// Time from this reporter's submission until the discovery run completed; later
        /// main-thread acceptance delay is excluded.
        duration: Duration,
    },
    /// The reporter could not enumerate its whole set, so its prior revision remains current.
    Failed {
        /// Batch whose discovery run failed.
        batch:    DiscoveryBatchId,
        /// Time from this reporter's submission until the discovery run completed; later
        /// main-thread acceptance delay is excluded.
        duration: Duration,
        /// Reporter error preserved for recovery policy and user diagnostics.
        error:    DeviceAccessError,
    },
    /// The reporter cannot run on this platform and remains stopped until explicit control input.
    Unsupported {
        /// Batch whose discovery run reported the unsupported platform integration.
        batch: DiscoveryBatchId,
        /// Classified platform failure that stopped the reporter.
        error: DeviceAccessError,
        /// Time when the reporter entered its current stopped state.
        since: crate::RiggingRuntimeTime,
    },
}

/// Startup gate derived from required reporters' retained whole-set outcomes.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub enum StartupDiscoveryState {
    /// At least one required reporter has not yet produced a successful complete result.
    Discovering,
    /// Every required reporter completed successfully, so hardware-dependent systems may run.
    Ready,
    /// A required reporter failed, leaving startup closed until a later successful complete result.
    BlockedByFailure {
        /// Required reporter whose latest accepted result could not establish a whole set.
        reporter: ReporterId,
        /// Failure retained instead of replacing the reporter's prior complete set.
        error:    DeviceAccessError,
    },
}

/// How one discovery run ended, as `crate::DiscoveryFinished` reports it.
///
/// Separate from `LastDiscoveryOutcome`, which retains `LastDiscoveryOutcome::NotCompleted` for a
/// reporter that has never finished: that state cannot be reached by a run that just ended, and a
/// consumer matching on the retained enum would have to write an arm for a case the event can never
/// carry.
#[derive(Clone, Debug, PartialEq, Eq, Reflect)]
pub enum CompletedDiscoveryOutcome {
    /// The reporter established its whole current set and the kernel accepted it.
    Succeeded {
        /// Time from this reporter's submission until the run completed; the later main-thread
        /// acceptance delay is excluded, because it measures the kernel's own admission budget
        /// rather than how long the hardware took to answer.
        duration: Duration,
    },
    /// The reporter reached a prerequisite that prevents a complete set, so its preceding set
    /// stays current.
    Deferred {
        /// Time from this reporter's submission until the prerequisite was reported.
        duration: Duration,
        /// Typed prerequisite that must complete before discovery can produce a complete set.
        deferral: ReporterDeferral,
    },
    /// The reporter could not establish its whole set, so its preceding set stays current.
    Failed {
        /// Time from this reporter's submission until the run completed.
        duration: Duration,
        /// Reporter error preserved so a diagnostic can name what failed rather than only that
        /// something did.
        error:    DeviceAccessError,
    },
}

/// One authoritative discovery state change, recorded where it happens and emitted once later.
///
/// Recorded rather than emitted in place because the scheduler mutates authoritative state inside
/// `crate::RiggingSystems::Collect`, and triggering public events from there would couple every
/// future scheduler change to event delivery order. Recorded rather than derived by comparing the
/// retained `DiscoveryStatus` against a previous copy because `DiscoveryStatus` deliberately keeps
/// only the current value: two transitions that land between two runs of the event system would
/// leave one of them undetectable.
pub(crate) enum DiscoveryTransition {
    /// A running reporter has been going for at least `DiscoveryLimits::progress_after` and its
    /// job-reported progress moved. The batch counts travel with it so the per-reporter and the
    /// aggregate events cannot disagree about the same moment.
    Progressed {
        batch:     DiscoveryBatchId,
        reporter:  ReporterId,
        progress:  DiscoveryProgress,
        completed: usize,
        total:     usize,
        running:   usize,
        queued:    usize,
    },
    /// One reporter's run reached a terminal outcome and the kernel accepted it.
    Finished {
        batch:    DiscoveryBatchId,
        reporter: ReporterId,
        outcome:  CompletedDiscoveryOutcome,
    },
    /// The required-before-ready startup gate moved to a different state.
    StartupChanged { startup: StartupDiscoveryState },
}

/// The bounded record of this frame's discovery transitions, drained once by the event stage.
///
/// Bounded by what one frame can actually produce rather than kept as a history: at most one
/// progress edge per registered reporter, at most `DiscoveryLimits::max_completions_per_frame`
/// completions, and at most one startup edge. An unbounded queue would grow without limit in any
/// application that installs the kernel and never runs the event stage.
#[derive(Default, Resource)]
pub(crate) struct DiscoveryTransitionJournal {
    transitions: Vec<DiscoveryTransition>,
}

impl DiscoveryTransitionJournal {
    /// Append one transition unless this frame has already produced everything it can.
    ///
    /// The bound is passed in by the scheduler because `discovery_transition_capacity` computes it
    /// from the registered reporter count, which lives in the reporter registry and not here. A
    /// refused append is not lost work: reaching the bound means the event stage has not run since
    /// these transitions were recorded, so nothing is listening for another one.
    pub(crate) fn record(&mut self, capacity: usize, discovery_transition: DiscoveryTransition) {
        if self.transitions.len() >= capacity {
            return;
        }
        self.transitions.push(discovery_transition);
    }

    /// Take everything recorded since the last drain, leaving the journal empty.
    pub(crate) fn drain(&mut self) -> Vec<DiscoveryTransition> {
        std::mem::take(&mut self.transitions)
    }
}

/// The most transitions one frame of discovery scheduling can produce.
///
/// At most one progress edge per registered reporter, at most
/// `DiscoveryLimits::max_completions_per_frame` completions, and at most one startup edge.
pub(crate) const fn discovery_transition_capacity(
    reporters: usize,
    discovery_limits: &DiscoveryLimits,
) -> usize {
    reporters + discovery_limits.max_completions_per_frame().get() + 1
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::num::NonZeroUsize;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::reflect::FromReflect;
    use bevy::reflect::PartialReflect;
    use bevy::reflect::ReflectMut;
    use bevy::reflect::ReflectRef;
    use bevy::reflect::structs::DynamicStruct;

    use super::AuthoritativeReporterCoverage;
    use super::CoveredDeviceIdentitySpace;
    use super::DEFAULT_MAX_COMPLETIONS_PER_FRAME;
    use super::DEFAULT_MAX_CONCURRENT_JOBS;
    use super::DEFAULT_PROGRESS_AFTER;
    use super::DiscoveryCadence;
    use super::DiscoveryControl;
    use super::DiscoveryControlError;
    use super::DiscoveryDirtyState;
    use super::DiscoveryLimits;
    use super::DiscoveryRequest;
    use super::ReporterActivation;
    use super::ReporterCoverage;
    use super::ReporterRegistration;
    use super::StartupRequirement;
    use super::effective_discovery_job_capacity;
    use crate::AuthoredId;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::ReporterId;

    #[test]
    fn default_limits_expose_scheduler_and_progress_policy() {
        let discovery_limits = DiscoveryLimits::default();

        assert_eq!(
            discovery_limits.max_concurrent_jobs.get(),
            DEFAULT_MAX_CONCURRENT_JOBS
        );
        assert_eq!(
            discovery_limits.max_completions_per_frame().get(),
            DEFAULT_MAX_COMPLETIONS_PER_FRAME
        );
        assert_eq!(discovery_limits.progress_after(), DEFAULT_PROGRESS_AFTER);
    }

    #[test]
    fn effective_capacity_reserves_one_thread_when_possible() {
        let configured =
            NonZeroUsize::new(DEFAULT_MAX_CONCURRENT_JOBS * 2).unwrap_or(NonZeroUsize::MIN);

        assert_eq!(effective_discovery_job_capacity(configured, 4).get(), 3);
        assert_eq!(effective_discovery_job_capacity(configured, 2).get(), 1);
        assert_eq!(effective_discovery_job_capacity(configured, 1).get(), 1);
        assert_eq!(
            effective_discovery_job_capacity(NonZeroUsize::MIN, 8).get(),
            1
        );
    }

    #[test]
    fn coverage_requires_an_explicit_identity_space_and_never_crosses_device_kind()
    -> Result<(), Box<dyn std::error::Error>> {
        let display_key = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new("studio-display")?,
            },
        };
        let camera_coverage = ReporterCoverage::EstablishesAbsence(
            AuthoritativeReporterCoverage::one(CoveredDeviceIdentitySpace::AllKeysOfKind {
                kind: DeviceKind::Camera,
            }),
        );

        assert!(!ReporterCoverage::MatchingEvidenceOnly.establishes_absence_for(&display_key));
        assert!(!camera_coverage.establishes_absence_for(&display_key));
        assert!(
            ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
                CoveredDeviceIdentitySpace::AuthoredKeysOfKind {
                    kind: DeviceKind::Display,
                }
            ))
            .establishes_absence_for(&display_key)
        );

        Ok(())
    }

    #[test]
    fn reporter_coverage_registers_for_reflection_without_manual_app_calls() {
        let app = App::new();
        let world = app.world();
        let type_registry = world.resource::<AppTypeRegistry>().read();

        assert!(type_registry.contains(TypeId::of::<ReporterCoverage>()));
        assert!(type_registry.contains(TypeId::of::<AuthoritativeReporterCoverage>()));
        assert!(type_registry.contains(TypeId::of::<CoveredDeviceIdentitySpace>()));
        drop(type_registry);
    }

    #[test]
    fn reflection_cannot_construct_or_mutate_authoritative_reporter_coverage() {
        let display_coverage = CoveredDeviceIdentitySpace::AllKeysOfKind {
            kind: DeviceKind::Display,
        };
        let mut authoritative_reporter_coverage =
            AuthoritativeReporterCoverage::one(display_coverage.clone());

        assert!(matches!(
            authoritative_reporter_coverage.reflect_ref(),
            ReflectRef::Opaque(_)
        ));
        assert!(matches!(
            authoritative_reporter_coverage.reflect_mut(),
            ReflectMut::Opaque(_)
        ));

        let mut unchecked_coverage = DynamicStruct::default();
        unchecked_coverage.insert("identity_space", display_coverage.clone());
        assert!(AuthoritativeReporterCoverage::from_reflect(&unchecked_coverage).is_none());
        assert!(
            authoritative_reporter_coverage
                .try_apply(&unchecked_coverage)
                .is_err()
        );
        assert_eq!(
            authoritative_reporter_coverage,
            AuthoritativeReporterCoverage::one(display_coverage)
        );
    }

    #[test]
    fn dirty_notifications_coalesce_in_runtime_control() {
        let reporter = ReporterId(0);
        let registration = ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
            std::time::Duration::from_secs(10),
        );
        assert!(matches!(
            registration.coverage(),
            ReporterCoverage::MatchingEvidenceOnly
        ));
        let mut discovery_control = DiscoveryControl::default();
        discovery_control.register(reporter, &registration);

        assert_eq!(discovery_control.mark_dirty(reporter), Ok(()));
        assert_eq!(discovery_control.mark_dirty(reporter), Ok(()));
        assert!(matches!(
            discovery_control.take_dirty(reporter),
            DiscoveryDirtyState::Dirty
        ));
        assert!(matches!(
            discovery_control.take_dirty(reporter),
            DiscoveryDirtyState::Clean
        ));
    }

    #[test]
    fn rejected_controls_leave_registered_reporter_state_unchanged() {
        let reporter = ReporterId(0);
        let unknown_reporter = ReporterId(u32::MAX);
        let registration = ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
            std::time::Duration::from_secs(10),
        );
        let mut discovery_control = DiscoveryControl::default();
        discovery_control.register(reporter, &registration);

        assert_eq!(
            discovery_control.enable(unknown_reporter),
            Err(DiscoveryControlError::ReporterNotRegistered {
                reporter: unknown_reporter,
            })
        );
        assert_control_state_unchanged(&discovery_control, reporter);
        assert_eq!(
            discovery_control.disable(unknown_reporter),
            Err(DiscoveryControlError::ReporterNotRegistered {
                reporter: unknown_reporter,
            })
        );
        assert_control_state_unchanged(&discovery_control, reporter);
        assert_eq!(
            discovery_control.request(unknown_reporter),
            Err(DiscoveryControlError::ReporterNotRegistered {
                reporter: unknown_reporter,
            })
        );
        assert_control_state_unchanged(&discovery_control, reporter);
        assert_eq!(
            discovery_control.mark_dirty(unknown_reporter),
            Err(DiscoveryControlError::ReporterNotRegistered {
                reporter: unknown_reporter,
            })
        );
        assert_control_state_unchanged(&discovery_control, reporter);
        assert_eq!(
            discovery_control.disable(reporter),
            Err(DiscoveryControlError::RequiredReporterCannotBeDisabled { reporter })
        );
        assert_control_state_unchanged(&discovery_control, reporter);
    }

    #[test]
    fn reflection_preserves_registration_and_control_invariants() {
        let reporter_registration = ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
            std::time::Duration::from_secs(10),
        );
        assert!(matches!(
            reporter_registration.reflect_ref(),
            ReflectRef::Opaque(_)
        ));

        let mut unchecked_registration = DynamicStruct::default();
        unchecked_registration.insert("cadence", DiscoveryCadence::OnDemand);
        unchecked_registration.insert("activation", ReporterActivation::Disabled);
        assert!(ReporterRegistration::from_reflect(&unchecked_registration).is_none());

        let mut discovery_control = DiscoveryControl::default();
        assert!(matches!(
            discovery_control.reflect_ref(),
            ReflectRef::Opaque(_)
        ));
        assert!(matches!(
            discovery_control.reflect_mut(),
            ReflectMut::Opaque(_)
        ));

        let mut unchecked_control = DynamicStruct::default();
        unchecked_control.insert("reporters", Vec::<u8>::new());
        assert!(DiscoveryControl::from_reflect(&unchecked_control).is_none());
        assert!(discovery_control.try_apply(&unchecked_control).is_err());
    }

    fn assert_control_state_unchanged(discovery_control: &DiscoveryControl, reporter: ReporterId) {
        assert_eq!(discovery_control.reporters.len(), 1);
        let reporter_control = &discovery_control.reporters[0];
        assert_eq!(reporter_control.reporter, reporter);
        assert_eq!(
            reporter_control.startup_requirement,
            StartupRequirement::Required
        );
        assert_eq!(reporter_control.activation, ReporterActivation::Enabled);
        assert!(matches!(
            reporter_control.request,
            DiscoveryRequest::Requested
        ));
        assert!(matches!(reporter_control.dirty, DiscoveryDirtyState::Clean));
    }
}
