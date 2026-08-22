//! Hardware-free device reporting for `hana_rigging`.
//!
//! A `ScriptedReporter` replays a hand-written list of whole-set scans, so a test or an example can
//! make a device arrive, depart, change claim, or fail enumeration without any hardware attached.
//! Real hardware cannot serve this purpose: a runner cannot unplug a monitor on command, the
//! attached set differs per machine, and states worth testing — a duplicate key in one scan, two
//! reporters disagreeing about a capability, a unit swapped on the same port — cannot be produced
//! physically at all.
//!
//! The crate is built from `hana_rigging`'s public surface only and is never published. It lives
//! outside `crates/hana_rigging/tests` because each file there compiles to its own binary that
//! nothing else can depend on, while the consumers are other crates and another repository.
//!
//! `ScriptedDevice` exists rather than a bare `hana_rigging::DeviceRecord` for two reasons. A
//! record has nine required fields and no defaults, so every scripted scan would restate the four
//! evidence fields it does not care about; and `hana_rigging::Capabilities` holds
//! `Box<dyn Reflect>`, which Bevy 0.19 cannot clone, so a template that is replayed more than once
//! has to build its declaration on demand instead of holding one.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;

use bevy::app::App;
use bevy::prelude::World;
use hana_rigging::AttachmentPath;
use hana_rigging::Capabilities;
use hana_rigging::Claim;
use hana_rigging::DeviceAccessError;
use hana_rigging::DeviceDescriptor;
use hana_rigging::DeviceIdSource;
use hana_rigging::DeviceKey;
use hana_rigging::DeviceKind;
use hana_rigging::DeviceRecord;
use hana_rigging::DeviceReporter;
use hana_rigging::DeviceScan;
use hana_rigging::DiscoveryControl;
use hana_rigging::DiscoveryJob;
use hana_rigging::DiscoveryProgress;
use hana_rigging::DiscoveryStatus;
use hana_rigging::DiscoveryWork;
use hana_rigging::MainThreadDiscoveryJob;
use hana_rigging::PlatformDeviceHandle;
use hana_rigging::Presence;
use hana_rigging::ReportedAs;
use hana_rigging::ReportedId;
use hana_rigging::ReportedIdError;
use hana_rigging::ReportedParent;
use hana_rigging::ReportedSerial;
use hana_rigging::ReporterActivity;
use hana_rigging::ReporterId;
use hana_rigging::SchemeName;
use hana_rigging::SchemeNameError;
use thiserror::Error;

/// How many frames `advance_reporter` will run before giving up on one requested scan.
///
/// A scheduled reporter needs one update to prepare and run its job and one more for the kernel to
/// accept the completed set, so a run that has not landed within this many frames has stalled for a
/// reason the caller wants reported rather than waited out.
const SCAN_FRAME_CEILING: u32 = 16;

/// A capability declaration rebuilt on demand for each replay of one scripted device.
type CapabilityBuilder = Arc<dyn Fn() -> Capabilities + Send + Sync>;

/// Failure building a durable key from scheme text.
#[derive(Debug, Error)]
pub enum ScriptedKeyError {
    /// The scheme name was rejected by `hana_rigging::SchemeName`.
    #[error("scripted scheme name rejected: {0}")]
    Scheme(#[from] SchemeNameError),
    /// The reported value was rejected by `hana_rigging::ReportedId`.
    #[error("scripted reported id rejected: {0}")]
    ReportedId(#[from] ReportedIdError),
}

/// Failure driving a scripted reporter through one requested discovery run.
#[derive(Debug, Error)]
pub enum ScriptedAdvanceError {
    /// The kernel refused the discovery request, usually because the reporter is not registered.
    #[error("the kernel refused a discovery request for the scripted reporter: {0}")]
    RequestRefused(String),
    /// The reporter has no retained status, so its completion count cannot be watched.
    #[error("the scripted reporter has no retained discovery status: {0}")]
    NoStatus(String),
    /// The requested run did not complete within `SCAN_FRAME_CEILING` frames.
    #[error("a scripted discovery run did not complete within {SCAN_FRAME_CEILING} frames")]
    Stalled,
}

/// Build the durable key a scripted reporter mints for one reported unit.
///
/// # Errors
///
/// Returns `ScriptedKeyError` when `scheme` or `value` is not an acceptable identity component.
pub fn reported_key(
    kind: DeviceKind,
    scheme: &str,
    value: &str,
) -> Result<DeviceKey, ScriptedKeyError> {
    Ok(DeviceKey {
        kind,
        id: DeviceIdSource::Reported {
            scheme: SchemeName::new(scheme)?,
            value:  ReportedId::new(value)?,
        },
    })
}

/// One unit as a scripted reporter will report it, replayable any number of times.
///
/// Every evidence field defaults to the variant that says the platform reported nothing, and the
/// parent defaults to a root, so a scan that only cares about presence and claim states only those.
#[derive(Clone)]
pub struct ScriptedDevice {
    reported_as:            ReportedAs,
    parent:                 ReportedParent,
    presence:               Presence,
    claim:                  Claim,
    capabilities:           CapabilityBuilder,
    serial:                 ReportedSerial,
    platform_device_handle: PlatformDeviceHandle,
    attachment:             AttachmentPath,
    descriptor:             DeviceDescriptor,
}

impl ScriptedDevice {
    /// Report a durably named unit as present, with no exclusive-ownership concept.
    #[must_use]
    pub fn present(device_key: DeviceKey) -> Self {
        Self::new(ReportedAs::Keyed(device_key), Presence::Present)
    }

    /// Report a durably named unit as established to be gone from the reporter's whole set.
    #[must_use]
    pub fn absent(device_key: DeviceKey) -> Self {
        Self::new(ReportedAs::Keyed(device_key), Presence::Absent)
    }

    /// Report a unit the reporter recognizes but cannot name durably.
    ///
    /// Reconciliation keeps this record only when it joins a keyed one, so a scripted
    /// evidence-only device is how a test exercises the two-reporter join.
    #[must_use]
    pub fn match_evidence_only(presence: Presence) -> Self {
        Self::new(ReportedAs::MatchEvidenceOnly, presence)
    }

    /// Report a unit under any naming status and reachability the caller wants to script.
    #[must_use]
    pub fn new(reported_as: ReportedAs, presence: Presence) -> Self {
        Self {
            reported_as,
            parent: ReportedParent::Root,
            presence,
            claim: Claim::NotApplicable,
            capabilities: Arc::new(Capabilities::new),
            serial: ReportedSerial::NotExposedByUnit,
            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
            attachment: AttachmentPath::PlatformHasNoConcept,
            descriptor: DeviceDescriptor::PlatformReportedNothing,
        }
    }

    /// Report this unit's exclusive ownership as something other than not-applicable.
    #[must_use]
    pub fn with_claim(mut self, claim: Claim) -> Self {
        self.claim = claim;
        self
    }

    /// Place this unit below the device it is reached through instead of at the root.
    #[must_use]
    pub fn with_parent(mut self, parent: ReportedParent) -> Self {
        self.parent = parent;
        self
    }

    /// Declare this unit's capability components, rebuilt fresh on every replay.
    #[must_use]
    pub fn with_capabilities(
        mut self,
        capabilities: impl Fn() -> Capabilities + Send + Sync + 'static,
    ) -> Self {
        self.capabilities = Arc::new(capabilities);
        self
    }

    /// Report serial evidence for this unit.
    #[must_use]
    pub fn with_serial(mut self, serial: ReportedSerial) -> Self {
        self.serial = serial;
        self
    }

    /// Report an operating-system handle that can join this record to another reporter's.
    #[must_use]
    pub fn with_platform_device_handle(
        mut self,
        platform_device_handle: PlatformDeviceHandle,
    ) -> Self {
        self.platform_device_handle = platform_device_handle;
        self
    }

    /// Report where this unit is attached, which is what a displaced-unit test varies.
    #[must_use]
    pub fn with_attachment(mut self, attachment: AttachmentPath) -> Self {
        self.attachment = attachment;
        self
    }

    /// Report vendor, product, and model evidence for this unit.
    #[must_use]
    pub fn with_descriptor(mut self, descriptor: DeviceDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    /// Build the record the kernel ingests for this replay.
    fn record(&self) -> DeviceRecord {
        DeviceRecord {
            reported_as:            self.reported_as.clone(),
            parent:                 self.parent.clone(),
            presence:               self.presence,
            claim:                  self.claim.clone(),
            capabilities:           (self.capabilities)(),
            serial:                 self.serial.clone(),
            platform_device_handle: self.platform_device_handle.clone(),
            attachment:             self.attachment.clone(),
            descriptor:             self.descriptor.clone(),
        }
    }
}

/// One scheduled outcome in a scripted reporter's list.
///
/// A failure is a separate variant rather than an empty completion because the kernel retains the
/// preceding set through an enumeration failure: treating the two the same would report every
/// device as departed whenever a scripted probe failed.
#[derive(Clone)]
pub enum ScriptedScan {
    /// The reporter enumerated successfully and these are all the units it can currently see.
    Complete(Vec<ScriptedDevice>),
    /// Enumeration failed before the reporter established its whole current set.
    Failed(DeviceAccessError),
}

impl ScriptedScan {
    fn scan(&self) -> DeviceScan {
        match self {
            Self::Complete(scripted_devices) => DeviceScan::Complete(
                scripted_devices
                    .iter()
                    .map(ScriptedDevice::record)
                    .collect(),
            ),
            Self::Failed(device_access_error) => DeviceScan::Failed(device_access_error.clone()),
        }
    }
}

/// A reporter that replays a written list of whole-set scans instead of touching hardware.
///
/// Once the list runs out the last scripted scan repeats. A reporter that stopped reporting
/// entirely would instead read as an empty whole set, which establishes absence for everything it
/// covers — so a script that ends would depart every device it just introduced.
pub struct ScriptedReporter {
    remaining: VecDeque<ScriptedScan>,
    repeated:  ScriptedScan,
    held:      Option<HeldRun>,
}

/// What a gated reporter reports and waits on before its scan is allowed to complete.
#[derive(Clone)]
struct HeldRun {
    progress: DiscoveryProgress,
    gate:     ScriptedRunGate,
}

impl ScriptedReporter {
    /// Replay `scans` in order, then repeat the last one for the rest of the run.
    ///
    /// An empty list reports an empty complete set forever, which is the reporter that sees no
    /// hardware at all.
    #[must_use]
    pub fn new(scans: impl IntoIterator<Item = ScriptedScan>) -> Self {
        let remaining: VecDeque<ScriptedScan> = scans.into_iter().collect();
        let repeated = remaining
            .back()
            .cloned()
            .unwrap_or_else(|| ScriptedScan::Complete(Vec::new()));

        Self {
            remaining,
            repeated,
            held: None,
        }
    }

    /// Replay the same list, but on the I/O pool, reporting `progress` and then holding at the
    /// gate.
    ///
    /// The scans of `ScriptedReporter::new` are `DiscoveryWork::Immediate`, so they complete inside
    /// the same admission call that started them and the kernel never observes the reporter
    /// running. A gated reporter is what a test uses to read a run that is still in flight —
    /// the progress the scheduler retains, the counts its batch carries, and the events derived
    /// from both.
    #[must_use]
    pub fn gated(
        scans: impl IntoIterator<Item = ScriptedScan>,
        progress: DiscoveryProgress,
    ) -> (Self, ScriptedRunGate) {
        let gate = ScriptedRunGate::new();
        let mut scripted_reporter = Self::new(scans);
        scripted_reporter.held = Some(HeldRun {
            progress,
            gate: gate.clone(),
        });

        (scripted_reporter, gate)
    }
}

impl DeviceReporter for ScriptedReporter {
    fn discover(&mut self) -> DiscoveryWork {
        let scripted_scan = self
            .remaining
            .pop_front()
            .unwrap_or_else(|| self.repeated.clone());
        let device_scan = scripted_scan.scan();

        let Some(held_run) = self.held.clone() else {
            return DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_: &mut World| {
                device_scan
            }));
        };

        DiscoveryWork::Background(DiscoveryJob::new(move |discovery_progress_sender| {
            drop(discovery_progress_sender.send(held_run.progress));
            held_run.gate.wait();
            device_scan
        }))
    }
}

/// Releases one held scan of a `ScriptedReporter::gated` reporter.
///
/// Releases are counted rather than signalled, so a test may release before or after the job
/// reaches the gate and neither ordering can lose the release or deadlock the I/O pool thread.
#[derive(Clone)]
pub struct ScriptedRunGate(Arc<ScriptedRunGateState>);

struct ScriptedRunGateState {
    releases: Mutex<usize>,
    released: Condvar,
}

impl ScriptedRunGate {
    fn new() -> Self {
        Self(Arc::new(ScriptedRunGateState {
            releases: Mutex::new(0),
            released: Condvar::new(),
        }))
    }

    /// Let one held scan finish and return its whole set to the kernel.
    pub fn release(&self) {
        let mut releases = self
            .0
            .releases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *releases += 1;
        drop(releases);
        self.0.released.notify_one();
    }

    fn wait(&self) {
        let mut releases = self
            .0
            .releases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *releases == 0 {
            releases = self
                .0
                .released
                .wait(releases)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *releases -= 1;
    }
}

/// Ask one scripted reporter for a run and advance frames until the kernel has accepted it.
///
/// Watching the reporter's own completion count rather than counting frames is what keeps a test
/// from encoding the scheduler's current admission timing: the kernel takes one update to run the
/// prepared job and at least one more to accept the completed set, and that is a scheduling detail
/// this harness deliberately does not pin.
///
/// # Errors
///
/// Returns `ScriptedAdvanceError` when the kernel refuses the request, when the reporter has no
/// retained status, or when the run has not been accepted within `SCAN_FRAME_CEILING` frames.
pub fn advance_reporter(app: &mut App, reporter: ReporterId) -> Result<(), ScriptedAdvanceError> {
    let completed_before = completed_batches(app, reporter)?;
    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .request(reporter)
        .map_err(|error| ScriptedAdvanceError::RequestRefused(error.to_string()))?;

    for _ in 0..SCAN_FRAME_CEILING {
        app.update();
        if completed_batches(app, reporter)? > completed_before {
            return Ok(());
        }
    }

    Err(ScriptedAdvanceError::Stalled)
}

/// Ask one gated reporter for a run and advance frames until the kernel reports it running.
///
/// Only a `ScriptedReporter::gated` reporter can reach this state: an immediate scan completes
/// inside the admission call that started it, so the kernel retains no running activity for it.
/// The run is left in flight, which is what lets a caller read a batch mid-run and then release it
/// with `ScriptedRunGate::release` and `advance_until_accepted`.
///
/// # Errors
///
/// Returns `ScriptedAdvanceError` when the kernel refuses the request, when the reporter has no
/// retained status, or when the run has not started within `SCAN_FRAME_CEILING` frames.
pub fn advance_until_running(
    app: &mut App,
    reporter: ReporterId,
) -> Result<(), ScriptedAdvanceError> {
    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .request(reporter)
        .map_err(|error| ScriptedAdvanceError::RequestRefused(error.to_string()))?;

    for _ in 0..SCAN_FRAME_CEILING {
        app.update();
        if is_running(app, reporter)? {
            return Ok(());
        }
    }

    Err(ScriptedAdvanceError::Stalled)
}

/// Advance frames until the kernel has accepted the run already in flight.
///
/// The counterpart to `advance_until_running`: the request has already been made, so this waits on
/// the completion count alone.
///
/// # Errors
///
/// Returns `ScriptedAdvanceError` when the reporter has no retained status, or when the run has not
/// been accepted within `SCAN_FRAME_CEILING` frames.
pub fn advance_until_accepted(
    app: &mut App,
    reporter: ReporterId,
) -> Result<(), ScriptedAdvanceError> {
    let completed_before = completed_batches(app, reporter)?;
    for _ in 0..SCAN_FRAME_CEILING {
        app.update();
        if completed_batches(app, reporter)? > completed_before {
            return Ok(());
        }
    }

    Err(ScriptedAdvanceError::Stalled)
}

/// Read whether one reporter's retained activity is a run the kernel currently holds.
fn is_running(app: &App, reporter: ReporterId) -> Result<bool, ScriptedAdvanceError> {
    app.world()
        .resource::<DiscoveryStatus>()
        .reporter_status(reporter)
        .map(|reporter_discovery_status| {
            matches!(
                reporter_discovery_status.activity,
                ReporterActivity::Running { .. }
            )
        })
        .map_err(|error| ScriptedAdvanceError::NoStatus(error.to_string()))
}

/// Read how many whole-set batches one reporter has finished accepting.
fn completed_batches(app: &App, reporter: ReporterId) -> Result<u64, ScriptedAdvanceError> {
    app.world()
        .resource::<DiscoveryStatus>()
        .reporter_status(reporter)
        .map(|reporter_discovery_status| reporter_discovery_status.completed_batches)
        .map_err(|error| ScriptedAdvanceError::NoStatus(error.to_string()))
}

/// Build a `ScriptedScan::Complete` from a list of scripted devices.
///
/// The macro exists so a scan reads as the whole set it is: a reporter always reports everything it
/// can currently see, and a device left out of the list is the departure evidence.
#[macro_export]
macro_rules! scan {
    [$($scripted_device:expr),* $(,)?] => {
        $crate::ScriptedScan::Complete(vec![$($scripted_device),*])
    };
}
