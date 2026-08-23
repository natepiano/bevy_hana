//! Demonstrates three distinct ways to drive an `OrbitCam`, explained by the
//! three boxes in the lower-left panel:
//!
//! - **Manual (M)** writes `OrbitCam` fields every frame for a continuous orbit loop — input
//!   disabled, smoothing zeroed.
//! - **`PlayAnimation` (P)** atomically requests a retained sequence. Each move eases over its own
//!   duration, then traversal advances to the next.
//! - **`AnimateToFit` (A)** declaratively frames a target entity — here the cube wearing its
//!   "`AnimateToFit` target" name on each face.
//!
//! Two cubes sit as a centered pair on the ground: the home cube and the
//! `AnimateToFit` target. Both are `CameraHomeTarget`s, so the startup home fit
//! frames their union and Lagrange stores that pose. `H` returns to the stored
//! pose; `A` animates in to frame the `AnimateToFit` target alone.
//!
//! `AnimateToFit` fits a target while home uses Lagrange's stored-pose glide.
//! The `A AnimateToFit` chip is wired with `fairy_dust`'s
//! `wire_chip_to_fit_target`, which matches on the framed `target` entity.
//!
//! Controls:
//!   M - Toggle manual orbit animation on/off
//!   P - `PlayAnimation` 5-step sequence
//!   A - `AnimateToFit` the labeled target cube
//!   H - Return to the camera home pose
//!   D - Replace the retained journey by inserting `CameraSequence`
//!   F/B/U/R/X/N/V - Native `SequenceCommand`s: play, backward, pause, resume,
//!       cancel, step, and step backward
//!   C/Y/T/L - Claim with authored/composed easing, make a replacing-easing
//!       takeover, then release the takeover and restore its displaced producer

use std::f32::consts::TAU;
use std::time::Duration;

use bevy::color::LinearRgba;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::math::curve::easing::EaseFunction;
use bevy::prelude::AlphaMode;
use bevy::prelude::Assets;
use bevy::prelude::ButtonInput;
use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Name;
use bevy::prelude::On;
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
use bevy::prelude::info;
use bevy::prelude::warn;
use fairy_dust::Anchor;
use fairy_dust::CameraHomeTarget;
use fairy_dust::ControlActivation;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::EXAMPLE_CUBE_SIZE;
use fairy_dust::Face;
use fairy_dust::LABEL_SIZE;
use fairy_dust::TITLE_SIZE;
use fairy_dust::TitleBar;
use fairy_dust::cube_face_transform;
use fairy_dust::example_cube_on_ground;
use hana_diegetic::AlignX;
use hana_diegetic::AlignY;
use hana_diegetic::Border;
use hana_diegetic::CornerRadius;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::GlyphShadowMode;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Padding;
use hana_diegetic::PanelBuildError;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::TextAlign;
use hana_diegetic::TextStyle;
use hana_diegetic::Unit;
use hana_diegetic::default_panel_material;
use hana_kana::SequencePlaybackSystems;
use hana_lagrange::AnimateToFit;
use hana_lagrange::AnimationBegin;
use hana_lagrange::AnimationEnd;
use hana_lagrange::AnimationSource;
use hana_lagrange::CameraCommands;
use hana_lagrange::CameraHomeKind;
use hana_lagrange::CameraInputDisabled;
use hana_lagrange::CameraMove;
use hana_lagrange::CameraPlaybackObservation;
use hana_lagrange::CameraSequence;
use hana_lagrange::Focus;
use hana_lagrange::FreeCamRollTarget;
use hana_lagrange::OrbitAngles;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamHomePose;
use hana_lagrange::OrbitCamKind;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::PlayAnimation;
use hana_lagrange::Radius;
use hana_lagrange::SequenceCommand;
use hana_lagrange::SequenceDirection;
use hana_lagrange::SequenceDriver;
use hana_lagrange::SequenceDriverTakeover;
use hana_lagrange::SequenceEasing;
use hana_lagrange::SequenceEvaluation;
use hana_lagrange::SequenceMovement;
use hana_lagrange::SequenceOwner;
use hana_lagrange::SequencePosition;
use hana_lagrange::SequenceSourceState;

const EXAMPLE_TITLE: &str = "Animation";

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        .with_studio_lighting()
        .key_light_illuminance(KEY_LIGHT_ILLUMINANCE)
        .with_ground_plane()
        .with_orbit_cam_preset(|_| {}, OrbitCamPreset::blender_like())
        .with_camera_home()
        .yaw(HOME_YAW)
        .pitch(HOME_PITCH)
        .margin(HOME_MARGIN)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft)
                .control(MANUAL_CONTROL)
                .control(PLAY_CONTROL)
                .control(FIT_CONTROL),
        )
        .wire_chip_to_state::<ManualAnimationState, _>(MANUAL_CONTROL, |state| {
            state.mode.control_activation()
        })
        .wire_chip_to_events_filtered::<AnimationBegin, AnimationEnd, _, _>(
            PLAY_CONTROL,
            |event| event.source == AnimationSource::PlayAnimation,
            |event| event.source == AnimationSource::PlayAnimation,
        )
        .wire_chip_to_fit_target::<FitTarget>(FIT_CONTROL)
        .with_camera_control_panel()
        .init_resource::<ManualAnimationState>()
        .add_systems(
            Startup,
            (spawn_target, spawn_fit_target, spawn_explainer_panel),
        )
        // M / A / P run through Fairy Dust's shortcut binding, which fires each
        // only when no modifier is held — so the `Ctrl+Shift+A` home-gizmo chord
        // no longer also triggers the bare-`A` AnimateToFit. H is read in
        // `manual_animate` only while manual mode has disabled the camera input
        // context; otherwise Fairy Dust's filled preset handles home.
        .with_shortcut(KeyCode::KeyM, toggle_manual)
        .with_shortcut(KeyCode::KeyA, animate_to_fit)
        .with_shortcut(KeyCode::KeyP, play_animation)
        .with_shortcut(KeyCode::KeyD, replace_retained_sequence)
        .with_shortcut(KeyCode::KeyF, play_forward)
        .with_shortcut(KeyCode::KeyB, play_backward)
        .with_shortcut(KeyCode::KeyU, pause_sequence)
        .with_shortcut(KeyCode::KeyR, resume_sequence)
        .with_shortcut(KeyCode::KeyX, cancel_sequence)
        .with_shortcut(KeyCode::KeyN, step_forward)
        .with_shortcut(KeyCode::KeyV, step_backward)
        .with_shortcut(KeyCode::KeyC, claim_with_authored_easing)
        .with_shortcut(KeyCode::KeyY, claim_with_composed_easing)
        .with_shortcut(KeyCode::KeyT, take_over_with_replacing_easing)
        .with_shortcut(KeyCode::KeyL, release_takeover)
        .add_observer(stop_manual_on_animation_begin)
        .add_systems(Update, manual_animate)
        .add_systems(
            Update,
            produce_example_sequence_movement.in_set(SequencePlaybackSystems::ProduceMovement),
        )
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// CAMERA DRIVERS — three ways to drive one OrbitCam: manual writes, PlayAnimation,
// AnimateToFit, and retained sequence commands or producers.
// ═════════════════════════════════════════════════════════════════════════════
//
// How it works: `main` wires one HUD chip per mechanism and binds M / A / P
// through Fairy Dust's shortcut API. `manual_animate` is the only per-frame
// `Update` system; the rest are one-shot handlers and an observer:
//   - `M` (`toggle_manual`) flips `ManualAnimationState`. While active, `manual_animate` writes
//     `OrbitCam` targets every frame with camera input disabled and smoothing zeroed, so the writes
//     apply with no easing lag. While manual mode is active, H calls `OrbitCamKind::apply_home`
//     because `CameraInputDisabled` gates the filled preset binding.
//   - `P` (`play_animation`) triggers `PlayAnimation` with `CameraMove` values built from
//     `PLAY_ANIMATION_STEPS`; retained traversal evaluates one move at a time.
//   - `A` (`animate_to_fit`) triggers `AnimateToFit` on the `FitTarget` cube.
//   - `stop_manual_on_animation_begin` observes `AnimationBegin` and leaves manual mode whenever A,
//     P, or another animation starts, so manual writes never fight it.
//   - `D` replaces the retained definition directly; F/B/U/R/X/N/V exercise every `SequenceCommand`
//     through `CameraCommands`.
//   - `C` first claims ownership with authored easing; a second press is an ordinary rejected
//     claim. `Y` makes the same ordinary claim with composed easing. `T` explicitly takes over with
//     replacing easing, and `L` releases it so the displaced producer is restored. Every producer
//     keeps its driver and movement together on its own entity.

// HUD chip strings
const FIT_CONTROL: &str = "A AnimateToFit";
const MANUAL_CONTROL: &str = "M Manual Orbit";
const PLAY_CONTROL: &str = "P Play Animation";

// manual orbit
const MANUAL_MODE_SMOOTHNESS_ACTIVE: f32 = 0.0;
const MANUAL_MODE_SMOOTHNESS_INACTIVE: f32 = 0.8;
const MANUAL_ORBIT_PITCH_AMPLITUDE: f32 = TAU * 0.1;
const MANUAL_ORBIT_RADIUS_BASE: f32 = 4.0;
const MANUAL_ORBIT_RADIUS_DELTA: f32 = 2.0;
const MANUAL_ORBIT_RADIUS_FREQUENCY: f32 = 2.0;
const MANUAL_ORBIT_YAW_RADIANS_PER_SECOND: f32 = TAU / 24.0;

// animate-to-fit
const ANIMATE_TO_FIT_DURATION: Duration = Duration::from_millis(1200);
const ANIMATE_TO_FIT_MARGIN: f32 = 0.15;
const ANIMATE_TO_FIT_PITCH: f32 = TAU / 12.0;
const ANIMATE_TO_FIT_YAW: f32 = TAU / 8.0;

// camera home
const HOME_MARGIN: f32 = 0.5;
const HOME_PITCH: f32 = ANIMATE_TO_FIT_PITCH;
const HOME_YAW: f32 = ANIMATE_TO_FIT_YAW;

/// One step of the `P` sequence; `play_animation` maps each to an orbital `CameraMove`.
#[derive(Clone, Copy)]
struct OrbitAnimationStep {
    duration: Duration,
    easing:   EaseFunction,
    pitch:    f32,
    radius:   f32,
    yaw:      f32,
}

const PLAY_ANIMATION_FOCUS: Vec3 =
    Vec3::new(0.0, example_cube_on_ground(CUBE_GROUND_CLEARANCE).y, 0.0);
const PLAY_ANIMATION_STEPS: [OrbitAnimationStep; 5] = [
    OrbitAnimationStep {
        duration: Duration::from_millis(800),
        easing:   EaseFunction::CubicInOut,
        pitch:    0.2,
        radius:   4.0,
        yaw:      1.5,
    },
    OrbitAnimationStep {
        duration: Duration::from_millis(1200),
        easing:   EaseFunction::CubicIn,
        pitch:    1.3,
        radius:   20.0,
        yaw:      2.5,
    },
    OrbitAnimationStep {
        duration: Duration::from_millis(1200),
        easing:   EaseFunction::SineInOut,
        pitch:    0.6,
        radius:   14.0,
        yaw:      4.5,
    },
    OrbitAnimationStep {
        duration: Duration::from_secs(1),
        easing:   EaseFunction::CubicIn,
        pitch:    0.1,
        radius:   2.0,
        yaw:      5.5,
    },
    OrbitAnimationStep {
        duration: Duration::from_millis(1200),
        easing:   EaseFunction::BounceOut,
        pitch:    0.3,
        radius:   8.0,
        yaw:      0.0,
    },
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ManualAnimationMode {
    #[default]
    Inactive,
    Active,
}

impl ManualAnimationMode {
    const fn control_activation(self) -> ControlActivation {
        match self {
            Self::Inactive => ControlActivation::Inactive,
            Self::Active => ControlActivation::Active,
        }
    }
}

#[derive(Resource, Default)]
struct ManualAnimationState {
    mode: ManualAnimationMode,
}

/// Marker for the cube the `A` key's `AnimateToFit` frames. `wire_chip_to_fit_target`
/// keys the `A AnimateToFit` chip on fits whose `target` carries this marker.
#[derive(Component)]
struct FitTarget;

/// Marks the producer whose explicit takeover `L` releases.
#[derive(Component)]
struct ReplacingEasingTakeover;

/// The example-owned absolute source state sampled by one sequence producer.
///
/// Each producer advances independently from the existing Bevy clock. Reaching
/// a retained endpoint publishes the arrival direction, then reverses on the
/// next sample; the producer never wraps or crosses outside its sequence.
#[derive(Component)]
struct ExampleSequenceProducerState {
    position:  SequencePosition,
    direction: SequenceDirection,
}

impl Default for ExampleSequenceProducerState {
    fn default() -> Self {
        Self {
            position:  SequencePosition::START,
            direction: SequenceDirection::Forward,
        }
    }
}

impl From<CameraPlaybackObservation> for ExampleSequenceProducerState {
    fn from(observation: CameraPlaybackObservation) -> Self {
        match observation {
            CameraPlaybackObservation::NoRetainedSequence => Self::default(),
            CameraPlaybackObservation::Retained { position, .. } => Self {
                position,
                ..Self::default()
            },
        }
    }
}

impl ExampleSequenceProducerState {
    fn sample(&mut self, delta_seconds: f32) -> (SequencePosition, SequenceDirection) {
        let direction = self.direction;
        let progress_delta = if delta_seconds.is_finite() {
            (delta_seconds * EXAMPLE_PRODUCER_PROGRESS_PER_SECOND).max(0.0)
        } else {
            0.0
        };
        let next = match direction {
            SequenceDirection::Forward => self.position.normalized() + progress_delta,
            SequenceDirection::Backward => self.position.normalized() - progress_delta,
        }
        .clamp(
            SequencePosition::START.normalized(),
            SequencePosition::END.normalized(),
        );
        self.position = SequencePosition::try_new(next).unwrap_or(self.position);
        if self.position == SequencePosition::END && direction == SequenceDirection::Forward {
            self.direction = SequenceDirection::Backward;
        } else if self.position == SequencePosition::START
            && direction == SequenceDirection::Backward
        {
            self.direction = SequenceDirection::Forward;
        }
        (self.position, direction)
    }
}

const EXAMPLE_PRODUCER_PROGRESS_PER_SECOND: f32 = 0.2;

/// Samples each producer's own source, publishes its complete movement, and
/// records that its source is current for a possible takeover restoration.
fn produce_example_sequence_movement(
    time: Res<Time>,
    mut producers: Query<(
        &mut ExampleSequenceProducerState,
        &mut SequenceMovement,
        &mut SequenceSourceState,
    )>,
) {
    for (mut producer_state, mut movement, mut source_state) in &mut producers {
        let (position, direction) = producer_state.sample(time.delta_secs());
        let Ok(sequence_movement) =
            SequenceMovement::try_new(position, direction, 0, hana_kana::RangeCrossings::NONE)
        else {
            continue;
        };
        *movement = sequence_movement;
        source_state.mark_current();
    }
}

/// `M` — toggles manual orbit. Turning it on disables camera input and zeroes
/// smoothing so `manual_animate`'s per-frame writes apply with no easing lag;
/// turning it off restores both via `stop_manual`.
fn toggle_manual(
    mut commands: Commands,
    mut manual: ResMut<ManualAnimationState>,
    mut orbit_cam_query: Query<(Entity, &mut OrbitCam)>,
) {
    let Ok((camera, mut orbit_cam)) = orbit_cam_query.single_mut() else {
        return;
    };

    if manual.mode == ManualAnimationMode::Active {
        stop_manual(&mut commands, &mut manual, camera, &mut orbit_cam);
    } else {
        manual.mode = ManualAnimationMode::Active;
        commands.entity(camera).insert(CameraInputDisabled);
        orbit_cam.orbit.set_damping(MANUAL_MODE_SMOOTHNESS_ACTIVE);
        orbit_cam.zoom.set_damping(MANUAL_MODE_SMOOTHNESS_ACTIVE);
        orbit_cam.pan.set_damping(MANUAL_MODE_SMOOTHNESS_ACTIVE);
        // Freeze the orbit target at the current angle so manual writes start
        // from where the camera is.
        let current = orbit_cam.orbit.current();
        orbit_cam.orbit.set_target(current);
    }
}

/// `A` — frames the `FitTarget` cube with `AnimateToFit`. Manual orbit, if
/// active, yields through `stop_manual_on_animation_begin`.
fn animate_to_fit(
    mut commands: Commands,
    camera_query: Query<Entity, With<OrbitCam>>,
    fit_target_query: Query<Entity, With<FitTarget>>,
) {
    let Ok(camera) = camera_query.single() else {
        return;
    };
    let Ok(fit_target) = fit_target_query.single() else {
        return;
    };
    commands.trigger(
        AnimateToFit::new(camera, fit_target)
            .yaw(ANIMATE_TO_FIT_YAW)
            .pitch(ANIMATE_TO_FIT_PITCH)
            .margin(ANIMATE_TO_FIT_MARGIN)
            .duration(ANIMATE_TO_FIT_DURATION),
    );
}

/// `P` — requests the five-step `PLAY_ANIMATION_STEPS` retained journey through
/// `PlayAnimation`. Manual orbit yields through `stop_manual_on_animation_begin`.
fn play_animation(mut commands: Commands, camera_query: Query<Entity, With<OrbitCam>>) {
    let Ok(camera) = camera_query.single() else {
        return;
    };
    let Ok(moves) = authored_play_moves() else {
        return;
    };

    commands.trigger(PlayAnimation::new(camera, moves));
}

/// Builds the `P` and `D` moves with fallible camera-move construction, so the
/// example keeps invalid orbital destinations at its authoring boundary.
fn authored_play_moves() -> Result<Vec<CameraMove>, hana_lagrange::CameraMoveError> {
    PLAY_ANIMATION_STEPS
        .iter()
        .map(|step| {
            CameraMove::try_to_orbital_look_at(
                Focus(PLAY_ANIMATION_FOCUS),
                OrbitAngles {
                    yaw:   step.yaw,
                    pitch: step.pitch,
                },
                Radius(step.radius),
                FreeCamRollTarget::InheritPrevious,
                step.duration,
                step.easing,
            )
        })
        .collect()
}

/// `D` — replaces retained authoring directly. `CameraSequence` preserves the
/// same single evaluator as `PlayAnimation`, while its lifecycle source is
/// `AnimationSource::CameraSequence`.
fn replace_retained_sequence(mut commands: Commands, camera_query: Query<Entity, With<OrbitCam>>) {
    let Ok(camera) = camera_query.single() else {
        return;
    };
    let Ok(moves) =
        authored_play_moves().inspect_err(|error| warn!("camera move rejected: {error}"))
    else {
        return;
    };
    let Ok(sequence) = CameraSequence::try_from_moves(moves)
        .inspect_err(|error| warn!("camera sequence rejected: {error}"))
    else {
        return;
    };

    commands.entity(camera).insert(sequence);
}

fn play_forward(camera_query: Query<Entity, With<OrbitCam>>, commands: CameraCommands) {
    apply_native_command(camera_query, commands, SequenceCommand::Play);
}

fn play_backward(camera_query: Query<Entity, With<OrbitCam>>, commands: CameraCommands) {
    apply_native_command(camera_query, commands, SequenceCommand::PlayBackward);
}

fn pause_sequence(camera_query: Query<Entity, With<OrbitCam>>, commands: CameraCommands) {
    apply_native_command(camera_query, commands, SequenceCommand::Pause);
}

fn resume_sequence(camera_query: Query<Entity, With<OrbitCam>>, commands: CameraCommands) {
    apply_native_command(camera_query, commands, SequenceCommand::Resume);
}

fn cancel_sequence(camera_query: Query<Entity, With<OrbitCam>>, commands: CameraCommands) {
    apply_native_command(camera_query, commands, SequenceCommand::Cancel);
}

fn step_forward(camera_query: Query<Entity, With<OrbitCam>>, commands: CameraCommands) {
    apply_native_command(camera_query, commands, SequenceCommand::Step);
}

fn step_backward(camera_query: Query<Entity, With<OrbitCam>>, commands: CameraCommands) {
    apply_native_command(camera_query, commands, SequenceCommand::StepBackward);
}

/// Applies a native command and logs whether retained playback was absent,
/// owned by a producer, or accepted by native playback.
fn apply_native_command(
    camera_query: Query<Entity, With<OrbitCam>>,
    mut commands: CameraCommands,
    command: SequenceCommand,
) {
    let Ok(camera) = camera_query.single() else {
        return;
    };

    let ownership = commands.owner(camera);
    let observation = commands.observe(camera);
    let response = commands.apply(camera, SequenceOwner::NativePlayback, command);
    info!(
        ?command,
        ?ownership,
        ?observation,
        ?response,
        "camera sequence command"
    );
}

/// `C` — claims retained movement with no external curve. Pressing it twice
/// illustrates that an ordinary second producer does not displace the selected
/// owner.
fn claim_with_authored_easing(
    mut commands: Commands,
    camera_query: Query<Entity, With<OrbitCam>>,
    camera_commands: CameraCommands,
) {
    let Ok(camera) = camera_query.single() else {
        return;
    };

    commands.spawn((
        SequenceDriver::new(camera),
        SequenceEvaluation::AUTHORED_WHOLE,
        SequenceMovement::default(),
        SequenceSourceState::default(),
        ExampleSequenceProducerState::from(camera_commands.observe(camera)),
    ));
}

/// `Y` — makes an ordinary claim with a producer curve composed with each
/// authored move's easing. When `C` already owns playback, arbitration rejects
/// this claim without changing the selected producer.
fn claim_with_composed_easing(
    mut commands: Commands,
    camera_query: Query<Entity, With<OrbitCam>>,
    camera_commands: CameraCommands,
) {
    let Ok(camera) = camera_query.single() else {
        return;
    };

    commands.spawn((
        SequenceDriver::new(camera),
        SequenceEvaluation::new(
            hana_lagrange::SequenceScope::WholeSequence,
            SequenceEasing::ComposedWith(EaseFunction::SineInOut.into()),
        ),
        SequenceMovement::default(),
        SequenceSourceState::default(),
        ExampleSequenceProducerState::from(camera_commands.observe(camera)),
    ));
}

/// `T` — explicitly replaces the selected driver with a producer whose curve
/// replaces authored easing. The marker lets `L` release this exact producer.
fn take_over_with_replacing_easing(
    mut commands: Commands,
    camera_query: Query<Entity, With<OrbitCam>>,
    camera_commands: CameraCommands,
) {
    let Ok(camera) = camera_query.single() else {
        return;
    };

    commands.spawn((
        ReplacingEasingTakeover,
        SequenceDriver::new(camera),
        SequenceDriverTakeover,
        SequenceEvaluation::new(
            hana_lagrange::SequenceScope::WholeSequence,
            SequenceEasing::ReplacedBy(EaseFunction::CubicInOut.into()),
        ),
        SequenceMovement::default(),
        SequenceSourceState::default(),
        ExampleSequenceProducerState::from(camera_commands.observe(camera)),
    ));
}

/// `L` — releases every example takeover. Arbitration restores each displaced
/// producer before native playback resumes.
fn release_takeover(
    mut commands: Commands,
    takeovers: Query<Entity, With<ReplacingEasingTakeover>>,
) {
    for takeover in &takeovers {
        commands.entity(takeover).despawn();
    }
}

/// Leaves manual orbit when any camera animation starts, so the per-frame
/// manual writes never fight a triggered animation. The manual-mode H path is
/// handled in `manual_animate` because stored-pose home has no `AnimationBegin`.
fn stop_manual_on_animation_begin(
    _begin: On<AnimationBegin>,
    mut commands: Commands,
    mut manual: ResMut<ManualAnimationState>,
    mut orbit_cam_query: Query<(Entity, &mut OrbitCam)>,
) {
    if manual.mode != ManualAnimationMode::Active {
        return;
    }
    let Ok((camera, mut orbit_cam)) = orbit_cam_query.single_mut() else {
        return;
    };
    stop_manual(&mut commands, &mut manual, camera, &mut orbit_cam);
}

/// Per-frame manual animation; only runs when the resource flag is active.
fn manual_animate(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut manual: ResMut<ManualAnimationState>,
    mut query: Query<(Entity, &mut OrbitCam, &OrbitCamHomePose)>,
) {
    if manual.mode != ManualAnimationMode::Active {
        return;
    }
    let home_pressed = keys.just_pressed(KeyCode::KeyH);
    for (camera, mut orbit_cam, home) in &mut query {
        if home_pressed {
            // No manual `CameraHomed` trigger here: `stop_manual`
            // re-enables camera input, so the preset home action sees the
            // still-held H next frame and fires the event itself. Triggering
            // here as well would announce the same press twice.
            stop_manual(&mut commands, &mut manual, camera, &mut orbit_cam);
            OrbitCamKind::apply_home(&mut orbit_cam, home);
            continue;
        }
        let mut angles = orbit_cam.orbit.target();
        angles.yaw = MANUAL_ORBIT_YAW_RADIANS_PER_SECOND.mul_add(time.delta_secs(), angles.yaw);
        angles.pitch = time.elapsed_secs_wrapped().sin() * MANUAL_ORBIT_PITCH_AMPLITUDE;
        orbit_cam.orbit.set_target(angles);
        orbit_cam.zoom.set_current(Radius(
            f32::midpoint(
                (time.elapsed_secs_wrapped() * MANUAL_ORBIT_RADIUS_FREQUENCY).cos(),
                1.0,
            )
            .mul_add(MANUAL_ORBIT_RADIUS_DELTA, MANUAL_ORBIT_RADIUS_BASE),
        ));
        orbit_cam.force_update();
    }
}

/// Leaves manual mode: re-enables camera input and restores smoothing so the
/// next triggered animation eases normally.
fn stop_manual(
    commands: &mut Commands,
    manual: &mut ManualAnimationState,
    camera: Entity,
    orbit_cam: &mut OrbitCam,
) {
    if manual.mode == ManualAnimationMode::Active {
        manual.mode = ManualAnimationMode::Inactive;
        commands.entity(camera).remove::<CameraInputDisabled>();
        orbit_cam.orbit.set_damping(MANUAL_MODE_SMOOTHNESS_INACTIVE);
        orbit_cam.zoom.set_damping(MANUAL_MODE_SMOOTHNESS_INACTIVE);
        orbit_cam.pan.set_damping(MANUAL_MODE_SMOOTHNESS_INACTIVE);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// TARGET CUBES — the two cubes the camera frames, each wearing its name on its
// faces in emissive text.
// ═════════════════════════════════════════════════════════════════════════════

/// Key light dimmed well below the studio default (`13_500` lux) so the emissive
/// face text reads against the lit cubes instead of being washed out.
const KEY_LIGHT_ILLUMINANCE: f32 = 2_500.0;

// Both cubes use the canonical `example_cube_on_ground` launch height and sit
// mirrored across the origin (`±CUBE_OFFSET_X`) so the pair reads as a centered
// group on the ground plane with a gap between them. Both are `CameraHomeTarget`s,
// so the startup fit stores a home pose for their union; H returns to that pose,
// while A frames only the AnimateToFit target.
const CUBE_SIZE: f32 = EXAMPLE_CUBE_SIZE;
const CUBE_OFFSET_X: f32 = 1.5;
/// Lift off the ground plane, matching the other examples' canonical clearance.
const CUBE_GROUND_CLEARANCE: f32 = 0.1;
// Each cube wears its name on its faces via a transparent cube-face
// `DiegeticPanel` whose `text_material` is strongly emissive — the same
// example-level recipe as `focus_bounds`/`follow_target`, so no hana_diegetic
// change is needed. Panel font sizes are in millimeters (the cube is 1 m).
const FACE_LABEL_PANEL_SIZE: f32 = CUBE_SIZE * 0.88;
const FACE_LABEL_TEXT_SIZE: f32 = 88.0;
const FACE_LABEL_PADDING: f32 = 0.06;
/// Over-bright base color of the face text; the emissive material multiplies
/// it. Pushed past 1.0 so it clamps to a full, punchy white on an SDR camera.
const FACE_LABEL_COLOR: Color = Color::linear_rgb(2.0, 2.0, 2.2);
/// How hard the face text glows — the emissive color is the base times this.
const FACE_LABEL_EMISSIVE_BOOST: f32 = 6.0;

const TARGET_COLOR: Color = Color::srgb(0.8, 0.7, 0.6);
const TARGET_LABEL: &str = "Just a cube";
const TARGET_TRANSLATION: Vec3 = Vec3::new(
    -CUBE_OFFSET_X,
    example_cube_on_ground(CUBE_GROUND_CLEARANCE).y,
    0.0,
);

const FIT_TARGET_COLOR: Color = Color::srgb(0.45, 0.62, 0.85);
const FIT_TARGET_LABEL: &str = "AnimateToFit target";
const FIT_TARGET_TRANSLATION: Vec3 = Vec3::new(
    CUBE_OFFSET_X,
    example_cube_on_ground(CUBE_GROUND_CLEARANCE).y,
    0.0,
);

fn spawn_target(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let face_material = materials.add(transparent_face_material());
    let face_text_material = materials.add(emissive_text_material());
    commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::from_length(CUBE_SIZE))),
            MeshMaterial3d(materials.add(TARGET_COLOR)),
            Transform::from_translation(TARGET_TRANSLATION),
            CameraHomeTarget,
        ))
        .with_children(|parent| {
            spawn_face_label_panels(
                parent,
                TARGET_LABEL,
                face_material.clone(),
                face_text_material.clone(),
            );
        });
}

/// Spawns the cube that the `A` key's `AnimateToFit` frames — a `FitTarget`-marked
/// entity wearing its name on each face, like the `input_*` examples. It is also a
/// `CameraHomeTarget`, so `H` frames it together with the home cube; `A` frames it
/// alone. The `target` entity on the animation events is what keeps the `A` and `H`
/// chips from co-lighting, even though both fits share the `AnimateToFit` source.
fn spawn_fit_target(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let face_material = materials.add(transparent_face_material());
    let face_text_material = materials.add(emissive_text_material());
    commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::from_length(CUBE_SIZE))),
            MeshMaterial3d(materials.add(FIT_TARGET_COLOR)),
            Transform::from_translation(FIT_TARGET_TRANSLATION),
            FitTarget,
            CameraHomeTarget,
        ))
        .with_children(|parent| {
            spawn_face_label_panels(
                parent,
                FIT_TARGET_LABEL,
                face_material.clone(),
                face_text_material.clone(),
            );
        });
}

/// Spawns one transparent cube-face [`DiegeticPanel`] per side face, each
/// centered on the cube and carrying `label` in strongly-emissive text. The
/// emissive lives in the panel's `text_material` (a `StandardMaterial` whose
/// `emissive` is the boosted [`FACE_LABEL_COLOR`]), the same example-level
/// recipe as `focus_bounds`/`follow_target`.
fn spawn_face_label_panels(
    parent: &mut ChildSpawnerCommands,
    label: &'static str,
    face_material: Handle<StandardMaterial>,
    face_text_material: Handle<StandardMaterial>,
) {
    for face in [Face::Front, Face::Back, Face::Left, Face::Right] {
        match face_label_panel(label, face_material.clone(), face_text_material.clone()) {
            Ok(panel) => {
                parent.spawn((panel, cube_face_transform(face, CUBE_SIZE)));
            },
            Err(error) => {
                error!("animation: failed to build cube face label panel: {error}");
            },
        }
    }
}

fn face_label_panel(
    label: &str,
    face_material: Handle<StandardMaterial>,
    face_text_material: Handle<StandardMaterial>,
) -> Result<DiegeticPanel, PanelBuildError> {
    DiegeticPanel::world()
        .size(FACE_LABEL_PANEL_SIZE, FACE_LABEL_PANEL_SIZE)
        .font_unit(Unit::Millimeters)
        .anchor(Anchor::Center)
        .material(face_material)
        .text_material(face_text_material)
        .with_tree(face_label_tree(label))
        .build()
}

/// Transparent, unlit panel background — only the emissive text shows.
fn transparent_face_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::NONE,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default_panel_material()
    }
}

/// Strongly-emissive text material: [`FACE_LABEL_COLOR`] multiplied by
/// [`FACE_LABEL_EMISSIVE_BOOST`], so the glyphs read as self-lit.
fn emissive_text_material() -> StandardMaterial {
    let mut emissive: LinearRgba = FACE_LABEL_COLOR.into();
    emissive.red *= FACE_LABEL_EMISSIVE_BOOST;
    emissive.green *= FACE_LABEL_EMISSIVE_BOOST;
    emissive.blue *= FACE_LABEL_EMISSIVE_BOOST;
    StandardMaterial {
        base_color: Color::NONE,
        emissive,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default_panel_material()
    }
}

fn face_label_tree(label: &str) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(
        El::column()
            .width(Sizing::fixed(FACE_LABEL_PANEL_SIZE))
            .height(Sizing::fixed(FACE_LABEL_PANEL_SIZE))
            .alignment(AlignX::Center, AlignY::Center)
            .padding(Padding::all(FACE_LABEL_PADDING))
            .clip(),
    );
    builder.text((
        label,
        TextStyle::new(FACE_LABEL_TEXT_SIZE)
            .with_color(FACE_LABEL_COLOR)
            .with_align(TextAlign::Center)
            .with_shadow_mode(GlyphShadowMode::None),
    ));
    builder.build()
}

// ═════════════════════════════════════════════════════════════════════════════
// EXPLAINER PANEL — a lower-left stack of one bordered box per mechanism, in the
// `aa_text` style. Static copy, spawned once; nothing refreshes it.
// ═════════════════════════════════════════════════════════════════════════════

const EXPLAINER_BOX_WIDTH: Px = Px(264.0);
const EXPLAINER_DIVIDER_HEIGHT: Px = Px(1.0);
const EXPLAINER_PADDING: Px = Px(10.0);
const EXPLAINER_RADIUS: Px = Px(10.0);
const EXPLAINER_BORDER_WIDTH: Px = Px(1.0);
const EXPLAINER_ROW_GAP: Px = Px(4.0);
const EXPLAINER_STACK_GAP: Px = Px(8.0);
const EXPLAINER_PANEL_NAME: &str = "Animation explainer panel";
const EXPLAINER_HEADER_COLOR: Color = Color::srgb(0.95, 0.95, 0.97);
const EXPLAINER_BODY_COLOR: Color = Color::srgba(0.68, 0.72, 0.82, 0.9);
const EXPLAINER_BORDER_COLOR: Color = Color::srgba(0.15, 0.7, 0.9, 0.4);

/// One mechanism's box: a heading over wrapped body lines.
struct Explainer {
    title: &'static str,
    lines: &'static [&'static str],
}

const EXPLAINERS: [Explainer; 3] = [
    Explainer {
        title: "Manual Orbit",
        lines: &[
            "Writes the OrbitCam orbit and zoom operations directly each frame for a continuous orbit.",
            "Camera input is disabled and smoothing zeroed so the writes apply with no easing lag.",
        ],
    },
    Explainer {
        title: "Play Animation",
        lines: &[
            "Requests a retained CameraMove journey through PlayAnimation.",
            "Each move eases over its own duration, then traversal advances.",
        ],
    },
    Explainer {
        title: "AnimateToFit",
        lines: &[
            "Trigger AnimateToFit with a target entity.",
            "The camera eases to fit it to the screen with a provided margin.",
        ],
    },
];

/// Spawns the lower-left explainer panel: a transparent, unlit screen panel
/// stacking one bordered box per [`Explainer`].
fn spawn_explainer_panel(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let unlit = materials.add(StandardMaterial {
        unlit: true,
        ..default_panel_material()
    });
    let panel = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_explainer_tree())
        .build();

    match panel {
        Ok(panel) => {
            commands.spawn((Name::new(EXPLAINER_PANEL_NAME), panel, Transform::default()));
        },
        Err(error) => {
            error!("animation: failed to build explainer panel: {error}");
        },
    }
}

fn build_explainer_tree() -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    let title = TextStyle::new(TITLE_SIZE).with_color(EXPLAINER_HEADER_COLOR);
    // Wrapped body text flows to the fixed box width.
    let body = TextStyle::new(LABEL_SIZE).with_color(EXPLAINER_BODY_COLOR);
    builder.with(
        El::column()
            .width(Sizing::fixed(EXPLAINER_BOX_WIDTH))
            .height(Sizing::FIT)
            .gap(EXPLAINER_STACK_GAP),
        |builder| {
            for explainer in &EXPLAINERS {
                build_explainer_box(builder, explainer, &title, &body);
            }
        },
    );
    builder.build()
}

fn build_explainer_box(
    builder: &mut LayoutBuilder,
    explainer: &Explainer,
    title: &TextStyle,
    body: &TextStyle,
) {
    builder.with(
        El::column()
            .width(Sizing::GROW)
            .height(Sizing::FIT)
            .gap(EXPLAINER_ROW_GAP)
            .padding(Padding::all(EXPLAINER_PADDING))
            .corner_radius(CornerRadius::all(EXPLAINER_RADIUS))
            .background(DEFAULT_PANEL_BACKGROUND)
            .border(Border::all(EXPLAINER_BORDER_WIDTH, EXPLAINER_BORDER_COLOR)),
        |builder| {
            builder.text((explainer.title, title.clone()));
            explainer_divider(builder);
            for line in explainer.lines {
                builder.text((*line, body.clone()));
            }
        },
    );
}

/// A horizontal hairline rule spanning the box width, drawn under each heading.
fn explainer_divider(builder: &mut LayoutBuilder) {
    builder.with(
        El::new()
            .width(Sizing::GROW)
            .height(Sizing::fixed(EXPLAINER_DIVIDER_HEIGHT))
            .background(EXPLAINER_BORDER_COLOR),
        |_| {},
    );
}
