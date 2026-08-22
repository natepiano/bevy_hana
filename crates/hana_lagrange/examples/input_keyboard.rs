//! Demonstrates keyboard user input through `OrbitCamPreset::keyboard()`.
//!
//! Controls:
//!   Arrows — orbit
//!   WASD   — pan
//!   +/-    — zoom
//!   H      — return to the camera home pose

use bevy::prelude::ButtonInput;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::PostStartup;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DescriptionPanel;
use fairy_dust::Face;
use fairy_dust::FairyDustOrbitCam;
use fairy_dust::HoldState;
use fairy_dust::LABEL_SIZE;
use fairy_dust::ReleaseHold;
use fairy_dust::TitleBar;
use fairy_dust::apply_example_orbit_cam_limits;
use fairy_dust::cube_face_label;
use hana_diegetic::DiegeticTextMut;
use hana_lagrange::CameraInputRoutingConfig;
use hana_lagrange::NoPositionFallback;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamControlSummary;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamInteractionKind;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::describe_orbit_cam_controls;

const EXAMPLE_TITLE: &str = "Keyboard Bindings";

fn main() {
    fairy_dust::sprinkle_example()
        .insert_resource(
            CameraInputRoutingConfig::cursor_hit_test()
                .with_no_position_fallback(NoPositionFallback::OnlyEligibleCamera),
        )
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(Transform::from_translation(CUBE_TRANSLATION))
        .insert((CameraHomeTarget, KeyboardInputCube))
        .with_camera_home()
        .yaw(CAMERA_YAW)
        .pitch(CAMERA_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft),
        )
        .with_cube_spin::<KeyboardInputCube>()
        .with_description_panel(description_panel())
        .with_camera_control_panel()
        .add_systems(Startup, spawn_camera)
        .add_systems(PostStartup, spawn_face_labels)
        .insert_resource(FaceLabelHold::default())
        .add_systems(Update, update_face_labels)
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// KEYBOARD PRESET — OrbitCamInputMode::with_preset(OrbitCamPreset::keyboard()).
// ═════════════════════════════════════════════════════════════════════════════

const CAMERA_FOCUS: Vec3 = CUBE_TRANSLATION;
const CAMERA_ORBIT_SENSITIVITY: f32 = 4.0;
const CAMERA_PAN_SENSITIVITY: f32 = 6.0;
const CAMERA_PITCH: f32 = 0.45;
const CAMERA_RADIUS: f32 = 6.0;
const CAMERA_YAW: f32 = 0.55;
const CAMERA_ZOOM_SENSITIVITY: f32 = 0.08;
const HOME_MARGIN: f32 = 0.5;

fn spawn_camera(mut commands: Commands) {
    let mut camera = OrbitCam::from_pose(CAMERA_FOCUS, (CAMERA_YAW, CAMERA_PITCH), CAMERA_RADIUS);
    camera.orbit.set_sensitivity(CAMERA_ORBIT_SENSITIVITY);
    camera.pan.set_sensitivity(CAMERA_PAN_SENSITIVITY);
    camera.zoom.set_sensitivity(CAMERA_ZOOM_SENSITIVITY);
    apply_example_orbit_cam_limits(&mut camera);
    commands.spawn((
        camera,
        OrbitCamInputMode::with_preset(OrbitCamPreset::keyboard()),
        FairyDustOrbitCam,
    ));
}

// ═════════════════════════════════════════════════════════════════════════════
// CUBE FACE LABELS — live world-space DiegeticText labels showing which bound keys are down.
// ═════════════════════════════════════════════════════════════════════════════

const ORBIT_FACE_KEYS: [(KeyCode, &str); 4] = [
    (KeyCode::ArrowUp, "Up"),
    (KeyCode::ArrowDown, "Down"),
    (KeyCode::ArrowLeft, "Left"),
    (KeyCode::ArrowRight, "Right"),
];
const PAN_FACE_KEYS: [(KeyCode, &str); 4] = [
    (KeyCode::KeyW, "W"),
    (KeyCode::KeyA, "A"),
    (KeyCode::KeyS, "S"),
    (KeyCode::KeyD, "D"),
];
const ZOOM_FACE_KEYS: [(KeyCode, &str); 2] = [(KeyCode::Equal, "+"), (KeyCode::Minus, "-")];

#[derive(Resource, Default)]
struct FaceLabelHold {
    orbit: ReleaseHold<String>,
    pan:   ReleaseHold<String>,
    zoom:  ReleaseHold<String>,
}

#[derive(Component, Clone, Copy)]
enum KeyboardFaceLabel {
    Orbit,
    Pan,
    Zoom,
}

impl KeyboardFaceLabel {
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

/// Holds the preset's described controls so idle face labels share the camera
/// control panel's vocabulary; live labels still show the pressed keys.
#[derive(Resource)]
struct FaceGuidance(OrbitCamControlSummary);

#[derive(Component)]
struct KeyboardInputCube;

fn spawn_face_labels(mut commands: Commands, cubes: Query<Entity, With<KeyboardInputCube>>) {
    let Ok(cube) = cubes.single() else {
        return;
    };

    let summary =
        describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(OrbitCamPreset::keyboard()));
    let idle = |face: KeyboardFaceLabel| {
        action_face_label(face.title(), &idle_label(&summary, face.kind()), None)
    };
    commands.entity(cube).with_children(|parent| {
        for face in [Face::Front, Face::Back] {
            parent.spawn((
                cube_face_label(face, idle(KeyboardFaceLabel::Orbit), CUBE_SIZE),
                KeyboardFaceLabel::Orbit,
            ));
        }
        for face in [Face::Left, Face::Right] {
            parent.spawn((
                cube_face_label(face, idle(KeyboardFaceLabel::Pan), CUBE_SIZE),
                KeyboardFaceLabel::Pan,
            ));
        }
        for face in [Face::Top, Face::Bottom] {
            parent.spawn((
                cube_face_label(face, idle(KeyboardFaceLabel::Zoom), CUBE_SIZE),
                KeyboardFaceLabel::Zoom,
            ));
        }
    });
    commands.insert_resource(FaceGuidance(summary));
}

fn update_face_labels(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut hold: ResMut<FaceLabelHold>,
    guidance: Res<FaceGuidance>,
    mut labels: DiegeticTextMut<KeyboardFaceLabel>,
) {
    let orbit = held_label(
        &mut hold.orbit,
        time.delta(),
        pressed_key_label(&keys, &ORBIT_FACE_KEYS),
        &idle_label(&guidance.0, OrbitCamInteractionKind::Orbit),
        "Orbit",
    );
    let pan = held_label(
        &mut hold.pan,
        time.delta(),
        pressed_key_label(&keys, &PAN_FACE_KEYS),
        &idle_label(&guidance.0, OrbitCamInteractionKind::Pan),
        "Pan",
    );
    let zoom = held_label(
        &mut hold.zoom,
        time.delta(),
        pressed_key_label(&keys, &ZOOM_FACE_KEYS),
        &idle_label(&guidance.0, OrbitCamInteractionKind::Zoom),
        "Zoom",
    );

    labels.for_each_mut(|kind, label| {
        let next = match kind {
            KeyboardFaceLabel::Orbit => orbit.as_str(),
            KeyboardFaceLabel::Pan => pan.as_str(),
            KeyboardFaceLabel::Zoom => zoom.as_str(),
        };
        if label.text() != next {
            label.set_text(next);
        }
    });
}

/// The control labels configured for `kind`, joined for the idle face display.
fn idle_label(summary: &OrbitCamControlSummary, kind: OrbitCamInteractionKind) -> String {
    summary
        .rows
        .iter()
        .filter(|row| row.kind == kind)
        .map(|row| row.label.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn pressed_key_label(keys: &ButtonInput<KeyCode>, bindings: &[(KeyCode, &str)]) -> Option<String> {
    let pressed = bindings
        .iter()
        .filter_map(|(key, label)| keys.pressed(*key).then_some(*label))
        .collect::<Vec<_>>();
    (!pressed.is_empty()).then(|| pressed.join("+"))
}

fn held_label(
    hold: &mut ReleaseHold<String>,
    delta: std::time::Duration,
    pressed: Option<String>,
    idle: &str,
    action: &str,
) -> String {
    match hold.update(delta, pressed) {
        HoldState::Active(pressed) | HoldState::Held(pressed) => {
            action_face_label(action, idle, Some(pressed))
        },
        HoldState::Idle => action_face_label(action, idle, None),
    }
}

fn action_face_label(action: &str, idle: &str, pressed: Option<&str>) -> String {
    if let Some(pressed) = pressed {
        format!("{action}: {pressed}")
    } else {
        format!("{action}: {idle}")
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// DESCRIPTION PANEL — on-screen explainer for the keyboard-bindings flow.
// ═════════════════════════════════════════════════════════════════════════════

const DESCRIPTION_TITLE: &str = "Keyboard Bindings";
const DESCRIPTION_LINES: [&str; 4] = [
    "This example uses OrbitCamInputMode::with_preset(OrbitCamPreset::keyboard()) with keyboard-only controls.",
    "Use it when the camera should have a keymap and Lagrange should route input.",
    "Unlike Manual mode, your app does not write per-frame input to camera intent (orbit, pan, zoom).",
    "Mapped keys can be held at the same time and will apply in the same frame.",
];

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_TITLE)
        .with_fit_width()
        .with_body_size(LABEL_SIZE.0)
        .lines(DESCRIPTION_LINES)
}

// ═════════════════════════════════════════════════════════════════════════════
// SCENE SCAFFOLDING — cube body and ground sized to match.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_GROUND_CLEARANCE: f32 = 0.1;
const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = fairy_dust::example_cube_on_ground(CUBE_GROUND_CLEARANCE);
