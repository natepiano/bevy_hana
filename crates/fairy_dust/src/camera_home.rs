//! Capability: a generalized "home" pose for Lagrange cameras.
//!
//! Maintains an invisible fit cube for every [`CameraHomeTarget`] entity, snaps
//! the camera to that cube on startup, and refits it when the available window
//! area changes while the camera is still home. When the capability is installed, empty Lagrange
//! presets are filled with Fairy Dust's home inputs: `H` for keyboard-family
//! presets, and both `H` and Select for gamepad presets. Those inputs invoke Lagrange's
//! home action through `bevy_enhanced_input`; Fairy Dust retargets the return
//! fit to the current viewport insets.
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

use std::f32::consts::FRAC_PI_2;
use std::time::Duration;

use bevy::camera::primitives::Aabb;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Changed;
use bevy::prelude::Children;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Event;
use bevy::prelude::Gizmos;
use bevy::prelude::GlobalTransform;
use bevy::prelude::InheritedVisibility;
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
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::World;
use bevy::prelude::warn;
use bevy::window::PrimaryWindow;
use bevy::window::WindowResized;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use bevy_enhanced_input::prelude::InputAction;
use hana_diegetic::Anchor;
use hana_diegetic::CoordinateSpace;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::PanelAnchorGeometryParam;
use hana_diegetic::PanelSystems;
use hana_diegetic::ScreenPosition;
use hana_lagrange::AnimateToFit;
use hana_lagrange::AnimationBegin;
use hana_lagrange::AnimationEnd;
use hana_lagrange::AnimationReason;
use hana_lagrange::AnimationSource;
use hana_lagrange::CameraHomePending;
use hana_lagrange::CameraHomed;
use hana_lagrange::CameraInputPhase;
use hana_lagrange::FitAnchor;
use hana_lagrange::FitViewportInsets;
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

use crate::camera_control_panel::CameraGuidancePanel;
use crate::constants::AABB_CORNER_SIGNS;
use crate::constants::HOME_AABB_GIZMO_COLOR;
use crate::constants::HOME_BUTTON;
use crate::constants::HOME_CONTROL;
use crate::constants::HOME_KEY;
use crate::constants::MIN_HOME_CUBE_SCALE;
use crate::ensure_plugin;
use crate::orbit_cam::FairyDustOrbitCam;
use crate::panel_insets;
use crate::panel_insets::CoveringPanel;
use crate::restart_camera::RestartCameraRestore;
use crate::restart_camera::RestoreWindowAnimation;
use crate::screen_panels::ControlActivation;
use crate::screen_panels::HomeTitleBarFlash;
use crate::screen_panels::SCREEN_PANEL_WINDOW_INSET;
use crate::screen_panels::TitleBarControlState;
use crate::screen_panels::separation;
use crate::screen_panels::separation::EdgePanelHomePlacement;
use crate::screen_panels::separation::EdgePanelManagement;
use crate::screen_panels::separation::EdgePanelPlacementRecord;
use crate::screen_panels::separation::PanelPlacement;
use crate::shortcuts;

type HomeFitCameraFilter = Or<(With<FairyDustOrbitCam>, With<FreeCam>)>;
const HOME_PANEL_GAP: f32 = 12.0;
const HOME_RETURN_DURATION: Duration = Duration::from_millis(350);

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
struct HomeViewportInsets(FitViewportInsets);

/// Insets reserved by panels other than the preset-driven camera guidance panel.
/// A change here may refit a camera already at Home.
#[derive(Resource, Clone, Copy, Default, PartialEq)]
struct OtherPanelHomeInsets(FitViewportInsets);

/// The horizontal cube face framed by the Home camera fit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HomeFace {
    /// The face toward positive Z, used by ordinary Home.
    #[default]
    Front,
    /// The face toward negative X.
    Left,
    /// The face toward positive X.
    Right,
}

/// Requests Home's fit, clearance, and animation for one cube face.
#[derive(Event, Clone, Copy, Debug)]
pub struct FrameHomeFace(pub HomeFace);

/// Frames another target with Home's orientation and current screen-panel clearance.
#[derive(Event, Clone, Copy, Debug)]
pub struct FrameCameraTarget {
    /// Entity whose bounds define the view.
    pub target:   Entity,
    /// Extra space around the target, as a fraction of the clear viewport.
    pub margin:   f32,
    /// Duration of the camera move.
    pub duration: Duration,
}

#[derive(Resource, Default)]
struct FramedHomeFace(HomeFace);

command! {
    action:      ToggleHomeAabbGizmo,
    event:       ToggleHomeAabbGizmoEvent,
    id:          "fairy_dust::toggle_home_aabb_gizmo",
    title:       "Toggle home bounds gizmo",
    description: "Draw a wireframe of the region the camera home framing fits.",
    toggle_state: home_bounds_gizmo_is_shown,
}

fn home_bounds_gizmo_is_shown(world: &World) -> bool {
    world
        .get_resource::<HomeAabbGizmoVisible>()
        .is_some_and(|visible| visible.0)
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

/// Home configuration recorded at install. The title-bar installer reads
/// `title_bar_control` from it and prepends the `H Home` chip when it is
/// [`HomeTitleBarControl::Shown`].
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

/// Tracks whether the camera is still at the home pose. The viewport
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
    app.init_resource::<HomeViewportInsets>();
    app.init_resource::<OtherPanelHomeInsets>();
    app.init_resource::<FramedHomeFace>();
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
            update_home_viewport_insets.after(PanelSystems::PositionScreenSpace),
            snap_home_on_ready,
            refit_home_on_viewport_change,
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
    app.add_observer(on_frame_home_face);
    app.add_observer(on_frame_camera_target);
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

fn update_home_viewport_insets(
    window: Query<&Window, With<PrimaryWindow>>,
    panels: Query<(
        Entity,
        &DiegeticPanel,
        &Visibility,
        &InheritedVisibility,
        Option<&EdgePanelPlacementRecord>,
        Option<&EdgePanelHomePlacement>,
        Option<&CameraGuidancePanel>,
    )>,
    geometry: PanelAnchorGeometryParam,
    mut home_insets: ResMut<HomeViewportInsets>,
    other_panel_insets: Option<ResMut<OtherPanelHomeInsets>>,
) {
    let Ok(window) = window.single() else {
        return;
    };
    let window_size = Vec2::new(window.width(), window.height());
    let covering: Vec<_> = panels
        .iter()
        .filter_map(
            |(entity, panel, visibility, inherited, record, home_placement, camera_panel)| {
                let management = EdgePanelManagement::for_home(home_placement, record);
                let visible = match *visibility {
                    Visibility::Hidden => matches!(
                        management,
                        EdgePanelManagement::ForHome(
                            PanelPlacement::AtAnchor | PanelPlacement::Moved { .. }
                        )
                    ),
                    Visibility::Visible => true,
                    Visibility::Inherited => {
                        inherited.get()
                            || matches!(
                                management,
                                EdgePanelManagement::ForHome(
                                    PanelPlacement::AtAnchor | PanelPlacement::Moved { .. }
                                ) | EdgePanelManagement::Recorded(
                                    PanelPlacement::AtAnchor | PanelPlacement::Moved { .. }
                                )
                            )
                    },
                };
                if !visible || !reserves_home_space(panel, management) {
                    return None;
                }
                let home_width = home_placement
                    .filter(|placement| placement.1 > 0.0)
                    .map_or_else(|| panel.width(), |placement| placement.1);
                let size = Vec2::new(home_width, panel.height());
                let top_left = match management {
                    EdgePanelManagement::ForHome(PanelPlacement::Moved { top_left })
                    | EdgePanelManagement::Recorded(PanelPlacement::Moved { top_left }) => top_left,
                    EdgePanelManagement::ForHome(PanelPlacement::AtAnchor)
                    | EdgePanelManagement::Recorded(PanelPlacement::AtAnchor) => {
                        separation::inset_anchor_top_left(window_size, size, panel.anchor())
                    },
                    EdgePanelManagement::ForHome(PanelPlacement::Hidden)
                    | EdgePanelManagement::Recorded(PanelPlacement::Hidden) => return None,
                    EdgePanelManagement::Unmanaged => geometry
                        .get(entity)
                        .ok()?
                        .points()
                        .screen_bounds()?
                        .top_left(),
                };
                Some((
                    home_covering_panel(window_size, panel.anchor(), top_left, size),
                    camera_panel.is_some(),
                ))
            },
        )
        .collect();
    let insets = panel_insets::balanced_horizontal_panel_insets(
        window_size,
        covering.iter().map(|(panel, _)| *panel),
        HOME_PANEL_GAP,
    );
    let others = panel_insets::balanced_horizontal_panel_insets(
        window_size,
        covering
            .iter()
            .filter_map(|(panel, camera)| (!camera).then_some(*panel)),
        HOME_PANEL_GAP,
    );
    if let Some(mut other_panel_insets) = other_panel_insets
        && other_panel_insets.0 != others
    {
        other_panel_insets.0 = others;
    }
    if home_insets.0 != insets {
        home_insets.0 = insets;
    }
}

fn home_covering_panel(window: Vec2, anchor: Anchor, top_left: Vec2, size: Vec2) -> CoveringPanel {
    let near_edge = |distance: f32| distance <= SCREEN_PANEL_WINDOW_INSET + 0.001;
    let left = HOME_PANEL_GAP
        + if near_edge(top_left.x) {
            SCREEN_PANEL_WINDOW_INSET
        } else {
            0.0
        };
    let top = HOME_PANEL_GAP
        + if near_edge(top_left.y) {
            SCREEN_PANEL_WINDOW_INSET
        } else {
            0.0
        };
    let right = HOME_PANEL_GAP
        + if near_edge(window.x - top_left.x - size.x) {
            SCREEN_PANEL_WINDOW_INSET
        } else {
            0.0
        };
    let bottom = HOME_PANEL_GAP
        + if near_edge(window.y - top_left.y - size.y) {
            SCREEN_PANEL_WINDOW_INSET
        } else {
            0.0
        };
    CoveringPanel {
        anchor,
        top_left: top_left - Vec2::new(left, top),
        size: size + Vec2::new(left + right, top + bottom),
    }
}

const fn reserves_home_space(panel: &DiegeticPanel, management: EdgePanelManagement) -> bool {
    let CoordinateSpace::Screen { position, .. } = panel.coordinate_space() else {
        return false;
    };
    match management {
        EdgePanelManagement::ForHome(PanelPlacement::Hidden)
        | EdgePanelManagement::Recorded(PanelPlacement::Hidden) => false,
        EdgePanelManagement::ForHome(PanelPlacement::AtAnchor | PanelPlacement::Moved { .. })
        | EdgePanelManagement::Recorded(PanelPlacement::AtAnchor | PanelPlacement::Moved { .. }) => {
            true
        },
        EdgePanelManagement::Unmanaged => matches!(position, ScreenPosition::Screen),
    }
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
    insets: FitViewportInsets,
    duration: Duration,
    face: HomeFace,
) -> AnimateToFit {
    let yaw = match face {
        HomeFace::Front => config.yaw,
        HomeFace::Left => config.yaw - FRAC_PI_2,
        HomeFace::Right => config.yaw + FRAC_PI_2,
    };
    AnimateToFit::new(camera, home)
        .yaw(yaw)
        .pitch(config.pitch)
        .margin(config.margin)
        .anchor(fit_anchor(config.anchor))
        .offset_px(config.offset_px)
        .viewport_insets(insets)
        .duration(duration)
}

fn on_frame_camera_target(
    trigger: On<FrameCameraTarget>,
    mut commands: Commands,
    config: Res<CameraHomeConfig>,
    insets: Res<HomeViewportInsets>,
    cameras: Query<Entity, HomeFitCameraFilter>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };
    commands.trigger(
        AnimateToFit::new(camera, trigger.target)
            .yaw(config.yaw)
            .pitch(config.pitch)
            .margin(trigger.margin)
            .anchor(fit_anchor(config.anchor))
            .offset_px(config.offset_px)
            .viewport_insets(insets.0)
            .duration(trigger.duration),
    );
}

/// Reports whether at least one [`CameraHomeTarget`] entity, or one of its
/// descendants, has an [`Aabb`] yet, so the union the cube was sized to covers
/// real geometry.
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
/// saved restart pose takes precedence: [`RestoreWindowAnimation`] fires instead
/// of the snap, restoring the prior window pose. A [`CameraHomeTarget`] waits
/// for its glyphs/meshes.
fn snap_home_on_ready(
    mut commands: Commands,
    home: Option<Res<CameraHomeEntity>>,
    config: Res<CameraHomeConfig>,
    insets: Res<HomeViewportInsets>,
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
    commands.trigger(home_fit(
        camera,
        home.0,
        &config,
        insets.0,
        Duration::ZERO,
        HomeFace::Front,
    ));
    *state = InitialAnimateState::Fired;
}

fn refit_home_on_viewport_change(
    mut events: MessageReader<WindowResized>,
    mut commands: Commands,
    home: Option<Res<CameraHomeEntity>>,
    config: Res<CameraHomeConfig>,
    insets: Res<HomeViewportInsets>,
    cameras: Query<Entity, HomeFitCameraFilter>,
    at_home: Res<AtHome>,
    face: Res<FramedHomeFace>,
    targets: Query<Entity, With<CameraHomeTarget>>,
    children: Query<&Children>,
    aabbs: Query<&Aabb>,
    other_panel_insets: Option<Res<OtherPanelHomeInsets>>,
) {
    let window_resized = !events.is_empty();
    if !window_resized
        && (!insets.is_changed()
            || other_panel_insets
                .as_ref()
                .is_some_and(|others| !others.is_changed()))
    {
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
    commands.trigger(home_fit(
        camera,
        home.0,
        &config,
        insets.0,
        Duration::ZERO,
        face.0,
    ));
}

fn on_frame_home_face(
    trigger: On<FrameHomeFace>,
    mut commands: Commands,
    home: Res<CameraHomeEntity>,
    config: Res<CameraHomeConfig>,
    insets: Res<HomeViewportInsets>,
    cameras: Query<Entity, HomeFitCameraFilter>,
    mut face: ResMut<FramedHomeFace>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };
    face.0 = trigger.0;
    commands.entity(camera).insert(CameraHomePending);
    commands.trigger(home_fit(
        camera,
        home.0,
        &config,
        insets.0,
        HOME_RETURN_DURATION,
        face.0,
    ));
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
    trigger: On<CameraHomed>,
    mut at_home: ResMut<AtHome>,
    flash: Option<ResMut<HomeTitleBarFlash>>,
    mut bars: Query<&mut TitleBarControlState>,
    home: Option<Res<CameraHomeEntity>>,
    config: Option<Res<CameraHomeConfig>>,
    insets: Option<Res<HomeViewportInsets>>,
    mut commands: Commands,
    mut face: ResMut<FramedHomeFace>,
) {
    if (*at_home == AtHome::No || face.0 != HomeFace::Front)
        && let (Some(home), Some(config), Some(insets)) = (home, config, insets)
    {
        face.0 = HomeFace::Front;
        commands.entity(trigger.camera).insert(CameraHomePending);
        commands.trigger(home_fit(
            trigger.camera,
            home.0,
            &config,
            insets.0,
            HOME_RETURN_DURATION,
            HomeFace::Front,
        ));
    }
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

    #[test]
    fn command_palette_toggle_home_gizmo_follows_the_live_resource() {
        let mut app = crate::test_app();
        app.init_resource::<HomeAabbGizmoVisible>()
            .add_observer(toggle_home_aabb_gizmo);
        assert!(!home_bounds_gizmo_is_shown(app.world()));
        app.world_mut().trigger(ToggleHomeAabbGizmoEvent);
        assert!(home_bounds_gizmo_is_shown(app.world()));
        app.world_mut().trigger(ToggleHomeAabbGizmoEvent);
        assert!(!home_bounds_gizmo_is_shown(app.world()));
    }

    use bevy::camera::RenderTarget;
    use bevy::camera::RenderTargetInfo;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::mouse::AccumulatedMouseMotion;
    use bevy::input::mouse::AccumulatedMouseScroll;
    use bevy::prelude::ButtonInput;
    use bevy::prelude::Camera;
    use bevy::prelude::Camera3d;
    use bevy::prelude::DefaultPlugins;
    use bevy::prelude::GamepadButton;
    use bevy::prelude::KeyCode;
    use bevy::prelude::Messages;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::MouseButton;
    use bevy::prelude::PerspectiveProjection;
    use bevy::prelude::PluginGroup;
    use bevy::prelude::Projection;
    use bevy::prelude::UVec2;
    use bevy::render::RenderPlugin;
    use bevy::time::TimeUpdateStrategy;
    use bevy::transform::TransformPlugin;
    use bevy::window::ExitCondition;
    use bevy::window::WindowPlugin;
    use bevy::window::WindowRef;
    use bevy::winit::WinitPlugin;
    use bevy_enhanced_input::prelude::Binding;
    use hana_diegetic::DiegeticTextMeasurer;
    use hana_diegetic::HeadlessLayoutPlugin;
    use hana_diegetic::PanelPlane;
    use hana_diegetic::Px;
    use hana_lagrange::CameraBasis;
    use hana_lagrange::CameraInputRoutingConfig;
    use hana_lagrange::CurrentFitTarget;
    use hana_lagrange::FreeCamHomePose;
    use hana_lagrange::InteractionSources;
    use hana_lagrange::LagrangePlugin;
    use hana_lagrange::LookAngles;
    use hana_lagrange::OrbitCam;
    use hana_lagrange::OrbitCamHomePose;
    use hana_lagrange::OrbitCamInteractionKind;
    use hana_lagrange::Position;
    use hana_lagrange::ResetFreeCamToHome;
    use hana_lagrange::Roll;

    use super::*;
    use crate::camera_control_panel::CyclePresetEvent;
    use crate::primitive::Face;
    use crate::screen_panels;
    use crate::screen_panels::separation::EdgePanelPrecedence;

    const CAMERA_POSITION: Vec3 = Vec3::new(0.0, 0.0, 8.0);
    const HOME_TARGET_MAX: Vec3 = Vec3::new(0.5, 0.5, 0.5);
    const HOME_TARGET_MIN: Vec3 = Vec3::new(-0.5, -0.5, -0.5);
    const RESIZED_HEIGHT: u16 = 768;
    const RESIZED_WIDTH: u16 = 1024;
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

    fn anchored_left_panel_home_reservation(width: f32) -> FitViewportInsets {
        panel_insets::balanced_horizontal_panel_insets(
            Vec2::new(800.0, 600.0),
            [home_covering_panel(
                Vec2::new(800.0, 600.0),
                Anchor::CenterLeft,
                Vec2::new(SCREEN_PANEL_WINDOW_INSET, 200.0),
                Vec2::new(width, 90.0),
            )],
            HOME_PANEL_GAP,
        )
    }

    #[test]
    fn home_covering_panel_adds_window_inset_only_at_near_edges() {
        let reserved = home_covering_panel(
            Vec2::new(800.0, 600.0),
            Anchor::TopLeft,
            Vec2::new(SCREEN_PANEL_WINDOW_INSET, SCREEN_PANEL_WINDOW_INSET),
            Vec2::new(170.0, 60.0),
        );
        assert!((reserved.top_left - Vec2::splat(-HOME_PANEL_GAP)).length() < 0.001);
        let expected_size = Vec2::new(170.0, 60.0)
            + Vec2::splat(HOME_PANEL_GAP.mul_add(2.0, SCREEN_PANEL_WINDOW_INSET));
        assert!((reserved.size - expected_size).length() < 0.001);
    }

    fn test_app(config: ConfigResource) -> App {
        let mut app = crate::test_app();
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
        let mut app = crate::test_app();
        app.add_plugins((MinimalPlugins, LagrangePlugin))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .add_message::<WindowResized>()
            .insert_resource(test_config())
            .insert_resource(AtHome::Yes)
            .init_resource::<HomeViewportInsets>()
            .init_resource::<FramedHomeFace>()
            .add_systems(
                Update,
                (update_home_cube, refit_home_on_viewport_change).chain(),
            );
        app.finish();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
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
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(u32::from(RESIZED_WIDTH), u32::from(RESIZED_HEIGHT)),
            scale_factor:  1.0,
        });
        app.world_mut()
            .spawn((
                FreeCam::from_pose(CAMERA_POSITION, (0.0, 0.0), 0.0),
                Projection::Perspective(PerspectiveProjection::default()),
                camera,
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

    fn orbit_pose(app: &App, camera: Entity) -> Result<(Vec3, f32, f32, f32), String> {
        let orbit_cam = app
            .world()
            .get::<OrbitCam>(camera)
            .ok_or("camera missing OrbitCam")?;
        let angles = orbit_cam.orbit.current();
        Ok((
            orbit_cam.pan.current().0,
            orbit_cam.zoom.current().0,
            angles.yaw,
            angles.pitch,
        ))
    }

    fn write_window_resized(app: &mut App) {
        let window = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<Messages<WindowResized>>()
            .write(WindowResized {
                window,
                width: f32::from(RESIZED_WIDTH),
                height: f32::from(RESIZED_HEIGHT),
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
    fn home_inset_change_refits_and_away_camera_does_not() -> TestResult {
        let mut app = refit_test_app();
        let camera = spawn_free_fit_camera(&mut app);
        spawn_home_cube(&mut app);
        spawn_home_target(&mut app);
        app.update();
        let full_viewport_pose = free_home_pose(&app, camera)?;

        app.world_mut().resource_mut::<HomeViewportInsets>().0 =
            FitViewportInsets::new(200.0, 0.0, 0.0, 0.0);
        app.update();
        let inset_pose = free_home_pose(&app, camera)?;
        assert_ne!(inset_pose, full_viewport_pose);

        *app.world_mut().resource_mut::<AtHome>() = AtHome::No;
        app.world_mut().resource_mut::<HomeViewportInsets>().0 =
            FitViewportInsets::new(0.0, 0.0, 200.0, 0.0);
        app.update();
        assert_eq!(free_home_pose(&app, camera)?, inset_pose);
        Ok(())
    }

    #[test]
    fn preset_cycle_keeps_home_pose_while_window_resize_refits() -> TestResult {
        for (width, height) in [(1440_u16, 900_u16), (1728_u16, 1079_u16)] {
            let mut app = refit_test_app();
            let camera = spawn_free_fit_camera(&mut app);
            let physical_size = UVec2::new(u32::from(width), u32::from(height));
            app.world_mut()
                .get_mut::<Camera>(camera)
                .ok_or("camera missing")?
                .computed
                .target_info
                .as_mut()
                .ok_or("camera target missing")?
                .physical_size = physical_size;
            spawn_home_cube(&mut app);
            spawn_home_target(&mut app);
            app.update();

            app.insert_resource(OtherPanelHomeInsets::default());
            app.update();

            let initial_pose = free_home_pose(&app, camera)?;
            let window = Vec2::new(f32::from(width), f32::from(height));
            let mut previous_insets = FitViewportInsets::NONE;
            for panel_height in [270.0, 190.0, 0.0, 270.0] {
                let insets = if panel_height == 0.0 {
                    FitViewportInsets::NONE
                } else {
                    panel_insets::balanced_horizontal_panel_insets(
                        window,
                        [home_covering_panel(
                            window,
                            Anchor::BottomRight,
                            window - Vec2::new(360.0, panel_height),
                            Vec2::new(360.0, panel_height),
                        )],
                        HOME_PANEL_GAP,
                    )
                };
                assert_ne!(insets, previous_insets);
                previous_insets = insets;
                app.world_mut().resource_mut::<HomeViewportInsets>().0 = insets;
                app.update();
                assert_eq!(free_home_pose(&app, camera)?, initial_pose);
            }

            write_window_resized(&mut app);
            app.update();
            assert_ne!(free_home_pose(&app, camera)?, initial_pose);
        }
        Ok(())
    }

    fn real_camera_panel_test_app() -> (App, Entity, Entity) {
        let mut app = crate::test_app();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..Default::default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..Default::default()
                })
                .disable::<WinitPlugin>(),
        );
        app.add_plugins(LagrangePlugin);
        install(&mut app, test_config());
        crate::camera_control_panel::install(&mut app);
        screen_panels::install_overlay_picking(&mut app);
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: (1440_u32, 900_u32).into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        app.finish();
        app.cleanup();
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                RenderTarget::Window(WindowRef::Primary),
                OrbitCam::from_pose(Vec3::ZERO, (0.2, -0.1), 4.0),
                OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                FairyDustOrbitCam,
            ))
            .id();
        app.insert_resource(CameraInputRoutingConfig::explicit(camera));
        spawn_home_target(&mut app);
        (app, window, camera)
    }

    fn assert_resize_refits_home(
        app: &mut App,
        window: Entity,
        camera: Entity,
        initial_home: OrbitCamHomePose,
    ) -> TestResult {
        *app.world_mut().resource_mut::<AtHome>() = AtHome::Yes;
        app.world_mut()
            .get_mut::<Window>(window)
            .ok_or("window missing")?
            .resolution
            .set(600.0, 900.0);
        app.world_mut()
            .get_mut::<Camera>(camera)
            .ok_or("camera missing")?
            .computed
            .target_info
            .as_mut()
            .ok_or("camera target missing")?
            .physical_size = UVec2::new(600, 900);
        app.world_mut()
            .resource_mut::<Messages<WindowResized>>()
            .write(WindowResized {
                window,
                width: 600.0,
                height: 900.0,
            });
        for _ in 0..4 {
            app.update();
        }
        assert_ne!(
            app.world().get::<OrbitCamHomePose>(camera),
            Some(&initial_home)
        );
        Ok(())
    }

    fn assert_cycle_step_preserves_home(
        app: &App,
        camera: Entity,
        initial_transform: Transform,
        initial_home: OrbitCamHomePose,
    ) -> TestResult {
        let transform = app
            .world()
            .get::<Transform>(camera)
            .ok_or("camera transform missing")?;
        assert!((transform.translation - initial_transform.translation).length() < 1.0e-4);
        assert!(transform.rotation.dot(initial_transform.rotation).abs() > 1.0 - 1.0e-5);
        assert_eq!(
            app.world().get::<OrbitCamHomePose>(camera),
            Some(&initial_home)
        );
        Ok(())
    }

    #[test]
    fn real_camera_panel_cycle_never_refits_home_after_layout_settles() -> TestResult {
        let (mut app, window, camera) = real_camera_panel_test_app();
        for (width, height) in [(1440_u16, 900_u16), (1728, 1079)] {
            app.world_mut()
                .get_mut::<Window>(window)
                .ok_or("window missing")?
                .resolution
                .set(f32::from(width), f32::from(height));
            if let Some(target) = app
                .world_mut()
                .get_mut::<Camera>(camera)
                .ok_or("camera missing")?
                .computed
                .target_info
                .as_mut()
            {
                target.physical_size = UVec2::new(u32::from(width), u32::from(height));
            }
            app.world_mut()
                .resource_mut::<Messages<WindowResized>>()
                .write(WindowResized {
                    window,
                    width: f32::from(width),
                    height: f32::from(height),
                });
            for _ in 0..12 {
                app.update();
            }
            let initial_transform = *app
                .world()
                .get::<Transform>(camera)
                .ok_or("camera transform missing")?;
            let initial_home = *app
                .world()
                .get::<OrbitCamHomePose>(camera)
                .ok_or("orbit home missing")?;
            let initial_limits = app
                .world()
                .get::<OrbitCam>(camera)
                .ok_or("orbit missing")?
                .zoom
                .limit();
            let mut panel_insets = vec![app.world().resource::<HomeViewportInsets>().0];
            assert_ne!(
                panel_insets[0],
                FitViewportInsets::NONE,
                "camera panel must reserve space"
            );
            let mut free_home_poses = Vec::new();
            for step in 0..4 {
                app.world_mut().trigger(CyclePresetEvent);
                for _ in 0..12 {
                    app.update();
                }
                assert_cycle_step_preserves_home(&app, camera, initial_transform, initial_home)?;
                if step >= 1 {
                    free_home_poses.push(
                        *app.world()
                            .get::<FreeCamHomePose>(camera)
                            .ok_or("free home missing")?,
                    );
                }
                if let Some(orbit) = app.world().get::<OrbitCam>(camera) {
                    assert_eq!(orbit.zoom.limit(), initial_limits);
                }
                panel_insets.push(app.world().resource::<HomeViewportInsets>().0);
            }
            assert_eq!(free_home_poses.len(), 3);
            assert!(
                free_home_poses
                    .iter()
                    .all(|home| *home == free_home_poses[0])
            );
            assert!(
                panel_insets.windows(2).any(|pair| pair[0] != pair[1]),
                "preset cycle must change camera panel insets"
            );
            assert_eq!(
                app.world()
                    .get::<OrbitCam>(camera)
                    .ok_or("orbit missing")?
                    .zoom
                    .limit(),
                initial_limits
            );
            assert_resize_refits_home(&mut app, window, camera, initial_home)?;
        }
        Ok(())
    }

    #[test]
    fn another_edge_panel_reservation_still_refits_home() -> TestResult {
        let mut app = refit_test_app();
        app.init_resource::<OtherPanelHomeInsets>().add_systems(
            Update,
            update_home_viewport_insets.before(refit_home_on_viewport_change),
        );
        let camera = spawn_free_fit_camera(&mut app);
        spawn_home_cube(&mut app);
        spawn_home_target(&mut app);
        app.update();
        let before = free_home_pose(&app, camera)?;
        let mut panel = DiegeticPanel::screen()
            .size(Px(170.0), Px(60.0))
            .anchor(Anchor::BottomRight)
            .build()
            .map_err(|_| "edge panel should build")?;
        panel.set_width(170.0);
        panel.set_height(60.0);
        app.world_mut().spawn((
            panel,
            Visibility::Inherited,
            InheritedVisibility::VISIBLE,
            EdgePanelPlacementRecord {
                placement: PanelPlacement::AtAnchor,
            },
        ));
        app.update();
        assert_ne!(free_home_pose(&app, camera)?, before);
        Ok(())
    }

    #[test]
    fn world_obstacle_hiding_edge_panel_does_not_change_home_fit() -> TestResult {
        let mut app = refit_test_app();
        let camera = spawn_free_fit_camera(&mut app);
        spawn_home_cube(&mut app);
        spawn_home_target(&mut app);
        let panel = DiegeticPanel::screen()
            .size(Px(170.0), Px(60.0))
            .anchor(Anchor::TopLeft)
            .build()
            .map_err(|_| "screen panel should build")?;
        let edge = app
            .world_mut()
            .spawn((
                panel,
                Visibility::Visible,
                InheritedVisibility::VISIBLE,
                EdgePanelPlacementRecord {
                    placement: PanelPlacement::AtAnchor,
                },
                EdgePanelHomePlacement(PanelPlacement::AtAnchor, 0.0),
            ))
            .id();
        app.world_mut()
            .run_system_once(update_home_viewport_insets)
            .map_err(|_| "home insets system should run")?;
        app.update();
        let stable_pose = free_home_pose(&app, camera)?;
        let stable_insets = app.world().resource::<HomeViewportInsets>().0;
        for frame in 0..20 {
            let hidden = frame % 2 == 0;
            app.world_mut().entity_mut(edge).insert((
                if hidden {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                },
                EdgePanelPlacementRecord {
                    placement: if hidden {
                        PanelPlacement::Hidden
                    } else {
                        PanelPlacement::AtAnchor
                    },
                },
            ));
            app.world_mut()
                .run_system_once(update_home_viewport_insets)
                .map_err(|_| "home insets system should run")?;
            app.update();
            assert_eq!(
                app.world().resource::<HomeViewportInsets>().0,
                stable_insets
            );
            assert_eq!(free_home_pose(&app, camera)?, stable_pose);
        }
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the viewport loop shares one complete scene"
    )]
    fn home_frames_three_faces_clear_of_five_example_panels_at_four_viewports() -> Result<(), String>
    {
        fn projected_face_bounds(
            app: &App,
            camera: Entity,
            face_entity: Entity,
        ) -> Result<(Vec2, Vec2), String> {
            let camera_component = app.world().get::<Camera>(camera).ok_or("camera missing")?;
            let camera_transform = app
                .world()
                .get::<GlobalTransform>(camera)
                .ok_or("camera transform missing")?;
            let face_panel = app
                .world()
                .get::<DiegeticPanel>(face_entity)
                .ok_or("face missing")?;
            let face_transform = app
                .world()
                .get::<GlobalTransform>(face_entity)
                .ok_or("face transform missing")?;
            let plane = PanelPlane::from_panel(face_panel, face_transform)
                .map_err(|error| error.to_string())?;
            let corners = [
                Anchor::TopLeft,
                Anchor::TopRight,
                Anchor::BottomLeft,
                Anchor::BottomRight,
            ]
            .map(|corner| {
                camera_component
                    .world_to_viewport(camera_transform, plane.point(corner))
                    .map_err(|error| error.to_string())
            });
            let corners: Vec<Vec2> = corners.into_iter().collect::<Result<_, _>>()?;
            Ok((
                corners
                    .iter()
                    .copied()
                    .fold(Vec2::splat(f32::INFINITY), Vec2::min),
                corners
                    .iter()
                    .copied()
                    .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max),
            ))
        }

        for (physical_width, physical_height) in
            [(400_u16, 500_u16), (900, 800), (1500, 2012), (2538, 2012)]
        {
            let logical_width = f32::from(physical_width) / 2.0;
            let logical_height = f32::from(physical_height) / 2.0;
            let mut config = test_config();
            config.margin = 0.1;
            let mut app = crate::test_app();
            app.add_plugins((MinimalPlugins, TransformPlugin, LagrangePlugin))
                .insert_resource(DiegeticTextMeasurer::default())
                .add_plugins(HeadlessLayoutPlugin)
                .init_resource::<Assets<Mesh>>()
                .init_resource::<ButtonInput<KeyCode>>()
                .init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<AccumulatedMouseMotion>()
                .init_resource::<AccumulatedMouseScroll>()
                .add_message::<WindowResized>()
                .insert_resource(config)
                .insert_resource(AtHome::Yes)
                .init_resource::<HomeViewportInsets>()
                .init_resource::<FramedHomeFace>()
                .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                    16,
                )))
                .add_systems(
                    PreUpdate,
                    fill_camera_home_presets.before(CameraInputPhase::PreInput),
                )
                .add_systems(
                    Update,
                    (
                        update_home_cube,
                        update_home_viewport_insets,
                        snap_home_on_ready,
                        refit_home_on_viewport_change,
                    )
                        .chain(),
                );
            app.add_observer(on_camera_homed);
            app.add_observer(on_frame_home_face);
            app.add_observer(on_home_animation_begin);
            app.add_observer(on_home_animation_end);
            app.add_observer(on_non_home_animation_begin);
            app.add_observer(on_orbit_user_interaction_started);
            screen_panels::install_overlay_picking(&mut app);
            app.finish();
            let window = app
                .world_mut()
                .spawn((
                    Window {
                        resolution: (
                            u32::from(physical_width / 2),
                            u32::from(physical_height / 2),
                        )
                            .into(),
                        ..Default::default()
                    },
                    PrimaryWindow,
                ))
                .id();
            let mut camera_component = Camera::default();
            camera_component.computed.target_info = Some(RenderTargetInfo {
                physical_size: UVec2::new(u32::from(physical_width), u32::from(physical_height)),
                scale_factor:  2.0,
            });
            let mut projection = Projection::Perspective(PerspectiveProjection::default());
            projection.update(logical_width, logical_height);
            camera_component.computed.clip_from_view = projection.get_clip_from_view();
            let camera = app
                .world_mut()
                .spawn((
                    OrbitCam::from_pose(Vec3::ZERO, (0.0, 0.0), CAMERA_POSITION.z),
                    FairyDustOrbitCam,
                    OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse()),
                    projection,
                    camera_component,
                    RenderTarget::Window(WindowRef::Primary),
                    CameraBasis::Y_UP,
                    Transform::from_translation(CAMERA_POSITION),
                    GlobalTransform::from_translation(CAMERA_POSITION),
                ))
                .id();
            app.insert_resource(CameraInputRoutingConfig::explicit(camera));
            spawn_home_cube(&mut app);
            let home = spawn_home_target(&mut app);
            let home_mesh = app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(Cuboid::from_size(Vec3::splat(2.0)));
            app.world_mut().entity_mut(home).insert((
                Mesh3d(home_mesh),
                Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
            ));
            let mut face = DiegeticPanel::world()
                .size(Px(200.0), Px(200.0))
                .world_width(2.0)
                .anchor(Anchor::TopLeft)
                .build()
                .map_err(|error| error.to_string())?;
            face.set_width(200.0);
            face.set_height(200.0);
            let face_entity = app
                .world_mut()
                .spawn((
                    face,
                    Transform::from_xyz(-1.0, 1.0, 1.0),
                    GlobalTransform::from_translation(Vec3::new(-1.0, 1.0, 1.0)),
                ))
                .id();
            app.world_mut().entity_mut(home).add_child(face_entity);
            let side_faces: Vec<_> = [Face::Left, Face::Right]
                .into_iter()
                .map(|cube_face| -> Result<_, String> {
                    let mut panel = DiegeticPanel::world()
                        .size(Px(200.0), Px(200.0))
                        .world_width(2.0)
                        .anchor(Anchor::TopLeft)
                        .build()
                        .map_err(|error| error.to_string())?;
                    panel.set_width(200.0);
                    panel.set_height(200.0);
                    let mut transform = crate::primitive::cube_face_transform(cube_face, 2.0);
                    transform.translation += transform.rotation * Vec3::new(-1.0, 1.0, 0.0);
                    let entity = app
                        .world_mut()
                        .spawn((panel, transform, GlobalTransform::from(transform)))
                        .id();
                    app.world_mut().entity_mut(home).add_child(entity);
                    Ok(entity)
                })
                .collect::<Result<_, _>>()?;
            // The widgets example's main bar, Floating panel, camera panel,
            // follower card, and help panel. FitMax panels use their authored
            // maxima; the text panels use conservative solved-size envelopes.
            let example_panels = [
                (
                    "main bar",
                    Anchor::TopLeft,
                    EdgePanelPrecedence::TitleBar,
                    Vec2::new(400.0, 70.0),
                ),
                (
                    "Floating",
                    Anchor::TopRight,
                    EdgePanelPrecedence::HostPanel,
                    Vec2::new(250.0, 120.0),
                ),
                (
                    "camera",
                    Anchor::BottomRight,
                    EdgePanelPrecedence::CameraControl,
                    Vec2::new(317.0, 463.0),
                ),
                (
                    "follower",
                    Anchor::TopRight,
                    EdgePanelPrecedence::HostPanel,
                    Vec2::new(360.0, 64.0),
                ),
                (
                    "help",
                    Anchor::BottomLeft,
                    EdgePanelPrecedence::Description,
                    Vec2::new(482.0, 300.0),
                ),
            ];
            let edges: Vec<_> = example_panels
                .iter()
                .copied()
                .map(
                    |(_name, anchor, precedence, size)| -> Result<Entity, String> {
                        let mut panel = DiegeticPanel::screen()
                            .size(Px(size.x), Px(size.y))
                            .anchor(anchor)
                            .build()
                            .map_err(|error| error.to_string())?;
                        panel.set_width(size.x);
                        panel.set_height(size.y);
                        Ok(app
                            .world_mut()
                            .spawn((
                                panel,
                                Visibility::Inherited,
                                InheritedVisibility::VISIBLE,
                                precedence,
                            ))
                            .id())
                    },
                )
                .collect::<Result<_, _>>()?;
            let mut settled = None;
            let mut projected_face = (Vec2::ZERO, Vec2::ZERO);
            for frame in 0..80 {
                app.update();
                let pose = orbit_pose(&app, camera)?;
                let panels: Vec<_> = edges
                    .iter()
                    .map(|&entity| {
                        (
                            app.world()
                                .get::<EdgePanelPlacementRecord>(entity)
                                .map(|record| record.placement),
                            app.world().get::<Visibility>(entity).copied(),
                        )
                    })
                    .collect();
                if frame >= 60 {
                    let state = (pose, panels.clone());
                    if let Some(previous) = settled {
                        assert_eq!(
                            state, previous,
                            "{physical_width}x{physical_height}: home loop did not settle"
                        );
                    }
                    settled = Some(state);
                    let window_size = Vec2::new(logical_width, logical_height);
                    let reserved: Vec<_> = edges
                        .iter()
                        .filter_map(|&entity| {
                            let panel = app.world().get::<DiegeticPanel>(entity)?;
                            let reservation = app.world().get::<EdgePanelHomePlacement>(entity)?.0;
                            let size = Vec2::new(panel.width(), panel.height());
                            let top_left = match reservation {
                                PanelPlacement::AtAnchor => {
                                    screen_panels::separation::inset_anchor_top_left(
                                        window_size,
                                        size,
                                        panel.anchor(),
                                    )
                                },
                                PanelPlacement::Moved { top_left } => top_left,
                                PanelPlacement::Hidden => return None,
                            };
                            Some(home_covering_panel(
                                window_size,
                                panel.anchor(),
                                top_left,
                                size,
                            ))
                        })
                        .collect();
                    let expected = panel_insets::balanced_horizontal_panel_insets(
                        window_size,
                        reserved,
                        HOME_PANEL_GAP,
                    );
                    assert_eq!(
                        app.world().resource::<HomeViewportInsets>().0,
                        expected,
                        "{physical_width}x{physical_height}: Home insets must use edge reservations"
                    );
                    assert_ne!(
                        app.world().get::<Visibility>(edges[0]),
                        Some(&Visibility::Hidden),
                        "{physical_width}x{physical_height}: title bar must stay shown"
                    );
                }
                let (min, max) = projected_face_bounds(&app, camera, face_entity)?;
                projected_face = (min, max);
                if frame < 60 {
                    continue;
                }
                for (&entity, (placement, visibility)) in edges.iter().zip(&panels) {
                    if *visibility == Some(Visibility::Hidden)
                        || *placement == Some(PanelPlacement::Hidden)
                    {
                        continue;
                    }
                    let panel = app
                        .world()
                        .get::<DiegeticPanel>(entity)
                        .ok_or("edge missing")?;
                    let size = Vec2::new(panel.width(), panel.height());
                    let top_left = match placement.ok_or("edge placement missing")? {
                        PanelPlacement::AtAnchor => {
                            screen_panels::separation::inset_anchor_top_left(
                                Vec2::new(logical_width, logical_height),
                                size,
                                panel.anchor(),
                            )
                        },
                        PanelPlacement::Moved { top_left } => top_left,
                        PanelPlacement::Hidden => continue,
                    };
                    assert!(
                        top_left.x >= max.x
                            || top_left.y >= max.y
                            || top_left.x + size.x <= min.x
                            || top_left.y + size.y <= min.y,
                        "{physical_width}x{physical_height}: shown edge panel covers the face"
                    );
                    if entity != edges[0] {
                        let title = app
                            .world()
                            .get::<DiegeticPanel>(edges[0])
                            .ok_or("title missing")?;
                        let title_top_left = screen_panels::separation::inset_anchor_top_left(
                            Vec2::new(logical_width, logical_height),
                            Vec2::new(title.width(), title.height()),
                            title.anchor(),
                        );
                        assert!(
                            top_left.x >= title_top_left.x + title.width()
                                || top_left.y >= title_top_left.y + title.height()
                                || top_left.x + size.x <= title_top_left.x
                                || top_left.y + size.y <= title_top_left.y,
                            "{physical_width}x{physical_height}: shown panel overlaps the title bar"
                        );
                    }
                }
            }
            if physical_width == 900 {
                assert!(
                    edges.iter().any(|&entity| {
                        app.world()
                            .get::<EdgePanelPlacementRecord>(entity)
                            .is_some_and(|record| record.placement == PanelPlacement::Hidden)
                    }),
                    "narrow viewport must hide panels that cannot clear the face"
                );
            }
            if physical_width == 1500 {
                assert!(
                    projected_face.1.x - projected_face.0.x >= logical_width * 0.65,
                    "middle viewport must give the face usable width: {projected_face:?}",
                );
            }
            for (home_face, face_entity) in [
                (HomeFace::Front, face_entity),
                (HomeFace::Left, side_faces[0]),
                (HomeFace::Right, side_faces[1]),
            ] {
                app.world_mut().trigger(FrameHomeFace(home_face));
                for _ in 0..80 {
                    app.update();
                }
                let (min, max) = projected_face_bounds(&app, camera, face_entity)?;
                for ((name, _, _, _), &entity) in example_panels.iter().zip(&edges) {
                    let placement = app
                        .world()
                        .get::<EdgePanelPlacementRecord>(entity)
                        .ok_or("edge placement missing")?
                        .placement;
                    if physical_width == 2538 {
                        assert_ne!(
                            placement,
                            PanelPlacement::Hidden,
                            "{physical_width}x{physical_height}: {home_face:?} hides {name}"
                        );
                    }
                    if placement == PanelPlacement::Hidden
                        || app.world().get::<Visibility>(entity) == Some(&Visibility::Hidden)
                    {
                        assert_eq!(
                            app.world().get::<Visibility>(entity),
                            Some(&Visibility::Hidden),
                            "{physical_width}x{physical_height}: hidden {name} still visible"
                        );
                        continue;
                    }
                    let panel = app
                        .world()
                        .get::<DiegeticPanel>(entity)
                        .ok_or("edge missing")?;
                    let size = Vec2::new(panel.width(), panel.height());
                    let top_left = match placement {
                        PanelPlacement::AtAnchor => {
                            screen_panels::separation::inset_anchor_top_left(
                                Vec2::new(logical_width, logical_height),
                                size,
                                panel.anchor(),
                            )
                        },
                        PanelPlacement::Moved { top_left } => top_left,
                        PanelPlacement::Hidden => continue,
                    };
                    assert!(
                        top_left.x >= max.x
                            || top_left.y >= max.y
                            || top_left.x + size.x <= min.x
                            || top_left.y + size.y <= min.y,
                        "{physical_width}x{physical_height}: {home_face:?} face overlaps {name}"
                    );
                }
            }
            app.world_mut().trigger(FrameHomeFace(HomeFace::Front));
            for _ in 0..80 {
                app.update();
            }
            let home_pose = orbit_pose(&app, camera)?;
            *app.world_mut().resource_mut::<AtHome>() = AtHome::No;
            {
                let mut orbit_cam = app
                    .world_mut()
                    .get_mut::<OrbitCam>(camera)
                    .ok_or("camera missing OrbitCam")?;
                orbit_cam.pan.snap_to(Vec3::splat(4.0));
                orbit_cam.force_update();
            }
            app.update();
            assert_ne!(orbit_pose(&app, camera)?, home_pose);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(HOME_KEY);
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(HOME_KEY);
            let mut returned = None;
            for frame in 0..80 {
                app.update();
                if frame >= 60 {
                    let pose = orbit_pose(&app, camera)?;
                    let visibility: Vec<_> = edges
                        .iter()
                        .map(|&entity| app.world().get::<Visibility>(entity).copied())
                        .collect();
                    let state = (pose, visibility);
                    if let Some(previous) = &returned {
                        assert_eq!(&state, previous, "H result did not settle");
                    }
                    returned = Some(state);
                }
            }
            assert_eq!(orbit_pose(&app, camera)?, home_pose);
            let (returned_min, returned_max) = projected_face_bounds(&app, camera, face_entity)?;
            for &entity in &edges {
                if app.world().get::<Visibility>(entity) == Some(&Visibility::Hidden) {
                    continue;
                }
                let panel = app
                    .world()
                    .get::<DiegeticPanel>(entity)
                    .ok_or("edge missing")?;
                let size = Vec2::new(panel.width(), panel.height());
                let placement = app
                    .world()
                    .get::<EdgePanelPlacementRecord>(entity)
                    .ok_or("edge placement missing")?
                    .placement;
                let top_left = match placement {
                    PanelPlacement::AtAnchor => screen_panels::separation::inset_anchor_top_left(
                        Vec2::new(logical_width, logical_height),
                        size,
                        panel.anchor(),
                    ),
                    PanelPlacement::Moved { top_left } => top_left,
                    PanelPlacement::Hidden => continue,
                };
                assert!(
                    top_left.x >= returned_max.x
                        || top_left.y >= returned_max.y
                        || top_left.x + size.x <= returned_min.x
                        || top_left.y + size.y <= returned_min.y,
                    "H left an edge panel over the face at {physical_width}x{physical_height}"
                );
            }
            if physical_width == 2538 {
                assert!(
                    edges
                        .iter()
                        .all(|&entity| app.world().get::<Visibility>(entity)
                            != Some(&Visibility::Hidden)),
                    "Home must show all five panels: {:?}, pose {:?}, insets {:?}, face {:?}",
                    edges
                        .iter()
                        .map(|&entity| (
                            app.world()
                                .get::<EdgePanelPlacementRecord>(entity)
                                .map(|record| record.placement),
                            app.world().get::<Visibility>(entity).copied(),
                        ))
                        .collect::<Vec<_>>(),
                    orbit_pose(&app, camera),
                    app.world().resource::<HomeViewportInsets>().0,
                    projected_face,
                );
                let (min, max) = projected_face_bounds(&app, camera, face_entity)?;
                for &entity in &edges {
                    let panel = app
                        .world()
                        .get::<DiegeticPanel>(entity)
                        .ok_or("edge missing")?;
                    let placement = app
                        .world()
                        .get::<EdgePanelPlacementRecord>(entity)
                        .ok_or("edge placement missing")?
                        .placement;
                    let size = Vec2::new(panel.width(), panel.height());
                    let top_left = match placement {
                        PanelPlacement::AtAnchor => {
                            screen_panels::separation::inset_anchor_top_left(
                                Vec2::new(logical_width, logical_height),
                                size,
                                panel.anchor(),
                            )
                        },
                        PanelPlacement::Moved { top_left } => top_left,
                        PanelPlacement::Hidden => return Err("H hid an edge panel".to_owned()),
                    };
                    assert!(
                        top_left.x >= max.x
                            || top_left.y >= max.y
                            || top_left.x + size.x <= min.x
                            || top_left.y + size.y <= min.y,
                        "H left an edge panel over the face"
                    );
                }
            }
            if physical_width == 900 {
                app.world_mut().despawn(face_entity);
                for &entity in &edges {
                    let mut panel = app
                        .world_mut()
                        .get_mut::<DiegeticPanel>(entity)
                        .ok_or("edge missing")?;
                    if panel.anchor() == Anchor::TopLeft {
                        panel.set_width(200.0);
                        panel.set_height(20.0);
                    } else {
                        panel.set_width(140.0);
                        panel.set_height(80.0);
                    }
                }
                let mut mesh_settled = None;
                for frame in 0..80 {
                    app.update();
                    if frame >= 60 {
                        let state = (
                            orbit_pose(&app, camera)?,
                            edges
                                .iter()
                                .map(|&entity| {
                                    (
                                        app.world()
                                            .get::<EdgePanelPlacementRecord>(entity)
                                            .map(|record| record.placement),
                                        app.world().get::<Visibility>(entity).copied(),
                                    )
                                })
                                .collect::<Vec<_>>(),
                        );
                        if let Some(previous) = &mesh_settled {
                            assert_eq!(&state, previous, "mesh Home loop did not settle");
                        }
                        mesh_settled = Some(state);
                    }
                }
                let insets = app.world().resource::<HomeViewportInsets>().0;
                let full = panel_insets::balanced_horizontal_panel_insets(
                    Vec2::new(logical_width, logical_height),
                    edges.iter().filter_map(|&entity| {
                        if app.world().get::<Visibility>(entity) == Some(&Visibility::Hidden) {
                            return None;
                        }
                        let panel = app.world().get::<DiegeticPanel>(entity)?;
                        let placement = app.world().get::<EdgePanelHomePlacement>(entity)?.0;
                        let size = Vec2::new(panel.width(), panel.height());
                        let top_left = match placement {
                            PanelPlacement::AtAnchor => {
                                screen_panels::separation::inset_anchor_top_left(
                                    Vec2::new(logical_width, logical_height),
                                    size,
                                    panel.anchor(),
                                )
                            },
                            PanelPlacement::Moved { top_left } => top_left,
                            PanelPlacement::Hidden => return None,
                        };
                        Some(home_covering_panel(
                            Vec2::new(logical_width, logical_height),
                            panel.anchor(),
                            top_left,
                            size,
                        ))
                    }),
                    HOME_PANEL_GAP,
                );
                assert_eq!(insets, full, "mesh target keeps every shown reservation");
                let camera_component = app.world().get::<Camera>(camera).ok_or("camera missing")?;
                let camera_transform = app
                    .world()
                    .get::<GlobalTransform>(camera)
                    .ok_or("camera transform missing")?;
                let corners: Vec<Vec2> = AABB_CORNER_SIGNS
                    .into_iter()
                    .map(|corner| {
                        camera_component
                            .world_to_viewport(camera_transform, corner)
                            .map_err(|error| error.to_string())
                    })
                    .collect::<Result<_, _>>()?;
                let min = corners
                    .iter()
                    .copied()
                    .fold(Vec2::splat(f32::INFINITY), Vec2::min);
                let max = corners
                    .iter()
                    .copied()
                    .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
                for &entity in &edges {
                    if app.world().get::<Visibility>(entity) == Some(&Visibility::Hidden) {
                        continue;
                    }
                    let panel = app
                        .world()
                        .get::<DiegeticPanel>(entity)
                        .ok_or("edge missing")?;
                    let size = Vec2::new(panel.width(), panel.height());
                    let placement = app
                        .world()
                        .get::<EdgePanelPlacementRecord>(entity)
                        .ok_or("placement missing")?
                        .placement;
                    let top_left = match placement {
                        PanelPlacement::AtAnchor => {
                            screen_panels::separation::inset_anchor_top_left(
                                Vec2::new(logical_width, logical_height),
                                size,
                                panel.anchor(),
                            )
                        },
                        PanelPlacement::Moved { top_left } => top_left,
                        PanelPlacement::Hidden => continue,
                    };
                    assert!(
                        top_left.x >= max.x
                            || top_left.y >= max.y
                            || top_left.x + size.x <= min.x
                            || top_left.y + size.y <= min.y,
                        "shown panel overlaps mesh home target"
                    );
                }
                assert_ne!(
                    app.world().get::<Visibility>(edges[0]),
                    Some(&Visibility::Hidden)
                );
                assert_eq!(
                    app.world().get::<Visibility>(edges[4]),
                    Some(&Visibility::Hidden),
                    "the small window keeps the auxiliary help panel hidden"
                );
            }
            let _ = window;
        }
        Ok(())
    }

    #[test]
    fn mesh_home_target_keeps_side_and_top_left_title_reservations() -> Result<(), String> {
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<HomeViewportInsets>()
            .add_systems(Update, update_home_viewport_insets);
        let window_size = Vec2::new(450.0, 400.0);
        app.world_mut().spawn((
            Window {
                resolution: (450, 400).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        app.world_mut().spawn((
            CameraHomeMarker,
            Transform::from_scale(Vec3::new(2.0, 2.0, 2.0)),
        ));
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::from_size(Vec3::splat(2.0)));
        app.world_mut().spawn((
            CameraHomeTarget,
            Mesh3d(mesh),
            Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
            Transform::default(),
            GlobalTransform::default(),
        ));
        let panels = [
            (
                Anchor::CenterLeft,
                Vec2::new(140.0, 120.0),
                EdgePanelPrecedence::HostPanel,
            ),
            (
                Anchor::CenterRight,
                Vec2::new(140.0, 120.0),
                EdgePanelPrecedence::HostPanel,
            ),
            (
                Anchor::TopLeft,
                Vec2::new(200.0, 30.0),
                EdgePanelPrecedence::TitleBar,
            ),
        ];
        for (anchor, size, precedence) in panels {
            let mut panel = DiegeticPanel::screen()
                .size(Px(size.x), Px(size.y))
                .anchor(anchor)
                .build()
                .map_err(|error| error.to_string())?;
            panel.set_width(size.x);
            panel.set_height(size.y);
            app.world_mut().spawn((
                panel,
                Visibility::Inherited,
                InheritedVisibility::VISIBLE,
                EdgePanelHomePlacement(PanelPlacement::AtAnchor, 0.0),
                precedence,
            ));
        }
        app.update();
        let insets = app.world().resource::<HomeViewportInsets>().0;
        let expected = panel_insets::balanced_horizontal_panel_insets(
            window_size,
            panels.map(|(anchor, size, _)| {
                home_covering_panel(
                    window_size,
                    anchor,
                    screen_panels::separation::inset_anchor_top_left(window_size, size, anchor),
                    size,
                )
            }),
            HOME_PANEL_GAP,
        );
        assert_eq!(insets, expected);
        assert_ne!(insets, FitViewportInsets::NONE);
        Ok(())
    }

    #[test]
    fn home_reservation_uses_its_solved_panel_width() -> Result<(), String> {
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin)
            .init_resource::<HomeViewportInsets>()
            .add_systems(Update, update_home_viewport_insets);
        let window_size = Vec2::new(1728.0, 1117.0);
        app.world_mut().spawn((
            Window {
                resolution: (1728, 1117).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        let mut panel = DiegeticPanel::screen()
            .size(Px(1000.0), Px(100.0))
            .anchor(Anchor::BottomLeft)
            .build()
            .map_err(|error| error.to_string())?;
        panel.set_width(1000.0);
        panel.set_height(100.0);
        app.world_mut().spawn((
            panel,
            Visibility::Inherited,
            InheritedVisibility::VISIBLE,
            EdgePanelHomePlacement(PanelPlacement::AtAnchor, 700.0),
        ));
        app.update();
        let size = Vec2::new(700.0, 100.0);
        let expected = panel_insets::balanced_horizontal_panel_insets(
            window_size,
            [home_covering_panel(
                window_size,
                Anchor::BottomLeft,
                screen_panels::separation::inset_anchor_top_left(
                    window_size,
                    size,
                    Anchor::BottomLeft,
                ),
                size,
            )],
            HOME_PANEL_GAP,
        );
        assert_eq!(app.world().resource::<HomeViewportInsets>().0, expected);
        Ok(())
    }

    #[test]
    fn home_right_panel_balances_window_and_panel_margins() -> Result<(), String> {
        let panel_size = Vec2::new(317.0, 463.0);
        for (width, height) in [(1440_u16, 900_u16), (1728, 1079)] {
            let mut app = crate::test_app();
            app.add_plugins(MinimalPlugins)
                .init_resource::<HomeViewportInsets>()
                .add_systems(Update, update_home_viewport_insets);
            app.world_mut().spawn((
                Window {
                    resolution: (u32::from(width), u32::from(height)).into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ));
            let mut panel = DiegeticPanel::screen()
                .size(Px(panel_size.x), Px(panel_size.y))
                .anchor(Anchor::BottomRight)
                .build()
                .map_err(|error| error.to_string())?;
            panel.set_width(panel_size.x);
            panel.set_height(panel_size.y);
            app.world_mut().spawn((
                panel,
                Visibility::Inherited,
                InheritedVisibility::VISIBLE,
                EdgePanelHomePlacement(PanelPlacement::AtAnchor, 0.0),
            ));
            app.update();

            let window = Vec2::new(f32::from(width), f32::from(height));
            let panel_left =
                separation::inset_anchor_top_left(window, panel_size, Anchor::BottomRight).x;
            let insets = app.world().resource::<HomeViewportInsets>().0;
            assert_eq!(
                insets,
                FitViewportInsets::new(
                    HOME_PANEL_GAP,
                    0.0,
                    window.x - panel_left + HOME_PANEL_GAP,
                    0.0,
                ),
                "{width}×{height}: Home must leave the same gap at the window and panel edges"
            );
        }
        Ok(())
    }

    #[test]
    fn home_printed_area_keeps_its_screen_size_when_camera_panel_changes() -> TestResult {
        let window = Vec2::new(1440.0, 900.0);
        let mut app = refit_test_app();
        app.add_observer(on_frame_home_face);
        let camera = spawn_free_fit_camera(&mut app);
        app.world_mut()
            .get_mut::<Camera>(camera)
            .ok_or("camera missing")?
            .computed
            .target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(1440, 900),
            scale_factor:  1.0,
        });
        app.world_mut()
            .get_mut::<Projection>(camera)
            .ok_or("projection missing")?
            .update(window.x, window.y);
        let home = spawn_home_cube(&mut app);
        let printed_area = spawn_home_target(&mut app);
        let _whole_page = app.world_mut().spawn((
            Aabb::from_min_max(Vec3::splat(-2.0), Vec3::splat(2.0)),
            Transform::default(),
            GlobalTransform::default(),
        ));
        let title = CoveringPanel {
            anchor:   Anchor::TopCenter,
            top_left: Vec2::ZERO,
            size:     Vec2::new(1440.0, 40.0),
        };
        let help = CoveringPanel {
            anchor:   Anchor::BottomLeft,
            top_left: Vec2::new(0.0, 780.0),
            size:     Vec2::new(250.0, 120.0),
        };
        let camera_panel = |size: Vec2| CoveringPanel {
            anchor: Anchor::BottomRight,
            top_left: window - size,
            size,
        };
        let screen_height = |app: &App| -> Result<f32, &'static str> {
            let free = app
                .world()
                .get::<FreeCam>(camera)
                .ok_or("FreeCam missing")?;
            let Projection::Perspective(projection) = app
                .world()
                .get::<Projection>(camera)
                .ok_or("projection missing")?
            else {
                return Err("perspective projection missing");
            };
            Ok(window.y / (2.0 * free.translate.current().0.z * (projection.fov / 2.0).tan()))
        };

        let mut first_height = 0.0;
        for (index, size) in [Vec2::new(317.0, 463.0), Vec2::new(280.0, 270.0)]
            .into_iter()
            .enumerate()
        {
            app.world_mut().resource_mut::<HomeViewportInsets>().0 =
                panel_insets::balanced_horizontal_panel_insets(
                    window,
                    [title, help, camera_panel(size)],
                    12.0,
                );
            if index == 0 {
                app.world_mut().trigger(FrameHomeFace(HomeFace::Front));
            }
            app.update();
            assert_eq!(
                app.world()
                    .get::<CurrentFitTarget>(camera)
                    .map(|target| target.0),
                Some(home)
            );
            assert!(app.world().get::<CameraHomeTarget>(printed_area).is_some());
            let height = screen_height(&app)?;
            if index == 0 {
                first_height = height;
            } else {
                assert!((height / first_height - 1.0).abs() <= 0.01);
            }
        }
        Ok(())
    }

    #[test]
    fn reset_after_away_inset_change_fits_the_current_viewport() -> TestResult {
        let mut app = refit_test_app();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        app.add_observer(on_camera_homed);
        let camera = spawn_free_fit_camera(&mut app);
        spawn_home_cube(&mut app);
        spawn_home_target(&mut app);
        write_window_resized(&mut app);
        app.update();
        let old_home = free_home_pose(&app, camera)?;

        *app.world_mut().resource_mut::<AtHome>() = AtHome::No;
        app.world_mut().resource_mut::<HomeViewportInsets>().0 =
            FitViewportInsets::new(260.0, 0.0, 0.0, 0.0);
        app.update();
        assert_eq!(free_home_pose(&app, camera)?, old_home);

        app.world_mut().trigger(ResetFreeCamToHome { camera });
        app.world_mut().flush();
        for _ in 0..50 {
            app.update();
        }
        let new_home = free_home_pose(&app, camera)?;
        let mut expected_app = refit_test_app();
        let expected_camera = spawn_free_fit_camera(&mut expected_app);
        spawn_home_cube(&mut expected_app);
        spawn_home_target(&mut expected_app);
        expected_app
            .world_mut()
            .resource_mut::<HomeViewportInsets>()
            .0 = FitViewportInsets::new(260.0, 0.0, 0.0, 0.0);
        expected_app.update();
        assert_ne!(new_home, old_home);
        assert_eq!(new_home, free_home_pose(&expected_app, expected_camera)?);
        Ok(())
    }

    #[test]
    fn transient_screen_panel_does_not_refit_home_camera() -> TestResult {
        let mut app = refit_test_app();
        let camera = spawn_free_fit_camera(&mut app);
        spawn_home_cube(&mut app);
        spawn_home_target(&mut app);
        write_window_resized(&mut app);
        app.update();
        let old_home = free_home_pose(&app, camera)?;
        let mut transient = DiegeticPanel::screen()
            .size(Px(170.0), Px(60.0))
            .anchor(Anchor::CenterLeft)
            .build()
            .map_err(|_| "transient screen panel should build")?;
        assert!(transient.set_screen_position(Vec2::new(0.0, 200.0)));
        assert!(!reserves_home_space(
            &transient,
            EdgePanelManagement::Unmanaged
        ));
        let window_size = Vec2::new(f32::from(RESIZED_WIDTH), f32::from(RESIZED_HEIGHT));
        let tooltip = CoveringPanel {
            anchor:   Anchor::CenterLeft,
            top_left: Vec2::new(0.0, 200.0),
            size:     Vec2::new(170.0, 60.0),
        };
        assert_ne!(
            panel_insets::covering_panel_insets(window_size, [tooltip]),
            FitViewportInsets::NONE
        );
        let insets = panel_insets::covering_panel_insets(
            window_size,
            std::iter::once(tooltip)
                .filter(|_| reserves_home_space(&transient, EdgePanelManagement::Unmanaged)),
        );
        assert_eq!(insets, FitViewportInsets::NONE);
        app.world_mut().resource_mut::<HomeViewportInsets>().0 = insets;
        app.update();
        assert_eq!(free_home_pose(&app, camera)?, old_home);
        Ok(())
    }

    fn insets_for_placed_panel(
        placement: PanelPlacement,
        visibility: Visibility,
        inherited: InheritedVisibility,
    ) -> Result<FitViewportInsets, &'static str> {
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HomeViewportInsets>()
            .add_systems(Update, update_home_viewport_insets);
        app.world_mut().spawn((
            Window {
                resolution: (800_u32, 600_u32).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        let mut panel = DiegeticPanel::screen()
            .size(Px(170.0), Px(60.0))
            .anchor(Anchor::CenterLeft)
            .build()
            .map_err(|_| "screen panel should build")?;
        panel.set_width(170.0);
        panel.set_height(60.0);
        // The placement record is authoritative even if panel coordinates differ.
        if !panel.set_screen_position(Vec2::new(200.0, 230.0)) {
            return Err("screen panel should move");
        }
        app.world_mut().spawn((
            panel,
            visibility,
            inherited,
            EdgePanelPlacementRecord { placement },
        ));
        app.update();
        Ok(app.world().resource::<HomeViewportInsets>().0)
    }

    #[test]
    fn moved_edge_panel_reserves_its_placed_rectangle_for_home() -> TestResult {
        let insets = insets_for_placed_panel(
            PanelPlacement::Moved {
                top_left: Vec2::new(0.0, 200.0),
            },
            Visibility::Inherited,
            InheritedVisibility::VISIBLE,
        )?;
        assert_eq!(
            insets,
            FitViewportInsets::new(170.0 + HOME_PANEL_GAP, 0.0, HOME_PANEL_GAP, 0.0)
        );
        Ok(())
    }

    #[test]
    fn hidden_edge_panel_reserves_no_home_space() -> TestResult {
        let insets = insets_for_placed_panel(
            PanelPlacement::Hidden,
            Visibility::Inherited,
            InheritedVisibility::VISIBLE,
        )?;
        assert_eq!(insets, FitViewportInsets::NONE);
        Ok(())
    }

    #[test]
    fn hidden_edge_panel_needs_home_placement_to_reserve_space() -> TestResult {
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HomeViewportInsets>()
            .add_systems(Update, update_home_viewport_insets);
        app.world_mut().spawn((
            Window {
                resolution: (800_u32, 600_u32).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        let mut panel = DiegeticPanel::screen()
            .size(Px(170.0), Px(60.0))
            .anchor(Anchor::CenterLeft)
            .build()
            .map_err(|_| "screen panel should build")?;
        panel.set_width(170.0);
        panel.set_height(60.0);
        let edge = app
            .world_mut()
            .spawn((
                panel,
                Visibility::Hidden,
                InheritedVisibility::VISIBLE,
                EdgePanelPlacementRecord {
                    placement: PanelPlacement::AtAnchor,
                },
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().resource::<HomeViewportInsets>().0,
            FitViewportInsets::NONE
        );

        app.world_mut()
            .entity_mut(edge)
            .insert(EdgePanelHomePlacement(PanelPlacement::AtAnchor, 0.0));
        app.update();
        assert_eq!(
            app.world().resource::<HomeViewportInsets>().0,
            anchored_left_panel_home_reservation(170.0)
        );
        Ok(())
    }

    #[test]
    fn revealed_edge_panel_reserves_home_space_before_visibility_propagates() -> TestResult {
        let insets = insets_for_placed_panel(
            PanelPlacement::Moved {
                top_left: Vec2::new(0.0, 200.0),
            },
            Visibility::Inherited,
            InheritedVisibility::HIDDEN,
        )?;
        assert_eq!(
            insets,
            FitViewportInsets::new(170.0 + HOME_PANEL_GAP, 0.0, HOME_PANEL_GAP, 0.0)
        );
        Ok(())
    }

    #[test]
    fn separation_reveal_updates_home_insets_in_the_same_frame() -> TestResult {
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .init_resource::<HomeViewportInsets>();
        crate::screen_panels::install_overlay_picking(&mut app);
        app.add_systems(
            Update,
            update_home_viewport_insets.after(PanelSystems::PositionScreenSpace),
        );
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: (150_u32, 100_u32).into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let mut panel = DiegeticPanel::screen()
            .size(Px(180.0), Px(90.0))
            .anchor(Anchor::CenterLeft)
            .build()
            .map_err(|_| "screen panel should build")?;
        panel.set_width(180.0);
        panel.set_height(90.0);
        let entity = app
            .world_mut()
            .spawn((panel, Visibility::Inherited, InheritedVisibility::VISIBLE))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(entity),
            Some(&Visibility::Hidden)
        );
        app.world_mut()
            .entity_mut(entity)
            .insert(InheritedVisibility::HIDDEN);
        app.world_mut()
            .get_mut::<Window>(window)
            .ok_or("window missing")?
            .resolution
            .set(300.0, 130.0);
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(entity),
            Some(&Visibility::Inherited)
        );
        assert_eq!(
            app.world().resource::<HomeViewportInsets>().0,
            anchored_left_panel_home_reservation(180.0)
        );
        Ok(())
    }

    #[test]
    fn explicit_visibility_reserves_home_space_despite_hidden_inherited_visibility() -> TestResult {
        let placement = PanelPlacement::Moved {
            top_left: Vec2::new(0.0, 200.0),
        };
        let explicit =
            insets_for_placed_panel(placement, Visibility::Visible, InheritedVisibility::HIDDEN)?;
        let inherited = insets_for_placed_panel(
            placement,
            Visibility::Inherited,
            InheritedVisibility::HIDDEN,
        )?;
        assert_eq!(
            explicit,
            FitViewportInsets::new(170.0 + HOME_PANEL_GAP, 0.0, HOME_PANEL_GAP, 0.0)
        );
        assert_eq!(
            inherited,
            FitViewportInsets::new(170.0 + HOME_PANEL_GAP, 0.0, HOME_PANEL_GAP, 0.0)
        );
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
        let mut app = crate::test_app();
        app.add_plugins(MinimalPlugins)
            .init_resource::<AtHome>()
            .init_resource::<FramedHomeFace>()
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
