//! Capability: a generalized "home" pose for Lagrange cameras.
//!
//! Maintains an invisible fit cube for every [`CameraHomeTarget`] entity, snaps
//! the camera to that cube on startup, and refits it after window resizes while
//! the camera is still home. When the capability is installed, empty Lagrange
//! presets are filled with Fairy Dust's home inputs: `H` for keyboard-family
//! presets, and both `H` and Select for gamepad presets. Those inputs invoke Lagrange's
//! stored-pose home glide through `bevy_enhanced_input`; Fairy Dust does not
//! bind a separate home action.
//!
//! If a title bar is installed, the `H Home` control chip is prepended
//! automatically unless the home builder opts out.
//!
//! Add [`CameraHomeTarget`] to any entity to frame that entity (and its
//! descendants). [`hana_lagrange::AnimateToFit`] extracts every descendant
//! mesh's vertices, so tagging a `WorldText` parent frames all its glyph
//! children without the caller measuring anything. Startup waits for the
//! target's meshes to exist, then snaps to it. If nothing carries the marker,
//! the home camera warns once and waits.

use std::time::Duration;

use bevy::camera::primitives::Aabb;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::Gizmos;
use bevy::prelude::GlobalTransform;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Local;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MessageReader;
use bevy::prelude::On;
use bevy::prelude::Or;
use bevy::prelude::PreUpdate;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectEvent;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Startup;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::warn;
use bevy::window::WindowResized;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use hana_diegetic::Anchor;
use hana_lagrange::AnimateToFit;
use hana_lagrange::AnimationBegin;
use hana_lagrange::AnimationEnd;
use hana_lagrange::AnimationReason;
use hana_lagrange::AnimationSource;
use hana_lagrange::CameraHomePending;
use hana_lagrange::CameraHomed;
use hana_lagrange::CameraInputPhase;
use hana_lagrange::FitAnchor;
use hana_lagrange::FreeCam;
use hana_lagrange::FreeCamInputMode;
use hana_lagrange::FreeCamInteractionStarted;
use hana_lagrange::FreeCamPreset;
use hana_lagrange::FreeCamPresetKind;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamInteractionStarted;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::OrbitCamPresetKind;
use hana_rubric::ReflectKeymapCommand;
use hana_rubric::command;

use crate::constants::AABB_CORNER_SIGNS;
use crate::constants::HOME_AABB_GIZMO_COLOR;
use crate::constants::HOME_BUTTON;
use crate::constants::HOME_CONTROL;
use crate::constants::HOME_KEY;
use crate::constants::MIN_HOME_CUBE_SCALE;
use crate::ensure_plugin;
use crate::orbit_cam::FairyDustOrbitCam;
use crate::restart_camera::RestartCameraRestore;
use crate::restart_camera::RestoreWindowAnimation;
use crate::screen_panels::ControlActivation;
use crate::screen_panels::HomeTitleBarFlash;
use crate::screen_panels::TitleBarControlState;
use crate::shortcuts;

type HomeFitCameraFilter = Or<(With<FairyDustOrbitCam>, With<FreeCam>)>;

command! {
    action:      ToggleHomeAabbGizmo,
    event:       ToggleHomeAabbGizmoEvent,
    id:          "fairy_dust::toggle_home_aabb_gizmo",
    title:       "Toggle Home Bounds Gizmo",
    description: "Draw a wireframe of the region the camera home framing fits.",
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InitialAnimateState {
    #[default]
    Pending,
    Fired,
}

#[derive(Component)]
pub(crate) struct CameraHomeMarker;

/// Frames this entity and its descendants as the camera home.
///
/// Add it to any entity whose [`Aabb`] should contribute to the home region.
/// `H` animates to the target union, window resizes refit it, and startup snaps
/// to it once its meshes exist. With no marked entity the capability warns once
/// and waits. When more than one entity carries the marker, Fairy Dust frames
/// their union.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CameraHomeTarget;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HomeTitleBarControl {
    #[default]
    Shown,
    Hidden,
}

/// Stashed home configuration. Read by the title-bar installer to decide
/// whether to prepend the `H Home` chip.
#[derive(Resource, Clone)]
pub(crate) struct CameraHomeConfig {
    pub yaw:               f32,
    pub pitch:             f32,
    pub margin:            f32,
    pub anchor:            Anchor,
    pub offset_px:         Vec2,
    pub title_bar_control: HomeTitleBarControl,
}

/// Resource holding the entity used as the invisible home cube.
///
/// The cube is an internal fit proxy: each frame its [`Transform`] is rewritten
/// to the union of every [`CameraHomeTarget`] entity.
#[derive(Resource)]
pub struct CameraHomeEntity(pub Entity);

/// Tracks whether the camera is still at the home pose. The window-resize
/// refit only fires when this is `Yes`, so user-driven pan/zoom/orbit isn't
/// undone by a resize.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AtHome {
    #[default]
    Yes,
    No,
}

/// Tracks whether [`draw_home_aabb_gizmo`] is currently drawing a wireframe
/// of the home cube. Toggled by **Ctrl+Shift+A** — undocumented debug
/// affordance available in every `fairy_dust`-built example, no setup needed.
/// Defaults to off.
#[derive(Resource, Default)]
struct HomeAabbGizmoVisible(bool);

/// Flips [`HomeAabbGizmoVisible`] on Ctrl+Shift+A. The gizmo combo is a chord,
/// not a single key, so it doesn't collide with bare-`A` bindings the caller
/// may have.
fn toggle_home_aabb_gizmo(
    _: On<ToggleHomeAabbGizmoEvent>,
    mut visible: ResMut<HomeAabbGizmoVisible>,
) {
    visible.0 = !visible.0;
}

/// Draws a wireframe of the home cube — sized to the union of every
/// [`CameraHomeTarget`] entity — while [`HomeAabbGizmoVisible`] is on. Lets
/// you see what region the camera is actually framing.
fn draw_home_aabb_gizmo(
    visible: Res<HomeAabbGizmoVisible>,
    home: Option<Res<CameraHomeEntity>>,
    cube: Query<&Transform, With<CameraHomeMarker>>,
    mut gizmos: Gizmos,
) {
    if !visible.0 {
        return;
    }
    let Some(home) = home else {
        return;
    };
    let Ok(transform) = cube.get(home.0) else {
        return;
    };
    gizmos.cube(*transform, HOME_AABB_GIZMO_COLOR);
}

pub(crate) fn install(app: &mut App, config: CameraHomeConfig) {
    ensure_plugin(app, EnhancedInputPlugin);
    app.insert_resource(config);
    app.init_resource::<AtHome>();
    app.init_resource::<HomeAabbGizmoVisible>();
    shortcuts::reserve_key::<CameraHomeConfig>(app, HOME_KEY, HOME_CONTROL);
    app.add_systems(Startup, spawn_home_marker);
    app.add_systems(
        PreUpdate,
        fill_camera_home_presets.before(CameraInputPhase::PreInput),
    );
    // `update_home_cube` runs first so the cube reflects the current union of
    // `CameraHomeTarget` entities before any handler triggers `AnimateToFit`
    // off it.
    app.add_systems(
        Update,
        (
            update_home_cube,
            snap_home_on_ready,
            refit_on_window_resized,
        )
            .chain(),
    );
    // Ctrl+Shift+A debug toggle — always available, opt-in by keypress.
    app.add_systems(Update, draw_home_aabb_gizmo);
    app.add_observer(toggle_home_aabb_gizmo);
    app.add_observer(on_home_animation_begin);
    app.add_observer(on_home_animation_end);
    app.add_observer(on_non_home_animation_begin);
    app.add_observer(on_camera_homed);
    app.add_observer(on_orbit_user_interaction_started);
    app.add_observer(on_free_user_interaction_started);
}

pub(crate) fn fill_camera_home_presets(
    config: Option<Res<CameraHomeConfig>>,
    mut orbit_modes: Query<&mut OrbitCamInputMode, Changed<OrbitCamInputMode>>,
    mut free_modes: Query<&mut FreeCamInputMode, Changed<FreeCamInputMode>>,
) {
    if config.is_none() {
        return;
    }
    for mut mode in &mut orbit_modes {
        if let Some(filled_mode) = fill_orbit_cam_home(&mode) {
            *mode = filled_mode;
        }
    }
    for mut mode in &mut free_modes {
        if let Some(filled_mode) = fill_free_cam_home(&mode) {
            *mode = filled_mode;
        }
    }
}

fn fill_orbit_cam_home(mode: &OrbitCamInputMode) -> Option<OrbitCamInputMode> {
    let OrbitCamInputMode::Preset(preset) = mode else {
        return None;
    };
    (!preset.has_home())
        .then(|| OrbitCamInputMode::with_preset(orbit_cam_home_preset(preset.clone())))
}

fn orbit_cam_home_preset(preset: OrbitCamPreset) -> OrbitCamPreset {
    match preset.kind() {
        OrbitCamPresetKind::Gamepad => preset.home(HOME_KEY).home(HOME_BUTTON),
        _ => preset.home(HOME_KEY),
    }
}

fn fill_free_cam_home(mode: &FreeCamInputMode) -> Option<FreeCamInputMode> {
    let FreeCamInputMode::Preset(preset) = mode else {
        return None;
    };
    (!preset.has_home())
        .then(|| FreeCamInputMode::with_preset(free_cam_home_preset(preset.clone())))
}

fn free_cam_home_preset(preset: FreeCamPreset) -> FreeCamPreset {
    match preset.kind() {
        FreeCamPresetKind::Gamepad => preset.with_home(HOME_KEY).with_home(HOME_BUTTON),
        _ => preset.with_home(HOME_KEY),
    }
}

/// World-space union of every [`CameraHomeTarget`] entity and its
/// descendants' [`Aabb`]s. Returns `(center, size)` of the union box, or
/// `None` when no marked entity (or descendant) has an [`Aabb`] yet.
fn world_aabb_union(
    targets: &Query<Entity, With<CameraHomeTarget>>,
    children: &Query<&Children>,
    aabbs: &Query<&Aabb>,
    transforms: &Query<&GlobalTransform, Without<CameraHomeMarker>>,
) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    for root in targets {
        for entity in std::iter::once(root).chain(children.iter_descendants(root)) {
            let (Ok(aabb), Ok(global_transform)) = (aabbs.get(entity), transforms.get(entity))
            else {
                continue;
            };
            let local_center = Vec3::from(aabb.center);
            let half = Vec3::from(aabb.half_extents);
            for sign in AABB_CORNER_SIGNS {
                let world = global_transform.transform_point(local_center + sign * half);
                min = min.min(world);
                max = max.max(world);
                found = true;
            }
        }
    }
    if !found {
        return None;
    }
    Some(((min + max) * 0.5, max - min))
}

/// Each frame, rewrites the hidden home cube's [`Transform`] (and its
/// [`GlobalTransform`], so same-tick observers see it) to the union of every
/// [`CameraHomeTarget`] entity. The cube is what every fit handler frames, so
/// updating it here is the only place the framed region changes.
fn update_home_cube(
    home: Option<Res<CameraHomeEntity>>,
    targets: Query<Entity, With<CameraHomeTarget>>,
    children: Query<&Children>,
    aabbs: Query<&Aabb>,
    target_transforms: Query<&GlobalTransform, Without<CameraHomeMarker>>,
    mut cube: Query<(&mut Transform, &mut GlobalTransform), With<CameraHomeMarker>>,
    mut warned_no_target: Local<bool>,
) {
    let Some(home) = home else {
        return;
    };
    let Ok((mut cube_transform, mut cube_global)) = cube.get_mut(home.0) else {
        return;
    };
    if targets.is_empty() {
        if !*warned_no_target {
            warn!("fairy_dust camera home has no CameraHomeTarget; home camera will wait");
            *warned_no_target = true;
        }
        return;
    }
    let Some((center, size)) = world_aabb_union(&targets, &children, &aabbs, &target_transforms)
    else {
        return;
    };
    let new_transform =
        Transform::from_translation(center).with_scale(size.max(Vec3::splat(MIN_HOME_CUBE_SCALE)));
    if *cube_transform == new_transform {
        return;
    }
    *cube_transform = new_transform;
    // Write `GlobalTransform` too — `AnimateToFit` observers run after this
    // system in the same tick and read `GlobalTransform`. `PostUpdate`'s
    // transform propagation will recompute it from `Transform` afterwards
    // (same value, no change).
    *cube_global = GlobalTransform::from(new_transform);
}

fn spawn_home_marker(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let mesh = meshes.add(Cuboid::from_size(Vec3::ONE));
    let entity = commands
        .spawn((
            CameraHomeMarker,
            Mesh3d(mesh),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    commands.insert_resource(CameraHomeEntity(entity));
}

const fn fit_anchor(anchor: Anchor) -> FitAnchor {
    match anchor {
        Anchor::TopLeft => FitAnchor::TopLeft,
        Anchor::TopCenter => FitAnchor::TopCenter,
        Anchor::TopRight => FitAnchor::TopRight,
        Anchor::CenterLeft => FitAnchor::CenterLeft,
        Anchor::Center => FitAnchor::Center,
        Anchor::CenterRight => FitAnchor::CenterRight,
        Anchor::BottomLeft => FitAnchor::BottomLeft,
        Anchor::BottomCenter => FitAnchor::BottomCenter,
        Anchor::BottomRight => FitAnchor::BottomRight,
    }
}

const fn home_fit(
    camera: Entity,
    home: Entity,
    config: &CameraHomeConfig,
    duration: Duration,
) -> AnimateToFit {
    AnimateToFit::new(camera, home)
        .yaw(config.yaw)
        .pitch(config.pitch)
        .margin(config.margin)
        .anchor(fit_anchor(config.anchor))
        .offset_px(config.offset_px)
        .duration(duration)
}

/// Whether the snap can fire: with marked entities present, wait until at
/// least one (or one of its descendants) has an [`Aabb`] so the union the
/// cube was sized to is meaningful.
fn target_meshes_ready(
    targets: &Query<Entity, With<CameraHomeTarget>>,
    children: &Query<&Children>,
    aabbs: &Query<&Aabb>,
) -> bool {
    targets.iter().any(|target| {
        std::iter::once(target)
            .chain(children.iter_descendants(target))
            .any(|entity| aabbs.contains(entity))
    })
}

/// Snaps the camera to the home target once its meshes exist, exactly once. A
/// saved restart pose wins — it restores the prior window pose instead of the
/// snap. A [`CameraHomeTarget`] waits for its glyphs/meshes.
fn snap_home_on_ready(
    mut commands: Commands,
    home: Option<Res<CameraHomeEntity>>,
    config: Res<CameraHomeConfig>,
    cameras: Query<Entity, HomeFitCameraFilter>,
    targets: Query<Entity, With<CameraHomeTarget>>,
    children: Query<&Children>,
    aabbs: Query<&Aabb>,
    restore: Option<Res<RestartCameraRestore>>,
    mut state: Local<InitialAnimateState>,
) {
    if *state == InitialAnimateState::Fired {
        return;
    }
    let Some(home) = home else {
        return;
    };
    let Ok(camera) = cameras.single() else {
        return;
    };
    if !target_meshes_ready(&targets, &children, &aabbs) {
        return;
    }
    if restore
        .as_deref()
        .is_some_and(RestartCameraRestore::has_restart_camera_pose)
    {
        commands.trigger(RestoreWindowAnimation);
        *state = InitialAnimateState::Fired;
        return;
    }
    commands.trigger(home_fit(camera, home.0, &config, Duration::ZERO));
    *state = InitialAnimateState::Fired;
}

fn refit_on_window_resized(
    mut events: MessageReader<WindowResized>,
    mut commands: Commands,
    home: Option<Res<CameraHomeEntity>>,
    config: Res<CameraHomeConfig>,
    cameras: Query<Entity, HomeFitCameraFilter>,
    at_home: Res<AtHome>,
    targets: Query<Entity, With<CameraHomeTarget>>,
    children: Query<&Children>,
    aabbs: Query<&Aabb>,
) {
    if events.is_empty() {
        return;
    }
    events.clear();
    if *at_home != AtHome::Yes {
        return;
    }
    let Some(home) = home else {
        return;
    };
    let Ok(camera) = cameras.single() else {
        return;
    };
    if !target_meshes_ready(&targets, &children, &aabbs) {
        return;
    }
    commands.entity(camera).insert(CameraHomePending);
    commands.trigger(home_fit(camera, home.0, &config, Duration::ZERO));
}

/// Whether an animation lifecycle event is the home fit.
///
/// The home fit is an `AnimateToFit` that frames the home cube. A user-issued
/// `AnimateToFit` aimed at some other entity shares the `AnimateToFit` source
/// but carries a different `target`, so it is not the home fit and must not
/// drive the `H Home` chip.
fn frames_home(
    home: Option<&CameraHomeEntity>,
    source: AnimationSource,
    target: Option<Entity>,
) -> bool {
    home.is_some_and(|home| source == AnimationSource::AnimateToFit && target == Some(home.0))
}

fn on_home_animation_begin(
    trigger: On<AnimationBegin>,
    home: Option<Res<CameraHomeEntity>>,
    mut bars: Query<&mut TitleBarControlState>,
    flash: Option<ResMut<HomeTitleBarFlash>>,
) {
    if !frames_home(home.as_deref(), trigger.source, trigger.target) {
        return;
    }
    cancel_home_title_bar_flash(flash);
    for mut bar in &mut bars {
        bar.set_active(HOME_CONTROL, ControlActivation::Active);
    }
}

fn on_home_animation_end(
    trigger: On<AnimationEnd>,
    home: Option<Res<CameraHomeEntity>>,
    mut bars: Query<&mut TitleBarControlState>,
    mut at_home: ResMut<AtHome>,
) {
    if !frames_home(home.as_deref(), trigger.source, trigger.target) {
        return;
    }
    for mut bar in &mut bars {
        bar.set_active(HOME_CONTROL, ControlActivation::Inactive);
    }
    if matches!(trigger.reason, AnimationReason::Completed) {
        *at_home = AtHome::Yes;
    }
}

fn cancel_home_title_bar_flash(flash: Option<ResMut<HomeTitleBarFlash>>) {
    let Some(mut flash) = flash else {
        return;
    };
    flash.cancel();
}

fn on_non_home_animation_begin(
    trigger: On<AnimationBegin>,
    home: Option<Res<CameraHomeEntity>>,
    mut at_home: ResMut<AtHome>,
) {
    if !frames_home(home.as_deref(), trigger.source, trigger.target) {
        *at_home = AtHome::No;
    }
}

fn pulse_home_title_bar(
    flash: Option<ResMut<HomeTitleBarFlash>>,
    bars: &mut Query<&mut TitleBarControlState>,
) {
    for mut bar in &mut *bars {
        bar.set_active(HOME_CONTROL, ControlActivation::Active);
    }
    let Some(mut flash) = flash else {
        return;
    };
    flash.start();
}

fn on_camera_homed(
    _: On<CameraHomed>,
    mut at_home: ResMut<AtHome>,
    flash: Option<ResMut<HomeTitleBarFlash>>,
    mut bars: Query<&mut TitleBarControlState>,
) {
    *at_home = AtHome::Yes;
    pulse_home_title_bar(flash, &mut bars);
}

fn on_orbit_user_interaction_started(
    trigger: On<OrbitCamInteractionStarted>,
    mut at_home: ResMut<AtHome>,
) {
    if trigger.sources.is_empty() {
        return;
    }
    *at_home = AtHome::No;
}

fn on_free_user_interaction_started(
    trigger: On<FreeCamInteractionStarted>,
    mut at_home: ResMut<AtHome>,
) {
    if trigger.sources.is_empty() {
        return;
    }
    *at_home = AtHome::No;
}

#[cfg(test)]
mod tests {
    use bevy::camera::RenderTarget;
    use bevy::input::mouse::AccumulatedMouseMotion;
    use bevy::input::mouse::AccumulatedMouseScroll;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Camera;
    use bevy::prelude::GamepadButton;
    use bevy::prelude::KeyCode;
    use bevy::prelude::Messages;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::MouseButton;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::Projection;
    use bevy::window::WindowRef;
    use bevy_enhanced_input::prelude::Binding;
    use hana_lagrange::CameraBasis;
    use hana_lagrange::CameraInputRoutingConfig;
    use hana_lagrange::FreeCamHomePose;
    use hana_lagrange::InteractionSources;
    use hana_lagrange::LagrangePlugin;
    use hana_lagrange::LookAngles;
    use hana_lagrange::OrbitCam;
    use hana_lagrange::OrbitCamInteractionKind;
    use hana_lagrange::Position;
    use hana_lagrange::Roll;

    use super::*;

    const CAMERA_POSITION: Vec3 = Vec3::new(0.0, 0.0, 8.0);
    const HOME_TARGET_MAX: Vec3 = Vec3::new(0.5, 0.5, 0.5);
    const HOME_TARGET_MIN: Vec3 = Vec3::new(-0.5, -0.5, -0.5);
    const RESIZED_HEIGHT: f32 = 768.0;
    const RESIZED_WIDTH: f32 = 1024.0;
    const STALE_HOME_PITCH: f32 = 0.125;
    const STALE_HOME_POSITION: Vec3 = Vec3::new(8.0, 7.0, 6.0);
    const STALE_HOME_ROLL: f32 = 0.25;
    const STALE_HOME_YAW: f32 = 0.5;

    type TestResult = Result<(), &'static str>;

    #[derive(Clone, Copy)]
    enum ConfigResource {
        Present,
        Missing,
    }

    fn test_config() -> CameraHomeConfig {
        CameraHomeConfig {
            yaw:               0.0,
            pitch:             0.0,
            margin:            0.0,
            anchor:            Anchor::Center,
            offset_px:         Vec2::ZERO,
            title_bar_control: HomeTitleBarControl::Hidden,
        }
    }

    fn test_app(config: ConfigResource) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LagrangePlugin))
            .add_systems(
                PreUpdate,
                fill_camera_home_presets.before(CameraInputPhase::PreInput),
            );
        if matches!(config, ConfigResource::Present) {
            app.insert_resource(test_config());
        }
        app.finish();
        app
    }

    fn refit_test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, LagrangePlugin))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .add_message::<WindowResized>()
            .insert_resource(test_config())
            .insert_resource(AtHome::Yes)
            .add_systems(Update, (update_home_cube, refit_on_window_resized).chain());
        app.finish();
        app
    }

    fn spawn_orbit_mode(app: &mut App, mode: OrbitCamInputMode) -> Entity {
        app.world_mut()
            .spawn((
                OrbitCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                mode,
            ))
            .id()
    }

    fn spawn_free_mode(app: &mut App, mode: FreeCamInputMode) -> Entity {
        app.world_mut()
            .spawn((
                FreeCam::default(),
                Camera::default(),
                RenderTarget::Window(WindowRef::Primary),
                mode,
            ))
            .id()
    }

    fn stale_free_home_pose() -> FreeCamHomePose {
        FreeCamHomePose {
            position: Position(STALE_HOME_POSITION),
            look:     LookAngles {
                yaw:   STALE_HOME_YAW,
                pitch: STALE_HOME_PITCH,
            },
            roll:     Roll(STALE_HOME_ROLL),
        }
    }

    fn spawn_home_cube(app: &mut App) -> Entity {
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::from_size(Vec3::ONE));
        let entity = app
            .world_mut()
            .spawn((
                CameraHomeMarker,
                Mesh3d(mesh),
                Transform::default(),
                GlobalTransform::default(),
                Visibility::Hidden,
            ))
            .id();
        app.world_mut().insert_resource(CameraHomeEntity(entity));
        entity
    }

    fn spawn_home_target(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                CameraHomeTarget,
                Aabb::from_min_max(HOME_TARGET_MIN, HOME_TARGET_MAX),
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id()
    }

    fn spawn_free_fit_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                FreeCam::from_pose(CAMERA_POSITION, (0.0, 0.0), 0.0),
                Projection::Perspective(PerspectiveProjection::default()),
                Camera::default(),
                CameraBasis::Y_UP,
                Transform::from_translation(CAMERA_POSITION),
                stale_free_home_pose(),
            ))
            .id()
    }

    fn free_home_pose(app: &App, camera: Entity) -> Result<FreeCamHomePose, &'static str> {
        app.world()
            .get::<FreeCamHomePose>(camera)
            .copied()
            .ok_or("camera missing FreeCamHomePose")
    }

    fn write_window_resized(app: &mut App) {
        let window = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<Messages<WindowResized>>()
            .write(WindowResized {
                window,
                width: RESIZED_WIDTH,
                height: RESIZED_HEIGHT,
            });
    }

    fn orbit_home_bindings(app: &App, camera: Entity) -> Result<Vec<Binding>, String> {
        let Some(mode) = app.world().get::<OrbitCamInputMode>(camera) else {
            return Err("missing OrbitCamInputMode".to_string());
        };
        let OrbitCamInputMode::Preset(preset) = mode else {
            return Err("expected OrbitCam preset mode".to_string());
        };
        preset
            .to_bindings()
            .map(|bindings| bindings.home().to_vec())
            .map_err(|error| error.to_string())
    }

    fn free_home_bindings(app: &App, camera: Entity) -> Result<Vec<Binding>, String> {
        let Some(mode) = app.world().get::<FreeCamInputMode>(camera) else {
            return Err("missing FreeCamInputMode".to_string());
        };
        let FreeCamInputMode::Preset(preset) = mode else {
            return Err("expected FreeCam preset mode".to_string());
        };
        preset
            .to_bindings()
            .map(|bindings| bindings.home().to_vec())
            .map_err(|error| error.to_string())
    }

    #[test]
    fn window_resize_refit_recaptures_free_cam_home_pose() -> TestResult {
        let mut app = refit_test_app();
        let camera = spawn_free_fit_camera(&mut app);
        spawn_home_cube(&mut app);
        spawn_home_target(&mut app);
        let stale_home = free_home_pose(&app, camera)?;

        write_window_resized(&mut app);
        app.update();

        let recaptured_home = free_home_pose(&app, camera)?;
        let free_cam = app
            .world()
            .get::<FreeCam>(camera)
            .ok_or("camera missing FreeCam")?;

        assert_ne!(recaptured_home, stale_home);
        assert_eq!(recaptured_home.position, free_cam.translate.current());
        assert_eq!(recaptured_home.look, free_cam.look.current());
        assert_eq!(recaptured_home.roll, free_cam.roll.current());
        assert!(app.world().get::<CameraHomePending>(camera).is_none());
        Ok(())
    }

    #[test]
    fn camera_home_config_fills_keyboard_and_gamepad_presets() -> Result<(), String> {
        let mut app = test_app(ConfigResource::Present);
        let orbit_keyboard = spawn_orbit_mode(
            &mut app,
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        let orbit_gamepad = spawn_orbit_mode(
            &mut app,
            OrbitCamInputMode::with_preset(OrbitCamPreset::gamepad()),
        );
        let free_keyboard = spawn_free_mode(
            &mut app,
            FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse()),
        );
        let free_gamepad = spawn_free_mode(
            &mut app,
            FreeCamInputMode::with_preset(FreeCamPreset::gamepad()),
        );
        app.insert_resource(CameraInputRoutingConfig::explicit(orbit_keyboard));

        app.update();

        assert_eq!(
            orbit_home_bindings(&app, orbit_keyboard)?,
            vec![Binding::from(HOME_KEY)]
        );
        assert_eq!(
            orbit_home_bindings(&app, orbit_gamepad)?,
            vec![Binding::from(HOME_KEY), Binding::from(HOME_BUTTON)]
        );
        assert_eq!(
            free_home_bindings(&app, free_keyboard)?,
            vec![Binding::from(HOME_KEY)]
        );
        assert_eq!(
            free_home_bindings(&app, free_gamepad)?,
            vec![Binding::from(HOME_KEY), Binding::from(HOME_BUTTON)]
        );
        Ok(())
    }

    #[test]
    fn presets_keep_empty_home_without_camera_home_config() -> Result<(), String> {
        let mut app = test_app(ConfigResource::Missing);
        let orbit = spawn_orbit_mode(
            &mut app,
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
        );
        let free = spawn_free_mode(
            &mut app,
            FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse()),
        );

        app.update();

        assert_eq!(orbit_home_bindings(&app, orbit)?, Vec::<Binding>::new());
        assert_eq!(free_home_bindings(&app, free)?, Vec::<Binding>::new());
        Ok(())
    }

    #[test]
    fn explicit_preset_home_bindings_are_preserved() -> Result<(), String> {
        let mut app = test_app(ConfigResource::Present);
        let orbit = spawn_orbit_mode(
            &mut app,
            OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse().home(KeyCode::KeyR)),
        );
        let free = spawn_free_mode(
            &mut app,
            FreeCamInputMode::with_preset(FreeCamPreset::gamepad().with_home(GamepadButton::North)),
        );

        app.update();

        assert_eq!(
            orbit_home_bindings(&app, orbit)?,
            vec![Binding::from(KeyCode::KeyR)]
        );
        assert_eq!(
            free_home_bindings(&app, free)?,
            vec![Binding::from(GamepadButton::North)]
        );
        Ok(())
    }

    #[test]
    fn lagrange_camera_homed_rearms_resize_refit() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<AtHome>()
            .add_observer(on_orbit_user_interaction_started)
            .add_observer(on_camera_homed);
        app.finish();
        let camera = app.world_mut().spawn_empty().id();

        app.world_mut().trigger(OrbitCamInteractionStarted {
            camera,
            kind: OrbitCamInteractionKind::Orbit,
            sources: InteractionSources::KEYBOARD,
        });
        assert_eq!(*app.world().resource::<AtHome>(), AtHome::No);

        app.world_mut().trigger(CameraHomed {
            camera,
            sources: InteractionSources::KEYBOARD,
        });
        assert_eq!(*app.world().resource::<AtHome>(), AtHome::Yes);
    }
}
