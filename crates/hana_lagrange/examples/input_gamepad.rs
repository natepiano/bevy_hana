//! Spawns an `OrbitCam` with `OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad())`
//! and uses the filled camera-home bindings for H and Select. A
//! `GamepadConnection` resource updates the connection chip via
//! `wire_chip_to_activation`. Cube faces show the preset's orbit / pan / zoom
//! controls and light up while sticks and triggers move.
//!
//! Controls:
//!   Orbit — right stick (RB + RS for slow)
//!   Pan   — left stick (LB + LS for slow)
//!   Zoom  — RT in / LT out (RB + RT, LB + LT for slow)
//!   H / Select — return to the camera home pose

use std::time::Duration;

use bevy::input::gamepad::Gamepad;
use bevy::input::gamepad::GamepadButton;
use bevy::prelude::Assets;
use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Name;
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
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::prelude::warn;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::CubeFacePanelContent;
use fairy_dust::CubeFacePanelStyle;
use fairy_dust::CubeSpinConfig;
use fairy_dust::CubeSpinControl;
use fairy_dust::Face;
use fairy_dust::FairyDustOrbitCam;
use fairy_dust::HoldState;
use fairy_dust::ReleaseHold;
use fairy_dust::TitleBar;
use fairy_dust::TitleChip;
use fairy_dust::TitleChipActivation;
use fairy_dust::apply_example_orbit_cam_limits;
use fairy_dust::cube_face_panel_tree;
use fairy_dust::cube_face_panel_with_tree;
use fairy_dust::cube_face_transform;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::El;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::Sizing;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_lagrange::CameraInputRoutingConfig;
use hana_lagrange::ControlSpeed;
use hana_lagrange::NoPositionFallback;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamControlSummary;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamInteractionKind;
use hana_lagrange::OrbitCamInteractionState;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::describe_orbit_cam_controls;

const CUBE_SPIN_CHIP_ID: &str = "cube_spin_pause";
const CUBE_SPIN_CHIP_LABEL: &str = "GamepadButton::West - Pause";
const EXAMPLE_TITLE: &str = "Gamepad";

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
        .insert((CameraHomeTarget, GamepadInputCube))
        .with_camera_home()
        .yaw(CAMERA_YAW)
        .pitch(CAMERA_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(GAMEPAD_CONNECTED_CONTROL),
        )
        .wire_chip_to_activation::<GamepadConnection>(GAMEPAD_CONNECTED_CONTROL)
        .with_cube_spin_config::<GamepadInputCube>(
            CubeSpinConfig::new()
                .without_key()
                .with_chip(TitleChip::new(CUBE_SPIN_CHIP_ID, CUBE_SPIN_CHIP_LABEL)),
        )
        .init_resource::<GamepadConnection>()
        .insert_resource(FaceLabelHold::default())
        .with_camera_control_panel()
        .lock_camera_preset()
        .add_systems(Startup, spawn_camera)
        .add_systems(PostStartup, spawn_face_labels)
        .add_systems(
            Update,
            (
                update_gamepad_connection,
                update_face_labels,
                toggle_spin_on_gamepad_west,
            ),
        )
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// GAMEPAD CAMERA — OrbitCamPreset::gamepad() + filled H / Select home bindings.
//
// How it works:
//   1. `spawn_camera` installs `OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad())` so
//      Lagrange reads the gamepad sticks and triggers for orbit, pan, and zoom. Fairy Dust fills H
//      and Select into the preset's home bindings.
//   2. `update_gamepad_connection` polls `Query<&Gamepad>` and updates `GamepadConnection`; the
//      connection chip is updated through `wire_chip_to_activation`.
//   3. H and Select use the same stored-pose home glide as every preset-mode camera.
// ═════════════════════════════════════════════════════════════════════════════

const CAMERA_FOCUS: Vec3 = CUBE_TRANSLATION;
const CAMERA_PITCH: f32 = 0.45;
const CAMERA_RADIUS: f32 = 6.0;
const CAMERA_YAW: f32 = 0.55;
const GAMEPAD_CONNECTED_CONTROL: &str = "Gamepad Connected";
const HOME_MARGIN: f32 = 0.5;

#[derive(Resource)]
struct GamepadConnection {
    control_activation: ControlActivation,
}

impl Default for GamepadConnection {
    fn default() -> Self {
        Self {
            control_activation: ControlActivation::Inactive,
        }
    }
}

impl TitleChipActivation for GamepadConnection {
    fn activation(&self) -> ControlActivation { self.control_activation }
}

fn spawn_camera(mut commands: Commands) {
    let mut camera = OrbitCam::from_pose(CAMERA_FOCUS, (CAMERA_YAW, CAMERA_PITCH), CAMERA_RADIUS);
    apply_example_orbit_cam_limits(&mut camera);
    commands.spawn((
        camera,
        OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()),
        FairyDustOrbitCam,
    ));
}

fn update_gamepad_connection(
    gamepads: Query<(), With<Gamepad>>,
    mut connection: ResMut<GamepadConnection>,
) {
    let control_activation = if gamepads.is_empty() {
        ControlActivation::Inactive
    } else {
        ControlActivation::Active
    };
    if connection.control_activation != control_activation {
        connection.control_activation = control_activation;
    }
}

fn toggle_spin_on_gamepad_west(
    gamepads: Query<&Gamepad>,
    mut control: ResMut<CubeSpinControl<GamepadInputCube>>,
) {
    if gamepads
        .iter()
        .any(|gamepad| gamepad.just_pressed(GamepadButton::West))
    {
        control.toggle();
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// CUBE FACE PANELS — gamepad controls grouped by camera action.
// ═════════════════════════════════════════════════════════════════════════════

const FACE_PANEL_STYLE: CubeFacePanelStyle = CubeFacePanelStyle {
    title_size: 78.0,
    body_size: 48.0,
    active_body_size: 56.0,
    ..CubeFacePanelStyle::for_cube(CUBE_SIZE)
};
const GAMEPAD_FACE_PANEL_NAME: &str = "Gamepad input face panel";
const ORBIT_CONTROL_LABEL: &str = "rs";
const ORBIT_SLOW_CONTROL_LABEL: &str = "rb+rs";
const PAN_CONTROL_LABEL: &str = "ls";
const PAN_SLOW_CONTROL_LABEL: &str = "lb+ls";
const STICK_ACTIVE_THRESHOLD: f32 = 0.18;
const STICK_DOWN_LABEL: &str = "down";
const STICK_LEFT_LABEL: &str = "left";
const STICK_RIGHT_LABEL: &str = "right";
const STICK_UP_LABEL: &str = "up";
const TRIGGER_ACTIVE_THRESHOLD: f32 = 0.05;
const ZOOM_DIRECTION_IN_LABEL: &str = "In";
const ZOOM_DIRECTION_OUT_LABEL: &str = "Out";
const ZOOM_IN_CONTROL_LABEL: &str = "rt";
const ZOOM_IN_SLOW_CONTROL_LABEL: &str = "rb+rt";
const ZOOM_OUT_CONTROL_LABEL: &str = "lt";
const ZOOM_OUT_SLOW_CONTROL_LABEL: &str = "lb+lt";
/// Gap between the idle table's speed column and its controls column.
const CONTROL_TABLE_COLUMN_GAP: f32 = CUBE_SIZE * 0.04;
/// Width of the idle table's `normal` / `slow` speed column, as a fraction of
/// the face width.
const SPEED_COLUMN_FRACTION: f32 = 0.34;

#[derive(Component)]
struct GamepadInputCube;

#[derive(Component, Clone, Copy)]
enum GamepadFaceLabel {
    Orbit,
    Pan,
    Zoom,
}

impl GamepadFaceLabel {
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

/// Holds the preset's described controls, read to build the idle face tables.
/// Live faces still show the active stick or trigger.
#[derive(Resource)]
struct FaceGuidance(OrbitCamControlSummary);

#[derive(Resource, Default)]
struct FaceLabelHold {
    orbit: ReleaseHold<CubeFacePanelContent>,
    pan:   ReleaseHold<CubeFacePanelContent>,
    zoom:  ReleaseHold<CubeFacePanelContent>,
}

fn spawn_face_labels(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cubes: Query<Entity, With<GamepadInputCube>>,
) {
    let Ok(cube) = cubes.single() else {
        return;
    };

    let summary =
        describe_orbit_cam_controls(&OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()));
    commands.entity(cube).with_children(|parent| {
        for face in [Face::Front, Face::Back] {
            spawn_face_panel(
                parent,
                face,
                GamepadFaceLabel::Orbit,
                &summary,
                &mut materials,
            );
        }
        for face in [Face::Left, Face::Right] {
            spawn_face_panel(
                parent,
                face,
                GamepadFaceLabel::Pan,
                &summary,
                &mut materials,
            );
        }
        for face in [Face::Top, Face::Bottom] {
            spawn_face_panel(
                parent,
                face,
                GamepadFaceLabel::Zoom,
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
    gamepads: Query<&Gamepad>,
    cameras: Query<&OrbitCamInteractionState, With<FairyDustOrbitCam>>,
    mut hold: ResMut<FaceLabelHold>,
    guidance: Res<FaceGuidance>,
    labels: Query<(Entity, &GamepadFaceLabel)>,
) {
    let active_gamepad = gamepads
        .iter()
        .find(|gamepad| gamepad_has_input(gamepad))
        .or_else(|| gamepads.iter().next());
    // The engine's resolved speed is the single source of truth for "Slow" —
    // the example never re-reads the rb/lb gate buttons itself.
    let orbit_cam_interaction_state = cameras.iter().next().copied().unwrap_or_default();

    let orbit = held_content(
        &mut hold.orbit,
        time.delta(),
        active_orbit_content(
            active_gamepad,
            orbit_cam_interaction_state.speed(OrbitCamInteractionKind::Orbit),
        ),
    );
    let pan = held_content(
        &mut hold.pan,
        time.delta(),
        active_pan_content(
            active_gamepad,
            orbit_cam_interaction_state.speed(OrbitCamInteractionKind::Pan),
        ),
    );
    let zoom = held_content(
        &mut hold.zoom,
        time.delta(),
        active_zoom_content(
            active_gamepad,
            orbit_cam_interaction_state.speed(OrbitCamInteractionKind::Zoom),
        ),
    );

    for (entity, label) in &labels {
        let active = match label {
            GamepadFaceLabel::Orbit => orbit.clone(),
            GamepadFaceLabel::Pan => pan.clone(),
            GamepadFaceLabel::Zoom => zoom.clone(),
        };
        if let Err(error) = commands.set_tree(entity, face_tree(*label, active, &guidance.0)) {
            warn!("failed to replace gamepad panel {entity:?} tree: {error}");
        }
    }
}

fn spawn_face_panel(
    parent: &mut ChildSpawnerCommands,
    face: Face,
    kind: GamepadFaceLabel,
    summary: &OrbitCamControlSummary,
    materials: &mut Assets<StandardMaterial>,
) {
    match cube_face_panel_with_tree(
        FACE_PANEL_STYLE.size,
        idle_grid_tree(kind, summary),
        materials,
    ) {
        Ok(panel) => {
            parent.spawn((
                Name::new(GAMEPAD_FACE_PANEL_NAME),
                kind,
                panel,
                cube_face_transform(face, CUBE_SIZE),
            ));
        },
        Err(error) => {
            error!("input_gamepad: failed to build cube face panel: {error}");
        },
    }
}

fn gamepad_has_input(gamepad: &Gamepad) -> bool {
    gamepad.right_stick().length() > STICK_ACTIVE_THRESHOLD
        || gamepad.left_stick().length() > STICK_ACTIVE_THRESHOLD
        || trigger_value(gamepad, GamepadButton::RightTrigger2) > TRIGGER_ACTIVE_THRESHOLD
        || trigger_value(gamepad, GamepadButton::LeftTrigger2) > TRIGGER_ACTIVE_THRESHOLD
}

fn active_orbit_content(
    gamepad: Option<&Gamepad>,
    speed: Option<ControlSpeed>,
) -> Option<CubeFacePanelContent> {
    let gamepad = gamepad?;
    let speed = speed?;
    let stick = gamepad.right_stick();
    if stick.length() <= STICK_ACTIVE_THRESHOLD {
        return None;
    }

    let slow = speed == ControlSpeed::Slow;
    let control = spell_out_control(if slow {
        ORBIT_SLOW_CONTROL_LABEL
    } else {
        ORBIT_CONTROL_LABEL
    });
    Some(CubeFacePanelContent::active(
        slow_title(GamepadFaceLabel::Orbit.title(), slow),
        vec![control, stick_direction(stick)],
    ))
}

fn active_pan_content(
    gamepad: Option<&Gamepad>,
    speed: Option<ControlSpeed>,
) -> Option<CubeFacePanelContent> {
    let gamepad = gamepad?;
    let speed = speed?;
    let stick = gamepad.left_stick();
    if stick.length() <= STICK_ACTIVE_THRESHOLD {
        return None;
    }

    let slow = speed == ControlSpeed::Slow;
    let control = spell_out_control(if slow {
        PAN_SLOW_CONTROL_LABEL
    } else {
        PAN_CONTROL_LABEL
    });
    Some(CubeFacePanelContent::active(
        slow_title(GamepadFaceLabel::Pan.title(), slow),
        vec![control, stick_direction(stick)],
    ))
}

fn active_zoom_content(
    gamepad: Option<&Gamepad>,
    speed: Option<ControlSpeed>,
) -> Option<CubeFacePanelContent> {
    let gamepad = gamepad?;
    let speed = speed?;
    let slow = speed == ControlSpeed::Slow;

    // The right trigger zooms in, the left zooms out.
    let zoom_in_trigger = trigger_value(gamepad, GamepadButton::RightTrigger2);
    let zoom_out_trigger = trigger_value(gamepad, GamepadButton::LeftTrigger2);

    let mut lines = Vec::new();
    if zoom_in_trigger > TRIGGER_ACTIVE_THRESHOLD {
        lines.push(spell_out_control(if slow {
            ZOOM_IN_SLOW_CONTROL_LABEL
        } else {
            ZOOM_IN_CONTROL_LABEL
        }));
    }
    if zoom_out_trigger > TRIGGER_ACTIVE_THRESHOLD {
        lines.push(spell_out_control(if slow {
            ZOOM_OUT_SLOW_CONTROL_LABEL
        } else {
            ZOOM_OUT_CONTROL_LABEL
        }));
    }

    if lines.is_empty() {
        None
    } else {
        Some(CubeFacePanelContent::active(
            zoom_title(slow, zoom_in_trigger >= zoom_out_trigger),
            lines,
        ))
    }
}

/// Appends a `Slow` suffix to a face title when the slow gate is engaged.
fn slow_title(base: &str, slow: bool) -> String {
    if slow {
        format!("{base} Slow")
    } else {
        base.to_string()
    }
}

/// The zoom face title, naming the direction the engaged trigger drives — the
/// right trigger zooms in, the left zooms out — plus the `Slow` gate when held.
fn zoom_title(slow: bool, zooming_in: bool) -> String {
    let direction = if zooming_in {
        ZOOM_DIRECTION_IN_LABEL
    } else {
        ZOOM_DIRECTION_OUT_LABEL
    };
    let face_title = GamepadFaceLabel::Zoom.title();
    if slow {
        format!("{face_title} Slow {direction}")
    } else {
        format!("{face_title} {direction}")
    }
}

fn held_content(
    hold: &mut ReleaseHold<CubeFacePanelContent>,
    delta: Duration,
    active: Option<CubeFacePanelContent>,
) -> Option<CubeFacePanelContent> {
    match hold.update(delta, active) {
        HoldState::Active(content) | HoldState::Held(content) => Some(content.clone()),
        HoldState::Idle => None,
    }
}

/// Builds a face's layout tree: the active readout when a control is engaged,
/// otherwise the idle two-column control table.
fn face_tree(
    kind: GamepadFaceLabel,
    active: Option<CubeFacePanelContent>,
    summary: &OrbitCamControlSummary,
) -> LayoutTree {
    active.map_or_else(
        || idle_grid_tree(kind, summary),
        |content| cube_face_panel_tree(FACE_PANEL_STYLE, content),
    )
}

/// Builds the idle control table: a left `normal` / `slow` column beside a
/// column of spelled-out controls, each speed label centered against its rows.
fn idle_grid_tree(kind: GamepadFaceLabel, summary: &OrbitCamControlSummary) -> LayoutTree {
    let style = FACE_PANEL_STYLE;
    let title_style = TextStyle::new(style.title_size)
        .with_color(style.color)
        .with_align(TextAlign::Center)
        .with_shadow_mode(GlyphShadowMode::None);
    let label_style = TextStyle::new(style.body_size)
        .with_color(style.color)
        .with_align(TextAlign::Left)
        .with_shadow_mode(GlyphShadowMode::None);

    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(style.size))
            .height(Sizing::fixed(style.size))
            .alignment(AlignX::Center, AlignY::Center)
            .gap(style.row_gap)
            .padding(Padding::all(style.padding))
            .clip(),
    );
    builder.text((kind.title(), title_style));
    for (speed_label, controls) in idle_speed_groups(summary, kind.kind()) {
        builder.with(
            El::row()
                .width(Sizing::GROW)
                .alignment(AlignX::Left, AlignY::Center)
                .gap(CONTROL_TABLE_COLUMN_GAP),
            |group| {
                group.with(
                    El::new()
                        .width(Sizing::percent(SPEED_COLUMN_FRACTION))
                        .alignment(AlignX::Left, AlignY::Center),
                    |cell| {
                        cell.text((speed_label, label_style.clone()));
                    },
                );
                group.with(
                    El::column()
                        .width(Sizing::GROW)
                        .alignment(AlignX::Left, AlignY::Center)
                        .gap(style.row_gap),
                    |column| {
                        for control in &controls {
                            column.text((control.clone(), label_style.clone()));
                        }
                    },
                );
            },
        );
    }
    builder.build()
}

/// Groups a kind's idle rows by speed, spelled out: `(speed label, controls)`
/// in `normal`-then-`slow` order, skipping speeds with no bindings.
fn idle_speed_groups(
    summary: &OrbitCamControlSummary,
    kind: OrbitCamInteractionKind,
) -> Vec<(&'static str, Vec<String>)> {
    [ControlSpeed::Normal, ControlSpeed::Slow]
        .into_iter()
        .filter_map(|speed| {
            let controls: Vec<String> = summary
                .rows
                .iter()
                .filter(|row| row.kind == kind && row.speed == speed)
                .map(|row| spell_out_control(&row.label))
                .collect();
            (!controls.is_empty()).then_some((speed_prefix(speed), controls))
        })
        .collect()
}

/// Spells out a gamepad control abbreviation for the cube faces. Compound labels
/// like `rb+rs` expand each token: `right bumper + right stick`.
fn spell_out_control(label: &str) -> String {
    label
        .split('+')
        .map(|token| match token {
            "ls" => "left stick",
            "rs" => "right stick",
            "lb" => "left bumper",
            "rb" => "right bumper",
            "lt" => "left trigger",
            "rt" => "right trigger",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Idle-row prefix that names the binding's speed variant.
const fn speed_prefix(speed: ControlSpeed) -> &'static str {
    match speed {
        ControlSpeed::Normal => "normal",
        ControlSpeed::Slow => "slow",
    }
}

fn trigger_value(gamepad: &Gamepad, button: GamepadButton) -> f32 {
    gamepad.get(button).unwrap_or(0.0)
}

fn stick_direction(stick: Vec2) -> String {
    let mut parts = Vec::new();
    if stick.y > STICK_ACTIVE_THRESHOLD {
        parts.push(STICK_UP_LABEL);
    } else if stick.y < -STICK_ACTIVE_THRESHOLD {
        parts.push(STICK_DOWN_LABEL);
    }
    if stick.x > STICK_ACTIVE_THRESHOLD {
        parts.push(STICK_RIGHT_LABEL);
    } else if stick.x < -STICK_ACTIVE_THRESHOLD {
        parts.push(STICK_LEFT_LABEL);
    }
    parts.join(" + ")
}

// ═════════════════════════════════════════════════════════════════════════════
// SCENE SCAFFOLDING — cube body and ground sized to match.
// ═════════════════════════════════════════════════════════════════════════════

const CUBE_GROUND_CLEARANCE: f32 = 0.1;
const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;
const CUBE_SIZE: f32 = fairy_dust::EXAMPLE_CUBE_SIZE;
const CUBE_TRANSLATION: Vec3 = fairy_dust::example_cube_on_ground(CUBE_GROUND_CLEARANCE);
