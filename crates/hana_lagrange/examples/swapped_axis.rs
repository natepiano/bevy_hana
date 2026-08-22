//! Demonstrates re-pointing an `OrbitCam` between coordinate-system conventions
//! at runtime: swapping the camera's `CameraBasis` per engine, animating the camera with
//! `PlayAnimation([CameraMove::try_to_orbital_look_at(..)?])`, and chaining a two-leg camera
//! animation by observing `AnimationEnd` to fire the second leg once the first lands.
//! `UpsideDownPolicy::Allow` removes the ±90° pitch clamp so orbit spins
//! freely; `FairyDustOrbitCam` tags the camera so the shared control panel
//! and home logic find it.
//!
//! Visually: one labeled ±X/±Y/±Z gizmo reorients to match how different
//! engines lay out their world axes on screen. Number keys `1`–`4` pick an
//! engine; the camera reframes to that engine's home view (its up-axis
//! vertical, orbiting around that up-axis) while the gizmo's six axis labels
//! orbit across the surrounding sphere to their new world directions and
//! settle there. A left-handed engine swaps a pair of endpoints, which a
//! rigid rotation can't reach, so each axis line is animated on its own
//! great-circle arc rather than as one rigid frame — the arms keep full
//! length and nothing retracts through the center.
//!
//! Arms are immediate-mode gizmos; the letters are unlit billboarded world
//! text. The bottom-left panel lists each engine with its up-axis / forward /
//! handedness.
//!
//! Controls:
//!   1 Bevy · 2 Blender · 3 Unity · 4 Unreal (re-press to re-home that engine)
//!   H - reset to the startup (Bevy) home

use std::f32::consts::PI;
use std::time::Duration;

use bevy::math::curve::easing::EaseFunction;
use bevy::prelude::Alpha;
use bevy::prelude::Assets;
use bevy::prelude::ChildSpawnerCommands;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Cuboid;
use bevy::prelude::DetectChanges;
use bevy::prelude::Entity;
use bevy::prelude::Gizmos;
use bevy::prelude::GlobalTransform;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::KeyCode;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Name;
use bevy::prelude::On;
use bevy::prelude::PostStartup;
use bevy::prelude::Quat;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Single;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Startup;
use bevy::prelude::Time;
use bevy::prelude::Transform;
use bevy::prelude::Update;
use bevy::prelude::Vec3;
use bevy::prelude::Visibility;
use bevy::prelude::With;
use bevy::prelude::error;
use bevy::prelude::warn;
use fairy_dust::CameraHomeTarget;
use fairy_dust::DEFAULT_PANEL_BACKGROUND;
use fairy_dust::FairyDustOrbitCam;
use fairy_dust::LABEL_SIZE;
use fairy_dust::TITLE_COLOR;
use fairy_dust::TitleBar;
use fairy_dust::screen_panel_frame;
use fairy_dust::screen_panel_material_handle;
use hana_diegetic::Anchor;
use hana_diegetic::DiegeticPanel;
use hana_diegetic::DiegeticPanelCommands;
use hana_diegetic::DiegeticText;
use hana_diegetic::El;
use hana_diegetic::Fit;
use hana_diegetic::LayoutBuilder;
use hana_diegetic::LayoutTree;
use hana_diegetic::Px;
use hana_diegetic::Sizing;
use hana_diegetic::StableTransparency;
use hana_diegetic::TextStyle;
use hana_lagrange::AnimationEnd;
use hana_lagrange::AnimationReason;
use hana_lagrange::CameraBasis;
use hana_lagrange::CameraHomed;
use hana_lagrange::CameraMove;
use hana_lagrange::CameraMoveError;
use hana_lagrange::Focus;
use hana_lagrange::FreeCamRollTarget;
use hana_lagrange::OrbitAngles;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamInputMode;
use hana_lagrange::OrbitCamPreset;
use hana_lagrange::PlayAnimation;
use hana_lagrange::Radius;
use hana_lagrange::UpsideDownPolicy;

const EXAMPLE_TITLE: &str = "Swapped Axis";

fn main() {
    fairy_dust::sprinkle_example()
        .with_brp_extras()
        .with_save_window_position()
        // Studio lighting shades the one PBR object in the scene — the ground.
        .with_studio_lighting()
        // The fairy_dust ground plane, sized to span the gizmo arms. It spawns flat
        // in the world XZ plane, which is the default engine's (Bevy, Y-up) floor;
        // `GroundPlane` tags it so `orient_ground` reorients it on a Y-up ↔ Z-up
        // switch.
        .with_ground_plane()
        .size(2.0 * AXIS_GIZMO_LENGTH)
        .insert(GroundPlane)
        .with_title_bar(
            TitleBar::new()
                .with_title(EXAMPLE_TITLE)
                .with_anchor(Anchor::TopLeft),
        )
        .with_camera_home()
        .yaw(HOME_YAW_Y_UP)
        .pitch(HOME_PITCH_Y_UP)
        .margin(HOME_FIT_MARGIN)
        .with_camera_control_panel()
        .init_resource::<Engine>()
        .init_resource::<PendingEngine>()
        .init_resource::<RequestedEngine>()
        // The camera animation's second leg fires from this observer when the first leg lands.
        .add_observer(advance_engine_animation)
        // `H` is bound by Fairy Dust's camera home; `reset_home_state` observes
        // the home-started event and resets the example's engine state to match.
        .add_observer(reset_home_state)
        .add_systems(Startup, (spawn_camera, spawn_gizmo, spawn_engine_panel))
        // After the ground spawns, fade its default material to mostly transparent.
        .add_systems(PostStartup, fade_ground)
        // 1 / 2 / 3 / 4 record an engine request through Fairy Dust's shortcut
        // binding, which fires each only when no modifier is held; `select_engine`
        // applies the request inside the chain below.
        .with_shortcut(KeyCode::Digit1, request_bevy)
        .with_shortcut(KeyCode::Digit2, request_blender)
        .with_shortcut(KeyCode::Digit3, request_unity)
        .with_shortcut(KeyCode::Digit4, request_unreal)
        .add_systems(
            Update,
            (
                // Chained so the gizmo draws and billboards against the camera
                // pose this frame's input produced.
                (
                    select_engine,
                    drive_gizmo_motion,
                    orient_ground,
                    draw_axis_gizmo,
                    billboard_axis_labels,
                )
                    .chain(),
                refresh_engine_panel,
            ),
        )
        .run();
}

// ═════════════════════════════════════════════════════════════════════════════
// CAMERA — driving OrbitCam: per-engine `CameraBasis`, `PlayAnimation(
// [orbital CameraMove])` for the camera animation, and an `AnimationEnd` observer
// chaining the second leg. This is the part to read to learn how the camera is
// driven.
//
// How it works:
//   1. `spawn_camera` (Startup) spawns one `OrbitCam` already at the default engine's home:
//      `engine_camera_basis(engine)` picks the orbit basis (Y-up engines orbit about world Y; Z-up
//      engines about Z via `Z_UP_BASIS`); `yaw`/`pitch`/`radius` are seeded by
//      `OrbitCam::from_pose`, so initialization keeps them and skips any opening fit.
//      `UpsideDownPolicy::Allow` removes the pitch clamp; `FairyDustOrbitCam` tags the camera for
//      the shared panel and home logic.
//   2. Number keys 1–4 record a `RequestedEngine` through Fairy Dust's shortcut binding;
//      `select_engine` (Update) applies it. Re-pressing the current engine fires one
//      `PlayAnimation` straight back to its home view. A new engine starts a two-leg camera
//      animation: it records the switch in `PendingEngine`, seeds the gizmo arc tweens, and fires
//      the first `PlayAnimation` to `yaw=0, pitch=0` — the shared front pose where every basis
//      renders identically, so the upcoming axis swap is invisible.
//   3. `advance_engine_animation` (observer on `AnimationEnd`) wakes when the first leg lands. It
//      pulls the switch out of `PendingEngine`, rotates the orbit focus through
//      `engine_up_world(from) → engine_up_world(to)` so the floor keeps its screen-space footprint,
//      swaps `CameraBasis` to the new basis, and fires the second `PlayAnimation` out to the new
//      engine's home angles. Cancelled or second-leg `AnimationEnd`s short-circuit because
//      `PendingEngine` is already empty.
//   4. `reset_home_state` (observer on `CameraHomed`) wakes when Fairy Dust's home input fires,
//      resets the resource state, swaps `CameraBasis` back to the default engine's basis, and
//      re-seeds gizmo + ground tweens. Lagrange glides the camera to the stored home pose; this
//      only realigns the example's state to match.
// ═════════════════════════════════════════════════════════════════════════════

// The Z-up engines' (Blender, Unreal) camera basis: a right-handed Z-up frame
// (right=X, up=Z, back=-Y). Using `-Y` for back keeps the determinant
// positive, so tilting to the Y-up basis `[X, Y, Z]` is a clean 90° rotation
// about X rather than a handedness flip the camera can't interpolate through.
const Z_UP_BASIS: CameraBasis = CameraBasis::Z_UP;
// Distance the camera orbits the origin-centered gizmo at — frames the arms and
// labels with a little margin.
const HOME_RADIUS: f32 = 6.5;
const HOME_FIT_MARGIN: f32 = 0.1;

// Home `(yaw, pitch)` for the Y-up engines (Bevy, Unity) on the `[X, Y, Z]` orbit
// basis — a 3/4 view with world Y standing vertical, framed to present the ground
// identically to the Z-up engines so every engine reads the same.
const HOME_YAW_Y_UP: f32 = 0.365;
const HOME_PITCH_Y_UP: f32 = 0.288;

// Home `(yaw, pitch)` for the Z-up engines (Blender, Unreal) on `Z_UP_BASIS`.
// This is the Y-up home view rotated with the floor onto Blender's Z-up plane,
// preserving the camera's screen-space relationship to the ground.
const HOME_YAW: f32 = HOME_YAW_Y_UP;
const HOME_PITCH: f32 = HOME_PITCH_Y_UP - PI / 2.0;

// Camera animation on engine switch — swing to the shared front pose (where the orbit
// basis swap is invisible), then out to the new engine's home view.
const CAMERA_TO_FRONT_MS: u64 = 350;
const CAMERA_TO_HOME_MS: u64 = 450;

/// The selected engine. Drives the bottom-left selector highlight and the camera
/// + gizmo targets that the number keys (and `H`) animate to.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Engine {
    #[default]
    Bevy,
    Blender,
    Unity,
    Unreal,
}

/// One row of the engine selector: the engine, then the name / up-axis /
/// forward / handedness shown one per panel column. Single source of truth for
/// the panel columns; the keys that pick each engine are bound in `main`.
const ENGINES: [(Engine, &str, &str, &str, &str); 4] = [
    (Engine::Bevy, "1 Bevy", "Y-up", "+Z toward", "right-handed"),
    (
        Engine::Blender,
        "2 Blender",
        "Z-up",
        "+Y away",
        "right-handed",
    ),
    (Engine::Unity, "3 Unity", "Y-up", "+Z away", "left-handed"),
    (Engine::Unreal, "4 Unreal", "Z-up", "+X away", "left-handed"),
];

impl Engine {
    /// Whether this engine stands `+Y` up (vs `+Z`). Selects the orbit basis in
    /// [`engine_camera_basis`] and the home angles in [`home_angles`](Self::home_angles).
    const fn standing_axis_is_y(self) -> bool { matches!(self, Self::Bevy | Self::Unity) }

    /// The camera's home `(yaw, pitch)` under this engine's orbit axis — paired
    /// with [`engine_camera_basis`] so the engine's up-axis stands vertical. Both
    /// Y-up engines share one 3/4 view and both Z-up engines share the baked
    /// view; the gizmo's per-engine endpoint swap, not the camera, distinguishes
    /// the two handednesses that share an up-axis.
    const fn home_angles(self) -> (f32, f32) {
        if self.standing_axis_is_y() {
            (HOME_YAW_Y_UP, HOME_PITCH_Y_UP)
        } else {
            (HOME_YAW, HOME_PITCH)
        }
    }
}

/// The engine switch in progress, held while the camera swings through the
/// shared front pose.
#[derive(Resource, Default)]
struct PendingEngine(Option<EngineSwitch>);

/// Source and target engines for one in-flight switch.
struct EngineSwitch {
    from: Engine,
    to:   Engine,
}

/// Spawns the one `OrbitCam` for the scene, already at the default engine's home.
fn spawn_camera(mut commands: Commands) {
    // Open directly at the default engine's home. Setting `yaw`/`pitch`/`radius`
    // to `Some` makes `OrbitCam` initialization keep them and build the transform
    // from them, so the scene opens at that view with no fit or snap. Tagging the
    // entity with `FairyDustOrbitCam` lets the camera control panel and the engine
    // animation find it. `UpsideDownPolicy::Allow` drops the ±90° pitch clamp so orbit
    // spins freely; `StableTransparency` enables OIT for the world-text labels.
    let engine = Engine::default();
    let (yaw, pitch) = engine.home_angles();
    let basis = engine_camera_basis(engine);
    let mut camera = OrbitCam::from_pose(Vec3::ZERO, (yaw, pitch), HOME_RADIUS);
    camera.upside_down_policy = UpsideDownPolicy::Allow;
    commands.spawn((
        camera,
        basis,
        OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
        FairyDustOrbitCam,
        StableTransparency,
    ));
}

/// The right-handed camera basis (`right`, `up`, `back`) the camera uses for
/// `engine`: Y-up engines orbit about world `Y`, Z-up engines about world `Z`.
/// Only the standing axis is matched — the horizontal orbit direction stays the
/// same for every engine regardless of handedness. `basis.up` is the axis a
/// drag-to-orbit revolves around, so this is what makes orbiting "feel" like the
/// engine, not just the framing.
const fn engine_camera_basis(engine: Engine) -> CameraBasis {
    if engine.standing_axis_is_y() {
        CameraBasis::Y_UP
    } else {
        Z_UP_BASIS
    }
}

/// Resets the example-owned state when Lagrange handles `H Home`. The camera
/// glides to the stored home pose; this observer resets the engine, gizmo, and
/// ground to the default engine to match it, without firing another animation.
fn reset_home_state(
    _event: On<CameraHomed>,
    mut current: ResMut<Engine>,
    mut pending: ResMut<PendingEngine>,
    mut camera_basis: Query<&mut CameraBasis, With<FairyDustOrbitCam>>,
    mut gizmo: Query<&mut GizmoMotion, With<GizmoRoot>>,
    mut ground: Query<&mut Transform, With<GroundPlane>>,
) {
    let engine = Engine::default();
    *current = engine;
    pending.0 = None;

    if let Ok(mut basis) = camera_basis.single_mut() {
        *basis = engine_camera_basis(engine);
    }

    if let Ok(mut ground) = ground.single_mut() {
        ground.rotation = Quat::from_rotation_arc(Vec3::Y, engine_up_world(engine));
    }

    let Ok(mut motion) = gizmo.single_mut() else {
        return;
    };
    let targets = engine_axis_dirs(engine);
    let up = engine_up_world(engine);
    let motion = &mut *motion;
    for ((tween, &dir), &target) in motion.tweens.iter_mut().zip(&motion.dirs).zip(&targets) {
        *tween = Some(axis_tween(dir, target, up, Vec3::X));
    }
}

/// An engine the number-key shortcuts have asked for but `select_engine` has
/// not yet applied. Set by `request_bevy` and friends, consumed once per frame.
#[derive(Resource, Default)]
struct RequestedEngine(Option<Engine>);

fn request_bevy(mut requested: ResMut<RequestedEngine>) { requested.0 = Some(Engine::Bevy); }
fn request_blender(mut requested: ResMut<RequestedEngine>) { requested.0 = Some(Engine::Blender); }
fn request_unity(mut requested: ResMut<RequestedEngine>) { requested.0 = Some(Engine::Unity); }
fn request_unreal(mut requested: ResMut<RequestedEngine>) { requested.0 = Some(Engine::Unreal); }

/// Applies a pending engine request (recorded by the number-key shortcuts) and
/// starts the camera moving. Re-pressing the current engine re-homes it directly;
/// switching to a new one routes through the shared front pose so the orbit-axis
/// swap stays invisible.
fn select_engine(
    mut requested: ResMut<RequestedEngine>,
    mut current: ResMut<Engine>,
    mut pending: ResMut<PendingEngine>,
    camera: Query<(Entity, &OrbitCam), With<FairyDustOrbitCam>>,
    mut gizmo: Query<&mut GizmoMotion, With<GizmoRoot>>,
    mut commands: Commands,
) {
    let Some(requested) = requested.0.take() else {
        return;
    };
    let Ok((entity, orbit)) = camera.single() else {
        return;
    };

    // Re-pressing the current engine's key re-homes it: the orbit axis and gizmo
    // are already correct, so skip the front-route axis swap and animate the camera
    // straight back to this engine's home view from wherever it was dragged.
    if *current == requested {
        let (yaw, pitch) = requested.home_angles();
        let Ok(home_move) = smooth_orbital_move(
            orbit.pan.current().0,
            yaw,
            pitch,
            orbit.zoom.current().0,
            Duration::from_millis(CAMERA_TO_HOME_MS),
        ) else {
            return;
        };
        commands.trigger(PlayAnimation::new(entity, [home_move]));
        return;
    }

    let Ok(mut motion) = gizmo.single_mut() else {
        return;
    };
    pending.0 = Some(EngineSwitch {
        from: *current,
        to:   requested,
    });
    *current = requested;

    // Orbit each labeled axis to its new world direction. A handedness swap flips
    // one axis 180°; `axis_tween` arcs it about the up axis (or X if it is itself
    // the up axis) so endpoints swap over the top rather than collapsing.
    let targets = engine_axis_dirs(requested);
    let up = engine_up_world(requested);
    // Reborrow so the `tweens`/`dirs` field iterators are disjoint borrows.
    let motion = &mut *motion;
    for ((tween, &dir), &target) in motion.tweens.iter_mut().zip(&motion.dirs).zip(&targets) {
        *tween = Some(axis_tween(dir, target, up, Vec3::X));
    }

    // Animate to the shared front pose under the current axis; [`advance_engine_animation`]
    // swaps the orbit basis there (invisible) and animates to the engine home. Using
    // the library's animation keeps smoothing handled and the controller's state
    // consistent, so the next drag continues cleanly.
    let Ok(front_move) = smooth_orbital_move(
        orbit.pan.current().0,
        0.0,
        0.0,
        orbit.zoom.current().0,
        Duration::from_millis(CAMERA_TO_FRONT_MS),
    ) else {
        return;
    };
    commands.trigger(PlayAnimation::new(entity, [front_move]));
}

/// Second leg of an engine switch. On reaching the front pose, swaps the orbit
/// basis to the pending engine — visually a no-op, since every basis renders
/// identically at `yaw=0, pitch=0` — then animates the camera out to that engine's
/// home view. The second leg's own completion is skipped (the pending engine is
/// already taken), as is any cancelled animation.
fn advance_engine_animation(
    event: On<AnimationEnd>,
    mut pending: ResMut<PendingEngine>,
    mut camera: Query<(Entity, &OrbitCam, &mut CameraBasis), With<FairyDustOrbitCam>>,
    mut commands: Commands,
) {
    if !matches!(event.reason, AnimationReason::Completed) {
        return;
    }
    let Some(engine_switch) = pending.0.take() else {
        return;
    };
    let Ok((entity, orbit, mut basis)) = camera.single_mut() else {
        return;
    };
    let engine = engine_switch.to;
    let focus = rotate_focus_between_engines(orbit.pan.current().0, engine_switch.from, engine);
    *basis = engine_camera_basis(engine);
    let (yaw, pitch) = engine.home_angles();
    let Ok(home_move) = smooth_orbital_move(
        focus,
        yaw,
        pitch,
        orbit.zoom.current().0,
        Duration::from_millis(CAMERA_TO_HOME_MS),
    ) else {
        return;
    };
    commands.trigger(PlayAnimation::new(entity, [home_move]));
}

/// Builds one smoothed orbital move, warning and reporting the rejection when
/// the authored pose is rejected.
fn smooth_orbital_move(
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    radius: f32,
    duration: Duration,
) -> Result<CameraMove, CameraMoveError> {
    CameraMove::try_to_orbital_look_at(
        Focus(focus),
        OrbitAngles { yaw, pitch },
        Radius(radius),
        FreeCamRollTarget::InheritPrevious,
        duration,
        EaseFunction::SmoothStep,
    )
    .inspect_err(|error: &CameraMoveError| warn!("camera move rejected: {error}"))
}

/// Rotates the camera fit focus with the floor when switching between Y-up and
/// Z-up engines so the ground plane keeps the same screen-space footprint.
fn rotate_focus_between_engines(focus: Vec3, from: Engine, to: Engine) -> Vec3 {
    Quat::from_rotation_arc(engine_up_world(from), engine_up_world(to)) * focus
}

// ═════════════════════════════════════════════════════════════════════════════
// GROUND PLANE — a mostly-transparent floor kept coplanar with the two gizmo arms
// that define it (everything but the engine's up-axis), so it turns with them on
// an engine switch. Scene scaffolding; tracks the gizmo, not the world axes.
// ═════════════════════════════════════════════════════════════════════════════

// How transparent the floor is (the fairy_dust default, 0.78, is fairly solid).
const GROUND_ALPHA: f32 = 0.3;
// Floor reorientation speed when the up-axis changes (Y-up ↔ Z-up). Tuned to
// roughly track the camera animation so the floor finishes turning as the camera lands.
const GROUND_TURN_RATE: f32 = 4.0;

/// Tags the `fairy_dust` ground plane so it can be faded and reoriented per engine.
#[derive(Component)]
struct GroundPlane;

/// Lowers the ground's default material alpha to mostly transparent, once the
/// plane has spawned. The default material is otherwise left intact.
fn fade_ground(
    ground: Query<&MeshMaterial3d<StandardMaterial>, With<GroundPlane>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(material) = ground.single() else {
        return;
    };
    if let Some(mut material) = materials.get_mut(&material.0) {
        material.base_color = material.base_color.with_alpha(GROUND_ALPHA);
    }
}

/// Eases the floor toward perpendicular-to-`engine`'s world up axis. The two
/// ground axes lie along the world axes spanning that plane, so the floor is a
/// real horizontal surface coplanar with them. Only Y-up ↔ Z-up switches move it;
/// same-up handedness swaps leave the up axis (and floor) put.
fn orient_ground(
    time: Res<Time>,
    engine: Res<Engine>,
    mut ground: Query<&mut Transform, With<GroundPlane>>,
) {
    let Ok(mut transform) = ground.single_mut() else {
        return;
    };
    let target = Quat::from_rotation_arc(Vec3::Y, engine_up_world(*engine));
    let t = 1.0 - (-GROUND_TURN_RATE * time.delta_secs()).exp();
    transform.rotation = transform.rotation.slerp(target, t);
}

// ═════════════════════════════════════════════════════════════════════════════
// ENGINE SELECTOR PANEL — the bottom-left diegetic table listing each engine's
// up-axis / forward / handedness, with the selected row highlighted. Presentation
// only; the camera works without it.
// ═════════════════════════════════════════════════════════════════════════════

const PANEL_COLUMN_GAP: Px = Px(24.0);
const PANEL_ROW_GAP: Px = Px(4.0);
const PANEL_ACTIVE_COLOR: Color = Color::srgb(1.0, 0.9, 0.25);
const PANEL_INACTIVE_COLOR: Color = Color::srgba(0.68, 0.72, 0.82, 0.9);

/// Marker for the bottom-left engine selector panel.
#[derive(Component)]
struct EnginePanel;

/// The three text styles the panel draws with: a column header, an active
/// (highlighted) chip, and an inactive chip.
struct ColumnStyles {
    header:   TextStyle,
    active:   TextStyle,
    inactive: TextStyle,
}

fn spawn_engine_panel(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    engine: Res<Engine>,
) {
    let unlit = screen_panel_material_handle(&mut materials);
    let panel = DiegeticPanel::screen()
        .size(Fit, Fit)
        .anchor(Anchor::BottomLeft)
        .material(unlit.clone())
        .text_material(unlit)
        .with_tree(build_engine_tree(*engine))
        .build();

    match panel {
        Ok(panel) => {
            commands.spawn((EnginePanel, panel, Transform::default()));
        },
        Err(error) => {
            error!("swapped_axis: failed to build engine panel: {error}");
        },
    }
}

/// Repaints the panel whenever the selected engine changes so the active row
/// tracks the live state.
fn refresh_engine_panel(
    engine: Res<Engine>,
    panel: Single<Entity, With<EnginePanel>>,
    mut commands: Commands,
) {
    if !engine.is_changed() {
        return;
    }
    if let Err(error) = commands.set_tree(*panel, build_engine_tree(*engine)) {
        warn!("failed to replace engine panel tree: {error}");
    }
}

fn build_engine_tree(selected: Engine) -> LayoutTree {
    let mut builder = LayoutBuilder::with_root(El::new().width(Sizing::FIT).height(Sizing::FIT));
    build_engine_layout(&mut builder, selected);
    builder.build()
}

fn build_engine_layout(builder: &mut LayoutBuilder, selected: Engine) {
    let styles = ColumnStyles {
        header:   TextStyle::new(LABEL_SIZE).with_color(TITLE_COLOR),
        active:   TextStyle::new(LABEL_SIZE).with_color(PANEL_ACTIVE_COLOR),
        inactive: TextStyle::new(LABEL_SIZE).with_color(PANEL_INACTIVE_COLOR),
    };
    screen_panel_frame(
        builder,
        Sizing::FIT,
        Sizing::FIT,
        DEFAULT_PANEL_BACKGROUND,
        |builder| {
            builder.with(
                El::row()
                    .width(Sizing::FIT)
                    .height(Sizing::FIT)
                    .gap(PANEL_COLUMN_GAP),
                |builder| {
                    build_column(
                        builder,
                        "ENGINE",
                        ENGINES
                            .into_iter()
                            .map(|(engine, name, ..)| (name, engine == selected)),
                        &styles,
                    );
                    build_column(
                        builder,
                        "UP",
                        ENGINES
                            .into_iter()
                            .map(|(engine, _, up, ..)| (up, engine == selected)),
                        &styles,
                    );
                    build_column(
                        builder,
                        "FORWARD",
                        ENGINES
                            .into_iter()
                            .map(|(engine, _, _, forward, _)| (forward, engine == selected)),
                        &styles,
                    );
                    build_column(
                        builder,
                        "HANDED",
                        ENGINES
                            .into_iter()
                            .map(|(engine, _, _, _, handed)| (handed, engine == selected)),
                        &styles,
                    );
                },
            );
        },
    );
}

/// Draws one labeled column: a header followed by one chip per row, each chip
/// using the active style when its `bool` is set.
fn build_column<'a>(
    builder: &mut LayoutBuilder,
    header: &str,
    rows: impl IntoIterator<Item = (&'a str, bool)>,
    styles: &ColumnStyles,
) {
    builder.with(
        El::column()
            .width(Sizing::FIT)
            .height(Sizing::FIT)
            .gap(PANEL_ROW_GAP),
        |builder| {
            builder.text((header, styles.header.clone()));
            for (label, active) in rows {
                let style = if active {
                    &styles.active
                } else {
                    &styles.inactive
                };
                builder.text((label, style.clone()));
            }
        },
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// AXIS GIZMO ANIMATION — the labeled ±X/±Y/±Z arms and the great-circle arcs the
// labels orbit along on an engine switch. Decorative; independent of the camera.
// ═════════════════════════════════════════════════════════════════════════════

const AXIS_GIZMO_LENGTH: f32 = 2.0;
const AXIS_X_COLOR: Color = Color::srgb(0.90, 0.20, 0.20);
const AXIS_X_INDEX: usize = 0;
const AXIS_X_NEGATIVE_LABEL: &str = "-x";
const AXIS_X_POSITIVE_LABEL: &str = "+x";
const AXIS_Y_COLOR: Color = Color::srgb(0.20, 0.80, 0.25);
const AXIS_Y_INDEX: usize = 1;
const AXIS_Y_NEGATIVE_LABEL: &str = "-y";
const AXIS_Y_POSITIVE_LABEL: &str = "+y";
const AXIS_Z_COLOR: Color = Color::srgb(0.30, 0.45, 0.95);
const AXIS_Z_INDEX: usize = 2;
const AXIS_Z_NEGATIVE_LABEL: &str = "-z";
const AXIS_Z_POSITIVE_LABEL: &str = "+z";

// Axis labels — billboarded world text just past each arrow tip.
const AXIS_LABEL_SIZE: f32 = 0.11;
const AXIS_LABEL_OFFSET: f32 = 0.13;
// Labels only lift clear once their arm points this much toward/away from the
// camera; below the threshold they stay colinear with the arrow.
const LABEL_OCCLUSION_THRESHOLD: f32 = 0.6;
// Max screen-vertical nudge once a label is fully occluded (down for the one in
// front, up for the one behind).
const LABEL_DEPTH_LIFT: f32 = 0.15;
const HOME_TARGET_HALF_EXTENT: f32 = AXIS_GIZMO_LENGTH + AXIS_LABEL_OFFSET + AXIS_LABEL_SIZE * 0.5;
const HOME_TARGET_NAME: &str = "Swapped axis home bounds";
const HOME_TARGET_SIZE: f32 = HOME_TARGET_HALF_EXTENT * 2.0;

// Transition — each axis line orbits to its target over this long.
const STEP_DURATION: f32 = 0.6;

/// Parent container for the gizmo labels. Stays at the world origin with an
/// identity transform; the axis directions live in [`GizmoMotion`].
#[derive(Component)]
struct GizmoRoot;

/// The live world direction of each axis line's `+` endpoint (`+x`, `+y`, `+z`),
/// plus the per-axis arc tween currently moving it. The `-` endpoint of each
/// axis is the negation, so the two stay antipodal and the arms never shorten.
#[derive(Component)]
struct GizmoMotion {
    dirs:   [Vec3; 3],
    tweens: [Option<AxisTween>; 3],
}

/// One ±axis world-text label. `axis` indexes [`GizmoMotion::dirs`]; `sign`
/// selects the `+` or `-` endpoint of that axis line.
#[derive(Component)]
struct AxisLabel {
    axis: usize,
    sign: f32,
}

/// Orbits one axis-line direction from `start` toward its target along a fixed
/// great-circle arc (`axis`, `angle`) over [`STEP_DURATION`].
#[derive(Clone, Copy)]
struct AxisTween {
    start:   Vec3,
    axis:    Vec3,
    angle:   f32,
    elapsed: f32,
}

impl AxisTween {
    /// Advances by `dt` and returns the eased direction plus whether it finished.
    fn advance(&mut self, dt: f32) -> (Vec3, bool) {
        self.elapsed += dt;
        let t = (self.elapsed / STEP_DURATION).clamp(0.0, 1.0);
        let eased = t * t * 2.0f32.mul_add(-t, 3.0);
        let dir = Quat::from_axis_angle(self.axis, self.angle * eased) * self.start;
        (dir, self.elapsed >= STEP_DURATION)
    }
}

fn spawn_gizmo(mut commands: Commands, engine: Res<Engine>, mut meshes: ResMut<Assets<Mesh>>) {
    // Open in the default engine's world-axis layout.
    let dirs = engine_axis_dirs(*engine);
    commands.spawn((
        Name::new(HOME_TARGET_NAME),
        CameraHomeTarget,
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(HOME_TARGET_SIZE)))),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands
        .spawn((
            GizmoRoot,
            GizmoMotion {
                dirs,
                tweens: [None, None, None],
            },
            Transform::default(),
            Visibility::default(),
        ))
        .with_children(|root| {
            for (axis, color, positive, negative) in [
                (
                    AXIS_X_INDEX,
                    AXIS_X_COLOR,
                    AXIS_X_POSITIVE_LABEL,
                    AXIS_X_NEGATIVE_LABEL,
                ),
                (
                    AXIS_Y_INDEX,
                    AXIS_Y_COLOR,
                    AXIS_Y_POSITIVE_LABEL,
                    AXIS_Y_NEGATIVE_LABEL,
                ),
                (
                    AXIS_Z_INDEX,
                    AXIS_Z_COLOR,
                    AXIS_Z_POSITIVE_LABEL,
                    AXIS_Z_NEGATIVE_LABEL,
                ),
            ] {
                spawn_label(root, axis, 1.0, color, positive);
                spawn_label(root, axis, -1.0, color, negative);
            }
        });
}

fn spawn_label(root: &mut ChildSpawnerCommands, axis: usize, sign: f32, color: Color, glyph: &str) {
    root.spawn((
        AxisLabel { axis, sign },
        DiegeticText::world(glyph)
            .size(AXIS_LABEL_SIZE)
            .color(color)
            .anchor(Anchor::Center)
            .unlit()
            .transform(Transform::default())
            .build(),
    ));
}

/// Advances each axis's arc tween, writing the eased direction back into
/// [`GizmoMotion::dirs`] and clearing the tween when it finishes.
fn drive_gizmo_motion(time: Res<Time>, mut gizmo: Query<&mut GizmoMotion, With<GizmoRoot>>) {
    let Ok(mut motion) = gizmo.single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    for axis in 0..3 {
        let Some((dir, finished)) = motion.tweens[axis].as_mut().map(|tween| tween.advance(dt))
        else {
            continue;
        };
        motion.dirs[axis] = dir;
        if finished {
            motion.tweens[axis] = None;
        }
    }
}

/// Draws the six arms as immediate-mode arrows from the origin along each axis's
/// `+` and `-` directions.
fn draw_axis_gizmo(
    mut gizmos: Gizmos,
    gizmo: Query<(&GlobalTransform, &GizmoMotion), With<GizmoRoot>>,
) {
    let Ok((root, motion)) = gizmo.single() else {
        return;
    };
    let origin = root.translation();
    for (axis, color) in [(0usize, AXIS_X_COLOR), (1, AXIS_Y_COLOR), (2, AXIS_Z_COLOR)] {
        let arm = motion.dirs[axis] * AXIS_GIZMO_LENGTH;
        gizmos.arrow(origin, origin + arm, color);
        gizmos.arrow(origin, origin - arm, color);
    }
}

/// Faces each label at the camera and nudges the toward/away ones clear of their
/// foreshortened arrows.
fn billboard_axis_labels(
    camera: Query<&GlobalTransform, With<OrbitCam>>,
    gizmo: Query<&GizmoMotion, With<GizmoRoot>>,
    mut labels: Query<(&AxisLabel, &mut Transform)>,
) {
    // The gizmo root sits at the origin with an identity transform, so a label's
    // local transform is its world transform. Face each label at the camera,
    // then slide toward/away labels along screen-up (down for the one in front,
    // up for the one behind) so they clear the foreshortened arrow.
    let Ok(camera) = camera.single() else {
        return;
    };
    let Ok(motion) = gizmo.single() else {
        return;
    };
    let billboard = camera.rotation();
    let screen_up = camera.rotation() * Vec3::Y;
    let to_camera = camera.translation().normalize_or_zero();
    for (label, mut transform) in &mut labels {
        let arm = label.sign * motion.dirs[label.axis];
        let world_dir = arm.normalize_or_zero();
        let depth = world_dir.dot(to_camera);
        // Ramp the lift in only past the occlusion threshold so colinear axes
        // keep their labels on the arrow.
        let engage = ((depth.abs() - LABEL_OCCLUSION_THRESHOLD)
            / (1.0 - LABEL_OCCLUSION_THRESHOLD))
            .clamp(0.0, 1.0);
        let world_offset = -screen_up * depth.signum() * engage * LABEL_DEPTH_LIFT;
        transform.translation = arm * (AXIS_GIZMO_LENGTH + AXIS_LABEL_OFFSET) + world_offset;
        transform.rotation = billboard;
    }
}

/// Builds the [`AxisTween`] that orbits `start` onto `target` along the shortest
/// great-circle arc. When the two are antipodal (an endpoint swap), the arc has
/// no unique plane, so it orbits about the vertical instead — or the camera's
/// right axis if `start` is itself vertical.
fn axis_tween(start: Vec3, target: Vec3, up: Vec3, camera_right: Vec3) -> AxisTween {
    let start = start.normalize_or_zero();
    let target = target.normalize_or_zero();
    let angle = start.dot(target).clamp(-1.0, 1.0).acos();
    let axis = if angle < 1.0e-4 {
        up
    } else if PI - angle < 1.0e-3 {
        let perpendicular = up - start * start.dot(up);
        if perpendicular.length_squared() > 1.0e-4 {
            perpendicular.normalize()
        } else {
            camera_right
        }
    } else {
        start.cross(target).normalize()
    };
    AxisTween {
        start,
        axis,
        angle,
        elapsed: 0.0,
    }
}

/// The world directions of the gizmo's `+x`/`+y`/`+z` arms under `engine`. The
/// up-labelled axis points along world up ([`engine_up_world`]) so the floor — the
/// plane of the other two — is a real horizontal surface the camera looks down on.
/// Right-handed engines keep the world basis; left-handed ones flip a single axis,
/// which the labels orbit through as an endpoint swap on a switch.
const fn engine_axis_dirs(engine: Engine) -> [Vec3; 3] {
    match engine {
        // Right-handed engines keep the world basis. Bevy and Blender share the
        // same arm directions; only which axis stands up differs (Bevy `+y`,
        // Blender `+z`), and that is carried by [`engine_up_world`].
        Engine::Bevy | Engine::Blender => [Vec3::X, Vec3::Y, Vec3::Z],
        // Left-handed engines flip one axis — the labels orbit through it as an
        // endpoint swap on a same-up switch. Unity flips `+Z` (its `+Z away` vs
        // Bevy's `+Z toward`); Unreal flips `+X`.
        Engine::Unity => [Vec3::X, Vec3::Y, Vec3::NEG_Z],
        Engine::Unreal => [Vec3::NEG_X, Vec3::Y, Vec3::Z],
    }
}

/// World up for `engine` — the axis its convention stands vertical, matching the
/// camera basis in [`engine_camera_basis`]. The floor's normal.
const fn engine_up_world(engine: Engine) -> Vec3 {
    if engine.standing_axis_is_y() {
        Vec3::Y
    } else {
        Vec3::Z
    }
}
