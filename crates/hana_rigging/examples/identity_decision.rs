//! Walkthrough of the identity-decision register: a saved unit leaves, a different unit takes its
//! place, and the operator decides whether the role should follow the new one.
//!
//! `N` advances the scripted reporter one whole-set scan. The first scan reports the saved panel,
//! the second reports a different panel at the same attachment, which is what makes the kernel ask.
//! `A`, `R`, and `D` answer the standing question — adopt the candidate, refuse it for good, or
//! leave it for later. The panel on the right shows the role's binding, the register, and the units
//! the kernel currently believes are present, so every answer is visible in all three at once.
//!
//! The chips are keyboard affordances rather than clickable buttons: `docs/fairy_dust/`
//! `canonical-example.md` records per-chip mouse activation as unimplemented, and a hand-rolled
//! click target would be example-local styling the guide asks examples not to invent.

use std::error::Error;

use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::prelude::error;
use bevy::prelude::info;
use bevy::prelude::warn;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DescriptionPanel;
use fairy_dust::Face;
use fairy_dust::StatsPanelRow;
use fairy_dust::StatsPanelSection;
use fairy_dust::TitleBar;
use fairy_dust::diegetic_stats_sections_panel_with_integral_advance;
use fairy_dust::diegetic_stats_sections_tree_with_integral_advance;
use fairy_dust::example_cube_on_ground;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::FontRegistry;
use hana_lagrange::OrbitCamPreset;
use hana_rigging::prelude::ApplyDeadline;
use hana_rigging::prelude::ApplyPermit;
use hana_rigging::prelude::AttachmentPath;
use hana_rigging::prelude::AttemptId;
use hana_rigging::prelude::AttemptOutcome;
use hana_rigging::prelude::AttemptProgress;
use hana_rigging::prelude::AuthoritativeReporterCoverage;
use hana_rigging::prelude::Binding;
use hana_rigging::prelude::Bindings;
use hana_rigging::prelude::CaptureOutcome;
use hana_rigging::prelude::ConfiguredDevice;
use hana_rigging::prelude::ConfiguredDeviceMode;
use hana_rigging::prelude::CoveredDeviceIdentitySpace;
use hana_rigging::prelude::DeviceEndpoint;
use hana_rigging::prelude::DeviceIdSource;
use hana_rigging::prelude::DeviceKey;
use hana_rigging::prelude::DeviceKind;
use hana_rigging::prelude::DeviceResolution;
use hana_rigging::prelude::Devices;
use hana_rigging::prelude::DiscoveryCadence;
use hana_rigging::prelude::DiscoveryControl;
use hana_rigging::prelude::EndpointDriver;
use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::HardwareInventory;
use hana_rigging::prelude::IdentityAnswer;
use hana_rigging::prelude::IdentityDecisions;
use hana_rigging::prelude::IdentityQuestionLookup;
use hana_rigging::prelude::LastKnownGoodConfiguration;
use hana_rigging::prelude::OnAbort;
use hana_rigging::prelude::OnSessionLoss;
use hana_rigging::prelude::RecoveryPolicy;
use hana_rigging::prelude::ReportedId;
use hana_rigging::prelude::ReporterActivation;
use hana_rigging::prelude::ReporterCoverage;
use hana_rigging::prelude::ReporterId;
use hana_rigging::prelude::ReporterRegistration;
use hana_rigging::prelude::RequestedConfiguration;
use hana_rigging::prelude::RetryOn;
use hana_rigging::prelude::RiggingAppExt;
use hana_rigging::prelude::RiggingPlugin;
use hana_rigging::prelude::RoleKey;
use hana_rigging::prelude::RoleState;
use hana_rigging::prelude::SchemeName;
use hana_rigging_scripted::ScriptedDevice;
use hana_rigging_scripted::ScriptedReporter;
use hana_rigging_scripted::ScriptedScan;
use hana_rigging_scripted::reported_key;

// camera and cube
const CUBE_CLEARANCE: f32 = 0.1;
const CUBE_FACE_LABEL: &str = "Role";
const HOME_MARGIN: f32 = 0.5;
const HOME_PITCH: f32 = 0.3;

// controls
const ADOPT_CONTROL: &str = "A Adopt";
const DEFER_CONTROL: &str = "D Defer";
const REJECT_CONTROL: &str = "R Reject";
const SCAN_CONTROL: &str = "N Next Scan";

// scripted hardware
const CANDIDATE_PANEL: &str = "PANEL-0002";
const PANEL_ROLE: &str = "primary-window";
const PANEL_SCHEME: &str = "example-usb-serial";
const SAVED_PANEL: &str = "PANEL-0001";
const SHARED_ATTACHMENT: &str = "usb-bus-1-port-4";
const THIRD_PANEL: &str = "PANEL-0003";

// panel copy
const BINDING_SECTION: &str = "Role binding";
const BOUND_ROW: &str = "Endpoint";
const CANDIDATE_ROW: &str = "Candidate";
const DESCRIPTION_TITLE: &str = "IdentityDecisions";
const DEVICES_SECTION: &str = "Present units";
const EXAMPLE_TITLE: &str = "Identity Decision";
const NONE_LABEL: &str = "none";
const PRESENT_ROW: &str = "Reported";
const QUESTION_SECTION: &str = "Register";
const RESOLUTION_ROW: &str = "Resolves";
const SAVED_ROW: &str = "Saved";
const STATE_ROW: &str = "State";

const DESCRIPTION_LINES: [&str; 6] = [
    "Scan 1 reports the saved panel and the role binds to it.",
    "Scan 2 reports a different panel at the same attachment.",
    "The kernel cannot tell a swap from a relabel, so it asks.",
    "A rebinds the role and the inventory entry to the new unit.",
    "R refuses that unit for good; the next one asks again.",
    "D leaves the question standing without re-prompting.",
];

/// What the example needs to reach the scripted reporter and the one role it drives.
#[derive(Resource)]
struct ScriptedRig {
    reporter: ReporterId,
    role:     RoleKey,
}

/// Marks the panel this example rewrites as the register changes.
#[derive(Component)]
struct RegisterStatusPanel;

/// Placement the scripted role asks its driver for, standing in for a window position.
#[derive(Component, Reflect)]
#[reflect(Component)]
struct PanelPlacement {
    slot: u32,
}

/// Driver that reports every apply converged, so nothing here turns on driver behaviour.
struct ConvergingDriver;

impl EndpointDriver for ConvergingDriver {
    type Configuration = PanelPlacement;

    fn capture(
        &mut self,
        _: &mut World,
        _: &DeviceEndpoint,
    ) -> CaptureOutcome<Self::Configuration> {
        CaptureOutcome::Read(PanelPlacement { slot: 1 })
    }

    fn start_apply(
        &mut self,
        _: &mut World,
        _: &DeviceEndpoint,
        _: &Self::Configuration,
        _: AttemptId,
        _: ApplyPermit,
    ) {
    }

    fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
        AttemptProgress::Finished(AttemptOutcome::Succeeded)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut example = fairy_dust::sprinkle_example().with_brp_extras();
    let rig = install_kernel(example.app_mut())?;
    example.app_mut().insert_resource(rig);

    example
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .transform(Transform::from_translation(example_cube_on_ground(
            CUBE_CLEARANCE,
        )))
        .face_label(Face::Front, CUBE_FACE_LABEL)
        .insert(CameraHomeTarget)
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_stable_transparency()
        .with_camera_home()
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(SCAN_CONTROL)
                .control(ADOPT_CONTROL)
                .control(REJECT_CONTROL)
                .control(DEFER_CONTROL),
        )
        .with_description_panel(
            DescriptionPanel::new(DESCRIPTION_TITLE)
                .with_fit_width()
                .lines(DESCRIPTION_LINES),
        )
        .with_camera_control_panel()
        .add_systems(Startup, spawn_status_panel)
        .add_systems(Update, refresh_status_panel)
        .with_shortcut(KeyCode::KeyN, request_scan)
        .with_shortcut(KeyCode::KeyA, adopt_candidate)
        .with_shortcut(KeyCode::KeyR, reject_candidate)
        .with_shortcut(KeyCode::KeyD, defer_question)
        .run();

    Ok(())
}

/// Install the kernel, the scripted reporter, and the one role the walkthrough drives.
///
/// Registration happens against `&mut App` rather than through a Fairy Dust helper so the calls
/// read the same way in an application that never depended on Fairy Dust.
fn install_kernel(app: &mut App) -> Result<ScriptedRig, Box<dyn Error>> {
    let saved = reported_key(DeviceKind::HidPanel, PANEL_SCHEME, SAVED_PANEL)?;
    let candidate = reported_key(DeviceKind::HidPanel, PANEL_SCHEME, CANDIDATE_PANEL)?;
    let third = reported_key(DeviceKind::HidPanel, PANEL_SCHEME, THIRD_PANEL)?;

    app.add_plugins(RiggingPlugin)
        .register_device_scheme(SchemeName::new(PANEL_SCHEME)?);

    let reporter = app.add_device_reporter(
        ScriptedReporter::new(vec![
            ScriptedScan::Complete(vec![at_shared_attachment(saved.clone())?]),
            ScriptedScan::Complete(vec![at_shared_attachment(candidate)?]),
            ScriptedScan::Complete(vec![at_shared_attachment(third)?]),
        ]),
        ReporterRegistration::optional(
            DiscoveryCadence::OnDemand,
            ReporterActivation::Enabled,
            ReporterCoverage::EstablishesAbsence(AuthoritativeReporterCoverage::one(
                CoveredDeviceIdentitySpace::ReportedScheme {
                    kind:   DeviceKind::HidPanel,
                    scheme: SchemeName::new(PANEL_SCHEME)?,
                },
            )),
        ),
    );

    let driver = app.add_endpoint_driver(ConvergingDriver);
    let role = RoleKey::new(PANEL_ROLE)?;
    app.world_mut()
        .resource_mut::<Bindings>()
        .register(Binding {
            role: role.clone(),
            endpoint: DeviceEndpoint {
                device: saved.clone(),
                id:     EndpointId::Whole,
            },
            driver,
            recovery: RecoveryPolicy::ReapplyOnReturn,
            retry: RetryOn::NewRevision,
            on_abort: OnAbort::default(),
            on_loss: OnSessionLoss::default(),
            state: RoleState::default(),
            requested: RequestedConfiguration::new(PanelPlacement { slot: 1 }),
            last_known_good: LastKnownGoodConfiguration::default(),
            apply_deadline: ApplyDeadline::ProcessDefault,
        })?;
    app.world_mut()
        .resource_mut::<HardwareInventory>()
        .configure(ConfiguredDevice {
            key:  saved,
            mode: ConfiguredDeviceMode::Managed,
        });

    Ok(ScriptedRig { reporter, role })
}

/// One scripted unit reported at the attachment the whole walkthrough turns on.
fn at_shared_attachment(device_key: DeviceKey) -> Result<ScriptedDevice, Box<dyn Error>> {
    Ok(
        ScriptedDevice::present(device_key).with_attachment(AttachmentPath::Reported(
            ReportedId::new(SHARED_ATTACHMENT)?,
        )),
    )
}

fn request_scan(mut discovery_control: ResMut<DiscoveryControl>, rig: Res<ScriptedRig>) {
    if let Err(error) = discovery_control.request(rig.reporter) {
        warn!("the scripted reporter refused a scan: {error}");
    }
}

fn adopt_candidate(identity_decisions: ResMut<IdentityDecisions>, rig: Res<ScriptedRig>) {
    answer_standing_question(identity_decisions, &rig.role, IdentityAnswer::Adopt);
}

fn reject_candidate(identity_decisions: ResMut<IdentityDecisions>, rig: Res<ScriptedRig>) {
    answer_standing_question(identity_decisions, &rig.role, IdentityAnswer::Reject);
}

fn defer_question(mut identity_decisions: ResMut<IdentityDecisions>, rig: Res<ScriptedRig>) {
    let Some(candidate) = standing_candidate(&identity_decisions, &rig.role) else {
        return;
    };
    identity_decisions.defer(&rig.role, &candidate);
}

/// Answer whatever the register is currently asking about this role.
///
/// The candidate is read out first because a question names a role *and* a unit: answering by role
/// alone would settle whichever unit happened to be asked about last.
fn answer_standing_question(
    mut identity_decisions: ResMut<IdentityDecisions>,
    role: &RoleKey,
    identity_answer: IdentityAnswer,
) {
    let Some(candidate) = standing_candidate(&identity_decisions, role) else {
        return;
    };
    let outcome = identity_decisions.answer(role, &candidate, identity_answer);
    info!(
        "answered {identity_answer:?} for {}: {outcome:?}",
        describe_key(&candidate)
    );
}

fn standing_candidate(identity_decisions: &IdentityDecisions, role: &RoleKey) -> Option<DeviceKey> {
    match identity_decisions.question(role) {
        IdentityQuestionLookup::Pending(question) => Some(question.candidate.clone()),
        IdentityQuestionLookup::NoQuestion => None,
    }
}

fn spawn_status_panel(
    mut commands: Commands,
    fonts: Res<FontRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    match diegetic_stats_sections_panel_with_integral_advance(
        &[StatsPanelSection::new(
            QUESTION_SECTION,
            [StatsPanelRow::new(STATE_ROW, NONE_LABEL)],
        )],
        &fonts,
        &mut materials,
    ) {
        Ok(panel) => {
            commands.spawn((RegisterStatusPanel, panel, Transform::default()));
        },
        Err(error) => {
            error!("identity_decision: failed to build the status panel: {error}");
        },
    }
}

/// Rewrite the status panel whenever the three things it reports stop agreeing with the display.
fn refresh_status_panel(
    bindings: Res<Bindings>,
    devices: Res<Devices>,
    identity_decisions: Res<IdentityDecisions>,
    rig: Res<ScriptedRig>,
    panels: Query<Entity, With<RegisterStatusPanel>>,
    mut displayed: Local<Option<Vec<String>>>,
    mut commands: Commands,
    fonts: Res<FontRegistry>,
) {
    let rows = status_rows(&bindings, &devices, &identity_decisions, &rig.role);
    if displayed.as_ref() == Some(&rows) {
        return;
    }
    for panel in &panels {
        if let Err(error) = commands.set_tree(
            panel,
            diegetic_stats_sections_tree_with_integral_advance(&status_sections(&rows), &fonts),
        ) {
            warn!("failed to replace the identity-decision status panel: {error}");
        }
    }
    *displayed = Some(rows);
}

/// The five values the panel reports, in the order the sections read them.
fn status_rows(
    bindings: &Bindings,
    devices: &Devices,
    identity_decisions: &IdentityDecisions,
    role: &RoleKey,
) -> Vec<String> {
    let (bound, resolution) = bindings.binding(role).map_or_else(
        |_| (NONE_LABEL.to_owned(), NONE_LABEL.to_owned()),
        |binding| {
            let resolution = match devices.resolve(&binding.endpoint.device) {
                DeviceResolution::Resolved(_) => "yes".to_owned(),
                DeviceResolution::NotResolved => "no".to_owned(),
            };

            (describe_key(&binding.endpoint.device), resolution)
        },
    );
    let (saved, candidate, state) = match identity_decisions.question(role) {
        IdentityQuestionLookup::Pending(question) => (
            describe_key(&question.saved),
            describe_key(&question.candidate),
            format!("{:?}", question.state),
        ),
        IdentityQuestionLookup::NoQuestion => (
            NONE_LABEL.to_owned(),
            NONE_LABEL.to_owned(),
            NONE_LABEL.to_owned(),
        ),
    };
    let present = devices
        .states()
        .map(|reconciled_device_state| describe_key(&reconciled_device_state.key))
        .collect::<Vec<_>>()
        .join(", ");

    vec![
        bound,
        resolution,
        saved,
        candidate,
        state,
        if present.is_empty() {
            NONE_LABEL.to_owned()
        } else {
            present
        },
    ]
}

fn status_sections(rows: &[String]) -> Vec<StatsPanelSection> {
    let mut values = rows.iter().map(String::as_str);
    let mut next = || values.next().unwrap_or(NONE_LABEL);

    vec![
        StatsPanelSection::new(
            BINDING_SECTION,
            [
                StatsPanelRow::new(BOUND_ROW, next()),
                StatsPanelRow::new(RESOLUTION_ROW, next()),
            ],
        ),
        StatsPanelSection::new(
            QUESTION_SECTION,
            [
                StatsPanelRow::new(SAVED_ROW, next()),
                StatsPanelRow::new(CANDIDATE_ROW, next()),
                StatsPanelRow::new(STATE_ROW, next()),
            ],
        ),
        StatsPanelSection::new(DEVICES_SECTION, [StatsPanelRow::new(PRESENT_ROW, next())]),
    ]
}

fn describe_key(device_key: &DeviceKey) -> String {
    match &device_key.id {
        DeviceIdSource::Reported { value, .. } => value.as_str().to_owned(),
        DeviceIdSource::Synthesized { digest } => format!("synthesized:{digest:?}"),
        DeviceIdSource::Authored { value } => format!("authored:{}", value.as_str()),
    }
}
