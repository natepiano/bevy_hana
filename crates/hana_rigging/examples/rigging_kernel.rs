//! Headless smoke run of the rigging kernel: two reporters push overlapping whole-set scans into
//! a live Bevy `App`, and the kernel merges them into one device set.
//!
//! The example registers an identity scheme, registers two `DeviceReporter` implementations, and
//! reports a `DmxUniverseAddress` capability for a shared `DmxUniverse`. A `PanelDriver` resolves
//! that typed address through `TargetResolutionContext::required_capability`. The run also authors
//! one inventory entry, drives real frames until both reporters have completed a scan, then reads
//! the reconciled set out of `Devices` and the role's live device link out of `Bindings`. It
//! provokes a departure by having one reporter stop naming a later key and prints the resulting
//! connection conclusion. It touches no window, renderer, filesystem, or network.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result as FormatResult;
use std::process::ExitCode;
use std::time::Duration;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::relationship::Relationship;
use bevy::ecs::relationship::RelationshipTarget;
use bevy::prelude::Component;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::On;
use bevy::prelude::Reflect;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Update;
use bevy::prelude::World;
use hana_rigging::prelude::Applied;
use hana_rigging::prelude::ApplyContext;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::AttemptCompletion;
use hana_rigging::prelude::AttemptEndingView;
use hana_rigging::prelude::AttemptInvalidation;
use hana_rigging::prelude::AttemptOutcomeView;
use hana_rigging::prelude::AttemptRef;
use hana_rigging::prelude::AuthoritativeReporterCoverage;
use hana_rigging::prelude::BindingAuthoring;
use hana_rigging::prelude::BindingPolicy;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::Capabilities;
use hana_rigging::prelude::CapabilityProjectionFailure;
use hana_rigging::prelude::CapabilityProjectionStatus;
use hana_rigging::prelude::Claim;
use hana_rigging::prelude::ConfiguredDevice;
use hana_rigging::prelude::ConfiguredDeviceConnection;
use hana_rigging::prelude::ConfiguredDeviceMode;
use hana_rigging::prelude::ConfiguredDeviceName;
use hana_rigging::prelude::CoveredDeviceIdentitySpace;
use hana_rigging::prelude::DeviceAccessError;
use hana_rigging::prelude::DeviceArrived;
use hana_rigging::prelude::DeviceChange;
use hana_rigging::prelude::DeviceDescriptor;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DeviceIdSource;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceRecord;
use hana_rigging::prelude::DeviceReporter;
use hana_rigging::prelude::DeviceResolution;
use hana_rigging::prelude::DeviceScan;
use hana_rigging::prelude::DeviceStateLookup;
use hana_rigging::prelude::Devices;
use hana_rigging::prelude::DiscoveryCadence;
use hana_rigging::prelude::DiscoveryControl;
use hana_rigging::prelude::DiscoveryWork;
use hana_rigging::prelude::DriverCleanupRoleEntity;
use hana_rigging::prelude::DriverCompletion;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointDriverRegistration;
use hana_rigging::prelude::EstablishedContext;
use hana_rigging::prelude::FirstCompleteSetStatus;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::IdentityChanged;
use hana_rigging::prelude::LiveRoleChange;
use hana_rigging::prelude::LiveRoleChanged;
use hana_rigging::prelude::MainThreadDiscoveryJob;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::PlatformDeviceHandle;
use hana_rigging::prelude::Presence;
use hana_rigging::prelude::PreviousSuccess;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::ReportedParent;
use hana_rigging::prelude::ReportedSerial;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterHealth;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterOutcomeHealth;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::ResolvedBindings;
use hana_rigging::prelude::ResolvedToDevice;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingLimits;
use hana_rigging::prelude::RiggingPlugin;
use hana_rigging::prelude::RiggingRevision;
use hana_rigging::prelude::RiggingSystems;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleStatus;
use hana_rigging::prelude::RoleStatusView;
use hana_rigging::prelude::SchemeName;
use hana_rigging::prelude::SessionDatumArrivalEvidence;
use hana_rigging::prelude::SessionLease;
use hana_rigging::prelude::SessionRef;
use hana_rigging::prelude::SessionReleaseCause;
use hana_rigging::prelude::TargetResolution;
use hana_rigging::prelude::TargetResolutionContext;
use hana_rigging::prelude::register_binding;

const COLOR_MANAGEMENT_REPORTER: &str = "color-management";
const DESK_MONITOR: &str = "DESK-4K-0002";
const DEVICE_SCHEME: &str = "example-device-address";
const DMX_CHANNEL: u16 = 1;
const DMX_LEVEL: u8 = 255;
const DMX_UNIVERSE: u16 = 7;
/// Frames the example may spend waiting for both reporters; discovery admits a bounded number of
/// jobs per frame, so a two-reporter startup takes more than one frame.
const FRAME_CEILING: u32 = 64;
/// Application role bound to the shared DMX universe.
const PANEL_ROLE: &str = "front-lighting-panel";
/// Completed scans the window-system reporter makes before it stops naming the desk monitor.
const SCANS_BEFORE_UNPLUG: u32 = 2;
const SHARED_UNIVERSE: &str = "ARTNET-NODE-0001-U7";
const STAGE_PROJECTOR: &str = "STAGE-PROJECTOR-0003";
const WINDOW_SYSTEM_REPORTER: &str = "window-system";

/// Address the panel driver uses after reporter projection validates the capability.
#[derive(Component, Reflect, PartialEq)]
#[reflect(Component, PartialEq)]
struct DmxUniverseAddress {
    universe: u16,
}

/// Reports the same authored whole set on every scan, standing in for a platform enumeration call.
///
/// A real reporter would ask the operating system here. Every scan returns the reporter's *whole*
/// current set, because an omitted key is how a departure reaches the kernel.
struct FixedSetReporter {
    reported_keys:   Vec<DeviceKey>,
    withdrawal:      KeyWithdrawal,
    completed_scans: u32,
}

/// Whether this reporter eventually stops naming one of its keys, standing in for an unplug.
///
/// A departure is only observable because the whole set arrives every scan, so provoking one means
/// leaving a key out of a later complete report rather than sending a removal message.
enum KeyWithdrawal {
    /// The reporter names its whole authored set for the life of the run.
    Never,
    /// Once the reporter has completed `after_scans` scans it stops naming `key`.
    AfterScans {
        after_scans: u32,
        key:         DeviceKey,
    },
}

impl DeviceReporter for FixedSetReporter {
    fn discover(&mut self) -> DiscoveryWork {
        self.completed_scans += 1;
        let mut reported_keys = self.reported_keys.clone();
        if let KeyWithdrawal::AfterScans { after_scans, key } = &self.withdrawal
            && self.completed_scans > *after_scans
        {
            reported_keys.retain(|reported_key| reported_key != key);
        }

        DiscoveryWork::Immediate(MainThreadDiscoveryJob::new(move |_: &mut World| {
            DeviceScan::Complete(reported_keys.into_iter().map(present_device).collect())
        }))
    }
}

/// Endpoint driver that records what the kernel asked it to apply and completes it after the
/// kernel's apply phase.
///
/// It touches no hardware. It runs one attempt the whole way from authorization to a terminal
/// outcome, so the example can show that the kernel dispatched the role's authored DMX output and
/// no other configuration. Its state resource retains every one-use completion and session lease
/// in the same ownership locations a hardware integration uses.
struct PanelDriver {
    reporter: ReporterId,
}

struct PendingPanelApply {
    configuration: PanelDmxOutput,
    completion:    AttemptCompletion<PanelDmxOutput>,
}

/// Applying and completion-queued panel attempts, both keyed by kernel attempt reference.
#[derive(Default)]
struct PanelAttemptStore {
    applying:          HashMap<AttemptRef, PendingPanelApply>,
    completion_queued: HashMap<AttemptRef, PanelDmxOutput>,
}

/// Lease and established configuration retained for one panel role.
struct EstablishedPanelSession {
    lease:         SessionLease<PanelDmxOutput>,
    configuration: PanelDmxOutput,
}

/// Result of pairing a kernel session lease with its dispatched panel configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelSessionEstablishment {
    /// The driver retained the lease and established configuration in one role-keyed entry.
    LeaseAndConfigurationRetained,
    /// The attempt configuration was missing, so the lease reported loss and was not retained.
    MissingAttemptReported,
}

/// Driver-owned authorities and configurations for the panel role.
#[derive(Default, Resource)]
struct PanelDriverState {
    attempts:    PanelAttemptStore,
    established: HashMap<RoleKey, EstablishedPanelSession>,
}

impl PanelDriverState {
    fn retain_panel_session(
        &mut self,
        role: RoleKey,
        attempt: AttemptRef,
        lease: SessionLease<PanelDmxOutput>,
    ) -> PanelSessionEstablishment {
        let Some(configuration) = self.attempts.completion_queued.remove(&attempt) else {
            lease.report_loss(DeviceAccessError::Transport {
                detail: format!(
                    "panel attempt {} was accepted without its dispatched configuration",
                    attempt.get()
                ),
            });
            return PanelSessionEstablishment::MissingAttemptReported;
        };
        if let Some(replaced) = self.established.insert(
            role,
            EstablishedPanelSession {
                lease,
                configuration,
            },
        ) {
            let detail = format!(
                "a panel role received a replacement lease before releasing channel {} level {}",
                replaced.configuration.channel, replaced.configuration.level
            );
            replaced
                .lease
                .report_loss(DeviceAccessError::Transport { detail });
        }
        PanelSessionEstablishment::LeaseAndConfigurationRetained
    }
}

impl EndpointDriver for PanelDriver {
    type Configuration = PanelDmxOutput;
    type Target = DmxUniverseAddress;

    fn resolve_target(
        &mut self,
        world: &mut World,
        context: &TargetResolutionContext<'_>,
        _: &Self::Configuration,
    ) -> TargetResolution<Self::Target> {
        match context.required_capability::<DmxUniverseAddress>(world, self.reporter) {
            Ok(address) => TargetResolution::Reached(DmxUniverseAddress {
                universe: address.universe,
            }),
            Err(unavailable) => TargetResolution::Deferred(unavailable.target_wait(world)),
        }
    }

    fn start_apply(
        &mut self,
        world: &mut World,
        context: ApplyContext<'_, Self::Configuration>,
        configuration: &Self::Configuration,
        address: Self::Target,
    ) {
        let attempt = context.attempt();
        let completion = context.into_completion();
        world
            .resource_mut::<AppliedPanelOutputs>()
            .0
            .push(AppliedPanelOutput {
                address,
                configuration: configuration.clone(),
            });
        world
            .resource_mut::<PanelDriverState>()
            .attempts
            .applying
            .insert(
                attempt,
                PendingPanelApply {
                    configuration: configuration.clone(),
                    completion,
                },
            );
    }

    fn established(
        &mut self,
        world: &mut World,
        context: EstablishedContext<'_, Self::Configuration>,
    ) {
        let role = context.role().clone();
        let attempt = context.attempt();
        let lease = context.into_lease(SessionDatumArrivalEvidence::NoDatumObserved);
        world
            .resource_mut::<PanelDriverState>()
            .retain_panel_session(role, attempt, lease);
    }

    fn cancel_apply(
        &mut self,
        world: &mut World,
        _: &RoleKey,
        _: DriverCleanupRoleEntity,
        attempt: AttemptRef,
        _: AttemptInvalidation,
    ) {
        let mut state = world.resource_mut::<PanelDriverState>();
        drop(state.attempts.applying.remove(&attempt));
        let _ = state.attempts.completion_queued.remove(&attempt);
    }

    fn release_session(
        &mut self,
        world: &mut World,
        role: &RoleKey,
        _: DriverCleanupRoleEntity,
        session: SessionRef,
        _: SessionReleaseCause,
    ) {
        let mut state = world.resource_mut::<PanelDriverState>();
        let matches_session = state
            .established
            .get(role)
            .is_some_and(|established| established.lease.session_ref() == session);
        if matches_session
            && let Some(EstablishedPanelSession {
                lease,
                configuration: _,
            }) = state.established.remove(role)
        {
            drop(lease);
        }
    }
}

/// Finish every exact configuration the driver retained during the preceding apply phase.
fn finish_panel_applies(mut state: ResMut<PanelDriverState>) {
    let attempts = std::mem::take(&mut state.attempts.applying);
    for (
        attempt,
        PendingPanelApply {
            configuration,
            completion,
        },
    ) in attempts
    {
        state
            .attempts
            .completion_queued
            .insert(attempt, configuration);
        completion.finish(DriverCompletion::Succeeded(Applied::AsDispatched));
    }
}

/// Every DMX output `PanelDriver` was handed, in dispatch order.
///
/// The driver writes it through the `World` it is given rather than keeping it in the driver value,
/// because the driver itself lives inside the kernel's registry and the example never sees it again
/// after registration.
#[derive(Default, Resource)]
struct AppliedPanelOutputs(Vec<AppliedPanelOutput>);

/// One DMX address and output configuration dispatched to `PanelDriver`.
struct AppliedPanelOutput {
    address:       DmxUniverseAddress,
    configuration: PanelDmxOutput,
}

/// One terminal attempt outcome an observer saw, in arrival order.
struct ObservedAttemptEnding {
    role:    RoleKey,
    attempt: AttemptRef,
    ending:  AttemptEndingView,
}

/// Every attempt ending observed on a binding entity this run.
#[derive(Default, Resource)]
struct ObservedAttemptEndings(Vec<ObservedAttemptEnding>);

/// Record one attempt ending that reached a live role's binding entity.
fn observe_attempt_ending(
    live_role_changed: On<LiveRoleChanged>,
    mut observed_attempt_endings: ResMut<ObservedAttemptEndings>,
) {
    let LiveRoleChange::AttemptEnded { attempt, ending } = &live_role_changed.change else {
        return;
    };
    observed_attempt_endings.0.push(ObservedAttemptEnding {
        role:    live_role_changed.role.clone(),
        attempt: *attempt,
        ending:  ending.clone(),
    });
}

/// State domain changed by one kernel lifecycle event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KernelLifecycleAxis {
    DeviceArrival,
    DeviceIdentity,
    DeviceAvailability,
    RoleStatus,
}

impl KernelLifecycleAxis {
    const EXPECTED_IN_THIS_RUN: [Self; 4] = [
        Self::DeviceArrival,
        Self::DeviceIdentity,
        Self::DeviceAvailability,
        Self::RoleStatus,
    ];
}

impl Display for KernelLifecycleAxis {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult {
        formatter.write_str(match self {
            Self::DeviceArrival => "device arrival",
            Self::DeviceIdentity => "device identity",
            Self::DeviceAvailability => "device availability",
            Self::RoleStatus => "role status",
        })
    }
}

/// One kernel lifecycle event this run observed, in arrival order.
///
/// The kernel reports every state change as an event, so a consumer that wants to react to a
/// device arriving, a claim moving, or a role losing its device writes an observer instead of
/// polling a resource. This example collects them into one list so the run can print the whole
/// lifecycle in the order it happened.
struct ObservedLifecycleEvent {
    /// Which kernel state domain changed.
    axis:  KernelLifecycleAxis,
    /// What the axis moved to, and which device or role it belongs to.
    moved: String,
}

/// Every lifecycle event observed this run.
#[derive(Default, Resource)]
struct ObservedLifecycle(Vec<ObservedLifecycleEvent>);

impl ObservedLifecycle {
    fn record(&mut self, axis: KernelLifecycleAxis, moved: String) {
        self.0.push(ObservedLifecycleEvent { axis, moved });
    }

    /// Report whether any event on `axis` reached this run.
    fn saw(&self, axis: KernelLifecycleAxis) -> bool {
        self.0
            .iter()
            .any(|observed_lifecycle_event| observed_lifecycle_event.axis == axis)
    }
}

fn observe_device_arrived(
    device_arrived: On<DeviceArrived>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = describe_key(&device_arrived.key);
    observed_lifecycle.record(KernelLifecycleAxis::DeviceArrival, moved);
}

fn observe_identity_changed(
    identity_changed: On<IdentityChanged>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!("{:?}", identity_changed.verdict);
    observed_lifecycle.record(KernelLifecycleAxis::DeviceIdentity, moved);
}

fn observe_device_departed(
    device_change: On<DeviceChange>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let DeviceChange::Availability { key, from, to } = device_change.event();
    let moved = format!("{} | {from:?} -> {to:?}", describe_key(key));
    observed_lifecycle.record(KernelLifecycleAxis::DeviceAvailability, moved);
}

fn observe_role_status_changed(
    live_role_changed: On<LiveRoleChanged>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let LiveRoleChange::Status { to, .. } = &live_role_changed.change else {
        return;
    };
    observed_lifecycle.record(
        KernelLifecycleAxis::RoleStatus,
        format!("role `{}` | {:?}", live_role_changed.role, to.view()),
    );
}

/// How the bounded wait for a successful apply on the panel role ended.
enum AttemptRun {
    /// An attempt succeeded, this many frames after the reporters completed.
    Succeeded { frames: u32 },
    /// The frame ceiling arrived before an attempt succeeded.
    CeilingReached,
}

/// DMX channel value the panel role asks its driver to establish.
#[derive(Clone, Component, Reflect)]
#[reflect(Component)]
struct PanelDmxOutput {
    channel: u16,
    level:   u8,
}

/// One registered reporter and the name the report lines print beside its `ReporterId`.
struct NamedReporter {
    id:   ReporterId,
    name: &'static str,
}

/// One device this example reports, and how many reporters should end up contributing to it.
struct ReportedDevice {
    key:                   DeviceKey,
    label:                 &'static str,
    expected_contributors: usize,
}

/// Whether every registered reporter has had a completed whole-set scan accepted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanCoverage {
    Pending,
    EveryReporterCompleted,
}

/// How the bounded frame loop ended.
enum ReporterStartup {
    /// Every reporter completed a scan, and the reconcile pass of this frame merged their sets.
    Completed { frames: u32 },
    /// The frame ceiling arrived first, so the kernel never reached the expected state.
    CeilingReached,
}

/// What checking the reconciled set against the reported overlap concluded.
enum SmokeCheck {
    Matched,
    Mismatched(Vec<String>),
}

fn main() -> ExitCode {
    match run() {
        Ok(SmokeCheck::Matched) => {
            println!(
                "OK — three reconciled devices, the DMX universe carries two contributing \
                 reporters and a live ResolvedToDevice link, no duplicate keys, the panel role's \
                 typed-address apply attempt succeeded and left the role Established, and the withdrawn \
                 desk monitor departed and left its authored inventory entry reading Absent"
            );
            ExitCode::SUCCESS
        },
        Ok(SmokeCheck::Mismatched(mismatches)) => {
            println!("FAILED — {}", mismatches.join("; "));
            ExitCode::FAILURE
        },
        Err(error) => {
            println!("FAILED — the kernel rejected this example's setup: {error}");
            ExitCode::FAILURE
        },
    }
}

/// The three reported devices the run checks, with how many reporters each one is expected to
/// reach the reconciled set through.
fn reported_devices(
    shared_universe: DeviceKey,
    desk_monitor: DeviceKey,
    stage_projector: DeviceKey,
) -> Vec<ReportedDevice> {
    vec![
        ReportedDevice {
            key:                   shared_universe,
            label:                 "DMX universe (reported by both)",
            expected_contributors: 2,
        },
        ReportedDevice {
            key:                   desk_monitor,
            label:                 "desk monitor (window-system only)",
            expected_contributors: 1,
        },
        ReportedDevice {
            key:                   stage_projector,
            label:                 "stage projector (color-management only)",
            expected_contributors: 1,
        },
    ]
}

fn run() -> Result<SmokeCheck, Box<dyn Error>> {
    let shared_universe = reported_device_key(DeviceKind::DmxUniverse, SHARED_UNIVERSE)?;
    let desk_monitor = reported_device_key(DeviceKind::Display, DESK_MONITOR)?;
    let stage_projector = reported_device_key(DeviceKind::Display, STAGE_PROJECTOR)?;

    let mut app = App::new();
    // `RiggingPlugin` preserves an existing `RiggingLimits`. With `departure_grace` set to zero, a
    // confirmed absence retires on the next kernel pass, within the bounded frame loop.
    app.add_plugins(MinimalPlugins)
        .insert_resource(RiggingLimits {
            departure_grace: Duration::ZERO,
            ..Default::default()
        })
        .add_plugins(RiggingPlugin);
    // An unregistered scheme is rejected at the ingest boundary, so the identity space the reported
    // keys name is registered before any reporter can report one.
    app.register_device_scheme(SchemeName::new(DEVICE_SCHEME)?)
        .init_resource::<AppliedPanelOutputs>()
        .init_resource::<PanelDriverState>()
        .init_resource::<ObservedAttemptEndings>()
        .init_resource::<ObservedLifecycle>()
        .add_systems(Update, finish_panel_applies.after(RiggingSystems::Apply))
        .add_observer(observe_attempt_ending)
        .add_observer(observe_device_arrived)
        .add_observer(observe_identity_changed)
        .add_observer(observe_device_departed)
        .add_observer(observe_role_status_changed);

    // Two independent platform sources both report the DMX universe with the same typed address.
    // Each one also enumerates a display the other never sees.
    // Authoring the desk monitor into inventory neither enables a reporter nor creates a device:
    // it is what gives the unit a connection conclusion to move when discovery stops naming it.
    app.world_mut()
        .resource_mut::<HardwareInventory>()
        .configure(ConfiguredDevice {
            key:  desk_monitor.clone(),
            mode: ConfiguredDeviceMode::Managed,
            name: ConfiguredDeviceName::NeverDerived,
        });

    let window_system_reporter = app.add_device_reporter(
        FixedSetReporter {
            reported_keys:   vec![shared_universe.clone(), desk_monitor.clone()],
            withdrawal:      KeyWithdrawal::AfterScans {
                after_scans: SCANS_BEFORE_UNPLUG,
                key:         desk_monitor.clone(),
            },
            completed_scans: 0,
        },
        on_demand_registration()?,
    );
    let color_management_reporter = app.add_device_reporter(
        FixedSetReporter {
            reported_keys:   vec![shared_universe.clone(), stage_projector.clone()],
            withdrawal:      KeyWithdrawal::Never,
            completed_scans: 0,
        },
        on_demand_registration()?,
    );
    let reporters = vec![
        NamedReporter {
            name: WINDOW_SYSTEM_REPORTER,
            id:   window_system_reporter,
        },
        NamedReporter {
            name: COLOR_MANAGEMENT_REPORTER,
            id:   color_management_reporter,
        },
    ];

    // Binding registration is application work. `PanelDriver` retains the reporter whose accepted
    // projection must support `DmxUniverseAddress` before the kernel dispatches an attempt.
    let panel_role = RoleKey::new(PANEL_ROLE)?;
    let panel_driver = app.add_endpoint_driver(PanelDriver {
        reporter: window_system_reporter,
    });
    register_binding(
        app.world_mut(),
        panel_binding(panel_role.clone(), shared_universe.clone(), panel_driver),
    )?;

    let reported_devices = reported_devices(shared_universe, desk_monitor.clone(), stage_projector);

    for named_reporter in &reporters {
        println!(
            "registered reporter: {} {:?}",
            named_reporter.name, named_reporter.id
        );
    }

    match run_until_reporters_complete(&mut app, &reporters) {
        ReporterStartup::CeilingReached => Ok(SmokeCheck::Mismatched(vec![format!(
            "reached the {FRAME_CEILING}-frame ceiling before every reporter completed a scan"
        )])),
        ReporterStartup::Completed { frames } => {
            println!("frames until every reporter completed a scan: {frames}");
            print_reporter_diagnostics(app.world(), &reporters);
            let devices = app.world().resource::<Devices>();
            print_reconciled_devices(devices, &reported_devices, &reporters);
            println!(
                "rigging revision: {}",
                app.world().resource::<RiggingRevision>().get()
            );
            let mut smoke_check = check_reconciled(devices, &reported_devices);
            print_binding(app.world(), &panel_role, &mut smoke_check);
            print_connection(app.world(), &desk_monitor, "before the unplug");

            report_attempt(&mut app, &panel_role, &mut smoke_check);

            provoke_departure(
                &mut app,
                &reporters,
                &desk_monitor,
                &reported_devices,
                &mut smoke_check,
            );
            report_lifecycle(app.world(), &mut smoke_check);

            Ok(smoke_check)
        },
    }
}

/// Drive frames until the withdrawn key leaves the reconciled set, then report what followed.
///
/// The departure is the whole point of the run: nothing tells the kernel a display was unplugged,
/// it simply stops appearing in a complete scan, and the authored inventory entry moves from
/// `Present` to `Absent` because the reporter that omitted it enumerates its identity space.
fn provoke_departure(
    app: &mut App,
    reporters: &[NamedReporter],
    departing: &DeviceKey,
    reported_devices: &[ReportedDevice],
    smoke_check: &mut SmokeCheck,
) {
    for frame in 1..=FRAME_CEILING {
        request_one_scan(app, reporters, smoke_check);
        app.update();
        if app.world().resource::<Devices>().resolve(departing) == DeviceResolution::NotResolved {
            println!("frames until the withdrawn key left the reconciled set: {frame}");
            let remaining: Vec<&ReportedDevice> = reported_devices
                .iter()
                .filter(|reported_device| &reported_device.key != departing)
                .collect();
            let devices = app.world().resource::<Devices>();
            println!(
                "reconciled devices after the departure: {}",
                devices.count()
            );
            if devices.count() != remaining.len() {
                record_mismatch(
                    smoke_check,
                    format!(
                        "expected {} reconciled devices after the departure, kernel retained {}",
                        remaining.len(),
                        devices.count()
                    ),
                );
            }
            print_connection(app.world(), departing, "after the unplug");
            let connection = app
                .world()
                .resource::<HardwareInventory>()
                .connection(departing);
            if connection != Ok(ConfiguredDeviceConnection::Absent) {
                record_mismatch(
                    smoke_check,
                    format!(
                        "expected the authored desk monitor to read Absent after the unplug, got \
                         {connection:?}"
                    ),
                );
            }

            return;
        }
    }

    record_mismatch(
        smoke_check,
        format!(
            "reached the {FRAME_CEILING}-frame ceiling before the withdrawn key left the \
             reconciled set"
        ),
    );
}

/// Drive frames until a DMX apply on the panel role succeeds, then print what the attempts did.
///
/// Nothing in the example dispatches the apply: the kernel authorizes it once reconciliation
/// resolves the role's durable endpoint to a present device, and every terminal outcome arrives as
/// an event on the binding entity rather than as a return value. Earlier attempts can end
/// as `AttemptEndingView::Invalidated` while the reported set is still settling, because an attempt
/// authorized against one rigging revision cannot continue once a later scan advances it.
fn report_attempt(app: &mut App, role: &RoleKey, smoke_check: &mut SmokeCheck) {
    let AttemptRun::Succeeded { frames } = run_until_attempt_succeeded(app) else {
        record_mismatch(
            smoke_check,
            format!(
                "reached the {FRAME_CEILING}-frame ceiling before an apply on role `{role}` \
                 succeeded"
            ),
        );
        return;
    };

    print_attempt_report(app, frames);
    check_success_named_the_role(app, role, smoke_check);
    check_driver_was_handed_the_apply(app, role, smoke_check);
    check_role_established(app, role, smoke_check);
    check_retained_configuration(app, role, smoke_check);
}

/// Print what the run dispatched and every attempt ending it observed.
fn print_attempt_report(app: &App, frames: u32) {
    println!("frames until a DMX apply on the panel role succeeded: {frames}");
    println!(
        "DMX outputs dispatched to the driver: {}",
        app.world()
            .resource::<AppliedPanelOutputs>()
            .0
            .iter()
            .map(describe_panel_output)
            .collect::<Vec<_>>()
            .join(", ")
    );
    for observed_attempt_ending in &app.world().resource::<ObservedAttemptEndings>().0 {
        println!(
            "  attempt ending | role `{}` | {:?} | {:?}",
            observed_attempt_ending.role,
            observed_attempt_ending.attempt,
            observed_attempt_ending.ending
        );
    }
}

/// The successful attempt must have reached the binding entity of the role under test.
fn check_success_named_the_role(app: &App, role: &RoleKey, smoke_check: &mut SmokeCheck) {
    let succeeded = app
        .world()
        .resource::<ObservedAttemptEndings>()
        .0
        .iter()
        .find(|observed_attempt_ending| attempt_succeeded(&observed_attempt_ending.ending))
        .map(|observed_attempt_ending| observed_attempt_ending.role.clone());
    match succeeded {
        None => record_mismatch(
            smoke_check,
            format!("role `{role}`'s successful attempt never reached its binding entity"),
        ),
        Some(succeeded_role) if succeeded_role != *role => record_mismatch(
            smoke_check,
            format!(
                "expected the successful attempt to name role `{role}`, got `{succeeded_role}`"
            ),
        ),
        Some(_) => {},
    }
}

/// A reported success with no dispatched output would mean the driver never saw the apply.
fn check_driver_was_handed_the_apply(app: &App, role: &RoleKey, smoke_check: &mut SmokeCheck) {
    if app.world().resource::<AppliedPanelOutputs>().0.is_empty() {
        record_mismatch(
            smoke_check,
            format!("role `{role}` reported a successful apply the driver was never handed"),
        );
    }
}

/// After a successful apply the role's authoritative status must be `Established`.
fn check_role_established(app: &App, role: &RoleKey, smoke_check: &mut SmokeCheck) {
    let role_status = app
        .world()
        .resource::<Bindings>()
        .role_entity(role)
        .ok()
        .and_then(|entity| app.world().get::<RoleStatus>(entity))
        .map(RoleStatus::view);
    match role_status {
        Some(RoleStatusView::Established { .. }) => {},
        Some(status) => record_mismatch(
            smoke_check,
            format!(
                "expected role `{role}` to be established after a successful apply, got {status:?}"
            ),
        ),
        None => record_mismatch(
            smoke_check,
            format!("role `{role}` has no authoritative status after its apply"),
        ),
    }
}

/// The driver must retain one lease-and-configuration entry matching what was dispatched.
fn check_retained_configuration(app: &App, role: &RoleKey, smoke_check: &mut SmokeCheck) {
    let driver_state = app.world().resource::<PanelDriverState>();
    println!(
        "driver-retained established sessions: {}",
        driver_state.established.len()
    );
    match driver_state.established.get(role) {
        None => record_mismatch(
            smoke_check,
            format!(
                "role `{role}` reached Established without one retained lease-and-configuration entry"
            ),
        ),
        Some(established)
            if established.configuration.channel != DMX_CHANNEL
                || established.configuration.level != DMX_LEVEL =>
        {
            record_mismatch(
                smoke_check,
                format!(
                    "role `{role}` retained channel {} level {} after dispatching channel \
                     {DMX_CHANNEL} level {DMX_LEVEL}",
                    established.configuration.channel, established.configuration.level
                ),
            );
        },
        Some(_) => {},
    }
}

/// Drive frames until a successful attempt ending has been observed, or the ceiling arrives.
fn run_until_attempt_succeeded(app: &mut App) -> AttemptRun {
    for frame in 0..FRAME_CEILING {
        if app
            .world()
            .resource::<ObservedAttemptEndings>()
            .0
            .iter()
            .any(|observed_attempt_ending| attempt_succeeded(&observed_attempt_ending.ending))
        {
            return AttemptRun::Succeeded { frames: frame };
        }
        app.update();
    }

    AttemptRun::CeilingReached
}

const fn attempt_succeeded(ending: &AttemptEndingView) -> bool {
    matches!(
        ending,
        AttemptEndingView::Reported(AttemptOutcomeView::Succeeded(_))
    )
}

/// Ask every registered reporter for one more run.
///
/// The reporters are on demand, so nothing runs them on a timer: an integration that refreshes on a
/// notification or a button press asks for a run exactly like this.
fn request_one_scan(app: &mut App, reporters: &[NamedReporter], smoke_check: &mut SmokeCheck) {
    let mut discovery_control = app.world_mut().resource_mut::<DiscoveryControl>();
    for named_reporter in reporters {
        if let Err(error) = discovery_control.request(named_reporter.id) {
            record_mismatch(
                smoke_check,
                format!(
                    "the kernel refused a discovery run for reporter {}: {error}",
                    named_reporter.name
                ),
            );
        }
    }
}

/// Print what passive evidence currently says about one authored inventory key.
fn print_connection(world: &World, device_key: &DeviceKey, when: &str) {
    match world.resource::<HardwareInventory>().connection(device_key) {
        Ok(connection) => println!(
            "  authored inventory {} | {when} | {connection:?}",
            describe_key(device_key)
        ),
        Err(error) => println!("  authored inventory read failed {when}: {error}"),
    }
}

/// Author one panel role that owns the whole DMX universe, with the default retention policy.
fn panel_binding(
    role: RoleKey,
    device: DeviceKey,
    driver: EndpointDriverRegistration<PanelDmxOutput>,
) -> BindingAuthoring<PanelDmxOutput> {
    BindingAuthoring::new(
        role,
        DeviceEndpoint::whole(device),
        driver,
        PanelDmxOutput {
            channel: DMX_CHANNEL,
            level:   DMX_LEVEL,
        },
        BindingPolicy::new(
            RecoveryPolicy::default(),
            RetryOn::NewRevision,
            OnAbort::default(),
            OnSessionLoss::default(),
            ApplyDeadline::ProcessDefault,
        ),
    )
}

fn describe_panel_output(applied: &AppliedPanelOutput) -> String {
    format!(
        "universe {} channel {} level {}",
        applied.address.universe, applied.configuration.channel, applied.configuration.level
    )
}

/// Register a reporter that runs only when this example asks it to, and whose first complete scan
/// gates readiness.
///
/// On demand rather than periodic so the run is deterministic and frame-rate independent: this
/// example asks for exactly the scans whose results it goes on to print, where a reporter
/// submitting a set on every frame would let the frame rate decide which stage each line describes.
///
/// The coverage is authoritative for this example's identity space: without it a reporter leaving a
/// key out of a complete scan would prove nothing, and the authored inventory entry could never
/// move off `NotObserved`.
fn on_demand_registration() -> Result<ReporterRegistration, Box<dyn Error>> {
    Ok(ReporterRegistration::required(
        DiscoveryCadence::OnDemand,
        ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
            CoveredDeviceIdentitySpace::ReportedScheme {
                kind:   DeviceKind::Display,
                scheme: SchemeName::new(DEVICE_SCHEME)?,
            },
        )),
        std::time::Duration::from_secs(10),
    ))
}

fn reported_device_key(kind: DeviceKind, value: &str) -> Result<DeviceKey, Box<dyn Error>> {
    Ok(DeviceKey::reported(
        kind,
        SchemeName::new(DEVICE_SCHEME)?,
        ReportedId::new(value)?,
    ))
}

/// Build the record a reporter hands the kernel for one reachable device it can name durably.
fn present_device(device_key: DeviceKey) -> DeviceRecord {
    let capabilities = match device_key.kind {
        DeviceKind::DmxUniverse => Capabilities::new().with(DmxUniverseAddress {
            universe: DMX_UNIVERSE,
        }),
        DeviceKind::Display
        | DeviceKind::Camera
        | DeviceKind::AudioInterface
        | DeviceKind::ControlSurface => Capabilities::new(),
    };
    DeviceRecord::keyed(
        device_key,
        ReportedParent::Root,
        Presence::Present,
        Claim::NotApplicable,
        capabilities,
        ReportedSerial::NotExposedByUnit,
        PlatformDeviceHandle::PlatformReportedNothing,
        AttachmentPath::PlatformHasNoConcept,
        DeviceDescriptor::PlatformReportedNothing,
    )
}

/// Drive frames until every reporter's whole set has been accepted, or the ceiling arrives.
///
/// `RiggingSystems::Reconcile` is chained after `RiggingSystems::Collect`, so the frame that
/// accepts the last outstanding scan is also the frame that merges it.
fn run_until_reporters_complete(app: &mut App, reporters: &[NamedReporter]) -> ReporterStartup {
    for frame in 1..=FRAME_CEILING {
        app.update();
        if scan_coverage(app.world(), reporters) == ScanCoverage::EveryReporterCompleted {
            return ReporterStartup::Completed { frames: frame };
        }
    }

    ReporterStartup::CeilingReached
}

fn scan_coverage(world: &World, reporters: &[NamedReporter]) -> ScanCoverage {
    for named_reporter in reporters {
        let completed = world.iter_entities().any(|entity| {
            entity.get::<ReporterHealth>().is_some_and(|health| {
                health.belongs_to(named_reporter.id)
                    && matches!(
                        health.first_complete_set(),
                        FirstCompleteSetStatus::Completed { .. }
                    )
            })
        });
        if !completed {
            return ScanCoverage::Pending;
        }
    }

    ScanCoverage::EveryReporterCompleted
}

fn print_reporter_diagnostics(world: &World, reporters: &[NamedReporter]) {
    for named_reporter in reporters {
        let Some(health) = world.iter_entities().find_map(|entity| {
            entity
                .get::<ReporterHealth>()
                .filter(|health| health.belongs_to(named_reporter.id))
        }) else {
            println!("reporter {} has no health component", named_reporter.name);
            continue;
        };
        let projection = match health.outcome() {
            ReporterOutcomeHealth::Succeeded {
                capability_projection,
                ..
            } => Some(capability_projection),
            ReporterOutcomeHealth::Failing { run, .. } => {
                if let PreviousSuccess::At {
                    capability_projection,
                    ..
                } = &run.previous_success
                {
                    println!(
                        "reporter {} currently failed: {:?}",
                        named_reporter.name, run.error
                    );
                    Some(capability_projection)
                } else {
                    None
                }
            },
            ReporterOutcomeHealth::NotCompleted
            | ReporterOutcomeHealth::Deferred { .. }
            | ReporterOutcomeHealth::Unsupported { .. } => None,
        };
        let Some(CapabilityProjectionStatus::Failed(failures)) = projection else {
            continue;
        };
        for failure in failures.entries() {
            match failure {
                CapabilityProjectionFailure::ReflectComponentNotRegistered { type_path } => {
                    println!(
                        "reporter {} accepted its complete set but dropped capability \
                         {type_path}; repair: register this type in your app",
                        named_reporter.name
                    );
                },
                CapabilityProjectionFailure::ApplicationTypeRegistryUnavailable {
                    affected_type_path,
                } => {
                    println!(
                        "reporter {} accepted its complete set but dropped capability \
                         {affected_type_path}; repair: restore the AppTypeRegistry resource",
                        named_reporter.name
                    );
                },
            }
        }
    }
}

fn print_reconciled_devices(
    devices: &Devices,
    reported_devices: &[ReportedDevice],
    reporters: &[NamedReporter],
) {
    println!("reconciled devices: {}", devices.count());
    for reported_device in reported_devices {
        let key = describe_key(&reported_device.key);
        match devices.resolve(&reported_device.key) {
            DeviceResolution::NotResolved => {
                println!(
                    "  {key} | {} | no device handle was issued",
                    reported_device.label
                );
            },
            DeviceResolution::Resolved(device_id) => match devices.state(device_id) {
                DeviceStateLookup::Retired => {
                    println!(
                        "  {key} | {} | DeviceId({}) | retired",
                        reported_device.label,
                        device_id.get()
                    );
                },
                DeviceStateLookup::Retained(reconciled_device_state) => {
                    println!(
                        "  {key} | {} | DeviceId({}) | {:?} | {:?} | contributors: {}",
                        reported_device.label,
                        device_id.get(),
                        reconciled_device_state.presence,
                        reconciled_device_state.verdict,
                        contributor_names(&reconciled_device_state.contributors, reporters)
                    );
                },
            },
        }
    }

    let duplicate_keys = devices.duplicate_keys();
    if duplicate_keys.is_empty() {
        println!("duplicate keys: none");
    } else {
        for duplicate_key in duplicate_keys {
            println!("duplicate key: {}", describe_key(duplicate_key));
        }
    }
}

fn check_reconciled(devices: &Devices, reported_devices: &[ReportedDevice]) -> SmokeCheck {
    let mut mismatches = Vec::new();

    if devices.count() != reported_devices.len() {
        mismatches.push(format!(
            "expected {} reconciled devices, kernel retained {}",
            reported_devices.len(),
            devices.count()
        ));
    }
    for reported_device in reported_devices {
        let contributors = match devices.resolve(&reported_device.key) {
            DeviceResolution::NotResolved => {
                mismatches.push(format!("{} resolved to no device", reported_device.label));
                continue;
            },
            DeviceResolution::Resolved(device_id) => match devices.state(device_id) {
                DeviceStateLookup::Retired => {
                    mismatches.push(format!(
                        "{} resolved to a retired handle",
                        reported_device.label
                    ));
                    continue;
                },
                DeviceStateLookup::Retained(reconciled_device_state) => {
                    reconciled_device_state.contributors.len()
                },
            },
        };
        if contributors != reported_device.expected_contributors {
            mismatches.push(format!(
                "{} expected {} contributing reporters, got {contributors}",
                reported_device.label, reported_device.expected_contributors
            ));
        }
    }
    if !devices.duplicate_keys().is_empty() {
        mismatches.push(format!(
            "expected no duplicate keys, kernel reported {}",
            devices.duplicate_keys().len()
        ));
    }

    if mismatches.is_empty() {
        SmokeCheck::Matched
    } else {
        SmokeCheck::Mismatched(mismatches)
    }
}

/// Print the role's binding entity and its live link to a device entity, if it has one.
///
/// A registered role reports no live link until reconciliation resolves its durable endpoint to a
/// device entity, which can be true even while its display is present. The example says which of
/// the two it observed rather than implying the link failed.
fn print_binding(world: &World, role: &RoleKey, smoke_check: &mut SmokeCheck) {
    let bindings = world.resource::<Bindings>();
    println!("registered role: {role}");
    match bindings.role_entity(role) {
        Err(_) => {
            record_mismatch(
                smoke_check,
                format!("role `{role}` has no binding entity after registration"),
            );
        },
        Ok(entity) => {
            match (
                world.get::<RecoveryPolicy>(entity),
                world.get::<RoleStatus>(entity),
            ) {
                (Some(recovery_policy), Some(role_state)) => {
                    println!(
                        "  role `{role}` | {entity} | {recovery_policy:?} | {:?}",
                        role_state.view()
                    );
                },
                _ => record_mismatch(
                    smoke_check,
                    format!(
                        "role `{role}`'s binding entity is missing a mirrored recovery policy or \
                         role state"
                    ),
                ),
            }
            match world.get::<ResolvedToDevice>(entity) {
                None => record_mismatch(
                    smoke_check,
                    format!(
                        "role `{role}` has no ResolvedToDevice link even though its endpoint names \
                         a device the kernel retains"
                    ),
                ),
                Some(resolved_to_device) => {
                    let device = resolved_to_device.get();
                    let resolved_bindings = world
                        .get::<ResolvedBindings>(device)
                        .map_or(0, RelationshipTarget::len);
                    println!(
                        "  role `{role}` | ResolvedToDevice({device}) | that device carries \
                         {resolved_bindings} resolved binding(s)"
                    );
                },
            }
        },
    }
}

/// Print every lifecycle event this run observed, and fail the run if an expected axis was silent.
///
/// The four axes checked here are the ones this run moves: devices arrive and gain identity and
/// availability conclusions, while the panel role changes status. The
/// unplug is a `KernelLifecycleAxis::DeviceAvailability` transition; the retirement and authored
/// `ConfiguredDeviceConnection::Absent` checks above establish that the departure completed. An
/// axis that stayed silent means a consumer watching only events missed a change the resources
/// went on to report.
fn report_lifecycle(world: &World, smoke_check: &mut SmokeCheck) {
    println!("kernel lifecycle events, in arrival order:");
    let observed_lifecycle = world.resource::<ObservedLifecycle>();
    for observed_lifecycle_event in &observed_lifecycle.0 {
        println!(
            "  {} | {}",
            observed_lifecycle_event.axis, observed_lifecycle_event.moved
        );
    }
    for axis in KernelLifecycleAxis::EXPECTED_IN_THIS_RUN {
        if !observed_lifecycle.saw(axis) {
            record_mismatch(
                smoke_check,
                format!("no {axis} event reached an observer during this run"),
            );
        }
    }
}

fn record_mismatch(smoke_check: &mut SmokeCheck, mismatch: String) {
    match smoke_check {
        SmokeCheck::Matched => *smoke_check = SmokeCheck::Mismatched(vec![mismatch]),
        SmokeCheck::Mismatched(mismatches) => mismatches.push(mismatch),
    }
}

fn contributor_names(contributors: &[ReporterId], reporters: &[NamedReporter]) -> String {
    contributors
        .iter()
        .map(|contributor| {
            reporters
                .iter()
                .find(|named_reporter| named_reporter.id == *contributor)
                .map_or_else(
                    || format!("{contributor:?}"),
                    |named_reporter| format!("{} {contributor:?}", named_reporter.name),
                )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn describe_key(device_key: &DeviceKey) -> String {
    match &device_key.id {
        DeviceIdSource::Reported { scheme, value } => {
            format!(
                "{:?} {}:{}",
                device_key.kind,
                scheme.as_str(),
                value.as_str()
            )
        },
        DeviceIdSource::Synthesized { digest } => {
            format!("{:?} synthesized:{digest:?}", device_key.kind)
        },
        DeviceIdSource::Authored { value } => {
            format!("{:?} authored:{}", device_key.kind, value.as_str())
        },
    }
}
