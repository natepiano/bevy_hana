//! Demonstrates choosing the camera time source so pausing virtual game time
//! does not pause camera controls. The camera spawns with a `TimeSource::Real`
//! component; `pause_game_system` toggles `Time<Virtual>`; `cube_rotator_system`
//! reads the default `Res<Time>` which resolves to virtual time inside `Update`,
//! so the cube freezes while the camera keeps lerping.
//!
//! Controls:
//!   P or Space — toggle game pause

use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Local;
use bevy::prelude::Name;
use bevy::prelude::PostStartup;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Virtual;
use bevy::prelude::With;
use bevy::prelude::warn;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::CubeFacePanelContent;
use fairy_dust::CubeFacePanelStyle;
use fairy_dust::DescriptionPanel;
use fairy_dust::Face;
use fairy_dust::LABEL_SIZE;
use fairy_dust::TitleBar;
use fairy_dust::TitleChipActivation;
use fairy_dust::cube_face_panel;
use fairy_dust::cube_face_panel_tree;
use fairy_dust::cube_face_transform;
use hana_diegetic::DiegeticPanelCommands;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::TimeSource;

const EXAMPLE_TITLE: &str = "Pausing";

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_ground_plane()
        .with_cube()
        .size(CUBE_SIZE)
        .color(CUBE_COLOR)
        .transform(Transform::from_translation(CUBE_TRANSLATION))
        .insert((Cube, CameraHomeTarget))
        .with_orbit_cam_preset_bundle(
            |_camera: &mut OrbitCam| {},
            OrbitCamPreset::blender_like(),
            TimeSource::Real,
        )
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(PAUSE_CONTROL),
        )
        .wire_chip_to_activation::<PauseState>(PAUSE_CONTROL)
        .with_description_panel(description_panel())
        .with_camera_control_panel()
        .init_resource::<PauseState>()
        .add_systems(PostStartup, spawn_face_panels)
        // `P` and `Space` both run `toggle_pause` through Fairy Dust's shortcut
        // binding, which fires it only when no modifier is held.
        .with_shortcut(KeyCode::KeyP, toggle_pause)
        .with_shortcut(KeyCode::Space, toggle_pause)
        // Chained so the panel refresh and cube rotation observe the same
        // `Time<Virtual>` state within a single frame.
        .add_systems(Update, (update_game_panels, cube_rotator_system).chain())
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// CAMERA TIME SOURCE & VIRTUAL-TIME PAUSE — the demonstrated API.
// The `TimeSource::Real` component on the camera plus `Time<Virtual>` pause is
// what makes the camera keep moving while the cube freezes.
//
// How it works:
//   1. The camera spawns with a `TimeSource::Real` component (passed as the spawn bundle to
//      `with_orbit_cam_preset_bundle`), so its smoothing reads wall-clock time.
//   2. `toggle_pause` toggles `Time<Virtual>`, bound to P / Space through Fairy Dust's shortcut
//      binding.
//   3. `cube_rotator_system` reads the default `Res<Time>`, which resolves to `Time<Virtual>`
//      inside `Update`, so its delta is zero while paused and the cube stops spinning. The camera,
//      reading real time, keeps lerping.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_ROTATION_SPEED: f32 = 0.8;
const PAUSE_CONTROL: &str = "P Pause";

#[derive(Component)]
struct Cube;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PauseState {
    #[default]
    Running,
    Paused,
}

impl PauseState {
    const fn control_activation(self) -> ControlActivation {
        match self {
            Self::Running => ControlActivation::Inactive,
            Self::Paused => ControlActivation::Active,
        }
    }
}

impl TitleChipActivation for PauseState {
    fn activation(&self) -> ControlActivation { self.control_activation() }
}

fn toggle_pause(mut time: ResMut<Time<Virtual>>, mut pause_state: ResMut<PauseState>) {
    if time.is_paused() {
        time.unpause();
    } else {
        time.pause();
    }
    *pause_state = if time.is_paused() {
        PauseState::Paused
    } else {
        PauseState::Running
    };
}

// `Res<Time>` resolves to `Time<Virtual>` in `Update`, so this freezes while
// the game is paused.
fn cube_rotator_system(time: Res<Time>, mut query: Query<&mut Transform, With<Cube>>) {
    for mut transform in &mut query {
        transform.rotate_y(CUBE_ROTATION_SPEED * time.delta_secs());
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// SCENE SCAFFOLDING — cube body, ground, and camera home placement.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_GROUND_CLEARANCE: f32 = 0.1;
const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = fairy_dust::example_cube_on_ground(CUBE_GROUND_CLEARANCE);

const HOME_PITCH: f32 = 0.42;
const HOME_YAW: f32 = -0.28;
const HOME_MARGIN: f32 = 0.5;

// ═════════════════════════════════════════════════════════════════════════════
// CUBE FACE PANELS — WorldText labels stuck to the cube faces that swap text
// when the pause state changes.
// ═════════════════════════════════════════════════════════════════════════════

const FACE_PANEL_TEXT_SIZE: f32 = 68.0;
const FACE_PANEL_STYLE: CubeFacePanelStyle = CubeFacePanelStyle {
    size:             CUBE_SIZE,
    padding:          0.04,
    row_gap:          0.05,
    title_size:       FACE_PANEL_TEXT_SIZE,
    body_size:        FACE_PANEL_TEXT_SIZE,
    active_body_size: FACE_PANEL_TEXT_SIZE,
    color:            fairy_dust::CUBE_FACE_PANEL_BLUE,
};

const CAMERA_LABEL: &str = "OrbitCam";
const CAMERA_TIME_SOURCE_LABEL: &str = "TimeSource::Real";
const CAMERA_TIME_SOURCE_PANEL_NAME: &str = "Camera time source panel";
const GAME_PAUSED_LABEL: &str = "Game Paused";
const GAME_TIME_SOURCE_LABEL: &str = "TimeSource::Virtual";
const GAME_TIME_SOURCE_PANEL_NAME: &str = "Game time source panel";
const GAME_UNPAUSED_LABEL: &str = "Game Unpaused";

#[derive(Component)]
struct GameStatusPanel;

fn spawn_face_panels(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cube_query: Query<Entity, With<Cube>>,
) {
    let Ok(cube) = cube_query.single() else {
        return;
    };

    let Ok(camera_panel) = cube_face_panel(
        FACE_PANEL_STYLE,
        panel_content(CAMERA_LABEL, CAMERA_TIME_SOURCE_LABEL),
        &mut materials,
    ) else {
        return;
    };
    let Ok(game_panel) = cube_face_panel(
        FACE_PANEL_STYLE,
        panel_content(game_status_title(false), GAME_TIME_SOURCE_LABEL),
        &mut materials,
    ) else {
        return;
    };

    commands.entity(cube).with_children(|parent| {
        for face in [Face::Front, Face::Back] {
            parent.spawn((
                Name::new(CAMERA_TIME_SOURCE_PANEL_NAME),
                camera_panel.clone(),
                cube_face_transform(face, CUBE_SIZE),
            ));
        }
        for face in [Face::Left, Face::Right] {
            parent.spawn((
                Name::new(GAME_TIME_SOURCE_PANEL_NAME),
                GameStatusPanel,
                game_panel.clone(),
                cube_face_transform(face, CUBE_SIZE),
            ));
        }
    });
}

fn update_game_panels(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    mut paused_state: Local<Option<bool>>,
    panels: Query<Entity, With<GameStatusPanel>>,
) {
    let paused = time.is_paused();
    if paused_state
        .as_ref()
        .is_some_and(|previous| *previous == paused)
    {
        return;
    }

    *paused_state = Some(paused);
    let tree = cube_face_panel_tree(
        FACE_PANEL_STYLE,
        panel_content(game_status_title(paused), GAME_TIME_SOURCE_LABEL),
    );

    for panel in &panels {
        if let Err(error) = commands.set_tree(panel, tree.clone()) {
            warn!("failed to replace pause panel {panel:?} tree: {error}");
        }
    }
}

fn panel_content(title: &'static str, time_source: &'static str) -> CubeFacePanelContent {
    CubeFacePanelContent::idle(title, [time_source])
}

const fn game_status_title(paused: bool) -> &'static str {
    if paused {
        GAME_PAUSED_LABEL
    } else {
        GAME_UNPAUSED_LABEL
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// UI — description panel explaining the two time sources on screen.
// ═════════════════════════════════════════════════════════════════════════════

const DESCRIPTION_HEADING: &str = "Time Sources";
const DESCRIPTION_LINES: [&str; 4] = [
    "Virtual time drives gameplay and respects pause.",
    "Real time is wall-clock time and keeps advancing.",
    "This camera opts into Real time for control smoothing.",
    "The cube freezes on pause; the OrbitCam still moves.",
];

fn description_panel() -> DescriptionPanel {
    DescriptionPanel::new(DESCRIPTION_HEADING)
        .with_fit_width()
        .with_body_size(LABEL_SIZE.0)
        .lines(DESCRIPTION_LINES)
}
