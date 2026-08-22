//! Crate-level constants shared by flat top-level modules.

use std::time::Duration;

use bevy::prelude::Color;
use bevy::prelude::GamepadButton;
use bevy::prelude::KeyCode;
use bevy::prelude::Vec3;
use hana_diegetic::Pt;
use hana_diegetic::Px;

// aabb
/// The 8 corner sign-patterns of a unit AABB, used by lighting and camera-home
/// systems to transform local bounds into world space through a `GlobalTransform`.
pub(crate) const AABB_CORNER_SIGNS: [Vec3; 8] = [
    Vec3::new(-1.0, -1.0, -1.0),
    Vec3::new(1.0, -1.0, -1.0),
    Vec3::new(-1.0, 1.0, -1.0),
    Vec3::new(1.0, 1.0, -1.0),
    Vec3::new(-1.0, -1.0, 1.0),
    Vec3::new(1.0, -1.0, 1.0),
    Vec3::new(-1.0, 1.0, 1.0),
    Vec3::new(1.0, 1.0, 1.0),
];

// bloom
pub(crate) const BLOOM_INTENSITY: f32 = 0.25;
/// Only pixels brighter than this (pre-tonemap luminance) contribute to bloom.
/// Lit colored text and mesh content peak above 1.0 under studio lighting, so the
/// threshold sits above them and below the over-bright emissive readout, which
/// is the only content meant to glow.
pub(crate) const BLOOM_THRESHOLD: f32 = 3.0;
pub(crate) const BLOOM_THRESHOLD_SOFTNESS: f32 = 0.2;

// camera home
/// Color of the home AABB wireframe drawn by `draw_home_aabb_gizmo` while
/// `HomeAabbGizmoVisible` is on. Bright orange so it reads against the usual
/// dark background.
pub(crate) const HOME_AABB_GIZMO_COLOR: Color = Color::srgb(1.0, 0.5, 0.0);
pub(crate) const HOME_CONTROL: &str = "H Home";
pub(crate) const HOME_DEFAULT_MARGIN: f32 = 0.15;
/// Keyboard binding filled into Lagrange keyboard-family presets when camera home is enabled.
pub(crate) const HOME_KEY: KeyCode = KeyCode::KeyH;
/// Gamepad binding filled into Lagrange gamepad presets when camera home is enabled.
pub(crate) const HOME_BUTTON: GamepadButton = GamepadButton::Select;
/// Minimum cube scale along any axis. Text and other planar geometry can give
/// a union with zero extent in one axis; the fit math handles a zero-extent
/// vertex cloud poorly, so floor each axis to a small positive value.
pub(crate) const MIN_HOME_CUBE_SCALE: f32 = 0.001;

// camera restart
pub(crate) const POSE_ENV: &str = "FAIRY_DUST_RESTART_CAMERA_POSE";
pub(crate) const POSE_FIELD_COUNT: usize = 6;
pub(crate) const POSE_FIELD_SEPARATOR: char = ',';
pub(crate) const RESTART_CAMERA_RESTORE_DURATION: Duration = Duration::from_secs(2);

// cargo restart
pub(crate) const CARGO_BIN: &str = "cargo";
pub(crate) const CARGO_EXAMPLES_DIR: &str = "examples";
pub(crate) const CARGO_EXAMPLE_FLAG: &str = "--example";
pub(crate) const CARGO_MANIFEST_PATH_FLAG: &str = "--manifest-path";
pub(crate) const CARGO_RELEASE_FLAG: &str = "--release";
pub(crate) const CARGO_RUN_SUBCOMMAND: &str = "run";
pub(crate) const EXIT_STATUS_FAILURE: i32 = 1;
#[cfg(windows)]
pub(crate) const EXIT_STATUS_SUCCESS: i32 = 0;

// cascade shadow
// An orthographic `OrbitCam` parks at a fixed `(near + far) / 2` distance that a
// small scene's radius-based fit can't reach, so the cascade also extends to
// cover the camera. This headroom keeps the scene's far edge inside the last
// cascade rather than exactly on its boundary; it must stay small enough that
// larger scenes (where the radius term already dominates) are left untouched.
pub(crate) const CASCADE_CAMERA_HEADROOM: f32 = 1.2;
// Matches Bevy's non-webgl default. The auto-fit only narrows the cascade
// distances; the count must stay constant after spawn, because changing the
// number of cascades on a live light desyncs `bevy_light`'s per-cascade
// visibility queues and panics.
pub(crate) const CASCADE_COUNT: usize = 4;
// `CascadeShadowConfigBuilder` requires `first_cascade_far_bound` to be
// strictly greater than `minimum_distance`; this margin keeps tiny fitted scenes
// from landing exactly on the minimum.
pub(crate) const CASCADE_FIRST_BOUND_HEADROOM: f32 = 0.001;
// Far bound of the first (high-resolution) cascade as a fraction of the fitted
// `maximum_distance`. 0.2 matches the proven 12/60 split.
pub(crate) const CASCADE_FIRST_BOUND_RATIO: f32 = 0.2;
pub(crate) const CASCADE_FIRST_FAR_BOUND: f32 = 6.0;
// Auto-fit cascade: once the scene's geometry exists, the key light's cascade
// `maximum_distance` is set to (scene bounding-sphere radius * this multiple),
// measured only over the meshes the key light actually shadows (those sharing
// its render layers, so screen-space UI panels are excluded). The multiple
// bakes in the studio camera's framing standoff; 5.0 gives ~18 for the
// canonical ~5-unit grounds and scales up for larger scenes.
pub(crate) const CASCADE_FIT_RADIUS_MULTIPLE: f32 = 5.0;
pub(crate) const CASCADE_MAX_DISTANCE: f32 = 18.0;
pub(crate) const CASCADE_MIN_DISTANCE: f32 = 0.1;
// The auto-fit re-runs each frame so the cascade re-adjusts when the projection
// toggles; it rewrites the config only when `maximum_distance` moves by more
// than this, so a steady camera costs nothing.
pub(crate) const CASCADE_REFIT_EPSILON: f32 = 0.01;

// clear color
pub(crate) const CLEAR_COLOR: Color = Color::srgb(0.012, 0.014, 0.018);

// connectors
pub(crate) const CENTER_FRACTION: f32 = 0.5;
pub(crate) const SPACER_EDGE_OFFSET: Px = Px(0.0);

// cube defaults
pub(crate) const CUBE_DEFAULT_COLOR: Color = Color::srgb(0.8, 0.7, 0.6);
pub(crate) const CUBE_DEFAULT_SIZE: f32 = 1.0;

/// Canonical example cube color.
pub const EXAMPLE_CUBE_COLOR: Color = CUBE_DEFAULT_COLOR;
/// Canonical example cube edge length in world units.
pub const EXAMPLE_CUBE_SIZE: f32 = CUBE_DEFAULT_SIZE;
// cube face panel
pub(crate) const CUBE_FACE_PANEL_ACTIVE_BODY_SIZE: f32 = 52.0;
pub(crate) const CUBE_FACE_PANEL_BODY_SIZE: f32 = 44.0;
pub(crate) const CUBE_FACE_PANEL_PADDING_FRACTION: f32 = 0.06;
pub(crate) const CUBE_FACE_PANEL_ROW_GAP_FRACTION: f32 = 0.02;
pub(crate) const CUBE_FACE_PANEL_SIZE_FRACTION: f32 = 0.88;
pub(crate) const CUBE_FACE_PANEL_TITLE_SIZE: f32 = 72.0;

// cube spin
pub(crate) const CUBE_SPIN_PAUSE_CONTROL_ID: &str = "cube_spin_pause";
pub(crate) const CUBE_SPIN_PAUSE_CONTROL_LABEL: &str = "P Pause";
pub(crate) const CUBE_SPIN_RESERVE_LABEL: &str = "cube spin";
pub(crate) const DEFAULT_CUBE_SPIN_RADIANS_PER_SECOND: f32 = 0.2;

// environment map
pub(crate) const DIFFUSE_MAP: &str =
    "embedded://fairy_dust/environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2";
/// Scales the environment map's contribution to lighting. Matches the value
/// used by the `hana` editor camera.
pub(crate) const ENV_LIGHT_INTENSITY: f32 = 2000.0;
pub(crate) const SPECULAR_MAP: &str =
    "embedded://fairy_dust/environment_maps/pisa_specular_rgb9e5_zstd.ktx2";

// face text
/// Default face-label size for simple cube-mounted `WorldText` labels.
pub const CUBE_FACE_LABEL_SIZE: f32 = 0.095;
/// Canonical blue for cube-mounted labels and panels.
pub const CUBE_FACE_PANEL_BLUE: Color = Color::srgb(0.1, 0.35, 1.0);
/// Default time that cube face input labels remain visible after release.
pub const CUBE_FACE_PANEL_RELEASE_HOLD: Duration = Duration::from_millis(300);
/// Outward offset for a face label so it does not z-fight the cube surface.
pub(crate) const FACE_TEXT_Z_OFFSET: f32 = 0.001;

// fill light
pub(crate) const FILL_LIGHT_ILLUMINANCE: f32 = 1_400.0;
pub(crate) const FILL_LIGHT_POS: Vec3 = Vec3::new(4.5, 4.0, -3.5);

// fold controls
pub(crate) const FOLD_CONTROL_DIAGNOSTIC_CAPACITY: usize = 128;
pub(crate) const FOLD_CONTROL_ID: &str = "fold_step";
pub(crate) const FOLD_CONTROL_LABEL: &str = "Space Fold";
pub(crate) const FOLD_CONTROL_RESERVE_LABEL: &str = "fold step";
pub(crate) const FOLD_PLAY_CONTROL_ID: &str = "fold_play";
pub(crate) const FOLD_PLAY_CONTROL_LABEL: &str = "P Play";
pub(crate) const FOLD_PLAY_RESERVE_LABEL: &str = "fold play";
pub(crate) const UNFOLD_CONTROL_ID: &str = "unfold_step";
pub(crate) const UNFOLD_CONTROL_LABEL: &str = "Shift+Space Unfold";

// ground plane defaults
pub(crate) const GROUND_PLANE_ALPHA: f32 = 0.78;
pub(crate) const GROUND_PLANE_DEFAULT_COLOR: Color = Color::srgb(0.125, 0.14, 0.16);
pub(crate) const GROUND_PLANE_DEFAULT_SIZE: f32 = 8.0;
pub(crate) const GROUND_PLANE_METALLIC: f32 = 0.0;
pub(crate) const GROUND_PLANE_REFLECTANCE: f32 = 0.45;
pub(crate) const GROUND_PLANE_ROUGHNESS: f32 = 0.40;

// key light
pub(crate) const KEY_LIGHT_ILLUMINANCE: f32 = 13_500.0;
pub(crate) const KEY_LIGHT_POS: Vec3 = Vec3::new(-3.5, 7.0, 4.8);
pub(crate) const KEY_SHADOW_DEPTH_BIAS: f32 = 0.03;
pub(crate) const KEY_SHADOW_NORMAL_BIAS: f32 = 0.7;

// log filter
/// Default `tracing` filter applied by [`crate::sprinkle_example`].
///
/// Quiets the most common chatty crates (`wgpu`, `naga`) while leaving the
/// rest at `info` so example-side `info!`/`warn!` calls remain visible.
pub const LOG_FILTER: &str = "info,wgpu=error,naga=error,bevy_winit=warn,bevy_render=warn";

// orbit cam
/// Canonical pitch limit for examples that manually spawn an `OrbitCam`.
pub(crate) const EXAMPLE_ORBIT_CAM_PITCH_LIMIT: f32 = std::f32::consts::TAU / 3.0;
/// Canonical lower zoom/radius limit for examples that manually spawn an `OrbitCam`.
pub(crate) const EXAMPLE_ORBIT_CAM_ZOOM_LOWER_LIMIT: f32 = 0.01;
/// Canonical upper zoom/radius limit for examples that manually spawn an `OrbitCam`.
pub(crate) const EXAMPLE_ORBIT_CAM_ZOOM_UPPER_LIMIT: f32 = 40.0;

// point light
pub(crate) const POINT_LIGHT_COLOR: Color = Color::srgb(0.45, 0.68, 1.0);
pub(crate) const POINT_LIGHT_INTENSITY: f32 = 1_900.0;
pub(crate) const POINT_LIGHT_POS: Vec3 = Vec3::new(-2.0, 1.15, 1.85);
pub(crate) const POINT_LIGHT_RANGE: f32 = 6.0;

// shadow map
pub(crate) const SHADOW_MAP_SIZE: usize = 4096;

// shortcuts
/// Keys whose press, while held, suppresses every bare example shortcut. Bare
/// shortcuts fire only when none of these is down, mirroring the `BlockBy` that
/// guards Fairy Dust's own bei chords.
pub(crate) const MODIFIER_KEYS: [KeyCode; 8] = [
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::ShiftLeft,
    KeyCode::ShiftRight,
    KeyCode::AltLeft,
    KeyCode::AltRight,
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
];

// target
pub(crate) const TARGET: Vec3 = Vec3::new(0.0, 0.45, 0.0);

// theme borders
pub(crate) const BORDER: Px = Px(2.0);
pub(crate) const BORDER_ACCENT: Color = Color::srgba(0.15, 0.7, 0.9, 0.5);
pub(crate) const BORDER_DIM: Color = Color::srgba(0.1, 0.4, 0.6, 0.3);
pub(crate) const INNER_BORDER_WIDTH: Px = Px(1.0);

// theme colors
/// Default inner background color for `fairy_dust` screen panels.
///
/// Used by the title bar, description panel, and camera control panel.
/// Exposed publicly so callers tweaking only the alpha can do:
/// `panel.with_background_color(DEFAULT_PANEL_BACKGROUND.with_alpha(0.85))`.
pub const DEFAULT_PANEL_BACKGROUND: Color = Color::srgba(0.02, 0.03, 0.07, 0.80);
pub(crate) const INNER_BACKGROUND: Color = DEFAULT_PANEL_BACKGROUND;
/// Canonical HUD title/header color used by Fairy Dust screen panels.
pub const TITLE_COLOR: Color = Color::srgb(0.9, 0.95, 1.0);

// theme padding
pub(crate) const FRAME_PAD: Px = Px(2.0);
pub(crate) const INNER_PAD: Px = Px(10.0);
const INSET: Px = Px(FRAME_PAD.0 + BORDER.0);

// theme radius
pub(crate) const INNER_RADIUS: Px = Px(RADIUS.0 - INSET.0);
pub(crate) const RADIUS: Px = Px(12.0);

// typography
/// Canonical HUD body / label size. Used by `fairy_dust` panels and re-exported
/// for ad-hoc panels that want to match the built-in look.
pub const LABEL_SIZE: Pt = Pt(11.0);
/// Canonical HUD title size. Used by `fairy_dust` panels and re-exported
/// for ad-hoc panels that want to match the built-in look.
pub const TITLE_SIZE: Pt = Pt(14.0);

// unclamp
/// Zoom floor left in place after unclamping. The zoom limit's lower bound must
/// stay > 0, or the camera sticks at radius 0.
pub(crate) const UNCLAMPED_ZOOM_LOWER_LIMIT: f32 = 1e-9;

/// Canonical cube transform for a cube sitting on the ground plane with extra clearance.
#[must_use]
pub const fn example_cube_on_ground(clearance: f32) -> Vec3 {
    Vec3::new(0.0, EXAMPLE_CUBE_SIZE * 0.5 + clearance, 0.0)
}
