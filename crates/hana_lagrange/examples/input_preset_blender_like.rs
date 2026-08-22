//! Attaches a tuned Blender-like input preset with `OrbitCam::with_preset`.
//! Mouse-backed input uses 0.65 sensitivity; smooth-scroll input uses 0.35.
//! Alt+S toggles slow orbit, pan, and zoom (5% scale).
//!
//! The cube faces display the effective controls while the summary remains
//! labeled `Preset / BlenderLike`.

use bevy::prelude::Assets;
use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
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
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::CubeFacePanelContent;
use fairy_dust::CubeFacePanelStyle;
use fairy_dust::Face;
use fairy_dust::FairyDustOrbitCam;
use fairy_dust::HoldState;
use fairy_dust::ReleaseHold;
use fairy_dust::TitleBar;
use fairy_dust::cube_face_panel;
use fairy_dust::cube_face_panel_tree;
use fairy_dust::cube_face_transform;
use hana_diegetic::DiegeticPanelCommands;
use hana_lagrange::InteractionSources;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamBlenderLikePreset;
use hana_lagrange::OrbitCamControlSummary;
use hana_lagrange::OrbitCamInputGain;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamInteractionKind;
use hana_lagrange::OrbitCamInteractionStarted;
use hana_lagrange::OrbitCamInteractionState;
use hana_lagrange::ZoomDirection;
use hana_lagrange::describe_orbit_cam_controls;

const EXAMPLE_TITLE: &str = "Blender-Like";

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
        .insert((CameraHomeTarget, BlenderLikeCube))
        .with_camera_home()
        .yaw(CAMERA_YAW)
        .pitch(CAMERA_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft),
        )
        .with_cube_spin::<BlenderLikeCube>()
        .with_camera_control_panel()
        .lock_camera_preset()
        .add_systems(Startup, spawn_camera)
        .add_systems(PostStartup, spawn_face_labels)
        .insert_resource(FaceLabelHold::default())
        .add_systems(Update, update_face_labels)
        .add_observer(capture_zoom_started)
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// BLENDER-LIKE PRESET — tuned OrbitCam::with_preset input mode.
// ═════════════════════════════════════════════════════════════════════════════

const CAMERA_PITCH: f32 = 0.45;
const CAMERA_YAW: f32 = 0.55;
const HOME_MARGIN: f32 = 0.5;
const TUNED_MOUSE_INPUT_GAIN: f32 = 0.65;
const TUNED_SMOOTH_SCROLL_INPUT_GAIN: f32 = 0.35;

/// Marks the `OrbitCam` used by the `fairy_dust` face-panel showcase for
/// `OrbitCamInputMode` and `OrbitCamInteractionState` queries.
///
/// Production code using `OrbitCam::with_preset` does not need this marker.
#[derive(Component)]
struct BlenderLikeCamera;

fn tuned_blender_like_preset() -> OrbitCamBlenderLikePreset {
    OrbitCamBlenderLikePreset::default()
        .mouse_input_gain(OrbitCamInputGain::uniform(TUNED_MOUSE_INPUT_GAIN))
        .smooth_scroll_input_gain(OrbitCamInputGain::uniform(TUNED_SMOOTH_SCROLL_INPUT_GAIN))
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Transform::from_xyz(0.0, 1.5, 5.0),
        OrbitCam::with_preset(tuned_blender_like_preset()),
        BlenderLikeCamera,
        FairyDustOrbitCam,
    ));
}

// ═════════════════════════════════════════════════════════════════════════════
// CUBE FACE PANELS — Blender-like binding controls that light on input.
// ═════════════════════════════════════════════════════════════════════════════

const FACE_PANEL_STYLE: CubeFacePanelStyle = {
    let mut style = CubeFacePanelStyle::for_cube(CUBE_SIZE);
    style.title_size *= 1.5;
    style.body_size *= 1.5;
    style.active_body_size *= 1.5;
    style
};
const FACE_PANEL_NAME: &str = "Blender-like face panel";

#[derive(Component)]
struct BlenderLikeCube;

#[derive(Component, Clone, Copy)]
enum FaceLabel {
    Orbit,
    Pan,
    Zoom,
}

impl FaceLabel {
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

/// Holds the binding set's described controls so idle/active face labels share the
/// camera control panel's vocabulary.
#[derive(Resource)]
struct FaceGuidance(OrbitCamControlSummary);

#[derive(Resource, Default)]
struct FaceLabelHold {
    orbit: ReleaseHold<Vec<String>>,
    pan:   ReleaseHold<Vec<String>>,
    zoom:  ReleaseHold<Vec<String>>,
}

fn spawn_face_labels(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cubes: Query<Entity, With<BlenderLikeCube>>,
    cameras: Query<&OrbitCamInputMode, With<BlenderLikeCamera>>,
) {
    let Ok(cube) = cubes.single() else {
        return;
    };
    let Ok(mode) = cameras.single() else {
        return;
    };

    let summary = describe_orbit_cam_controls(mode);
    commands.entity(cube).with_children(|parent| {
        for face in [Face::Front, Face::Back] {
            spawn_face_panel(parent, face, FaceLabel::Orbit, &summary, &mut materials);
        }
        for face in [Face::Left, Face::Right] {
            spawn_face_panel(parent, face, FaceLabel::Pan, &summary, &mut materials);
        }
        for face in [Face::Top, Face::Bottom] {
            spawn_face_panel(parent, face, FaceLabel::Zoom, &summary, &mut materials);
        }
    });
    commands.insert_resource(FaceGuidance(summary));
}

fn update_face_labels(
    mut commands: Commands,
    time: Res<Time>,
    mut hold: ResMut<FaceLabelHold>,
    guidance: Res<FaceGuidance>,
    cameras: Query<&OrbitCamInteractionState, With<BlenderLikeCamera>>,
    labels: Query<(Entity, &FaceLabel)>,
) {
    let Ok(interaction_state) = cameras.single() else {
        return;
    };

    let orbit = held_content(
        &mut hold.orbit,
        time.delta(),
        active_labels(
            &guidance.0,
            OrbitCamInteractionKind::Orbit,
            interaction_state.orbit_sources(),
            None,
        ),
        &guidance.0,
        FaceLabel::Orbit,
    );
    let pan = held_content(
        &mut hold.pan,
        time.delta(),
        active_labels(
            &guidance.0,
            OrbitCamInteractionKind::Pan,
            interaction_state.pan_sources(),
            None,
        ),
        &guidance.0,
        FaceLabel::Pan,
    );
    let zoom = held_content(
        &mut hold.zoom,
        time.delta(),
        active_labels(
            &guidance.0,
            OrbitCamInteractionKind::Zoom,
            interaction_state.zoom_sources(),
            interaction_state.zoom_direction(),
        ),
        &guidance.0,
        FaceLabel::Zoom,
    );

    for (entity, kind) in &labels {
        let next = match kind {
            FaceLabel::Orbit => orbit.clone(),
            FaceLabel::Pan => pan.clone(),
            FaceLabel::Zoom => zoom.clone(),
        };
        if let Err(error) = commands.set_tree(entity, cube_face_panel_tree(FACE_PANEL_STYLE, next))
        {
            warn!("failed to replace input panel {entity:?} tree: {error}");
        }
    }
}

fn capture_zoom_started(
    event: On<OrbitCamInteractionStarted>,
    cameras: Query<&OrbitCamInteractionState, With<BlenderLikeCamera>>,
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

fn held_content(
    hold: &mut ReleaseHold<Vec<String>>,
    delta: std::time::Duration,
    active_lines: Option<Vec<String>>,
    summary: &OrbitCamControlSummary,
    label: FaceLabel,
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

fn spawn_face_panel(
    parent: &mut ChildSpawnerCommands,
    face: Face,
    kind: FaceLabel,
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
            error!("input_preset_blender_like: failed to build cube face panel: {error}");
        },
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// SCENE SCAFFOLDING — cube body and ground sized to match.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_GROUND_CLEARANCE: f32 = 0.1;
const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = fairy_dust::example_cube_on_ground(CUBE_GROUND_CLEARANCE);
