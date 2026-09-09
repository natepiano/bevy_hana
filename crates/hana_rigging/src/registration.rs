use std::any::type_name;
use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

use bevy::app::App;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::relationship::Relationship;
use bevy::log::info;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use bevy::time::Real;
use bevy::time::Time;

use crate::BatchRef;
use crate::CapabilityProjectionFailure;
use crate::CapabilityProjectionStatus;
use crate::DeviceAccessErrorView;
use crate::DeviceReporter;
use crate::DeviceScan;
use crate::DiscoveryBatchId;
use crate::DiscoveryCadence;
use crate::DiscoveryControl;
use crate::DiscoveryProgress;
use crate::DiscoveryProgressSender;
use crate::DiscoveryProgressView;
use crate::DiscoverySchedulerState;
use crate::DiscoveryStatus;
use crate::DiscoveryWork;
use crate::DriverContractError;
use crate::FirstCompleteSetStatus;
use crate::LastDiscoveryOutcome;
use crate::PreviousSuccess;
use crate::RegisteredSchemes;
use crate::ReportAcceptanceProjection;
use crate::ReporterActivation;
use crate::ReporterActivity;
use crate::ReporterActivityView;
use crate::ReporterCoverage;
use crate::ReporterDeferral;
use crate::ReporterDiscoveryStatus;
use crate::ReporterHealth;
use crate::ReporterId;
use crate::ReporterOutcomeHealth;
use crate::ReporterRegistration;
use crate::ReporterResume;
use crate::RiggingRuntimeClock;
use crate::RiggingRuntimeTime;
use crate::SchemeName;
use crate::StartupDiscoveryState;
use crate::StartupRequirement;
use crate::WaitTiming;
use crate::binding::RiggingRoleRelationshipRepairs;
use crate::capabilities::CapabilitySource;
use crate::contract::DiscoveryProgressReceiver;
use crate::contract::DriverEntry;
use crate::contract::DriverReports;
use crate::contract::EndpointDriver;
use crate::contract::EndpointDriverRegistration;
use crate::contract::ErasedTarget;
use crate::contract::PendingDiscoveryProgress;
use crate::contract::SessionDatumArrivalReceiver;
use crate::discovery;
use crate::discovery::CompletedDiscoveryOutcome;
use crate::discovery::DiscoveryDirtyState;
use crate::discovery::DiscoveryLimits;
use crate::discovery::DiscoveryRequest;
use crate::discovery::DiscoveryTransition;
use crate::discovery::DiscoveryTransitionJournal;
use crate::presence;
use crate::presence::DeviceSet;
use crate::presence::RetainedSetChange;
use crate::reporter_health;
use crate::reporter_health::ReporterFailureRun;
use crate::reporter_health::ReporterLogState;
use crate::reporter_health::SuccessfulReportContext;

/// Process-local driver handle that the driver registry issues in registration order.
///
/// `DriverId` has no public constructor because a binding receives it from
/// `RiggingAppExt::add_endpoint_driver`; allowing app code or a driver to fabricate one could route
/// an apply to a different registered implementation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect)]
#[reflect(opaque)]
pub struct DriverId(pub(crate) u32);

/// Proof that the kernel authorized an endpoint apply.
///
/// The private field and constructor prevent a driver from manufacturing permission for a device
/// whose identity did not authorize it. `#[reflect(opaque)]` extends that boundary to reflection:
/// a dynamic reflected tuple cannot construct this token.
///
/// The type is public and nameable, because an authorized apply receives one:
///
/// ```
/// fn authorized(_: hana_rigging::ApplyPermit) {}
/// ```
///
/// It cannot be built, and it cannot be taken apart by pattern either — a
/// destructuring match would be a second way to reach the same private field.
/// The signature above is what keeps these cases meaningful: a rename would
/// break it loudly rather than leaving these failing for an unrelated reason.
///
/// ```compile_fail,E0423
/// let _ = hana_rigging::ApplyPermit(());
/// ```
///
/// ```compile_fail,E0532
/// use hana_rigging::ApplyPermit;
///
/// fn cannot_match(permit: ApplyPermit) {
///     let ApplyPermit(_) = permit;
/// }
/// ```
#[derive(Clone, Copy, Debug, Reflect)]
#[reflect(opaque)]
pub struct ApplyPermit(());

impl ApplyPermit {
    /// Create the token that permits an authorized device operation.
    ///
    /// Crate-private because `Devices::authorize_service` is the only in-service decision point; a
    /// driver that could fabricate one would be claiming an authorization the kernel never granted.
    pub(crate) const fn authorized() -> Self { Self(()) }
}

/// Whether a reporter can supply an accepted complete device set to reconciliation.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "reconciliation reads retained sets through `Reporters::registered_reporters`, \
                  and this per-reporter lookup answers the handoff question for tests and later \
                  diagnostics"
    )
)]
enum ReporterDeviceSetState<'a> {
    /// The requested `ReporterId` does not select a registry entry.
    NotRegistered,
    /// The reporter is registered but has no successfully accepted complete set.
    AwaitingCompleteSet,
    /// The reporter's latest successfully accepted complete set remains retained.
    Available(&'a DeviceSet),
}

/// Whether an accepted complete set still needs one reconciliation pass.
#[derive(Default)]
enum ReporterReconciliationRequest {
    /// Reconciliation has consumed every accepted complete set.
    #[default]
    Settled,
    /// At least one complete set was accepted after the preceding reconciliation pass.
    AcceptedCompleteSet,
}

/// Erased reporter implementations and their registry-owned discovery state.
///
/// The collect system borrows this resource out of `World` before it calls
/// `DeviceReporter::discover` or runs a returned `MainThreadDiscoveryJob`. The world-free reporter
/// method, the absent registry during main-thread enumeration, and the background job's sendable
/// closure prevent every discovery boundary from accessing its own registration.
#[derive(Default, Resource)]
pub(crate) struct Reporters {
    entries:                Vec<ReporterEntry>,
    next_batch:             u64,
    next_id:                u32,
    changed:                Vec<ReporterId>,
    reconciliation_request: ReporterReconciliationRequest,
}

impl Reporters {
    #[cfg(test)]
    fn add<Reporter>(
        &mut self,
        reporter: Reporter,
        reporter_registration: ReporterRegistration,
    ) -> ReporterId
    where
        Reporter: DeviceReporter,
    {
        let reporter_id = ReporterId(self.next_id);
        self.next_id += 1;
        self.entries.push(ReporterEntry::new(
            reporter,
            reporter_id,
            reporter_registration,
        ));

        reporter_id
    }

    fn add_published<Reporter>(
        &mut self,
        world: &mut World,
        reporter: Reporter,
        reporter_registration: ReporterRegistration,
        registered_at: RiggingRuntimeTime,
    ) -> ReporterId
    where
        Reporter: DeviceReporter,
    {
        let reporter_id = ReporterId(self.next_id);
        self.next_id += 1;
        let reporter_health = ReporterHealth::at_registration(
            reporter_id,
            type_name::<Reporter>().to_owned(),
            &reporter_registration,
            registered_at,
        );
        let reporter_entity = world.spawn(reporter_health.clone()).id();
        self.entries.push(
            ReporterEntry::new(reporter, reporter_id, reporter_registration)
                .with_published_health(reporter_entity, reporter_health),
        );

        reporter_id
    }

    pub(crate) fn collect(
        &mut self,
        world: &mut World,
        now: Instant,
        discovery_control: &mut DiscoveryControl,
        discovery_limits: &DiscoveryLimits,
        discovery_status: &mut DiscoveryStatus,
        journal: &mut DiscoveryTransitionJournal,
    ) {
        let runtime_clock = *world.resource::<RiggingRuntimeClock>();
        let runtime_time = runtime_clock.time_at(now);
        let capacity =
            discovery::discovery_transition_capacity(self.entries.len(), discovery_limits);
        self.poll_running(now);
        self.accept_completed(
            world,
            discovery_control,
            discovery_limits,
            discovery_status,
            journal,
            capacity,
        );
        self.refresh_startup(discovery_status, journal, capacity);
        self.sync_activation(discovery_control, runtime_time);
        self.queue_due(now, discovery_control, discovery_status);
        self.admit(
            world,
            now,
            discovery_control,
            discovery_limits,
            discovery_status,
        );
        self.refresh_startup(discovery_status, journal, capacity);
        self.refresh_activity(now, discovery_limits, discovery_status, journal, capacity);
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "reconciliation iterates every reporter instead of selecting one, so this \
                      lookup serves tests and later diagnostics"
        )
    )]
    fn latest_device_set(&self, reporter: ReporterId) -> ReporterDeviceSetState<'_> {
        let Some(reporter_entry) = self
            .entries
            .iter()
            .find(|reporter_entry| reporter_entry.reporter_id == reporter)
        else {
            return ReporterDeviceSetState::NotRegistered;
        };

        match &reporter_entry.latest_set {
            RetainedDeviceSet::NotCompleted => ReporterDeviceSetState::AwaitingCompleteSet,
            RetainedDeviceSet::Complete { device_set, .. } => {
                ReporterDeviceSetState::Available(device_set)
            },
        }
    }

    /// Report whether any reporter has completed a scan since the last reconcile pass.
    ///
    /// The reconcile pass asks this before merging, so the queue is left in place for
    /// `Self::take_changed_reporters` to drain once the merge is actually going to happen.
    pub(crate) const fn any_reporter_changed(&self) -> bool {
        matches!(
            self.reconciliation_request,
            ReporterReconciliationRequest::AcceptedCompleteSet
        )
    }

    pub(crate) fn take_changed_reporters(&mut self) -> Vec<ReporterId> {
        self.reconciliation_request = ReporterReconciliationRequest::Settled;
        std::mem::take(&mut self.changed)
    }

    /// Report every registered reporter with the cadence it declared and what it can contribute to
    /// the current reconcile pass.
    ///
    /// Reconciliation borrows retained sets through this iterator instead of copying them, so the
    /// registry stays the single owner of reporter evidence and a reporter that did not re-scan
    /// this frame still contributes its latest complete set.
    pub(crate) fn registered_reporters(&self) -> impl Iterator<Item = RegisteredReporter<'_>> {
        self.entries
            .iter()
            .map(|reporter_entry| RegisteredReporter {
                reporter:                 reporter_entry.reporter_id,
                activation:               reporter_entry.activation(),
                cadence:                  reporter_entry.registration.cadence(),
                first_complete_set_bound: reporter_entry.registration.first_complete_set_bound(),
                coverage:                 reporter_entry.registration.coverage(),
                contribution:             match &reporter_entry.latest_set {
                    RetainedDeviceSet::NotCompleted => {
                        ReporterContribution::AwaitingFirstCompleteSet
                    },
                    RetainedDeviceSet::Complete {
                        batch,
                        device_set,
                        freshness_anchor,
                        ..
                    } => ReporterContribution::Completed {
                        batch: *batch,
                        device_set,
                        freshness_anchor: *freshness_anchor,
                    },
                },
            })
    }

    /// Move one reporter's retained freshness anchor backwards so a test can reach the freshness
    /// lease without advancing a clock or sleeping.
    #[cfg(test)]
    pub(crate) fn backdate_completion(&mut self, reporter: ReporterId, by: Duration) {
        for reporter_entry in &mut self.entries {
            if reporter_entry.reporter_id != reporter {
                continue;
            }
            if let RetainedDeviceSet::Complete {
                freshness_anchor, ..
            } = &mut reporter_entry.latest_set
            {
                *freshness_anchor -= by;
            }
        }
    }

    fn poll_running(&mut self, now: Instant) {
        for reporter_entry in &mut self.entries {
            reporter_entry.drain_progress();
            reporter_entry.collect_finished_run(now);
        }
    }

    fn accept_completed(
        &mut self,
        world: &mut World,
        discovery_control: &DiscoveryControl,
        discovery_limits: &DiscoveryLimits,
        discovery_status: &mut DiscoveryStatus,
        journal: &mut DiscoveryTransitionJournal,
        capacity: usize,
    ) {
        let runtime_clock = *world.resource::<RiggingRuntimeClock>();
        let completed =
            self.completed_in_acceptance_order(discovery_limits.max_completions_per_frame().get());
        for index in completed {
            self.accept_reporter_completion(
                index,
                world,
                discovery_control,
                discovery_status,
                journal,
                capacity,
                runtime_clock,
            );
        }
    }

    fn completed_in_acceptance_order(&self, maximum: usize) -> Vec<usize> {
        let mut completed = Vec::new();
        for (index, reporter_entry) in self.entries.iter().enumerate() {
            match reporter_entry.completion_time() {
                ReporterCompletionTime::NoCompletedResult => {},
                ReporterCompletionTime::CompletedAt(completed_at) => {
                    completed.push((index, !reporter_entry.is_required(), completed_at));
                },
            }
        }
        completed.sort_by_key(|(_, optional, completed_at)| (*optional, *completed_at));
        completed
            .into_iter()
            .take(maximum)
            .map(|(index, _, _)| index)
            .collect()
    }

    fn accept_reporter_completion(
        &mut self,
        index: usize,
        world: &mut World,
        discovery_control: &DiscoveryControl,
        discovery_status: &mut DiscoveryStatus,
        journal: &mut DiscoveryTransitionJournal,
        transition_capacity: usize,
        runtime_clock: RiggingRuntimeClock,
    ) {
        let Self {
            entries,
            changed,
            reconciliation_request,
            ..
        } = self;
        let reporter_entry = &mut entries[index];
        let reporter_id = reporter_entry.reporter_id;
        let completed_discovery =
            match reporter_entry.take_completed(discovery_control.activation(reporter_id)) {
                ReporterCompletionAcceptance::NoCompletedResult => return,
                ReporterCompletionAcceptance::Accepted(completed_discovery) => completed_discovery,
            };

        let Ok(reporter_discovery_status) = discovery_status.reporter_status_mut(reporter_id)
        else {
            return;
        };
        let accepted_completion = AcceptedReporterCompletion {
            reporter: reporter_id,
            batch: completed_discovery.batch,
            accepted_at: completed_discovery.accepted_at,
            duration: completed_discovery.measured_work_time,
            completed_runtime_time: runtime_clock.time_at(completed_discovery.accepted_at),
            failure_run_time: runtime_clock.time_at(completed_discovery.accepted_at),
            previous_success: reporter_entry.previous_success(runtime_clock),
            transition_capacity,
        };
        match CompletedDeviceScan::from(completed_discovery.scan) {
            CompletedDeviceScan::Deferred(deferral) => accept_deferred_scan(
                reporter_entry,
                reporter_discovery_status,
                journal,
                deferral,
                accepted_completion,
            ),
            CompletedDeviceScan::Failed(error) => accept_failed_scan(
                reporter_entry,
                reporter_discovery_status,
                journal,
                world,
                error,
                accepted_completion,
            ),
            CompletedDeviceScan::Complete(successful_scan) => accept_complete_scan(
                reporter_entry,
                reporter_discovery_status,
                changed,
                reconciliation_request,
                journal,
                world,
                successful_scan,
                accepted_completion,
            ),
        }
    }

    fn sync_activation(
        &mut self,
        discovery_control: &DiscoveryControl,
        runtime_time: RiggingRuntimeTime,
    ) {
        for reporter_entry in &mut self.entries {
            let reporter_id = reporter_entry.reporter_id;
            reporter_entry.sync_activation(discovery_control.activation(reporter_id), runtime_time);
        }
    }

    pub(crate) fn write_health(
        &mut self,
        world: &mut World,
        now: Instant,
        runtime_clock: RiggingRuntimeClock,
    ) {
        let runtime_time = runtime_clock.time_at(now);
        for reporter_entry in &mut self.entries {
            reporter_entry
                .health
                .set_activity(reporter_entry.health_activity(runtime_clock));
            if reporter_entry
                .health
                .cross_first_complete_set_deadline(runtime_time)
            {
                let decision = reporter_entry
                    .log_state
                    .first_complete_set_overdue_decision();
                let roles_waiting = reporter_health::roles_waiting_on(
                    world.resource::<crate::Bindings>(),
                    reporter_entry.health.identity().reporter_ref,
                );
                reporter_health::log_first_complete_set_overdue(
                    &reporter_entry.health,
                    decision,
                    &roles_waiting,
                );
            }

            reporter_entry.publish_health(world);
        }
    }

    pub(crate) fn record_capability_projection_failures(
        &mut self,
        world: &mut World,
        failures: impl IntoIterator<Item = (ReporterId, CapabilityProjectionFailure)>,
    ) {
        let mut failures_by_reporter: HashMap<ReporterId, Vec<CapabilityProjectionFailure>> =
            HashMap::new();
        for (reporter, failure) in failures {
            failures_by_reporter
                .entry(reporter)
                .or_default()
                .push(failure);
        }
        for reporter_entry in &mut self.entries {
            let Some(failures) = failures_by_reporter.remove(&reporter_entry.reporter_id) else {
                continue;
            };
            reporter_entry
                .health
                .record_capability_projection_failures(failures);
            reporter_entry.publish_health(world);
        }
    }

    fn queue_due(
        &mut self,
        now: Instant,
        discovery_control: &mut DiscoveryControl,
        discovery_status: &mut DiscoveryStatus,
    ) {
        let startup_ready = matches!(discovery_status.startup, StartupDiscoveryState::Ready);
        let mut due_indexes = Vec::new();
        for (index, reporter_entry) in self.entries.iter_mut().enumerate() {
            if !reporter_entry.is_required() && !startup_ready {
                continue;
            }

            let reporter_id = reporter_entry.reporter_id;
            let requested = discovery_control.take_request(reporter_id);
            let dirty = discovery_control.take_dirty(reporter_id);
            if reporter_entry.record_due_signal(now, requested, dirty) {
                due_indexes.push(index);
            }
        }

        if due_indexes.is_empty() {
            return;
        }

        let batch = DiscoveryBatchId(self.next_batch);
        self.next_batch += 1;
        for index in due_indexes {
            let reporter_entry = &mut self.entries[index];
            reporter_entry.queue(batch, now);
            if let Ok(reporter_discovery_status) =
                discovery_status.reporter_status_mut(reporter_entry.reporter_id)
            {
                reporter_discovery_status.activity = ReporterActivity::Queued { batch };
            }
        }
    }

    fn admit(
        &mut self,
        world: &mut World,
        now: Instant,
        discovery_control: &DiscoveryControl,
        discovery_limits: &DiscoveryLimits,
        discovery_status: &mut DiscoveryStatus,
    ) {
        let mut queued = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, reporter_entry)| reporter_entry.can_prepare())
            .map(|(index, reporter_entry)| (index, !reporter_entry.is_required()))
            .collect::<Vec<_>>();
        queued.sort_by_key(|(_, optional)| *optional);

        for (index, optional) in queued {
            if optional && !matches!(discovery_status.startup, StartupDiscoveryState::Ready) {
                continue;
            }

            let reporter_entry = &mut self.entries[index];
            reporter_entry.prepare_at(world, now);
            self.start_prepared_background(
                index,
                now,
                discovery_control,
                discovery_limits,
                discovery_status,
            );
        }

        let prepared = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, reporter_entry)| reporter_entry.has_prepared_background())
            .map(|(index, reporter_entry)| (index, !reporter_entry.is_required()))
            .collect::<Vec<_>>();
        for (index, optional) in prepared {
            if optional && !matches!(discovery_status.startup, StartupDiscoveryState::Ready) {
                continue;
            }
            self.start_prepared_background(
                index,
                now,
                discovery_control,
                discovery_limits,
                discovery_status,
            );
        }
    }

    fn start_prepared_background(
        &mut self,
        index: usize,
        now: Instant,
        discovery_control: &DiscoveryControl,
        discovery_limits: &DiscoveryLimits,
        discovery_status: &mut DiscoveryStatus,
    ) {
        if !self.entries[index].has_prepared_background()
            || discovery_control.activation(self.entries[index].reporter_id)
                == ReporterActivation::Disabled
        {
            return;
        }
        if self.running_count() >= Self::effective_capacity(discovery_limits, discovery_status) {
            return;
        }
        self.entries[index].start_prepared_background(now);
    }

    fn effective_capacity(
        discovery_limits: &DiscoveryLimits,
        discovery_status: &mut DiscoveryStatus,
    ) -> usize {
        match discovery_limits.effective_max_concurrent_jobs() {
            Ok(capacity) => {
                discovery_status.scheduler = DiscoverySchedulerState::Available;
                capacity.get()
            },
            Err(error) => {
                discovery_status.scheduler = DiscoverySchedulerState::Failed { error };
                0
            },
        }
    }

    fn running_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|reporter_entry| reporter_entry.is_running())
            .count()
    }

    fn refresh_startup(
        &self,
        discovery_status: &mut DiscoveryStatus,
        journal: &mut DiscoveryTransitionJournal,
        capacity: usize,
    ) {
        let startup_before = discovery_status.startup.clone();
        for reporter_entry in self
            .entries
            .iter()
            .filter(|reporter_entry| reporter_entry.is_required())
        {
            let reporter_id = reporter_entry.reporter_id;
            let Ok(reporter_discovery_status) = discovery_status.reporter_status(reporter_id)
            else {
                continue;
            };
            if let LastDiscoveryOutcome::Failed { error, .. }
            | LastDiscoveryOutcome::Unsupported { error, .. } =
                &reporter_discovery_status.last_outcome
            {
                discovery_status.startup = StartupDiscoveryState::BlockedByFailure {
                    reporter: reporter_id,
                    error:    error.clone(),
                };
                record_startup_edge(&startup_before, discovery_status, journal, capacity);
                return;
            }
        }

        let all_required_reporters_succeeded = self
            .entries
            .iter()
            .filter(|reporter_entry| reporter_entry.is_required())
            .all(|reporter_entry| {
                discovery_status
                    .reporter_status(reporter_entry.reporter_id)
                    .is_ok_and(|reporter_discovery_status| {
                        matches!(
                            reporter_discovery_status.last_outcome,
                            LastDiscoveryOutcome::Succeeded { .. }
                        ) && matches!(
                            &reporter_entry.latest_set,
                            RetainedDeviceSet::Complete { .. }
                        )
                    })
            });
        discovery_status.startup = if all_required_reporters_succeeded {
            StartupDiscoveryState::Ready
        } else {
            StartupDiscoveryState::Discovering
        };
        record_startup_edge(&startup_before, discovery_status, journal, capacity);
    }

    /// Write every reporter's current activity, recording the progress edges a consumer may see.
    ///
    /// A progress transition is recorded only for a run that has already been going for
    /// `DiscoveryLimits::progress_after` and whose reported progress differs from what the previous
    /// pass retained. Both conditions are the point: a run that finishes inside the delay produces
    /// no progress traffic at all, and a job that keeps reporting the same count produces one edge
    /// rather than one per frame.
    fn refresh_activity(
        &self,
        now: Instant,
        discovery_limits: &DiscoveryLimits,
        discovery_status: &mut DiscoveryStatus,
        journal: &mut DiscoveryTransitionJournal,
        capacity: usize,
    ) {
        let batch_counts = self.batch_counts(now, discovery_status);
        for reporter_entry in &self.entries {
            let activity = reporter_entry.activity(now);
            let Ok(reporter_discovery_status) =
                discovery_status.reporter_status_mut(reporter_entry.reporter_id)
            else {
                continue;
            };
            let moved = reporter_discovery_status.activity != activity;
            reporter_discovery_status.activity = activity;
            let ReporterActivity::Running {
                batch,
                elapsed,
                progress,
            } = &reporter_discovery_status.activity
            else {
                continue;
            };
            if !moved || *elapsed < discovery_limits.progress_after() {
                continue;
            }
            let DiscoveryBatchCounts {
                completed,
                total,
                running,
                queued,
                ..
            } = batch_counts
                .iter()
                .find(|batch_counts| batch_counts.batch == *batch)
                .copied()
                .unwrap_or_else(|| DiscoveryBatchCounts::empty(*batch));
            journal.record(
                capacity,
                DiscoveryTransition::Progressed {
                    batch: *batch,
                    reporter: reporter_entry.reporter_id,
                    progress: progress.clone(),
                    completed,
                    total,
                    running,
                    queued,
                },
            );
        }
    }

    /// Count how far each batch with a reporter still in it has got.
    ///
    /// A reporter's membership comes from its current activity while the batch is live and from its
    /// retained `LastDiscoveryOutcome` once it is not: a reporter that finished stops naming the
    /// batch in its activity, and the finished count is exactly the reporters that did so.
    fn batch_counts(
        &self,
        now: Instant,
        discovery_status: &DiscoveryStatus,
    ) -> Vec<DiscoveryBatchCounts> {
        let mut batch_counts: Vec<DiscoveryBatchCounts> = Vec::new();
        for reporter_entry in &self.entries {
            let membership = match reporter_entry.activity(now) {
                ReporterActivity::Queued { batch } => (batch, BatchMembership::Queued),
                ReporterActivity::Running { batch, .. } => (batch, BatchMembership::Running),
                ReporterActivity::Disabled | ReporterActivity::Idle => {
                    match discovery_status.reporter_status(reporter_entry.reporter_id) {
                        Ok(ReporterDiscoveryStatus {
                            last_outcome:
                                LastDiscoveryOutcome::Deferred { batch, .. }
                                | LastDiscoveryOutcome::Succeeded { batch, .. }
                                | LastDiscoveryOutcome::Failed { batch, .. }
                                | LastDiscoveryOutcome::Unsupported { batch, .. },
                            ..
                        }) => (*batch, BatchMembership::Finished),
                        _ => continue,
                    }
                },
            };
            let (batch, batch_membership) = membership;
            if !batch_counts
                .iter()
                .any(|discovery_batch_counts| discovery_batch_counts.batch == batch)
            {
                batch_counts.push(DiscoveryBatchCounts::empty(batch));
            }
            let Some(discovery_batch_counts) = batch_counts
                .iter_mut()
                .find(|discovery_batch_counts| discovery_batch_counts.batch == batch)
            else {
                continue;
            };
            discovery_batch_counts.total += 1;
            match batch_membership {
                BatchMembership::Queued => discovery_batch_counts.queued += 1,
                BatchMembership::Running => discovery_batch_counts.running += 1,
                BatchMembership::Finished => discovery_batch_counts.completed += 1,
            }
        }

        batch_counts
    }
}

/// How far one batch of discovery runs has got, counted over the reporters that belong to it.
#[derive(Clone, Copy)]
struct DiscoveryBatchCounts {
    batch:     DiscoveryBatchId,
    completed: usize,
    total:     usize,
    running:   usize,
    queued:    usize,
}

impl DiscoveryBatchCounts {
    const fn empty(batch: DiscoveryBatchId) -> Self {
        Self {
            batch,
            completed: 0,
            total: 0,
            running: 0,
            queued: 0,
        }
    }
}

/// Which count one reporter contributes to its batch.
#[derive(Clone, Copy)]
enum BatchMembership {
    /// Waiting for a job slot.
    Queued,
    /// Enumerating hardware now.
    Running,
    /// Its run ended, whether it succeeded or failed.
    Finished,
}

/// Record the startup gate's move, and nothing at all when it did not move.
///
/// `Reporters::collect` refreshes the gate twice per pass, before and after admission, so an
/// unconditional record would emit two identical events every frame the gate stayed put.
fn record_startup_edge(
    startup_before: &StartupDiscoveryState,
    discovery_status: &DiscoveryStatus,
    journal: &mut DiscoveryTransitionJournal,
    capacity: usize,
) {
    if &discovery_status.startup == startup_before {
        return;
    }
    journal.record(
        capacity,
        DiscoveryTransition::StartupChanged {
            startup: discovery_status.startup.clone(),
        },
    );
}

/// One registered reporter as reconciliation reads it: its handle, its declared cadence, and what
/// it can contribute right now.
pub(crate) struct RegisteredReporter<'a> {
    pub(crate) reporter:                 ReporterId,
    pub(crate) activation:               ReporterActivation,
    pub(crate) cadence:                  &'a DiscoveryCadence,
    pub(crate) first_complete_set_bound: Duration,
    /// What this reporter's omission of a durable key is worth, which decides whether a complete
    /// set without an authored unit proves the unit is gone or proves nothing at all.
    pub(crate) coverage:                 &'a ReporterCoverage,
    pub(crate) contribution:             ReporterContribution<'a>,
}

/// What one registered reporter offers the current reconcile pass.
///
/// The kernel schedules completions, stamps reporter health, and judges retained-set freshness on
/// the frame clock. Scan duration is measured separately from that frame instant, and a failed scan
/// retains the preceding set and its freshness anchor.
pub(crate) enum ReporterContribution<'a> {
    /// The reporter has never completed a scan. It contributes no devices and is never stale by
    /// the clock: it is starting up, not late.
    AwaitingFirstCompleteSet,
    /// The reporter's latest accepted whole set and the instant used by its freshness lease.
    Completed {
        batch:            BatchRef,
        device_set:       &'a DeviceSet,
        freshness_anchor: Instant,
    },
}

struct ReporterEntry {
    reporter:        Box<dyn DeviceReporter>,
    reporter_id:     ReporterId,
    registration:    ReporterRegistration,
    latest_set:      RetainedDeviceSet,
    next_due:        NextDue,
    state:           ReporterRunState,
    reporter_entity: Entity,
    health:          ReporterHealth,
    failure_run:     ReporterFailureRun,
    log_state:       ReporterLogState,
}

enum RetainedDeviceSet {
    NotCompleted,
    Complete {
        batch:                 BatchRef,
        device_set:            DeviceSet,
        completed_at:          Instant,
        freshness_anchor:      Instant,
        capability_projection: CapabilityProjectionStatus,
    },
}

enum NextDue {
    NotScheduled,
    At(Instant),
}

enum ReporterRunState {
    Disabled,
    Idle {
        rerun: RerunRequest,
    },
    Queued {
        batch:   DiscoveryBatchId,
        rerun:   RerunRequest,
        pending: PendingDiscovery,
    },
    Running {
        task:                        Task<DeviceScan>,
        batch:                       DiscoveryBatchId,
        started_at:                  Instant,
        discovery_progress_receiver: DiscoveryProgressReceiver,
        progress:                    DiscoveryProgress,
        rerun:                       RerunRequest,
    },
    Unsupported {
        error: crate::DeviceAccessError,
        since: RiggingRuntimeTime,
    },
}

enum PendingDiscovery {
    Admission,
    Background(crate::DiscoveryJob),
    Completed {
        scan:               DeviceScan,
        accepted_at:        Instant,
        measured_work_time: Duration,
    },
}

#[derive(Clone, Copy)]
enum RerunRequest {
    NotRequested,
    Requested,
}

struct CompletedDiscovery {
    scan:               DeviceScan,
    batch:              DiscoveryBatchId,
    accepted_at:        Instant,
    measured_work_time: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReporterCompletionTime {
    NoCompletedResult,
    CompletedAt(Instant),
}

enum ReporterCompletionAcceptance {
    NoCompletedResult,
    Accepted(CompletedDiscovery),
}

/// One completed reporter result normalized from the public `DeviceScan` paths.
enum CompletedDeviceScan {
    Complete(SuccessfulDeviceScan),
    Deferred(ReporterDeferral),
    Failed(crate::DeviceAccessError),
}

impl From<DeviceScan> for CompletedDeviceScan {
    fn from(scan: DeviceScan) -> Self {
        match scan {
            DeviceScan::Complete(devices) => Self::Complete(SuccessfulDeviceScan {
                devices,
                projection: SuccessfulReportProjection::KernelStateOnly,
            }),
            DeviceScan::CompleteWithProjection {
                devices,
                report_acceptance_projection,
            } => Self::Complete(SuccessfulDeviceScan {
                devices,
                projection: SuccessfulReportProjection::IntegrationState(
                    report_acceptance_projection,
                ),
            }),
            DeviceScan::Deferred(reporter_deferral) => Self::Deferred(reporter_deferral),
            DeviceScan::Failed(error) => Self::Failed(error),
        }
    }
}

/// Successful whole-set data normalized from the two public `DeviceScan` complete paths.
struct SuccessfulDeviceScan {
    devices:    Vec<crate::DeviceRecord>,
    projection: SuccessfulReportProjection,
}

struct AcceptedReporterCompletion {
    reporter:               ReporterId,
    batch:                  DiscoveryBatchId,
    accepted_at:            Instant,
    duration:               Duration,
    completed_runtime_time: RiggingRuntimeTime,
    failure_run_time:       RiggingRuntimeTime,
    previous_success:       PreviousSuccess,
    transition_capacity:    usize,
}

fn accept_deferred_scan(
    reporter_entry: &mut ReporterEntry,
    reporter_status: &mut ReporterDiscoveryStatus,
    journal: &mut DiscoveryTransitionJournal,
    deferral: ReporterDeferral,
    completion: AcceptedReporterCompletion,
) {
    reporter_entry.failure_run.end();
    reporter_status.completed_batches += 1;
    let since = match &reporter_status.last_outcome {
        LastDiscoveryOutcome::Deferred {
            since,
            deferral: preceding_deferral,
            ..
        } if *preceding_deferral == deferral => *since,
        _ => {
            reporter_health::log_reporter_deferral(
                &reporter_entry.health,
                BatchRef::from_batch_id(completion.batch),
                completion.duration,
                deferral,
            );
            completion.completed_runtime_time
        },
    };
    reporter_status.last_outcome = LastDiscoveryOutcome::Deferred {
        batch: completion.batch,
        since,
        deferral,
    };
    reporter_entry
        .health
        .set_outcome(ReporterOutcomeHealth::Deferred { since, deferral });
    journal.record(
        completion.transition_capacity,
        DiscoveryTransition::Finished {
            batch:    completion.batch,
            reporter: completion.reporter,
            outcome:  CompletedDiscoveryOutcome::Deferred {
                duration: completion.duration,
                deferral,
            },
        },
    );
    reporter_entry.schedule_after_completion(completion.accepted_at);
}

fn accept_failed_scan(
    reporter_entry: &mut ReporterEntry,
    reporter_status: &mut ReporterDiscoveryStatus,
    journal: &mut DiscoveryTransitionJournal,
    world: &World,
    error: crate::DeviceAccessError,
    completion: AcceptedReporterCompletion,
) {
    reporter_status.completed_batches += 1;
    let unsupported = matches!(&error, crate::DeviceAccessError::Unsupported { .. });
    reporter_status.last_outcome = if unsupported {
        LastDiscoveryOutcome::Unsupported {
            batch: completion.batch,
            error: error.clone(),
            since: completion.completed_runtime_time,
        }
    } else {
        LastDiscoveryOutcome::Failed {
            batch:    completion.batch,
            duration: completion.duration,
            error:    error.clone(),
        }
    };
    let error_view = DeviceAccessErrorView::from(&error);
    if unsupported {
        let decision = reporter_entry
            .failure_run
            .finish_as_unsupported(&error_view);
        reporter_entry
            .health
            .set_outcome(ReporterOutcomeHealth::Unsupported {
                error: error_view.clone(),
                since: completion.completed_runtime_time,
            });
        let roles_waiting = reporter_health::roles_waiting_on(
            world.resource::<crate::Bindings>(),
            reporter_entry.health.identity().reporter_ref,
        );
        reporter_health::log_unsupported_reporter_failure(
            &reporter_entry.health,
            BatchRef::from_batch_id(completion.batch),
            completion.duration,
            &error_view,
            decision,
            &roles_waiting,
        );
    } else {
        let accepted_failure_run = reporter_entry.failure_run.accept_failure(
            error_view,
            completion.failure_run_time,
            completion.previous_success,
        );
        reporter_entry
            .health
            .set_outcome(ReporterOutcomeHealth::Failing {
                batch:    BatchRef::from_batch_id(completion.batch),
                duration: completion.duration,
                run:      accepted_failure_run.status,
            });
        let roles_waiting = reporter_health::roles_waiting_on(
            world.resource::<crate::Bindings>(),
            reporter_entry.health.identity().reporter_ref,
        );
        reporter_health::log_reporter_failure(
            &reporter_entry.health,
            accepted_failure_run.decisions,
            &roles_waiting,
        );
    }
    journal.record(
        completion.transition_capacity,
        DiscoveryTransition::Finished {
            batch:    completion.batch,
            reporter: completion.reporter,
            outcome:  CompletedDiscoveryOutcome::Failed {
                duration: completion.duration,
                error:    error.clone(),
            },
        },
    );
    if unsupported {
        reporter_entry.stop_unsupported(error, completion.completed_runtime_time);
    } else {
        reporter_entry.schedule_after_completion(completion.accepted_at);
    }
}

fn accept_complete_scan(
    reporter_entry: &mut ReporterEntry,
    reporter_status: &mut ReporterDiscoveryStatus,
    changed: &mut Vec<ReporterId>,
    reconciliation_request: &mut ReporterReconciliationRequest,
    journal: &mut DiscoveryTransitionJournal,
    world: &mut World,
    successful_scan: SuccessfulDeviceScan,
    completion: AcceptedReporterCompletion,
) {
    let successful_report_context = match reporter_entry.health.outcome() {
        ReporterOutcomeHealth::Deferred { .. }
        | ReporterOutcomeHealth::Failing { .. }
        | ReporterOutcomeHealth::Unsupported { .. } => SuccessfulReportContext::Recovery,
        ReporterOutcomeHealth::NotCompleted | ReporterOutcomeHealth::Succeeded { .. } => {
            SuccessfulReportContext::Ordinary
        },
    };
    let SuccessfulDeviceScan {
        mut devices,
        projection,
    } = successful_scan;
    let record_count = devices.len();
    let capability_projection =
        retain_projectable_capabilities(world, completion.reporter, &mut devices);
    let retained_set_change = match &reporter_entry.latest_set {
        RetainedDeviceSet::NotCompleted => RetainedSetChange::Changed,
        RetainedDeviceSet::Complete { device_set, .. } => {
            presence::retained_set_change(&device_set.devices, &devices)
        },
    };
    reporter_entry.latest_set = RetainedDeviceSet::Complete {
        batch:                 BatchRef::from_batch_id(completion.batch),
        device_set:            DeviceSet { devices },
        completed_at:          completion.accepted_at,
        freshness_anchor:      completion.accepted_at,
        capability_projection: capability_projection.clone(),
    };
    *reconciliation_request = ReporterReconciliationRequest::AcceptedCompleteSet;
    if retained_set_change == RetainedSetChange::Changed {
        changed.push(completion.reporter);
    }
    projection.publish(world);
    reporter_status.completed_batches += 1;
    reporter_status.last_outcome = LastDiscoveryOutcome::Succeeded {
        batch:    completion.batch,
        duration: completion.duration,
    };
    if matches!(
        reporter_entry.health.first_complete_set(),
        FirstCompleteSetStatus::Waiting(_)
    ) {
        reporter_entry
            .health
            .set_first_complete_set(FirstCompleteSetStatus::Completed {
                batch: BatchRef::from_batch_id(completion.batch),
                at:    completion.completed_runtime_time,
            });
    }
    reporter_entry
        .health
        .set_outcome(ReporterOutcomeHealth::Succeeded {
            batch: BatchRef::from_batch_id(completion.batch),
            completed_at: completion.completed_runtime_time,
            duration: completion.duration,
            records: record_count,
            capability_projection,
        });
    reporter_entry.health.set_retained_records(record_count);
    reporter_entry.failure_run.end();
    let decisions = reporter_entry.log_state.accepted_success_decisions(
        retained_set_change,
        record_count,
        successful_report_context,
    );
    let roles_waiting = reporter_health::roles_waiting_on(
        world.resource::<crate::Bindings>(),
        reporter_entry.health.identity().reporter_ref,
    );
    reporter_health::log_reporter_success(&reporter_entry.health, decisions, &roles_waiting);
    journal.record(
        completion.transition_capacity,
        DiscoveryTransition::Finished {
            batch:    completion.batch,
            reporter: completion.reporter,
            outcome:  CompletedDiscoveryOutcome::Succeeded {
                duration: completion.duration,
            },
        },
    );
    reporter_entry.schedule_after_completion(completion.accepted_at);
}

fn retain_projectable_capabilities(
    world: &World,
    reporter: ReporterId,
    devices: &mut [crate::DeviceRecord],
) -> CapabilityProjectionStatus {
    let app_type_registry = world.get_resource::<AppTypeRegistry>();
    let failures = match app_type_registry {
        Some(app_type_registry) => {
            let type_registry = app_type_registry.read();
            devices
                .iter_mut()
                .enumerate()
                .flat_map(|(record_index, device_record)| {
                    device_record.capabilities.retain_projectable(
                        CapabilitySource {
                            reporter,
                            record_index,
                        },
                        &type_registry,
                    )
                })
                .collect()
        },
        None => devices
            .iter_mut()
            .enumerate()
            .flat_map(|(record_index, device_record)| {
                device_record
                    .capabilities
                    .retain_without_type_registry(CapabilitySource {
                        reporter,
                        record_index,
                    })
            })
            .collect(),
    };

    CapabilityProjectionStatus::from_failures(failures)
}

/// How accepting one successful report publishes integration-owned state.
enum SuccessfulReportProjection {
    /// This reporter result changes only the kernel's retained device set.
    KernelStateOnly,
    /// This reporter result owns one integration-state update published whenever the set is
    /// accepted, whether or not its retained records changed.
    IntegrationState(ReportAcceptanceProjection),
}

impl SuccessfulReportProjection {
    fn publish(self, world: &mut World) {
        match self {
            Self::KernelStateOnly => {},
            Self::IntegrationState(report_acceptance_projection) => {
                report_acceptance_projection.publish(world);
            },
        }
    }
}

impl ReporterEntry {
    fn publish_health(&mut self, world: &mut World) {
        let health_changed = world
            .get::<ReporterHealth>(self.reporter_entity)
            .is_none_or(|published| published != &self.health);
        if !health_changed {
            return;
        }
        if let Some(mut published) = world.get_mut::<ReporterHealth>(self.reporter_entity) {
            *published = self.health.clone();
        } else {
            self.reporter_entity = world.spawn(self.health.clone()).id();
        }
    }

    const fn activation(&self) -> ReporterActivation {
        match &self.state {
            ReporterRunState::Disabled => ReporterActivation::Disabled,
            ReporterRunState::Idle { .. }
            | ReporterRunState::Queued { .. }
            | ReporterRunState::Running { .. }
            | ReporterRunState::Unsupported { .. } => ReporterActivation::Enabled,
        }
    }

    fn new<Reporter>(
        reporter: Reporter,
        reporter_id: ReporterId,
        registration: ReporterRegistration,
    ) -> Self
    where
        Reporter: DeviceReporter,
    {
        let state = match registration.activation() {
            ReporterActivation::Enabled => ReporterRunState::Idle {
                rerun: RerunRequest::NotRequested,
            },
            ReporterActivation::Disabled => ReporterRunState::Disabled,
        };
        let health = ReporterHealth::at_registration(
            reporter_id,
            type_name::<Reporter>().to_owned(),
            &registration,
            RiggingRuntimeTime::from_elapsed(Duration::ZERO),
        );
        Self {
            reporter: Box::new(reporter),
            reporter_id,
            registration,
            latest_set: RetainedDeviceSet::NotCompleted,
            next_due: NextDue::NotScheduled,
            state,
            reporter_entity: Entity::PLACEHOLDER,
            health,
            failure_run: ReporterFailureRun::default(),
            log_state: ReporterLogState::default(),
        }
    }

    fn with_published_health(mut self, reporter_entity: Entity, health: ReporterHealth) -> Self {
        self.reporter_entity = reporter_entity;
        self.health = health;
        self
    }

    fn is_required(&self) -> bool {
        self.registration.requirement() == StartupRequirement::Required
    }

    const fn completion_time(&self) -> ReporterCompletionTime {
        match &self.state {
            ReporterRunState::Queued {
                pending: PendingDiscovery::Completed { accepted_at, .. },
                ..
            } => ReporterCompletionTime::CompletedAt(*accepted_at),
            ReporterRunState::Disabled
            | ReporterRunState::Idle { .. }
            | ReporterRunState::Queued { .. }
            | ReporterRunState::Running { .. }
            | ReporterRunState::Unsupported { .. } => ReporterCompletionTime::NoCompletedResult,
        }
    }

    fn take_completed(&mut self, activation: ReporterActivation) -> ReporterCompletionAcceptance {
        let state = std::mem::replace(&mut self.state, ReporterRunState::Disabled);
        let ReporterRunState::Queued {
            batch,
            rerun,
            pending:
                PendingDiscovery::Completed {
                    scan,
                    accepted_at,
                    measured_work_time,
                },
            ..
        } = state
        else {
            self.state = state;
            return ReporterCompletionAcceptance::NoCompletedResult;
        };

        self.state = match activation {
            ReporterActivation::Disabled => ReporterRunState::Disabled,
            ReporterActivation::Enabled => ReporterRunState::Idle { rerun },
        };
        ReporterCompletionAcceptance::Accepted(CompletedDiscovery {
            scan,
            batch,
            accepted_at,
            measured_work_time,
        })
    }

    fn schedule_after_completion(&mut self, completed_at: Instant) {
        self.next_due = match self.registration.cadence() {
            DiscoveryCadence::OnDemand => NextDue::NotScheduled,
            DiscoveryCadence::EventDriven { backstop } => NextDue::At(completed_at + *backstop),
            DiscoveryCadence::Periodic { interval } => NextDue::At(completed_at + *interval),
        };
    }

    fn stop_unsupported(&mut self, error: crate::DeviceAccessError, since: RiggingRuntimeTime) {
        self.next_due = NextDue::NotScheduled;
        self.state = ReporterRunState::Unsupported { error, since };
    }

    fn sync_activation(
        &mut self,
        activation: ReporterActivation,
        runtime_time: RiggingRuntimeTime,
    ) {
        match (activation, &self.state) {
            (ReporterActivation::Enabled, ReporterRunState::Disabled) => {
                self.restart_first_complete_set_bound(runtime_time);
                self.state = ReporterRunState::Idle {
                    rerun: RerunRequest::NotRequested,
                };
            },
            (ReporterActivation::Disabled, ReporterRunState::Running { .. })
            | (ReporterActivation::Enabled, _) => {},
            (ReporterActivation::Disabled, _) => {
                self.health.transition_to_disabled(runtime_time);
                self.state = ReporterRunState::Disabled;
            },
        }
    }

    const fn restart_first_complete_set_bound(&mut self, runtime_time: RiggingRuntimeTime) {
        if !matches!(
            self.health.first_complete_set(),
            FirstCompleteSetStatus::Waiting(WaitTiming::Unbounded { .. })
        ) {
            return;
        }

        self.health.begin_first_complete_set_bound(runtime_time);
        self.log_state.restart_first_complete_set_bound();
    }

    fn record_due_signal(
        &mut self,
        now: Instant,
        requested: DiscoveryRequest,
        dirty: DiscoveryDirtyState,
    ) -> bool {
        if let ReporterRunState::Unsupported { error, since } = &self.state {
            if matches!(requested, DiscoveryRequest::Requested) {
                info!(
                    "[hana_rigging] explicitly retrying reporter {:?}, stopped at {:?} after \
                     {error:?}",
                    self.reporter_id,
                    since.elapsed()
                );
                self.state = ReporterRunState::Idle {
                    rerun: RerunRequest::NotRequested,
                };
                return true;
            }
            return false;
        }

        let cadence_due = matches!(self.next_due, NextDue::At(deadline) if deadline <= now);
        let signalled = matches!(requested, crate::discovery::DiscoveryRequest::Requested)
            || matches!(dirty, crate::discovery::DiscoveryDirtyState::Dirty)
            || cadence_due;

        match &mut self.state {
            ReporterRunState::Idle { rerun } => match rerun {
                RerunRequest::Requested => {
                    *rerun = RerunRequest::NotRequested;
                    true
                },
                RerunRequest::NotRequested => signalled,
            },
            ReporterRunState::Queued {
                pending: PendingDiscovery::Completed { .. },
                rerun,
                ..
            }
            | ReporterRunState::Running { rerun, .. } => {
                if signalled {
                    *rerun = RerunRequest::Requested;
                }

                false
            },
            ReporterRunState::Queued { .. }
            | ReporterRunState::Disabled
            | ReporterRunState::Unsupported { .. } => false,
        }
    }

    fn queue(&mut self, batch: DiscoveryBatchId, queued_at: Instant) {
        if matches!(self.next_due, NextDue::At(deadline) if deadline <= queued_at) {
            self.next_due = NextDue::NotScheduled;
        }
        self.state = ReporterRunState::Queued {
            batch,
            rerun: RerunRequest::NotRequested,
            pending: PendingDiscovery::Admission,
        };
    }

    const fn can_prepare(&self) -> bool {
        matches!(
            self.state,
            ReporterRunState::Queued {
                pending: PendingDiscovery::Admission,
                ..
            }
        )
    }

    #[cfg(test)]
    fn prepare(&mut self, world: &mut World) { self.prepare_at(world, Instant::now()); }

    fn prepare_at(&mut self, world: &mut World, scheduled_at: Instant) {
        let discovery_work = self.reporter.discover();
        let ReporterRunState::Queued { pending, .. } = &mut self.state else {
            return;
        };
        *pending = match discovery_work {
            DiscoveryWork::Immediate(main_thread_discovery_job) => {
                let started = Instant::now();
                let scan = main_thread_discovery_job.run(world);
                let measured_work_time = started.elapsed();
                PendingDiscovery::Completed {
                    scan,
                    accepted_at: scheduled_at,
                    measured_work_time,
                }
            },
            DiscoveryWork::Background(discovery_job) => PendingDiscovery::Background(discovery_job),
        };
    }

    const fn has_prepared_background(&self) -> bool {
        matches!(
            self.state,
            ReporterRunState::Queued {
                pending: PendingDiscovery::Background(_),
                ..
            }
        )
    }

    fn start_prepared_background(&mut self, started_at: Instant) {
        self.start_prepared_background_with(|| started_at);
    }

    fn start_prepared_background_with(&mut self, sample_started_at: impl FnOnce() -> Instant) {
        let state = std::mem::replace(&mut self.state, ReporterRunState::Disabled);
        let ReporterRunState::Queued {
            batch,
            rerun,
            pending: PendingDiscovery::Background(discovery_job),
            ..
        } = state
        else {
            self.state = state;
            return;
        };
        let (discovery_progress_sender, discovery_progress_receiver) =
            DiscoveryProgressSender::scheduler_mailbox();
        let started_at = sample_started_at();
        let task =
            IoTaskPool::get().spawn(async move { discovery_job.run(discovery_progress_sender) });
        self.state = ReporterRunState::Running {
            task,
            batch,
            started_at,
            discovery_progress_receiver,
            progress: DiscoveryProgress::Indeterminate,
            rerun,
        };
    }

    fn drain_progress(&mut self) {
        let ReporterRunState::Running {
            discovery_progress_receiver,
            progress,
            ..
        } = &mut self.state
        else {
            return;
        };
        if let PendingDiscoveryProgress::Latest(discovery_progress) =
            discovery_progress_receiver.take_latest()
        {
            *progress = discovery_progress;
        }
    }

    fn collect_finished_run(&mut self, now: Instant) {
        let is_complete = match &mut self.state {
            ReporterRunState::Running { task, .. } => block_on(poll_once(task)),
            ReporterRunState::Disabled
            | ReporterRunState::Idle { .. }
            | ReporterRunState::Queued { .. }
            | ReporterRunState::Unsupported { .. } => None,
        };
        let Some(scan) = is_complete else {
            return;
        };

        let state = std::mem::replace(&mut self.state, ReporterRunState::Disabled);
        let ReporterRunState::Running {
            batch,
            started_at,
            rerun,
            ..
        } = state
        else {
            self.state = state;
            return;
        };
        let measured_work_time = now.saturating_duration_since(started_at);
        self.state = ReporterRunState::Queued {
            batch,
            rerun,
            pending: PendingDiscovery::Completed {
                scan,
                accepted_at: now,
                measured_work_time,
            },
        };
    }

    const fn is_running(&self) -> bool { matches!(self.state, ReporterRunState::Running { .. }) }

    fn activity(&self, now: Instant) -> ReporterActivity {
        match &self.state {
            ReporterRunState::Disabled => ReporterActivity::Disabled,
            ReporterRunState::Idle { .. } | ReporterRunState::Unsupported { .. } => {
                ReporterActivity::Idle
            },
            ReporterRunState::Queued { batch, .. } => ReporterActivity::Queued { batch: *batch },
            ReporterRunState::Running {
                batch,
                started_at,
                progress,
                ..
            } => ReporterActivity::Running {
                batch:    *batch,
                elapsed:  now.duration_since(*started_at),
                progress: progress.clone(),
            },
        }
    }

    fn health_activity(&self, runtime_clock: RiggingRuntimeClock) -> ReporterActivityView {
        match &self.state {
            ReporterRunState::Disabled => ReporterActivityView::Disabled,
            ReporterRunState::Idle { .. } => ReporterActivityView::Idle,
            ReporterRunState::Queued { batch, .. } => ReporterActivityView::Queued {
                batch: BatchRef::from_batch_id(*batch),
            },
            ReporterRunState::Running {
                batch,
                started_at,
                progress,
                ..
            } => ReporterActivityView::Running {
                batch:      BatchRef::from_batch_id(*batch),
                started_at: runtime_clock.time_at(*started_at),
                progress:   DiscoveryProgressView::from(progress),
            },
            ReporterRunState::Unsupported { since, .. } => ReporterActivityView::Stopped {
                since:        *since,
                resumes_when: ReporterResume::ExplicitRequestOrEnable,
            },
        }
    }

    fn previous_success(&self, runtime_clock: RiggingRuntimeClock) -> PreviousSuccess {
        match &self.latest_set {
            RetainedDeviceSet::NotCompleted => PreviousSuccess::Never,
            RetainedDeviceSet::Complete {
                completed_at,
                capability_projection,
                ..
            } => PreviousSuccess::At {
                completed_at:          runtime_clock.time_at(*completed_at),
                capability_projection: capability_projection.clone(),
            },
        }
    }
}

/// Erased endpoint drivers that kernel systems borrow out of `World` for capture and apply.
///
/// Kernel systems call the crate-private dispatch methods through `World::resource_scope`.
/// Endpoint driver implementations must not access `Drivers` from their own trait methods because
/// the resource is absent for that call. Every apply still requires an `ApplyPermit`, whose private
/// construction prevents application code from authorizing a device by selecting a driver route.
///
/// The type is `pub` so that code outside the kernel can name it in a `Res<Drivers>` and ask
/// `is_changed()`. That is the only thing it can do with it: every field and every method is
/// crate-private, so the name grants read access to one change tick and to nothing else. The
/// settled-frame invariant is stated over `Bindings` and `Drivers` together, and an integration
/// test that cannot name half of it cannot assert it.
///
/// `Default` is deliberately not derived. A derived `Default` on a `pub` resource would let
/// outside code call `insert_resource(Drivers::default())` and replace a populated registry with
/// an empty one, silently unregistering every driver; `new` is crate-private so only the kernel
/// can install one. `Reflect` is likewise not derived: `is_changed()` needs the type nameable, not
/// reflected, and a derive would publish only `next_id` because `DriverEntry` holds a boxed erased
/// driver plus function pointers.
#[derive(Resource)]
pub struct Drivers {
    drivers: Vec<DriverEntry>,
    next_id: u32,
}

impl Drivers {
    /// Build the empty registry the kernel plugin installs.
    pub(crate) const fn new() -> Self {
        Self {
            drivers: Vec::new(),
            next_id: 0,
        }
    }

    pub(crate) fn add<Driver>(&mut self, driver: Driver) -> DriverId
    where
        Driver: EndpointDriver,
    {
        let driver_id = DriverId(self.next_id);
        self.next_id += 1;
        self.drivers.push(DriverEntry::new(driver));

        driver_id
    }

    pub(crate) fn resolve_target(
        &mut self,
        world: &mut World,
        driver: DriverId,
        context: &crate::TargetResolutionContext<'_>,
        configuration: &dyn Reflect,
    ) -> Result<crate::TargetResolution<ErasedTarget>, DriverContractError> {
        self.get_mut(driver)
            .ok_or(DriverContractError::DriverNotRegistered { driver_id: driver })?
            .resolve_target(world, context, configuration)
    }

    pub(crate) fn start_apply(
        &mut self,
        world: &mut World,
        driver: DriverId,
        context: crate::TargetResolutionContext<'_>,
        deadline: Instant,
        permit: ApplyPermit,
        attempt: crate::AttemptRef,
        reports: &DriverReports,
        configuration: &dyn Reflect,
        target: ErasedTarget,
    ) -> Result<(), DriverContractError> {
        self.get_mut(driver)
            .ok_or(DriverContractError::DriverNotRegistered { driver_id: driver })?
            .start_apply(
                world,
                context,
                deadline,
                permit,
                attempt,
                reports,
                configuration,
                target,
            )
    }

    pub(crate) fn established(
        &mut self,
        world: &mut World,
        driver: DriverId,
        role: &crate::RoleKey,
        role_entity: Entity,
        attempt: crate::AttemptRef,
        session: crate::SessionRef,
        reports: &DriverReports,
    ) -> Result<SessionDatumArrivalReceiver, DriverContractError> {
        self.get_mut(driver)
            .ok_or(DriverContractError::DriverNotRegistered { driver_id: driver })?
            .established(world, role, role_entity, attempt, session, reports)
    }

    pub(crate) fn cancel_apply(
        &mut self,
        world: &mut World,
        driver: DriverId,
        role: &crate::RoleKey,
        role_entity: crate::DriverCleanupRoleEntity,
        attempt: crate::AttemptRef,
        cause: crate::AttemptInvalidation,
    ) -> Result<(), DriverContractError> {
        self.get_mut(driver)
            .ok_or(DriverContractError::DriverNotRegistered { driver_id: driver })?
            .cancel_apply(world, role, role_entity, attempt, cause)
    }

    pub(crate) fn release_session(
        &mut self,
        world: &mut World,
        driver: DriverId,
        role: &crate::RoleKey,
        role_entity: crate::DriverCleanupRoleEntity,
        session: crate::SessionRef,
        cause: crate::SessionReleaseCause,
    ) -> Result<(), DriverContractError> {
        self.get_mut(driver)
            .ok_or(DriverContractError::DriverNotRegistered { driver_id: driver })?
            .release_session(world, role, role_entity, session, cause)
    }

    fn get_mut(&mut self, driver_id: DriverId) -> Option<&mut DriverEntry> {
        self.drivers.get_mut(usize::try_from(driver_id.0).ok()?)
    }
}

/// Adds Hana Rigging reporters, endpoint drivers, and identity schemes to a Bevy `App`.
///
/// Bevy's `App` cannot receive inherent methods from this crate, so integration crates import
/// `RiggingAppExt` before registering their hardware-specific reporter or driver implementation.
pub trait RiggingAppExt {
    /// Register one reporter with its startup and cadence policy, then return its process-local id.
    ///
    /// This initializes the reporter registry and retained discovery resources itself, so adding a
    /// reporter before `RiggingPlugin` does not make plugin insertion order control whether the
    /// reporter can register.
    fn add_device_reporter<Reporter>(
        &mut self,
        reporter: Reporter,
        reporter_registration: ReporterRegistration,
    ) -> ReporterId
    where
        Reporter: DeviceReporter;

    /// Register one endpoint driver and return the process-local id that bindings use for routing.
    ///
    /// This initializes `Drivers` itself, so adding a driver before `RiggingPlugin` does not make
    /// plugin insertion order control whether the driver can register.
    fn add_endpoint_driver<Driver>(
        &mut self,
        driver: Driver,
    ) -> EndpointDriverRegistration<Driver::Configuration>
    where
        Driver: EndpointDriver;

    /// Register a client-to-role relationship for automatic role-entity recovery.
    ///
    /// When a live role entity is removed outside the kernel, the registered relationship target
    /// identifies every client that must be retargeted to the replacement entity.
    fn register_rigging_role_relationship<Source>(&mut self) -> &mut Self
    where
        Source: Relationship;

    /// Record a reportable device-identity scheme and return this app for further setup.
    ///
    /// The method initializes `RegisteredSchemes` before recording `name`, so an integration
    /// plugin can register its scheme before or after `RiggingPlugin`. Repeated names stay valid:
    /// two reporters using one scheme assert that their reported values are comparable.
    fn register_device_scheme(&mut self, name: SchemeName) -> &mut Self;
}

impl RiggingAppExt for App {
    fn add_device_reporter<Reporter>(
        &mut self,
        reporter: Reporter,
        reporter_registration: ReporterRegistration,
    ) -> ReporterId
    where
        Reporter: DeviceReporter,
    {
        self.init_resource::<DiscoveryControl>()
            .init_resource::<DiscoveryLimits>()
            .init_resource::<DiscoveryStatus>()
            .init_resource::<Reporters>()
            .init_resource::<Time<Real>>();
        let runtime_started_at = self.world().resource::<Time<Real>>().startup();
        let runtime_clock = *self
            .world_mut()
            .get_resource_or_insert_with(|| RiggingRuntimeClock::starting_at(runtime_started_at));
        let registered_at = runtime_clock.time_at(runtime_started_at);
        let reporter_id =
            self.world_mut()
                .resource_scope::<Reporters, _>(|world, mut reporters| {
                    reporters.add_published(
                        world,
                        reporter,
                        reporter_registration.clone(),
                        registered_at,
                    )
                });
        self.world_mut()
            .resource_mut::<DiscoveryControl>()
            .register(reporter_id, &reporter_registration);
        self.world_mut()
            .resource_mut::<DiscoveryStatus>()
            .register(reporter_id, &reporter_registration);

        reporter_id
    }

    fn add_endpoint_driver<Driver>(
        &mut self,
        driver: Driver,
    ) -> EndpointDriverRegistration<Driver::Configuration>
    where
        Driver: EndpointDriver,
    {
        let driver_id = self
            .world_mut()
            .get_resource_or_insert_with(Drivers::new)
            .add(driver);
        EndpointDriverRegistration::new(driver_id)
    }

    fn register_rigging_role_relationship<Source>(&mut self) -> &mut Self
    where
        Source: Relationship,
    {
        self.world_mut().register_component::<Source>();
        self.world_mut()
            .register_component::<Source::RelationshipTarget>();
        self.init_resource::<RiggingRoleRelationshipRepairs>();
        self.world_mut()
            .resource_mut::<RiggingRoleRelationshipRepairs>()
            .register::<Source>();
        self
    }

    fn register_device_scheme(&mut self, name: SchemeName) -> &mut Self {
        self.init_resource::<RegisteredSchemes>();
        self.world_mut()
            .resource_mut::<RegisteredSchemes>()
            .register(name);
        self
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::VecDeque;
    use std::error::Error;
    use std::num::NonZeroU32;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::Receiver;
    use std::sync::mpsc::Sender;
    use std::sync::mpsc::TryRecvError;
    use std::sync::mpsc::channel;
    use std::thread::JoinHandle;
    use std::time::Duration;
    use std::time::Instant;

    use bevy::MinimalPlugins;
    use bevy::app::App;
    use bevy::app::Plugin;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use bevy::prelude::Resource;
    use bevy::prelude::World;
    use bevy::reflect::FromReflect;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;
    use bevy::tasks::IoTaskPool;
    use bevy::tasks::TaskPoolBuilder;
    use bevy::time::Real;
    use bevy::time::Time;
    use bevy::time::TimeUpdateStrategy;

    use super::ApplyPermit;
    use super::DriverId;
    use super::NextDue;
    use super::PendingDiscovery;
    use super::ReporterCompletionAcceptance;
    use super::ReporterCompletionTime;
    use super::ReporterDeviceSetState;
    use super::ReporterEntry;
    use super::ReporterId;
    use super::ReporterRunState;
    use super::Reporters;
    use super::RerunRequest;
    use super::RetainedDeviceSet;
    use super::RiggingAppExt;
    use crate::Applied;
    use crate::ApplyContext;
    use crate::AttachmentPath;
    use crate::AttemptInvalidation;
    use crate::AttemptRef;
    use crate::Capabilities;
    use crate::Claim;
    use crate::DeviceAccessError;
    use crate::DeviceAccessErrorView;
    use crate::DeviceDescriptor;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DeviceRecord;
    use crate::DeviceReporter;
    use crate::DeviceScan;
    use crate::DiscoveryBatchId;
    use crate::DiscoveryCadence;
    use crate::DiscoveryControl;
    use crate::DiscoveryJob;
    use crate::DiscoveryLimits;
    use crate::DiscoveryProgress;
    use crate::DiscoveryStatus;
    use crate::DiscoveryWork;
    use crate::DriverCleanupRoleEntity;
    use crate::DriverCompletion;
    use crate::EndpointDriver;
    use crate::EstablishedContext;
    use crate::FirstCompleteSetStatus;
    use crate::LastDiscoveryOutcome;
    use crate::MainThreadDiscoveryJob;
    use crate::PlatformDeviceHandle;
    use crate::Presence;
    use crate::RegisteredSchemes;
    use crate::ReportAcceptanceProjection;
    use crate::ReportedAs;
    use crate::ReportedId;
    use crate::ReportedParent;
    use crate::ReportedSerial;
    use crate::ReporterActivation;
    use crate::ReporterActivity;
    use crate::ReporterCoverage;
    use crate::ReporterDeferral;
    use crate::ReporterOutcomeHealth;
    use crate::ReporterRegistration;
    use crate::RiggingPlugin;
    use crate::RiggingRevision;
    use crate::RiggingRuntimeClock;
    use crate::RiggingRuntimeTime;
    use crate::RoleKey;
    use crate::SchemeName;
    use crate::SessionRef;
    use crate::SessionReleaseCause;
    use crate::StartupDiscoveryState;
    use crate::TargetResolution;
    use crate::TargetResolutionContext;
    use crate::WaitTiming;
    use crate::discovery;
    use crate::discovery::DiscoveryDirtyState;
    use crate::discovery::DiscoveryRequest;
    use crate::discovery::DiscoveryTransitionJournal;
    use crate::presence::DeviceSet;
    use crate::reporter_health::FailureLogMilestoneIndex;
    use crate::reporter_health::ReporterFailureRun;
    use crate::reporter_health::ReporterLogDecision;
    use crate::reporter_health::ReporterLogState;

    /// How long a test waits for one reporter's background result before calling it stuck.
    ///
    /// The work runs on another thread, so the bound is elapsed time rather than a poll count:
    /// a loaded machine can burn through any fixed number of polls before it schedules that
    /// thread even once, which reports a result that is merely late as one that never arrived.
    const BACKGROUND_RESULT_DEADLINE: Duration = Duration::from_secs(10);

    /// How long to wait between polls once the first one finds the result unfinished.
    const BACKGROUND_RESULT_POLL_INTERVAL: Duration = Duration::from_micros(100);

    #[test]
    fn reporter_registration_and_plugin_order_share_the_runtime_clock_anchor() {
        let registration = || {
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                Duration::from_secs(10),
            )
        };

        let mut reporter_first = App::new();
        reporter_first.add_device_reporter(
            SequenceReporter {
                scans: VecDeque::new(),
            },
            registration(),
        );
        reporter_first.add_plugins(RiggingPlugin);
        let reporter_first_started_at = reporter_first.world().resource::<Time<Real>>().startup();
        let reporter_first_runtime_time = reporter_first
            .world()
            .resource::<RiggingRuntimeClock>()
            .time_at(reporter_first_started_at);

        let mut plugin_first = App::new();
        plugin_first.add_plugins(RiggingPlugin);
        plugin_first.add_device_reporter(
            SequenceReporter {
                scans: VecDeque::new(),
            },
            registration(),
        );
        let plugin_first_started_at = plugin_first.world().resource::<Time<Real>>().startup();
        let plugin_first_runtime_time = plugin_first
            .world()
            .resource::<RiggingRuntimeClock>()
            .time_at(plugin_first_started_at);

        assert_eq!(reporter_first_runtime_time, plugin_first_runtime_time);
        assert_eq!(
            reporter_first_runtime_time,
            RiggingRuntimeTime::from_elapsed(Duration::ZERO)
        );
    }

    #[test]
    fn reporter_diagnostics_emit_failure_and_overdue_decisions_once_per_edge() {
        let error = DeviceAccessErrorView::Transport {
            detail: String::from("test reporter transport failure"),
        };
        let mut failure_run = ReporterFailureRun::default();
        let mut log_state = ReporterLogState::default();

        assert_eq!(
            failure_run.accept_failure_at_elapsed(&error, Duration::ZERO),
            crate::reporter_health::FailureLogDecisions {
                error:     ReporterLogDecision::FirstFailure,
                milestone: ReporterLogDecision::NoLog,
            }
        );
        assert_eq!(
            failure_run.accept_failure_at_elapsed(&error, Duration::ZERO),
            crate::reporter_health::FailureLogDecisions {
                error:     ReporterLogDecision::NoLog,
                milestone: ReporterLogDecision::NoLog,
            }
        );
        assert_eq!(
            failure_run.accept_failure_at_elapsed(&error, Duration::from_secs(60)),
            crate::reporter_health::FailureLogDecisions {
                error:     ReporterLogDecision::NoLog,
                milestone: ReporterLogDecision::FailureMilestone(
                    FailureLogMilestoneIndex::OneMinute,
                ),
            }
        );
        assert_eq!(
            failure_run.accept_failure_at_elapsed(&error, Duration::from_secs(60)),
            crate::reporter_health::FailureLogDecisions {
                error:     ReporterLogDecision::NoLog,
                milestone: ReporterLogDecision::NoLog,
            }
        );
        assert_eq!(
            failure_run.last_logged_milestone(),
            FailureLogMilestoneIndex::OneMinute
        );
        assert_eq!(
            log_state.first_complete_set_overdue_decision(),
            ReporterLogDecision::FirstCompleteSetOverdue
        );
        assert_eq!(
            log_state.first_complete_set_overdue_decision(),
            ReporterLogDecision::NoLog
        );
        log_state.restart_first_complete_set_bound();
        assert_eq!(
            log_state.first_complete_set_overdue_decision(),
            ReporterLogDecision::FirstCompleteSetOverdue
        );
        assert_eq!(
            log_state.most_recent_overdue_decision(),
            ReporterLogDecision::FirstCompleteSetOverdue
        );
    }

    #[test]
    fn deferral_resets_the_failure_decision_run() {
        let failure = || {
            DeviceScan::Failed(DeviceAccessError::Transport {
                detail: String::from("repeated test transport failure"),
            })
        };
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let reporter = app.add_device_reporter(
            SequenceReporter {
                scans: VecDeque::from([
                    failure(),
                    DeviceScan::Deferred(ReporterDeferral::WaitingForTopology),
                    failure(),
                ]),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                Duration::from_secs(10),
            ),
        );

        update_until_completed_batches(&mut app, reporter, 1);
        {
            let reporters = app.world().resource::<Reporters>();
            let reporter_entry = reporters
                .entries
                .iter()
                .find(|entry| entry.reporter_id == reporter)
                .expect("registered reporter must retain its log state");
            assert_eq!(
                reporter_entry.failure_run.most_recent_decisions(),
                crate::reporter_health::FailureLogDecisions {
                    error:     ReporterLogDecision::FirstFailure,
                    milestone: ReporterLogDecision::NoLog,
                }
            );
        }

        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .request(reporter)
            .expect("the registered reporter must accept a deferral run request");
        update_until_completed_batches(&mut app, reporter, 2);
        {
            let reporters = app.world().resource::<Reporters>();
            let reporter_entry = reporters
                .entries
                .iter()
                .find(|entry| entry.reporter_id == reporter)
                .expect("registered reporter must retain its log state");
            assert_eq!(
                reporter_entry.failure_run.most_recent_decisions(),
                crate::reporter_health::FailureLogDecisions::default()
            );
        }

        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .request(reporter)
            .expect("the registered reporter must accept a second failure run request");
        update_until_completed_batches(&mut app, reporter, 3);
        let reporters = app.world().resource::<Reporters>();
        let reporter_entry = reporters
            .entries
            .iter()
            .find(|entry| entry.reporter_id == reporter)
            .expect("registered reporter must retain its log state");
        assert_eq!(
            reporter_entry.failure_run.most_recent_decisions(),
            crate::reporter_health::FailureLogDecisions {
                error:     ReporterLogDecision::FirstFailure,
                milestone: ReporterLogDecision::NoLog,
            }
        );
    }

    #[test]
    fn one_minute_periodic_failure_run_emits_one_typed_milestone_decision() {
        const FAILURES_AFTER_RUN_START: u64 = 60;
        const REPORTER_CADENCE: Duration = Duration::from_secs(1);

        let scans = (0..=FAILURES_AFTER_RUN_START + 1)
            .map(|_| {
                DeviceScan::Failed(DeviceAccessError::Transport {
                    detail: String::from("one-minute test transport failure"),
                })
            })
            .collect::<VecDeque<_>>();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
            .add_plugins(RiggingPlugin);
        let reporter = app.add_device_reporter(
            SequenceReporter { scans },
            ReporterRegistration::required(
                DiscoveryCadence::Periodic {
                    interval: REPORTER_CADENCE,
                },
                ReporterCoverage::MatchingEvidenceOnly,
                Duration::from_secs(30),
            ),
        );

        update_until_completed_batches(&mut app, reporter, 1);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(REPORTER_CADENCE));
        update_until_completed_batches(&mut app, reporter, FAILURES_AFTER_RUN_START + 1);
        {
            let reporters = app.world().resource::<Reporters>();
            let reporter_entry = reporters
                .entries
                .iter()
                .find(|entry| entry.reporter_id == reporter)
                .expect("registered reporter must retain its log state");
            assert_eq!(
                reporter_entry.failure_run.last_logged_milestone(),
                FailureLogMilestoneIndex::OneMinute
            );
            assert_eq!(
                reporter_entry.failure_run.most_recent_decisions(),
                crate::reporter_health::FailureLogDecisions {
                    error:     ReporterLogDecision::NoLog,
                    milestone: ReporterLogDecision::FailureMilestone(
                        FailureLogMilestoneIndex::OneMinute,
                    ),
                }
            );
            assert!(matches!(
                reporter_entry.health.first_complete_set(),
                FirstCompleteSetStatus::Waiting(WaitTiming::Overdue { .. })
            ));
        }

        update_until_completed_batches(&mut app, reporter, FAILURES_AFTER_RUN_START + 2);
        let reporters = app.world().resource::<Reporters>();
        let reporter_entry = reporters
            .entries
            .iter()
            .find(|entry| entry.reporter_id == reporter)
            .expect("registered reporter must retain its log state");
        assert_eq!(
            reporter_entry.failure_run.last_logged_milestone(),
            FailureLogMilestoneIndex::OneMinute
        );
        assert_eq!(
            reporter_entry.failure_run.most_recent_decisions(),
            crate::reporter_health::FailureLogDecisions::default()
        );
        assert!(matches!(
            reporter_entry.health.outcome(),
            ReporterOutcomeHealth::Failing { .. }
        ));
    }

    #[test]
    fn reenabled_reporter_emits_an_overdue_decision_for_the_restarted_bound() {
        let first_complete_set_bound = Duration::from_secs(10);
        let frame_past_bound = first_complete_set_bound + Duration::from_secs(1);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
            .add_plugins(RiggingPlugin);
        let reporter = app.add_device_reporter(
            SequenceReporter {
                scans: VecDeque::from([
                    DeviceScan::Deferred(ReporterDeferral::WaitingForTopology),
                    DeviceScan::Deferred(ReporterDeferral::WaitingForTopology),
                ]),
            },
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                ReporterCoverage::MatchingEvidenceOnly,
                first_complete_set_bound,
            ),
        );

        update_until_completed_batches(&mut app, reporter, 1);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(frame_past_bound));
        app.update();
        {
            let reporters = app.world().resource::<Reporters>();
            let reporter_entry = reporters
                .entries
                .iter()
                .find(|entry| entry.reporter_id == reporter)
                .expect("registered reporter must retain its log state");
            assert_eq!(
                reporter_entry.log_state.most_recent_overdue_decision(),
                ReporterLogDecision::FirstCompleteSetOverdue
            );
        }

        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .disable(reporter)
            .expect("the registered optional reporter must accept disable");
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        app.update();
        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .enable(reporter)
            .expect("the registered optional reporter must accept re-enable");
        app.update();
        {
            let reporters = app.world().resource::<Reporters>();
            let reporter_entry = reporters
                .entries
                .iter()
                .find(|entry| entry.reporter_id == reporter)
                .expect("registered reporter must retain its log state");
            assert_eq!(
                reporter_entry.log_state.most_recent_overdue_decision(),
                ReporterLogDecision::NoLog
            );
        }

        app.insert_resource(TimeUpdateStrategy::ManualDuration(frame_past_bound));
        app.update();
        let reporters = app.world().resource::<Reporters>();
        let reporter_entry = reporters
            .entries
            .iter()
            .find(|entry| entry.reporter_id == reporter)
            .expect("registered reporter must retain its log state");
        assert_eq!(
            reporter_entry.log_state.most_recent_overdue_decision(),
            ReporterLogDecision::FirstCompleteSetOverdue
        );
    }

    struct CountingReporter {
        scans: Arc<AtomicUsize>,
    }

    impl DeviceReporter for CountingReporter {
        fn discover(&mut self) -> DiscoveryWork {
            self.scans.fetch_add(1, Ordering::Relaxed);
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(|_| {
                DeviceScan::Complete(Vec::new())
            }))
        }
    }

    struct MeasuredImmediateReporter;

    impl DeviceReporter for MeasuredImmediateReporter {
        fn discover(&mut self) -> DiscoveryWork {
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| {
                std::thread::sleep(Duration::from_millis(1));
                DeviceScan::Complete(Vec::new())
            }))
        }
    }

    struct RecordReporter {
        scans: Arc<AtomicUsize>,
    }

    struct OrderedReporter {
        name:        &'static str,
        discoveries: Arc<Mutex<Vec<&'static str>>>,
    }

    impl DeviceReporter for OrderedReporter {
        fn discover(&mut self) -> DiscoveryWork {
            self.discoveries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(self.name);

            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(|_| {
                DeviceScan::Complete(Vec::new())
            }))
        }
    }

    struct SequenceReporter {
        scans: VecDeque<DeviceScan>,
    }

    struct BackgroundReporter {
        discoveries: Arc<AtomicUsize>,
    }

    impl DeviceReporter for BackgroundReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let discoveries = Arc::clone(&self.discoveries);
            DiscoveryWork::Background(DiscoveryJob::new(move |discovery_progress_sender| {
                discovery_progress_sender
                    .send(DiscoveryProgress::Measured {
                        completed: 1,
                        total:     NonZeroU32::MIN,
                    })
                    .expect("scheduler must retain a running job's progress receiver");
                discoveries.fetch_add(1, Ordering::Relaxed);

                DeviceScan::Complete(Vec::new())
            }))
        }
    }

    struct CountingBackgroundReporter {
        discoveries: Arc<AtomicUsize>,
    }

    impl DeviceReporter for CountingBackgroundReporter {
        fn discover(&mut self) -> DiscoveryWork {
            self.discoveries.fetch_add(1, Ordering::Relaxed);
            DiscoveryWork::Background(DiscoveryJob::new(|_| DeviceScan::Complete(Vec::new())))
        }
    }

    enum ProjectedBackgroundRun {
        Complete(&'static str),
        Failed,
    }

    struct ProjectedBackgroundReporter {
        runs: VecDeque<ProjectedBackgroundRun>,
    }

    impl DeviceReporter for ProjectedBackgroundReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let run = self
                .runs
                .pop_front()
                .expect("the projection test should queue every requested run");
            DiscoveryWork::Background(DiscoveryJob::new(move |_| match run {
                ProjectedBackgroundRun::Complete(marker) => DeviceScan::CompleteWithProjection {
                    devices:                      Vec::new(),
                    report_acceptance_projection: ReportAcceptanceProjection::new(move |world| {
                        world
                            .resource_mut::<PublishedReportProjections>()
                            .0
                            .push(marker);
                    }),
                },
                ProjectedBackgroundRun::Failed => {
                    DeviceScan::Failed(DeviceAccessError::Transport {
                        detail: String::from("projected background test failure"),
                    })
                },
            }))
        }
    }

    #[derive(Resource, Default)]
    struct PublishedReportProjections(Vec<&'static str>);

    struct BackgroundJobGate {
        reporter_name: &'static str,
        started:       Sender<&'static str>,
        released:      Arc<AtomicBool>,
        progress:      DiscoveryProgress,
    }

    struct GatedBackgroundReporter {
        background_job_gate: Arc<BackgroundJobGate>,
    }

    impl DeviceReporter for GatedBackgroundReporter {
        fn discover(&mut self) -> DiscoveryWork {
            gated_discovery_work(Arc::clone(&self.background_job_gate))
        }
    }

    #[derive(Clone, Copy)]
    enum FirstDiscoveryOutcome {
        Succeeded,
        Failed,
    }

    #[derive(Clone, Copy)]
    enum RequiredReporterRegistrationOrder {
        FailureBeforeIncomplete,
        IncompleteBeforeFailure,
    }

    #[derive(Clone, Copy)]
    enum ReporterActivityExpectation {
        Disabled,
        Idle,
        Queued,
        Running,
    }

    #[derive(Debug)]
    enum DeviceSetUnavailable {
        ReporterNotRegistered,
        AwaitingCompleteSet,
    }

    struct OutcomeThenGatedReporter {
        first_discovery_outcome: FirstDiscoveryOutcome,
        discoveries:             Arc<AtomicUsize>,
        runs:                    usize,
        background_job_gate:     Arc<BackgroundJobGate>,
    }

    impl DeviceReporter for OutcomeThenGatedReporter {
        fn discover(&mut self) -> DiscoveryWork {
            self.discoveries.fetch_add(1, Ordering::Relaxed);
            self.runs += 1;
            if self.runs == 1 {
                let device_scan = match self.first_discovery_outcome {
                    FirstDiscoveryOutcome::Succeeded => DeviceScan::Complete(Vec::new()),
                    FirstDiscoveryOutcome::Failed => {
                        DeviceScan::Failed(DeviceAccessError::Transport {
                            detail: String::from("test discovery failure"),
                        })
                    },
                };
                return DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| device_scan));
            }

            gated_discovery_work(Arc::clone(&self.background_job_gate))
        }
    }

    struct BackgroundJobRelease {
        released: Arc<AtomicBool>,
    }

    impl BackgroundJobRelease {
        fn release(self) { self.released.store(true, Ordering::Release); }
    }

    impl Drop for BackgroundJobRelease {
        fn drop(&mut self) { self.released.store(true, Ordering::Release); }
    }

    fn background_job_gate(
        reporter_name: &'static str,
        started: Sender<&'static str>,
        progress: DiscoveryProgress,
    ) -> (Arc<BackgroundJobGate>, BackgroundJobRelease) {
        let released = Arc::new(AtomicBool::new(false));
        (
            Arc::new(BackgroundJobGate {
                reporter_name,
                started,
                released: Arc::clone(&released),
                progress,
            }),
            BackgroundJobRelease { released },
        )
    }

    fn gated_discovery_work(background_job_gate: Arc<BackgroundJobGate>) -> DiscoveryWork {
        DiscoveryWork::Background(DiscoveryJob::new(move |discovery_progress_sender| {
            discovery_progress_sender
                .send(background_job_gate.progress.clone())
                .expect("scheduler must retain a running job's progress receiver");
            background_job_gate
                .started
                .send(background_job_gate.reporter_name)
                .expect("test must retain the job-start receiver");
            while !background_job_gate.released.load(Ordering::Acquire) {
                std::thread::yield_now();
            }

            DeviceScan::Complete(Vec::new())
        }))
    }

    fn wait_for_started_jobs(
        started: &Receiver<&'static str>,
        expected_count: usize,
    ) -> Vec<&'static str> {
        const JOB_START_TIMEOUT: Duration = Duration::from_secs(5);

        let mut reporter_names = (0..expected_count)
            .map(|_| {
                started
                    .recv_timeout(JOB_START_TIMEOUT)
                    .expect("admitted discovery job must reach its deterministic gate")
            })
            .collect::<Vec<_>>();
        reporter_names.sort_unstable();
        reporter_names
    }

    fn assert_no_additional_job_started(started: &Receiver<&'static str>) {
        assert_eq!(started.try_recv(), Err(TryRecvError::Empty));
    }

    fn release_jobs_after_start(
        started: Receiver<&'static str>,
        expected_count: usize,
        releases: Vec<BackgroundJobRelease>,
    ) -> JoinHandle<(Vec<&'static str>, Receiver<&'static str>)> {
        std::thread::spawn(move || {
            let reporter_names = wait_for_started_jobs(&started, expected_count);
            for release in releases {
                release.release();
            }
            (reporter_names, started)
        })
    }

    fn update_until_completed_batches(app: &mut App, reporter: ReporterId, completed_batches: u64) {
        const MAX_UPDATES: usize = 100;

        for _ in 0..MAX_UPDATES {
            app.update();
            let reporter_discovery_status = app
                .world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter)
                .expect("registered reporter must retain status");
            if reporter_discovery_status.completed_batches >= completed_batches {
                return;
            }
            std::thread::yield_now();
        }

        assert_eq!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter)
                .expect("registered reporter must retain status")
                .completed_batches,
            completed_batches
        );
    }

    fn reporter_result_becomes_ready(app: &mut App, reporter: ReporterId) -> bool {
        let deadline = Instant::now() + BACKGROUND_RESULT_DEADLINE;
        loop {
            let result_is_ready =
                app.world_mut()
                    .resource_scope::<Reporters, _>(|_, mut reporters| {
                        reporters.poll_running(Instant::now());
                        reporters
                            .entries
                            .iter()
                            .find(|reporter_entry| reporter_entry.reporter_id == reporter)
                            .is_some_and(|reporter_entry| {
                                matches!(
                                    reporter_entry.completion_time(),
                                    ReporterCompletionTime::CompletedAt(_)
                                )
                            })
                    });
            if result_is_ready {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(BACKGROUND_RESULT_POLL_INTERVAL);
        }
    }

    fn published_report_projections(app: &App) -> &[&'static str] {
        &app.world().resource::<PublishedReportProjections>().0
    }

    fn registered_reporter_status(
        app: &App,
        reporter: ReporterId,
    ) -> &crate::ReporterDiscoveryStatus {
        app.world()
            .resource::<DiscoveryStatus>()
            .reporter_status(reporter)
            .expect("registered reporter must retain status")
    }

    fn install_frame_anchored_runtime_clock(world: &mut World) {
        world.init_resource::<crate::Bindings>();
        world.init_resource::<Time<Real>>();
        let runtime_started_at = world.resource::<Time<Real>>().startup();
        world.insert_resource(RiggingRuntimeClock::starting_at(runtime_started_at));
    }

    fn request_reporter(app: &mut App, reporter: ReporterId) {
        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .request(reporter)
            .expect("the test reporter should accept an on-demand request");
    }

    fn assert_successful_reporter_status(
        app: &App,
        reporter: ReporterId,
        activity_expectation: ReporterActivityExpectation,
        completed_batches: u64,
    ) {
        let reporter_discovery_status = app
            .world()
            .resource::<DiscoveryStatus>()
            .reporter_status(reporter)
            .expect("registered reporter must retain status");
        assert!(matches!(
            reporter_discovery_status.last_outcome,
            LastDiscoveryOutcome::Succeeded { .. }
        ));
        assert_eq!(
            reporter_discovery_status.completed_batches,
            completed_batches
        );
        match activity_expectation {
            ReporterActivityExpectation::Disabled => assert!(matches!(
                reporter_discovery_status.activity,
                ReporterActivity::Disabled
            )),
            ReporterActivityExpectation::Idle => assert!(matches!(
                reporter_discovery_status.activity,
                ReporterActivity::Idle
            )),
            ReporterActivityExpectation::Queued => assert!(matches!(
                reporter_discovery_status.activity,
                ReporterActivity::Queued { .. }
            )),
            ReporterActivityExpectation::Running => assert!(matches!(
                reporter_discovery_status.activity,
                ReporterActivity::Running { .. }
            )),
        }
    }

    /// A journal these cases collect transitions into and then drop.
    ///
    /// The cases here assert on retained `DiscoveryStatus`, which is the query surface; the
    /// transitions are asserted end to end in `tests/scripted.rs`, through the events the journal
    /// produces. Sized by the same rule the scheduler uses so a case never silently loses a
    /// recording to a capacity a test invented.
    fn discarded_journal(
        reporters: &Reporters,
        discovery_limits: &DiscoveryLimits,
    ) -> (DiscoveryTransitionJournal, usize) {
        (
            DiscoveryTransitionJournal::default(),
            discovery::discovery_transition_capacity(reporters.entries.len(), discovery_limits),
        )
    }

    /// Put one reporter's run in the queued-with-a-finished-scan state acceptance reads.
    fn queue_completed_scan(
        reporter_entry: &mut ReporterEntry,
        batch: u64,
        accepted_at: Instant,
        measured_work_time: Duration,
    ) {
        reporter_entry.state = ReporterRunState::Queued {
            batch:   DiscoveryBatchId(batch),
            rerun:   RerunRequest::NotRequested,
            pending: PendingDiscovery::Completed {
                scan: DeviceScan::Complete(Vec::new()),
                accepted_at,
                measured_work_time,
            },
        };
    }

    fn admit_without_polling_running_jobs(app: &mut App) {
        app.world_mut()
            .resource_scope::<Reporters, _>(|world, mut reporters| {
                let now = Instant::now();
                let discovery_limits = world.resource::<DiscoveryLimits>().clone();
                let (mut journal, capacity) = discarded_journal(&reporters, &discovery_limits);
                world.resource_scope::<DiscoveryControl, _>(|world, discovery_control| {
                    world.resource_scope::<DiscoveryStatus, _>(|world, mut discovery_status| {
                        reporters.admit(
                            world,
                            now,
                            &discovery_control,
                            &discovery_limits,
                            &mut discovery_status,
                        );
                        reporters.refresh_activity(
                            now,
                            &discovery_limits,
                            &mut discovery_status,
                            &mut journal,
                            capacity,
                        );
                    });
                });
            });
    }

    fn accept_finished_job_without_polling_other_jobs(app: &mut App, reporter_index: usize) {
        app.world_mut()
            .resource_scope::<Reporters, _>(|world, mut reporters| {
                let now = Instant::now();
                reporters.entries[reporter_index].drain_progress();
                reporters.entries[reporter_index].collect_finished_run(now);
                let discovery_limits = world.resource::<DiscoveryLimits>().clone();
                let (mut journal, capacity) = discarded_journal(&reporters, &discovery_limits);
                world.resource_scope::<DiscoveryControl, _>(|world, discovery_control| {
                    world.resource_scope::<DiscoveryStatus, _>(|world, mut discovery_status| {
                        reporters.accept_completed(
                            world,
                            &discovery_control,
                            &discovery_limits,
                            &mut discovery_status,
                            &mut journal,
                            capacity,
                        );
                        reporters.refresh_startup(&mut discovery_status, &mut journal, capacity);
                        reporters.admit(
                            world,
                            now,
                            &discovery_control,
                            &discovery_limits,
                            &mut discovery_status,
                        );
                        reporters.refresh_activity(
                            now,
                            &discovery_limits,
                            &mut discovery_status,
                            &mut journal,
                            capacity,
                        );
                    });
                });
            });
    }

    fn refresh_progress_without_polling_running_jobs(app: &mut App) {
        app.world_mut()
            .resource_scope::<Reporters, _>(|world, mut reporters| {
                let now = Instant::now();
                for reporter_entry in &mut reporters.entries {
                    reporter_entry.drain_progress();
                }
                let discovery_limits = world.resource::<DiscoveryLimits>().clone();
                let (mut journal, capacity) = discarded_journal(&reporters, &discovery_limits);
                let mut discovery_status = world.resource_mut::<DiscoveryStatus>();
                reporters.refresh_activity(
                    now,
                    &discovery_limits,
                    &mut discovery_status,
                    &mut journal,
                    capacity,
                );
            });
    }

    fn available_device_set(
        reporter_device_set_state: ReporterDeviceSetState<'_>,
    ) -> Result<&DeviceSet, DeviceSetUnavailable> {
        match reporter_device_set_state {
            ReporterDeviceSetState::NotRegistered => {
                Err(DeviceSetUnavailable::ReporterNotRegistered)
            },
            ReporterDeviceSetState::AwaitingCompleteSet => {
                Err(DeviceSetUnavailable::AwaitingCompleteSet)
            },
            ReporterDeviceSetState::Available(device_set) => Ok(device_set),
        }
    }

    fn initialize_io_task_pool() {
        IoTaskPool::get_or_init(|| TaskPoolBuilder::new().num_threads(4).build());
    }

    fn add_failed_required_reporter(app: &mut App) -> ReporterId {
        app.add_device_reporter(
            SequenceReporter {
                scans: std::collections::VecDeque::from([DeviceScan::Failed(
                    DeviceAccessError::Transport {
                        detail: String::from("test required discovery failure"),
                    },
                )]),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        )
    }

    fn add_incomplete_required_reporter(
        app: &mut App,
        background_job_gate: Arc<BackgroundJobGate>,
    ) -> ReporterId {
        app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        )
    }

    fn assert_required_failure_outweighs_incomplete_reporter(
        registration_order: RequiredReporterRegistrationOrder,
    ) {
        initialize_io_task_pool();
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let (started_sender, started_receiver) = channel();
        let (background_job_gate, background_job_release) = background_job_gate(
            "incomplete",
            started_sender,
            DiscoveryProgress::Indeterminate,
        );
        let (failed, incomplete) = match registration_order {
            RequiredReporterRegistrationOrder::FailureBeforeIncomplete => {
                let failed = add_failed_required_reporter(&mut app);
                let incomplete = add_incomplete_required_reporter(&mut app, background_job_gate);
                (failed, incomplete)
            },
            RequiredReporterRegistrationOrder::IncompleteBeforeFailure => {
                let incomplete = add_incomplete_required_reporter(&mut app, background_job_gate);
                let failed = add_failed_required_reporter(&mut app);
                (failed, incomplete)
            },
        };

        app.update();
        assert_eq!(
            wait_for_started_jobs(&started_receiver, 1),
            vec!["incomplete"]
        );
        app.update();

        let discovery_status = app.world().resource::<DiscoveryStatus>();
        assert!(matches!(
            discovery_status
                .reporter_status(failed)
                .expect("registered reporter must retain status"),
            crate::ReporterDiscoveryStatus {
                last_outcome: LastDiscoveryOutcome::Failed { .. },
                completed_batches: 1,
                ..
            }
        ));
        assert!(matches!(
            discovery_status
                .reporter_status(incomplete)
                .expect("registered reporter must retain status"),
            crate::ReporterDiscoveryStatus {
                last_outcome: LastDiscoveryOutcome::NotCompleted,
                completed_batches: 0,
                ..
            }
        ));
        assert!(matches!(
            discovery_status.startup,
            StartupDiscoveryState::BlockedByFailure { reporter, .. } if reporter == failed
        ));

        background_job_release.release();
    }

    fn finish_background_discovery_task(reporter_entry: &mut ReporterEntry, now: Instant) {
        // The pooled task has to be scheduled before it can report completion, so the
        // wait is bounded by wall clock rather than by a poll count. A fixed number of
        // yields elapses in milliseconds on an idle machine and can run out before the
        // pool thread is scheduled at all on a host whose cores are already committed.
        let deadline = Instant::now() + BACKGROUND_RESULT_DEADLINE;

        loop {
            reporter_entry.collect_finished_run(now);
            if matches!(
                reporter_entry.completion_time(),
                ReporterCompletionTime::CompletedAt(_)
            ) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "background discovery task must reach completion within the wait budget"
            );
            std::thread::sleep(BACKGROUND_RESULT_POLL_INTERVAL);
        }
    }

    fn expire_reporter_deadline(app: &mut App, reporter: ReporterId) {
        let frame_instant = {
            let time = app.world().resource::<Time<Real>>();
            time.last_update().unwrap_or_else(|| time.startup())
        };
        let mut reporters = app.world_mut().resource_mut::<Reporters>();
        let reporter_entry = reporters
            .entries
            .iter_mut()
            .find(|reporter_entry| reporter_entry.reporter_id == reporter)
            .expect("registered reporter must retain its scheduler entry");
        reporter_entry.next_due = NextDue::At(frame_instant);
    }

    fn assert_expired_deadline_supplies_one_background_run(cadence: DiscoveryCadence) {
        initialize_io_task_pool();
        let discoveries = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let (started_sender, started_receiver) = channel();
        let (background_job_gate, background_job_release) =
            background_job_gate("subject", started_sender, DiscoveryProgress::Indeterminate);
        let reporter = app.add_device_reporter(
            OutcomeThenGatedReporter {
                first_discovery_outcome: FirstDiscoveryOutcome::Succeeded,
                discoveries: Arc::clone(&discoveries),
                runs: 0,
                background_job_gate,
            },
            ReporterRegistration::required(
                cadence,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();
        assert_eq!(discoveries.load(Ordering::Relaxed), 1);
        assert_eq!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter)
                .expect("registered reporter must retain status")
                .completed_batches,
            1
        );

        expire_reporter_deadline(&mut app, reporter);
        app.update();
        assert_eq!(wait_for_started_jobs(&started_receiver, 1), vec!["subject"]);
        assert_eq!(discoveries.load(Ordering::Relaxed), 2);

        for _ in 0..3 {
            app.update();
            assert!(matches!(
                app.world()
                    .resource::<DiscoveryStatus>()
                    .reporter_status(reporter)
                    .expect("registered reporter must retain status")
                    .activity,
                ReporterActivity::Running { .. }
            ));
        }
        assert_eq!(discoveries.load(Ordering::Relaxed), 2);

        background_job_release.release();
        update_until_completed_batches(&mut app, reporter, 2);
        for _ in 0..3 {
            app.update();
        }

        let reporter_discovery_status = app
            .world()
            .resource::<DiscoveryStatus>()
            .reporter_status(reporter)
            .expect("registered reporter must retain status");
        assert_eq!(discoveries.load(Ordering::Relaxed), 2);
        assert_eq!(reporter_discovery_status.completed_batches, 2);
        assert!(matches!(
            reporter_discovery_status.activity,
            ReporterActivity::Idle
        ));
        assert_no_additional_job_started(&started_receiver);
    }

    impl DeviceReporter for SequenceReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let device_scan = self
                .scans
                .pop_front()
                .unwrap_or(DeviceScan::Complete(Vec::new()));
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| device_scan))
        }
    }

    fn test_device_record() -> DeviceRecord {
        DeviceRecord {
            reported_as:            ReportedAs::MatchEvidenceOnly,
            parent:                 ReportedParent::Root,
            presence:               Presence::Present,
            claim:                  Claim::NotApplicable,
            capabilities:           Capabilities::new(),
            serial:                 ReportedSerial::NotExposedByUnit,
            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
            attachment:             AttachmentPath::PlatformHasNoConcept,
            descriptor:             DeviceDescriptor::PlatformReportedNothing,
        }
    }

    impl DeviceReporter for RecordReporter {
        fn discover(&mut self) -> DiscoveryWork {
            self.scans.fetch_add(1, Ordering::Relaxed);
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(|_| {
                DeviceScan::Complete(vec![test_device_record()])
            }))
        }
    }

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct TestConfiguration;

    struct TestDriver;

    impl EndpointDriver for TestDriver {
        type Configuration = TestConfiguration;
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
            _: &mut World,
            context: ApplyContext<'_, Self::Configuration>,
            _: &Self::Configuration,
            (): Self::Target,
        ) {
            context
                .into_completion()
                .finish(DriverCompletion::Succeeded(Applied::AsDispatched));
        }

        fn established(&mut self, _: &mut World, _: EstablishedContext<'_, Self::Configuration>) {}

        fn cancel_apply(
            &mut self,
            _: &mut World,
            _: &RoleKey,
            _: DriverCleanupRoleEntity,
            _: AttemptRef,
            _: AttemptInvalidation,
        ) {
        }

        fn release_session(
            &mut self,
            _: &mut World,
            _: &RoleKey,
            _: DriverCleanupRoleEntity,
            _: SessionRef,
            _: SessionReleaseCause,
        ) {
        }
    }

    struct FirstSchemePlugin(SchemeName);

    impl Plugin for FirstSchemePlugin {
        fn build(&self, app: &mut App) { app.register_device_scheme(self.0.clone()); }
    }

    struct SecondSchemePlugin(SchemeName);

    impl Plugin for SecondSchemePlugin {
        fn build(&self, app: &mut App) { app.register_device_scheme(self.0.clone()); }
    }

    #[test]
    fn deferred_and_unsupported_reporters_remain_in_batch_counts_while_a_sibling_runs() {
        initialize_io_task_pool();
        let now = Instant::now();
        let batch = DiscoveryBatchId(7);
        let deferred = ReporterId(0);
        let unsupported = ReporterId(1);
        let running = ReporterId(2);
        let mut reporters = Reporters::default();
        let mut discovery_status = DiscoveryStatus::default();

        for reporter in [deferred, unsupported] {
            let registration = ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            );
            discovery_status.register(reporter, &registration);
            reporters.entries.push(ReporterEntry::new(
                SequenceReporter {
                    scans: VecDeque::new(),
                },
                reporter,
                registration,
            ));
        }
        let running_registration = ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
            std::time::Duration::from_secs(10),
        );
        discovery_status.register(running, &running_registration);
        reporters.entries.push(ReporterEntry::new(
            CountingBackgroundReporter {
                discoveries: Arc::new(AtomicUsize::new(0)),
            },
            running,
            running_registration,
        ));

        let deferred_reporter_status = discovery_status
            .reporter_status_mut(deferred)
            .expect("registered reporter must retain status");
        deferred_reporter_status.last_outcome = LastDiscoveryOutcome::Deferred {
            batch,
            since: RiggingRuntimeTime::from_elapsed(Duration::ZERO),
            deferral: ReporterDeferral::WaitingForTopology,
        };
        deferred_reporter_status.completed_batches = 1;
        let unsupported_error = DeviceAccessError::Unsupported {
            detail: String::from("test platform has no reporter integration"),
        };
        let unsupported_reporter_status = discovery_status
            .reporter_status_mut(unsupported)
            .expect("registered reporter must retain status");
        unsupported_reporter_status.last_outcome = LastDiscoveryOutcome::Unsupported {
            batch,
            error: unsupported_error.clone(),
            since: RiggingRuntimeTime::from_elapsed(Duration::ZERO),
        };
        unsupported_reporter_status.completed_batches = 1;
        reporters.entries[1].state = ReporterRunState::Unsupported {
            error: unsupported_error,
            since: RiggingRuntimeTime::from_elapsed(Duration::ZERO),
        };
        reporters.entries[2].queue(batch, now);
        reporters.entries[2].prepare(&mut World::new());
        reporters.entries[2].start_prepared_background_with(|| now);

        let batch_counts = reporters.batch_counts(now, &discovery_status);
        assert_eq!(batch_counts.len(), 1);
        assert_eq!(batch_counts[0].batch, batch);
        assert_eq!(batch_counts[0].completed, 2);
        assert_eq!(batch_counts[0].total, 3);
        assert_eq!(batch_counts[0].running, 1);
        assert_eq!(batch_counts[0].queued, 0);
    }

    #[test]
    fn reporter_completion_types_name_absent_ready_and_accepted_transitions() {
        let started_at = Instant::now();
        let completed_at = started_at + Duration::from_secs(1);
        let mut reporter_entry = ReporterEntry::new(
            CountingReporter {
                scans: Arc::new(AtomicUsize::new(0)),
            },
            ReporterId(0),
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        assert!(matches!(
            reporter_entry.completion_time(),
            ReporterCompletionTime::NoCompletedResult
        ));
        assert!(matches!(
            reporter_entry.take_completed(ReporterActivation::Enabled),
            ReporterCompletionAcceptance::NoCompletedResult
        ));
        assert!(matches!(
            reporter_entry.state,
            ReporterRunState::Idle {
                rerun: RerunRequest::NotRequested,
            }
        ));

        reporter_entry.state = ReporterRunState::Queued {
            batch:   DiscoveryBatchId(7),
            rerun:   RerunRequest::Requested,
            pending: PendingDiscovery::Completed {
                scan:               DeviceScan::Complete(Vec::new()),
                accepted_at:        completed_at,
                measured_work_time: Duration::from_secs(1),
            },
        };
        assert_eq!(
            reporter_entry.completion_time(),
            ReporterCompletionTime::CompletedAt(completed_at)
        );

        let completion_acceptance = reporter_entry.take_completed(ReporterActivation::Enabled);
        assert!(matches!(
            &completion_acceptance,
            ReporterCompletionAcceptance::Accepted(_)
        ));
        let ReporterCompletionAcceptance::Accepted(completed_discovery) = completion_acceptance
        else {
            return;
        };
        assert_eq!(completed_discovery.batch, DiscoveryBatchId(7));
        assert_eq!(completed_discovery.accepted_at, completed_at);
        assert_eq!(
            completed_discovery.measured_work_time,
            Duration::from_secs(1)
        );
        assert!(matches!(
            completed_discovery.scan,
            DeviceScan::Complete(devices) if devices.is_empty()
        ));
        assert!(matches!(
            reporter_entry.completion_time(),
            ReporterCompletionTime::NoCompletedResult
        ));
        assert!(matches!(
            reporter_entry.state,
            ReporterRunState::Idle {
                rerun: RerunRequest::Requested,
            }
        ));
        assert!(matches!(
            reporter_entry.take_completed(ReporterActivation::Enabled),
            ReporterCompletionAcceptance::NoCompletedResult
        ));
    }

    #[test]
    fn immediate_scan_separates_frame_acceptance_from_measured_work_time() {
        let scheduled_at = Instant::now();
        let mut reporter_entry = ReporterEntry::new(
            MeasuredImmediateReporter,
            ReporterId(0),
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                Duration::from_secs(10),
            ),
        );
        reporter_entry.queue(DiscoveryBatchId(0), scheduled_at);

        reporter_entry.prepare_at(&mut World::new(), scheduled_at);

        assert!(matches!(
            &reporter_entry.state,
            ReporterRunState::Queued {
                pending: PendingDiscovery::Completed { accepted_at, .. },
                ..
            } if *accepted_at == scheduled_at
        ));
        assert!(matches!(
            &reporter_entry.state,
            ReporterRunState::Queued {
                pending: PendingDiscovery::Completed { measured_work_time, .. },
                ..
            } if !measured_work_time.is_zero()
        ));
    }

    #[test]
    fn periodic_reporter_runs_again_after_accepting_its_previous_whole_set() {
        let scans = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let _: ReporterId = app.add_device_reporter(
            CountingReporter {
                scans: Arc::clone(&scans),
            },
            ReporterRegistration::required(
                DiscoveryCadence::Periodic {
                    interval: Duration::ZERO,
                },
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        assert_eq!(scans.load(Ordering::Relaxed), 1);

        app.update();
        assert_eq!(scans.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn periodic_background_deadline_supplies_one_run_across_multiple_updates() {
        assert_expired_deadline_supplies_one_background_run(DiscoveryCadence::Periodic {
            interval: Duration::from_secs(30),
        });
    }

    #[test]
    fn event_driven_background_backstop_supplies_one_run_across_multiple_updates() {
        assert_expired_deadline_supplies_one_background_run(DiscoveryCadence::EventDriven {
            backstop: Duration::from_secs(30),
        });
    }

    #[test]
    fn periodic_deadline_uses_completion_without_catch_up_runs() {
        let scans = Arc::new(AtomicUsize::new(0));
        let interval = Duration::from_secs(10);
        let initial_due = Instant::now();
        let first_completed_at = initial_due + Duration::from_secs(35);
        let mut reporter_entry = ReporterEntry::new(
            CountingReporter {
                scans: Arc::clone(&scans),
            },
            ReporterId(0),
            ReporterRegistration::required(
                DiscoveryCadence::Periodic { interval },
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        assert!(reporter_entry.record_due_signal(
            initial_due,
            DiscoveryRequest::Requested,
            DiscoveryDirtyState::Clean,
        ));
        reporter_entry.queue(DiscoveryBatchId(0), initial_due);
        reporter_entry.prepare(&mut World::new());
        assert!(matches!(
            reporter_entry.take_completed(ReporterActivation::Enabled),
            ReporterCompletionAcceptance::Accepted(_)
        ));
        reporter_entry.schedule_after_completion(first_completed_at);
        assert!(matches!(
            reporter_entry.next_due,
            NextDue::At(deadline) if deadline == first_completed_at + interval
        ));
        assert!(!reporter_entry.record_due_signal(
            first_completed_at + interval.saturating_sub(Duration::from_nanos(1)),
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));

        let much_later = first_completed_at + interval * 4;
        assert!(reporter_entry.record_due_signal(
            much_later,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));
        reporter_entry.queue(DiscoveryBatchId(1), much_later);
        for _ in 0..3 {
            assert!(!reporter_entry.record_due_signal(
                much_later,
                DiscoveryRequest::NotRequested,
                DiscoveryDirtyState::Clean,
            ));
        }
        reporter_entry.prepare(&mut World::new());
        let second_completed_at = much_later + Duration::from_secs(1);
        assert!(matches!(
            reporter_entry.take_completed(ReporterActivation::Enabled),
            ReporterCompletionAcceptance::Accepted(_)
        ));
        reporter_entry.schedule_after_completion(second_completed_at);

        assert_eq!(scans.load(Ordering::Relaxed), 2);
        assert!(matches!(
            reporter_entry.next_due,
            NextDue::At(deadline) if deadline == second_completed_at + interval
        ));
        assert!(!reporter_entry.record_due_signal(
            second_completed_at,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));
    }

    #[test]
    fn event_driven_notifications_coalesce_and_backstop_runs_once() {
        let scans = Arc::new(AtomicUsize::new(0));
        let backstop = Duration::from_secs(30);
        let first_completed_at = Instant::now();
        let mut reporter_entry = ReporterEntry::new(
            CountingReporter {
                scans: Arc::clone(&scans),
            },
            ReporterId(0),
            ReporterRegistration::required(
                DiscoveryCadence::EventDriven { backstop },
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        reporter_entry.schedule_after_completion(first_completed_at);
        let dirty_at = first_completed_at + Duration::from_secs(1);
        assert!(reporter_entry.record_due_signal(
            dirty_at,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Dirty,
        ));
        reporter_entry.queue(DiscoveryBatchId(0), dirty_at);
        for _ in 0..3 {
            assert!(!reporter_entry.record_due_signal(
                dirty_at,
                DiscoveryRequest::NotRequested,
                DiscoveryDirtyState::Dirty,
            ));
        }
        reporter_entry.prepare(&mut World::new());
        assert!(matches!(
            reporter_entry.take_completed(ReporterActivation::Enabled),
            ReporterCompletionAcceptance::Accepted(_)
        ));
        reporter_entry.schedule_after_completion(dirty_at);
        assert_eq!(scans.load(Ordering::Relaxed), 1);

        assert!(!reporter_entry.record_due_signal(
            dirty_at + backstop.saturating_sub(Duration::from_nanos(1)),
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));
        let backstop_due = dirty_at + backstop;
        assert!(reporter_entry.record_due_signal(
            backstop_due,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));
        reporter_entry.queue(DiscoveryBatchId(1), backstop_due);
        for _ in 0..3 {
            assert!(!reporter_entry.record_due_signal(
                backstop_due + backstop,
                DiscoveryRequest::NotRequested,
                DiscoveryDirtyState::Clean,
            ));
        }
        reporter_entry.prepare(&mut World::new());

        assert_eq!(scans.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn two_due_reporters_prepare_whole_sets_in_one_update() {
        let first_scans = Arc::new(AtomicUsize::new(0));
        let second_scans = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        app.add_device_reporter(
            RecordReporter {
                scans: Arc::clone(&first_scans),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        app.add_device_reporter(
            CountingReporter {
                scans: Arc::clone(&second_scans),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();

        assert_eq!(first_scans.load(Ordering::Relaxed), 1);
        assert_eq!(second_scans.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn default_limit_holds_two_io_jobs_in_flight_and_queues_a_third() {
        initialize_io_task_pool();
        // The I/O pool is a process global: the first test to reach it fixes its thread count
        // for the whole binary, and a test that installs Bevy's task pool plugin without
        // calling `initialize_io_task_pool` first leaves that count sized from the host's
        // cores. The capacity asserted here is what the rest of this test rests on, and it
        // holds for any pool of three or more threads.
        assert_eq!(
            DiscoveryLimits::default()
                .effective_max_concurrent_jobs()
                .expect("test initializes the I/O task pool")
                .get(),
            2
        );
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let (started_sender, started_receiver) = channel();
        let (first_gate, first_release) = background_job_gate(
            "first",
            started_sender.clone(),
            DiscoveryProgress::Indeterminate,
        );
        let (second_gate, second_release) = background_job_gate(
            "second",
            started_sender.clone(),
            DiscoveryProgress::Indeterminate,
        );
        let (third_gate, third_release) =
            background_job_gate("third", started_sender, DiscoveryProgress::Indeterminate);
        let first = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: first_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let second = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: second_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let third = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: third_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        let release_jobs =
            release_jobs_after_start(started_receiver, 2, vec![first_release, second_release]);
        app.update();

        let (reporter_names, started_receiver) = release_jobs
            .join()
            .expect("gate coordinator must return observed reporter names");
        assert_eq!(reporter_names, vec!["first", "second"]);
        assert_no_additional_job_started(&started_receiver);
        let discovery_status = app.world().resource::<DiscoveryStatus>();
        assert!(matches!(
            discovery_status
                .reporter_status(first)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Running { .. }
        ));
        assert!(matches!(
            discovery_status
                .reporter_status(second)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Running { .. }
        ));
        assert!(matches!(
            discovery_status
                .reporter_status(third)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Queued {
                batch: DiscoveryBatchId(0),
            }
        ));

        third_release.release();
        update_until_completed_batches(&mut app, first, 1);
        update_until_completed_batches(&mut app, second, 1);
        update_until_completed_batches(&mut app, third, 1);
    }

    #[test]
    fn prepared_background_runtime_starts_when_capacity_admits_it() {
        initialize_io_task_pool();
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let progress_after = Duration::from_mins(1);
        {
            let mut discovery_limits = app.world_mut().resource_mut::<DiscoveryLimits>();
            discovery_limits.set_max_concurrent_jobs(NonZeroUsize::MIN);
            discovery_limits.set_progress_after(progress_after);
        }
        let (started_sender, started_receiver) = channel();
        let (blocker_gate, blocker_release) = background_job_gate(
            "blocker",
            started_sender.clone(),
            DiscoveryProgress::Indeterminate,
        );
        let (subject_gate, subject_release) =
            background_job_gate("subject", started_sender, DiscoveryProgress::Indeterminate);
        let blocker = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: blocker_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let subject = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: subject_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        assert_eq!(wait_for_started_jobs(&started_receiver, 1), vec!["blocker"]);
        assert_no_additional_job_started(&started_receiver);
        {
            let reporters = app.world().resource::<Reporters>();
            let subject_entry = reporters
                .entries
                .iter()
                .find(|reporter_entry| reporter_entry.reporter_id == subject)
                .expect("registered reporter must retain its scheduler entry");
            assert!(subject_entry.has_prepared_background());
        }

        app.world_mut()
            .resource_mut::<DiscoveryLimits>()
            .set_max_concurrent_jobs(NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN));
        admit_without_polling_running_jobs(&mut app);
        assert_eq!(wait_for_started_jobs(&started_receiver, 1), vec!["subject"]);

        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(subject)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Running { elapsed, .. } if elapsed < progress_after
        ));

        subject_release.release();
        blocker_release.release();
        update_until_completed_batches(&mut app, subject, 1);
        update_until_completed_batches(&mut app, blocker, 1);
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(subject)
                .expect("registered reporter must retain status")
                .last_outcome,
            LastDiscoveryOutcome::Succeeded { duration, .. } if duration < progress_after
        ));
    }

    #[test]
    fn raising_runtime_limit_admits_waiting_job_without_cancelling_running_job() {
        initialize_io_task_pool();
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        app.world_mut()
            .resource_mut::<DiscoveryLimits>()
            .set_max_concurrent_jobs(NonZeroUsize::MIN);
        let (started_sender, started_receiver) = channel();
        let (first_gate, first_release) = background_job_gate(
            "first",
            started_sender.clone(),
            DiscoveryProgress::Indeterminate,
        );
        let (second_gate, second_release) =
            background_job_gate("second", started_sender, DiscoveryProgress::Indeterminate);
        let first = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: first_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let second = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: second_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        let release_first_job = release_jobs_after_start(started_receiver, 1, vec![first_release]);
        app.update();
        let (reporter_names, started_receiver) = release_first_job
            .join()
            .expect("gate coordinator must return the first reporter name");
        assert_eq!(reporter_names, vec!["first"]);
        assert_no_additional_job_started(&started_receiver);
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(second)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Queued { .. }
        ));

        app.world_mut()
            .resource_mut::<DiscoveryLimits>()
            .set_max_concurrent_jobs(NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN));
        let release_second_job =
            release_jobs_after_start(started_receiver, 1, vec![second_release]);
        admit_without_polling_running_jobs(&mut app);

        let (reporter_names, _) = release_second_job
            .join()
            .expect("gate coordinator must return the second reporter name");
        assert_eq!(reporter_names, vec!["second"]);
        let discovery_status = app.world().resource::<DiscoveryStatus>();
        assert!(matches!(
            discovery_status
                .reporter_status(first)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Running { .. }
        ));
        assert!(matches!(
            discovery_status
                .reporter_status(second)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Running { .. }
        ));

        update_until_completed_batches(&mut app, first, 1);
        update_until_completed_batches(&mut app, second, 1);
    }

    #[test]
    fn lowering_runtime_limit_changes_later_admission_without_cancelling_running_jobs() {
        initialize_io_task_pool();
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let (started_sender, started_receiver) = channel();
        let (first_gate, first_release) = background_job_gate(
            "first",
            started_sender.clone(),
            DiscoveryProgress::Indeterminate,
        );
        let (second_gate, second_release) = background_job_gate(
            "second",
            started_sender.clone(),
            DiscoveryProgress::Indeterminate,
        );
        let (third_gate, third_release) =
            background_job_gate("third", started_sender, DiscoveryProgress::Indeterminate);
        let first = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: first_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let second = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: second_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let third = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: third_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        let release_jobs =
            release_jobs_after_start(started_receiver, 2, vec![first_release, second_release]);
        app.update();
        let (reporter_names, started_receiver) = release_jobs
            .join()
            .expect("gate coordinator must return observed reporter names");
        assert_eq!(reporter_names, vec!["first", "second"]);
        app.world_mut()
            .resource_mut::<DiscoveryLimits>()
            .set_max_concurrent_jobs(NonZeroUsize::MIN);
        {
            let discovery_status = app.world().resource::<DiscoveryStatus>();
            assert!(matches!(
                discovery_status
                    .reporter_status(first)
                    .expect("registered reporter must retain status")
                    .activity,
                ReporterActivity::Running { .. }
            ));
            assert!(matches!(
                discovery_status
                    .reporter_status(second)
                    .expect("registered reporter must retain status")
                    .activity,
                ReporterActivity::Running { .. }
            ));
        }

        accept_finished_job_without_polling_other_jobs(&mut app, 0);

        assert_no_additional_job_started(&started_receiver);
        let discovery_status = app.world().resource::<DiscoveryStatus>();
        assert!(matches!(
            discovery_status
                .reporter_status(second)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Running { .. }
        ));
        assert!(matches!(
            discovery_status
                .reporter_status(third)
                .expect("registered reporter must retain status")
                .activity,
            ReporterActivity::Queued { .. }
        ));

        third_release.release();
        update_until_completed_batches(&mut app, second, 1);
        update_until_completed_batches(&mut app, third, 1);
    }

    #[test]
    fn required_and_optional_reporters_receive_distinct_startup_batches() {
        let discoveries = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let required = app.add_device_reporter(
            OrderedReporter {
                name:        "required",
                discoveries: Arc::clone(&discoveries),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let optional = app.add_device_reporter(
            OrderedReporter {
                name:        "optional",
                discoveries: Arc::clone(&discoveries),
            },
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        assert_eq!(
            discoveries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_slice(),
            ["required"]
        );
        assert!(matches!(
            app.world().resource::<DiscoveryStatus>().startup,
            StartupDiscoveryState::Discovering
        ));

        app.update();
        assert_eq!(
            discoveries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_slice(),
            ["required", "optional"]
        );
        assert!(matches!(
            app.world().resource::<DiscoveryStatus>().startup,
            StartupDiscoveryState::Ready
        ));

        app.update();
        let discovery_status = app.world().resource::<DiscoveryStatus>();
        let required_outcome = &discovery_status
            .reporter_status(required)
            .expect("registered reporter must retain status")
            .last_outcome;
        let optional_outcome = &discovery_status
            .reporter_status(optional)
            .expect("registered reporter must retain status")
            .last_outcome;
        assert!(matches!(
            required_outcome,
            LastDiscoveryOutcome::Succeeded { .. }
        ));
        assert!(matches!(
            optional_outcome,
            LastDiscoveryOutcome::Succeeded { .. }
        ));
        let LastDiscoveryOutcome::Succeeded {
            batch: required_batch,
            ..
        } = required_outcome
        else {
            return;
        };
        let LastDiscoveryOutcome::Succeeded {
            batch: optional_batch,
            ..
        } = optional_outcome
        else {
            return;
        };
        assert_ne!(required_batch, optional_batch);
    }

    #[test]
    fn earlier_required_failure_outweighs_later_incomplete_reporter() {
        assert_required_failure_outweighs_incomplete_reporter(
            RequiredReporterRegistrationOrder::FailureBeforeIncomplete,
        );
    }

    #[test]
    fn later_required_failure_outweighs_earlier_incomplete_reporter() {
        assert_required_failure_outweighs_incomplete_reporter(
            RequiredReporterRegistrationOrder::IncompleteBeforeFailure,
        );
    }

    #[test]
    fn required_failure_retains_its_previous_set_and_blocks_readiness() {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let reporter_id = app.add_device_reporter(
            SequenceReporter {
                scans: std::collections::VecDeque::from([
                    DeviceScan::Complete(Vec::new()),
                    DeviceScan::Failed(DeviceAccessError::Transport {
                        detail: String::from("test transport failure"),
                    }),
                ]),
            },
            ReporterRegistration::required(
                DiscoveryCadence::Periodic {
                    interval: Duration::ZERO,
                },
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();
        app.update();

        let device_set = available_device_set(
            app.world()
                .resource::<Reporters>()
                .latest_device_set(reporter_id),
        )
        .expect("failed discovery must retain the preceding whole set");
        assert!(device_set.devices.is_empty());
        assert!(matches!(
            app.world().resource::<DiscoveryStatus>().startup,
            StartupDiscoveryState::BlockedByFailure { reporter, .. } if reporter == reporter_id
        ));
    }

    #[test]
    fn failure_health_remains_readable_after_reconciliation() {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let reporter_id = app.add_device_reporter(
            SequenceReporter {
                scans: std::collections::VecDeque::from([
                    DeviceScan::Complete(Vec::new()),
                    DeviceScan::Failed(DeviceAccessError::Transport {
                        detail: String::from("test reconciliation handoff"),
                    }),
                ]),
            },
            ReporterRegistration::required(
                DiscoveryCadence::Periodic {
                    interval: Duration::ZERO,
                },
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();
        app.update();

        // The completed set reached reconciliation and advanced the rigging revision. The later
        // failure retained that set and added no device work for a following frame.
        assert_eq!(app.world().resource::<RiggingRevision>().get(), 1);
        let mut reporters = app.world_mut().resource_mut::<Reporters>();
        assert_eq!(reporters.take_changed_reporters(), Vec::new());

        // The reporter's discovery status retains the failure's error directly.
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter_id)
                .map(|reporter_status| &reporter_status.last_outcome),
            Ok(LastDiscoveryOutcome::Failed {
                error: DeviceAccessError::Transport { .. },
                ..
            })
        ));
    }

    #[test]
    fn enabling_optional_reporter_requests_initial_discovery_without_authored_inventory() {
        let scans = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let reporter_id = app.add_device_reporter(
            CountingReporter {
                scans: Arc::clone(&scans),
            },
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Disabled,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        assert_eq!(scans.load(Ordering::Relaxed), 0);
        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .enable(reporter_id)
            .expect("registered optional reporter must enable");
        app.update();

        assert_eq!(scans.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn completion_budget_accepts_whole_reporter_sets_without_splitting_them() {
        let second_scans = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        app.world_mut()
            .resource_mut::<DiscoveryLimits>()
            .set_max_completions_per_frame(std::num::NonZeroUsize::MIN);
        let first = app.add_device_reporter(
            SequenceReporter {
                scans: std::collections::VecDeque::from([DeviceScan::Complete(vec![
                    test_device_record(),
                    test_device_record(),
                    test_device_record(),
                ])]),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let second = app.add_device_reporter(
            CountingReporter {
                scans: Arc::clone(&second_scans),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();

        let first_status = app
            .world()
            .resource::<DiscoveryStatus>()
            .reporter_status(first)
            .expect("registered reporter must retain status");
        let second_status = app
            .world()
            .resource::<DiscoveryStatus>()
            .reporter_status(second)
            .expect("registered reporter must retain status");
        assert!(matches!(
            first_status.last_outcome,
            LastDiscoveryOutcome::Succeeded { .. }
        ));
        assert!(matches!(
            second_status.last_outcome,
            LastDiscoveryOutcome::NotCompleted
        ));
        assert_eq!(second_scans.load(Ordering::Relaxed), 1);
        let first_device_set =
            available_device_set(app.world().resource::<Reporters>().latest_device_set(first))
                .expect("accepted complete set must remain available");
        assert_eq!(first_device_set.devices.len(), 3);

        app.update();
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(second)
                .expect("registered reporter must retain status")
                .last_outcome,
            LastDiscoveryOutcome::Succeeded { .. }
        ));
    }

    #[test]
    fn completion_budget_preserves_frame_acceptance_time_and_measured_duration() {
        let scans = Arc::new(AtomicUsize::new(0));
        let cadence_interval = Duration::from_secs(10);
        let first_registration = ReporterRegistration::required(
            DiscoveryCadence::OnDemand,
            ReporterCoverage::MatchingEvidenceOnly,
            std::time::Duration::from_secs(10),
        );
        let delayed_registration = ReporterRegistration::required(
            DiscoveryCadence::Periodic {
                interval: cadence_interval,
            },
            ReporterCoverage::MatchingEvidenceOnly,
            std::time::Duration::from_secs(10),
        );
        let mut reporters = Reporters::default();
        let first = reporters.add(
            CountingReporter {
                scans: Arc::clone(&scans),
            },
            first_registration.clone(),
        );
        let delayed = reporters.add(CountingReporter { scans }, delayed_registration.clone());
        let mut discovery_control = DiscoveryControl::default();
        discovery_control.register(first, &first_registration);
        discovery_control.register(delayed, &delayed_registration);
        let mut discovery_status = DiscoveryStatus::default();
        discovery_status.register(first, &first_registration);
        discovery_status.register(delayed, &delayed_registration);
        let mut discovery_limits = DiscoveryLimits::default();
        discovery_limits.set_max_completions_per_frame(NonZeroUsize::MIN);
        let mut world = World::new();
        install_frame_anchored_runtime_clock(&mut world);

        let first_result_accepted_at = Instant::now() + Duration::from_secs(1);
        let delayed_result_accepted_at = first_result_accepted_at + Duration::from_secs(1);
        let later_frame_at = delayed_result_accepted_at + Duration::from_secs(40);
        queue_completed_scan(
            &mut reporters.entries[0],
            0,
            first_result_accepted_at,
            Duration::from_secs(1),
        );
        queue_completed_scan(
            &mut reporters.entries[1],
            1,
            delayed_result_accepted_at,
            Duration::from_secs(2),
        );

        let (mut journal, capacity) = discarded_journal(&reporters, &discovery_limits);
        reporters.accept_completed(
            &mut world,
            &discovery_control,
            &discovery_limits,
            &mut discovery_status,
            &mut journal,
            capacity,
        );
        let first_status = discovery_status
            .reporter_status(first)
            .expect("registered reporter must retain status");
        let delayed_status = discovery_status
            .reporter_status(delayed)
            .expect("registered reporter must retain status");
        assert!(matches!(
            first_status.last_outcome,
            LastDiscoveryOutcome::Succeeded { .. }
        ));
        assert!(matches!(
            delayed_status.last_outcome,
            LastDiscoveryOutcome::NotCompleted
        ));

        reporters.accept_completed(
            &mut world,
            &discovery_control,
            &discovery_limits,
            &mut discovery_status,
            &mut journal,
            capacity,
        );

        let expected_duration = Duration::from_secs(2);
        let delayed_status = discovery_status
            .reporter_status(delayed)
            .expect("registered reporter must retain status");
        assert!(matches!(
            delayed_status.last_outcome,
            LastDiscoveryOutcome::Succeeded { duration, .. } if duration == expected_duration
        ));
        assert!(matches!(
            &reporters.entries[1].latest_set,
            RetainedDeviceSet::Complete { completed_at, .. }
                if *completed_at == delayed_result_accepted_at
        ));
        assert!(matches!(
            &reporters.entries[1].next_due,
            NextDue::At(deadline) if *deadline == delayed_result_accepted_at + cadence_interval
        ));
        assert!(reporters.entries[1].record_due_signal(
            later_frame_at,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));
    }

    #[test]
    fn later_co_reporter_completion_preserves_unchanged_retained_set() {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let first = app.add_device_reporter(
            SequenceReporter {
                scans: std::collections::VecDeque::from([DeviceScan::Complete(vec![
                    test_device_record(),
                    test_device_record(),
                ])]),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let second = app.add_device_reporter(
            SequenceReporter {
                scans: std::collections::VecDeque::from([DeviceScan::Complete(vec![
                    test_device_record(),
                ])]),
            },
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Disabled,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();
        {
            let reporters = app.world().resource::<Reporters>();
            let first_device_set = available_device_set(reporters.latest_device_set(first))
                .expect("first reporter's complete set must remain available");
            assert_eq!(first_device_set.devices.len(), 2);
            assert!(matches!(
                reporters.latest_device_set(second),
                ReporterDeviceSetState::AwaitingCompleteSet
            ));
        }

        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .enable(second)
            .expect("registered optional reporter must enable");
        app.update();
        app.update();

        let reporters = app.world().resource::<Reporters>();
        let first_device_set = available_device_set(reporters.latest_device_set(first))
            .expect("unchanged first reporter set must stay borrowable");
        assert_eq!(first_device_set.devices.len(), 2);
        let second_device_set = available_device_set(reporters.latest_device_set(second))
            .expect("second reporter's later completion must be retained");
        assert_eq!(second_device_set.devices.len(), 1);
    }

    #[test]
    fn background_discovery_runs_on_io_pool_and_updates_its_retained_outcome() {
        const MAX_UPDATES: usize = 50;

        initialize_io_task_pool();
        let discoveries = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let reporter_id = app.add_device_reporter(
            BackgroundReporter {
                discoveries: Arc::clone(&discoveries),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        for _ in 0..MAX_UPDATES {
            app.update();
            if matches!(
                app.world()
                    .resource::<DiscoveryStatus>()
                    .reporter_status(reporter_id)
                    .expect("registered reporter must retain status")
                    .last_outcome,
                LastDiscoveryOutcome::Succeeded { .. }
            ) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(discoveries.load(Ordering::Relaxed), 1);
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter_id)
                .expect("registered reporter must retain status")
                .last_outcome,
            LastDiscoveryOutcome::Succeeded { .. }
        ));
        let expected_capacity = IoTaskPool::get().thread_num().saturating_sub(1).clamp(1, 2);
        assert_eq!(
            DiscoveryLimits::default()
                .effective_max_concurrent_jobs()
                .expect("test initializes the I/O task pool")
                .get(),
            expected_capacity
        );
    }

    #[test]
    fn accepted_unchanged_report_publishes_projection_without_advancing_revision() {
        initialize_io_task_pool();
        let mut app = App::new();
        app.add_plugins(RiggingPlugin)
            .init_resource::<PublishedReportProjections>();
        let reporter = app.add_device_reporter(
            ProjectedBackgroundReporter {
                runs: VecDeque::from([
                    ProjectedBackgroundRun::Complete("owned"),
                    ProjectedBackgroundRun::Complete("unchanged"),
                    ProjectedBackgroundRun::Failed,
                ]),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        request_reporter(&mut app, reporter);

        assert!(
            reporter_result_becomes_ready(&mut app, reporter),
            "the owned background result should become ready"
        );
        assert!(
            published_report_projections(&app).is_empty(),
            "finishing producer work must not publish its acceptance projection"
        );
        assert_eq!(
            registered_reporter_status(&app, reporter).completed_batches,
            0
        );

        app.update();
        assert_eq!(published_report_projections(&app), ["owned"]);
        let revision_after_first_acceptance = *app.world().resource::<RiggingRevision>();
        let status = app
            .world()
            .resource::<DiscoveryStatus>()
            .reporter_status(reporter)
            .expect("the projected reporter should retain status");
        assert!(matches!(
            status,
            crate::ReporterDiscoveryStatus {
                activity: ReporterActivity::Running { batch, .. },
                last_outcome: LastDiscoveryOutcome::Succeeded {
                    batch: accepted_batch,
                    ..
                },
                completed_batches: 1,
            } if accepted_batch.get() == 0 && batch.get() == 1
        ));

        assert!(
            reporter_result_becomes_ready(&mut app, reporter),
            "the successor background result should become ready"
        );
        assert_eq!(
            published_report_projections(&app),
            ["owned"],
            "a completed successor must remain unpublished before its acceptance"
        );
        app.update();
        assert_eq!(
            published_report_projections(&app),
            ["owned", "unchanged"],
            "an accepted unchanged record set must publish its integration projection"
        );
        assert_eq!(
            *app.world().resource::<RiggingRevision>(),
            revision_after_first_acceptance,
            "an unchanged record set must not advance kernel revision"
        );
        app.update();
        assert_eq!(
            published_report_projections(&app),
            ["owned", "unchanged"],
            "accepted projections must be one-shot"
        );

        request_reporter(&mut app, reporter);
        app.update();
        assert!(
            reporter_result_becomes_ready(&mut app, reporter),
            "the failed background result should become ready"
        );
        assert_eq!(published_report_projections(&app), ["owned", "unchanged"]);
        app.update();

        let status = app
            .world()
            .resource::<DiscoveryStatus>()
            .reporter_status(reporter)
            .expect("the projected reporter should retain status");
        assert_eq!(status.completed_batches, 3);
        assert!(matches!(
            status.last_outcome,
            LastDiscoveryOutcome::Failed { batch, .. } if batch.get() == 2
        ));
        assert_eq!(
            published_report_projections(&app),
            ["owned", "unchanged"],
            "a failed scan must retain the preceding accepted integration state"
        );
    }

    #[test]
    fn prior_success_remains_visible_across_queued_running_idle_and_disabled_activity() {
        initialize_io_task_pool();
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        app.world_mut()
            .resource_mut::<DiscoveryLimits>()
            .set_max_concurrent_jobs(NonZeroUsize::MIN);
        let (started_sender, started_receiver) = channel();
        let (blocker_gate, blocker_release) = background_job_gate(
            "blocker",
            started_sender.clone(),
            DiscoveryProgress::Indeterminate,
        );
        let (subject_gate, subject_release) =
            background_job_gate("subject", started_sender, DiscoveryProgress::Indeterminate);
        let blocker = app.add_device_reporter(
            GatedBackgroundReporter {
                background_job_gate: blocker_gate,
            },
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Disabled,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let subject = app.add_device_reporter(
            OutcomeThenGatedReporter {
                first_discovery_outcome: FirstDiscoveryOutcome::Succeeded,
                discoveries:             Arc::new(AtomicUsize::new(0)),
                runs:                    0,
                background_job_gate:     subject_gate,
            },
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();
        assert_successful_reporter_status(&app, subject, ReporterActivityExpectation::Idle, 1);

        {
            let mut discovery_control = app.world_mut().resource_mut::<DiscoveryControl>();
            discovery_control
                .enable(blocker)
                .expect("registered optional reporter must enable");
            discovery_control
                .request(subject)
                .expect("registered reporter must accept a request");
        }
        let release_blocker = release_jobs_after_start(started_receiver, 1, vec![blocker_release]);
        app.update();
        let (reporter_names, started_receiver) = release_blocker
            .join()
            .expect("gate coordinator must return the blocker reporter name");
        assert_eq!(reporter_names, vec!["blocker"]);
        assert_successful_reporter_status(&app, subject, ReporterActivityExpectation::Queued, 1);

        let release_subject = release_jobs_after_start(started_receiver, 1, vec![subject_release]);
        update_until_completed_batches(&mut app, blocker, 1);
        let (reporter_names, _) = release_subject
            .join()
            .expect("gate coordinator must return the subject reporter name");
        assert_eq!(reporter_names, vec!["subject"]);
        assert_successful_reporter_status(&app, subject, ReporterActivityExpectation::Running, 1);

        update_until_completed_batches(&mut app, subject, 2);
        assert_successful_reporter_status(&app, subject, ReporterActivityExpectation::Idle, 2);

        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .disable(subject)
            .expect("registered optional reporter must disable");
        app.update();
        assert_successful_reporter_status(&app, subject, ReporterActivityExpectation::Disabled, 2);
    }

    #[test]
    fn running_status_retains_failure_identity_batch_elapsed_and_immediate_progress() {
        initialize_io_task_pool();
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let measured_progress = DiscoveryProgress::Measured {
            completed: 2,
            total:     NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
        };
        let (started_sender, started_receiver) = channel();
        let (background_job_gate, background_job_release) =
            background_job_gate("subject", started_sender, measured_progress.clone());
        let reporter = app.add_device_reporter(
            OutcomeThenGatedReporter {
                first_discovery_outcome: FirstDiscoveryOutcome::Failed,
                discoveries: Arc::new(AtomicUsize::new(0)),
                runs: 0,
                background_job_gate,
            },
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter)
                .expect("registered reporter must retain status"),
            crate::ReporterDiscoveryStatus {
                activity:          ReporterActivity::Idle,
                last_outcome:      LastDiscoveryOutcome::Failed { .. },
                completed_batches: 1,
            }
        ));

        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .request(reporter)
            .expect("registered reporter must accept a request");
        let release_job =
            release_jobs_after_start(started_receiver, 1, vec![background_job_release]);
        app.update();
        let progress_after = app.world().resource::<DiscoveryLimits>().progress_after();
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter)
                .expect("reporter lookup identity must select its running status"),
            crate::ReporterDiscoveryStatus {
                activity: ReporterActivity::Running {
                    batch,
                    elapsed,
                    progress: DiscoveryProgress::Indeterminate,
                },
                last_outcome: LastDiscoveryOutcome::Failed { .. },
                completed_batches: 1,
            } if batch.get() == 1 && *elapsed < progress_after
        ));

        let (reporter_names, _) = release_job
            .join()
            .expect("gate coordinator must return the subject reporter name");
        assert_eq!(reporter_names, vec!["subject"]);
        refresh_progress_without_polling_running_jobs(&mut app);
        assert!(matches!(
            app.world()
                .resource::<DiscoveryStatus>()
                .reporter_status(reporter)
                .expect("reporter lookup identity must select its running status"),
            crate::ReporterDiscoveryStatus {
                activity: ReporterActivity::Running {
                    batch,
                    progress,
                    ..
                },
                last_outcome: LastDiscoveryOutcome::Failed { .. },
                completed_batches: 1,
            } if batch.get() == 1 && progress == &measured_progress
        ));

        update_until_completed_batches(&mut app, reporter, 2);
    }

    #[test]
    fn triggers_while_background_reporter_runs_coalesce_into_one_rerun() {
        initialize_io_task_pool();
        let discoveries = Arc::new(AtomicUsize::new(0));
        let now = Instant::now();
        let mut reporter_entry = ReporterEntry::new(
            CountingBackgroundReporter {
                discoveries: Arc::clone(&discoveries),
            },
            ReporterId(0),
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        assert!(reporter_entry.record_due_signal(
            now,
            DiscoveryRequest::Requested,
            DiscoveryDirtyState::Clean,
        ));
        reporter_entry.queue(DiscoveryBatchId(0), now);
        reporter_entry.prepare(&mut World::new());
        reporter_entry.start_prepared_background_with(|| now);
        assert!(reporter_entry.is_running());
        assert_eq!(discoveries.load(Ordering::Relaxed), 1);

        assert!(!reporter_entry.record_due_signal(
            now,
            DiscoveryRequest::Requested,
            DiscoveryDirtyState::Clean,
        ));
        assert!(!reporter_entry.record_due_signal(
            now,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Dirty,
        ));
        reporter_entry.next_due = NextDue::At(now);
        assert!(!reporter_entry.record_due_signal(
            now,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));
        assert_eq!(discoveries.load(Ordering::Relaxed), 1);

        finish_background_discovery_task(&mut reporter_entry, now);
        assert!(matches!(
            reporter_entry.take_completed(ReporterActivation::Enabled),
            ReporterCompletionAcceptance::Accepted(_)
        ));
        reporter_entry.schedule_after_completion(now);
        assert!(reporter_entry.record_due_signal(
            now,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));
        reporter_entry.queue(DiscoveryBatchId(1), now);
        reporter_entry.prepare(&mut World::new());
        reporter_entry.start_prepared_background_with(|| now);
        assert!(reporter_entry.is_running());
        assert_eq!(discoveries.load(Ordering::Relaxed), 2);

        finish_background_discovery_task(&mut reporter_entry, now);
        assert!(matches!(
            reporter_entry.take_completed(ReporterActivation::Enabled),
            ReporterCompletionAcceptance::Accepted(_)
        ));
        reporter_entry.schedule_after_completion(now);
        for _ in 0..3 {
            assert!(!reporter_entry.record_due_signal(
                now,
                DiscoveryRequest::NotRequested,
                DiscoveryDirtyState::Clean,
            ));
        }
        assert_eq!(discoveries.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn new_control_signals_during_background_run_coalesce_into_one_rerun() {
        initialize_io_task_pool();
        let discoveries = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let (started_sender, started_receiver) = channel();
        let (background_job_gate, background_job_release) =
            background_job_gate("subject", started_sender, DiscoveryProgress::Indeterminate);
        let reporter = app.add_device_reporter(
            OutcomeThenGatedReporter {
                first_discovery_outcome: FirstDiscoveryOutcome::Succeeded,
                discoveries: Arc::clone(&discoveries),
                runs: 0,
                background_job_gate,
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();
        app.world_mut()
            .resource_mut::<DiscoveryControl>()
            .request(reporter)
            .expect("registered reporter must accept a request");
        app.update();
        assert_eq!(wait_for_started_jobs(&started_receiver, 1), vec!["subject"]);
        assert_eq!(discoveries.load(Ordering::Relaxed), 2);

        for _ in 0..3 {
            {
                let mut discovery_control = app.world_mut().resource_mut::<DiscoveryControl>();
                discovery_control
                    .request(reporter)
                    .expect("registered reporter must accept a request");
                discovery_control
                    .mark_dirty(reporter)
                    .expect("registered reporter must accept a dirty notification");
            }
            app.update();
        }
        assert_eq!(discoveries.load(Ordering::Relaxed), 2);

        background_job_release.release();
        update_until_completed_batches(&mut app, reporter, 2);
        assert_eq!(wait_for_started_jobs(&started_receiver, 1), vec!["subject"]);
        update_until_completed_batches(&mut app, reporter, 3);
        for _ in 0..3 {
            app.update();
        }

        let reporter_discovery_status = app
            .world()
            .resource::<DiscoveryStatus>()
            .reporter_status(reporter)
            .expect("registered reporter must retain status");
        assert_eq!(discoveries.load(Ordering::Relaxed), 3);
        assert_eq!(reporter_discovery_status.completed_batches, 3);
        assert!(matches!(
            reporter_discovery_status.activity,
            ReporterActivity::Idle
        ));
        assert_no_additional_job_started(&started_receiver);
    }

    #[test]
    fn disabling_running_optional_reporter_suppresses_its_recorded_rerun() {
        initialize_io_task_pool();
        let discoveries = Arc::new(AtomicUsize::new(0));
        let now = Instant::now();
        let mut reporter_entry = ReporterEntry::new(
            CountingBackgroundReporter {
                discoveries: Arc::clone(&discoveries),
            },
            ReporterId(0),
            ReporterRegistration::optional(
                DiscoveryCadence::OnDemand,
                ReporterActivation::Enabled,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        assert!(reporter_entry.record_due_signal(
            now,
            DiscoveryRequest::Requested,
            DiscoveryDirtyState::Clean,
        ));
        reporter_entry.queue(DiscoveryBatchId(0), now);
        reporter_entry.prepare(&mut World::new());
        reporter_entry.start_prepared_background_with(|| now);
        assert!(reporter_entry.is_running());
        assert_eq!(discoveries.load(Ordering::Relaxed), 1);

        assert!(!reporter_entry.record_due_signal(
            now,
            DiscoveryRequest::Requested,
            DiscoveryDirtyState::Dirty,
        ));
        finish_background_discovery_task(&mut reporter_entry, now);
        assert!(matches!(
            reporter_entry.take_completed(ReporterActivation::Disabled),
            ReporterCompletionAcceptance::Accepted(_)
        ));
        reporter_entry.schedule_after_completion(now);

        assert!(matches!(reporter_entry.state, ReporterRunState::Disabled));
        assert!(!reporter_entry.record_due_signal(
            now,
            DiscoveryRequest::NotRequested,
            DiscoveryDirtyState::Clean,
        ));
        assert_eq!(discoveries.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn registration_returns_distinct_reporter_and_driver_ids() {
        let scans = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();

        let first_reporter = app.add_device_reporter(
            CountingReporter {
                scans: Arc::clone(&scans),
            },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let second_reporter = app.add_device_reporter(
            CountingReporter { scans },
            ReporterRegistration::required(
                DiscoveryCadence::OnDemand,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );
        let first_driver = app.add_endpoint_driver(TestDriver);
        let second_driver = app.add_endpoint_driver(TestDriver);

        assert_ne!(first_reporter, second_reporter);
        assert_ne!(first_driver, second_driver);
        let reporters = app.world().resource::<Reporters>();
        assert!(matches!(
            reporters.latest_device_set(first_reporter),
            ReporterDeviceSetState::AwaitingCompleteSet
        ));
        assert!(matches!(
            reporters.latest_device_set(ReporterId(u32::MAX)),
            ReporterDeviceSetState::NotRegistered
        ));
    }

    #[test]
    fn completed_scans_are_retained_under_the_returned_reporter_id() {
        let scans = Arc::new(AtomicUsize::new(0));
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        let reporter_id = app.add_device_reporter(
            CountingReporter { scans },
            ReporterRegistration::required(
                DiscoveryCadence::Periodic {
                    interval: Duration::ZERO,
                },
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        );

        app.update();
        app.update();

        let device_set = available_device_set(
            app.world()
                .resource::<Reporters>()
                .latest_device_set(reporter_id),
        )
        .expect("accepted complete set must remain available");
        assert!(device_set.devices.is_empty());
    }

    #[test]
    fn reflection_cannot_construct_apply_permit() {
        let mut dynamic_permit = DynamicTupleStruct::default();
        dynamic_permit.insert(());

        assert!(ApplyPermit::from_reflect(&dynamic_permit).is_none());
    }

    #[test]
    fn reflection_cannot_construct_driver_id() {
        let mut dynamic_driver_id = DynamicTupleStruct::default();
        dynamic_driver_id.insert(0_u32);

        assert!(DriverId::from_reflect(&dynamic_driver_id).is_none());
    }

    #[test]
    fn scheme_registration_works_before_and_after_rigging_plugin() -> Result<(), Box<dyn Error>> {
        let scheme = SchemeName::new("edid-serial")?;
        let mut before = App::new();
        before.add_plugins((FirstSchemePlugin(scheme.clone()), RiggingPlugin));
        assert!(
            before
                .world()
                .resource::<RegisteredSchemes>()
                .contains(&scheme)
        );

        let mut after = App::new();
        after.add_plugins((RiggingPlugin, FirstSchemePlugin(scheme.clone())));
        assert!(
            after
                .world()
                .resource::<RegisteredSchemes>()
                .contains(&scheme)
        );

        Ok(())
    }

    #[test]
    fn duplicate_scheme_plugins_keep_one_registered_name() -> Result<(), Box<dyn Error>> {
        let scheme = SchemeName::new("edid-serial")?;
        let device_key = DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: scheme.clone(),
                value:  ReportedId::new("DELL-U2723QE-9J4K2H3")?,
            },
        };
        let mut first_then_second = App::new();
        first_then_second.add_plugins((
            RiggingPlugin,
            FirstSchemePlugin(scheme.clone()),
            SecondSchemePlugin(scheme.clone()),
        ));

        let first_registered_schemes = first_then_second.world().resource::<RegisteredSchemes>();
        assert!(first_registered_schemes.contains(&scheme));
        assert_eq!(first_registered_schemes.count(), 1);
        first_registered_schemes.validate(&device_key)?;

        let mut second_then_first = App::new();
        second_then_first.add_plugins((
            RiggingPlugin,
            SecondSchemePlugin(scheme.clone()),
            FirstSchemePlugin(scheme.clone()),
        ));

        let second_registered_schemes = second_then_first.world().resource::<RegisteredSchemes>();
        assert!(second_registered_schemes.contains(&scheme));
        assert_eq!(second_registered_schemes.count(), 1);
        second_registered_schemes.validate(&device_key)?;

        Ok(())
    }
}
