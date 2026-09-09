//! Hardware-free device reporting for `hana_rigging`.
//!
//! A [`ScriptedReporter`] replays a hand-written list of whole-set scans, so a test or an example
//! can make a device arrive, depart, change claim, or fail enumeration without any hardware
//! attached.
//! Real hardware cannot serve this purpose: a runner cannot unplug a monitor on command, the
//! attached set differs per machine, and states worth testing — a duplicate key in one scan, two
//! reporters disagreeing about a capability, a unit swapped on the same port — cannot be produced
//! physically at all.
//!
//! The crate is built from `hana_rigging`'s public surface only and is never published. It lives
//! outside `crates/hana_rigging/tests` because each file there compiles to its own binary that
//! nothing else can depend on, while the consumers are other crates and another repository.
//!
//! [`ScriptedDevice`] exists rather than a bare [`hana_rigging::DeviceRecord`] for two reasons. A
//! record has nine required fields and no defaults, so every scripted scan would restate the four
//! evidence fields it does not vary; and [`hana_rigging::Capabilities`] holds
//! `Box<dyn Reflect>`, which Bevy 0.19 cannot clone, so a template that is replayed more than once
//! has to build its declaration on demand instead of holding one.

/// A walk that proves any endpoint driver against the kernel's whole lifecycle.
mod conformance;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::Duration;
use std::time::Instant;

use bevy::app::App;
use bevy::prelude::Component;
use bevy::prelude::FromReflect;
use bevy::prelude::Reflect;
use bevy::prelude::World;
pub use conformance::CapabilityDeclaration;
pub use conformance::ConformanceFailure;
pub use conformance::ConformanceRefusal;
pub use conformance::ConformanceReport;
pub use conformance::ConformanceStep;
pub use conformance::ConformanceStop;
pub use conformance::ConformanceSubject;
pub use conformance::RecordedCleanup;
pub use conformance::RecordedCleanups;
pub use conformance::run;
use hana_rigging::ApplyContext;
use hana_rigging::AttachmentPath;
use hana_rigging::AttemptCompletion;
use hana_rigging::AttemptInvalidation;
use hana_rigging::AttemptRef;
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
use hana_rigging::DiscoveryWork;
use hana_rigging::DriverCleanupRoleEntity;
use hana_rigging::DriverCompletion;
use hana_rigging::EndpointDriver;
use hana_rigging::EstablishedContext;
use hana_rigging::MainThreadDiscoveryJob;
use hana_rigging::PlatformDeviceHandle;
use hana_rigging::Presence;
use hana_rigging::ReportAcceptanceProjection;
use hana_rigging::ReportedAs;
use hana_rigging::ReportedId;
use hana_rigging::ReportedIdError;
use hana_rigging::ReportedParent;
use hana_rigging::ReportedSerial;
use hana_rigging::ReporterActivityView;
use hana_rigging::ReporterDeferral;
use hana_rigging::ReporterHealth;
use hana_rigging::ReporterId;
use hana_rigging::RoleKey;
use hana_rigging::SchemeName;
use hana_rigging::SchemeNameError;
use hana_rigging::SessionDatumArrivalEvidence;
use hana_rigging::SessionLease;
use hana_rigging::SessionRef;
use hana_rigging::SessionReleaseCause;
use hana_rigging::TargetResolution;
use hana_rigging::TargetResolutionContext;
use thiserror::Error;

/// How long [`advance_reporter`] drives frames before it reports one requested scan as stalled.
///
/// A scheduled reporter needs one update to prepare and run its job and one more for the kernel to
/// accept the completed set, but the job itself runs off the main thread. A frame count cannot
/// bound that wait: frames are cheap enough to exhaust before a loaded machine has scheduled the
/// worker even once, which reports a run that is merely late as one that stalled. The bound is
/// elapsed time instead, and the harness keeps driving frames until it passes.
const SCAN_DEADLINE: Duration = Duration::from_secs(10);

/// A capability declaration rebuilt on demand for each replay of one scripted device.
pub(crate) type CapabilityBuilder = Arc<dyn Fn() -> Capabilities + Send + Sync>;

/// Failure building a durable key from scheme text.
#[derive(Debug, Error)]
pub enum ScriptedKeyError {
    /// The scheme name was rejected by [`hana_rigging::SchemeName`].
    #[error("scripted scheme name rejected: {0}")]
    Scheme(#[from] SchemeNameError),
    /// The reported value was rejected by [`hana_rigging::ReportedId`].
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
    /// The requested run did not complete within `SCAN_DEADLINE`.
    #[error("a scripted discovery run did not complete within {SCAN_DEADLINE:?}")]
    Stalled,
    /// No held scan reached the gate within `SCAN_DEADLINE`.
    #[error("a held scan did not reach its gate within {SCAN_DEADLINE:?}")]
    NeverHeld,
}

/// Failure selecting an authority retained by a [`ScriptedDriver`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScriptedDriverControlError {
    /// No pending attempt has this process-local reference.
    #[error("the scripted driver has no pending attempt {attempt:?}")]
    UnknownAttempt {
        /// Attempt reference supplied by the caller.
        attempt: AttemptRef,
    },
    /// No established session belongs to this role.
    #[error("the scripted driver has no established session for role `{role}`")]
    UnknownSession {
        /// Durable role key supplied by the caller.
        role: RoleKey,
    },
}

/// Endpoint driver that retains completions until its paired control finishes them.
///
/// A test advances Bevy's manual frame time, runs one update so the kernel publishes that frame
/// instant to its authorities, and then calls [`ScriptedDriverControl::finish_attempt`]. This
/// stamps the result at that scripted frame without consulting a second clock.
pub struct ScriptedDriver<Configuration> {
    state: Arc<Mutex<ScriptedDriverState<Configuration>>>,
}

/// Test-side control for completions and leases retained by a [`ScriptedDriver`].
pub struct ScriptedDriverControl<Configuration> {
    state: Arc<Mutex<ScriptedDriverState<Configuration>>>,
}

struct ScriptedDriverState<Configuration> {
    attempts:      VecDeque<(AttemptRef, AttemptCompletion<Configuration>)>,
    sessions:      Vec<(RoleKey, SessionLease<Configuration>)>,
    cancellations: Vec<(AttemptRef, AttemptInvalidation)>,
    releases:      Vec<(SessionRef, SessionReleaseCause)>,
}

impl<Configuration> Default for ScriptedDriverState<Configuration> {
    fn default() -> Self {
        Self {
            attempts:      VecDeque::new(),
            sessions:      Vec::new(),
            cancellations: Vec::new(),
            releases:      Vec::new(),
        }
    }
}

impl<Configuration> Clone for ScriptedDriverControl<Configuration> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl<Configuration> ScriptedDriver<Configuration> {
    /// Create one driver and the control that owns its scripted actions.
    #[must_use]
    pub fn new() -> (Self, ScriptedDriverControl<Configuration>) {
        let state = Arc::new(Mutex::new(ScriptedDriverState::default()));
        (
            Self {
                state: Arc::clone(&state),
            },
            ScriptedDriverControl { state },
        )
    }
}

impl<Configuration> ScriptedDriverControl<Configuration>
where
    Configuration: Reflect,
{
    /// Return the pending attempt references in dispatch order.
    #[must_use]
    pub fn pending_attempts(&self) -> Vec<AttemptRef> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .attempts
            .iter()
            .map(|(attempt, _)| *attempt)
            .collect()
    }

    /// Finish one selected attempt at the frame instant most recently published by the kernel.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptedDriverControlError::UnknownAttempt`] when the driver no longer owns that
    /// completion.
    pub fn finish_attempt(
        &self,
        attempt: AttemptRef,
        completion: DriverCompletion<Configuration>,
    ) -> Result<(), ScriptedDriverControlError> {
        let authority = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let index = state
                .attempts
                .iter()
                .position(|(candidate, _)| *candidate == attempt)
                .ok_or(ScriptedDriverControlError::UnknownAttempt { attempt })?;
            state
                .attempts
                .remove(index)
                .map(|(_, authority)| authority)
                .ok_or(ScriptedDriverControlError::UnknownAttempt { attempt })?
        };
        authority.finish(completion);
        Ok(())
    }

    /// Return every cancellation received by the driver.
    #[must_use]
    pub fn cancellations(&self) -> Vec<(AttemptRef, AttemptInvalidation)> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancellations
            .clone()
    }

    /// Return every session release received by the driver.
    #[must_use]
    pub fn releases(&self) -> Vec<(SessionRef, SessionReleaseCause)> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .releases
            .clone()
    }

    /// Return the session reference retained for one role.
    #[must_use]
    pub fn session_ref(&self, role: &RoleKey) -> Option<SessionRef> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sessions
            .iter()
            .find_map(|(candidate, lease)| (candidate == role).then(|| lease.session_ref()))
    }

    /// Report a changed configuration through the lease retained for `role`.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptedDriverControlError::UnknownSession`] when no retained lease belongs to
    /// the role.
    pub fn configuration_changed(
        &self,
        role: &RoleKey,
        configuration: Configuration,
    ) -> Result<(), ScriptedDriverControlError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let lease = state
            .sessions
            .iter_mut()
            .find_map(|(candidate, lease)| (candidate == role).then_some(lease))
            .ok_or_else(|| ScriptedDriverControlError::UnknownSession { role: role.clone() })?;
        lease.configuration_changed(configuration);
        drop(state);
        Ok(())
    }

    /// Consume the lease retained for `role` and report device-access loss.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptedDriverControlError::UnknownSession`] when no retained lease belongs to
    /// the role.
    pub fn report_loss(
        &self,
        role: &RoleKey,
        error: DeviceAccessError,
    ) -> Result<(), ScriptedDriverControlError> {
        let lease = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let index = state
                .sessions
                .iter()
                .position(|(candidate, _)| candidate == role)
                .ok_or_else(|| ScriptedDriverControlError::UnknownSession { role: role.clone() })?;
            state.sessions.remove(index).1
        };
        lease.report_loss(error);
        Ok(())
    }
}

impl<Configuration> EndpointDriver for ScriptedDriver<Configuration>
where
    Configuration: Reflect + FromReflect + Component,
{
    type Configuration = Configuration;
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
        let attempt = context.attempt();
        let completion = context.into_completion();
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .attempts
            .push_back((attempt, completion));
    }

    fn established(&mut self, _: &mut World, context: EstablishedContext<'_, Self::Configuration>) {
        let role = context.role().clone();
        let lease = context.into_lease(SessionDatumArrivalEvidence::NoDatumObserved);
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .sessions
            .push((role, lease));
    }

    fn cancel_apply(
        &mut self,
        _: &mut World,
        _: &RoleKey,
        _: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        cause: AttemptInvalidation,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cancellations.push((attempt, cause));
        if let Some(index) = state
            .attempts
            .iter()
            .position(|(candidate, _)| *candidate == attempt)
        {
            state.attempts.remove(index);
        }
    }

    fn release_session(
        &mut self,
        _: &mut World,
        role: &RoleKey,
        _: DriverCleanupRoleEntity,
        session: SessionRef,
        cause: SessionReleaseCause,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.releases.push((session, cause));
        state
            .sessions
            .retain(|(candidate, lease)| candidate != role || lease.session_ref() != session);
    }
}

/// Build the durable key a scripted reporter creates for one reported unit.
///
/// # Errors
///
/// Returns [`ScriptedKeyError`] when `scheme` or `value` is not an acceptable identity component.
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
/// parent defaults to a root, so a scan that varies only presence and claim states only those.
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

    /// Report a durably named unit whose reporter can no longer reach it.
    #[must_use]
    pub fn unreachable(device_key: DeviceKey, since: Duration) -> Self {
        Self::new(
            ReportedAs::Keyed(device_key),
            Presence::Unreachable { since },
        )
    }

    /// Report a unit the reporter recognizes but cannot name durably.
    ///
    /// Reconciliation keeps this record only when it joins a keyed one, so a scripted
    /// evidence-only device is how a test exercises the two-reporter join.
    #[must_use]
    pub fn match_evidence_only(presence: Presence) -> Self {
        Self::new(ReportedAs::MatchEvidenceOnly, presence)
    }

    /// Report a unit under any combination of naming status and reachability.
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
    /// The reporter enumerated successfully through the complete-with-projection contract.
    CompleteWithProjection(Vec<ScriptedDevice>),
    /// A reporter prerequisite is not ready yet.
    Deferred(ReporterDeferral),
    /// Enumeration failed before the reporter established its whole current set.
    Failed(DeviceAccessError),
    /// The current platform has no implementation for this reporter.
    Unsupported {
        /// Text naming the unsupported platform contract.
        detail: String,
    },
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
            Self::CompleteWithProjection(scripted_devices) => DeviceScan::CompleteWithProjection {
                devices:                      scripted_devices
                    .iter()
                    .map(ScriptedDevice::record)
                    .collect(),
                report_acceptance_projection: ReportAcceptanceProjection::new(|_| {}),
            },
            Self::Deferred(reporter_deferral) => DeviceScan::Deferred(*reporter_deferral),
            Self::Failed(device_access_error) => DeviceScan::Failed(device_access_error.clone()),
            Self::Unsupported { detail } => DeviceScan::Failed(DeviceAccessError::Unsupported {
                detail: detail.clone(),
            }),
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
    /// The scans of [`ScriptedReporter::new`] are [`DiscoveryWork::Immediate`], so they complete
    /// inside the same admission call that started them and the kernel never observes the reporter
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

/// Releases one held scan of a [`ScriptedReporter::gated`] reporter, and reports its arrival.
///
/// Releases are counted rather than signalled, so a test may release before or after the job
/// reaches the gate and neither ordering can lose the release or deadlock the I/O pool thread.
/// Arrivals are counted for the same reason, so [`ScriptedRunGate::wait_until_held`] cannot miss a
/// job that reached the gate before the caller asked.
#[derive(Clone)]
pub struct ScriptedRunGate(Arc<ScriptedRunGateState>);

struct ScriptedRunGateState {
    counts:  Mutex<ScriptedRunGateCounts>,
    changed: Condvar,
}

/// Held scans that have reached the gate, and releases no held scan has claimed yet.
#[derive(Default)]
struct ScriptedRunGateCounts {
    arrivals: usize,
    releases: usize,
}

impl ScriptedRunGate {
    fn new() -> Self {
        Self(Arc::new(ScriptedRunGateState {
            counts:  Mutex::new(ScriptedRunGateCounts::default()),
            changed: Condvar::new(),
        }))
    }

    /// Let one held scan finish and return its whole set to the kernel.
    pub fn release(&self) {
        let mut counts = self.counts();
        counts.releases += 1;
        drop(counts);
        self.0.changed.notify_all();
    }

    /// Block until one held scan has reached the gate, so what it reported is on its way.
    ///
    /// A gated job sends its progress and only then holds, and the kernel marks the reporter
    /// running on the main thread as it spawns that job — before the I/O pool has run it even
    /// once. So [`advance_until_running`] returns while the retained progress is still the
    /// `Indeterminate` placeholder, and a test that reads what the run reported has to wait for
    /// the job itself rather than for the kernel's view of it. One frame after this returns, the
    /// scheduler has drained the sent progress.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptedAdvanceError::NeverHeld`] when no scan reaches the gate within
    /// `SCAN_DEADLINE`.
    pub fn wait_until_held(&self) -> Result<(), ScriptedAdvanceError> {
        let deadline = Instant::now() + SCAN_DEADLINE;
        let mut counts = self.counts();
        while counts.arrivals == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ScriptedAdvanceError::NeverHeld);
            }
            counts = self
                .0
                .changed
                .wait_timeout(counts, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        counts.arrivals -= 1;
        drop(counts);
        Ok(())
    }

    fn wait(&self) {
        let mut counts = self.counts();
        counts.arrivals += 1;
        self.0.changed.notify_all();
        while counts.releases == 0 {
            counts = self
                .0
                .changed
                .wait(counts)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        counts.releases -= 1;
    }

    fn counts(&self) -> MutexGuard<'_, ScriptedRunGateCounts> {
        self.0
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
/// Returns [`ScriptedAdvanceError`] when the kernel refuses the request, when the reporter has no
/// retained status, or when the run has not been accepted within `SCAN_DEADLINE`.
pub fn advance_reporter(app: &mut App, reporter: ReporterId) -> Result<(), ScriptedAdvanceError> {
    let completed_before = completed_batches(app, reporter)?;
    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .request(reporter)
        .map_err(|error| ScriptedAdvanceError::RequestRefused(error.to_string()))?;

    let deadline = Instant::now() + SCAN_DEADLINE;
    while Instant::now() < deadline {
        app.update();
        if completed_batches(app, reporter)? > completed_before {
            return Ok(());
        }
    }

    Err(ScriptedAdvanceError::Stalled)
}

/// Ask one gated reporter for a run and advance frames until the kernel reports it running.
///
/// Only a [`ScriptedReporter::gated`] reporter can reach this state: an immediate scan completes
/// inside the admission call that started it, so the kernel retains no running activity for it.
/// The run is left in flight, which is what lets a caller read a batch mid-run and then release it
/// with [`ScriptedRunGate::release`] and [`advance_until_accepted`].
///
/// # Errors
///
/// Returns [`ScriptedAdvanceError`] when the kernel refuses the request, when the reporter has no
/// retained status, or when the run has not started within `SCAN_DEADLINE`.
pub fn advance_until_running(
    app: &mut App,
    reporter: ReporterId,
) -> Result<(), ScriptedAdvanceError> {
    app.world_mut()
        .resource_mut::<DiscoveryControl>()
        .request(reporter)
        .map_err(|error| ScriptedAdvanceError::RequestRefused(error.to_string()))?;

    let deadline = Instant::now() + SCAN_DEADLINE;
    while Instant::now() < deadline {
        app.update();
        if is_running(app, reporter)? {
            return Ok(());
        }
    }

    Err(ScriptedAdvanceError::Stalled)
}

/// Advance frames until the kernel has accepted the run already in flight.
///
/// The counterpart to [`advance_until_running`]: the request has already been made, so this waits
/// on the completion count alone.
///
/// # Errors
///
/// Returns [`ScriptedAdvanceError`] when the reporter has no retained status, or when the run has
/// not been accepted within `SCAN_DEADLINE`.
pub fn advance_until_accepted(
    app: &mut App,
    reporter: ReporterId,
) -> Result<(), ScriptedAdvanceError> {
    let completed_before = completed_batches(app, reporter)?;
    let deadline = Instant::now() + SCAN_DEADLINE;
    while Instant::now() < deadline {
        app.update();
        if completed_batches(app, reporter)? > completed_before {
            return Ok(());
        }
    }

    Err(ScriptedAdvanceError::Stalled)
}

/// Read whether one reporter's retained activity is a run the kernel currently holds.
fn is_running(app: &App, reporter: ReporterId) -> Result<bool, ScriptedAdvanceError> {
    reporter_health(app, reporter)
        .map(|health| matches!(health.activity(), ReporterActivityView::Running { .. }))
}

/// Read how many whole-set batches one reporter has finished accepting.
fn completed_batches(app: &App, reporter: ReporterId) -> Result<u64, ScriptedAdvanceError> {
    reporter_health(app, reporter).map(ReporterHealth::completed_runs)
}

fn reporter_health(
    app: &App,
    reporter: ReporterId,
) -> Result<&ReporterHealth, ScriptedAdvanceError> {
    app.world()
        .iter_entities()
        .filter_map(|entity| entity.get::<ReporterHealth>())
        .find(|health| health.belongs_to(reporter))
        .ok_or_else(|| {
            ScriptedAdvanceError::NoStatus(String::from("reporter has no health component"))
        })
}

/// Build a [`ScriptedScan::Complete`] from a list of scripted devices.
///
/// The macro exists so a scan reads as the whole set it is: a reporter always reports everything it
/// can currently see, and a device left out of the list is the departure evidence.
#[macro_export]
macro_rules! scan {
    [$($scripted_device:expr),* $(,)?] => {
        $crate::ScriptedScan::Complete(vec![$($scripted_device),*])
    };
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use bevy::ecs::reflect::ReflectComponent;
    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use hana_rigging::Capabilities;
    use hana_rigging::Presence;
    use hana_rigging::ReportedAs;

    use super::ScriptedDevice;

    #[derive(Component, PartialEq, Reflect)]
    #[reflect(Component)]
    struct ReplayedCapability;

    #[test]
    fn evidence_only_device_rebuilds_its_capability_for_each_replay() {
        let rebuilds = Arc::new(AtomicUsize::new(0));
        let counted_rebuilds = Arc::clone(&rebuilds);
        let device =
            ScriptedDevice::match_evidence_only(Presence::Present).with_capabilities(move || {
                counted_rebuilds.fetch_add(1, Ordering::Relaxed);
                Capabilities::new().with(ReplayedCapability)
            });

        let first = device.record();
        let second = device.record();

        assert_eq!(first.reported_as, ReportedAs::MatchEvidenceOnly);
        assert_eq!(second.reported_as, ReportedAs::MatchEvidenceOnly);
        assert_eq!(rebuilds.load(Ordering::Relaxed), 2);
    }
}
