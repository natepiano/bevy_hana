//! Demonstrates a fixed `FreeCam` using the built-in keyboard/mouse preset.
//!
//! Controls:
//!   1 Free Flight
//!   2 Pitch Limit, +/- adjusts the shared clamp for modes 2 and 3
//!   3 Horizon Lock, the pitch limit plus roll locked to 0
//!   RMB drag - look
//!   Alt+I - toggle invert Y
//!   WASD + Space/Ctrl - translate
//!   Q/E - roll
//!   Alt+S - toggle slow mode
//!   H - return to the start pose
//!   G - cycle input preset: keyboard/mouse, then gamepad + southpaw when a gamepad is connected
//!       (greyed out as "(no gamepad)" while none is connected)
//!   Gamepad - sticks move/look, triggers up/down, bumpers roll, L3 boost, Select or H home

use std::f32::consts::FRAC_PI_2;

use bevy::input::gamepad::Gamepad;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::ChildOf;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Cuboid;
use bevy::prelude::Entity;
use bevy::prelude::GamepadButton;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Meshable;
use bevy::prelude::Name;
use bevy::prelude::Plane3d;
use bevy::prelude::Query;
use bevy::prelude::Real;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::default;
use fairy_dust::Anchor;
use fairy_dust::ControlActivation;
use fairy_dust::TitleBar;
use fairy_dust::TitleBarControl;
use fairy_dust::TitleBarOrientation;
use fairy_dust::TitleBarSegment;
use hana_lagrange::AnglePairLimit;
use hana_lagrange::FreeCam;
use hana_lagrange::FreeCamGamepadLayout;
use hana_lagrange::FreeCamGamepadPreset;
use hana_lagrange::FreeCamHomePose;
use hana_lagrange::FreeCamInputGain;
use hana_lagrange::FreeCamInputMode;
use hana_lagrange::FreeCamKeyboardMousePreset;
use hana_lagrange::FreeCamLookPitch;
use hana_lagrange::FreeCamPreset;
use hana_lagrange::Limit;
use hana_lagrange::LookAngles;
use hana_lagrange::Position;
use hana_lagrange::Roll;
use hana_lagrange::ScalarLimit;

// app
const EXAMPLE_TITLE: &str = "FreeCam";

// camera
const CAMERA_NAME: &str = "FreeCam";
const CAMERA_POSITION: Vec3 = Vec3::ZERO;
const CAMERA_LOOK: LookAngles = LookAngles {
    yaw:   0.7,
    pitch: 0.08,
};
const CAMERA_ROLL: Roll = Roll(0.0);
const GAMEPAD_INPUT_GAIN: FreeCamInputGain = FreeCamInputGain::new()
    .translate(GAMEPAD_TRANSLATE_INPUT_GAIN)
    .look(GAMEPAD_LOOK_INPUT_GAIN);
const GAMEPAD_LOOK_INPUT_GAIN: f32 = 0.8;
const GAMEPAD_TRANSLATE_INPUT_GAIN: f32 = 0.9;
const KEYBOARD_MOUSE_INPUT_GAIN: FreeCamInputGain =
    FreeCamInputGain::new().look(KEYBOARD_MOUSE_LOOK_INPUT_GAIN);
const KEYBOARD_MOUSE_LOOK_INPUT_GAIN: f32 = 0.85;

// title bar
const CAMERA_HOME_CONTROL: &str = "H Home";
const CYCLE_INPUT_CONTROL: &str = "G Cycle Input";
const CYCLE_INPUT_NO_GAMEPAD_NOTE: &str = "(no gamepad)";
const FREE_FLIGHT_CONTROL: &str = "free-flight";
const PITCH_LIMITED_CONTROL: &str = "pitch-limited";
const HORIZON_LOCKED_CONTROL: &str = "horizon-locked";
const DECREASE_PITCH_LIMIT_CONTROL: &str = "decrease-pitch-limit";
const INCREASE_PITCH_LIMIT_CONTROL: &str = "increase-pitch-limit";

// pitch limit
const PITCH_LIMIT_MARGIN: f32 = 0.01;
const DEFAULT_PITCH_LIMIT: f32 = FRAC_PI_2 - PITCH_LIMIT_MARGIN;
const MIN_PITCH_LIMIT: f32 = 0.0;
const PITCH_LIMIT_ADJUST_RADIANS_PER_SECOND: f32 = 1.5;

// scene
const GRID_SIDE_COUNT: usize = 10;
const GRID_CENTER_OFFSET_CELLS: f32 = 4.5;
const GROUND_SIDE: f32 = 2.0;
const GRID_SPACING: f32 = GROUND_SIDE * 2.0;
const GRID_START: f32 = -GRID_CENTER_OFFSET_CELLS * GRID_SPACING;
const CUBE_SIZE: f32 = 0.6;
const CUBE_CENTER_Y: f32 = CUBE_SIZE * 0.5;
const GROUND_COLOR: Color = Color::srgb(0.11, 0.15, 0.13);
const CUBE_COLOR: Color = fairy_dust::EXAMPLE_CUBE_COLOR;

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .with_title_bar(free_cam_title_bar(DEFAULT_PITCH_LIMIT))
        .wire_chip_to_state::<FreeCamExamplePreset, _>(FREE_FLIGHT_CONTROL, |preset| {
            preset.activation_for_control(FreeCamExamplePreset::FreeFlight)
        })
        .wire_chip_to_state::<FreeCamExamplePreset, _>(PITCH_LIMITED_CONTROL, |preset| {
            preset.activation_for_control(FreeCamExamplePreset::PitchLimited)
        })
        .wire_chip_to_state::<FreeCamExamplePreset, _>(HORIZON_LOCKED_CONTROL, |preset| {
            preset.activation_for_control(FreeCamExamplePreset::HorizonLocked)
        })
        .wire_chip_to_state::<PitchLimitAdjustment, _>(DECREASE_PITCH_LIMIT_CONTROL, |adjustment| {
            adjustment.decrease_activation()
        })
        .wire_chip_to_state::<PitchLimitAdjustment, _>(INCREASE_PITCH_LIMIT_CONTROL, |adjustment| {
            adjustment.increase_activation()
        })
        .wire_chip_to_state::<GamepadAvailability, _>(CYCLE_INPUT_CONTROL, |availability| {
            availability.cycle_input_activation()
        })
        .init_resource::<FreeCamExamplePreset>()
        .init_resource::<PitchLimit>()
        .init_resource::<PitchLimitAdjustment>()
        .init_resource::<FreeCamInputDevice>()
        .init_resource::<GamepadAvailability>()
        .with_camera_control_panel()
        .lock_camera_preset()
        .add_systems(Startup, (spawn_camera, spawn_grid))
        .add_systems(
            Update,
            (
                update_pitch_limit,
                revert_free_cam_to_keyboard_mouse,
                track_gamepad_availability,
            ),
        )
        .with_shortcut(KeyCode::Digit1, select_free_flight)
        .with_shortcut(KeyCode::Digit2, select_pitch_limited)
        .with_shortcut(KeyCode::Digit3, select_horizon_locked)
        .with_shortcut(KeyCode::KeyG, cycle_free_cam_input_device)
        .run();
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FreeCamExamplePreset {
    #[default]
    FreeFlight,
    PitchLimited,
    HorizonLocked,
}

impl FreeCamExamplePreset {
    const fn activation_for_control(self, control: Self) -> ControlActivation {
        match (self, control) {
            (Self::FreeFlight, Self::FreeFlight)
            | (Self::PitchLimited, Self::PitchLimited)
            | (Self::HorizonLocked, Self::HorizonLocked) => ControlActivation::Active,
            _ => ControlActivation::Inactive,
        }
    }

    fn apply_to(self, camera: &mut FreeCam, pitch_limit: f32) {
        match self {
            Self::FreeFlight => {
                constrain_look(camera, AnglePairLimit::default());
                constrain_roll(camera, ScalarLimit::default());
            },
            Self::PitchLimited => {
                constrain_look(camera, pitch_limit_constraint(pitch_limit));
                constrain_roll(camera, ScalarLimit::default());
            },
            Self::HorizonLocked => {
                constrain_look(camera, pitch_limit_constraint(pitch_limit));
                constrain_roll(camera, ScalarLimit::Clamp { min: 0.0, max: 0.0 });
            },
        }
        camera.force_update();
    }
}

#[derive(Resource, Clone, Copy, Debug)]
struct PitchLimit {
    radians: f32,
}

impl Default for PitchLimit {
    fn default() -> Self {
        Self {
            radians: DEFAULT_PITCH_LIMIT,
        }
    }
}

impl PitchLimit {
    const fn radians(self) -> f32 { self.radians }

    fn adjust(&mut self, delta: f32) {
        self.radians = (self.radians + delta).clamp(MIN_PITCH_LIMIT, DEFAULT_PITCH_LIMIT);
    }
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PitchLimitAdjustment {
    #[default]
    None,
    Decrease,
    Increase,
    Both,
}

impl PitchLimitAdjustment {
    fn from_keyboard(keys: &ButtonInput<KeyCode>) -> Self {
        let decrease_pressed =
            keys.pressed(KeyCode::Minus) || keys.pressed(KeyCode::NumpadSubtract);
        let increase_pressed = keys.pressed(KeyCode::Equal) || keys.pressed(KeyCode::NumpadAdd);

        match (decrease_pressed, increase_pressed) {
            (false, false) => Self::None,
            (true, false) => Self::Decrease,
            (false, true) => Self::Increase,
            (true, true) => Self::Both,
        }
    }

    const fn decrease_activation(self) -> ControlActivation {
        match self {
            Self::Decrease | Self::Both => ControlActivation::Active,
            Self::None | Self::Increase => ControlActivation::Inactive,
        }
    }

    const fn increase_activation(self) -> ControlActivation {
        match self {
            Self::Increase | Self::Both => ControlActivation::Active,
            Self::None | Self::Decrease => ControlActivation::Inactive,
        }
    }

    const fn direction(self) -> f32 {
        match self {
            Self::Decrease => -1.0,
            Self::Increase => 1.0,
            Self::None | Self::Both => 0.0,
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FreeCamInputDevice {
    #[default]
    KeyboardMouse,
    Gamepad,
    GamepadSouthpaw,
}

impl FreeCamInputDevice {
    const fn next(self) -> Self {
        match self {
            Self::KeyboardMouse => Self::Gamepad,
            Self::Gamepad => Self::GamepadSouthpaw,
            Self::GamepadSouthpaw => Self::KeyboardMouse,
        }
    }

    const fn is_gamepad(self) -> bool { matches!(self, Self::Gamepad | Self::GamepadSouthpaw) }

    fn preset(self) -> FreeCamPreset {
        match self {
            Self::KeyboardMouse => FreeCamPreset::from(
                FreeCamKeyboardMousePreset::default()
                    .mouse_input_gain(KEYBOARD_MOUSE_INPUT_GAIN)
                    .with_home(KeyCode::KeyH)
                    .with_look_pitch(FreeCamLookPitch::Inverted),
            ),
            Self::Gamepad => FreeCamPreset::from(
                FreeCamGamepadPreset::default()
                    .gamepad_input_gain(GAMEPAD_INPUT_GAIN)
                    .with_home(KeyCode::KeyH)
                    .with_home(GamepadButton::Select)
                    .with_look_pitch(FreeCamLookPitch::Inverted),
            ),
            Self::GamepadSouthpaw => FreeCamPreset::from(
                FreeCamGamepadPreset::default()
                    .with_layout(FreeCamGamepadLayout::Southpaw)
                    .gamepad_input_gain(GAMEPAD_INPUT_GAIN)
                    .with_home(KeyCode::KeyH)
                    .with_home(GamepadButton::Select)
                    .with_look_pitch(FreeCamLookPitch::Inverted),
            ),
        }
    }
}

/// Whether a gamepad is currently connected. Drives the greyed-out state of the
/// `G Cycle Input` control: with no gamepad there is no preset to cycle to.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GamepadAvailability {
    #[default]
    Absent,
    Present,
}

impl GamepadAvailability {
    const fn cycle_input_activation(self) -> ControlActivation {
        match self {
            Self::Absent => ControlActivation::Disabled,
            Self::Present => ControlActivation::Inactive,
        }
    }
}

/// Mirrors gamepad connection into [`GamepadAvailability`] so the `G Cycle Input`
/// control greys out while no gamepad is present.
fn track_gamepad_availability(
    mut availability: ResMut<GamepadAvailability>,
    gamepads: Query<(), With<Gamepad>>,
) {
    let next = if gamepads.is_empty() {
        GamepadAvailability::Absent
    } else {
        GamepadAvailability::Present
    };
    if *availability != next {
        *availability = next;
    }
}

/// Cycles the routed `FreeCam` through the keyboard/mouse and gamepad input presets. The gamepad
/// presets are reachable only while a gamepad is connected; without one, cycling stays on the
/// keyboard/mouse preset. Swapping [`FreeCamInputMode`] re-installs the bindings and the control
/// panel re-reads the new controls.
fn cycle_free_cam_input_device(
    mut commands: Commands,
    mut device: ResMut<FreeCamInputDevice>,
    gamepads: Query<(), With<Gamepad>>,
    cameras: Query<Entity, With<FreeCam>>,
) {
    let next = if gamepads.is_empty() {
        FreeCamInputDevice::KeyboardMouse
    } else {
        device.next()
    };
    if next == *device {
        return;
    }
    *device = next;
    let Ok(camera) = cameras.single() else {
        return;
    };
    commands
        .entity(camera)
        .insert(FreeCamInputMode::with_preset(next.preset()));
}

/// Returns the `FreeCam` to the keyboard/mouse preset when a gamepad preset is active but no
/// gamepad is connected, so the control panel never shows gamepad controls that cannot be driven.
fn revert_free_cam_to_keyboard_mouse(
    mut commands: Commands,
    mut device: ResMut<FreeCamInputDevice>,
    gamepads: Query<(), With<Gamepad>>,
    cameras: Query<Entity, With<FreeCam>>,
) {
    if !gamepads.is_empty() || !device.is_gamepad() {
        return;
    }
    *device = FreeCamInputDevice::KeyboardMouse;
    let Ok(camera) = cameras.single() else {
        return;
    };
    commands
        .entity(camera)
        .insert(FreeCamInputMode::with_preset(device.preset()));
}

fn spawn_camera(mut commands: Commands) {
    // Pose and preset constructors do not compose into a single call; combine
    // them as a bundle tuple — `from_pose` seeds the start pose, and
    // `FreeCamInputMode::with_preset` selects the tuned input mode.
    commands.spawn((
        Name::new(CAMERA_NAME),
        FreeCam::from_pose(CAMERA_POSITION, CAMERA_LOOK, CAMERA_ROLL),
        FreeCamHomePose {
            position: Position(CAMERA_POSITION),
            look:     CAMERA_LOOK,
            roll:     CAMERA_ROLL,
        },
        FreeCamInputMode::with_preset(
            FreeCamKeyboardMousePreset::default()
                .mouse_input_gain(KEYBOARD_MOUSE_INPUT_GAIN)
                .with_home(KeyCode::KeyH)
                .with_look_pitch(FreeCamLookPitch::Inverted),
        ),
    ));
}

fn spawn_grid(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let root = commands
        .spawn((
            Name::new("FreeCam Grid"),
            Transform::default(),
            Visibility::default(),
        ))
        .id();

    let ground_mesh = meshes.add(Plane3d::default().mesh().size(GROUND_SIDE, GROUND_SIDE));
    let cube_mesh = meshes.add(Cuboid::from_size(Vec3::splat(CUBE_SIZE)));
    let ground_material = materials.add(StandardMaterial {
        base_color: GROUND_COLOR,
        cull_mode: None,
        ..default()
    });
    let cube_material = materials.add(StandardMaterial::from(CUBE_COLOR));

    let mut x = GRID_START;
    for _ in 0..GRID_SIDE_COUNT {
        let mut y = GRID_START;
        for _ in 0..GRID_SIDE_COUNT {
            let mut z = GRID_START;
            for _ in 0..GRID_SIDE_COUNT {
                let ground_position = Vec3::new(x, y, z);
                commands.spawn((
                    Name::new("FreeCam Ground Cell"),
                    Mesh3d(ground_mesh.clone()),
                    MeshMaterial3d(ground_material.clone()),
                    Transform::from_translation(ground_position),
                    ChildOf(root),
                ));
                commands.spawn((
                    Name::new("FreeCam Box Cell"),
                    Mesh3d(cube_mesh.clone()),
                    MeshMaterial3d(cube_material.clone()),
                    Transform::from_translation(ground_position + Vec3::Y * CUBE_CENTER_Y),
                    ChildOf(root),
                ));
                z += GRID_SPACING;
            }
            y += GRID_SPACING;
        }
        x += GRID_SPACING;
    }
}

fn select_free_flight(
    preset: ResMut<FreeCamExamplePreset>,
    pitch_limit: Res<PitchLimit>,
    camera_query: Query<&mut FreeCam>,
) {
    switch_free_cam_preset(
        FreeCamExamplePreset::FreeFlight,
        preset,
        pitch_limit,
        camera_query,
    );
}

fn select_pitch_limited(
    preset: ResMut<FreeCamExamplePreset>,
    pitch_limit: Res<PitchLimit>,
    camera_query: Query<&mut FreeCam>,
) {
    switch_free_cam_preset(
        FreeCamExamplePreset::PitchLimited,
        preset,
        pitch_limit,
        camera_query,
    );
}

fn select_horizon_locked(
    preset: ResMut<FreeCamExamplePreset>,
    pitch_limit: Res<PitchLimit>,
    camera_query: Query<&mut FreeCam>,
) {
    switch_free_cam_preset(
        FreeCamExamplePreset::HorizonLocked,
        preset,
        pitch_limit,
        camera_query,
    );
}

fn switch_free_cam_preset(
    next_preset: FreeCamExamplePreset,
    mut preset: ResMut<FreeCamExamplePreset>,
    pitch_limit: Res<PitchLimit>,
    mut camera_query: Query<&mut FreeCam>,
) {
    if *preset == next_preset {
        return;
    }

    let Ok(mut camera) = camera_query.single_mut() else {
        return;
    };

    next_preset.apply_to(&mut camera, pitch_limit.radians());
    *preset = next_preset;
}

fn update_pitch_limit(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    preset: Res<FreeCamExamplePreset>,
    mut pitch_limit: ResMut<PitchLimit>,
    mut adjustment: ResMut<PitchLimitAdjustment>,
    mut camera_query: Query<&mut FreeCam>,
    mut title_bars: Query<&mut TitleBar>,
) {
    let next_adjustment = PitchLimitAdjustment::from_keyboard(&keys);
    if *adjustment != next_adjustment {
        *adjustment = next_adjustment;
    }

    let previous_limit = pitch_limit.radians();
    pitch_limit.adjust(
        next_adjustment.direction() * PITCH_LIMIT_ADJUST_RADIANS_PER_SECOND * time.delta_secs(),
    );
    if (pitch_limit.radians() - previous_limit).abs() <= f32::EPSILON {
        return;
    }

    for mut title_bar in &mut title_bars {
        *title_bar = free_cam_title_bar(pitch_limit.radians());
    }

    match *preset {
        FreeCamExamplePreset::FreeFlight => {},
        FreeCamExamplePreset::PitchLimited | FreeCamExamplePreset::HorizonLocked => {
            let Ok(mut camera) = camera_query.single_mut() else {
                return;
            };
            constrain_look(&mut camera, pitch_limit_constraint(pitch_limit.radians()));
            camera.force_update();
        },
    }
}

fn constrain_look(camera: &mut FreeCam, limit: AnglePairLimit) {
    *camera.look.limit_mut() = limit;
    camera
        .look
        .set_current(limit.constrain(camera.look.current()));
    camera
        .look
        .set_target(limit.constrain(camera.look.target()));
}

fn constrain_roll(camera: &mut FreeCam, limit: ScalarLimit) {
    *camera.roll.limit_mut() = limit;
    camera
        .roll
        .set_current(limit.constrain(camera.roll.current()));
    camera
        .roll
        .set_target(limit.constrain(camera.roll.target()));
}

const fn pitch_limit_constraint(limit: f32) -> AnglePairLimit {
    AnglePairLimit {
        yaw:   ScalarLimit::None,
        pitch: ScalarLimit::Clamp {
            min: -limit,
            max: limit,
        },
    }
}

fn free_cam_title_bar(pitch_limit: f32) -> TitleBar {
    TitleBar::new()
        .with_title(EXAMPLE_TITLE)
        .with_anchor(Anchor::TopLeft)
        .with_orientation(TitleBarOrientation::Vertical)
        .control(CAMERA_HOME_CONTROL)
        .control(
            TitleBarControl::from(CYCLE_INPUT_CONTROL)
                .with_disabled_note(CYCLE_INPUT_NO_GAMEPAD_NOTE),
        )
        .control(TitleBarControl::segmented(
            "1",
            [TitleBarSegment::new(FREE_FLIGHT_CONTROL, "Free Flight")],
        ))
        .control(TitleBarControl::segmented(
            "2",
            [
                TitleBarSegment::new(
                    PITCH_LIMITED_CONTROL,
                    format!("Pitch Limit ±{:.1}°", pitch_limit.to_degrees()),
                ),
                TitleBarSegment::new(DECREASE_PITCH_LIMIT_CONTROL, "-"),
                TitleBarSegment::new(INCREASE_PITCH_LIMIT_CONTROL, "+"),
            ],
        ))
        .control(TitleBarControl::segmented(
            "3",
            [TitleBarSegment::new(
                HORIZON_LOCKED_CONTROL,
                format!("Horizon Lock ±{:.1}° + Roll 0°", pitch_limit.to_degrees()),
            )],
        ))
}
