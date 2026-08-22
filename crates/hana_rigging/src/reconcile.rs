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
use bevy::log::warn;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::World;
use bevy::reflect::PartialReflect;
use bevy::reflect::TypeRegistry;
use bevy::time::Real;
use bevy::time::Time;

use crate::AttachmentPath;
use crate::BindingEntities;
use crate::BindingEntityLookup;
use crate::Bindings;
use crate::CapabilitiesDisputed;
use crate::Claim;
use crate::ConfiguredDeviceConnection;
use crate::ConfiguredDeviceMode;
use crate::Device;
use crate::DeviceId;
use crate::DeviceIdSource;
use crate::DeviceKey;
use crate::DeviceRecord;
use crate::DeviceResolution;
use crate::DeviceStateLookup;
use crate::Devices;
use crate::DiscoveryCadence;
use crate::HardwareInventory;
use crate::IdentityDecisionOwed;
use crate::IdentityVerdict;
use crate::LastKnownGoodConfiguration;
use crate::PlatformDeviceHandle;
use crate::Presence;
use crate::ReconciledDeviceState;
use crate::RecoveryPolicy;
use crate::RegisteredSchemes;
use crate::ReportedAs;
use crate::ReportedId;
use crate::ReportedParent;
use crate::ReporterId;
use crate::ResolvedToDevice;
use crate::RiggingLimits;
use crate::RiggingRevision;
use crate::RoleKey;
use crate::RoleState;
use crate::binding::CaptureDispatch;
use crate::binding::ConfigurationReadability;
use crate::binding::RoleView;
use crate::binding::WaitingWork;
use crate::capabilities;
use crate::capabilities::reflect_component_for;
use crate::devices::ConfiguredDeviceConnectionChange;
use crate::devices::DepartedDevice;
use crate::devices::DepartureAnnouncements;
use crate::devices::DeviceEntityLookup;
use crate::devices::DeviceRegisterChangeDetection;
use crate::devices::PresentWithUsableClaim;
use crate::devices::ReconciledDeviceChanges;
use crate::devices::ReconciledDeviceReplacement;
use crate::registration::Drivers;
use crate::registration::RegisteredReporter;
use crate::registration::ReporterContribution;
use crate::registration::Reporters;

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
    rigging_limits: Res<RiggingLimits>,
    registered_schemes: Res<RegisteredSchemes>,
    hardware_inventory: Res<HardwareInventory>,
    time: Res<Time<Real>>,
) {
    let freshness_lease = FreshnessLease {
        rigging_limits: &rigging_limits,
        clock:          FrameClockReading::from(&*time),
    };
    if reconcile_work(&reporters, &devices, freshness_lease) == ReconcileWork::Settled {
        // The queue is drained even here: a failure leaves every retained device alone, so the
        // record of it is the one thing a settled frame would otherwise let grow.
        drop(reporters.take_reporter_failures());
        return;
    }

    if let ReconcilePass::Merged(replacement) = reconcile_devices(
        &mut reporters,
        devices.bypass_change_detection(),
        &mut rigging_revision,
        &rigging_limits,
        &registered_schemes,
        &hardware_inventory,
        FrameClockReading::from(&*time),
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

/// Whether the freshness lease has anything left to apply to the devices already retained.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FreshnessLeaseWork {
    /// Every retained device already reflects how fresh its reporters are, so the lease alone is
    /// no reason to merge again.
    Settled,
    /// At least one still-reachable device belongs to a reporter that went silent past its lease.
    MarksDevicesUnreachable,
}

/// How long each reporter may stay silent this frame, so freshness is answered one reporter at a
/// time instead of by collecting the silent ones.
///
/// Answering per reporter rather than building a list is what keeps a settled frame free of
/// allocation while a reporter stays wedged: a permanently silent reporter is judged again every
/// frame, and judging it costs nothing.
#[derive(Clone, Copy)]
struct FreshnessLease<'a> {
    rigging_limits: &'a RiggingLimits,
    clock:          FrameClockReading,
}

impl FreshnessLease<'_> {
    /// Report how much one reporter's records still count as evidence that its devices are there.
    ///
    /// A reporter that has not completed a first scan is not silent — it has never promised
    /// anything yet — and a reporter that declared no cadence may stay quiet indefinitely without
    /// being late.
    fn freshness_of(&self, registered_reporter: &RegisteredReporter<'_>) -> ReporterFreshness {
        let FrameClockReading::Measurable(now) = self.clock else {
            return ReporterFreshness::Fresh;
        };
        let ReporterContribution::Completed { completed_at, .. } =
            &registered_reporter.contribution
        else {
            return ReporterFreshness::Fresh;
        };
        let ReporterFreshnessLease::Expires(lease) =
            freshness_lease(registered_reporter.cadence, self.rigging_limits)
        else {
            return ReporterFreshness::Fresh;
        };

        let silence = now.saturating_duration_since(*completed_at);
        if silence > lease {
            ReporterFreshness::SilentFor(silence)
        } else {
            ReporterFreshness::Fresh
        }
    }

    /// Report the freshness of the reporter a retained device names as a contributor.
    ///
    /// A contributor the registry no longer lists cannot be judged silent: the reporter is gone,
    /// not late, and the next merge drops it from the device anyway.
    fn freshness_by_id(&self, reporters: &Reporters, reporter: ReporterId) -> ReporterFreshness {
        reporters
            .registered_reporters()
            .find(|registered_reporter| registered_reporter.reporter == reporter)
            .map_or(ReporterFreshness::Fresh, |registered_reporter| {
                self.freshness_of(&registered_reporter)
            })
    }
}

/// How much a reporter's silence is worth trusting when its records reach the merge.
#[derive(Clone, Copy)]
enum ReporterFreshness {
    /// The reporter is inside its lease, so its records read as it reported them.
    Fresh,
    /// The reporter has been quiet this long past its lease, so the kernel stops treating its
    /// records as evidence that the devices are still reachable.
    SilentFor(Duration),
}

/// How silent a reporter may be before the kernel stops trusting its devices.
enum ReporterFreshnessLease {
    /// The reporter declared no cadence, so silence proves nothing: it runs when the application
    /// asks and can stay quiet indefinitely without being late.
    NoDeclaredCadence,
    /// The reporter promised a run within this interval, so exceeding it plus the configured grace
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
type CoReportView<'a> = HashMap<TypeId, Vec<&'a dyn PartialReflect>>;

fn reconcile_devices(
    reporters: &mut Reporters,
    devices: &mut Devices,
    rigging_revision: &mut RiggingRevision,
    rigging_limits: &RiggingLimits,
    registered_schemes: &RegisteredSchemes,
    hardware_inventory: &HardwareInventory,
    clock: FrameClockReading,
) -> ReconcilePass {
    // `DiscoveryStatus` holds each failure's error, and a failed scan retains the preceding whole
    // set and revision, so a failure changes no device state here. Draining keeps the queue from
    // growing; the attempt lifecycle aborts what a failure invalidates.
    drop(reporters.take_reporter_failures());

    // The lease feeds the merge instead of editing the merged result, because the merge rebuilds
    // every device from the retained sets: a presence written before it would be overwritten, and
    // a presence written after it would have to be re-derived on every following frame.
    let freshness_lease = FreshnessLease {
        rigging_limits,
        clock,
    };

    if reconcile_work(reporters, devices, freshness_lease) == ReconcileWork::Settled {
        return ReconcilePass::Settled;
    }
    let changed_reporters = reporters.take_changed_reporters();

    let reconciled_device_changes = ingest(
        reporters,
        devices,
        registered_schemes,
        hardware_inventory,
        freshness_lease,
    );

    if !changed_reporters.is_empty() {
        rigging_revision.advance();
    }

    ReconcilePass::Merged(reconciled_device_changes)
}

/// Whether this frame reaches the merge at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ReconcileWork {
    /// No reporter completed a scan and no lease has anything left to apply, so the retained
    /// device set already says what this frame would conclude.
    Settled,
    /// Either new evidence arrived or a lease expired, so the device set has to be rebuilt.
    Merges,
}

/// Decide whether the frame has reconcile work, without consuming any of the evidence that says so.
///
/// Separate from `reconcile_devices` so the answer is available before `Devices` is borrowed
/// mutably, and shared with it so the settled frame is defined in exactly one place.
fn reconcile_work(
    reporters: &Reporters,
    devices: &Devices,
    freshness_lease: FreshnessLease<'_>,
) -> ReconcileWork {
    if reporters.any_reporter_changed()
        || lease_work(devices, reporters, freshness_lease)
            == FreshnessLeaseWork::MarksDevicesUnreachable
    {
        return ReconcileWork::Merges;
    }

    ReconcileWork::Settled
}

/// Report whether an expired lease would still change any retained device.
///
/// This is what keeps a frame with no completed reporter free of work: a reporter that stayed
/// silent for an hour is judged once, and every following frame sees its devices already
/// unreachable and merges nothing.
///
/// A device only has lease work left when *every* contributor to it has gone silent. A silent
/// reporter withdraws its evidence rather than reporting absence, so a device another reporter
/// still reports keeps the presence that reporter gives it.
fn lease_work(
    devices: &Devices,
    reporters: &Reporters,
    freshness_lease: FreshnessLease<'_>,
) -> FreshnessLeaseWork {
    let every_reporter_fresh = reporters.registered_reporters().all(|registered_reporter| {
        matches!(
            freshness_lease.freshness_of(&registered_reporter),
            ReporterFreshness::Fresh
        )
    });
    if every_reporter_fresh {
        return FreshnessLeaseWork::Settled;
    }

    let marks_devices_unreachable = devices.states().any(|reconciled_device_state| {
        Reachability::from_presence(reconciled_device_state.presence) == Reachability::Reachable
            && !reconciled_device_state.contributors.is_empty()
            && reconciled_device_state
                .contributors
                .iter()
                .all(|contributor| {
                    matches!(
                        freshness_lease.freshness_by_id(reporters, *contributor),
                        ReporterFreshness::SilentFor(_)
                    )
                })
    });

    if marks_devices_unreachable {
        FreshnessLeaseWork::MarksDevicesUnreachable
    } else {
        FreshnessLeaseWork::Settled
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

/// Which keyed device one reported platform handle names.
///
/// A handle two reporters attached to different keys names no device: joining an evidence-only
/// record to whichever key happened to be ingested last is exactly the plausible fallback that
/// exact-match identity exists to forbid.
enum HandleOwner {
    /// Every keyed record carrying this handle reported the same key.
    OneKey(DeviceKey),
    /// Keyed records disagree about which key this handle belongs to.
    SeveralKeys,
}

/// One record that carried no key, held with what it would contribute to the keyed device its
/// platform handle names.
struct EvidenceOnlyReport<'a> {
    reporter:      ReporterId,
    handle:        &'a ReportedId,
    device_record: &'a DeviceRecord,
    presence:      CoReportedPresence,
}

/// Merge every retained whole set into one device set and hand it to the registry.
fn ingest(
    reporters: &Reporters,
    devices: &mut Devices,
    registered_schemes: &RegisteredSchemes,
    hardware_inventory: &HardwareInventory,
    freshness_lease: FreshnessLease<'_>,
) -> ReconciledDeviceReplacement {
    let mut merged: HashMap<DeviceKey, MergedDevice<'_>> = HashMap::new();
    // First-seen order, so the handles the registry issues to new keys depend on the reporters'
    // own report order rather than on hash iteration order, which varies between runs.
    let mut ingest_order: Vec<DeviceKey> = Vec::new();
    let mut keyed_by_handle: HashMap<&ReportedId, HandleOwner> = HashMap::new();
    let mut evidence_only: Vec<EvidenceOnlyReport<'_>> = Vec::new();
    let mut duplicate_keys = HashSet::new();
    let mut unregistered_schemes = HashSet::new();

    // One pass over every retained record. Keyed records group through one `HashMap::entry`, which
    // is what makes co-report merging linear in device count rather than a pairwise join across
    // reporter lists; the same pass collects the evidence-only records the join below needs.
    for registered_reporter in reporters.registered_reporters() {
        let freshness = freshness_lease.freshness_of(&registered_reporter);
        let reporter = registered_reporter.reporter;
        let ReporterContribution::Completed { device_set, .. } = registered_reporter.contribution
        else {
            continue;
        };

        // A silent reporter's records still describe the devices it named; what they stop being is
        // evidence that those devices are reachable right now.
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
                    if let PlatformDeviceHandle::Reported(handle) =
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
                // An evidence-only record carries no key, so it can only join a keyed record
                // through a handle the platform actually reported. Two records that each reported
                // no handle compare equal, which is exactly the plausible-fallback join that
                // exact-match identity exists to prevent, so every other variant joins nothing.
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

    // A joined record is a co-report of the device its handle names, so it merges through the same
    // path a keyed record does: its presence, claim, and capability declarations all count.
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
            &mut duplicate_keys,
            key,
        );
    }

    let departed_slots = departed_slots(devices, &merged);
    let identity_evidence = IdentityEvidence {
        duplicate_keys: &duplicate_keys,
        departed_slots: &departed_slots,
        hardware_inventory,
    };

    let reconciled = fold_presence_roots_first(&merged, ingest_order, devices, identity_evidence);

    let mut replacement =
        devices.replace_reconciled(reconciled, duplicate_keys, unregistered_schemes);
    replacement.changes.connections =
        configured_device_connection_changes(reporters, hardware_inventory, freshness_lease);

    replacement
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
/// only strengthen what the kernel knows, never retract another reporter's fresher evidence.
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
            if !matches!(occupied.get(), HandleOwner::OneKey(owner) if owner == key) {
                occupied.insert(HandleOwner::SeveralKeys);
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
            .entry(capability.as_any().type_id())
            .or_default()
            .push(capability.as_partial_reflect());
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
/// cannot be trusted. Following the parent links first means a child is never folded against a
/// parent whose own presence has not been settled.
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
/// Grouped rather than passed as four arguments because all four are read together at exactly one
/// call site, and a reader of `verdict_for` should see one word for "what the rest of this pass
/// knows".
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
/// Equality is `PartialReflect::reflect_partial_eq` across the references under one type. A type
/// whose reflected comparison cannot answer counts as disputed: unavailable equality evidence is
/// not agreement.
fn disputed_capabilities(capabilities: &CoReportView<'_>) -> HashSet<TypeId> {
    capabilities
        .iter()
        .filter(|(_, declarations)| {
            declarations.windows(2).any(|pair| {
                pair.first().is_some_and(|first| {
                    pair.get(1)
                        .is_some_and(|second| first.reflect_partial_eq(*second) != Some(true))
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
    let app_type_registry = world.resource::<AppTypeRegistry>().clone();
    let type_registry = app_type_registry.read();
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

    // Every mutable path to `Bindings` marks the resource changed, so a pass with no departure
    // must not open one: a once-per-change consumer would otherwise fire on every frame.
    if !reconciled_device_changes.departed.is_empty() {
        owe_departure_work(
            &mut world.resource_mut::<Bindings>(),
            &reconciled_device_changes.departed,
        );
    }

    // The two facts with no mirrored component behind them are moved to the event stage before the
    // rest of this pass consumes them; nothing else retains a departure once the entities are gone.
    {
        let mut departure_announcements = world.resource_mut::<DepartureAnnouncements>();
        departure_announcements
            .departed
            .append(&mut reconciled_device_changes.departed);
        departure_announcements
            .connections
            .append(&mut reconciled_device_changes.connections);
    }

    let device_set_write = world.resource_scope::<Devices, _>(|world, mut devices| {
        let entered = devices.last_changed();
        let device_set_projection = world.resource_scope::<Reporters, _>(|world, reporters| {
            mirror_device_entities(world, &mut devices, &reporters, &type_registry)
        });
        resolve_binding_links(world, &devices);
        capture_ready_configurations(world, &devices);
        announce_disputes(
            world,
            &devices,
            &reconciled_device_changes.disputes_changed,
            &type_registry,
        );

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
    mirror_last_known_good(world, &type_registry);
}

/// Record what each role bound to a departed device is owed, as its `RecoveryPolicy` defines it.
///
/// `RecoveryPolicy::ReapplyOnReturn` owes a restoration, which is what makes the saved value return
/// with the unit. The other three owe an application request instead: without that record a
/// departed role falls back to `WaitingWork::Nothing`, reaches `WaitingRole::Hardware`, and has
/// its authored request dispatched automatically on the device's return — the one thing
/// `RecoveryPolicy::Retain` promises never happens. `RecoveryPolicy::Forget` additionally drops the
/// saved value at the departure rather than leaving it for a later restore.
///
/// Nothing else records it: a role that owes a restoration must not have its endpoint read back
/// first, because that would record the state the departure left behind as the value last known to
/// work.
///
/// Both departure causes count. A unit whose key left the reconciled set and a retained unit that
/// stopped being present are the same event for the role bound to it: the endpoint the saved value
/// belongs on is gone, and the value returns with the unit.
fn owe_departure_work(bindings: &mut Bindings, departed: &[DepartedDevice]) {
    for departed_device in departed {
        let roles: Vec<RoleKey> = bindings.roles_for(&departed_device.key).cloned().collect();
        for role in roles {
            let Ok(binding) = bindings.binding(&role) else {
                continue;
            };
            let recovery = binding.recovery;
            let established = matches!(
                binding.last_known_good,
                LastKnownGoodConfiguration::Known(_)
            );
            // The recorded work is only ever read through `RoleView::Waiting`, so a role the
            // departure left in `RoleState::Ready` would never reach it and every policy would
            // behave the same.
            bindings.await_departed_device(&role);
            match recovery {
                RecoveryPolicy::ReapplyOnReturn if established => {
                    bindings.set_waiting_work(&role, WaitingWork::RestorationOwed);
                },
                RecoveryPolicy::ReapplyOnReturn => {},
                RecoveryPolicy::Retain | RecoveryPolicy::ReapplyOnRequest => {
                    bindings.set_waiting_work(&role, WaitingWork::ApplicationRequestOwed);
                },
                RecoveryPolicy::Forget => {
                    bindings.forget_last_known_good(&role);
                    bindings.set_waiting_work(&role, WaitingWork::ApplicationRequestOwed);
                },
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
    reporters: &Reporters,
    type_registry: &TypeRegistry,
) -> DeviceSetProjection {
    let mut device_set_projection = DeviceSetProjection::Unwritten;
    let mirrored: Vec<MirroredDevice> = devices
        .states()
        .filter_map(|reconciled_device_state| {
            let DeviceResolution::Resolved(device_id) =
                devices.resolve(&reconciled_device_state.key)
            else {
                return None;
            };
            Some(MirroredDevice {
                device_id,
                key: reconciled_device_state.key.clone(),
                verdict: reconciled_device_state.verdict.clone(),
                presence: reconciled_device_state.presence,
                claim: reconciled_device_state.claim.clone(),
                disputed: reconciled_device_state.disputed.clone(),
            })
        })
        .collect();

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
        let (agreed, disputed): (Vec<&dyn Reflect>, Vec<&dyn Reflect>) =
            capability_declarations(reporters, &mirrored_device.key)
                .into_iter()
                .partition(|declaration| {
                    !mirrored_device
                        .disputed
                        .contains(&declaration.as_any().type_id())
                });
        let mut device_entity = world.entity_mut(entity);

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

        let usable = mirrored_device.presence == Presence::Present
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

        if let Err(capability_attach_error) =
            capabilities::attach_declarations(&mut device_entity, type_registry, agreed)
        {
            warn!(
                "device `{:?}` declares a capability the projection cannot attach: \
                 {capability_attach_error}",
                mirrored_device.key
            );
        }
        if let Err(capability_attach_error) =
            capabilities::detach_declarations(&mut device_entity, type_registry, disputed)
        {
            warn!(
                "device `{:?}` disputes a capability the projection cannot detach: \
                 {capability_attach_error}",
                mirrored_device.key
            );
        }
    }

    device_set_projection
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
    device_id: DeviceId,
    key:       DeviceKey,
    verdict:   IdentityVerdict,
    presence:  Presence,
    claim:     Claim,
    disputed:  HashSet<TypeId>,
}

/// Borrow every capability declaration the contributing reporters retain for one durable key.
///
/// Read from the reporter registry rather than from `ReconciledDeviceState`, which keeps the
/// declared and disputed type identifiers but not the values: `Box<dyn Reflect>` is neither
/// clonable nor reflectable, so a copy in the registry could drift from what its reporter holds.
fn capability_declarations<'a>(reporters: &'a Reporters, key: &DeviceKey) -> Vec<&'a dyn Reflect> {
    reporters
        .registered_reporters()
        .filter_map(
            |registered_reporter| match registered_reporter.contribution {
                ReporterContribution::Completed { device_set, .. } => Some(device_set),
                ReporterContribution::AwaitingFirstCompleteSet => None,
            },
        )
        .flat_map(|device_set| device_set.devices.iter())
        .filter(|device_record| device_record.reported_as == ReportedAs::Keyed(key.clone()))
        .flat_map(|device_record| device_record.capabilities.declarations())
        .collect()
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
    let binding_entities = world.resource::<BindingEntities>();

    bindings
        .registered_roles()
        .filter_map(|role| {
            let BindingEntityLookup::Registered(binding_entity) = binding_entities.entity(role)
            else {
                return None;
            };
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
    let binding_entities = world.resource::<BindingEntities>();
    let mut planned_mirrors = Vec::new();

    for role in bindings.registered_roles() {
        let BindingEntityLookup::Registered(binding_entity) = binding_entities.entity(role) else {
            continue;
        };
        let Ok(binding) = bindings.binding(role) else {
            continue;
        };
        let LastKnownGoodConfiguration::Known(configuration) = &binding.last_known_good else {
            if let Some(planned_erase) =
                planned_configuration_erase(world, type_registry, binding_entity)
            {
                planned_mirrors.push(planned_erase);
            }
            continue;
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

/// Take every safe opportunity this pass opened to learn what is actually on an endpoint.
///
/// The conditions are all of: a managed configured device, a role in `RoleState::Ready` so no
/// driver operation is in flight, nothing owed to the role, a present endpoint, a driver that has
/// not permanently declined, and no configuration established yet.
/// `ReadyRole::capture_request` is the only way to mint the request driver dispatch accepts, so the
/// checks cannot be bypassed by choosing a driver directly.
fn capture_ready_configurations(world: &mut World, devices: &Devices) {
    let capture_roles = roles_with_a_safe_capture_opportunity(world, devices);
    if capture_roles.is_empty() {
        return;
    }

    world.resource_scope::<Bindings, _>(|world, mut bindings| {
        world.resource_scope::<Drivers, _>(|world, mut drivers| {
            world.resource_scope::<HardwareInventory, _>(|world, hardware_inventory| {
                for role in capture_roles {
                    let capture_outcome = {
                        let Ok(RoleView::Ready(ready_role)) = bindings.role_view(&role) else {
                            continue;
                        };
                        let Ok(capture_request) = ready_role.capture_request(&hardware_inventory)
                        else {
                            continue;
                        };
                        let dispatched_role = capture_request.role.clone();
                        match drivers.capture(world, capture_request) {
                            Ok(capture_outcome) => capture_outcome,
                            Err(driver_contract_error) => {
                                warn!(
                                    "role `{dispatched_role:?}` configuration readback failed: \
                                     {driver_contract_error}"
                                );
                                continue;
                            },
                        }
                    };
                    if let Ok(RoleView::Ready(mut ready_role)) = bindings.role_view(&role) {
                        ready_role.record_capture(capture_outcome);
                    }
                }
            });
        });
    });
}

/// List the roles whose every safe-capture condition already holds, before anything is borrowed
/// mutably.
///
/// Selecting first is what keeps a settled frame silent: `Bindings` and `Drivers` are reached
/// through `World::resource_scope`, which marks a resource changed on reinsertion whether or not
/// the closure wrote to it, so a frame with no capture opportunity must never enter one. A role
/// whose `LastKnownGoodConfiguration` is already `Known` is one such settled role: the safe
/// readback it needed has happened, and repeating it every pass would dispatch a driver call per
/// frame for as long as the binding lives. A later departure clears the readback opportunity's
/// other conditions instead, through `WaitingWork::RestorationOwed`. Every condition here is
/// re-checked by `ReadyRole::capture_request`, which remains the only way to mint the request
/// driver dispatch accepts.
fn roles_with_a_safe_capture_opportunity(world: &World, devices: &Devices) -> Vec<RoleKey> {
    let bindings = world.resource::<Bindings>();
    let hardware_inventory = world.resource::<HardwareInventory>();

    bindings
        .registered_roles()
        .filter(|role| {
            bindings.waiting_work(role) == WaitingWork::Nothing
                && bindings.capture_dispatch(role) == CaptureDispatch::Eligible
                && bindings.configuration_readability(role) == ConfigurationReadability::Readable
                && bindings.binding(role).is_ok_and(|binding| {
                    binding.state == RoleState::Ready
                        && matches!(
                            binding.last_known_good,
                            LastKnownGoodConfiguration::NotEstablished
                        )
                        && endpoint_device_present(devices, &binding.endpoint.device)
                        && hardware_inventory
                            .ensure_operational(&binding.endpoint.device)
                            .is_ok()
                })
        })
        .cloned()
        .collect()
}

/// Report whether the device one endpoint names is currently reachable.
fn endpoint_device_present(devices: &Devices, key: &DeviceKey) -> bool {
    let DeviceResolution::Resolved(device_id) = devices.resolve(key) else {
        return false;
    };
    matches!(
        devices.state(device_id),
        DeviceStateLookup::Retained(reconciled_device_state)
            if reconciled_device_state.presence == Presence::Present
    )
}

/// Report every device whose contributors changed what they disagree about, once per change.
///
/// The warning is what makes a disagreement visible with no user interface attached; the event is
/// what a diagnostic panel consumes instead of polling `Devices`. An empty payload means the
/// disagreement cleared and the device is fully drivable again.
fn announce_disputes(
    world: &mut World,
    devices: &Devices,
    disputes_changed: &[DeviceId],
    type_registry: &TypeRegistry,
) {
    for device_id in disputes_changed {
        let DeviceStateLookup::Retained(reconciled_device_state) = devices.state(*device_id) else {
            continue;
        };
        let DeviceEntityLookup::Projected(device) = devices.entity(*device_id) else {
            continue;
        };
        // Sorted so one disagreement reads the same in every log line and in every event, since
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
        world.trigger(CapabilitiesDisputed {
            device,
            capabilities,
        });
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
    use bevy::prelude::On;
    use bevy::prelude::Query;
    use bevy::prelude::Reflect;
    use bevy::prelude::Res;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::World;
    use bevy::world_serialization::DynamicWorldBuilder;

    use super::FrameClockReading;
    use super::ReconcilePass;
    use super::planned_configuration_mirrors;
    use super::project_device_entities;
    use super::reconcile_devices;
    use super::reflect_component_for;
    use crate::ApplyDeadline;
    use crate::ApplyPermit;
    use crate::AttachmentPath;
    use crate::AttemptId;
    use crate::AttemptOutcome;
    use crate::AttemptProgress;
    use crate::AuthoritativeReporterCoverage;
    use crate::Binding;
    use crate::BindingEntities;
    use crate::BindingEntityLookup;
    use crate::Bindings;
    use crate::Capabilities;
    use crate::CapabilitiesDisputed;
    use crate::CaptureOutcome;
    use crate::Claim;
    use crate::ClaimHolder;
    use crate::ConfiguredDevice;
    use crate::ConfiguredDeviceConnection;
    use crate::ConfiguredDeviceMode;
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
    use crate::DiscoveryWork;
    use crate::EndpointDriver;
    use crate::EndpointId;
    use crate::HardwareInventory;
    use crate::IdentityVerdict;
    use crate::LastKnownGoodConfiguration;
    use crate::MainThreadDiscoveryJob;
    use crate::OnAbort;
    use crate::OnSessionLoss;
    use crate::PlatformDeviceHandle;
    use crate::Presence;
    use crate::RecoveryPolicy;
    use crate::RegisteredSchemes;
    use crate::ReportedAs;
    use crate::ReportedId;
    use crate::ReportedParent;
    use crate::ReportedSerial;
    use crate::ReporterCoverage;
    use crate::ReporterId;
    use crate::ReporterRegistration;
    use crate::RequestedConfiguration;
    use crate::RetryOn;
    use crate::RiggingAppExt;
    use crate::RiggingLimits;
    use crate::RiggingPlugin;
    use crate::RiggingRevision;
    use crate::RoleKey;
    use crate::RoleState;
    use crate::SchemeName;
    use crate::UnverifiedReason;
    use crate::binding::RoleView;
    use crate::binding::WaitingRole;
    use crate::binding::WaitingWork;
    use crate::capabilities::CapabilityAttachError;
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
            ReporterRegistration::required(cadence, ReporterCoverage::MatchingEvidenceOnly),
        )
    }

    /// Run one reconcile pass over the app's resources without going through the schedule, so a
    /// test controls the frame clock and the reporter registry contents exactly.
    fn reconcile_once(app: &mut App, clock: FrameClockReading) -> usize {
        let mut allocations = 0;
        app.world_mut()
            .resource_scope::<Reporters, _>(|world, mut reporters| {
                world.resource_scope::<Devices, _>(|world, mut devices| {
                    world.resource_scope::<RiggingRevision, _>(|world, mut rigging_revision| {
                        world.resource_scope::<RiggingLimits, _>(|world, rigging_limits| {
                            world.resource_scope::<RegisteredSchemes, _>(
                                |_, registered_schemes| {
                                    allocations = allocations_during(|| {
                                        reconcile_devices(
                                            &mut reporters,
                                            &mut devices,
                                            &mut rigging_revision,
                                            &rigging_limits,
                                            &registered_schemes,
                                            &HardwareInventory::default(),
                                            clock,
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

        // The handle names two keys, so it names neither: attaching the evidence to whichever key
        // was ingested last would be the plausible fallback exact-match identity forbids.
        let devices = app.world().resource::<Devices>();
        assert_eq!(devices.count(), 2);
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

        assert_eq!(app.world().resource::<Devices>().count(), 0);
    }

    #[test]
    fn two_evidence_only_records_without_platform_handles_do_not_join_each_other() {
        let mut app = app_with_scheme();
        add_reporter(&mut app, || vec![record(ReportedAs::MatchEvidenceOnly)]);
        add_reporter(&mut app, || vec![record(ReportedAs::MatchEvidenceOnly)]);

        run_until_reconciled(&mut app);

        // Both records carry `PlatformReportedNothing`, which compares equal to itself. Joining on
        // it would mint a device out of two reports that share no evidence at all.
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
        let first = add_reporter(&mut app, || {
            vec![
                record(ReportedAs::Keyed(key("panel-a"))),
                record(ReportedAs::Keyed(key("panel-b"))),
            ]
        });
        let second = add_reporter(&mut app, || {
            vec![
                record(ReportedAs::Keyed(key("panel-a"))),
                record(ReportedAs::Keyed(key("panel-b"))),
            ]
        });
        let third = add_reporter(&mut app, || {
            vec![
                record(ReportedAs::Keyed(key("panel-a"))),
                record(ReportedAs::Keyed(key("panel-b"))),
            ]
        });

        // Discovery admits a bounded number of jobs per frame, so three reporters need more frames
        // than two before every whole set has been accepted.
        run_until_reconciled(&mut app);
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
        key:        DeviceKey,
        parent:     ReportedParent,
        attachment: AttachmentPath,
        claim:      Claim,
        presence:   Presence,
        brightness: Vec<u8>,
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

    /// A driver that reads one fixed configuration back and counts how often it was asked.
    struct CountingCaptureDriver(Arc<AtomicUsize>);

    impl EndpointDriver for CountingCaptureDriver {
        type Configuration = PanelConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &crate::DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            self.0.fetch_add(1, Ordering::Relaxed);
            CaptureOutcome::Read(PanelConfiguration(7))
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            _: &crate::DeviceEndpoint,
            _: &Self::Configuration,
            _: crate::AttemptId,
            _: crate::ApplyPermit,
        ) {
        }

        fn poll(&mut self, _: &mut World, _: crate::AttemptId) -> crate::AttemptProgress {
            AttemptProgress::Finished(AttemptOutcome::Succeeded)
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

    /// A driver that reads back a configuration the mirror cannot project.
    struct UnmirrorableDriver;

    impl EndpointDriver for UnmirrorableDriver {
        type Configuration = UnmirrorableConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &crate::DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            CaptureOutcome::Read(UnmirrorableConfiguration(7))
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            _: &crate::DeviceEndpoint,
            _: &Self::Configuration,
            _: crate::AttemptId,
            _: crate::ApplyPermit,
        ) {
        }

        fn poll(&mut self, _: &mut World, _: crate::AttemptId) -> crate::AttemptProgress {
            AttemptProgress::Finished(AttemptOutcome::Succeeded)
        }
    }

    fn unit(device_key: DeviceKey) -> ReportedUnit {
        ReportedUnit {
            key:        device_key,
            parent:     ReportedParent::Root,
            attachment: AttachmentPath::PlatformHasNoConcept,
            claim:      Claim::NotApplicable,
            presence:   Presence::Present,
            brightness: Vec::new(),
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
            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
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
            ReporterRegistration::required(cadence, coverage),
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
        let world = app.world_mut();
        world.resource_scope::<Reporters, _>(|world, mut reporters| {
            world.resource_scope::<Devices, _>(|world, mut devices| {
                world.resource_scope::<RiggingRevision, _>(|world, mut rigging_revision| {
                    world.resource_scope::<RiggingLimits, _>(|world, rigging_limits| {
                        world.resource_scope::<RegisteredSchemes, _>(
                            |world, registered_schemes| {
                                world.resource_scope::<HardwareInventory, _>(
                                    |world, hardware_inventory| {
                                        if let ReconcilePass::Merged(replacement) =
                                            reconcile_devices(
                                                &mut reporters,
                                                &mut devices,
                                                &mut rigging_revision,
                                                &rigging_limits,
                                                &registered_schemes,
                                                &hardware_inventory,
                                                clock,
                                            )
                                        {
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
            Some(IdentityVerdict::RestoreOnly)
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
        // An absent unit is a connection conclusion, never an entity: nothing was reported to
        // mirror.
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
        let panel = key("panel-a");
        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(panel.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );

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
        let panel = key("panel-a");
        let role = crate::RoleKey::new("primary-window")?;
        app.world_mut()
            .resource_mut::<crate::Bindings>()
            .register(crate::Binding {
                role:            role.clone(),
                endpoint:        crate::DeviceEndpoint {
                    device: panel.clone(),
                    id:     EndpointId::Whole,
                },
                driver:          DriverId(0),
                recovery:        RecoveryPolicy::Forget,
                retry:           RetryOn::NewRevision,
                on_abort:        crate::OnAbort::default(),
                on_loss:         crate::OnSessionLoss::default(),
                state:           crate::RoleState::default(),
                requested:       crate::RequestedConfiguration::new(()),
                last_known_good: crate::LastKnownGoodConfiguration::default(),
                apply_deadline:  ApplyDeadline::ProcessDefault,
            })?;
        let reported_units = add_set_reporter(
            &mut app,
            vec![unit(panel.clone())],
            ReporterCoverage::MatchingEvidenceOnly,
        );

        run_until_reconciled(&mut app);

        let BindingEntityLookup::Registered(binding_entity) = app
            .world()
            .resource::<crate::BindingEntities>()
            .entity(&role)
        else {
            return Err("a registered role owns a binding entity".into());
        };
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
            state: RoleState::default(),
            requested: RequestedConfiguration::new(PanelConfiguration(3)),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline: ApplyDeadline::ProcessDefault,
        }
    }

    /// Drive one registered role from waiting to ready through a completed apply.
    ///
    /// Registration always resets a role to waiting, and only a finished operation opens the
    /// safe-capture window this phase reads back through. The permit is a parameter because a role
    /// only leaves waiting on an authorized operation, and a test that reads its permit out of
    /// `Devices` proves the authorization step rather than assuming it.
    fn reach_ready(app: &mut App, role: &RoleKey, permit: ApplyPermit) {
        let hardware_inventory = HardwareInventory::default();
        let world = app.world_mut();
        world.resource_scope::<Bindings, _>(|world, mut bindings| {
            world.resource_scope::<Drivers, _>(|world, mut drivers| {
                let Ok(RoleView::Waiting(WaitingRole::Hardware(requesting_role))) =
                    bindings.role_view(role)
                else {
                    panic!("a registered role starts out waiting for hardware");
                };
                let start_apply_request = requesting_role
                    .start_requested_apply(AttemptId::default(), permit, &hardware_inventory)
                    .expect("an unauthored endpoint accepts an in-service apply");
                drivers
                    .start_apply(world, start_apply_request)
                    .expect("the registered driver accepts its own configuration type");
                let Ok(RoleView::Applying(mut applying_role)) = bindings.role_view(role) else {
                    panic!("a dispatched apply selects the applying view");
                };
                applying_role.finish(AttemptOutcome::Succeeded);
            });
        });
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

        let BindingEntityLookup::Registered(binding_entity) =
            app.world().resource::<BindingEntities>().entity(&role)
        else {
            return Err("a registered role owns a binding entity".into());
        };
        // A waiting role is not a safe readback opportunity, and an unestablished value mirrors
        // nothing onto the entity.
        assert_eq!(captures.load(Ordering::Relaxed), 0);
        assert_eq!(app.world().get::<PanelConfiguration>(binding_entity), None);

        reach_ready(&mut app, &role, ApplyPermit::in_service());
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
        reach_ready(&mut app, &role, ApplyPermit::in_service());
        app.update();

        let BindingEntityLookup::Registered(binding_entity) =
            app.world().resource::<BindingEntities>().entity(&role)
        else {
            return Err("a registered role owns a binding entity".into());
        };
        let app_type_registry = app.world().resource::<AppTypeRegistry>().clone();
        let type_registry = app_type_registry.read();
        let bindings = app.world().resource::<Bindings>();
        // The readback established a value, so the mirror reached the driver's own type rather than
        // skipping the role for having nothing to project.
        let LastKnownGoodConfiguration::Known(configuration) =
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
        assert_eq!(
            app.world().resource::<Bindings>().binding(&role)?.state,
            RoleState::Waiting
        );
        assert_eq!(captures.load(Ordering::Relaxed), 0);

        rewrite(&reported_units, vec![unit(panel.clone())]);
        run_until_reconciled(&mut app);

        // The same role reaches ready only through a permit the reconciled device minted.
        let devices = app.world().resource::<Devices>();
        let DeviceResolution::Resolved(device_id) = devices.resolve(&panel) else {
            return Err("a reported key resolves after reconciliation".into());
        };
        let permit = devices.authorize_service(device_id)?;
        reach_ready(&mut app, &role, permit);

        assert_eq!(
            app.world().resource::<Bindings>().binding(&role)?.state,
            RoleState::Ready
        );

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

    #[test]
    fn a_capability_disagreement_announces_once_and_announces_once_more_when_it_clears() {
        #[derive(Default, Resource)]
        struct AnnouncedDisputes(Vec<Vec<String>>);

        let mut app = app_with_scheme();
        let contested = key("streamdeck-xl");
        let mut agreeing = unit(contested.clone());
        agreeing.brightness = vec![50];
        let mut disagreeing = unit(contested.clone());
        disagreeing.brightness = vec![90];
        add_set_reporter(
            &mut app,
            vec![agreeing],
            ReporterCoverage::MatchingEvidenceOnly,
        );
        let disagreeing_units = add_set_reporter(
            &mut app,
            vec![disagreeing],
            ReporterCoverage::MatchingEvidenceOnly,
        );
        app.init_resource::<AnnouncedDisputes>().add_observer(
            |capabilities_disputed: On<CapabilitiesDisputed>,
             mut announced_disputes: ResMut<AnnouncedDisputes>| {
                announced_disputes
                    .0
                    .push(capabilities_disputed.capabilities.clone());
            },
        );

        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);

        assert_eq!(
            app.world().resource::<AnnouncedDisputes>().0,
            vec![vec![String::from(
                "hana_rigging::reconcile::tests::Brightness"
            )]]
        );

        // A frame that changes nothing restates nothing.
        app.update();

        assert_eq!(app.world().resource::<AnnouncedDisputes>().0.len(), 1);

        let mut agreeing_again = unit(contested);
        agreeing_again.brightness = vec![50];
        rewrite(&disagreeing_units, vec![agreeing_again]);
        run_until_reconciled(&mut app);
        run_until_reconciled(&mut app);

        let announced_disputes = &app.world().resource::<AnnouncedDisputes>().0;
        assert_eq!(announced_disputes.len(), 2);
        assert!(
            announced_disputes[1].is_empty(),
            "a cleared disagreement announces itself with an empty payload"
        );
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
        reach_ready(&mut app, &role, ApplyPermit::in_service());
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
        reach_ready(&mut app, &role, ApplyPermit::in_service());
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
        let BindingEntityLookup::Registered(binding_entity) =
            app.world().resource::<BindingEntities>().entity(&role)
        else {
            return Err("a registered role owns a binding entity".into());
        };
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

    /// Drive one binding under `recovery` to an established last-known-good value, make its unit
    /// absent, and read back what the departure recorded and whether the saved value survived.
    fn depart_under(recovery: RecoveryPolicy) -> Result<(WaitingWork, bool), Box<dyn Error>> {
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
        reach_ready(&mut app, &role, ApplyPermit::in_service());
        app.update();
        assert_eq!(captures.load(Ordering::Relaxed), 1);

        let mut absent = unit(panel);
        absent.presence = Presence::Absent;
        rewrite(&reported_units, vec![absent]);
        run_until_reconciled(&mut app);

        let bindings = app.world().resource::<Bindings>();
        Ok((
            bindings.waiting_work(&role),
            matches!(
                bindings.binding(&role)?.last_known_good,
                LastKnownGoodConfiguration::Known(_)
            ),
        ))
    }

    #[test]
    fn each_recovery_policy_records_its_own_departure_work() -> Result<(), Box<dyn Error>> {
        // `ReapplyOnReturn` is the only policy that reapplies without being asked, so it is the
        // only one that owes a restoration. The other three owe an application request: without
        // that record a departed role falls back to `WaitingWork::Nothing`, reaches
        // `WaitingRole::Hardware`, and has its authored request dispatched automatically when
        // the unit returns.
        assert_eq!(
            depart_under(RecoveryPolicy::ReapplyOnReturn)?,
            (WaitingWork::RestorationOwed, true)
        );
        assert_eq!(
            depart_under(RecoveryPolicy::Retain)?,
            (WaitingWork::ApplicationRequestOwed, true)
        );
        assert_eq!(
            depart_under(RecoveryPolicy::ReapplyOnRequest)?,
            (WaitingWork::ApplicationRequestOwed, true)
        );
        // `Forget` drops the saved value at the departure rather than leaving it for a later
        // restore.
        assert_eq!(
            depart_under(RecoveryPolicy::Forget)?,
            (WaitingWork::ApplicationRequestOwed, false)
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

        let mut bindings = app.world_mut().resource_mut::<Bindings>();
        assert_eq!(bindings.waiting_work(&role), WaitingWork::Nothing);
        let Ok(RoleView::Waiting(WaitingRole::Hardware(_))) = bindings.role_view(&role) else {
            return Err("a newly registered role waits for hardware".into());
        };

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
        reach_ready(&mut app, &role, ApplyPermit::in_service());
        app.update();

        let BindingEntityLookup::Registered(binding_entity) =
            app.world().resource::<BindingEntities>().entity(&role)
        else {
            return Err("a registered role owns a binding entity".into());
        };
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
        let BindingEntityLookup::Registered(binding_entity) =
            app.world().resource::<BindingEntities>().entity(&role)
        else {
            return Err("a registered role owns a binding entity".into());
        };
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
        let BindingEntityLookup::Registered(binding_entity) =
            app.world().resource::<BindingEntities>().entity(&role)
        else {
            return Err("a registered role owns a binding entity".into());
        };
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
