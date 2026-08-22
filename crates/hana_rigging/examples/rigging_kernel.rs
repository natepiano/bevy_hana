//! Headless smoke run of the rigging kernel: two reporters push overlapping whole-set scans into
//! a live Bevy `App`, and the kernel merges them into one device set.
//!
//! The example registers an identity scheme, registers two `DeviceReporter` implementations, one
//! authored role binding, and one authored inventory entry, drives real frames until both reporters
//! have completed a scan, then reads the reconciled set out of `Devices` and the role's binding
//! entity and its live device link out of `BindingEntities`. It then provokes a departure by having
//! one reporter stop naming a key in a later complete scan, and prints the connection conclusion
//! that moves as a result. It touches no window, renderer, filesystem, or network.

use std::error::Error;
use std::process::ExitCode;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::relationship::Relationship;
use bevy::ecs::relationship::RelationshipTarget;
use bevy::prelude::Component;
use bevy::prelude::On;
use bevy::prelude::Reflect;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::World;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::ApplyPermit;
use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::AttemptFinished;
use hana_rigging::prelude::AttemptId;
use hana_rigging::prelude::AttemptOutcome;
use hana_rigging::prelude::AttemptProgress;
use hana_rigging::prelude::AuthoritativeReporterCoverage;
use hana_rigging::prelude::Binding;
use hana_rigging::prelude::BindingEntities;
use hana_rigging::prelude::BindingEntityLookup;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::Capabilities;
use hana_rigging::prelude::CaptureOutcome;
use hana_rigging::prelude::Claim;
use hana_rigging::prelude::ClaimChanged;
use hana_rigging::prelude::ConfiguredDevice;
use hana_rigging::prelude::ConfiguredDeviceConnection;
use hana_rigging::prelude::ConfiguredDeviceConnectionChanged;
use hana_rigging::prelude::ConfiguredDeviceMode;
use hana_rigging::prelude::CoveredDeviceIdentitySpace;
use hana_rigging::prelude::DeviceArrived;
use hana_rigging::prelude::DeviceDeparted;
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
use hana_rigging::prelude::DiscoveryStatus;
use hana_rigging::prelude::DiscoveryWork;
use hana_rigging::prelude::DriverId;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::IdentityChanged;
use hana_rigging::prelude::LastDiscoveryOutcome;
use hana_rigging::prelude::LastKnownGoodConfiguration;
use hana_rigging::prelude::MainThreadDiscoveryJob;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::PlatformDeviceHandle;
use hana_rigging::prelude::Presence;
use hana_rigging::prelude::PresenceChanged;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::ReportedAs;
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::ReportedParent;
use hana_rigging::prelude::ReportedSerial;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::RequestedConfiguration;
use hana_rigging::prelude::ResolvedBindings;
use hana_rigging::prelude::ResolvedToDevice;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingPlugin;
use hana_rigging::prelude::RiggingRevision;
use hana_rigging::prelude::RoleAvailable;
use hana_rigging::prelude::RoleAwaiting;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleState;
use hana_rigging::prelude::RoleStateChanged;
use hana_rigging::prelude::SchemeName;

const COLOR_MANAGEMENT_REPORTER: &str = "color-management";
/// Application role bound to the built-in panel, standing in for a window whose placement outlives
/// the display it was on.
const PANEL_ROLE: &str = "primary-window";
const DESK_MONITOR: &str = "DESK-4K-0002";
const DISPLAY_SCHEME: &str = "example-edid-serial";
/// Frames the example may spend waiting for both reporters; discovery admits a bounded number of
/// jobs per frame, so a two-reporter startup takes more than one frame.
const FRAME_CEILING: u32 = 64;
/// Completed scans the window-system reporter makes before it stops naming the desk monitor.
const SCANS_BEFORE_UNPLUG: u32 = 2;
const SHARED_PANEL: &str = "BUILT-IN-PANEL-0001";
const STAGE_PROJECTOR: &str = "STAGE-PROJECTOR-0003";
const WINDOW_SYSTEM_REPORTER: &str = "window-system";

/// Reports the same authored whole set on every scan, standing in for a platform enumeration call.
///
/// A real reporter would ask the operating system here. Every scan returns the reporter's *whole*
/// current set, because an omitted key is how the kernel learns a device departed.
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
            DeviceScan::Complete(reported_keys.into_iter().map(present_display).collect())
        }))
    }
}

/// Endpoint driver that records what the kernel asked it to apply and reports the apply converged
/// on the first poll.
///
/// It touches no hardware. The point is to run one attempt the whole way from authorization to a
/// terminal outcome, so the example can show that the kernel dispatched the role's authored
/// placement and not some configuration of its own choosing. Readback stays unavailable, which is
/// what a driver whose platform exposes no query answers.
struct PanelDriver;

impl EndpointDriver for PanelDriver {
    type Configuration = PanelPlacement;

    fn capture(
        &mut self,
        _: &mut World,
        _: &DeviceEndpoint,
    ) -> CaptureOutcome<Self::Configuration> {
        CaptureOutcome::NotReadable
    }

    fn start_apply(
        &mut self,
        world: &mut World,
        _: &DeviceEndpoint,
        configuration: &Self::Configuration,
        _: AttemptId,
        _: ApplyPermit,
    ) {
        world.resource_mut::<AppliedPlacements>().0.push(format!(
            "left {} top {}",
            configuration.left_pixels, configuration.top_pixels
        ));
    }

    fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
        AttemptProgress::Finished(AttemptOutcome::Succeeded)
    }
}

/// Every placement `PanelDriver` was handed, in dispatch order.
///
/// The driver writes it through the `World` it is given rather than keeping it in the driver value,
/// because the driver itself lives inside the kernel's registry and the example never sees it again
/// after registration.
#[derive(Default, Resource)]
struct AppliedPlacements(Vec<String>);

/// One terminal attempt outcome an observer saw, in arrival order.
struct ObservedAttemptEnding {
    role:    RoleKey,
    attempt: AttemptId,
    outcome: AttemptOutcome,
}

/// Every attempt ending observed on a binding entity this run.
#[derive(Default, Resource)]
struct ObservedAttemptEndings(Vec<ObservedAttemptEnding>);

/// Record one attempt ending that reached a live role's binding entity.
fn observe_attempt_ending(
    attempt_finished: On<AttemptFinished>,
    mut observed_attempt_endings: ResMut<ObservedAttemptEndings>,
) {
    observed_attempt_endings.0.push(ObservedAttemptEnding {
        role:    attempt_finished.role.clone(),
        attempt: attempt_finished.attempt,
        outcome: attempt_finished.outcome.clone(),
    });
}

/// One kernel lifecycle event this run observed, in arrival order.
///
/// The kernel reports every state change as an event, so a consumer that wants to react to a
/// display arriving, a claim moving, or a role losing its device writes an observer instead of
/// polling a resource. This example collects them into one list so the run can print the whole
/// lifecycle in the order it happened.
struct ObservedLifecycleEvent {
    /// Which axis moved, such as `presence` or `role state`.
    axis:  &'static str,
    /// What the axis moved to, and which device or role it belongs to.
    moved: String,
}

/// Every lifecycle event observed this run.
#[derive(Default, Resource)]
struct ObservedLifecycle(Vec<ObservedLifecycleEvent>);

impl ObservedLifecycle {
    fn record(&mut self, axis: &'static str, moved: String) {
        self.0.push(ObservedLifecycleEvent { axis, moved });
    }

    /// Report whether any event on `axis` reached this run.
    fn saw(&self, axis: &str) -> bool {
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
    observed_lifecycle.record("arrival", moved);
}

fn observe_presence_changed(
    presence_changed: On<PresenceChanged>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!("{:?}", presence_changed.presence);
    observed_lifecycle.record("presence", moved);
}

fn observe_claim_changed(
    claim_changed: On<ClaimChanged>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!("{:?}", claim_changed.claim);
    observed_lifecycle.record("claim", moved);
}

fn observe_identity_changed(
    identity_changed: On<IdentityChanged>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!("{:?}", identity_changed.verdict);
    observed_lifecycle.record("identity", moved);
}

fn observe_device_departed(
    device_departed: On<DeviceDeparted>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!(
        "{} | {:?}",
        describe_key(&device_departed.key),
        device_departed.departure
    );
    observed_lifecycle.record("departure", moved);
}

fn observe_connection_changed(
    connection_changed: On<ConfiguredDeviceConnectionChanged>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!(
        "{} | {:?}",
        describe_key(&connection_changed.key),
        connection_changed.connection
    );
    observed_lifecycle.record("authored connection", moved);
}

fn observe_role_state_changed(
    role_state_changed: On<RoleStateChanged>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!(
        "role `{}` | {:?}",
        role_state_changed.role, role_state_changed.state
    );
    observed_lifecycle.record("role state", moved);
}

fn observe_role_awaiting(
    role_awaiting: On<RoleAwaiting>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!("role `{}` has no live device", role_awaiting.role);
    observed_lifecycle.record("role availability", moved);
}

fn observe_role_available(
    role_available: On<RoleAvailable>,
    mut observed_lifecycle: ResMut<ObservedLifecycle>,
) {
    let moved = format!("role `{}` resolved to a live device", role_available.role);
    observed_lifecycle.record("role availability", moved);
}

/// How the bounded wait for a successful apply on the panel role ended.
enum AttemptRun {
    /// An attempt succeeded, this many frames after the reporters completed.
    Succeeded { frames: u32 },
    /// The frame ceiling arrived before an attempt succeeded.
    CeilingReached,
}

/// Placement this example's role would ask its driver for, standing in for a window rectangle.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct PanelPlacement {
    left_pixels: i32,
    top_pixels:  i32,
}

/// One registered reporter and the name the report lines print beside its `ReporterId`.
struct NamedReporter {
    id:   ReporterId,
    name: &'static str,
}

/// One display this example reports, and how many reporters should end up contributing to it.
struct ReportedDisplay {
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
                "OK — three reconciled devices, the shared panel carries two contributing \
                 reporters and a live ResolvedToDevice link, no duplicate keys, the panel role's \
                 apply attempt succeeded and left the role Ready, and the withdrawn desk monitor \
                 departed and left its authored inventory entry reading Absent"
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

/// The three reported displays the run checks, with how many reporters each one is expected to
/// reach the reconciled set through.
fn reported_displays(
    shared_panel: DeviceKey,
    desk_monitor: DeviceKey,
    stage_projector: DeviceKey,
) -> Vec<ReportedDisplay> {
    vec![
        ReportedDisplay {
            key:                   shared_panel,
            label:                 "built-in panel (reported by both)",
            expected_contributors: 2,
        },
        ReportedDisplay {
            key:                   desk_monitor,
            label:                 "desk monitor (window-system only)",
            expected_contributors: 1,
        },
        ReportedDisplay {
            key:                   stage_projector,
            label:                 "stage projector (color-management only)",
            expected_contributors: 1,
        },
    ]
}

fn run() -> Result<SmokeCheck, Box<dyn Error>> {
    let shared_panel = reported_display_key(SHARED_PANEL)?;
    let desk_monitor = reported_display_key(DESK_MONITOR)?;
    let stage_projector = reported_display_key(STAGE_PROJECTOR)?;

    let mut app = App::new();
    // An unregistered scheme is rejected at the ingest boundary, so the identity space the reported
    // keys name is registered before any reporter can report one.
    app.add_plugins(MinimalPlugins)
        .add_plugins(RiggingPlugin)
        .register_device_scheme(SchemeName::new(DISPLAY_SCHEME)?)
        .init_resource::<AppliedPlacements>()
        .init_resource::<ObservedAttemptEndings>()
        .init_resource::<ObservedLifecycle>()
        .add_observer(observe_attempt_ending)
        .add_observer(observe_device_arrived)
        .add_observer(observe_presence_changed)
        .add_observer(observe_claim_changed)
        .add_observer(observe_identity_changed)
        .add_observer(observe_device_departed)
        .add_observer(observe_connection_changed)
        .add_observer(observe_role_state_changed)
        .add_observer(observe_role_awaiting)
        .add_observer(observe_role_available);

    // Binding registration is application work, not discovery work: this role is registered before
    // any reporter has run, and its binding entity is spawned by the first frame regardless.
    let panel_role = RoleKey::new(PANEL_ROLE)?;
    let panel_driver = app.add_endpoint_driver(PanelDriver);
    app.world_mut()
        .resource_mut::<Bindings>()
        .register(panel_binding(
            panel_role.clone(),
            shared_panel.clone(),
            panel_driver,
        ))?;

    // Two independent platform sources that both see the built-in panel and disagree about nothing
    // else: each one also enumerates a display the other never sees.
    // Authoring the desk monitor into inventory neither enables a reporter nor creates a device:
    // it is what gives the unit a connection conclusion to move when discovery stops naming it.
    app.world_mut()
        .resource_mut::<HardwareInventory>()
        .configure(ConfiguredDevice {
            key:  desk_monitor.clone(),
            mode: ConfiguredDeviceMode::Managed,
        });

    let reporters = vec![
        NamedReporter {
            name: WINDOW_SYSTEM_REPORTER,
            id:   app.add_device_reporter(
                FixedSetReporter {
                    reported_keys:   vec![shared_panel.clone(), desk_monitor.clone()],
                    withdrawal:      KeyWithdrawal::AfterScans {
                        after_scans: SCANS_BEFORE_UNPLUG,
                        key:         desk_monitor.clone(),
                    },
                    completed_scans: 0,
                },
                on_demand_registration()?,
            ),
        },
        NamedReporter {
            name: COLOR_MANAGEMENT_REPORTER,
            id:   app.add_device_reporter(
                FixedSetReporter {
                    reported_keys:   vec![shared_panel.clone(), stage_projector.clone()],
                    withdrawal:      KeyWithdrawal::Never,
                    completed_scans: 0,
                },
                on_demand_registration()?,
            ),
        },
    ];

    let displays = reported_displays(shared_panel, desk_monitor.clone(), stage_projector);

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
            let devices = app.world().resource::<Devices>();
            print_reconciled_devices(devices, &displays, &reporters);
            println!(
                "rigging revision: {}",
                app.world().resource::<RiggingRevision>().get()
            );
            let mut smoke_check = check_reconciled(devices, &displays);
            print_binding(app.world(), &panel_role, &mut smoke_check);
            print_connection(app.world(), &desk_monitor, "before the unplug");

            report_attempt(&mut app, &panel_role, &mut smoke_check);

            provoke_departure(
                &mut app,
                &reporters,
                &desk_monitor,
                &displays,
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
    displays: &[ReportedDisplay],
    smoke_check: &mut SmokeCheck,
) {
    for frame in 1..=FRAME_CEILING {
        request_one_scan(app, reporters, smoke_check);
        app.update();
        if app.world().resource::<Devices>().resolve(departing) == DeviceResolution::NotResolved {
            println!("frames until the withdrawn key left the reconciled set: {frame}");
            let remaining: Vec<&ReportedDisplay> = displays
                .iter()
                .filter(|display| &display.key != departing)
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

/// Drive frames until an apply on the panel role succeeds, then print what the attempts did.
///
/// Nothing in the example dispatches the apply: the kernel authorizes it once reconciliation
/// resolves the role's durable endpoint to a present device, and every terminal outcome arrives as
/// an event on the binding entity rather than as a return value. Earlier attempts can end
/// `Aborted` while the reported set is still settling, because an attempt authorized against one
/// rigging revision is abandoned rather than continued once a later scan advances it.
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

    println!("frames until an apply on the panel role succeeded: {frames}");
    println!(
        "placements dispatched to the driver: {}",
        app.world().resource::<AppliedPlacements>().0.join(", ")
    );
    for observed_attempt_ending in &app.world().resource::<ObservedAttemptEndings>().0 {
        println!(
            "  attempt ending | role `{}` | {:?} | {:?}",
            observed_attempt_ending.role,
            observed_attempt_ending.attempt,
            observed_attempt_ending.outcome
        );
    }

    let succeeded = app
        .world()
        .resource::<ObservedAttemptEndings>()
        .0
        .iter()
        .find(|observed_attempt_ending| {
            observed_attempt_ending.outcome == AttemptOutcome::Succeeded
        })
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
    if app.world().resource::<AppliedPlacements>().0.is_empty() {
        record_mismatch(
            smoke_check,
            format!("role `{role}` reported a successful apply the driver was never handed"),
        );
    }
    match app.world().resource::<Bindings>().binding(role) {
        Ok(binding) if binding.state == RoleState::Ready => {},
        Ok(binding) => record_mismatch(
            smoke_check,
            format!(
                "expected role `{role}` to be Ready after a successful apply, got {:?}",
                binding.state
            ),
        ),
        Err(error) => record_mismatch(
            smoke_check,
            format!("role `{role}` is no longer bound after its apply: {error}"),
        ),
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
            .any(|observed_attempt_ending| {
                observed_attempt_ending.outcome == AttemptOutcome::Succeeded
            })
        {
            return AttemptRun::Succeeded { frames: frame };
        }
        app.update();
    }

    AttemptRun::CeilingReached
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

/// Author one role that owns the whole built-in panel, with the default retention policy.
fn panel_binding(role: RoleKey, device: DeviceKey, driver: DriverId) -> Binding {
    Binding {
        role,
        endpoint: DeviceEndpoint {
            device,
            id: EndpointId::Whole,
        },
        driver,
        recovery: RecoveryPolicy::default(),
        retry: RetryOn::NewRevision,
        on_abort: OnAbort::default(),
        on_loss: OnSessionLoss::default(),
        state: RoleState::default(),
        requested: RequestedConfiguration::new(PanelPlacement {
            left_pixels: 0,
            top_pixels:  0,
        }),
        last_known_good: LastKnownGoodConfiguration::default(),
        apply_deadline: ApplyDeadline::ProcessDefault,
    }
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
                scheme: SchemeName::new(DISPLAY_SCHEME)?,
            },
        )),
    ))
}

fn reported_display_key(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
    Ok(DeviceKey {
        kind: DeviceKind::Display,
        id:   DeviceIdSource::Reported {
            scheme: SchemeName::new(DISPLAY_SCHEME)?,
            value:  ReportedId::new(value)?,
        },
    })
}

/// Build the record a reporter hands the kernel for one reachable display it can name durably.
fn present_display(device_key: DeviceKey) -> DeviceRecord {
    DeviceRecord {
        reported_as:            ReportedAs::Keyed(device_key),
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
    let discovery_status = world.resource::<DiscoveryStatus>();
    for named_reporter in reporters {
        match discovery_status.reporter_status(named_reporter.id) {
            Ok(reporter_discovery_status)
                if matches!(
                    reporter_discovery_status.last_outcome,
                    LastDiscoveryOutcome::Succeeded { .. }
                ) => {},
            Ok(_) => return ScanCoverage::Pending,
            Err(error) => {
                println!("reporter status unavailable: {error}");
                return ScanCoverage::Pending;
            },
        }
    }

    ScanCoverage::EveryReporterCompleted
}

fn print_reconciled_devices(
    devices: &Devices,
    displays: &[ReportedDisplay],
    reporters: &[NamedReporter],
) {
    println!("reconciled devices: {}", devices.count());
    for display in displays {
        let key = describe_key(&display.key);
        match devices.resolve(&display.key) {
            DeviceResolution::NotResolved => {
                println!("  {key} | {} | no device handle was issued", display.label);
            },
            DeviceResolution::Resolved(device_id) => match devices.state(device_id) {
                DeviceStateLookup::Retired => {
                    println!(
                        "  {key} | {} | DeviceId({}) | retired",
                        display.label,
                        device_id.get()
                    );
                },
                DeviceStateLookup::Retained(reconciled_device_state) => {
                    println!(
                        "  {key} | {} | DeviceId({}) | {:?} | {:?} | contributors: {}",
                        display.label,
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

fn check_reconciled(devices: &Devices, displays: &[ReportedDisplay]) -> SmokeCheck {
    let mut mismatches = Vec::new();

    if devices.count() != displays.len() {
        mismatches.push(format!(
            "expected {} reconciled devices, kernel retained {}",
            displays.len(),
            devices.count()
        ));
    }
    for display in displays {
        let contributors = match devices.resolve(&display.key) {
            DeviceResolution::NotResolved => {
                mismatches.push(format!("{} resolved to no device", display.label));
                continue;
            },
            DeviceResolution::Resolved(device_id) => match devices.state(device_id) {
                DeviceStateLookup::Retired => {
                    mismatches.push(format!("{} resolved to a retired handle", display.label));
                    continue;
                },
                DeviceStateLookup::Retained(reconciled_device_state) => {
                    reconciled_device_state.contributors.len()
                },
            },
        };
        if contributors != display.expected_contributors {
            mismatches.push(format!(
                "{} expected {} contributing reporters, got {contributors}",
                display.label, display.expected_contributors
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
    let binding_entities = world.resource::<BindingEntities>();
    println!("binding entities: {}", binding_entities.count());
    match binding_entities.entity(role) {
        BindingEntityLookup::Unregistered => {
            record_mismatch(
                smoke_check,
                format!("role `{role}` has no binding entity after registration"),
            );
        },
        BindingEntityLookup::Registered(entity) => {
            match (
                world.get::<RecoveryPolicy>(entity),
                world.get::<RoleState>(entity),
            ) {
                (Some(recovery_policy), Some(role_state)) => {
                    println!("  role `{role}` | {entity} | {recovery_policy:?} | {role_state:?}");
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
/// The four axes checked here are the ones this run definitely moves: a display arrives, its
/// reachability is established, the panel role reaches a live device and applies, and the desk
/// monitor is unplugged. An axis that stayed silent means a consumer watching only events would
/// have missed a change the resources went on to report, which is the defect the derived event list
/// exists to prevent.
fn report_lifecycle(world: &World, smoke_check: &mut SmokeCheck) {
    println!("kernel lifecycle events, in arrival order:");
    let observed_lifecycle = world.resource::<ObservedLifecycle>();
    for observed_lifecycle_event in &observed_lifecycle.0 {
        println!(
            "  {} | {}",
            observed_lifecycle_event.axis, observed_lifecycle_event.moved
        );
    }
    for axis in ["arrival", "presence", "role state", "departure"] {
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
