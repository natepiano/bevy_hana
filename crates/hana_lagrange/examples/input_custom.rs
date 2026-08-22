//! Builds an app-owned `OrbitCamBindings` set that mixes mouse, smooth scroll,
//! wheel, and pinch inputs, then attaches it through
//! `OrbitCamInputMode::Bindings(...)`. Press `T` to toggle
//! `CameraInputDisabled` and see how an app can disable camera controls at
//! runtime without rebuilding the binding set.
//!
//! Controls:
//!   Orbit — middle mouse drag, smooth scroll
//!   Pan   — right mouse drag, Shift + smooth scroll
//!   Zoom  — wheel, Ctrl + smooth scroll, pinch
//!   H     — return to the camera home pose
//!   T     — toggle camera input

use std::f32::consts::TAU;

use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::PostStartup;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::prelude::warn;
use bevy_enhanced_input::prelude::ModKeys;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::CubeFacePanelContent;
use fairy_dust::CubeFacePanelStyle;
use fairy_dust::DescriptionPanel;
use fairy_dust::Face;
use fairy_dust::FairyDustOrbitCam;
use fairy_dust::HoldState;
use fairy_dust::LABEL_SIZE;
use fairy_dust::ReleaseHold;
use fairy_dust::TitleBar;
use fairy_dust::TitleChipActivation;
use fairy_dust::apply_example_orbit_cam_limits;
use fairy_dust::cube_face_panel;
use fairy_dust::cube_face_panel_tree;
use fairy_dust::cube_face_transform;
use hana_diegetic::DiegeticPanelCommands;
use hana_lagrange::BindingsError;
use hana_lagrange::CameraInputDisabled;
use hana_lagrange::CameraInputRoutingConfig;
use hana_lagrange::InteractionSources;
use hana_lagrange::NoPositionFallback;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamBindings;
use hana_lagrange::OrbitCamControlSummary;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamInteractionKind;
use hana_lagrange::OrbitCamInteractionStarted;
use hana_lagrange::OrbitCamInteractionState;
use hana_lagrange::OrbitCamMouseDrag;
use hana_lagrange::OrbitCamMouseWheelZoom;
use hana_lagrange::OrbitCamPinchZoom;
use hana_lagrange::OrbitCamTrackpadScroll;
use hana_lagrange::ZoomDirection;
use hana_lagrange::ZoomInversion;
use hana_lagrange::describe_orbit_cam_controls;

const EXAMPLE_TITLE: &str = "Custom Bindings";

fn main() {
    let Ok(bindings) = custom_bindings() else {
        error!("custom camera bindings failed to validate");
        return;
    };

    fairy_dust::sprinkle_example()
        .insert_resource(
            CameraInputRoutingConfig::cursor_hit_test()
                .with_no_position_fallback(NoPositionFallback::OnlyEligibleCamera),
        )
        .insert_resource(CustomBindings(bindings))
        .init_resource::<InputDisabledState>()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(Transform::from_translation(CUBE_TRANSLATION))
        .insert((CameraHomeTarget, CustomInputCube))
        .with_camera_home()
        .yaw(CAMERA_YAW)
        .pitch(CAMERA_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(INPUT_DISABLED_CONTROL),
        )
        .wire_chip_to_activation::<InputDisabledState>(INPUT_DISABLED_CONTROL)
        .with_cube_spin::<CustomInputCube>()
        .with_description_panel(description_panel())
        .with_camera_control_panel()
        .add_systems(Startup, spawn_camera)
        .add_systems(PostStartup, spawn_face_labels)
        .insert_resource(FaceLabelHold::default())
        .add_systems(Update, (toggle_camera_controls, update_face_labels))
        .add_observer(capture_zoom_started)
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// CUSTOM BINDINGS — OrbitCamBindings::builder() + CameraInputDisabled toggling.
//
// How it works:
//   1. `custom_bindings` builds one validated `OrbitCamBindings` value with mouse-drag,
//      smooth-scroll, wheel, and pinch entries.
//   2. `spawn_camera` clones that binding set into `OrbitCamInputMode::Bindings(...)` on the
//      `OrbitCam`, while also showing camera-level sensitivity, limit, and upside-down settings.
//   3. `toggle_camera_controls` adds or removes `CameraInputDisabled`; Lagrange keeps the binding
//      set installed, but ignores camera input, including H home, while the component is present.
// ═════════════════════════════════════════════════════════════════════════════

const CAMERA_FOCUS: Vec3 = CUBE_TRANSLATION;
const CAMERA_ORBIT_SENSITIVITY: f32 = 1.5;
const CAMERA_PAN_SENSITIVITY: f32 = 0.5;
const CAMERA_PITCH: f32 = TAU / 8.0;
const CAMERA_RADIUS: f32 = 5.0;
const CAMERA_YAW: f32 = TAU / 8.0;
const CAMERA_ZOOM_SENSITIVITY: f32 = 0.5;
const CUSTOM_WHEEL_INPUT_GAIN: f32 = 0.75;
const HOME_MARGIN: f32 = 0.5;
const INPUT_DISABLED_CONTROL: &str = "T Disabled";

#[derive(Resource)]
struct CustomBindings(OrbitCamBindings);

#[derive(Resource, Default)]
struct InputDisabledState {
    disabled: bool,
}

impl TitleChipActivation for InputDisabledState {
    fn activation(&self) -> ControlActivation { activation_for(self.disabled) }
}

#[derive(Component)]
struct CustomCamera;

fn spawn_camera(mut commands: Commands, bindings: Res<CustomBindings>) {
    let mut camera = OrbitCam::from_pose(CAMERA_FOCUS, (CAMERA_YAW, CAMERA_PITCH), CAMERA_RADIUS);
    camera.orbit.set_sensitivity(CAMERA_ORBIT_SENSITIVITY);
    camera.pan.set_sensitivity(CAMERA_PAN_SENSITIVITY);
    camera.zoom.set_sensitivity(CAMERA_ZOOM_SENSITIVITY);
    apply_example_orbit_cam_limits(&mut camera);
    commands.spawn((
        camera,
        OrbitCamInputMode::Bindings(bindings.0.clone()),
        CustomCamera,
        FairyDustOrbitCam,
    ));
}

fn toggle_camera_controls(
    key_input: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut disabled_state: ResMut<InputDisabledState>,
    cameras: Query<(Entity, Option<&CameraInputDisabled>), With<CustomCamera>>,
) {
    if !key_input.just_pressed(KeyCode::KeyT) {
        return;
    }

    for (camera, disabled) in &cameras {
        if disabled.is_some() {
            commands.entity(camera).remove::<CameraInputDisabled>();
            disabled_state.disabled = false;
        } else {
            commands.entity(camera).insert(CameraInputDisabled);
            disabled_state.disabled = true;
        }
    }
}

fn custom_bindings() -> Result<OrbitCamBindings, BindingsError> {
    OrbitCamBindings::builder()
        .orbit(OrbitCamMouseDrag::new(MouseButton::Middle))
        .orbit(OrbitCamTrackpadScroll::default())
        .pan(OrbitCamMouseDrag::new(MouseButton::Right))
        .pan(OrbitCamTrackpadScroll::default().with_mod_keys(ModKeys::SHIFT))
        .zoom(OrbitCamMouseWheelZoom.with_input_gain(CUSTOM_WHEEL_INPUT_GAIN))
        .zoom(OrbitCamTrackpadScroll::default().with_mod_keys(ModKeys::CONTROL))
        .zoom(OrbitCamPinchZoom)
        .home(KeyCode::KeyH)
        .zoom_inversion(ZoomInversion::Inverted)
        .build()
}

const fn activation_for(active: bool) -> ControlActivation {
    if active {
        ControlActivation::Active
    } else {
        ControlActivation::Inactive
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// CUBE FACE PANELS — live face panels showing active custom input sources.
// ═════════════════════════════════════════════════════════════════════════════

const DISABLED_FACE_STATUS: &str = "Disabled";
const FACE_PANEL_NAME: &str = "Custom input face panel";
const FACE_PANEL_STYLE: CubeFacePanelStyle = CubeFacePanelStyle::for_cube(CUBE_SIZE);

#[derive(Component)]
struct CustomInputCube;

#[derive(Component, Clone, Copy)]
enum CustomFaceLabel {
    Orbit,
    Pan,
    Zoom,
}

impl CustomFaceLabel {
    const fn kind(self) -> OrbitCamInteractionKind {
        match self {
            Self::Orbit => OrbitCamInteractionKind::Orbit,
            Self::Pan => OrbitCamInteractionKind::Pan,
            Self::Zoom => OrbitCamInteractionKind::Zoom,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Orbit => "Orbit",
            Self::Pan => "Pan",
            Self::Zoom => "Zoom",
        }
    }
}

/// Holds the bindings' described controls so idle/active face labels share the
/// camera control panel's vocabulary.
#[derive(Resource)]
struct FaceGuidance(OrbitCamControlSummary);

#[derive(Resource, Default)]
struct FaceLabelHold {
    orbit: ReleaseHold<Vec<String>>,
    pan:   ReleaseHold<Vec<String>>,
    zoom:  ReleaseHold<Vec<String>>,
}

impl FaceLabelHold {
    fn clear(&mut self) {
        self.orbit.clear();
        self.pan.clear();
        self.zoom.clear();
    }
}

fn spawn_face_labels(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    bindings: Res<CustomBindings>,
    cubes: Query<Entity, With<CustomInputCube>>,
) {
    let Ok(cube) = cubes.single() else {
        return;
    };

    let summary = describe_orbit_cam_controls(&OrbitCamInputMode::Bindings(bindings.0.clone()));
    commands.entity(cube).with_children(|parent| {
        for face in [Face::Front, Face::Back] {
            spawn_face_panel(
                parent,
                face,
                CustomFaceLabel::Orbit,
                &summary,
                &mut materials,
            );
        }
        for face in [Face::Left, Face::Right] {
            spawn_face_panel(parent, face, CustomFaceLabel::Pan, &summary, &mut materials);
        }
        for face in [Face::Top, Face::Bottom] {
            spawn_face_panel(
                parent,
                face,
                CustomFaceLabel::Zoom,
                &summary,
                &mut materials,
            );
        }
    });
    commands.insert_resource(FaceGuidance(summary));
}

fn update_face_labels(
    mut commands: Commands,
    time: Res<Time>,
    mut hold: ResMut<FaceLabelHold>,
    guidance: Res<FaceGuidance>,
    cameras: Query<(&OrbitCamInteractionState, Option<&CameraInputDisabled>), With<CustomCamera>>,
    labels: Query<(Entity, &CustomFaceLabel)>,
) {
    let Ok((interaction_state, disabled)) = cameras.single() else {
        return;
    };

    let (orbit, pan, zoom) = if disabled.is_some() {
        hold.clear();
        (
            CubeFacePanelContent::active(CustomFaceLabel::Orbit.title(), [DISABLED_FACE_STATUS]),
            CubeFacePanelContent::active(CustomFaceLabel::Pan.title(), [DISABLED_FACE_STATUS]),
            CubeFacePanelContent::active(CustomFaceLabel::Zoom.title(), [DISABLED_FACE_STATUS]),
        )
    } else {
        (
            held_content(
                &mut hold.orbit,
                time.delta(),
                active_labels(
                    &guidance.0,
                    OrbitCamInteractionKind::Orbit,
                    interaction_state.orbit_sources(),
                    None,
                ),
                &guidance.0,
                CustomFaceLabel::Orbit,
            ),
            held_content(
                &mut hold.pan,
                time.delta(),
                active_labels(
                    &guidance.0,
                    OrbitCamInteractionKind::Pan,
                    interaction_state.pan_sources(),
                    None,
                ),
                &guidance.0,
                CustomFaceLabel::Pan,
            ),
            held_content(
                &mut hold.zoom,
                time.delta(),
                active_labels(
                    &guidance.0,
                    OrbitCamInteractionKind::Zoom,
                    interaction_state.zoom_sources(),
                    interaction_state.zoom_direction(),
                ),
                &guidance.0,
                CustomFaceLabel::Zoom,
            ),
        )
    };

    for (entity, kind) in &labels {
        let next = match kind {
            CustomFaceLabel::Orbit => orbit.clone(),
            CustomFaceLabel::Pan => pan.clone(),
            CustomFaceLabel::Zoom => zoom.clone(),
        };
        if let Err(error) = commands.set_tree(entity, cube_face_panel_tree(FACE_PANEL_STYLE, next))
        {
            warn!("failed to replace input panel {entity:?} tree: {error}");
        }
    }
}

fn capture_zoom_started(
    event: On<OrbitCamInteractionStarted>,
    cameras: Query<&OrbitCamInteractionState, With<CustomCamera>>,
    guidance: Res<FaceGuidance>,
    mut hold: ResMut<FaceLabelHold>,
) {
    let Ok(interaction_state) = cameras.get(event.camera) else {
        return;
    };
    if event.kind != OrbitCamInteractionKind::Zoom {
        return;
    }
    let Some(lines) = active_labels(
        &guidance.0,
        OrbitCamInteractionKind::Zoom,
        event.sources,
        interaction_state.zoom_direction(),
    ) else {
        return;
    };
    hold.zoom.update(std::time::Duration::ZERO, Some(lines));
}

fn spawn_face_panel(
    parent: &mut ChildSpawnerCommands,
    face: Face,
    kind: CustomFaceLabel,
    summary: &OrbitCamControlSummary,
    materials: &mut Assets<StandardMaterial>,
) {
    let content = CubeFacePanelContent::idle(kind.title(), idle_labels(summary, kind.kind()));
    match cube_face_panel(FACE_PANEL_STYLE, content, materials) {
        Ok(panel) => {
            parent.spawn((
                Name::new(FACE_PANEL_NAME),
                kind,
                panel,
                cube_face_transform(face, CUBE_SIZE),
            ));
        },
        Err(error) => {
            error!("input_custom: failed to build cube face panel: {error}");
        },
    }
}

/// All control labels configured for `kind`, shown while idle.
fn idle_labels(summary: &OrbitCamControlSummary, kind: OrbitCamInteractionKind) -> Vec<String> {
    summary
        .rows
        .iter()
        .filter(|row| row.kind == kind)
        .map(|row| row.label.clone())
        .collect()
}

/// The control labels for `kind` whose sources are live, or `None` when idle.
/// For zoom, `zoom_direction` keeps only the engaged direction's rows so a
/// single wheel/scroll/pinch source does not light both its `↑` and `↓` rows at
/// once. Pass `None` for orbit and pan — their rows are not direction-specific.
fn active_labels(
    summary: &OrbitCamControlSummary,
    kind: OrbitCamInteractionKind,
    sources: InteractionSources,
    zoom_direction: Option<ZoomDirection>,
) -> Option<Vec<String>> {
    let labels = summary
        .rows
        .iter()
        .filter(|row| row.kind == kind && row.camera_interaction_sources.intersects(sources))
        .filter(|row| row_matches_zoom_direction(row.zoom_direction, zoom_direction))
        .map(|row| row.label.clone())
        .collect::<Vec<_>>();
    (!labels.is_empty()).then_some(labels)
}

/// A row shows when it is non-directional (orbit, pan) or when its zoom
/// direction matches the live one. While the live direction is unknown — a
/// zero-delta frame before the sign resolves — directional rows are kept so the
/// zoom face never blanks mid-gesture.
fn row_matches_zoom_direction(row: Option<ZoomDirection>, live: Option<ZoomDirection>) -> bool {
    match (row, live) {
        (None, _) | (Some(_), None) => true,
        (Some(row), Some(live)) => row == live,
    }
}

fn held_content(
    hold: &mut ReleaseHold<Vec<String>>,
    delta: std::time::Duration,
    active_lines: Option<Vec<String>>,
    summary: &OrbitCamControlSummary,
    label: CustomFaceLabel,
) -> CubeFacePanelContent {
    match hold.update(delta, active_lines) {
        HoldState::Active(lines) | HoldState::Held(lines) => {
            CubeFacePanelContent::active(label.title(), lines.clone())
        },
        HoldState::Idle => {
            CubeFacePanelContent::idle(label.title(), idle_labels(summary, label.kind()))
        },
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// DESCRIPTION PANEL — on-screen explainer for custom bindings.
// ═════════════════════════════════════════════════════════════════════════════

const DESCRIPTION_TITLE: &str = "Custom Bindings";
const DESCRIPTION_LINES: [&str; 5] = [
    "This example uses OrbitCamInputMode::Bindings with a multi-device OrbitCamBindings set.",
    "Bindings can combine mouse drag, smooth scroll, wheel, and pinch input.",
    "Unlike keyboard-only bindings, one camera action can accept several source types.",
    "Unlike Manual mode, Lagrange still reads and routes the input for you.",
    "Press T to toggle CameraInputDisabled on the camera.",
];

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_TITLE)
        .with_fit_width()
        .with_body_size(LABEL_SIZE.0)
        .lines(DESCRIPTION_LINES)
}

// ═════════════════════════════════════════════════════════════════════════════
// CUBE SPIN — decorative idle spin toggled by Fairy Dust's `P Pause` helper.
// ═════════════════════════════════════════════════════════════════════════════

// ═════════════════════════════════════════════════════════════════════════════
// SCENE SCAFFOLDING — cube body and ground sized to match.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_GROUND_CLEARANCE: f32 = 0.1;
const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = fairy_dust::example_cube_on_ground(CUBE_GROUND_CLEARANCE);
