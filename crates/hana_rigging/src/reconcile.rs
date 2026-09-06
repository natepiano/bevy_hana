use std::any::TypeId;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::time::Duration;
use std::time::Instant;

use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::change_detection::DetectChangesMut;
use bevy::ecs::change_detection::Tick;
use bevy::ecs::component::Component;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::world::EntityWorldMut;
use bevy::log::warn;
use bevy::prelude::Entity;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::World;
use bevy::reflect::PartialReflect;
use bevy::reflect::Reflect;
use bevy::reflect::TypeRegistry;
use bevy::time::Real;
use bevy::time::Time;

use crate::AttachmentPath;
use crate::BatchRef;
use crate::Bindings;
use crate::CapabilityProjectionFailure;
use crate::Claim;
use crate::ConfiguredDeviceConnection;
use crate::ConfiguredDeviceMode;
use crate::Device;
use crate::DeviceId;
use crate::DeviceIdSource;
use crate::DeviceKey;
use crate::DeviceRecord;
use crate::DeviceRef;
use crate::DeviceResolution;
use crate::DeviceRevisionView;
use crate::DeviceStateLookup;
use crate::DeviceStatus;
use crate::Devices;
use crate::DiscoveryCadence;
use crate::DriverCleanupRoleEntity;
use crate::HardwareInventory;
use crate::IdentityDecisionOwed;
use crate::IdentityVerdict;
use crate::KeyAvailability;
use crate::LastKnownGoodConfiguration;
use crate::NonEmptyReporterRefs;
use crate::PlatformDeviceHandle;
use crate::Presence;
use crate::PresentEvidence;
use crate::ReconciledDeviceState;
use crate::RecoveryPolicy;
use crate::RegisteredSchemes;
use crate::ReportedAs;
use crate::ReportedHandleResolution;
use crate::ReportedId;
use crate::ReportedParent;
use crate::ReporterCoverage;
use crate::ReporterId;
use crate::ReporterRef;
use crate::ResolvedToDevice;
use crate::RetirementEvidence;
use crate::RiggingLimits;
use crate::RiggingRevision;
use crate::RiggingRuntimeClock;
use crate::RiggingRuntimeTime;
use crate::SchemeName;
use crate::SessionReleaseCause;
use crate::UnconfirmedBasis;
use crate::apply;
use crate::binding;
use crate::binding::AvailabilityWaitAction;
use crate::binding::AvailabilityWaitTarget;
use crate::binding::WaitingWork;
use crate::capabilities;
use crate::capabilities::CapabilityDeclaration;
use crate::capabilities::CapabilityProjectionDeclaration;
use crate::capabilities::CapabilitySourceState;
use crate::capabilities::ReporterCapabilityProjectionFailure;
use crate::capabilities::reflect_component_for;
use crate::devices;
use crate::devices::ConfiguredDeviceConnectionChange;
use crate::devices::DepartureGraceDeadlineStatus;
use crate::devices::DeviceAvailabilityChange;
use crate::devices::DeviceChangeAnnouncements;
use crate::devices::DeviceEntityLookup;
use crate::devices::DeviceRegisterChangeDetection;
use crate::devices::DeviceRevision;
use crate::devices::DeviceRevisionLookup;
use crate::devices::HandleOwner;
use crate::devices::KeyAvailabilityEvidence;
use crate::devices::PresentWithUsableClaim;
use crate::devices::PriorKeyAvailability;
use crate::devices::ReconcilePassConclusions;
use crate::devices::ReconciledDeviceChanges;
use crate::devices::ReconciledDeviceReplacement;
use crate::presence::DeviceSet;
use crate::registration::Drivers;
use crate::registration::RegisteredReporter;
use crate::registration::ReporterContribution;
use crate::registration::Reporters;
use crate::status::ContributorView;
use crate::status::NonEmptyContributors;
use crate::status::PresenceView;

/// Merge every contributing reporter's latest whole set into one device set, once per tick.
///
/// The system reads the frame's real-time clock, asks `reconcile_work` whether the frame has
/// anything to merge, and only then hands the resources to `reconcile_devices`.
///
/// The settled decision is taken here, from immutable borrows, rather than left to the
/// settled-frame return inside `reconcile_devices`: passing `ResMut<Devices>` on as `&mut Devices`
/// dereferences it mutably, and that alone marks the device set changed for every consumer
/// downstream, on a frame where nothing about it changed.
pub(crate) fn reconcile(
    mut reporters: ResMut<Reporters>,
    mut devices: ResMut<Devices>,
    mut rigging_revision: ResMut<RiggingRevision>,
    mut reconciled_device_changes: ResMut<ReconciledDeviceChanges>,
    bindings: Res<Bindings>,
    departure_grace_deadline_status: Res<DepartureGraceDeadlineStatus>,
    rigging_limits: Res<RiggingLimits>,
    registered_schemes: Res<RegisteredSchemes>,
    hardware_inventory: Res<HardwareInventory>,
    runtime_clock: Res<RiggingRuntimeClock>,
    time: Res<Time<Real>>,
) {
    let observed_at = time.last_update().unwrap_or_else(|| time.startup());
    let freshness_lease = FreshnessLease {
        rigging_limits: &rigging_limits,
        clock:          FrameClockReading::from(&*time),
    };
    let reconcile_work = reconcile_work(
        &reporters,
        &devices,
        &registered_schemes,
        &hardware_inventory,
        &bindings,
        freshness_lease,
        &rigging_limits,
        observed_at,
        *runtime_clock,
        departure_grace_deadline_status.requires_reconciliation(),
    );
    if matches!(&reconcile_work, ReconcileWork::Settled) {
        return;
    }

    if let ReconcilePass::Merged(replacement) = reconcile_devices(
        &mut reporters,
        devices.bypass_change_detection(),
        &mut rigging_revision,
        reconcile_work,
    ) {
        match replacement.device_register_change_detection {
            DeviceRegisterChangeDetection::Preserve => {},
            DeviceRegisterChangeDetection::MarkChanged => devices.set_changed(),
        }
        *reconciled_device_changes = replacement.changes;
    }
}

/// Whether a reconcile pass reached the merge, so a settled frame leaves the previous pass's
/// changes alone rather than replacing them with an empty record the projection would apply as
/// "nothing left".
enum ReconcilePass {
    /// Nothing had changed and no lease had expired, so the retained device set still stands.
    Settled,
    /// The retained sets were merged again; the result carries both register change detection and
    /// the differences the projection must apply.
    Merged(ReconciledDeviceReplacement),
}

/// The real-time reading the freshness lease measures reporter silence against.
///
/// Real time rather than the game clock, because hardware does not pause when the application
/// does: a paused app must not conclude an hour later that a monitor is still fresh.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FrameClockReading {
    /// The real-time clock has advanced at least once, so silence can be measured against it.
    Measurable(Instant),
    /// The real-time clock has not advanced past application startup, so no elapsed time exists to
    /// judge and no reporter is stale this frame.
    NotYetAdvanced,
}

impl From<&Time<Real>> for FrameClockReading {
    fn from(time: &Time<Real>) -> Self {
        time.last_update()
            .map_or(Self::NotYetAdvanced, Self::Measurable)
    }
}

/// Whether the freshness lease must withdraw reporter records in this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LeaseWork {
    /// Every reporter is fresh, or every expired reporter was withdrawn by an earlier pass.
    Settled,
    /// At least one reporter crossed its lease since the latest published reconciliation.
    WithdrawsReporterRecords,
}

/// How long each reporter may stay silent this frame, so freshness is answered one reporter at a
/// time instead of by collecting the silent ones.
///
/// Answering per reporter rather than building a list keeps the fast path free of allocation while
/// checking whether a reporter newly crossed its lease.
#[derive(Clone, Copy)]
struct FreshnessLease<'a> {
    rigging_limits: &'a RiggingLimits,
    clock:          FrameClockReading,
}

impl FreshnessLease<'_> {
    /// Report how much one reporter's records still count as evidence that its devices are there.
    ///
    /// A reporter that has not completed a first scan is not silent — there is no completion time
    /// to measure silence from — and a reporter that declared no cadence may stay quiet
    /// indefinitely without being late.
    fn freshness_of(&self, registered_reporter: &RegisteredReporter<'_>) -> ReporterFreshness {
        let FrameClockReading::Measurable(now) = self.clock else {
            return ReporterFreshness::Fresh;
        };
        let ReporterContribution::Completed {
            freshness_anchor, ..
        } = &registered_reporter.contribution
        else {
            return ReporterFreshness::Fresh;
        };
        let ReporterFreshnessLease::Expires(lease) =
            freshness_lease(registered_reporter.cadence, self.rigging_limits)
        else {
            return ReporterFreshness::Fresh;
        };

        let silence = now.saturating_duration_since(*freshness_anchor);
        if silence > lease {
            ReporterFreshness::SilentFor(silence)
        } else {
            ReporterFreshness::Fresh
        }
    }
}

/// Whether a reporter is still inside its freshness lease when its records reach the merge, and
/// how far past it when it is not.
#[derive(Clone, Copy)]
enum ReporterFreshness {
    /// The reporter is inside its lease, so its records read as it reported them.
    Fresh,
    /// The reporter has been quiet this long past its lease, so the kernel stops treating its
    /// records as evidence that the devices are still reachable.
    SilentFor(Duration),
}

/// How long a reporter may stay silent before the kernel stops counting its records as evidence
/// that its devices are reachable.
enum ReporterFreshnessLease {
    /// The reporter declared no cadence, so silence proves nothing: it runs when the application
    /// asks and can stay quiet indefinitely without being late.
    NoDeclaredCadence,
    /// The reporter declared a run within this interval, so exceeding it plus the configured grace
    /// means the reporter is wedged rather than idle.
    Expires(Duration),
}

/// How reachable one presence observation is, so a child can be folded against its parent.
///
/// The order is the whole conjunctive rule: reconciliation keeps whichever of the two is less
/// reachable, because a device is never more reachable than the device it hangs off.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Reachability {
    /// The unit is established as gone.
    Departed,
    /// Whether the unit remains available cannot be determined.
    Uncertain,
    /// The unit was observed and can be used.
    Reachable,
}

impl Reachability {
    const fn from_presence(presence: Presence) -> Self {
        match presence {
            Presence::Present => Self::Reachable,
            Presence::Unreachable { .. } => Self::Uncertain,
            Presence::Absent => Self::Departed,
        }
    }
}

/// What one device's contributors say about its reachability, kept apart by whether the reporter
/// that said so is still inside its freshness lease.
///
/// A reporter past its lease has withdrawn its evidence, not reported an absence, so its records
/// never lower a device that a fresh contributor still reports. Silence becomes the device's own
/// presence only once every contributor to it has gone quiet.
#[derive(Clone, Copy)]
enum CoReportedPresence {
    /// Every contributor merged so far is past its lease: the least reachable presence they last
    /// reported, and how long the most recent of them has been silent.
    WithdrawnBySilence {
        last_reported: Presence,
        silence:       Duration,
    },
    /// At least one contributor is inside its lease: the least reachable presence those fresh
    /// contributors report.
    StillReported(Presence),
}

impl CoReportedPresence {
    const fn from_contribution(freshness: ReporterFreshness, reported: Presence) -> Self {
        match freshness {
            ReporterFreshness::Fresh => Self::StillReported(reported),
            ReporterFreshness::SilentFor(silence) => Self::WithdrawnBySilence {
                last_reported: reported,
                silence,
            },
        }
    }

    /// Add one contributor's report to what the device's contributors say so far.
    ///
    /// Merging is idempotent, so the first record may seed the merged device and be merged again
    /// without counting twice.
    fn merge(self, contribution: Self) -> Self {
        match (self, contribution) {
            (Self::StillReported(first), Self::StillReported(second)) => {
                Self::StillReported(least_reachable(first, second))
            },
            (Self::StillReported(reported), Self::WithdrawnBySilence { .. })
            | (Self::WithdrawnBySilence { .. }, Self::StillReported(reported)) => {
                Self::StillReported(reported)
            },
            (
                Self::WithdrawnBySilence {
                    last_reported: first,
                    silence: first_silence,
                },
                Self::WithdrawnBySilence {
                    last_reported: second,
                    silence: second_silence,
                },
            ) => Self::WithdrawnBySilence {
                last_reported: least_reachable(first, second),
                silence:       first_silence.min(second_silence),
            },
        }
    }

    /// Report the presence the device carries once every contributor has been merged.
    const fn settled(self) -> Presence {
        match self {
            Self::StillReported(presence) => presence,
            Self::WithdrawnBySilence {
                last_reported,
                silence,
            } => least_reachable(last_reported, Presence::Unreachable { since: silence }),
        }
    }
}

/// Every record reported under one durable key, before the merge draws any conclusion from them.
struct MergedDevice<'a> {
    parent:       ReportedParent,
    attachment:   AttachmentPath,
    presence:     CoReportedPresence,
    claim:        Claim,
    contributors: Vec<ReporterId>,
    capabilities: CoReportView<'a>,
}

/// Capability declarations borrowed from every contributing reporter, grouped by component type.
///
/// A borrowed view rather than a built collection: `Box<dyn Reflect>` is not clonable in Bevy
/// 0.19, and consuming a declaration out of a retained set would destroy evidence a reporter that
/// did not re-scan this frame still needs. It points at data the reporter registry already holds,
/// so each site that needs one builds its own instead of storing it in a resource.
type CoReportView<'a> = HashMap<TypeId, Vec<&'a CapabilityDeclaration>>;

fn reconcile_devices(
    reporters: &mut Reporters,
    devices: &mut Devices,
    rigging_revision: &mut RiggingRevision,
    reconcile_work: ReconcileWork,
) -> ReconcilePass {
    let ReconcileWork::Merges(reconcile_publication) = reconcile_work else {
        return ReconcilePass::Settled;
    };
    let changed_reporters = reporters.take_changed_reporters();
    let reconciled_device_changes = reconcile_publication.publish(devices);

    if !changed_reporters.is_empty() {
        rigging_revision.advance();
    }

    ReconcilePass::Merged(reconciled_device_changes)
}

/// Whether this frame reaches the merge at all.
enum ReconcileWork {
    /// No reporter completed a scan and no lease has anything left to apply, so the retained
    /// device set already says what this frame would conclude.
    Settled,
    /// Either new evidence arrived or a lease expired, so the device set has to be rebuilt.
    Merges(Box<ReconcilePublication>),
}

/// One authoritative frame result ready to replace the retained device conclusions.
struct ReconcilePublication {
    reconciled:         Vec<ReconciledDeviceState>,
    conclusions:        ReconcilePassConclusions,
    connection_changes: Vec<ConfiguredDeviceConnectionChange>,
}

impl ReconcilePublication {
    fn publish(self, devices: &mut Devices) -> ReconciledDeviceReplacement {
        let mut replacement = devices.replace_reconciled(self.reconciled, self.conclusions);
        replacement.changes.connections = self.connection_changes;
        replacement
    }
}

/// Decide whether the frame has reconcile work, without consuming any of the evidence that says so.
///
/// Separate from `reconcile_devices` so the answer is available before `Devices` is borrowed
/// mutably, and shared with it so the settled frame is defined in exactly one place.
fn reconcile_work(
    reporters: &Reporters,
    devices: &Devices,
    registered_schemes: &RegisteredSchemes,
    hardware_inventory: &HardwareInventory,
    bindings: &Bindings,
    freshness_lease: FreshnessLease<'_>,
    rigging_limits: &RiggingLimits,
    observed_at: Instant,
    runtime_clock: RiggingRuntimeClock,
    departure_grace_deadline_reached: bool,
) -> ReconcileWork {
    if reporters.any_reporter_changed()
        || bindings.bound_device_keys().any(|key| {
            matches!(
                devices.key_availability(key),
                PriorKeyAvailability::NeverPublished
            )
        })
        || departure_grace_deadline_reached
        || lease_work(devices, reporters, freshness_lease) == LeaseWork::WithdrawsReporterRecords
    {
        return ReconcileWork::Merges(Box::new(calculate_reconcile_publication(
            reporters,
            devices,
            registered_schemes,
            hardware_inventory,
            bindings,
            freshness_lease,
            rigging_limits,
            observed_at,
            runtime_clock,
        )));
    }

    ReconcileWork::Settled
}

/// Report whether a reporter's newly expired lease must withdraw its retained records.
///
/// `Devices` retains which reporters the latest pass withdrew. A reporter that remains silent is
/// therefore reconciled once when it crosses the lease, while following frames remain settled.
fn lease_work(
    devices: &Devices,
    reporters: &Reporters,
    freshness_lease: FreshnessLease<'_>,
) -> LeaseWork {
    let every_reporter_fresh = reporters.registered_reporters().all(|registered_reporter| {
        matches!(
            freshness_lease.freshness_of(&registered_reporter),
            ReporterFreshness::Fresh
        )
    });
    if every_reporter_fresh {
        return LeaseWork::Settled;
    }

    if reporters.registered_reporters().any(|registered_reporter| {
        matches!(
            freshness_lease.freshness_of(&registered_reporter),
            ReporterFreshness::SilentFor(_)
        ) && !devices.reporter_records_are_withdrawn(registered_reporter.reporter)
    }) {
        LeaseWork::WithdrawsReporterRecords
    } else {
        LeaseWork::Settled
    }
}

/// Keep whichever of two observations claims less reachability.
const fn least_reachable(first: Presence, second: Presence) -> Presence {
    if (Reachability::from_presence(second) as u8) < (Reachability::from_presence(first) as u8) {
        second
    } else {
        first
    }
}

fn freshness_lease(
    cadence: &DiscoveryCadence,
    rigging_limits: &RiggingLimits,
) -> ReporterFreshnessLease {
    match cadence {
        DiscoveryCadence::OnDemand => ReporterFreshnessLease::NoDeclaredCadence,
        DiscoveryCadence::EventDriven { backstop } => {
            ReporterFreshnessLease::Expires(*backstop + rigging_limits.report_grace)
        },
        DiscoveryCadence::Periodic { interval } => {
            ReporterFreshnessLease::Expires(*interval + rigging_limits.report_grace)
        },
    }
}

/// One record that carried no key, held with what it would contribute to the keyed device its
/// platform handle names.
struct EvidenceOnlyReport<'a> {
    reporter:      ReporterId,
    handle:        &'a ReportedId,
    device_record: &'a DeviceRecord,
    presence:      CoReportedPresence,
}

/// One fresh retained completion as availability reconciliation reads it.
struct FreshRetainedReport<'a> {
    reporter:   ReporterId,
    batch:      BatchRef,
    coverage:   &'a ReporterCoverage,
    device_set: &'a DeviceSet,
}

fn fresh_retained_reports<'a>(
    reporters: &'a Reporters,
    freshness_lease: FreshnessLease<'_>,
) -> Vec<FreshRetainedReport<'a>> {
    reporters
        .registered_reporters()
        .filter_map(|registered_reporter| {
            if !matches!(
                freshness_lease.freshness_of(&registered_reporter),
                ReporterFreshness::Fresh
            ) {
                return None;
            }
            let ReporterContribution::Completed {
                batch, device_set, ..
            } = registered_reporter.contribution
            else {
                return None;
            };
            Some(FreshRetainedReport {
                reporter: registered_reporter.reporter,
                batch,
                coverage: registered_reporter.coverage,
                device_set,
            })
        })
        .collect()
}

fn key_availability_evidence(
    reporters: &Reporters,
    fresh_reports: &[FreshRetainedReport<'_>],
    key: &DeviceKey,
    keyed_by_handle: &HashMap<&ReportedId, HandleOwner>,
    freshness_lease: FreshnessLease<'_>,
    observed_at: Instant,
    runtime_clock: RiggingRuntimeClock,
) -> KeyAvailabilityEvidence {
    let mut contributors = Vec::new();
    let mut combined_presence = None;
    let mut confirmed_absence = Vec::new();
    let mut uncovered_absence = Vec::new();
    let mut unreachable_reporters = Vec::new();
    let mut unreachable_since = None;

    for report in fresh_reports {
        let reported_presence = report
            .device_set
            .devices
            .iter()
            .filter(|record| fresh_record_contributes_to_key(record, key, keyed_by_handle))
            .map(|record| record.presence)
            .reduce(least_reachable);
        let reporter_ref = ReporterRef::from_reporter_id(report.reporter);
        match reported_presence {
            Some(presence) => {
                combined_presence = Some(
                    combined_presence
                        .map_or(presence, |combined| least_reachable(combined, presence)),
                );
                contributors.push(ContributorView::new(
                    reporter_ref,
                    report.batch,
                    PresenceView::from(presence),
                ));
                match presence {
                    Presence::Present => {},
                    Presence::Absent if report.coverage.establishes_absence_for(key) => {
                        confirmed_absence.push(RetirementEvidence::new(reporter_ref, report.batch));
                    },
                    Presence::Absent => {
                        uncovered_absence.push(RetirementEvidence::new(reporter_ref, report.batch));
                    },
                    Presence::Unreachable { since } => {
                        unreachable_reporters.push(report.reporter);
                        retain_earliest_runtime_time(
                            &mut unreachable_since,
                            runtime_clock
                                .time_at(observed_at.checked_sub(since).unwrap_or(observed_at)),
                        );
                    },
                }
            },
            None if report.coverage.establishes_absence_for(key) => {
                confirmed_absence.push(RetirementEvidence::new(reporter_ref, report.batch));
            },
            None => {},
        }
    }

    if combined_presence == Some(Presence::Present)
        && let Ok(contributors) = NonEmptyContributors::from_contributors(contributors)
    {
        return KeyAvailabilityEvidence::Present(PresentEvidence::new(contributors));
    }

    if let Some(established_by) = confirmed_absence
        .into_iter()
        .min_by_key(|evidence| evidence.reporter.get())
    {
        return KeyAvailabilityEvidence::ConfirmedAbsent(established_by);
    }

    if combined_presence == Some(Presence::Absent)
        && let Some(evidence) = uncovered_absence
            .into_iter()
            .min_by_key(|evidence| evidence.reporter.get())
    {
        return KeyAvailabilityEvidence::Unconfirmed(UnconfirmedBasis::UncoveredAbsence {
            reporter: evidence.reporter,
            batch:    evidence.batch,
        });
    }

    if combined_presence.is_some_and(|presence| matches!(presence, Presence::Unreachable { .. }))
        && let Ok(reporters) =
            NonEmptyReporterRefs::from_reporter_ids(unreachable_reporters.iter().copied())
    {
        return KeyAvailabilityEvidence::Unreachable {
            since: unreachable_since.unwrap_or_else(|| runtime_clock.time_at(observed_at)),
            reporters,
        };
    }

    availability_when_fresh_reports_do_not_conclude(
        reporters,
        key,
        keyed_by_handle,
        freshness_lease,
        observed_at,
        runtime_clock,
    )
}

fn availability_when_fresh_reports_do_not_conclude(
    reporters: &Reporters,
    key: &DeviceKey,
    keyed_by_handle: &HashMap<&ReportedId, HandleOwner>,
    freshness_lease: FreshnessLease<'_>,
    observed_at: Instant,
    runtime_clock: RiggingRuntimeClock,
) -> KeyAvailabilityEvidence {
    let mut awaited_covering_reporters = Vec::new();
    let mut covering_reporter_completed = false;
    let mut expired_reporters = Vec::new();
    let mut unreachable_since = None;
    for registered_reporter in reporters.registered_reporters() {
        let covers_key = registered_reporter.coverage.establishes_absence_for(key);
        match registered_reporter.contribution {
            ReporterContribution::AwaitingFirstCompleteSet if covers_key => {
                awaited_covering_reporters.push(registered_reporter.reporter);
            },
            ReporterContribution::AwaitingFirstCompleteSet => {},
            ReporterContribution::Completed { device_set, .. } => {
                if covers_key {
                    covering_reporter_completed = true;
                }
                if let ReporterFreshness::SilentFor(silence) =
                    freshness_lease.freshness_of(&registered_reporter)
                {
                    let names_key = device_set.devices.iter().any(|record| {
                        fresh_record_contributes_to_key(record, key, keyed_by_handle)
                    });
                    if names_key || covers_key {
                        expired_reporters.push(registered_reporter.reporter);
                        retain_earliest_runtime_time(
                            &mut unreachable_since,
                            runtime_clock
                                .time_at(observed_at.checked_sub(silence).unwrap_or(observed_at)),
                        );
                    }
                }
            },
        }
    }

    if let Ok(reporters) = NonEmptyReporterRefs::from_reporter_ids(expired_reporters) {
        return KeyAvailabilityEvidence::Unreachable {
            since: unreachable_since.unwrap_or_else(|| runtime_clock.time_at(observed_at)),
            reporters,
        };
    }

    if !covering_reporter_completed
        && let Ok(reporters) = NonEmptyReporterRefs::from_reporter_ids(awaited_covering_reporters)
    {
        return KeyAvailabilityEvidence::AwaitingFirstReport(reporters);
    }

    KeyAvailabilityEvidence::Unconfirmed(UnconfirmedBasis::NoFreshEvidence)
}

fn fresh_record_contributes_to_key(
    device_record: &DeviceRecord,
    key: &DeviceKey,
    keyed_by_handle: &HashMap<&ReportedId, HandleOwner>,
) -> bool {
    match &device_record.reported_as {
        ReportedAs::Keyed(reported_key) => reported_key == key,
        ReportedAs::MatchEvidenceOnly => {
            let PlatformDeviceHandle::Reported(handle) = &device_record.platform_device_handle
            else {
                return false;
            };
            matches!(keyed_by_handle.get(handle), Some(HandleOwner::OneKey(owner)) if owner == key)
        },
    }
}

fn retain_earliest_runtime_time(
    retained: &mut Option<RiggingRuntimeTime>,
    candidate: RiggingRuntimeTime,
) {
    if retained.is_none_or(|current| candidate.elapsed() < current.elapsed()) {
        *retained = Some(candidate);
    }
}

/// Retained records combined by durable key before identity and availability are concluded.
struct RetainedReportMerge<'reports> {
    merged:               HashMap<DeviceKey, MergedDevice<'reports>>,
    ingest_order:         Vec<DeviceKey>,
    keyed_by_handle:      HashMap<&'reports ReportedId, HandleOwner>,
    duplicate_keys:       HashSet<DeviceKey>,
    unregistered_schemes: HashSet<SchemeName>,
}

fn merge_retained_records<'reports>(
    reporters: &'reports Reporters,
    registered_schemes: &RegisteredSchemes,
    freshness_lease: FreshnessLease<'_>,
) -> RetainedReportMerge<'reports> {
    let mut merged = HashMap::new();
    // First-seen order makes newly issued handles follow reporter order instead of hash order.
    let mut ingest_order = Vec::new();
    let mut keyed_by_handle = HashMap::new();
    let mut evidence_only = Vec::new();
    let mut duplicate_keys = HashSet::new();
    let mut unregistered_schemes = HashSet::new();

    for registered_reporter in reporters.registered_reporters() {
        let freshness = freshness_lease.freshness_of(&registered_reporter);
        let reporter = registered_reporter.reporter;
        let ReporterContribution::Completed { device_set, .. } = registered_reporter.contribution
        else {
            continue;
        };
        let reported_presence = |device_record: &DeviceRecord| {
            CoReportedPresence::from_contribution(freshness, device_record.presence)
        };

        for device_record in &device_set.devices {
            match &device_record.reported_as {
                ReportedAs::Keyed(key) => {
                    if let Err(unregistered_scheme) = registered_schemes.validate(key) {
                        unregistered_schemes.insert(unregistered_scheme.scheme().clone());
                        continue;
                    }
                    if matches!(freshness, ReporterFreshness::Fresh)
                        && let PlatformDeviceHandle::Reported(handle) =
                            &device_record.platform_device_handle
                    {
                        record_handle_owner(&mut keyed_by_handle, handle, key);
                    }
                    merge_keyed_record(
                        merged.entry(key.clone()).or_insert_with(|| {
                            ingest_order.push(key.clone());
                            MergedDevice {
                                parent:       device_record.parent.clone(),
                                attachment:   device_record.attachment.clone(),
                                presence:     reported_presence(device_record),
                                claim:        device_record.claim.clone(),
                                contributors: Vec::new(),
                                capabilities: HashMap::new(),
                            }
                        }),
                        reporter,
                        device_record,
                        reported_presence(device_record),
                        &mut duplicate_keys,
                        key,
                    );
                },
                ReportedAs::MatchEvidenceOnly => {
                    if let PlatformDeviceHandle::Reported(handle) =
                        &device_record.platform_device_handle
                    {
                        evidence_only.push(EvidenceOnlyReport {
                            reporter,
                            handle,
                            device_record,
                            presence: reported_presence(device_record),
                        });
                    }
                },
            }
        }
    }

    join_evidence_only_records(
        &mut merged,
        &keyed_by_handle,
        evidence_only,
        &mut duplicate_keys,
    );
    RetainedReportMerge {
        merged,
        ingest_order,
        keyed_by_handle,
        duplicate_keys,
        unregistered_schemes,
    }
}

fn join_evidence_only_records<'reports>(
    merged: &mut HashMap<DeviceKey, MergedDevice<'reports>>,
    keyed_by_handle: &HashMap<&'reports ReportedId, HandleOwner>,
    evidence_only: Vec<EvidenceOnlyReport<'reports>>,
    duplicate_keys: &mut HashSet<DeviceKey>,
) {
    for evidence_only_report in evidence_only {
        let Some(HandleOwner::OneKey(key)) = keyed_by_handle.get(evidence_only_report.handle)
        else {
            continue;
        };
        let Some(merged_device) = merged.get_mut(key) else {
            continue;
        };
        merge_keyed_record(
            merged_device,
            evidence_only_report.reporter,
            evidence_only_report.device_record,
            evidence_only_report.presence,
            duplicate_keys,
            key,
        );
    }
}

/// Calculate every conclusion one reconcile pass will publish.
fn calculate_reconcile_publication(
    reporters: &Reporters,
    devices: &Devices,
    registered_schemes: &RegisteredSchemes,
    hardware_inventory: &HardwareInventory,
    bindings: &Bindings,
    freshness_lease: FreshnessLease<'_>,
    rigging_limits: &RiggingLimits,
    observed_at: Instant,
    runtime_clock: RiggingRuntimeClock,
) -> ReconcilePublication {
    let RetainedReportMerge {
        merged,
        ingest_order,
        keyed_by_handle,
        duplicate_keys,
        unregistered_schemes,
    } = merge_retained_records(reporters, registered_schemes, freshness_lease);

    let departed_slots = departed_slots(devices, &merged);
    let identity_evidence = IdentityEvidence {
        duplicate_keys: &duplicate_keys,
        departed_slots: &departed_slots,
        hardware_inventory,
    };

    let reported = fold_presence_roots_first(&merged, ingest_order, devices, identity_evidence);
    let fresh_reports = fresh_retained_reports(reporters, freshness_lease);
    let (reconciled, availability) = KeyAvailabilityReconciliation {
        reporters,
        devices,
        hardware_inventory,
        bindings,
        fresh_reports: &fresh_reports,
        keyed_by_handle: &keyed_by_handle,
        freshness_lease,
        rigging_limits,
        observed_at,
        runtime_clock,
    }
    .reconcile(reported);

    let reported_handle_owners = keyed_by_handle
        .into_iter()
        .map(|(reported_id, handle_owner)| (reported_id.clone(), handle_owner))
        .collect();
    let withdrawn_reporters = reporters
        .registered_reporters()
        .filter(|registered_reporter| {
            matches!(
                freshness_lease.freshness_of(registered_reporter),
                ReporterFreshness::SilentFor(_)
            )
        })
        .map(|registered_reporter| registered_reporter.reporter)
        .collect();
    ReconcilePublication {
        reconciled,
        conclusions: ReconcilePassConclusions {
            availability,
            duplicate_keys,
            reported_handle_owners,
            unregistered_schemes,
            withdrawn_reporters,
        },
        connection_changes: configured_device_connection_changes(
            reporters,
            hardware_inventory,
            freshness_lease,
        ),
    }
}

/// Inputs shared while every key receives one availability conclusion.
struct KeyAvailabilityReconciliation<'reports, 'pass, 'limits> {
    reporters:          &'reports Reporters,
    devices:            &'pass Devices,
    hardware_inventory: &'pass HardwareInventory,
    bindings:           &'pass Bindings,
    fresh_reports:      &'pass [FreshRetainedReport<'reports>],
    keyed_by_handle:    &'pass HashMap<&'reports ReportedId, HandleOwner>,
    freshness_lease:    FreshnessLease<'limits>,
    rigging_limits:     &'limits RiggingLimits,
    observed_at:        Instant,
    runtime_clock:      RiggingRuntimeClock,
}

impl KeyAvailabilityReconciliation<'_, '_, '_> {
    fn reconcile(
        self,
        reported: Vec<ReconciledDeviceState>,
    ) -> (
        Vec<ReconciledDeviceState>,
        HashMap<DeviceKey, KeyAvailability>,
    ) {
        let mut key_order = Vec::new();
        let mut seen = HashSet::new();
        for state in &reported {
            if seen.insert(state.key.clone()) {
                key_order.push(state.key.clone());
            }
        }
        for key in self
            .devices
            .availability_keys()
            .chain(self.devices.states().map(|state| &state.key))
            .chain(self.hardware_inventory.configured_keys())
            .chain(self.bindings.bound_device_keys())
        {
            if seen.insert(key.clone()) {
                key_order.push(key.clone());
            }
        }

        let mut reported_by_key = reported
            .into_iter()
            .map(|state| (state.key.clone(), state))
            .collect::<HashMap<_, _>>();
        let mut reconciled = Vec::new();
        let mut availability = HashMap::with_capacity(key_order.len());

        for key in key_order {
            let evidence = key_availability_evidence(
                self.reporters,
                self.fresh_reports,
                &key,
                self.keyed_by_handle,
                self.freshness_lease,
                self.observed_at,
                self.runtime_clock,
            );
            let evidence = match reported_by_key.get(&key) {
                Some(reported_state) => carry_parent_uncertainty(
                    evidence,
                    reported_state,
                    self.observed_at,
                    self.runtime_clock,
                ),
                None => evidence,
            };
            let key_availability = devices::transition_key_availability(
                evidence,
                self.observed_at,
                self.runtime_clock,
                self.rigging_limits.departure_grace,
                self.devices.key_availability(&key),
            );

            if !matches!(key_availability, KeyAvailability::Absent { .. }) {
                if let Some(reconciled_device_state) = reported_by_key.remove(&key) {
                    reconciled.push(reconciled_device_state);
                } else if let DeviceResolution::Resolved(device_id) = self.devices.resolve(&key)
                    && let DeviceStateLookup::Retained(prior_state) = self.devices.state(device_id)
                {
                    reconciled.push(retain_device_state_for_availability(
                        prior_state.clone(),
                        &key_availability,
                        self.hardware_inventory,
                    ));
                }
            }
            availability.insert(key, key_availability);
        }

        (reconciled, availability)
    }
}

fn carry_parent_uncertainty(
    evidence: KeyAvailabilityEvidence,
    reported_state: &ReconciledDeviceState,
    observed_at: Instant,
    runtime_clock: RiggingRuntimeClock,
) -> KeyAvailabilityEvidence {
    if !matches!(evidence, KeyAvailabilityEvidence::Present(_))
        || matches!(reported_state.presence, Presence::Present)
    {
        return evidence;
    }
    let Ok(reporters) =
        NonEmptyReporterRefs::from_reporter_ids(reported_state.contributors.iter().copied())
    else {
        return evidence;
    };
    let since = match reported_state.presence {
        Presence::Unreachable { since } => {
            runtime_clock.time_at(observed_at.checked_sub(since).unwrap_or(observed_at))
        },
        Presence::Absent | Presence::Present => runtime_clock.time_at(observed_at),
    };
    KeyAvailabilityEvidence::Unreachable { since, reporters }
}

fn retain_device_state_for_availability(
    mut state: ReconciledDeviceState,
    availability: &KeyAvailability,
    hardware_inventory: &HardwareInventory,
) -> ReconciledDeviceState {
    state.mode = hardware_inventory
        .configured_device(&state.key)
        .map_or(ConfiguredDeviceMode::Managed, |configured_device| {
            configured_device.mode
        });
    state.presence = presence_from_availability(availability);
    state.contributors.clear();
    state.declared.clear();
    state.disputed.clear();
    state
}

const fn presence_from_availability(availability: &KeyAvailability) -> Presence {
    match availability {
        KeyAvailability::Present(_) => Presence::Present,
        KeyAvailability::DepartureGrace { .. } | KeyAvailability::Absent { .. } => Presence::Absent,
        KeyAvailability::AwaitingFirstReport { .. }
        | KeyAvailability::Unconfirmed { .. }
        | KeyAvailability::Unreachable { .. } => Presence::Unreachable {
            since: Duration::ZERO,
        },
    }
}

/// Reconcile every authored inventory key to what current reporter evidence says about it.
///
/// This runs whether or not the key produced a live device: an authored unit nothing reported is
/// exactly the case a walk over the merged set would miss, and its connection conclusion is the
/// only thing that tells an authoring interface the difference between *not looked for yet* and
/// *looked for and gone*.
///
/// Only reporters whose `ReporterCoverage` covers the key can conclude `Absent` from omitting it:
/// a camera-only reporter that never enumerates displays proves nothing by leaving one out. A
/// reporter past its freshness lease establishes nothing either — a set that aged out has withdrawn
/// its evidence rather than reported an absence, and a failed scan leaves the preceding set's
/// completion time where it was, so it ages out by the same measure.
///
/// The conclusion never enables a reporter and never authorizes an offline binding; it records
/// connectivity beside the authored operational mode, which stays untouched.
fn configured_device_connection_changes(
    reporters: &Reporters,
    hardware_inventory: &HardwareInventory,
    freshness_lease: FreshnessLease<'_>,
) -> Vec<ConfiguredDeviceConnectionChange> {
    hardware_inventory
        .configured_keys()
        .filter_map(|key| {
            let connection = configured_device_connection(reporters, key, freshness_lease);
            if hardware_inventory.connection(key) == Ok(connection) {
                return None;
            }
            Some(ConfiguredDeviceConnectionChange {
                key: key.clone(),
                connection,
            })
        })
        .collect()
}

/// Decide one authored key's connection conclusion from every reporter's retained evidence.
fn configured_device_connection(
    reporters: &Reporters,
    key: &DeviceKey,
    freshness_lease: FreshnessLease<'_>,
) -> ConfiguredDeviceConnection {
    let mut evidence = AuthoredKeyEvidence::default();

    for registered_reporter in reporters.registered_reporters() {
        let freshness = freshness_lease.freshness_of(&registered_reporter);
        let ReporterContribution::Completed { device_set, .. } = registered_reporter.contribution
        else {
            continue;
        };
        let names_key = device_set
            .devices
            .iter()
            .any(|device_record| device_record.reported_as == ReportedAs::Keyed(key.clone()));
        let establishes_absence = registered_reporter.coverage.establishes_absence_for(key);

        let strength = EvidenceStrength::from(freshness);
        if names_key {
            evidence.sighting = evidence.sighting.max(strength);
        } else if establishes_absence {
            evidence.authoritative_omission = evidence.authoritative_omission.max(strength);
        }
    }

    evidence.conclusion()
}

/// What every reporter's retained evidence adds up to for one authored key.
///
/// Collected before it is judged because the conclusion orders the two facts against each other
/// rather than answering per reporter: one reporter still inside its lease naming the key outranks
/// any number of authoritative omissions, and either fact inside its lease outranks either fact
/// that has expired.
#[derive(Default)]
struct AuthoredKeyEvidence {
    /// The strongest evidence any reporter offers that this key is currently there.
    sighting:               EvidenceStrength,
    /// The strongest omission by a reporter that enumerates this key's whole identity space.
    authoritative_omission: EvidenceStrength,
}

impl AuthoredKeyEvidence {
    const fn conclusion(&self) -> ConfiguredDeviceConnection {
        match (self.sighting, self.authoritative_omission) {
            (EvidenceStrength::Fresh, _) => ConfiguredDeviceConnection::Present,
            (_, EvidenceStrength::Fresh) => ConfiguredDeviceConnection::Absent,
            (EvidenceStrength::Expired, _) | (_, EvidenceStrength::Expired) => {
                ConfiguredDeviceConnection::Unreachable
            },
            (EvidenceStrength::None, EvidenceStrength::None) => {
                ConfiguredDeviceConnection::NotObserved
            },
        }
    }
}

/// How much one fact about an authored key is currently worth.
///
/// Ordered weakest to strongest so accumulating across reporters is a `max`: a second reporter can
/// only strengthen the accumulated fact, never retract another reporter's fresher evidence.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum EvidenceStrength {
    /// No reporter has offered this fact at all.
    #[default]
    None,
    /// A reporter offered this fact, but its retained set has aged past its freshness lease, so it
    /// describes what was true rather than what is.
    Expired,
    /// A reporter inside its freshness lease offers this fact about the current world.
    Fresh,
}

impl From<ReporterFreshness> for EvidenceStrength {
    fn from(reporter_freshness: ReporterFreshness) -> Self {
        match reporter_freshness {
            ReporterFreshness::Fresh => Self::Fresh,
            ReporterFreshness::SilentFor(_) => Self::Expired,
        }
    }
}

/// Record which key a reported platform handle belongs to, or that reporters disagree about it.
fn record_handle_owner<'a>(
    keyed_by_handle: &mut HashMap<&'a ReportedId, HandleOwner>,
    handle: &'a ReportedId,
    key: &DeviceKey,
) {
    match keyed_by_handle.entry(handle) {
        Entry::Vacant(vacant) => {
            vacant.insert(HandleOwner::OneKey(key.clone()));
        },
        Entry::Occupied(mut occupied) => {
            let newly_ambiguous = match occupied.get_mut() {
                HandleOwner::OneKey(owner) if owner != key => {
                    Some(HashSet::from([owner.clone(), key.clone()]))
                },
                HandleOwner::SeveralKeys(device_keys) => {
                    device_keys.insert(key.clone());
                    None
                },
                HandleOwner::OneKey(_) => None,
            };
            if let Some(device_keys) = newly_ambiguous {
                occupied.insert(HandleOwner::SeveralKeys(device_keys));
            }
        },
    }
}

/// Add one keyed record to the device its key names.
fn merge_keyed_record<'a>(
    merged_device: &mut MergedDevice<'a>,
    reporter: ReporterId,
    device_record: &'a DeviceRecord,
    reported_presence: CoReportedPresence,
    duplicate_keys: &mut HashSet<DeviceKey>,
    key: &DeviceKey,
) {
    if merged_device.contributors.contains(&reporter) {
        duplicate_keys.insert(key.clone());
    } else {
        merged_device.contributors.push(reporter);
    }

    merged_device.presence = merged_device.presence.merge(reported_presence);
    if claim_restriction(&device_record.claim) > claim_restriction(&merged_device.claim) {
        merged_device.claim = device_record.claim.clone();
    }
    if matches!(merged_device.parent, ReportedParent::Root) {
        merged_device.parent = device_record.parent.clone();
    }
    // A contributor that observed where the unit hangs outranks one that could not look: only a
    // reported attachment can place a unit in a slot, so it is adopted whenever it arrives.
    if !matches!(merged_device.attachment, AttachmentPath::Reported(_)) {
        merged_device.attachment = device_record.attachment.clone();
    }

    for capability in device_record.capabilities.declarations() {
        merged_device
            .capabilities
            .entry(capability.value().as_any().type_id())
            .or_default()
            .push(capability);
    }
}

/// How much a reported claim restricts this process, so co-reported claims merge to the most
/// restrictive report.
///
/// One reporter seeing an idle camera does not make it idle when another reporter watched a second
/// application open it; authorizing capture on the optimistic report would fail at the driver.
const fn claim_restriction(claim: &Claim) -> u8 {
    match claim {
        Claim::NotApplicable | Claim::Free => 0,
        Claim::Held => 1,
        Claim::Contended { .. } => 2,
        Claim::Blocked { .. } => 3,
    }
}

/// Order the merged devices roots first and fold each child's presence against its parent.
///
/// Reporters may list a child before the device it hangs off, so the order the records arrived in
/// does not put parents ahead of their children. Following the parent links first means a child is
/// never folded against a parent whose own presence has not been settled.
fn fold_presence_roots_first(
    merged: &HashMap<DeviceKey, MergedDevice<'_>>,
    ingest_order: Vec<DeviceKey>,
    devices: &Devices,
    identity_evidence: IdentityEvidence<'_>,
) -> Vec<ReconciledDeviceState> {
    let mut reconciled: Vec<ReconciledDeviceState> = Vec::with_capacity(merged.len());
    let mut folded: HashMap<DeviceKey, Presence> = HashMap::with_capacity(merged.len());
    let mut pending = ingest_order;

    while !pending.is_empty() {
        let mut deferred = Vec::new();
        let mut settled_any = false;

        for key in pending {
            let merged_device = &merged[&key];
            let parent_presence = match &merged_device.parent {
                ReportedParent::Root => None,
                ReportedParent::ChildOf(parent_key) => {
                    if !merged.contains_key(parent_key) {
                        // The whole set omits the device this one hangs off, so the kernel cannot
                        // see whether it can still be reached through it. Established departure is
                        // the reporter's to declare; the kernel only records uncertainty.
                        Some(Presence::Unreachable {
                            since: Duration::ZERO,
                        })
                    } else if let Some(parent_presence) = folded.get(parent_key) {
                        Some(*parent_presence)
                    } else {
                        deferred.push(key);
                        continue;
                    }
                },
            };

            let merged_presence = merged_device.presence.settled();
            let presence = parent_presence.map_or(merged_presence, |parent_presence| {
                if Reachability::from_presence(parent_presence)
                    < Reachability::from_presence(merged_presence)
                {
                    parent_presence
                } else {
                    merged_presence
                }
            });
            folded.insert(key.clone(), presence);
            let decided_identity = verdict_for(&key, merged_device, devices, identity_evidence);
            reconciled.push(ReconciledDeviceState {
                key: key.clone(),
                verdict: decided_identity.verdict,
                decision_owed: decided_identity.decision_owed,
                mode: identity_evidence.configured_mode(&key),
                attachment: merged_device.attachment.clone(),
                parent: merged_device.parent.clone(),
                presence,
                claim: merged_device.claim.clone(),
                contributors: merged_device.contributors.clone(),
                declared: merged_device.capabilities.keys().copied().collect(),
                disputed: disputed_capabilities(&merged_device.capabilities),
            });
            settled_any = true;
        }

        if !settled_any {
            // Every remaining device names a parent inside a cycle no reporter can resolve.
            // Settling them as uncertain keeps a malformed forest from stalling reconciliation.
            for key in deferred {
                let merged_device = &merged[&key];
                let decided_identity = verdict_for(&key, merged_device, devices, identity_evidence);
                reconciled.push(ReconciledDeviceState {
                    key:           key.clone(),
                    verdict:       decided_identity.verdict,
                    decision_owed: decided_identity.decision_owed,
                    mode:          identity_evidence.configured_mode(&key),
                    attachment:    merged_device.attachment.clone(),
                    parent:        merged_device.parent.clone(),
                    presence:      Presence::Unreachable {
                        since: Duration::ZERO,
                    },
                    claim:         merged_device.claim.clone(),
                    contributors:  merged_device.contributors.clone(),
                    declared:      merged_device.capabilities.keys().copied().collect(),
                    disputed:      disputed_capabilities(&merged_device.capabilities),
                });
            }
            break;
        }

        pending = deferred;
    }

    reconciled
}

/// The slot a unit that left occupied, kept so a unit arriving into it can be judged against it.
///
/// Only a reported attachment makes a slot: `AttachmentPath::PlatformHasNoConcept` and
/// `AttachmentPath::PlatformReportedNothing` compare equal to themselves, so joining on them would
/// fuse two units that each reported *no* location — the plausible-match fallback exact identity
/// exists to forbid.
struct DepartedSlot {
    saved:      DeviceKey,
    parent:     ReportedParent,
    attachment: ReportedId,
}

/// Everything outside one merged device that its verdict depends on.
///
/// Grouped rather than passed as three separate arguments because they travel together from the
/// merge through `fold_presence_roots_first` into `verdict_for`, and a reader of `verdict_for`
/// should see one word for "what the rest of this pass concluded".
#[derive(Clone, Copy)]
struct IdentityEvidence<'a> {
    duplicate_keys:     &'a HashSet<DeviceKey>,
    departed_slots:     &'a [DepartedSlot],
    hardware_inventory: &'a HardwareInventory,
}

impl IdentityEvidence<'_> {
    /// Report the authored operation mode for one key, treating an unauthored key as managed.
    ///
    /// A key nobody authored is not withheld: inventory records the application's decision to hold
    /// hardware back, and having made no decision is not that decision.
    fn configured_mode(&self, key: &DeviceKey) -> ConfiguredDeviceMode {
        self.hardware_inventory
            .configured_device(key)
            .map_or(ConfiguredDeviceMode::Managed, |configured_device| {
                configured_device.mode
            })
    }
}

/// Collect the slots the previous pass's devices held that this pass no longer names.
fn departed_slots(
    devices: &Devices,
    merged: &HashMap<DeviceKey, MergedDevice<'_>>,
) -> Vec<DepartedSlot> {
    devices
        .states()
        .filter(|reconciled_device_state| !merged.contains_key(&reconciled_device_state.key))
        .filter_map(|reconciled_device_state| {
            let AttachmentPath::Reported(attachment) = &reconciled_device_state.attachment else {
                return None;
            };
            Some(DepartedSlot {
                saved:      reconciled_device_state.key.clone(),
                parent:     reconciled_device_state.parent.clone(),
                attachment: attachment.clone(),
            })
        })
        .collect()
}

/// Decide what one merged device's durable key establishes about the live unit reported under it.
///
/// The verdict is produced here and never carried in from a reporter: a reporter asserting its own
/// identity conclusion would be making the claim the merge is the only thing able to check.
fn verdict_for(
    key: &DeviceKey,
    merged_device: &MergedDevice<'_>,
    devices: &Devices,
    identity_evidence: IdentityEvidence<'_>,
) -> DecidedIdentity {
    let resolution = devices.resolve(key);
    let decision_owed = decision_owed(devices, resolution);
    let scanned_verdict =
        IdentityVerdict::concluded_from_scan(key, identity_evidence.duplicate_keys);

    // The duplicate is an observation of this scan, so it is what the pass reports; the outstanding
    // decision travels alongside it and is reported again as soon as the scan is unique. Reporting
    // the outstanding verdict instead would hide a duplicate the scan is showing right now, and
    // storing the duplicate in its place is what let a transient duplicate erase the displacement.
    if identity_evidence.duplicate_keys.contains(key) {
        return DecidedIdentity {
            verdict: scanned_verdict,
            decision_owed,
        };
    }

    if let IdentityDecisionOwed::HumanDecision(outstanding_verdict) = &decision_owed {
        return DecidedIdentity {
            verdict: outstanding_verdict.clone(),
            decision_owed,
        };
    }

    // A key the previous pass already retained did not arrive into anything; only a unit that was
    // not here before can be sitting in the slot a departed one left.
    let arrived = resolution == DeviceResolution::NotResolved;
    if arrived
        && let AttachmentPath::Reported(attachment) = &merged_device.attachment
        && let Some(departed_slot) = identity_evidence
            .departed_slots
            .iter()
            .find(|departed_slot| {
                departed_slot.attachment == *attachment
                    && departed_slot.parent == merged_device.parent
                    && departed_slot.saved.kind == key.kind
            })
    {
        // The conflict belongs to the saved side of the join: an authored saved key names the unit
        // a human assigned to this slot, and the arriving unit reporting a different identity is
        // what makes the assignment wrong.
        let verdict = match departed_slot.saved.id {
            DeviceIdSource::Authored { .. } => IdentityVerdict::WrongUnit {
                authored: departed_slot.saved.clone(),
            },
            _ => IdentityVerdict::Displaced {
                saved: departed_slot.saved.clone(),
            },
        };

        return DecidedIdentity {
            decision_owed: IdentityDecisionOwed::HumanDecision(verdict.clone()),
            verdict,
        };
    }

    DecidedIdentity {
        verdict:       scanned_verdict,
        decision_owed: IdentityDecisionOwed::Nothing,
    }
}

/// What one pass concluded about a device's identity and what a human still owes it.
///
/// The two travel together because they are decided together and can differ: a pass reporting the
/// duplicate its scan is showing still carries the displacement verdict that outlives the scan.
struct DecidedIdentity {
    verdict:       IdentityVerdict,
    decision_owed: IdentityDecisionOwed,
}

/// Read the verdict a human still owes this device out of the state the previous pass retained.
///
/// `Displaced` and `WrongUnit` describe a join between an arriving unit and the slot a saved one
/// left. That evidence exists only on the pass the unit arrived: the next pass sees the key
/// retained and the departed slot gone, so recomputing from the current scan alone would return
/// `Proven` and quietly authorize the unit a human never accepted.
fn decision_owed(devices: &Devices, resolution: DeviceResolution) -> IdentityDecisionOwed {
    let DeviceResolution::Resolved(device_id) = resolution else {
        return IdentityDecisionOwed::Nothing;
    };
    let DeviceStateLookup::Retained(reconciled_device_state) = devices.state(device_id) else {
        return IdentityDecisionOwed::Nothing;
    };

    reconciled_device_state.decision_owed.clone()
}

/// Report which capability component types the contributors disagree about.
///
/// Equality calls the typed function stored with each erased declaration under one component type.
fn disputed_capabilities(capabilities: &CoReportView<'_>) -> HashSet<TypeId> {
    capabilities
        .iter()
        .filter(|(_, declarations)| {
            declarations.windows(2).any(|pair| {
                pair.first().is_some_and(|first| {
                    pair.get(1)
                        .is_some_and(|second| !first.equals(second.value()))
                })
            })
        })
        .map(|(type_id, _)| *type_id)
        .collect()
}

/// Mirror the reconciled device set onto entities and apply everything that follows from it.
///
/// This runs after `reconcile` rather than inside it: device entities cannot exist until the merge
/// has decided which devices there are, and the merge holds borrows of every reporter's retained
/// set for as long as it runs. It is exclusive because it spawns and despawns entities, reads the
/// reporter registry for capability values, and dispatches driver capture in one pass.
pub(crate) fn project_device_entities(world: &mut World) {
    let app_type_registry = world.get_resource::<AppTypeRegistry>().cloned();
    let type_registry = app_type_registry.as_ref().map(|registry| registry.read());
    let mut reconciled_device_changes =
        std::mem::take(&mut *world.resource_mut::<ReconciledDeviceChanges>());

    for orphaned_entity in reconciled_device_changes.orphaned_entities {
        if let Ok(entity) = world.get_entity_mut(orphaned_entity) {
            entity.despawn();
        }
    }
    if !reconciled_device_changes.connections.is_empty() {
        let mut hardware_inventory = world.resource_mut::<HardwareInventory>();
        for connection_change in &reconciled_device_changes.connections {
            // A key can leave the inventory between the merge and here; the conclusion is then
            // about a device nobody authored any more, and dropping it is the whole response.
            drop(
                hardware_inventory
                    .set_connection(&connection_change.key, connection_change.connection),
            );
        }
    }

    let newly_unavailable = reconciled_device_changes
        .availability
        .iter()
        .filter(|change| {
            matches!(change.from, KeyAvailability::Present(_))
                && !matches!(change.to, KeyAvailability::Present(_))
        })
        .cloned()
        .collect::<Vec<_>>();
    if !newly_unavailable.is_empty() {
        let now = world
            .resource::<Time<Real>>()
            .last_update()
            .unwrap_or_else(|| world.resource::<Time<Real>>().startup());
        apply_device_unavailability(world, &newly_unavailable, now);
    }
    if !reconciled_device_changes.availability.is_empty() {
        let now = world
            .resource::<Time<Real>>()
            .last_update()
            .unwrap_or_else(|| world.resource::<Time<Real>>().startup());
        project_role_availability_waits(world, &reconciled_device_changes.availability, now);
    }

    // Availability edges and connection changes move to the event stage before the projection
    // resource is cleared. Some `Absent` edges no longer have a device entity to read there.
    {
        let mut device_change_announcements = world.resource_mut::<DeviceChangeAnnouncements>();
        device_change_announcements
            .availability
            .append(&mut reconciled_device_changes.availability);
        device_change_announcements
            .connections
            .append(&mut reconciled_device_changes.connections);
    }

    let device_set_write = world.resource_scope::<Devices, _>(|world, mut devices| {
        let entered = devices.last_changed();
        let device_set_projection = world.resource_scope::<Reporters, _>(|world, mut reporters| {
            mirror_device_entities(
                world,
                &mut devices,
                &mut reporters,
                type_registry.as_deref(),
            )
        });
        resolve_binding_links(world, &devices);
        if let Some(type_registry) = type_registry.as_deref() {
            announce_disputes(
                &devices,
                &reconciled_device_changes.disputes_changed,
                type_registry,
            );
        }

        match device_set_projection {
            DeviceSetProjection::Projected => DeviceSetWrite::Written,
            DeviceSetProjection::Unwritten => DeviceSetWrite::Unwritten(entered),
        }
    });
    if let DeviceSetWrite::Unwritten(entered) = device_set_write {
        // `World::resource_scope` takes the resource out and puts it back under the current tick,
        // so borrowing the device set to read it announces a change to every consumer downstream.
        // Putting the tick the frame started with back is what keeps a frame that only read the
        // device set from reading as one that rewrote it.
        world.resource_mut::<Devices>().set_last_changed(entered);
    }
    if let Some(type_registry) = type_registry.as_deref() {
        mirror_last_known_good(world, type_registry);
    }
}

/// Revoke each role bound to a newly unavailable device and record what its recovery policy owes.
///
/// `RecoveryPolicy::ReapplyOnReturn` owes a restoration once a readback has established a value,
/// which is what makes that value return with the unit; with nothing established there is nothing
/// to restore and no work is recorded. The other two owe application work instead: without that
/// record a departed role falls back to `WaitingWork::Nothing`, reaches `WaitingRole::Hardware`,
/// and has its authored request dispatched automatically on the device's return — the automatic
/// reapply both asked the kernel not to perform.
///
/// Which work they owe differs, and the recorded value says which.
/// `RecoveryPolicy::ReapplyOnRequest` keeps its saved value, so `ReapplyConfiguration` sends it
/// back and clears the hold. `RecoveryPolicy::Forget` drops the value here, so no request can
/// restore it and only a fresh registration restarts the role.
///
/// Nothing else records it: a role that owes a restoration must not have its endpoint read back
/// first, because that would record the state the departure left behind as the value last known to
/// work.
///
/// This runs only on a transition from `Present`: later unavailable-to-unavailable edges update the
/// published wait without repeating session release or recovery bookkeeping.
fn apply_device_unavailability(
    world: &mut World,
    changes: &[DeviceAvailabilityChange],
    now: Instant,
) {
    for change in changes {
        let Ok(unavailable) = crate::UnavailableKeyAvailability::try_from(change.to.clone()) else {
            continue;
        };
        let roles = world
            .resource::<Bindings>()
            .roles_for(&change.key)
            .cloned()
            .collect::<Vec<_>>();

        for role in roles {
            let (recovery, established, session) = {
                let bindings = world.resource::<Bindings>();
                let Ok(binding) = bindings.binding(&role) else {
                    continue;
                };
                let session = bindings
                    .established_sessions()
                    .into_iter()
                    .find(|session| session.role == role);
                (
                    binding.recovery,
                    binding.last_known_good.is_established(),
                    session,
                )
            };
            if let Some(session) = session {
                let role_entity = world.resource::<Bindings>().role_entity(&role).ok();
                if let Some(role_entity) = role_entity {
                    let role_entity = DriverCleanupRoleEntity::checked(world, role_entity);
                    world.resource_scope::<Drivers, _>(|world, mut drivers| {
                        if let Err(error) = drivers.release_session(
                            world,
                            session.driver,
                            &role,
                            role_entity,
                            session.session,
                            SessionReleaseCause::DeviceUnavailable {
                                availability: unavailable.clone(),
                            },
                        ) {
                            warn!("role `{role}`: unavailable session release failed: {error}");
                        }
                    });
                } else {
                    warn!("role `{role}`: unavailable session has no live role entity");
                }
            }
            let mut bindings = world.resource_mut::<Bindings>();
            bindings.await_departed_device(&role, FrameClockReading::Measurable(now));
            match recovery {
                RecoveryPolicy::ReapplyOnReturn if established => {
                    bindings.set_waiting_work(&role, WaitingWork::RestorationOwed);
                },
                RecoveryPolicy::ReapplyOnReturn => {},
                RecoveryPolicy::ReapplyOnRequest => {
                    bindings.set_waiting_work(&role, WaitingWork::ReapplyRequestOwed);
                },
                RecoveryPolicy::Forget => {
                    bindings.forget_last_known_good(&role);
                    bindings.set_waiting_work(&role, WaitingWork::RegistrationOwed);
                },
            }
        }
    }
}

fn project_role_availability_waits(
    world: &mut World,
    changes: &[DeviceAvailabilityChange],
    now: Instant,
) {
    for change in changes {
        let roles = world
            .resource::<Bindings>()
            .roles_for(&change.key)
            .cloned()
            .collect::<Vec<_>>();
        for role in roles {
            let target = if matches!(change.to, KeyAvailability::Present(_)) {
                AvailabilityWaitTarget::Present
            } else {
                AvailabilityWaitTarget::Unavailable
            };
            let action = world
                .resource::<Bindings>()
                .availability_wait_action(&role, target);
            let waiting_condition = if target == AvailabilityWaitTarget::Present {
                binding::waiting_condition_for_work(
                    world.resource::<Bindings>().waiting_work(&role),
                )
            } else {
                apply::availability_wait(world, &change.key, &change.to, now)
            };
            match action {
                AvailabilityWaitAction::SetWait => {
                    world
                        .resource_mut::<Bindings>()
                        .set_wait(&role, now, waiting_condition);
                },
                AvailabilityWaitAction::StageForApplyingRole => {
                    world
                        .resource_mut::<Bindings>()
                        .stage_device_unavailability_wait(&role, now, waiting_condition);
                },
                AvailabilityWaitAction::PreserveLifecycle => {},
            }
        }
    }
}

/// Give every retained device an entity carrying the identity, reachability, and capability
/// components a query or the Bevy Remote Protocol reads.
///
/// The verdict, the presence, the claim, and the capability components a reporter declared are all
/// written only when they differ from what the entity already carries, so a settled reporter
/// rescanning on its own cadence produces no component change and a once-per-change consumer stays
/// quiet. `attach_declarations` holds the capability half of that guard, where it also keeps the
/// all-or-none attachment a mixed declaration needs.
fn mirror_device_entities(
    world: &mut World,
    devices: &mut Devices,
    reporters: &mut Reporters,
    type_registry: Option<&TypeRegistry>,
) -> DeviceSetProjection {
    let mut device_set_projection = DeviceSetProjection::Unwritten;
    let mut capability_projection_failures = Vec::new();
    let mirrored = collect_mirrored_devices(devices);

    for mirrored_device in mirrored {
        let entity = match devices.entity(mirrored_device.device_id) {
            DeviceEntityLookup::Projected(entity) if world.get_entity(entity).is_ok() => entity,
            _ => {
                let entity = world.spawn(Device).id();
                devices.project_entity(mirrored_device.device_id, entity);
                device_set_projection = DeviceSetProjection::Projected;

                entity
            },
        };
        // A disputed type has one value per contributor and the kernel adjudicates neither, so
        // attaching the union would write both in turn and make `Changed<C>` true on every frame
        // for the whole life of the disagreement.
        let (agreed, disputed) = partition_capability_declarations(
            reporters,
            devices,
            &mirrored_device.key,
            &mirrored_device.disputed,
        );
        let mut device_entity = world.entity_mut(entity);

        update_device_status(&mirrored_device, &mut device_entity);

        if !device_entity.contains::<DeviceId>() {
            device_entity.insert(mirrored_device.device_id);
        }
        if !device_entity.contains::<DeviceKey>() {
            device_entity.insert(mirrored_device.key.clone());
        }
        if device_entity.get::<IdentityVerdict>() != Some(&mirrored_device.verdict) {
            device_entity.insert(mirrored_device.verdict);
        }
        if device_entity
            .get::<Presence>()
            .is_none_or(|held| !held.is_same_variant(mirrored_device.presence))
        {
            device_entity.insert(mirrored_device.presence);
        }
        if device_entity.get::<Claim>() != Some(&mirrored_device.claim) {
            device_entity.insert(mirrored_device.claim.clone());
        }

        let usable = matches!(mirrored_device.availability, KeyAvailability::Present(_))
            && matches!(
                mirrored_device.claim,
                Claim::Held | Claim::Free | Claim::NotApplicable
            );
        if usable != device_entity.contains::<PresentWithUsableClaim>() {
            if usable {
                device_entity.insert(PresentWithUsableClaim);
            } else {
                device_entity.remove::<PresentWithUsableClaim>();
            }
        }

        if let Some(type_registry) = type_registry {
            capability_projection_failures.extend(attach_device_capabilities(
                &mut device_entity,
                type_registry,
                agreed,
                disputed,
                &mirrored_device.key,
            ));
        }
    }

    if type_registry.is_some() {
        publish_capability_projection_failures(world, reporters, capability_projection_failures);
    }

    device_set_projection
}

fn collect_mirrored_devices(devices: &Devices) -> Vec<MirroredDevice> {
    devices
        .states()
        .filter_map(|reconciled_device_state| {
            let DeviceResolution::Resolved(device_id) =
                devices.resolve(&reconciled_device_state.key)
            else {
                return None;
            };
            let DeviceRevisionLookup::Retained(revision) = devices.revision(device_id) else {
                return None;
            };
            Some(MirroredDevice {
                device_id,
                key: reconciled_device_state.key.clone(),
                revision,
                verdict: reconciled_device_state.verdict.clone(),
                presence: reconciled_device_state.presence,
                availability: match devices.key_availability(&reconciled_device_state.key) {
                    PriorKeyAvailability::Published(availability) => availability.clone(),
                    PriorKeyAvailability::NeverPublished => return None,
                },
                claim: reconciled_device_state.claim.clone(),
                disputed: reconciled_device_state.disputed.clone(),
            })
        })
        .collect()
}

fn publish_capability_projection_failures(
    world: &mut World,
    reporters: &mut Reporters,
    failures: Vec<ReporterCapabilityProjectionFailure>,
) {
    reporters.record_capability_projection_failures(
        world,
        failures
            .into_iter()
            .map(ReporterCapabilityProjectionFailure::into_parts),
    );
}

fn attach_device_capabilities(
    device_entity: &mut EntityWorldMut<'_>,
    type_registry: &TypeRegistry,
    agreed: Vec<CapabilityProjectionDeclaration<'_>>,
    disputed: Vec<CapabilityProjectionDeclaration<'_>>,
    key: &DeviceKey,
) -> Vec<ReporterCapabilityProjectionFailure> {
    match capabilities::project_declarations(device_entity, type_registry, &agreed, &disputed) {
        Ok(()) => Vec::new(),
        Err(failures) => {
            for failure in &failures {
                let type_path = match failure.failure() {
                    CapabilityProjectionFailure::ApplicationTypeRegistryUnavailable {
                        affected_type_path,
                    } => affected_type_path,
                    CapabilityProjectionFailure::ReflectComponentNotRegistered { type_path } => {
                        type_path
                    },
                };
                warn!(
                    "device `{key:?}` has capability `{type_path}` from reporter {:?} but its \
                     reflected component metadata is unavailable during projection",
                    failure.reporter()
                );
            }
            failures
        },
    }
}

fn partition_capability_declarations<'a>(
    reporters: &'a Reporters,
    devices: &Devices,
    key: &DeviceKey,
    disputed_types: &HashSet<TypeId>,
) -> (
    Vec<CapabilityProjectionDeclaration<'a>>,
    Vec<CapabilityProjectionDeclaration<'a>>,
) {
    capability_declarations(reporters, devices, key)
        .into_iter()
        .partition(|contribution| {
            !disputed_types.contains(&contribution.declaration().value().as_any().type_id())
        })
}

fn update_device_status(mirrored_device: &MirroredDevice, device_entity: &mut EntityWorldMut<'_>) {
    let status = projected_device_status(mirrored_device);
    if device_entity.get::<DeviceStatus>() != Some(&status) {
        device_entity.insert(status);
    }
}

fn projected_device_status(mirrored_device: &MirroredDevice) -> DeviceStatus {
    DeviceStatus::new(
        mirrored_device.key.clone(),
        DeviceRef::from_device_id(mirrored_device.device_id),
        DeviceRevisionView::from_revision(mirrored_device.revision),
        mirrored_device.availability.clone(),
    )
}

/// What the projection pass leaves behind on the device set's change tick.
enum DeviceSetWrite {
    /// The pass only read the device set; this is the tick it carried before the pass borrowed it.
    Unwritten(Tick),
    /// The pass recorded a projection, so the tick the scope reinserted under is the true one.
    Written,
}

/// Whether the projection recorded a new device entity in `Devices` itself.
///
/// Reported rather than read back off the resource's change tick: the pass takes `&mut Devices` to
/// reach `Devices::project_entity`, and the mutable dereference marks the resource changed whether
/// or not a projection followed it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DeviceSetProjection {
    /// Every retained device already had a live entity, so the device set itself was only read.
    Unwritten,
    /// At least one device was given an entity, which `Devices` now holds.
    Projected,
}

/// The conclusions one device's entity carries, copied out of the registry so the projection can
/// spawn and write while the registry stays borrowable.
struct MirroredDevice {
    device_id:    DeviceId,
    key:          DeviceKey,
    revision:     DeviceRevision,
    verdict:      IdentityVerdict,
    presence:     Presence,
    availability: KeyAvailability,
    claim:        Claim,
    disputed:     HashSet<TypeId>,
}

/// Borrow every capability declaration the contributing reporters retain for one durable key.
///
/// Read from the reporter registry rather than from `ReconciledDeviceState`, which keeps the
/// declared and disputed type identifiers but not the values: `Box<dyn Reflect>` is neither
/// clonable nor reflectable, so a copy kept beside the reconciled state could drift from what its
/// reporter holds.
fn capability_declarations<'a>(
    reporters: &'a Reporters,
    devices: &Devices,
    key: &DeviceKey,
) -> Vec<CapabilityProjectionDeclaration<'a>> {
    contributing_records(reporters, devices, key)
        .into_iter()
        .flat_map(|contribution| {
            contribution
                .record
                .capabilities
                .declarations()
                .map(move |declaration| {
                    CapabilityProjectionDeclaration::new(declaration, contribution.reporter)
                })
        })
        .collect()
}

/// One retained reporter record whose source and identity evidence resolve to a durable key.
struct ContributingRecord<'a> {
    reporter: ReporterId,
    record:   &'a DeviceRecord,
}

fn contributing_records<'a>(
    reporters: &'a Reporters,
    devices: &Devices,
    key: &DeviceKey,
) -> Vec<ContributingRecord<'a>> {
    reporters
        .registered_reporters()
        .filter_map(|registered_reporter| {
            let ReporterContribution::Completed { device_set, .. } =
                registered_reporter.contribution
            else {
                return None;
            };
            Some((registered_reporter.reporter, device_set))
        })
        .flat_map(|(reporter, device_set)| {
            device_set
                .devices
                .iter()
                .enumerate()
                .filter_map(move |(record_index, record)| {
                    let CapabilitySourceState::Retained(source) = record.capabilities.source()
                    else {
                        return None;
                    };
                    if source.reporter != reporter
                        || source.record_index != record_index
                        || !record_contributes_to_key(record, devices, key)
                    {
                        return None;
                    }
                    Some(ContributingRecord { reporter, record })
                })
        })
        .collect()
}

fn record_contributes_to_key(record: &DeviceRecord, devices: &Devices, key: &DeviceKey) -> bool {
    match &record.reported_as {
        ReportedAs::Keyed(reported_key) => reported_key == key,
        ReportedAs::MatchEvidenceOnly => {
            let PlatformDeviceHandle::Reported(reported_id) = &record.platform_device_handle else {
                return false;
            };
            matches!(
                devices.resolve_reported_handle(reported_id),
                ReportedHandleResolution::OneKey(resolved_key) if resolved_key == *key
            )
        },
    }
}

/// Point each binding entity at the device entity its durable endpoint currently resolves to.
///
/// The link is the projection of live resolution, never authored ownership: `Bindings` stays
/// authoritative for which role owns which endpoint, and a role whose device is absent simply
/// carries no link. Bevy maintains the device-side `ResolvedBindings` collection from this side.
fn resolve_binding_links(world: &mut World, devices: &Devices) {
    for planned_link in planned_binding_links(world, devices) {
        let Ok(mut entity) = world.get_entity_mut(planned_link.binding_entity) else {
            continue;
        };
        match planned_link.device_entity {
            ResolvedDeviceEntity::Projected(device_entity) => {
                if entity.get::<ResolvedToDevice>().map(|link| link.device()) != Some(device_entity)
                {
                    entity.insert(ResolvedToDevice::new(device_entity));
                }
            },
            ResolvedDeviceEntity::NotProjected => {
                entity.remove::<ResolvedToDevice>();
            },
        }
    }
}

/// Decide every binding entity's link while the world is still borrowed immutably.
///
/// `Bindings` is read through a shared borrow rather than `World::resource_scope`, which reinserts
/// the resource and marks it changed whether or not the closure wrote to it. Deciding first and
/// writing afterwards keeps a pass that resolves nothing new invisible to a change filter.
fn planned_binding_links(world: &World, devices: &Devices) -> Vec<PlannedBindingLink> {
    let bindings = world.resource::<Bindings>();

    bindings
        .registered_roles()
        .filter_map(|role| {
            let binding_entity = bindings.role_entity(role).ok()?;
            let Ok(binding) = bindings.binding(role) else {
                return None;
            };
            Some(PlannedBindingLink {
                binding_entity,
                device_entity: resolved_device_entity(devices, &binding.endpoint.device),
            })
        })
        .collect()
}

/// One binding entity and the device entity its durable endpoint resolves to on this pass.
struct PlannedBindingLink {
    binding_entity: Entity,
    device_entity:  ResolvedDeviceEntity,
}

/// Whether a durable device key currently names an entity the projection has produced.
///
/// Distinct from `DeviceResolution`, which answers only whether the key has a process-local handle:
/// a key can resolve to a `DeviceId` on a pass whose entity has not been spawned yet, and the link
/// must be removed in that case rather than pointed at nothing.
enum ResolvedDeviceEntity {
    /// The key names this live device entity, so the binding's link is inserted or replaced.
    Projected(Entity),
    /// No live device entity carries the key, so the binding's link is removed.
    NotProjected,
}

/// Find the live device entity one durable key currently names.
fn resolved_device_entity(devices: &Devices, key: &DeviceKey) -> ResolvedDeviceEntity {
    let DeviceResolution::Resolved(device_id) = devices.resolve(key) else {
        return ResolvedDeviceEntity::NotProjected;
    };
    match devices.entity(device_id) {
        DeviceEntityLookup::Projected(entity) => ResolvedDeviceEntity::Projected(entity),
        DeviceEntityLookup::NotProjected => ResolvedDeviceEntity::NotProjected,
    }
}

/// Mirror each role's last-known-good configuration onto its binding entity at the driver's own
/// type.
///
/// It lands on the binding entity rather than the device entity because one unit can serve several
/// roles — a Stream Deck's key, dial, and strip — each holding a different configuration, which a
/// single component on the device would lose. An unchanged value is skipped: `ReflectComponent::
/// apply` writes unconditionally, so mirroring every pass would make every downstream `Changed`
/// filter true on every frame.
///
/// A role whose authority returned to `LastKnownGoodConfiguration::NotEstablished` has its mirror
/// removed, because a component left behind would read as a configuration this kernel would restore
/// while nothing here would restore it. The removal is driven by `MirroredConfigurationType`, so it
/// happens on the pass the value went away and not on every later pass.
fn mirror_last_known_good(world: &mut World, type_registry: &TypeRegistry) {
    for planned_mirror in planned_configuration_mirrors(world, type_registry) {
        match planned_mirror {
            PlannedConfigurationMirror::Write {
                binding_entity,
                reflect_component,
                configuration,
                type_path,
            } => {
                let Ok(mut entity) = world.get_entity_mut(binding_entity) else {
                    continue;
                };
                reflect_component.insert(&mut entity, configuration.as_ref(), type_registry);
                entity.insert(MirroredConfigurationType { type_path });
            },
            PlannedConfigurationMirror::Erase {
                binding_entity,
                reflect_component,
            } => {
                let Ok(mut entity) = world.get_entity_mut(binding_entity) else {
                    continue;
                };
                reflect_component.remove(&mut entity);
                entity.remove::<MirroredConfigurationType>();
            },
        }
    }
}

/// Which driver configuration type the mirror last wrote onto one binding entity.
///
/// The mirror needs it to remove that component later: the authority holding
/// `LastKnownGoodConfiguration::NotEstablished` no longer names the type it once held, and nothing
/// else on the binding entity records what a driver's configuration type was.
#[derive(Component)]
struct MirroredConfigurationType {
    type_path: String,
}

/// Decide which binding entities need a configuration write while the world is borrowed immutably.
///
/// Both the authoritative value and the entity's current component are read here, so the
/// equal-value skip is decided before anything can be written. `Bindings` is read through a shared
/// borrow rather than `World::resource_scope`, which marks the resource changed on reinsertion even
/// for a pass that wrote nothing. The value is copied out as a dynamic so the borrow can be
/// released before the insert; `ReflectComponent::insert` rebuilds the driver's concrete type from
/// it.
fn planned_configuration_mirrors<'a>(
    world: &World,
    type_registry: &'a TypeRegistry,
) -> Vec<PlannedConfigurationMirror<'a>> {
    let bindings = world.resource::<Bindings>();
    let mut planned_mirrors = Vec::new();

    for role in bindings.registered_roles() {
        let Ok(binding_entity) = bindings.role_entity(role) else {
            continue;
        };
        let Ok(binding) = bindings.binding(role) else {
            continue;
        };
        let configuration: &dyn Reflect = match &binding.last_known_good {
            LastKnownGoodConfiguration::NotEstablished => {
                if let Some(planned_erase) =
                    planned_configuration_erase(world, type_registry, binding_entity)
                {
                    planned_mirrors.push(planned_erase);
                }
                continue;
            },
            LastKnownGoodConfiguration::MatchesRequested => binding.requested.configuration(),
            LastKnownGoodConfiguration::DiffersFromDispatched(configuration) => {
                configuration.as_ref()
            },
        };
        let reflect_component =
            match reflect_component_for(configuration.as_partial_reflect(), type_registry) {
                Ok(reflect_component) => reflect_component,
                Err(capability_attach_error) => {
                    warn!(
                        "role `{role:?}` driver configuration cannot be mirrored: \
                         {capability_attach_error}"
                    );
                    continue;
                },
            };
        let Ok(mirrored_entity) = world.get_entity(binding_entity) else {
            continue;
        };
        let already_mirrored = reflect_component
            .reflect(mirrored_entity)
            .is_some_and(|mirrored| {
                mirrored.reflect_partial_eq(configuration.as_partial_reflect()) == Some(true)
            });
        if already_mirrored {
            continue;
        }
        planned_mirrors.push(PlannedConfigurationMirror::Write {
            binding_entity,
            reflect_component,
            configuration: configuration.as_partial_reflect().to_dynamic(),
            type_path: configuration.reflect_type_path().to_owned(),
        });
    }

    planned_mirrors
}

/// Decide whether one binding entity still carries a mirror the authority no longer backs.
///
/// The type path recorded by the last write is what identifies the component to remove: the
/// authority holds `LastKnownGoodConfiguration::NotEstablished` at this point and no longer names a
/// driver type. An entity with no `MirroredConfigurationType` never had a mirror, so this plans
/// nothing for it and a settled role stays settled.
fn planned_configuration_erase<'a>(
    world: &World,
    type_registry: &'a TypeRegistry,
    binding_entity: Entity,
) -> Option<PlannedConfigurationMirror<'a>> {
    let mirrored_type = world
        .get_entity(binding_entity)
        .ok()?
        .get::<MirroredConfigurationType>()?;
    let reflect_component = type_registry
        .get_with_type_path(&mirrored_type.type_path)
        .and_then(|type_registration| type_registration.data::<ReflectComponent>())?;

    Some(PlannedConfigurationMirror::Erase {
        binding_entity,
        reflect_component,
    })
}

/// One binding entity's pending configuration change, held while the `Bindings` borrow is released.
enum PlannedConfigurationMirror<'a> {
    /// Put the authority's current value on the binding entity at the driver's own type.
    Write {
        binding_entity:    Entity,
        reflect_component: &'a ReflectComponent,
        configuration:     Box<dyn PartialReflect>,
        type_path:         String,
    },
    /// Take the previously mirrored component off the binding entity.
    Erase {
        binding_entity:    Entity,
        reflect_component: &'a ReflectComponent,
    },
}

/// Report every device whose contributors changed what they disagree about, once per change.
///
/// The warning makes a disagreement visible with no user interface attached. An empty capability
/// list means the disagreement cleared and the device is fully drivable again.
fn announce_disputes(
    devices: &Devices,
    disputes_changed: &[DeviceId],
    type_registry: &TypeRegistry,
) {
    for device_id in disputes_changed {
        let DeviceStateLookup::Retained(reconciled_device_state) = devices.state(*device_id) else {
            continue;
        };
        // Sorted so one disagreement reads the same in every log line, since
        // `ReconciledDeviceState::disputed` is a set whose iteration order varies between runs.
        let mut capabilities: Vec<String> = reconciled_device_state
            .disputed
            .iter()
            .map(|type_id| {
                type_registry.get_type_info(*type_id).map_or_else(
                    || format!("{type_id:?}"),
                    |type_info| type_info.type_path().to_owned(),
                )
            })
            .collect();
        capabilities.sort();

        if capabilities.is_empty() {
            warn!(
                "device `{:?}` reporters no longer disagree about any capability",
                reconciled_device_state.key
            );
        } else {
            warn!(
                "device `{:?}` reporters disagree about capabilities {capabilities:?}",
                reconciled_device_state.key
            );
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::alloc::GlobalAlloc;
    use std::alloc::Layout;
    use std::alloc::System;
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use std::time::Instant;

    use bevy::app::App;
    use bevy::app::PostUpdate;
    use bevy::ecs::change_detection::DetectChanges;
    use bevy::ecs::entity::Entity;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::ecs::relationship::RelationshipTarget;
    use bevy::prelude::Changed;
    use bevy::prelude::Component;
    use bevy::prelude::Query;
    use bevy::prelude::Reflect;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::World;
    use bevy::time::Real;
    use bevy::time::Time;
    use bevy::world_serialization::DynamicWorldBuilder;

    use super::FrameClockReading;
    use super::FreshnessLease;
    use super::PriorKeyAvailability;
    use super::ReconcilePass;
    use super::planned_configuration_mirrors;
    use super::project_device_entities;
    use super::reconcile_devices;
    use super::reconcile_work;
    use super::reflect_component_for;
    use crate::Applied;
    use crate::ApplyContext;
    use crate::ApplyDeadline;
    use crate::AttachmentPath;
    use crate::AttemptInvalidation;
    use crate::AttemptRef;
    use crate::AuthoritativeReporterCoverage;
    use crate::Binding;
    use crate::Bindings;
    use crate::Capabilities;
    use crate::Claim;
    use crate::ClaimHolder;
    use crate::ConfiguredDevice;
    use crate::ConfiguredDeviceConnection;
    use crate::ConfiguredDeviceMode;
    use crate::ConfiguredDeviceName;
    use crate::CoveredDeviceIdentitySpace;
    use crate::DeviceDescriptor;
    use crate::DeviceEndpoint;
    use crate::DeviceId;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DeviceRecord;
    use crate::DeviceReporter;
    use crate::DeviceResolution;
    use crate::DeviceScan;
    use crate::DeviceStateLookup;
    use crate::Devices;
    use crate::Digest;
    use crate::DiscoveryCadence;
    use crate::DiscoveryLimits;
    use crate::DiscoveryWork;
    use crate::DriverCleanupRoleEntity;
    use crate::DriverCompletion;
    use crate::EndpointDriver;
    use crate::EndpointId;
    use crate::EstablishedContext;
    use crate::FlowExpectation;
    use crate::HardwareInventory;
    use crate::HardwareWait;
    use crate::IdentityVerdict;
    use crate::KeyAvailability;
    use crate::LastKnownGoodConfiguration;
    use crate::MainThreadDiscoveryJob;
    use crate::OnAbort;
    use crate::OnSessionLoss;
    use crate::PlatformDeviceHandle;
    use crate::Presence;
    use crate::RecoveryPolicy;
    use crate::RegisteredSchemes;
    use crate::ReportedAs;
    use crate::ReportedHandleResolution;
    use crate::ReportedId;
    use crate::ReportedParent;
    use crate::ReportedSerial;
    use crate::ReporterCoverage;
    use crate::ReporterId;
    use crate::ReporterRef;
    use crate::ReporterRegistration;
    use crate::RequestedConfiguration;
    use crate::RetryOn;
    use crate::RiggingAppExt;
    use crate::RiggingLimits;
    use crate::RiggingPlugin;
    use crate::RiggingRevision;
    use crate::RiggingRuntimeClock;
    use crate::RiggingRuntimeTime;
    use crate::RoleKey;
    use crate::RoleStatusView;
    use crate::SchemeName;
    use crate::SessionRef;
    use crate::SessionReleaseCause;
    use crate::TargetResolution;
    use crate::TargetResolutionContext;
    use crate::UnconfirmedBasis;
    use crate::UnverifiedReason;
    use crate::WaitingStatusView;
    use crate::binding::WaitingWork;
    use crate::capabilities::CapabilityAttachError;
    use crate::devices::DeviceAvailabilityChange;
    use crate::devices::DeviceEntityLookup;
    use crate::devices::ReconciledDeviceChanges;
    use crate::registration::DriverId;
    use crate::registration::Drivers;
    use crate::registration::Reporters;
    use crate::scheme::AuthoredId;

    /// Counts the bytes one thread requests so a test can prove the settled reconcile path asks
    /// the allocator for nothing.
    ///
    /// The counter is thread-local and const-initialized, so the allocator itself never allocates
    /// and a test measuring its own thread cannot be disturbed by another test's work.
    struct CountingAllocator;

    thread_local! {
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        static COUNTING: Cell<bool> = const { Cell::new(false) };
    }

    // SAFETY: every method forwards to the system allocator with the same layout it received, so
    // the allocation contract is the system allocator's. The counter only reads and writes a
    // `Cell<usize>` in thread-local storage and allocates nothing itself.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            record_allocation();
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) };
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            record_allocation();
            unsafe { System.realloc(pointer, layout, new_size) }
        }
    }

    fn record_allocation() {
        let _ = COUNTING.try_with(|counting| {
            if counting.get() {
                let _ = ALLOCATIONS.try_with(|allocations| {
                    allocations.set(allocations.get() + 1);
                });
            }
        });
    }

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    /// Run one closure with this thread's allocation counter enabled and report the count.
    fn allocations_during(measured: impl FnOnce()) -> usize {
        ALLOCATIONS.with(|allocations| allocations.set(0));
        COUNTING.with(|counting| counting.set(true));
        measured();
        COUNTING.with(|counting| counting.set(false));
        ALLOCATIONS.with(Cell::get)
    }

    /// A reporter that returns the same authored records on every scan.
    struct FixedReporter(fn() -> Vec<DeviceRecord>);

    impl DeviceReporter for FixedReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let build = self.0;
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| {
                DeviceScan::Complete(build())
            }))
        }
    }

    const TEST_SCHEME: &str = "test-scheme";

    fn scheme() -> SchemeName { SchemeName::new(TEST_SCHEME).expect("test scheme is well formed") }

    fn key(value: &str) -> DeviceKey { keyed_in(TEST_SCHEME, value) }

    fn keyed_in(scheme_name: &str, value: &str) -> DeviceKey {
        DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: SchemeName::new(scheme_name).expect("test scheme is well formed"),
                value:  ReportedId::new(value).expect("test reported id is well formed"),
            },
        }
    }

    fn record(reported_as: ReportedAs) -> DeviceRecord {
        DeviceRecord {
            reported_as,
            parent: ReportedParent::Root,
            presence: Presence::Present,
            claim: Claim::NotApplicable,
            capabilities: Capabilities::new(),
            serial: ReportedSerial::NotExposedByUnit,
            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
            attachment: AttachmentPath::PlatformHasNoConcept,
            descriptor: DeviceDescriptor::PlatformReportedNothing,
        }
    }

    fn app_with_scheme() -> App {
        let mut app = App::new();
        app.add_plugins(RiggingPlugin);
        app.register_device_scheme(scheme());
        app
    }

    fn every_frame() -> DiscoveryCadence {
        DiscoveryCadence::Periodic {
            interval: Duration::ZERO,
        }
    }

    fn add_reporter(app: &mut App, build: fn() -> Vec<DeviceRecord>) -> ReporterId {
        add_reporter_with_cadence(app, build, every_frame())
    }

    fn add_reporter_with_cadence(
        app: &mut App,
        build: fn() -> Vec<DeviceRecord>,
        cadence: DiscoveryCadence,
    ) -> ReporterId {
        app.add_device_reporter(
            FixedReporter(build),
            ReporterRegistration::required(
                cadence,
                ReporterCoverage::MatchingEvidenceOnly,
                std::time::Duration::from_secs(10),
            ),
        )
    }

    /// Run one reconcile pass over the app's resources without going through the schedule, so a
    /// test controls the frame clock and the reporter registry contents exactly.
    fn reconcile_once(app: &mut App, clock: FrameClockReading) -> usize {
        let mut allocations = 0;
        let bindings = Bindings::default();
        let runtime_clock = *app.world().resource::<RiggingRuntimeClock>();
        let observed_at = match clock {
            FrameClockReading::Measurable(observed_at) => observed_at,
            FrameClockReading::NotYetAdvanced => {
                runtime_clock.instant_at(RiggingRuntimeTime::from_elapsed(Duration::ZERO))
            },
        };
        app.world_mut()
            .resource_scope::<Reporters, _>(|world, mut reporters| {
                world.resource_scope::<Devices, _>(|world, mut devices| {
                    world.resource_scope::<RiggingRevision, _>(|world, mut rigging_revision| {
                        world.resource_scope::<RiggingLimits, _>(|world, rigging_limits| {
                            world.resource_scope::<RegisteredSchemes, _>(
                                |_, registered_schemes| {
                                    allocations = allocations_during(|| {
                                        let hardware_inventory = HardwareInventory::default();
                                        let freshness_lease = FreshnessLease {
                                            rigging_limits: &rigging_limits,
                                            clock,
                                        };
                                        let reconcile_work = reconcile_work(
                                            &reporters,
                                            &devices,
                                            &registered_schemes,
                                            &hardware_inventory,
                                            &bindings,
                                            freshness_lease,
                                            &rigging_limits,
                                            observed_at,
                                            runtime_clock,
                                            devices.departure_grace_due(
                                                runtime_clock.time_at(observed_at),
                                            ),
                                        );
                                        reconcile_devices(
                                            &mut reporters,
                                            &mut devices,
                                            &mut rigging_revision,
                                            reconcile_work,
                                        );
                                    });
                                },
                            );
                        });
                    });
                });
            });
        allocations
    }

    fn resolved(devices: &Devices, device_key: &DeviceKey) -> Option<crate::DeviceId> {
        match devices.resolve(device_key) {
            DeviceResolution::Resolved(device_id) => Some(device_id),
            DeviceResolution::NotResolved => None,
        }
    }

    fn presence_of(devices: &Devices, device_key: &DeviceKey) -> Option<Presence> {
        let device_id = resolved(devices, device_key)?;
        match devices.state(device_id) {
            DeviceStateLookup::Retained(state) => Some(state.presence),
            DeviceStateLookup::Retired => None,
        }
    }

    fn claim_of(devices: &Devices, device_key: &DeviceKey) -> Option<Claim> {
        let device_id = resolved(devices, device_key)?;
        match devices.state(device_id) {
            DeviceStateLookup::Retained(state) => Some(state.claim.clone()),
            DeviceStateLookup::Retired => None,
        }
    }

    fn contributors(devices: &Devices, device_key: &DeviceKey) -> Vec<ReporterId> {
        let Some(device_id) = resolved(devices, device_key) else {
            return Vec::new();
        };
        match devices.state(device_id) {
            DeviceStateLookup::Retained(state) => state.contributors.clone(),
            DeviceStateLookup::Retired => Vec::new(),
        }
    }

    /// Run the frames one accepted whole set needs: discovery admits the scan on the first frame
    /// and reconciliation sees the accepted set on the next one.
    fn run_until_reconciled(app: &mut App) {
        app.update();
        app.update();
    }

    fn advance_past_departure_grace(app: &mut App) {
        let departure_grace = app.world().resource::<RiggingLimits>().departure_grace;
        let time = app.world().resource::<Time<Real>>();
        let now = time.last_update().unwrap_or_else(|| time.startup());
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .update_with_instant(now + departure_grace + Duration::from_secs(1));
        app.update();
    }

    #[test]
    fn two_reporters_naming_one_display_produce_one_device_with_two_contributors() {
        let mut app = app_with_scheme();
        let first = add_reporter(&mut app, || vec![record(ReportedAs::Keyed(key("panel-a")))]);
        let second = add_reporter(&mut app, || vec![record(ReportedAs::Keyed(key("panel-a")))]);

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert_eq!(devices.count(), 1);
        assert_eq!(contributors(devices, &key("panel-a")), vec![first, second]);
    }

    #[test]
    fn one_reporter_repeating_a_key_within_one_scan_reports_it_as_duplicated() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || {
            vec![
                record(ReportedAs::Keyed(key("panel-a"))),
                record(ReportedAs::Keyed(key("panel-a"))),
            ]
        });

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert!(devices.duplicate_keys().contains(&key("panel-a")));
        // The pass draws no conclusion from the duplication: the key still resolves to one device,
        // and the identity verdict stage is what turns the report into an unverified verdict.
        assert_eq!(devices.count(), 1);
    }

    #[test]
    fn a_child_of_an_unreachable_parent_is_unreachable_and_not_absent() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || {
            let mut parent = record(ReportedAs::Keyed(key("capture-card")));
            parent.presence = Presence::Unreachable {
                since: Duration::from_secs(3),
            };
            let mut child = record(ReportedAs::Keyed(key("camera")));
            child.parent = ReportedParent::ChildOf(key("capture-card"));
            vec![parent, child]
        });

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert!(matches!(
            presence_of(devices, &key("camera")),
            Some(Presence::Unreachable { .. })
        ));
        assert!(matches!(
            devices.key_availability(&key("camera")),
            PriorKeyAvailability::Published(KeyAvailability::Unreachable { .. })
        ));
    }

    #[test]
    fn a_set_listing_children_before_parents_still_ingests_roots_first() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || {
            let mut grandchild = record(ReportedAs::Keyed(key("camera")));
            grandchild.parent = ReportedParent::ChildOf(key("capture-card"));
            let mut child = record(ReportedAs::Keyed(key("capture-card")));
            child.parent = ReportedParent::ChildOf(key("dock"));
            let mut root = record(ReportedAs::Keyed(key("dock")));
            root.presence = Presence::Unreachable {
                since: Duration::from_secs(1),
            };
            vec![grandchild, child, root]
        });

        run_until_reconciled(&mut app);

        // The root's uncertainty reaches the leaf, which is only possible if the fold settled the
        // root before the two devices that were reported ahead of it.
        let devices = app.world().resource::<Devices>();
        assert!(matches!(
            presence_of(devices, &key("camera")),
            Some(Presence::Unreachable { .. })
        ));
        assert!(matches!(
            presence_of(devices, &key("capture-card")),
            Some(Presence::Unreachable { .. })
        ));
    }

    #[test]
    fn a_child_whose_parent_is_missing_from_the_whole_set_is_unreachable() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || {
            let mut child = record(ReportedAs::Keyed(key("camera")));
            child.parent = ReportedParent::ChildOf(key("capture-card"));
            vec![child]
        });

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert!(matches!(
            presence_of(devices, &key("camera")),
            Some(Presence::Unreachable { .. })
        ));
    }

    #[test]
    fn resolution_names_the_unresolved_case_without_an_optional_handle() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || vec![record(ReportedAs::Keyed(key("panel-a")))]);

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert!(matches!(
            devices.resolve(&key("panel-a")),
            DeviceResolution::Resolved(_)
        ));
        assert_eq!(
            devices.resolve(&key("never-reported")),
            DeviceResolution::NotResolved
        );
    }

    #[test]
    fn a_settled_frame_leaves_the_revision_unmoved_and_allocates_nothing() {
        let mut app = app_with_scheme();
        add_reporter_with_cadence(
            &mut app,
            || vec![record(ReportedAs::Keyed(key("panel-a")))],
            DiscoveryCadence::EventDriven {
                backstop: Duration::from_hours(1),
            },
        );
        let wedged = add_reporter_with_cadence(
            &mut app,
            || vec![record(ReportedAs::Keyed(key("panel-b")))],
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(5),
            },
        );

        run_until_reconciled(&mut app);
        let settled_revision = *app.world().resource::<RiggingRevision>();

        // Nothing completed and no lease can expire within an hour-long backstop, so this pass has
        // no work: it must neither move the revision nor ask the allocator for anything.
        let allocations = reconcile_once(&mut app, FrameClockReading::Measurable(Instant::now()));

        assert_eq!(allocations, 0);
        assert_eq!(*app.world().resource::<RiggingRevision>(), settled_revision);

        let report_grace = app.world().resource::<RiggingLimits>().report_grace;
        app.world_mut()
            .resource_mut::<Reporters>()
            .backdate_completion(wedged, report_grace + Duration::from_mins(10));
        reconcile_once(&mut app, FrameClockReading::Measurable(Instant::now()));

        // The wedged reporter stays wedged for as long as the application runs. Its device was
        // judged once, so every following frame has to settle without collecting anything.
        let allocations = reconcile_once(&mut app, FrameClockReading::Measurable(Instant::now()));

        assert_eq!(allocations, 0);
        assert_eq!(*app.world().resource::<RiggingRevision>(), settled_revision);
        assert!(matches!(
            presence_of(app.world().resource::<Devices>(), &key("panel-b")),
            Some(Presence::Unreachable { .. })
        ));
    }

    #[test]
    fn a_reporter_silent_past_its_cadence_loses_its_devices_and_no_others() {
        let mut app = app_with_scheme();
        let silent = add_reporter_with_cadence(
            &mut app,
            || vec![record(ReportedAs::Keyed(key("silent-panel")))],
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(5),
            },
        );
        add_reporter_with_cadence(
            &mut app,
            || vec![record(ReportedAs::Keyed(key("live-panel")))],
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(5),
            },
        );

        run_until_reconciled(&mut app);

        let report_grace = app.world().resource::<RiggingLimits>().report_grace;
        app.world_mut()
            .resource_mut::<Reporters>()
            .backdate_completion(silent, report_grace + Duration::from_mins(10));
        reconcile_once(&mut app, FrameClockReading::Measurable(Instant::now()));

        let devices = app.world().resource::<Devices>();
        assert!(matches!(
            presence_of(devices, &key("silent-panel")),
            Some(Presence::Unreachable { .. })
        ));
        assert_eq!(
            presence_of(devices, &key("live-panel")),
            Some(Presence::Present)
        );
    }

    #[test]
    fn a_silent_reporter_does_not_lower_a_device_a_fresh_reporter_still_reports() {
        let mut app = app_with_scheme();
        let winit_like = add_reporter_with_cadence(
            &mut app,
            || vec![record(ReportedAs::Keyed(key("panel-a")))],
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(5),
            },
        );
        let wedged = add_reporter_with_cadence(
            &mut app,
            || vec![record(ReportedAs::Keyed(key("panel-a")))],
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(5),
            },
        );

        run_until_reconciled(&mut app);

        let report_grace = app.world().resource::<RiggingLimits>().report_grace;
        let silence = report_grace + Duration::from_mins(10);
        app.world_mut()
            .resource_mut::<Reporters>()
            .backdate_completion(wedged, silence);
        reconcile_once(&mut app, FrameClockReading::Measurable(Instant::now()));

        // One reporter wedging withdraws its evidence; it does not report the device gone, and the
        // reporter that still enumerates the device every few seconds keeps it present.
        assert_eq!(
            presence_of(app.world().resource::<Devices>(), &key("panel-a")),
            Some(Presence::Present)
        );
        let devices = app.world().resource::<Devices>();
        let PriorKeyAvailability::Published(KeyAvailability::Present(evidence)) =
            devices.key_availability(&key("panel-a"))
        else {
            panic!("the fresh contributor did not keep the key present");
        };
        assert_eq!(evidence.contributors().as_slice().len(), 1);
        assert_eq!(
            evidence.contributors().as_slice()[0].reporter,
            ReporterRef::from_reporter_id(winit_like)
        );

        app.world_mut()
            .resource_mut::<Reporters>()
            .backdate_completion(winit_like, silence);
        reconcile_once(&mut app, FrameClockReading::Measurable(Instant::now()));

        assert!(matches!(
            presence_of(app.world().resource::<Devices>(), &key("panel-a")),
            Some(Presence::Unreachable { .. })
        ));
    }

    #[test]
    fn an_expired_keyed_owner_withdraws_evidence_only_presence_in_the_expiry_frame()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let device_key = key("lease-owned-panel");
        let role = RoleKey::new("lease-owned-panel")?;
        let handle = ReportedId::new("lease-owned-handle")?;
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::new(AtomicUsize::new(0))));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(panel_binding(role.clone(), device_key.clone(), driver))?;
        let owner = add_reporter_with_cadence(
            &mut app,
            || {
                let mut owner = record(ReportedAs::Keyed(key("lease-owned-panel")));
                owner.platform_device_handle = PlatformDeviceHandle::Reported(
                    ReportedId::new("lease-owned-handle").expect("test handle is well formed"),
                );
                vec![owner]
            },
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(5),
            },
        );
        let evidence_only = add_reporter_with_cadence(
            &mut app,
            || {
                let mut evidence = record(ReportedAs::MatchEvidenceOnly);
                evidence.platform_device_handle = PlatformDeviceHandle::Reported(
                    ReportedId::new("lease-owned-handle").expect("test handle is well formed"),
                );
                vec![evidence]
            },
            DiscoveryCadence::OnDemand,
        );

        run_until_reconciled(&mut app);
        let devices = app.world().resource::<Devices>();
        let PriorKeyAvailability::Published(KeyAvailability::Present(evidence)) =
            devices.key_availability(&device_key)
        else {
            return Err("the initial exact handle match was not published as present".into());
        };
        assert_eq!(evidence.contributors().as_slice().len(), 2);
        assert!(
            evidence
                .contributors()
                .as_slice()
                .iter()
                .any(|contributor| {
                    contributor.reporter == ReporterRef::from_reporter_id(evidence_only)
                })
        );

        let report_grace = app.world().resource::<RiggingLimits>().report_grace;
        app.world_mut()
            .resource_mut::<Reporters>()
            .backdate_completion(owner, report_grace + Duration::from_mins(10));
        let changes = reconcile_and_project_changes(&mut app, now());

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].key, device_key);
        assert!(matches!(changes[0].from, KeyAvailability::Present(_)));
        let KeyAvailability::Unreachable { reporters, .. } = &changes[0].to else {
            return Err("lease expiry did not publish unreachable availability".into());
        };
        assert!(reporters.contains(ReporterRef::from_reporter_id(owner)));
        assert_eq!(
            app.world()
                .resource::<Devices>()
                .resolve_reported_handle(&handle),
            ReportedHandleResolution::NoKeyedRecord
        );
        let projected_status = app.world().resource::<Bindings>().projected_status(&role)?;
        let RoleStatusView::Waiting(WaitingStatusView::Reporter(HardwareWait::Unreachable {
            key: waited_key,
            reporters: waited_reporters,
            ..
        })) = projected_status
        else {
            return Err("the role did not enter its unreachable hardware wait".into());
        };
        assert_eq!(waited_key, device_key);
        assert!(waited_reporters.contains(ReporterRef::from_reporter_id(owner)));

        Ok(())
    }

    #[test]
    fn keyed_and_evidence_only_records_publish_the_same_present_decision() {
        let mut keyed_app = app_with_scheme();
        add_reporter(&mut keyed_app, || {
            vec![record(ReportedAs::Keyed(key("keyed-decision")))]
        });
        add_reporter(&mut keyed_app, || {
            vec![record(ReportedAs::Keyed(key("keyed-decision")))]
        });
        run_until_reconciled(&mut keyed_app);

        let mut evidence_app = app_with_scheme();
        add_reporter(&mut evidence_app, || {
            let mut keyed = record(ReportedAs::Keyed(key("evidence-decision")));
            keyed.platform_device_handle = PlatformDeviceHandle::Reported(
                ReportedId::new("decision-handle").expect("test handle is well formed"),
            );
            vec![keyed]
        });
        add_reporter(&mut evidence_app, || {
            let mut evidence = record(ReportedAs::MatchEvidenceOnly);
            evidence.platform_device_handle = PlatformDeviceHandle::Reported(
                ReportedId::new("decision-handle").expect("test handle is well formed"),
            );
            vec![evidence]
        });
        run_until_reconciled(&mut evidence_app);

        for (app, device_key) in [
            (&keyed_app, key("keyed-decision")),
            (&evidence_app, key("evidence-decision")),
        ] {
            let PriorKeyAvailability::Published(KeyAvailability::Present(evidence)) = app
                .world()
                .resource::<Devices>()
                .key_availability(&device_key)
            else {
                panic!("the two-report contribution did not publish present availability");
            };
            assert_eq!(evidence.contributors().as_slice().len(), 2);
        }
    }

    #[test]
    fn covering_and_noncovering_omissions_publish_distinct_availability() {
        let omitted_key = key("omitted-panel");
        let availability_after = |coverage| {
            let mut app = app_with_scheme();
            app.world_mut()
                .resource_mut::<HardwareInventory>()
                .configure(ConfiguredDevice {
                    key:  omitted_key.clone(),
                    mode: ConfiguredDeviceMode::Managed,
                    name: ConfiguredDeviceName::NeverDerived,
                });
            add_set_reporter(&mut app, Vec::new(), coverage);
            run_until_reconciled(&mut app);
            match app
                .world()
                .resource::<Devices>()
                .key_availability(&omitted_key)
            {
                PriorKeyAvailability::Published(availability) => availability.clone(),
                PriorKeyAvailability::NeverPublished => {
                    panic!("the configured key received no availability conclusion");
                },
            }
        };

        assert!(matches!(
            availability_after(establishes_absence()),
            KeyAvailability::Absent { .. }
        ));
        assert!(matches!(
            availability_after(ReporterCoverage::MatchingEvidenceOnly),
            KeyAvailability::Unconfirmed {
                basis: UnconfirmedBasis::NoFreshEvidence,
                ..
            }
        ));
    }

    #[test]
    fn a_reporter_that_never_completed_a_scan_is_not_stale() {
        let mut app = app_with_scheme();
        add_reporter_with_cadence(
            &mut app,
            Vec::new,
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(5),
            },
        );

        // No frame has run, so the reporter holds no completed set. A lease that judged silence
        // from registration would call every reporter stale before its first scan.
        let allocations = reconcile_once(&mut app, FrameClockReading::Measurable(Instant::now()));

        assert_eq!(allocations, 0);
        assert_eq!(app.world().resource::<RiggingRevision>().get(), 0);
    }

    #[test]
    fn a_reporter_with_no_declared_cadence_is_never_stale() {
        let mut app = app_with_scheme();
        let on_demand = add_reporter_with_cadence(
            &mut app,
            || vec![record(ReportedAs::Keyed(key("panel-a")))],
            DiscoveryCadence::OnDemand,
        );

        run_until_reconciled(&mut app);
        app.world_mut()
            .resource_mut::<Reporters>()
            .backdate_completion(on_demand, Duration::from_hours(24));
        reconcile_once(&mut app, FrameClockReading::Measurable(Instant::now()));

        assert_eq!(
            presence_of(app.world().resource::<Devices>(), &key("panel-a")),
            Some(Presence::Present)
        );
    }

    #[test]
    fn evidence_only_records_join_a_keyed_record_through_the_reported_os_handle() {
        let mut app = app_with_scheme();
        let keyed = add_reporter(&mut app, || {
            let mut keyed_record = record(ReportedAs::Keyed(key("panel-a")));
            keyed_record.platform_device_handle =
                PlatformDeviceHandle::Reported(ReportedId::new("display-7").expect("well formed"));
            vec![keyed_record]
        });
        let evidence = add_reporter(&mut app, || {
            let mut evidence_record = record(ReportedAs::MatchEvidenceOnly);
            evidence_record.platform_device_handle =
                PlatformDeviceHandle::Reported(ReportedId::new("display-7").expect("well formed"));
            vec![evidence_record]
        });

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert_eq!(devices.count(), 1);
        assert_eq!(
            contributors(devices, &key("panel-a")),
            vec![keyed, evidence]
        );
        assert_eq!(
            devices.resolve_reported_handle(&ReportedId::new("display-7").expect("well formed")),
            ReportedHandleResolution::OneKey(key("panel-a"))
        );
    }

    #[test]
    fn a_joined_evidence_only_record_merges_as_a_co_report_of_the_device_it_joins() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || {
            let mut keyed_record = record(ReportedAs::Keyed(key("panel-a")));
            keyed_record.platform_device_handle =
                PlatformDeviceHandle::Reported(ReportedId::new("display-7").expect("well formed"));
            vec![keyed_record]
        });
        add_reporter(&mut app, || {
            let mut evidence_record = record(ReportedAs::MatchEvidenceOnly);
            evidence_record.platform_device_handle =
                PlatformDeviceHandle::Reported(ReportedId::new("display-7").expect("well formed"));
            evidence_record.presence = Presence::Unreachable {
                since: Duration::from_secs(4),
            };
            evidence_record.claim = Claim::Held;
            vec![evidence_record]
        });

        run_until_reconciled(&mut app);

        // The joined record is a report about the same device, so the merge takes its presence and
        // its more restrictive claim rather than only its reporter id.
        let devices = app.world().resource::<Devices>();
        assert!(matches!(
            presence_of(devices, &key("panel-a")),
            Some(Presence::Unreachable { .. })
        ));
        assert_eq!(claim_of(devices, &key("panel-a")), Some(Claim::Held));
    }

    #[test]
    fn a_platform_handle_two_reporters_give_different_keys_joins_nothing() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || {
            let mut keyed_record = record(ReportedAs::Keyed(key("panel-a")));
            keyed_record.platform_device_handle =
                PlatformDeviceHandle::Reported(ReportedId::new("display-7").expect("well formed"));
            vec![keyed_record]
        });
        add_reporter(&mut app, || {
            let mut keyed_record = record(ReportedAs::Keyed(key("panel-b")));
            keyed_record.platform_device_handle =
                PlatformDeviceHandle::Reported(ReportedId::new("display-7").expect("well formed"));
            vec![keyed_record]
        });
        let evidence = add_reporter(&mut app, || {
            let mut evidence_record = record(ReportedAs::MatchEvidenceOnly);
            evidence_record.platform_device_handle =
                PlatformDeviceHandle::Reported(ReportedId::new("display-7").expect("well formed"));
            vec![evidence_record]
        });

        // Discovery admits a bounded number of jobs per frame, so three reporters need more frames
        // than two before every whole set has been accepted.
        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);

        // The handle names two keys, so it names neither: attaching the evidence to whichever key
        // was ingested last would be the plausible fallback exact-match identity forbids.
        let devices = app.world().resource::<Devices>();
        assert_eq!(devices.count(), 2);
        assert_eq!(
            devices.resolve_reported_handle(&ReportedId::new("display-7").expect("well formed")),
            ReportedHandleResolution::SeveralKeys(HashSet::from([key("panel-a"), key("panel-b"),]))
        );
        for device_key in [key("panel-a"), key("panel-b")] {
            assert!(!contributors(devices, &device_key).contains(&evidence));
        }
    }

    #[test]
    fn an_evidence_only_record_that_matches_nothing_produces_no_device_and_no_key() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || {
            let mut evidence_record = record(ReportedAs::MatchEvidenceOnly);
            evidence_record.platform_device_handle =
                PlatformDeviceHandle::Reported(ReportedId::new("display-9").expect("well formed"));
            vec![evidence_record]
        });

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert_eq!(devices.count(), 0);
        assert_eq!(
            devices.resolve_reported_handle(&ReportedId::new("display-9").expect("well formed")),
            ReportedHandleResolution::NoKeyedRecord
        );
    }

    #[test]
    fn a_later_scan_drops_the_handle_resolution_from_the_prior_scan() {
        let mut app = app_with_scheme();
        let reported_id = ReportedId::new("display-7").expect("well formed");
        let panel = key("panel-a");
        let mut reported_unit = unit(panel.clone());
        reported_unit.platform_device_handle = PlatformDeviceHandle::Reported(reported_id.clone());
        let reported_units = add_set_reporter(
            &mut app,
            vec![reported_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        assert_eq!(
            app.world()
                .resource::<Devices>()
                .resolve_reported_handle(&reported_id),
            ReportedHandleResolution::OneKey(panel.clone())
        );

        rewrite(&reported_units, vec![unit(panel)]);
        run_until_reconciled(&mut app);

        assert_eq!(
            app.world()
                .resource::<Devices>()
                .resolve_reported_handle(&reported_id),
            ReportedHandleResolution::NoKeyedRecord
        );
    }

    #[test]
    fn two_evidence_only_records_without_platform_handles_do_not_join_each_other() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || vec![record(ReportedAs::MatchEvidenceOnly)]);
        add_reporter(&mut app, || vec![record(ReportedAs::MatchEvidenceOnly)]);

        run_until_reconciled(&mut app);

        // Both records carry `PlatformReportedNothing`, which compares equal to itself. Joining on
        // it would create a device out of two reports that share no evidence at all.
        assert_eq!(app.world().resource::<Devices>().count(), 0);
    }

    #[test]
    fn a_key_naming_an_unregistered_scheme_is_rejected_at_the_ingest_boundary() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || {
            vec![
                record(ReportedAs::Keyed(keyed_in("not-registered", "panel-a"))),
                record(ReportedAs::Keyed(key("panel-b"))),
            ]
        });

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert_eq!(devices.count(), 1);
        assert!(
            devices
                .unregistered_schemes()
                .contains(&SchemeName::new("not-registered").expect("test scheme is well formed"))
        );
        assert_eq!(
            devices.resolve(&keyed_in("not-registered", "panel-a")),
            DeviceResolution::NotResolved
        );
    }

    #[test]
    fn merging_reads_each_reporter_set_once_rather_than_joining_them_pairwise() {
        // Three reporters that each name the same two devices. A pairwise join would compare every
        // reporter against every other; the single-pass merge visits six records and stops.
        let mut app = app_with_scheme();
        app.world_mut()
            .resource_mut::<DiscoveryLimits>()
            .set_max_completions_per_frame(
                std::num::NonZeroUsize::new(3).unwrap_or(std::num::NonZeroUsize::MIN),
            );
        let add_subject = |app: &mut App| {
            add_reporter_with_cadence(
                app,
                || {
                    vec![
                        record(ReportedAs::Keyed(key("panel-a"))),
                        record(ReportedAs::Keyed(key("panel-b"))),
                    ]
                },
                DiscoveryCadence::OnDemand,
            )
        };
        let first = add_subject(&mut app);
        let second = add_subject(&mut app);
        let third = add_subject(&mut app);

        // The completion budget admits all three whole sets together so this test isolates merge
        // behavior from scheduler pacing.
        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert_eq!(devices.count(), 2);
        for device_key in [key("panel-a"), key("panel-b")] {
            assert_eq!(
                contributors(devices, &device_key),
                vec![first, second, third]
            );
            assert!(!devices.duplicate_keys().contains(&device_key));
        }
    }

    // --- verdicts, the entity projection, and what follows from them ---

    /// One unit a test reporter names in its whole set.
    ///
    /// Clonable so a test can change the set between passes and provoke a departure;
    /// `DeviceRecord` itself cannot be cloned, because its capability declarations are erased.
    #[derive(Clone)]
    struct ReportedUnit {
        key:                    DeviceKey,
        parent:                 ReportedParent,
        attachment:             AttachmentPath,
        claim:                  Claim,
        presence:               Presence,
        brightness:             Vec<u8>,
        platform_device_handle: PlatformDeviceHandle,
    }

    /// A reporter whose whole set the owning test rewrites between scans.
    struct SetReporter(Arc<Mutex<Vec<ReportedUnit>>>);

    impl DeviceReporter for SetReporter {
        fn discover(&mut self) -> DiscoveryWork {
            let units = Arc::clone(&self.0);
            DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_| {
                DeviceScan::Complete(
                    units
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .iter()
                        .map(reported_record)
                        .collect(),
                )
            }))
        }
    }

    /// A capability whose value two reporters can be made to disagree about.
    #[derive(Clone, PartialEq, Debug, Component, Reflect)]
    #[reflect(Component, PartialEq)]
    struct Brightness(u8);

    /// The driver configuration the last-known-good mirror projects onto a binding entity.
    #[derive(Clone, PartialEq, Debug, Component, Reflect)]
    #[reflect(Component, PartialEq)]
    struct PanelConfiguration(u8);

    /// A driver that establishes one fixed configuration and counts how often it was asked.
    struct CountingCaptureDriver(Arc<AtomicUsize>);

    impl EndpointDriver for CountingCaptureDriver {
        type Configuration = PanelConfiguration;
        type Target = ();

        fn resolve_target(
            &mut self,
            _: &mut World,
            _: &TargetResolutionContext<'_>,
            _: &Self::Configuration,
        ) -> crate::TargetResolution<Self::Target> {
            TargetResolution::Reached(())
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            context: ApplyContext<'_, Self::Configuration>,
            _: &Self::Configuration,
            (): Self::Target,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
            context
                .into_completion()
                .finish(DriverCompletion::Succeeded(Applied::DiffersFromDispatched(
                    PanelConfiguration(7),
                )));
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

    /// A driver configuration that reflects without registering `ReflectComponent`.
    ///
    /// `EndpointDriver::Configuration` requires `Reflect + Component`, which the compiler can
    /// check, but nothing in the type system requires the reflect registration the mirror needs
    /// to put the value on an entity. This type is the driver contract broken in exactly that
    /// way.
    #[derive(Clone, PartialEq, Debug, Component, Reflect)]
    struct UnmirrorableConfiguration(u8);

    /// A driver that establishes a configuration the mirror cannot project.
    struct UnmirrorableDriver;

    impl EndpointDriver for UnmirrorableDriver {
        type Configuration = UnmirrorableConfiguration;
        type Target = ();

        fn resolve_target(
            &mut self,
            _: &mut World,
            _: &TargetResolutionContext<'_>,
            _: &Self::Configuration,
        ) -> crate::TargetResolution<Self::Target> {
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
                .finish(DriverCompletion::Succeeded(Applied::DiffersFromDispatched(
                    UnmirrorableConfiguration(7),
                )));
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

    fn unit(device_key: DeviceKey) -> ReportedUnit {
        ReportedUnit {
            key:                    device_key,
            parent:                 ReportedParent::Root,
            attachment:             AttachmentPath::PlatformHasNoConcept,
            claim:                  Claim::NotApplicable,
            presence:               Presence::Present,
            brightness:             Vec::new(),
            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
        }
    }

    fn reported_record(reported_unit: &ReportedUnit) -> DeviceRecord {
        let mut capabilities = Capabilities::new();
        for brightness in &reported_unit.brightness {
            capabilities.add(Brightness(*brightness));
        }
        DeviceRecord {
            reported_as: ReportedAs::Keyed(reported_unit.key.clone()),
            parent: reported_unit.parent.clone(),
            presence: reported_unit.presence,
            claim: reported_unit.claim.clone(),
            capabilities,
            serial: ReportedSerial::NotExposedByUnit,
            platform_device_handle: reported_unit.platform_device_handle.clone(),
            attachment: reported_unit.attachment.clone(),
            descriptor: DeviceDescriptor::PlatformReportedNothing,
        }
    }

    fn slot(value: &str) -> AttachmentPath {
        AttachmentPath::Reported(ReportedId::new(value).expect("test slot is well formed"))
    }

    fn synthesized_key(digest: u64) -> DeviceKey {
        DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Synthesized {
                digest: Digest::new(digest),
            },
        }
    }

    fn authored_key(value: &str) -> DeviceKey {
        DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Authored {
                value: AuthoredId::new(value).expect("test authored id is well formed"),
            },
        }
    }

    /// Register a reporter whose whole set the returned handle rewrites.
    fn add_set_reporter(
        app: &mut App,
        units: Vec<ReportedUnit>,
        coverage: ReporterCoverage,
    ) -> Arc<Mutex<Vec<ReportedUnit>>> {
        registered_set_reporter(app, units, coverage, every_frame()).1
    }

    /// Register a rewritable reporter and keep its handle, for a test that has to age its retained
    /// set deliberately.
    fn registered_set_reporter(
        app: &mut App,
        units: Vec<ReportedUnit>,
        coverage: ReporterCoverage,
        cadence: DiscoveryCadence,
    ) -> (ReporterId, Arc<Mutex<Vec<ReportedUnit>>>) {
        let reported_units = Arc::new(Mutex::new(units));
        let reporter = app.add_device_reporter(
            SetReporter(Arc::clone(&reported_units)),
            ReporterRegistration::required(cadence, coverage, std::time::Duration::from_secs(10)),
        );

        (reporter, reported_units)
    }

    /// Absence authority over the whole test identity scheme, so an omission from a fresh complete
    /// scan is evidence the unit is gone rather than evidence about nothing.
    fn establishes_absence() -> ReporterCoverage {
        ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
            CoveredDeviceIdentitySpace::AllKeysOfKind {
                kind: DeviceKind::Display,
            },
        ))
    }

    fn rewrite(reported_units: &Arc<Mutex<Vec<ReportedUnit>>>, units: Vec<ReportedUnit>) {
        *reported_units
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = units;
    }

    fn verdict_of(devices: &Devices, device_key: &DeviceKey) -> Option<IdentityVerdict> {
        let device_id = resolved(devices, device_key)?;
        match devices.state(device_id) {
            DeviceStateLookup::Retained(state) => Some(state.verdict.clone()),
            DeviceStateLookup::Retired => None,
        }
    }

    fn device_entity_of(app: &App, device_key: &DeviceKey) -> Option<Entity> {
        let devices = app.world().resource::<Devices>();
        let device_id = resolved(devices, device_key)?;
        match devices.entity(device_id) {
            DeviceEntityLookup::Projected(entity) => Some(entity),
            DeviceEntityLookup::NotProjected => None,
        }
    }

    /// Run one reconcile pass and the projection that follows it, against the frame clock the test
    /// chooses.
    ///
    /// Separate from `reconcile_once`, which measures the merge alone against an empty inventory:
    /// this one reads the app's authored inventory and applies the pass's changes, so a test can
    /// judge entities, links, capture, and connection conclusions after aging a retained set.
    fn reconcile_and_project(app: &mut App, clock: FrameClockReading) {
        drop(reconcile_and_project_changes(app, clock));
    }

    /// Run one reconcile pass and return the availability edges before projecting its changes.
    fn reconcile_and_project_changes(
        app: &mut App,
        clock: FrameClockReading,
    ) -> Vec<DeviceAvailabilityChange> {
        let runtime_clock = *app.world().resource::<RiggingRuntimeClock>();
        let observed_at = match clock {
            FrameClockReading::Measurable(observed_at) => observed_at,
            FrameClockReading::NotYetAdvanced => {
                runtime_clock.instant_at(RiggingRuntimeTime::from_elapsed(Duration::ZERO))
            },
        };
        let world = app.world_mut();
        let mut availability_changes = Vec::new();
        world.resource_scope::<Reporters, _>(|world, mut reporters| {
            world.resource_scope::<Devices, _>(|world, mut devices| {
                world.resource_scope::<RiggingRevision, _>(|world, mut rigging_revision| {
                    world.resource_scope::<RiggingLimits, _>(|world, rigging_limits| {
                        world.resource_scope::<RegisteredSchemes, _>(
                            |world, registered_schemes| {
                                world.resource_scope::<HardwareInventory, _>(
                                    |world, hardware_inventory| {
                                        let freshness_lease = FreshnessLease {
                                            rigging_limits: &rigging_limits,
                                            clock,
                                        };
                                        let reconcile_work = reconcile_work(
                                            &reporters,
                                            &devices,
                                            &registered_schemes,
                                            &hardware_inventory,
                                            world.resource::<Bindings>(),
                                            freshness_lease,
                                            &rigging_limits,
                                            observed_at,
                                            runtime_clock,
                                            devices.departure_grace_due(
                                                runtime_clock.time_at(observed_at),
                                            ),
                                        );
                                        if let ReconcilePass::Merged(replacement) =
                                            reconcile_devices(
                                                &mut reporters,
                                                &mut devices,
                                                &mut rigging_revision,
                                                reconcile_work,
                                            )
                                        {
                                            availability_changes =
                                                replacement.changes.availability.clone();
                                            *world.resource_mut::<ReconciledDeviceChanges>() =
                                                replacement.changes;
                                        }
                                    },
                                );
                            },
                        );
                    });
                });
            });
        });
        project_device_entities(world);
        availability_changes
    }

    fn now() -> FrameClockReading { FrameClockReading::Measurable(Instant::now()) }

    #[test]
    fn a_unique_key_takes_the_verdict_its_identity_source_supports() {
        let mut app = app_with_scheme();
        let reported = key("panel-a");
        let synthesized = synthesized_key(0x1234_5678);
        let authored = authored_key("studio-panel");
        add_set_reporter(
            &mut app,
            vec![
                unit(reported.clone()),
                unit(synthesized.clone()),
                unit(authored.clone()),
            ],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert_eq!(
            verdict_of(devices, &reported),
            Some(IdentityVerdict::Proven)
        );
        assert_eq!(
            verdict_of(devices, &synthesized),
            Some(IdentityVerdict::Presumed)
        );
        assert_eq!(
            verdict_of(devices, &authored),
            Some(IdentityVerdict::Authored)
        );
    }

    #[test]
    fn an_authored_entry_no_reporter_names_produces_no_device_entity_or_verdict() {
        let mut app = app_with_scheme();
        let unreported = authored_key("dark-panel");
        app.world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  unreported.clone(),
                mode: ConfiguredDeviceMode::Managed,
                name: ConfiguredDeviceName::NeverDerived,
            });
        add_set_reporter(
            &mut app,
            vec![unit(key("panel-a"))],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        assert_eq!(
            app.world().resource::<Devices>().resolve(&unreported),
            DeviceResolution::NotResolved
        );
        assert_eq!(device_entity_of(&app, &unreported), None);
        assert_eq!(
            verdict_of(app.world().resource::<Devices>(), &unreported),
            None
        );
    }

    #[test]
    fn a_key_duplicated_within_one_scan_is_unverified_rather_than_proven() {
        let mut app = app_with_scheme();
        let duplicated = key("twin-webcam");
        add_set_reporter(
            &mut app,
            vec![unit(duplicated.clone()), unit(duplicated.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        assert_eq!(
            verdict_of(app.world().resource::<Devices>(), &duplicated),
            Some(IdentityVerdict::Unverified(
                crate::UnverifiedReason::NotUniqueInScan
            ))
        );
    }

    #[test]
    fn two_units_that_both_reported_no_attachment_are_not_displaced_onto_each_other() {
        let mut app = app_with_scheme();
        let departing = key("panel-a");
        let arriving = key("panel-b");
        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(departing)],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        rewrite(&reported_units, vec![unit(arriving.clone())]);
        run_until_reconciled(&mut app);

        // Both records carry `AttachmentPath::PlatformHasNoConcept`, which compares equal to
        // itself: joining on it would fuse two units that each reported no location at all.
        assert_eq!(
            verdict_of(app.world().resource::<Devices>(), &arriving),
            Some(IdentityVerdict::Proven)
        );
    }

    #[test]
    fn a_unit_arriving_into_a_departed_reported_slot_is_displaced_by_the_key_that_left()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let departing = key("panel-a");
        let arriving = key("panel-b");
        let occupied_slot = slot("usb-3-port-1");
        let mut departing_unit = unit(departing.clone());
        departing_unit.attachment = occupied_slot.clone();
        let mut arriving_unit = unit(arriving.clone());
        arriving_unit.attachment = occupied_slot;
        bind_displaced_role(&mut app, departing.clone())?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![departing_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        rewrite(&reported_units, vec![arriving_unit]);
        run_until_reconciled(&mut app);

        assert_eq!(
            verdict_of(app.world().resource::<Devices>(), &arriving),
            Some(IdentityVerdict::Displaced { saved: departing })
        );

        Ok(())
    }

    /// Bind one role to the key a displacement is about, so the debt has a human to owe an answer
    /// to.
    ///
    /// Without a bound role no question can be raised about the saved key at all, and
    /// `crate::identity_decisions` discharges a debt nobody can be asked about rather than pinning
    /// the unit for the life of the process.
    fn bind_displaced_role(app: &mut App, device: DeviceKey) -> Result<RoleKey, Box<dyn Error>> {
        let role = RoleKey::new("displaced-panel")?;
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::new(AtomicUsize::new(0))));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(panel_binding(role.clone(), device, driver))?;

        Ok(role)
    }

    #[test]
    fn a_displaced_verdict_stays_until_a_human_decides_it() -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let departing = key("panel-a");
        let arriving = key("panel-b");
        let occupied_slot = slot("usb-3-port-1");
        let mut departing_unit = unit(departing.clone());
        departing_unit.attachment = occupied_slot.clone();
        let mut arriving_unit = unit(arriving.clone());
        arriving_unit.attachment = occupied_slot;
        bind_displaced_role(&mut app, departing.clone())?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![departing_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        rewrite(&reported_units, vec![arriving_unit]);
        run_until_reconciled(&mut app);
        // Every later pass sees one healthy unit and no departed slot, which is exactly the
        // evidence that would recompute this verdict to `Proven` and authorize a unit nobody
        // accepted.
        for _ in 0..3 {
            run_until_reconciled(&mut app);
        }

        let devices = app.world().resource::<Devices>();
        assert_eq!(
            verdict_of(devices, &arriving),
            Some(IdentityVerdict::Displaced { saved: departing })
        );
        let DeviceResolution::Resolved(device_id) = devices.resolve(&arriving) else {
            panic!("the arriving unit stays retained across the later passes");
        };
        assert!(devices.authorize_service(device_id).is_err());

        Ok(())
    }

    #[test]
    fn a_duplicated_key_stays_recomputed_while_a_displacement_is_carried()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let departing = key("panel-a");
        let arriving = key("panel-b");
        let occupied_slot = slot("usb-3-port-1");
        let mut departing_unit = unit(departing.clone());
        departing_unit.attachment = occupied_slot.clone();
        let mut arriving_unit = unit(arriving.clone());
        arriving_unit.attachment = occupied_slot;
        bind_displaced_role(&mut app, departing.clone())?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![departing_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        rewrite(&reported_units, vec![arriving_unit.clone()]);
        run_until_reconciled(&mut app);
        rewrite(
            &reported_units,
            vec![arriving_unit.clone(), arriving_unit.clone()],
        );
        run_until_reconciled(&mut app);

        // The scan itself re-establishes a duplicate every pass, so the observation has to be able
        // to take over from the carried verdict and to clear when the scan stops showing it.
        assert_eq!(
            verdict_of(app.world().resource::<Devices>(), &arriving),
            Some(IdentityVerdict::Unverified(
                UnverifiedReason::NotUniqueInScan
            ))
        );

        rewrite(&reported_units, vec![arriving_unit]);
        run_until_reconciled(&mut app);

        // The duplicate cleared, and what is underneath it is still the displacement nobody
        // decided: a duplicate episode that consumed the carried verdict would leave this pass
        // reporting `Proven` and authorizing a unit a human never accepted.
        let devices = app.world().resource::<Devices>();
        assert_eq!(
            verdict_of(devices, &arriving),
            Some(IdentityVerdict::Displaced { saved: departing })
        );
        let DeviceResolution::Resolved(device_id) = devices.resolve(&arriving) else {
            panic!("the arriving unit stays retained across the duplicate episode");
        };
        assert!(devices.authorize_service(device_id).is_err());

        Ok(())
    }

    #[test]
    fn an_authored_key_arriving_into_a_reported_slot_is_displaced_not_wrong()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let departing = key("panel-a");
        let arriving = authored_key("studio-panel");
        let occupied_slot = slot("usb-3-port-1");
        let mut departing_unit = unit(departing.clone());
        departing_unit.attachment = occupied_slot.clone();
        let mut arriving_unit = unit(arriving.clone());
        arriving_unit.attachment = occupied_slot;
        bind_displaced_role(&mut app, departing.clone())?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![departing_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        rewrite(&reported_units, vec![arriving_unit]);
        run_until_reconciled(&mut app);

        // Nobody authored the slot's saved key, so no human assignment is being contradicted: the
        // arriving unit's own key being authored says nothing about the unit that left.
        assert_eq!(
            verdict_of(app.world().resource::<Devices>(), &arriving),
            Some(IdentityVerdict::Displaced { saved: departing })
        );

        Ok(())
    }

    #[test]
    fn a_unit_arriving_into_a_departed_authored_slot_reports_the_saved_key_as_the_wrong_unit()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let departing = authored_key("studio-panel");
        let arriving = key("panel-b");
        let occupied_slot = slot("usb-3-port-1");
        let mut departing_unit = unit(departing.clone());
        departing_unit.attachment = occupied_slot.clone();
        let mut arriving_unit = unit(arriving.clone());
        arriving_unit.attachment = occupied_slot;
        bind_displaced_role(&mut app, departing.clone())?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![departing_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        rewrite(&reported_units, vec![arriving_unit]);
        run_until_reconciled(&mut app);

        // The payload is the authored key a human assigned to this slot, which is what makes the
        // arriving unit's different identity a conflict to resolve rather than a new device.
        assert_eq!(
            verdict_of(app.world().resource::<Devices>(), &arriving),
            Some(IdentityVerdict::WrongUnit {
                authored: departing,
            })
        );

        Ok(())
    }

    #[test]
    fn a_slot_match_under_a_different_parent_leaves_the_arriving_unit_proven() {
        let mut app = app_with_scheme();
        let parent = key("dock-a");
        let departing = key("panel-a");
        let arriving = key("panel-b");
        let occupied_slot = slot("usb-3-port-1");
        let mut departing_unit = unit(departing);
        departing_unit.attachment = occupied_slot.clone();
        departing_unit.parent = ReportedParent::ChildOf(parent.clone());
        let mut arriving_unit = unit(arriving.clone());
        arriving_unit.attachment = occupied_slot;
        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(parent.clone()), departing_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        rewrite(&reported_units, vec![unit(parent), arriving_unit]);
        run_until_reconciled(&mut app);

        assert_eq!(
            verdict_of(app.world().resource::<Devices>(), &arriving),
            Some(IdentityVerdict::Proven)
        );
    }

    #[test]
    fn authored_connection_moves_from_not_observed_through_present_absent_and_unreachable() {
        let mut app = app_with_scheme();
        let authored = key("panel-a");
        app.world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  authored.clone(),
                mode: ConfiguredDeviceMode::Managed,
                name: ConfiguredDeviceName::NeverDerived,
            });

        assert_eq!(
            app.world()
                .resource::<HardwareInventory>()
                .connection(&authored),
            Ok(ConfiguredDeviceConnection::NotObserved)
        );

        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(authored.clone())],
            establishes_absence(),
        );
        run_until_reconciled(&mut app);

        assert_eq!(
            app.world()
                .resource::<HardwareInventory>()
                .connection(&authored),
            Ok(ConfiguredDeviceConnection::Present)
        );

        rewrite(&reported_units, Vec::new());
        run_until_reconciled(&mut app);

        assert_eq!(
            app.world()
                .resource::<HardwareInventory>()
                .connection(&authored),
            Ok(ConfiguredDeviceConnection::Absent)
        );
        // Grace revokes authorization while retaining the device entity.
        assert!(device_entity_of(&app, &authored).is_some());
        advance_past_departure_grace(&mut app);
        assert_eq!(device_entity_of(&app, &authored), None);
    }

    #[test]
    fn evidence_that_aged_past_its_lease_reports_an_authored_key_unreachable_not_absent() {
        let mut app = app_with_scheme();
        let authored = key("panel-a");
        app.world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  authored.clone(),
                mode: ConfiguredDeviceMode::Managed,
                name: ConfiguredDeviceName::NeverDerived,
            });
        let (reporter, _reported_units) = registered_set_reporter(
            &mut app,
            vec![unit(authored.clone())],
            establishes_absence(),
            DiscoveryCadence::Periodic {
                interval: Duration::from_secs(5),
            },
        );
        run_until_reconciled(&mut app);

        assert_eq!(
            app.world()
                .resource::<HardwareInventory>()
                .connection(&authored),
            Ok(ConfiguredDeviceConnection::Present)
        );

        // A set that aged out withdrew its evidence rather than reporting an absence, so the
        // conclusion weakens to unreachable instead of concluding the unit left.
        let report_grace = app.world().resource::<RiggingLimits>().report_grace;
        app.world_mut()
            .resource_mut::<Reporters>()
            .backdate_completion(reporter, report_grace + Duration::from_mins(10));
        reconcile_and_project(&mut app, now());

        assert_eq!(
            app.world()
                .resource::<HardwareInventory>()
                .connection(&authored),
            Ok(ConfiguredDeviceConnection::Unreachable)
        );
    }

    #[test]
    fn a_matching_evidence_only_reporter_never_establishes_absence() {
        let mut app = app_with_scheme();
        let authored = key("panel-a");
        app.world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  authored.clone(),
                mode: ConfiguredDeviceMode::Managed,
                name: ConfiguredDeviceName::NeverDerived,
            });
        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(authored.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );
        run_until_reconciled(&mut app);
        rewrite(&reported_units, Vec::new());
        run_until_reconciled(&mut app);

        assert_eq!(
            app.world()
                .resource::<HardwareInventory>()
                .connection(&authored),
            Ok(ConfiguredDeviceConnection::NotObserved)
        );
    }

    #[test]
    fn the_most_restrictive_claim_wins_and_refuses_service_on_a_co_reported_device() {
        let mut app = app_with_scheme();
        let contested = key("shared-camera");
        let mut free_unit = unit(contested.clone());
        free_unit.claim = Claim::Free;
        let mut contended_unit = unit(contested.clone());
        contended_unit.claim = Claim::Contended {
            holder: ClaimHolder::Named(String::from("another capture application")),
        };
        add_set_reporter(
            &mut app,
            vec![free_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );
        add_set_reporter(
            &mut app,
            vec![contended_unit],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);

        let devices = app.world().resource::<Devices>();
        assert!(matches!(
            claim_of(devices, &contested),
            Some(Claim::Contended { .. })
        ));
        let device_id = resolved(devices, &contested).expect("the co-reported device resolves");
        assert!(matches!(
            devices.authorize_service(device_id).err(),
            Some(crate::ApplyAuthorizationError::ClaimUnavailable { .. })
        ));
        let entity = device_entity_of(&app, &contested).expect("a retained device is mirrored");
        assert!(
            app.world()
                .get::<crate::devices::PresentWithUsableClaim>(entity)
                .is_none()
        );
    }

    #[test]
    fn a_reconciled_device_gains_an_entity_and_its_departure_despawns_it() {
        let mut app = app_with_scheme();
        app.world_mut()
            .resource_mut::<RiggingLimits>()
            .departure_grace = Duration::ZERO;
        let panel = key("panel-a");
        let reported_units =
            add_set_reporter(&mut app, vec![unit(panel.clone())], establishes_absence());

        run_until_reconciled(&mut app);

        let entity = device_entity_of(&app, &panel).expect("a retained device is mirrored");
        assert_eq!(app.world().get::<DeviceKey>(entity), Some(&panel));
        assert_eq!(
            app.world().get::<IdentityVerdict>(entity),
            Some(&IdentityVerdict::Proven)
        );
        assert!(app.world().get::<crate::Device>(entity).is_some());
        assert!(
            app.world()
                .get::<crate::devices::PresentWithUsableClaim>(entity)
                .is_some()
        );
        // The handle lives on the entity as `DeviceId` itself, never wrapped in a second type.
        assert!(app.world().get::<crate::DeviceId>(entity).is_some());

        rewrite(&reported_units, Vec::new());
        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);

        assert_eq!(
            app.world().resource::<Devices>().resolve(&panel),
            DeviceResolution::NotResolved
        );
        assert!(app.world().get_entity(entity).is_err());
    }

    #[test]
    fn a_reconcile_pass_links_a_binding_to_its_device_and_a_departure_removes_the_link()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        app.world_mut()
            .resource_mut::<RiggingLimits>()
            .departure_grace = Duration::ZERO;
        let panel = key("panel-a");
        let role = crate::RoleKey::new("primary-window")?;
        app.world_mut()
            .resource_mut::<crate::Bindings>()
            .register(crate::Binding {
                role:             role.clone(),
                endpoint:         crate::DeviceEndpoint {
                    device: panel.clone(),
                    id:     EndpointId::Whole,
                },
                driver:           DriverId(0),
                recovery:         RecoveryPolicy::Forget,
                retry:            RetryOn::NewRevision,
                on_abort:         crate::OnAbort::default(),
                on_loss:          crate::OnSessionLoss::default(),
                requested:        crate::RequestedConfiguration::new(()),
                last_known_good:  crate::LastKnownGoodConfiguration::default(),
                apply_deadline:   ApplyDeadline::ProcessDefault,
                flow_expectation: FlowExpectation::NotMonitored,
            })?;
        let reported_units =
            add_set_reporter(&mut app, vec![unit(panel.clone())], establishes_absence());

        run_until_reconciled(&mut app);

        let binding_entity = app
            .world()
            .resource::<crate::Bindings>()
            .role_entity(&role)?;
        let device_entity = device_entity_of(&app, &panel).expect("a retained device is mirrored");
        assert_eq!(
            app.world()
                .get::<crate::ResolvedToDevice>(binding_entity)
                .map(|link| link.device()),
            Some(device_entity)
        );
        assert_eq!(
            app.world()
                .get::<crate::ResolvedBindings>(device_entity)
                .map(|resolved_bindings| resolved_bindings.iter().collect::<Vec<_>>()),
            Some(vec![binding_entity])
        );

        rewrite(&reported_units, Vec::new());
        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);

        assert!(
            app.world()
                .get::<crate::ResolvedToDevice>(binding_entity)
                .is_none()
        );
        assert!(app.world().get_entity(binding_entity).is_ok());
        assert!(
            app.world()
                .resource::<crate::Bindings>()
                .binding(&role)
                .is_ok()
        );

        Ok(())
    }

    /// Build one binding whose endpoint names a reported device and whose driver reads back a
    /// `PanelConfiguration`.
    fn panel_binding(role: RoleKey, device: DeviceKey, driver: DriverId) -> Binding {
        Binding {
            role,
            endpoint: DeviceEndpoint {
                device,
                id: EndpointId::Whole,
            },
            driver,
            recovery: RecoveryPolicy::Forget,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            requested: RequestedConfiguration::new(PanelConfiguration(3)),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline: ApplyDeadline::ProcessDefault,
            flow_expectation: FlowExpectation::NotMonitored,
        }
    }

    /// Drive one registered role from waiting to ready through a completed apply.
    fn reach_ready(app: &mut App, role: &RoleKey) {
        let startup = app.world().resource::<Time<Real>>().startup();
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .update_with_instant(startup + Duration::from_millis(1));
        for _ in 0..16 {
            app.update();
            if app
                .world()
                .resource::<Bindings>()
                .projected_status(role)
                .is_ok_and(|status| matches!(status, crate::RoleStatusView::Established { .. }))
            {
                return;
            }
        }
        panic!("the role did not establish within sixteen updates");
    }

    #[test]
    fn a_ready_managed_role_reads_its_configuration_back_and_mirrors_it_without_rewriting()
    -> Result<(), Box<dyn Error>> {
        /// One entry per frame in which the mirrored configuration component was written.
        #[derive(Default, Resource)]
        struct MirrorWrites(usize);

        fn count_mirror_writes(
            mirrored: Query<(), Changed<PanelConfiguration>>,
            mut mirror_writes: ResMut<MirrorWrites>,
        ) {
            mirror_writes.0 += mirrored.iter().count();
        }

        let mut app = app_with_scheme();
        app.init_resource::<MirrorWrites>()
            .add_systems(PostUpdate, count_mirror_writes);
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(panel_binding(role.clone(), panel.clone(), driver))?;
        add_set_reporter(
            &mut app,
            vec![unit(panel.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        let binding_entity = app.world().resource::<Bindings>().role_entity(&role)?;
        // A waiting role is not a safe readback opportunity, and an unestablished value mirrors
        // nothing onto the entity.
        assert_eq!(captures.load(Ordering::Relaxed), 0);
        assert_eq!(app.world().get::<PanelConfiguration>(binding_entity), None);

        reach_ready(&mut app, &role);
        app.update();

        assert_eq!(captures.load(Ordering::Relaxed), 1);
        assert_eq!(
            app.world().get::<PanelConfiguration>(binding_entity),
            Some(&PanelConfiguration(7))
        );
        assert_eq!(app.world().resource::<MirrorWrites>().0, 1);

        // The driver reads the same value back on the next pass, so the mirror writes nothing and
        // every downstream change filter stays quiet.
        app.update();

        assert_eq!(app.world().resource::<MirrorWrites>().0, 1);

        // The mirror is a projection of kernel state, so an outside write through reflection is
        // replaced rather than adopted as the value last known to work.
        app.world_mut()
            .entity_mut(binding_entity)
            .insert(PanelConfiguration(99));
        app.update();

        assert_eq!(
            app.world().get::<PanelConfiguration>(binding_entity),
            Some(&PanelConfiguration(7))
        );

        // An owed restoration closes the window: reading the endpoint back now would record the
        // state the departure left behind as the value last known to work.
        app.world_mut()
            .resource_mut::<Bindings>()
            .set_waiting_work(&role, WaitingWork::RestorationOwed);
        let captures_before_restoration_owed = captures.load(Ordering::Relaxed);
        app.update();

        assert_eq!(
            captures.load(Ordering::Relaxed),
            captures_before_restoration_owed
        );

        // An offline authored entry may still be discovered passively, but no driver call may
        // touch it.
        app.world_mut()
            .resource_mut::<Bindings>()
            .set_waiting_work(&role, WaitingWork::Nothing);
        app.world_mut()
            .resource_mut::<HardwareInventory>()
            .configure(ConfiguredDevice {
                key:  panel,
                mode: ConfiguredDeviceMode::Offline,
                name: ConfiguredDeviceName::NeverDerived,
            });
        let captures_before_offline = captures.load(Ordering::Relaxed);
        app.update();

        assert_eq!(captures.load(Ordering::Relaxed), captures_before_offline);

        Ok(())
    }

    /// Build one binding whose driver reads back a configuration the mirror cannot project.
    fn unmirrorable_binding(role: RoleKey, device: DeviceKey, driver: DriverId) -> Binding {
        Binding {
            requested: RequestedConfiguration::new(UnmirrorableConfiguration(3)),
            ..panel_binding(role, device, driver)
        }
    }

    #[test]
    fn a_driver_configuration_without_component_reflection_reports_a_contract_error()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(UnmirrorableDriver);
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(unmirrorable_binding(role.clone(), panel.clone(), driver))?;
        add_set_reporter(
            &mut app,
            vec![unit(panel)],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        reach_ready(&mut app, &role);
        app.update();

        let binding_entity = app.world().resource::<Bindings>().role_entity(&role)?;
        let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
        let type_registry = app_type_registry.read();
        let bindings = app.world().resource::<Bindings>();
        // The readback established a value, so the mirror reached the driver's own type rather than
        // skipping the role for having nothing to project.
        let LastKnownGoodConfiguration::DiffersFromDispatched(configuration) =
            &bindings.binding(&role)?.last_known_good
        else {
            return Err("a ready managed role establishes its configuration".into());
        };
        let Err(CapabilityAttachError::NotAComponent { type_path }) =
            reflect_component_for(configuration.as_partial_reflect(), &type_registry)
        else {
            return Err("a configuration without component reflection is a contract error".into());
        };

        assert_eq!(
            type_path,
            "hana_rigging::reconcile::tests::UnmirrorableConfiguration"
        );
        assert!(planned_configuration_mirrors(app.world(), &type_registry).is_empty());
        assert_eq!(
            app.world().get::<UnmirrorableConfiguration>(binding_entity),
            None
        );
        drop(type_registry);

        Ok(())
    }

    /// One binding whose requested configuration is not the registered driver's configuration
    /// type: `CountingCaptureDriver` accepts `PanelConfiguration`, and this hands it an
    /// `UnmirrorableConfiguration`.
    fn mistyped_binding(role: RoleKey, device: DeviceKey, driver: DriverId) -> Binding {
        Binding {
            requested: RequestedConfiguration::new(UnmirrorableConfiguration(3)),
            ..panel_binding(role, device, driver)
        }
    }

    /// The surviving contract-failure path, driven through the real lifecycle.
    ///
    /// No public API can make a driver return `Err`, so the only way to reach
    /// `DriverContractError` is the erased boundary's own downcast. A configuration of the wrong
    /// type is the reachable case, and it has to be refused before any driver method runs: a
    /// mismatch that slipped through would establish a session against a value the driver never
    /// agreed to.
    #[test]
    fn a_configuration_of_the_wrong_type_is_refused_before_any_driver_method_runs()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(mistyped_binding(role.clone(), panel.clone(), driver))?;
        add_set_reporter(
            &mut app,
            vec![unit(panel)],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        let startup = app.world().resource::<Time<Real>>().startup();
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .update_with_instant(startup + Duration::from_millis(1));
        for _ in 0..16 {
            app.update();
        }

        let status = app.world().resource::<Bindings>().projected_status(&role)?;
        let RoleStatusView::Waiting(WaitingStatusView::DriverRepair { error, .. }) = &status else {
            return Err(format!(
                "a mistyped configuration must leave the role waiting on driver repair, not {status:?}"
            )
            .into());
        };
        assert_eq!(
            error,
            &crate::DriverContractFailureView::ConfigurationTypeMismatch {
                expected_configuration: "hana_rigging::reconcile::tests::PanelConfiguration"
                    .to_owned(),
                received_configuration: "hana_rigging::reconcile::tests::UnmirrorableConfiguration"
                    .to_owned(),
            }
        );
        assert_eq!(
            captures.load(Ordering::Relaxed),
            0,
            "the driver's own methods must never see a configuration it cannot accept"
        );
        assert!(
            app.world()
                .resource::<Bindings>()
                .established_sessions()
                .is_empty(),
            "a refused dispatch establishes no session"
        );

        Ok(())
    }

    #[test]
    fn a_role_bound_to_an_unreported_device_stays_waiting_until_service_is_authorized()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(panel_binding(role.clone(), panel.clone(), driver))?;
        let reported_units = add_set_reporter(&mut app, Vec::new(), establishes_absence());

        run_until_reconciled(&mut app);

        // No reporter named the endpoint, so there is no handle to authorize, nothing moves the
        // role out of waiting, and no driver call reaches hardware nobody has seen.
        assert!(matches!(
            app.world().resource::<Devices>().resolve(&panel),
            DeviceResolution::NotResolved
        ));
        assert!(matches!(
            app.world().resource::<Bindings>().projected_status(&role)?,
            crate::RoleStatusView::Waiting(_)
        ));
        assert_eq!(captures.load(Ordering::Relaxed), 0);

        rewrite(&reported_units, vec![unit(panel.clone())]);
        run_until_reconciled(&mut app);

        // The same role reaches ready only through a permit the reconciled device issued.
        let devices = app.world().resource::<Devices>();
        let DeviceResolution::Resolved(device_id) = devices.resolve(&panel) else {
            return Err("a reported key resolves after reconciliation".into());
        };
        devices.authorize_service(device_id)?;
        reach_ready(&mut app, &role);

        assert!(matches!(
            app.world().resource::<Bindings>().projected_status(&role)?,
            crate::RoleStatusView::Established { .. }
        ));

        Ok(())
    }

    #[test]
    fn a_saved_device_entity_carries_no_process_local_handle() -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let panel = key("panel-a");
        add_set_reporter(
            &mut app,
            vec![unit(panel.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        let device_entity = device_entity_of(&app, &panel).expect("a retained device is mirrored");
        let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
        let type_registry = app_type_registry.read();
        // `DeviceId` reflects opaquely and registers no serializer, so a save that kept the handle
        // cannot be written at all.
        assert!(
            DynamicWorldBuilder::from_world(app.world(), &type_registry)
                .extract_entity(device_entity)
                .build()
                .serialize(&type_registry)
                .is_err()
        );

        let serialized = DynamicWorldBuilder::from_world(app.world(), &type_registry)
            .deny_component::<DeviceId>()
            .extract_entity(device_entity)
            .build()
            .serialize(&type_registry)?;
        drop(type_registry);

        // The durable key crosses the storage boundary; the handle the registry issued this process
        // does not, so a later run cannot read a saved file as if it named a live device.
        assert!(serialized.contains("DeviceKey"));
        assert!(!serialized.contains("DeviceId"));

        Ok(())
    }

    /// One entry per frame in which a rescanned capability component was written.
    #[derive(Default, Resource)]
    struct CapabilityWrites(usize);

    fn count_capability_writes(
        rescanned: Query<(), Changed<Brightness>>,
        mut capability_writes: ResMut<CapabilityWrites>,
    ) {
        capability_writes.0 += rescanned.iter().count();
    }

    #[test]
    fn a_reporter_rescanning_an_unchanged_capability_writes_no_component() {
        let mut app = app_with_scheme();
        app.init_resource::<CapabilityWrites>()
            .add_systems(PostUpdate, count_capability_writes);
        let panel = key("streamdeck-xl");
        let mut reported = unit(panel.clone());
        reported.brightness = vec![50];
        let reported_units = add_set_reporter(
            &mut app,
            vec![reported.clone()],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        let device_entity =
            device_entity_of(&app, &panel).expect("a retained device owns an entity");
        assert_eq!(
            app.world().get::<Brightness>(device_entity),
            Some(&Brightness(50))
        );
        assert_eq!(app.world().resource::<CapabilityWrites>().0, 1);

        // The reporter keeps scanning on its own cadence and keeps declaring the same value.
        app.update();
        app.update();

        assert_eq!(app.world().resource::<CapabilityWrites>().0, 1);

        // A declaration that actually changed still reaches the entity.
        let mut brighter = reported;
        brighter.brightness = vec![90];
        rewrite(&reported_units, vec![brighter]);
        run_until_reconciled(&mut app);

        assert_eq!(
            app.world().get::<Brightness>(device_entity),
            Some(&Brightness(90))
        );
        assert_eq!(app.world().resource::<CapabilityWrites>().0, 2);
    }

    #[test]
    fn a_disputed_capability_settles_on_the_entity_instead_of_alternating() {
        let mut app = app_with_scheme();
        app.init_resource::<CapabilityWrites>()
            .add_systems(PostUpdate, count_capability_writes);
        let contested = key("streamdeck-xl");
        let mut dim = unit(contested.clone());
        dim.brightness = vec![50];
        let mut bright = unit(contested.clone());
        bright.brightness = vec![90];
        add_set_reporter(&mut app, vec![dim], ReporterCoverage::MatchingEvidenceOnly);
        let brighter_units = add_set_reporter(
            &mut app,
            vec![bright],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);

        // Neither reporter's value is established, and the kernel announces the disagreement rather
        // than picking a winner, so no value of the disputed type sits on the entity at all.
        let device_entity =
            device_entity_of(&app, &contested).expect("a retained device owns an entity");
        assert_eq!(app.world().get::<Brightness>(device_entity), None);

        let settled_writes = app.world().resource::<CapabilityWrites>().0;
        app.update();
        app.update();

        // Attaching the union would write one contributor's value and then the other's on every
        // pass, making a change filter true forever for a device that changed nothing.
        assert_eq!(app.world().resource::<CapabilityWrites>().0, settled_writes);

        let mut agreeing = unit(contested);
        agreeing.brightness = vec![50];
        rewrite(&brighter_units, vec![agreeing]);
        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);

        // Agreement is what establishes the value, so the component arrives when the dispute ends.
        assert_eq!(
            app.world().get::<Brightness>(device_entity),
            Some(&Brightness(50))
        );
    }

    /// One entry per frame in which the kernel state a settled frame must not touch was written.
    ///
    /// `Devices` is absent because no frame here is settled for it: the reporter this test drives
    /// re-completes its scan every frame, so every frame reaches the merge and rebuilds the
    /// reconciled set. The device set's idle-frame silence is covered where the reporter scans on
    /// demand — `frames_after_an_answer_write_neither_register` in `tests/scripted.rs`.
    #[derive(Default, Debug, PartialEq, Eq, Resource)]
    struct SettledFrameWrites {
        bindings:           usize,
        drivers:            usize,
        hardware_inventory: usize,
        identity_decisions: usize,
    }

    fn count_settled_frame_writes(
        bindings: Res<Bindings>,
        drivers: Res<Drivers>,
        hardware_inventory: Res<HardwareInventory>,
        identity_decisions: Res<crate::IdentityDecisions>,
        mut settled_frame_writes: ResMut<SettledFrameWrites>,
    ) {
        settled_frame_writes.bindings += usize::from(bindings.is_changed());
        settled_frame_writes.drivers += usize::from(drivers.is_changed());
        settled_frame_writes.hardware_inventory += usize::from(hardware_inventory.is_changed());
        settled_frame_writes.identity_decisions += usize::from(identity_decisions.is_changed());
    }

    #[test]
    fn an_established_configuration_closes_the_safe_capture_window() -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        app.init_resource::<SettledFrameWrites>()
            .add_systems(PostUpdate, count_settled_frame_writes);
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(panel_binding(role.clone(), panel.clone(), driver))?;
        add_set_reporter(
            &mut app,
            vec![unit(panel)],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        reach_ready(&mut app, &role);
        app.update();

        assert_eq!(captures.load(Ordering::Relaxed), 1);
        *app.world_mut().resource_mut::<SettledFrameWrites>() = SettledFrameWrites::default();
        for _ in 0..3 {
            app.update();
        }

        // The readback established the value it was there to learn, so every later frame is
        // settled: no driver call, and no mutable path opened to the resources dispatch
        // would reach through.
        assert_eq!(captures.load(Ordering::Relaxed), 1);
        assert_eq!(
            *app.world().resource::<SettledFrameWrites>(),
            SettledFrameWrites::default()
        );

        Ok(())
    }

    #[test]
    fn a_retained_unit_that_stops_being_present_owes_its_restoration() -> Result<(), Box<dyn Error>>
    {
        let mut app = app_with_scheme();
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(Binding {
                recovery: RecoveryPolicy::ReapplyOnReturn,
                ..panel_binding(role.clone(), panel.clone(), driver)
            })?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(panel.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        reach_ready(&mut app, &role);
        app.update();

        assert_eq!(captures.load(Ordering::Relaxed), 1);
        let device_entity =
            device_entity_of(&app, &panel).expect("a retained device owns an entity");

        let mut absent = unit(panel.clone());
        absent.presence = Presence::Absent;
        rewrite(&reported_units, vec![absent]);
        run_until_reconciled(&mut app);

        // The key is still in the reconciled set, so the unit keeps its handle, its entity, and the
        // binding's link to it: only the hardware went away.
        assert_eq!(
            app.world().resource::<Bindings>().waiting_work(&role),
            WaitingWork::RestorationOwed
        );
        assert!(app.world().get_entity(device_entity).is_ok());
        let binding_entity = app.world().resource::<Bindings>().role_entity(&role)?;
        assert!(
            app.world()
                .get::<crate::ResolvedToDevice>(binding_entity)
                .is_some()
        );
        assert!(matches!(
            app.world().resource::<Devices>().resolve(&panel),
            DeviceResolution::Resolved(_)
        ));

        Ok(())
    }

    /// What a departure did to the configuration the role had already established.
    ///
    /// Named rather than a `bool` because the assertions below read as a table: `Dropped` states
    /// which policy is the one that cannot be restarted by a request, which is the whole difference
    /// between the two holds and unreadable as `false`.
    #[derive(PartialEq, Eq, Debug)]
    enum SavedValue {
        Kept,
        Dropped,
    }

    /// Drive one binding under `recovery` to an established last-known-good value, make its unit
    /// absent, and read back what the departure recorded and whether the saved value survived.
    fn depart_under(recovery: RecoveryPolicy) -> Result<(WaitingWork, SavedValue), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(Binding {
                recovery,
                ..panel_binding(role.clone(), panel.clone(), driver)
            })?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(panel.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        reach_ready(&mut app, &role);
        app.update();
        assert_eq!(captures.load(Ordering::Relaxed), 1);

        let mut absent = unit(panel);
        absent.presence = Presence::Absent;
        rewrite(&reported_units, vec![absent]);
        run_until_reconciled(&mut app);

        let bindings = app.world().resource::<Bindings>();
        let saved_value = if matches!(
            bindings.binding(&role)?.last_known_good,
            LastKnownGoodConfiguration::DiffersFromDispatched(_)
        ) {
            SavedValue::Kept
        } else {
            SavedValue::Dropped
        };
        Ok((bindings.waiting_work(&role), saved_value))
    }

    #[test]
    fn each_recovery_policy_records_its_own_departure_work() -> Result<(), Box<dyn Error>> {
        // `ReapplyOnReturn` is the only policy that reapplies without being asked, so it is the
        // only one that owes a restoration. The other two hold the role for application code:
        // without that record a departed role falls back to `WaitingWork::Nothing`, reaches
        // `WaitingRole::Hardware`, and has its authored request dispatched automatically when
        // the unit returns.
        assert_eq!(
            depart_under(RecoveryPolicy::ReapplyOnReturn)?,
            (WaitingWork::RestorationOwed, SavedValue::Kept,)
        );
        // Which hold each owes is what makes the two answerable by different moves: the kept value
        // is what `ReapplyConfiguration` sends back, and `Forget` drops it at the departure so only
        // a fresh registration restarts the role.
        assert_eq!(
            depart_under(RecoveryPolicy::ReapplyOnRequest)?,
            (WaitingWork::ReapplyRequestOwed, SavedValue::Kept,)
        );
        assert_eq!(
            depart_under(RecoveryPolicy::Forget)?,
            (WaitingWork::RegistrationOwed, SavedValue::Dropped,)
        );

        Ok(())
    }

    #[test]
    fn a_first_apply_is_unaffected_by_the_default_recovery_policy() -> Result<(), Box<dyn Error>> {
        // `RecoveryPolicy` governs a saved value's treatment after a departure, never a role's
        // first apply. Under the `Forget` default a newly registered binding has no recorded
        // work, so it still reaches the view that dispatches its authored request.
        let mut app = app_with_scheme();
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::new(AtomicUsize::new(0))));
        let binding = panel_binding(role.clone(), panel.clone(), driver);
        assert_eq!(binding.recovery, RecoveryPolicy::default());
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(binding)?;
        add_set_reporter(
            &mut app,
            vec![unit(panel)],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        let bindings = app.world_mut().resource_mut::<Bindings>();
        assert_eq!(bindings.waiting_work(&role), WaitingWork::Nothing);
        assert!(matches!(
            bindings.projected_status(&role)?,
            crate::RoleStatusView::Waiting(_)
        ));

        Ok(())
    }

    #[test]
    fn a_configuration_that_returns_to_unestablished_loses_its_mirror() -> Result<(), Box<dyn Error>>
    {
        let mut app = app_with_scheme();
        let panel = key("panel-a");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(panel_binding(role.clone(), panel.clone(), driver))?;
        add_set_reporter(
            &mut app,
            vec![unit(panel.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);
        reach_ready(&mut app, &role);
        app.update();

        let binding_entity = app.world().resource::<Bindings>().role_entity(&role)?;
        assert_eq!(
            app.world().get::<PanelConfiguration>(binding_entity),
            Some(&PanelConfiguration(7))
        );

        // Replacing the binding is the shipped way an established value goes away while the role
        // stays registered and keeps the same binding entity.
        app.world_mut()
            .resource_mut::<Bindings>()
            .replace(panel_binding(role.clone(), panel, driver))?;
        app.update();

        // A mirror left behind would read as a configuration this kernel would put back, while the
        // authority it projects no longer holds one.
        assert_eq!(app.world().get::<PanelConfiguration>(binding_entity), None);

        // The removal is driven by the component the last write recorded, so a binding entity with
        // no mirror is not touched again on any later pass.
        app.world_mut()
            .entity_mut(binding_entity)
            .insert(PanelConfiguration(99));
        app.update();

        assert_eq!(
            app.world().get::<PanelConfiguration>(binding_entity),
            Some(&PanelConfiguration(99)),
            "a role that never had a mirror is left alone, so nothing removes an outside write"
        );

        Ok(())
    }

    #[test]
    fn a_reconcile_pass_moves_a_binding_entity_between_live_device_collections()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let source_panel = authored_key("studio-panel");
        let destination_panel = authored_key("edit-panel");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(panel_binding(role.clone(), source_panel.clone(), driver))?;
        add_set_reporter(
            &mut app,
            vec![unit(source_panel.clone()), unit(destination_panel.clone())],
            establishes_absence(),
        );

        run_until_reconciled(&mut app);

        let source_entity =
            device_entity_of(&app, &source_panel).expect("a retained device owns an entity");
        let destination_entity =
            device_entity_of(&app, &destination_panel).expect("a retained device owns an entity");
        let binding_entity = app.world().resource::<Bindings>().role_entity(&role)?;
        assert_eq!(
            resolved_binding_entities(&app, source_entity),
            vec![binding_entity]
        );
        assert!(resolved_binding_entities(&app, destination_entity).is_empty());

        // The role is re-authored onto the other endpoint while both units stay plugged in, so the
        // only thing that changes is which live device entity the durable endpoint resolves to.
        app.world_mut()
            .resource_mut::<Bindings>()
            .replace(panel_binding(role, destination_panel, driver))?;
        run_until_reconciled(&mut app);

        assert!(
            app.world().get_entity(source_entity).is_ok(),
            "the source device is still reported, so its entity survives the move"
        );
        assert_eq!(
            device_entity_of(&app, &source_panel),
            Some(source_entity),
            "the source device keeps the handle and entity it was projected onto"
        );
        assert!(
            !resolved_binding_entities(&app, source_entity).contains(&binding_entity),
            "a device the binding no longer resolves to must lose it from its reverse collection"
        );
        assert_eq!(
            resolved_binding_entities(&app, destination_entity),
            vec![binding_entity]
        );

        Ok(())
    }

    /// Read one device entity's reverse collection, treating a device no binding resolves to as
    /// owning none rather than as missing the component.
    fn resolved_binding_entities(app: &App, device_entity: Entity) -> Vec<Entity> {
        app.world()
            .get::<crate::ResolvedBindings>(device_entity)
            .map(|bindings_on_device| bindings_on_device.iter().collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_returning_unit_relinks_its_binding_entity_to_its_new_device_entity()
    -> Result<(), Box<dyn Error>> {
        let mut app = app_with_scheme();
        let first_panel = authored_key("studio-panel");
        let role = RoleKey::new("primary-window")?;
        let captures = Arc::new(AtomicUsize::new(0));
        let driver = app
            .world_mut()
            .resource_mut::<Drivers>()
            .add(CountingCaptureDriver(Arc::clone(&captures)));
        app.world_mut()
            .resource_mut::<Bindings>()
            .register(panel_binding(role.clone(), first_panel.clone(), driver))?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(first_panel.clone())],
            establishes_absence(),
        );

        run_until_reconciled(&mut app);

        let first_entity =
            device_entity_of(&app, &first_panel).expect("a retained device owns an entity");
        let binding_entity = app.world().resource::<Bindings>().role_entity(&role)?;
        assert_eq!(
            app.world()
                .get::<crate::ResolvedBindings>(first_entity)
                .map(|bindings_on_device| bindings_on_device.iter().collect::<Vec<_>>()),
            Some(vec![binding_entity])
        );

        // The authored endpoint outlives the unit that served it: the same durable key is reported
        // by a different physical unit, and only a reconcile pass re-resolves the link.
        rewrite(&reported_units, Vec::new());
        run_until_reconciled(&mut app);
        advance_past_departure_grace(&mut app);
        rewrite(&reported_units, vec![unit(first_panel.clone())]);
        run_until_reconciled(&mut app);

        let second_entity =
            device_entity_of(&app, &first_panel).expect("the returning unit owns an entity");
        assert_ne!(second_entity, first_entity);
        assert!(app.world().get_entity(first_entity).is_err());
        assert_eq!(
            app.world()
                .get::<crate::ResolvedBindings>(second_entity)
                .map(|bindings_on_device| bindings_on_device.iter().collect::<Vec<_>>()),
            Some(vec![binding_entity])
        );

        Ok(())
    }
}
