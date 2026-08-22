use bevy::prelude::OrthographicProjection;
use bevy::prelude::PerspectiveProjection;
use bevy::prelude::Projection;
use bevy::prelude::Timer;
use bevy::prelude::TimerMode;
use bevy::prelude::warn;
use fairy_dust::ControlActivation;
use fairy_dust::TitleChipActivation;

use super::*;

/// Mirrors whether the camera carries [`FitOverlay`], so the title bar's
/// `O Fit Overlay` chip stays highlighted while the overlay is shown.
#[derive(Resource, Default)]
pub(crate) struct FitOverlayActive(bool);

impl TitleChipActivation for FitOverlayActive {
    fn activation(&self) -> ControlActivation {
        if self.0 {
            ControlActivation::Active
        } else {
            ControlActivation::Inactive
        }
    }
}

pub(crate) fn toggle_debug_overlay(
    mut commands: Commands,
    scene: Res<SceneEntities>,
    visualization_query: Query<(), With<FitOverlay>>,
    mut active: ResMut<FitOverlayActive>,
) {
    let camera = scene.camera;
    if visualization_query.get(camera).is_ok() {
        commands.entity(camera).remove::<FitOverlay>();
        active.0 = false;
    } else {
        commands.entity(camera).insert(FitOverlay);
        active.0 = true;
    }
}

pub(crate) fn animate_camera(
    mut commands: Commands,
    scene: Res<SceneEntities>,
    easing: Res<ActiveEasing>,
    camera_query: Query<&OrbitCam>,
) {
    let camera = scene.camera;
    let Ok(orbit_camera) = camera_query.get(camera) else {
        return;
    };

    let easing_function = easing.0;
    let angles = orbit_camera.orbit.target();
    let yaw = angles.yaw;
    let pitch = angles.pitch;
    let radius = orbit_camera.zoom.target().0;
    let focus = orbit_camera.pan.target().0;

    let quarter_turns = [
        FIRST_ORBIT_MOVE_QUARTER_TURNS,
        SECOND_ORBIT_MOVE_QUARTER_TURNS,
        THIRD_ORBIT_MOVE_QUARTER_TURNS,
        FOURTH_ORBIT_MOVE_QUARTER_TURNS,
    ];
    let camera_moves = quarter_turns
        .iter()
        .map(|quarter_turns| {
            CameraMove::try_to_orbital_look_at(
                Focus(focus),
                OrbitAngles {
                    yaw: QUARTER_TURN_RADIANS.mul_add(*quarter_turns, yaw),
                    pitch,
                },
                Radius(radius),
                FreeCamRollTarget::InheritPrevious,
                Duration::from_millis(ORBIT_MOVE_DURATION_MILLIS),
                easing_function,
            )
        })
        .collect::<Result<Vec<_>, _>>();
    let Ok(camera_moves) = camera_moves.inspect_err(|error| warn!("camera move rejected: {error}"))
    else {
        return;
    };

    commands.trigger(PlayAnimation::new(camera, camera_moves));
}

pub(crate) fn randomize_easing(
    mut easing: ResMut<ActiveEasing>,
    time: Res<Time>,
    mut log: ResMut<event_log::EventLog>,
    mut flash: ResMut<EasingFlash>,
) {
    let index = (time.elapsed_secs() * SECONDS_TO_MILLIS).to_usize() % ALL_EASINGS.len();
    easing.0 = ALL_EASINGS[index];
    log.push(format!("Easing: {:#?}", easing.0));
    flash.flash_random();
}

pub(crate) fn reset_easing(
    mut easing: ResMut<ActiveEasing>,
    mut log: ResMut<event_log::EventLog>,
    mut flash: ResMut<EasingFlash>,
) {
    easing.0 = EaseFunction::CubicOut;
    log.push(EVENT_LOG_EASING_RESET.into());
    flash.flash_reset();
}

/// Briefly highlights the `R Random Easing` / `0 Reset Easing` title-bar chips after a
/// press. Each press arms a one-shot timer; [`tick_easing_flash`] clears it when
/// it elapses, and the title bar polls [`Self::random_active`] /
/// [`Self::reset_active`] to drive the chip highlight.
#[derive(Resource, Default)]
pub(crate) struct EasingFlash {
    random: Option<Timer>,
    reset:  Option<Timer>,
}

impl EasingFlash {
    fn flash_random(&mut self) {
        self.random = Some(Timer::from_seconds(EASING_FLASH_SECONDS, TimerMode::Once));
    }

    fn flash_reset(&mut self) {
        self.reset = Some(Timer::from_seconds(EASING_FLASH_SECONDS, TimerMode::Once));
    }

    pub(crate) const fn random_active(&self) -> bool { self.random.is_some() }

    pub(crate) const fn reset_active(&self) -> bool { self.reset.is_some() }
}

/// Ticks the easing-chip flash timers, clearing each when it elapses so the
/// title bar's `R Random Easing` / `0 Reset Easing` chips flip back to inactive.
pub(crate) fn tick_easing_flash(time: Res<Time>, mut flash: ResMut<EasingFlash>) {
    if flash.random.is_none() && flash.reset.is_none() {
        return;
    }
    let random_ended = flash
        .random
        .as_mut()
        .is_some_and(|timer| timer.tick(time.delta()).just_finished());
    if random_ended {
        flash.random = None;
    }
    let reset_ended = flash
        .reset
        .as_mut()
        .is_some_and(|timer| timer.tick(time.delta()).just_finished());
    if reset_ended {
        flash.reset = None;
    }
}

/// Tracks the one-frame-deferred scene re-fit after a projection change.
///
/// `OrbitCam` needs a frame to process the projection change (syncing radius ↔
/// orthographic scale) before the fit math is correct. [`set_perspective`] /
/// [`set_orthographic`] arm `Armed`; [`apply_projection_refit`] advances
/// `Armed → Pending` on its next run and triggers the fit on the run after, so
/// at least one full frame passes before the fit.
#[derive(Resource, Default, PartialEq, Eq)]
pub(crate) enum ProjectionRefit {
    #[default]
    Idle,
    Armed,
    Pending,
}

/// Which projection the camera currently uses, so the title bar can highlight
/// the matching `P Perspective` / `O Orthographic` row via
/// [`wire_chip_to_state`](fairy_dust::TitleBarBuilder::wire_chip_to_state).
#[derive(Resource, Clone, Copy, Default)]
pub(crate) enum ProjectionMode {
    #[default]
    Perspective,
    Orthographic,
}

/// Switches the camera to perspective projection, a no-op when already there.
pub(crate) fn set_perspective(
    scene: Res<SceneEntities>,
    camera_query: Query<(&mut Projection, &mut OrbitCam)>,
    log: ResMut<event_log::EventLog>,
    refit: ResMut<ProjectionRefit>,
    mode: ResMut<ProjectionMode>,
) {
    switch_projection(
        scene,
        camera_query,
        log,
        refit,
        mode,
        ProjectionMode::Perspective,
    );
}

/// Switches the camera to orthographic projection, a no-op when already there.
pub(crate) fn set_orthographic(
    scene: Res<SceneEntities>,
    camera_query: Query<(&mut Projection, &mut OrbitCam)>,
    log: ResMut<event_log::EventLog>,
    refit: ResMut<ProjectionRefit>,
    mode: ResMut<ProjectionMode>,
) {
    switch_projection(
        scene,
        camera_query,
        log,
        refit,
        mode,
        ProjectionMode::Orthographic,
    );
}

/// Applies `target` to the camera, logs the change, records the active mode for
/// the title-bar highlight, and arms a deferred scene re-fit. Returns early
/// when the camera already uses `target`, so re-pressing the active row does
/// nothing.
fn switch_projection(
    scene: Res<SceneEntities>,
    mut camera_query: Query<(&mut Projection, &mut OrbitCam)>,
    mut log: ResMut<event_log::EventLog>,
    mut refit: ResMut<ProjectionRefit>,
    mut mode: ResMut<ProjectionMode>,
    target: ProjectionMode,
) {
    let Ok((mut projection, mut orbit_camera)) = camera_query.get_mut(scene.camera) else {
        return;
    };
    let already_active = match target {
        ProjectionMode::Perspective => matches!(*projection, Projection::Perspective(_)),
        ProjectionMode::Orthographic => matches!(*projection, Projection::Orthographic(_)),
    };
    if already_active {
        return;
    }
    match target {
        ProjectionMode::Perspective => {
            *projection = Projection::Perspective(PerspectiveProjection::default());
            log.push(PROJECTION_LOG_PERSPECTIVE.into());
        },
        ProjectionMode::Orthographic => {
            *projection = Projection::from(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: ORTHOGRAPHIC_VIEWPORT_HEIGHT,
                },
                far: ORTHOGRAPHIC_FAR_PLANE,
                ..OrthographicProjection::default_3d()
            });
            log.push(PROJECTION_LOG_ORTHOGRAPHIC.into());
        },
    }
    *mode = target;
    orbit_camera.force_update();
    *refit = ProjectionRefit::Armed;
}

/// Applies the deferred scene re-fit one frame after a projection toggle, once
/// `OrbitCam` has synced radius ↔ orthographic scale.
pub(crate) fn apply_projection_refit(
    mut commands: Commands,
    scene: Res<SceneEntities>,
    active_easing: Res<ActiveEasing>,
    mut refit: ResMut<ProjectionRefit>,
) {
    match *refit {
        ProjectionRefit::Idle => {},
        ProjectionRefit::Armed => *refit = ProjectionRefit::Pending,
        ProjectionRefit::Pending => {
            *refit = ProjectionRefit::Idle;
            commands.trigger(
                AnimateToFit::new(scene.camera, scene.scene_bounds)
                    .yaw(CAMERA_START_YAW)
                    .pitch(CAMERA_START_PITCH)
                    .margin(ZOOM_MARGIN_SCENE)
                    .duration(Duration::from_millis(ANIMATE_FIT_DURATION_MILLIS))
                    .easing(active_easing.0),
            );
        },
    }
}

pub(crate) fn toggle_interrupt_behavior(
    mut commands: Commands,
    scene: Res<SceneEntities>,
    mut behavior_query: Query<&mut CameraInputInterruptBehavior>,
    mut display: ResMut<policy_panel::PolicyDisplay>,
    mut flash: ResMut<policy_panel::KeyFlash>,
    mut log: ResMut<event_log::EventLog>,
) {
    let new_behavior =
        behavior_query
            .get(scene.camera)
            .map_or(
                CameraInputInterruptBehavior::Ignore,
                |behavior| match *behavior {
                    CameraInputInterruptBehavior::Ignore => CameraInputInterruptBehavior::Cancel,
                    CameraInputInterruptBehavior::Cancel => CameraInputInterruptBehavior::Complete,
                    CameraInputInterruptBehavior::Complete => CameraInputInterruptBehavior::Ignore,
                },
            );

    if let Ok(mut behavior) = behavior_query.get_mut(scene.camera) {
        *behavior = new_behavior;
    } else {
        commands.entity(scene.camera).insert(new_behavior);
    }

    display.interrupt_behavior = new_behavior;
    flash.flash_interrupt();
    log.push(format!("CameraInputInterruptBehavior: {new_behavior:?}"));
}

pub(crate) fn toggle_animation_conflict_policy(
    mut commands: Commands,
    scene: Res<SceneEntities>,
    mut policy_query: Query<&mut AnimationConflictPolicy>,
    mut display: ResMut<policy_panel::PolicyDisplay>,
    mut flash: ResMut<policy_panel::KeyFlash>,
    mut log: ResMut<event_log::EventLog>,
) {
    let new_policy =
        policy_query
            .get(scene.camera)
            .map_or(AnimationConflictPolicy::FirstWins, |policy| match *policy {
                AnimationConflictPolicy::LastWins => AnimationConflictPolicy::FirstWins,
                AnimationConflictPolicy::FirstWins => AnimationConflictPolicy::LastWins,
            });

    if let Ok(mut policy) = policy_query.get_mut(scene.camera) {
        *policy = new_policy;
    } else {
        commands.entity(scene.camera).insert(new_policy);
    }

    display.conflict_policy = new_policy;
    flash.flash_conflict();
    log.push(format!("AnimationConflictPolicy: {new_policy:?}"));
}
