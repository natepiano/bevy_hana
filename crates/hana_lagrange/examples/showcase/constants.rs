use std::f32::consts::PI;

use bevy::math::curve::easing::EaseFunction;
use bevy::prelude::Color;
use bevy::prelude::Vec3;

// camera home
pub(super) const CAMERA_START_PITCH: f32 = 0.4;
pub(super) const CAMERA_START_YAW: f32 = -0.2;
/// Framing margin for the Home view — 32% looser than the scene-fit margin.
pub(super) const HOME_MARGIN: f32 = ZOOM_MARGIN_SCENE * 1.5;

// durations
pub(super) const ANIMATE_FIT_DURATION_MILLIS: u64 = 1200;
pub(super) const LOOK_AT_DURATION_MILLIS: u64 = 800;
pub(super) const ORBIT_MOVE_DURATION_MILLIS: u64 = 800;
pub(super) const ZOOM_DURATION_MILLIS: u64 = 1000;

// easings
pub(super) const ALL_EASINGS: &[EaseFunction] = &[
    EaseFunction::Linear,
    EaseFunction::QuadraticIn,
    EaseFunction::QuadraticOut,
    EaseFunction::QuadraticInOut,
    EaseFunction::CubicIn,
    EaseFunction::CubicOut,
    EaseFunction::CubicInOut,
    EaseFunction::QuarticIn,
    EaseFunction::QuarticOut,
    EaseFunction::QuarticInOut,
    EaseFunction::QuinticIn,
    EaseFunction::QuinticOut,
    EaseFunction::QuinticInOut,
    EaseFunction::SmoothStepIn,
    EaseFunction::SmoothStepOut,
    EaseFunction::SmoothStep,
    EaseFunction::SmootherStepIn,
    EaseFunction::SmootherStepOut,
    EaseFunction::SmootherStep,
    EaseFunction::SineIn,
    EaseFunction::SineOut,
    EaseFunction::SineInOut,
    EaseFunction::CircularIn,
    EaseFunction::CircularOut,
    EaseFunction::CircularInOut,
    EaseFunction::ExponentialIn,
    EaseFunction::ExponentialOut,
    EaseFunction::ExponentialInOut,
    EaseFunction::ElasticIn,
    EaseFunction::ElasticOut,
    EaseFunction::ElasticInOut,
    EaseFunction::BackIn,
    EaseFunction::BackOut,
    EaseFunction::BackInOut,
    EaseFunction::BounceIn,
    EaseFunction::BounceOut,
    EaseFunction::BounceInOut,
];

// event log
pub(super) const EVENT_LOG_CAMERA_MOVE_END: &str = "CameraMoveEnd";
pub(super) const EVENT_LOG_CHILD_GAP: f32 = 8.0;
/// Normal log text — white, matching the panel title.
pub(super) const EVENT_LOG_COLOR: Color = Color::srgb(0.9, 0.95, 1.0);
/// Blue divider under the title and between the footer hints, matching the
/// title bar's separators.
pub(super) const EVENT_LOG_DIVIDER_COLOR: Color = Color::srgba(0.35, 0.8, 1.0, 0.35);
pub(super) const EVENT_LOG_DIVIDER_THICKNESS: f32 = 1.0;
pub(super) const EVENT_LOG_EASING_RESET: &str = "Easing: reset to CubicOut";
pub(super) const EVENT_LOG_ENTRY_GAP: f32 = 4.0;
pub(super) const EVENT_LOG_ERROR_COLOR: Color = Color::srgba(1.0, 0.3, 0.3, 0.9);
pub(super) const EVENT_LOG_HINT_SEPARATOR_HEIGHT: f32 = 14.0;
pub(super) const EVENT_LOG_HINT_SIZE: f32 = 11.0;
/// Upper bound on a single entry's rendered height, used to cap how far the
/// scrollback value may run past the real content extent.
pub(super) const EVENT_LOG_MAX_ENTRY_HEIGHT: f32 = EVENT_LOG_TEXT_SIZE * 6.0;
/// Event-log scroll speed in logical px per second (Up/Down arrows).
pub(super) const EVENT_LOG_SCROLL_SPEED: f32 = 400.0;
pub(super) const EVENT_LOG_TEXT_SIZE: f32 = 10.0;
pub(super) const EVENT_LOG_SEPARATOR: &str = "- - - - - - - - - - - -";
pub(super) const EVENT_LOG_TITLE: &str = "Event Log";
/// Fixed panel width — 20% narrower than the camera control panel's ~340px.
pub(super) const EVENT_LOG_WIDTH: f32 = 272.0;
pub(super) const EVENT_LOG_ZOOM_CANCELLED: &str = "ZoomEnd\n  reason=Cancelled";
pub(super) const EVENT_LOG_ZOOM_COMPLETED: &str = "ZoomEnd\n  reason=Completed";
pub(super) const LOG_CLEAR_HINT_TEXT: &str = "⌫ clear";
pub(super) const LOG_SCROLL_HINT_TEXT: &str = "↑↓ scroll log";

// gizmos
pub(super) const GIZMO_DEPTH_BIAS: f32 = -0.005;
pub(super) const GIZMO_LINE_WIDTH: f32 = 2.0;
pub(super) const GIZMO_SCALE: f32 = 1.001;

// hints
pub(super) const HINT_TEXT_COLOR: Color = Color::srgba(0.7, 0.7, 0.7, 0.7);

// easing flash
/// Seconds the `R Random Easing` / `0 Reset Easing` title-bar chips stay highlighted
/// after a press.
pub(super) const EASING_FLASH_SECONDS: f32 = 0.5;

// title bar controls
pub(super) const ANIMATE_CONTROL: &str = "M Animate";
pub(super) const EASING_CONTROL: &str = "R Random Easing";
pub(super) const EASING_RESET_CONTROL: &str = "0 Reset Easing";
pub(super) const EVENT_LOG_CONTROL: &str = "G Log";
pub(super) const LOOK_AND_FIT_CONTROL: &str = "L LookAt+Fit";
pub(super) const LOOK_AT_CONTROL: &str = "K LookAt";
pub(super) const ORTHOGRAPHIC_CONTROL: &str = "O Orthographic";
pub(super) const OVERLAY_CONTROL: &str = "Y Fit Overlay";
pub(super) const PAUSE_CONTROL: &str = "Esc Pause";
pub(super) const PERSPECTIVE_CONTROL: &str = "P Perspective";
pub(super) const SHOWCASE_TITLE: &str = "Showcase";

// mesh settings
pub(super) const MESH_CENTER_Y: f32 = 1.0;
pub(super) const ZOOM_MARGIN_MESH: f32 = 0.15;
pub(super) const ZOOM_MARGIN_SCENE: f32 = 0.08;

// paused overlay
pub(super) const OVERLAY_TEXT_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.4);
pub(super) const PAUSED_OVERLAY_FONT_SIZE: f32 = 48.0;
pub(super) const PAUSED_TEXT: &str = "PAUSED";

// policy panel
pub(super) const POLICY_PANEL_ACTIVE_COLOR: Color = Color::srgb(1.0, 0.9, 0.25);
pub(super) const POLICY_PANEL_ARROW: &str = "->";
pub(super) const POLICY_PANEL_COLUMN_GAP: f32 = 8.0;
pub(super) const POLICY_PANEL_CONFLICT_HEADER: &str = "AnimationConflictPolicy";
pub(super) const POLICY_PANEL_CONFLICT_KEY: &str = "C";
pub(super) const POLICY_PANEL_GROUP_GAP: f32 = 10.0;
pub(super) const POLICY_PANEL_HEADER_GAP: f32 = 6.0;
pub(super) const POLICY_PANEL_HEADER_SIZE: f32 = 12.0;
/// Panel fits its content but never grows taller than this fraction of the
/// viewport height.
pub(super) const POLICY_PANEL_HEIGHT_PERCENT: f32 = 0.30;
pub(super) const POLICY_PANEL_INTERRUPT_HEADER: &str = "CameraInputInterruptBehavior";
pub(super) const POLICY_PANEL_INTERRUPT_KEY: &str = "I";
pub(super) const POLICY_PANEL_KEY_COLUMN_WIDTH: f32 = 40.0;
/// Seconds the toggle key stays highlighted after a press, signaling the cycle.
pub(super) const POLICY_PANEL_KEY_FLASH_SECONDS: f32 = 1.0;
pub(super) const POLICY_PANEL_KEY_TEXT_SIZE: f32 = 12.0;
pub(super) const POLICY_PANEL_NAME_COLUMN_WIDTH: f32 = 76.0;
pub(super) const POLICY_PANEL_NAME_GAP: f32 = 8.0;
pub(super) const POLICY_PANEL_ROW_GAP: f32 = 5.0;
pub(super) const POLICY_PANEL_TEXT_SIZE: f32 = 10.0;
/// Panel fits its content but never grows wider than this fraction of the
/// viewport width.
pub(super) const POLICY_PANEL_WIDTH_PERCENT: f32 = 0.50;

// projection
pub(super) const ORTHOGRAPHIC_FAR_PLANE: f32 = 40.0;
pub(super) const ORTHOGRAPHIC_VIEWPORT_HEIGHT: f32 = 1.0;
pub(super) const PROJECTION_LOG_ORTHOGRAPHIC: &str = "Projection: Orthographic";
pub(super) const PROJECTION_LOG_PERSPECTIVE: &str = "Projection: Perspective";

// render layers
pub(super) const DEFAULT_SCENE_LAYER: usize = 0;

// rotation
pub(super) const FIRST_ORBIT_MOVE_QUARTER_TURNS: f32 = 1.0;
pub(super) const FOURTH_ORBIT_MOVE_QUARTER_TURNS: f32 = 4.0;
pub(super) const QUARTER_TURN_RADIANS: f32 = PI / 2.0;
pub(super) const SECOND_ORBIT_MOVE_QUARTER_TURNS: f32 = 2.0;
pub(super) const THIRD_ORBIT_MOVE_QUARTER_TURNS: f32 = 3.0;

// scene
pub(super) const GROUND_ALPHA: f32 = 0.85;
pub(super) const GROUND_COLOR: Color = Color::srgb(0.125, 0.14, 0.16);
pub(super) const GROUND_SIZE: f32 = 12.0;
pub(super) const MESH_CUBOID_COLOR: Color = Color::srgb(0.8, 0.7, 0.6);
pub(super) const MESH_CUBOID_SIZE: Vec3 = Vec3::splat(1.0);
pub(super) const MESH_CUBOID_TRANSLATION: Vec3 = Vec3::new(-2.5, MESH_CENTER_Y, 0.0);
pub(super) const MESH_SPHERE_COLOR: Color = Color::srgb(0.9, 0.3, 0.2);
pub(super) const MESH_SPHERE_LATITUDES: u32 = 64;
pub(super) const MESH_SPHERE_LONGITUDES: u32 = 128;
pub(super) const MESH_SPHERE_RADIUS: f32 = 0.5;
pub(super) const MESH_SPHERE_TRANSLATION: Vec3 = Vec3::new(0.0, MESH_CENTER_Y, 0.0);
pub(super) const MESH_TORUS_COLOR: Color = Color::srgb(0.9, 0.5, 0.1);
pub(super) const MESH_TORUS_MAJOR_RADIUS: f32 = 0.75;
pub(super) const MESH_TORUS_MAJOR_RESOLUTION: usize = 64;
pub(super) const MESH_TORUS_MINOR_RADIUS: f32 = 0.25;
pub(super) const MESH_TORUS_MINOR_RESOLUTION: usize = 64;
pub(super) const MESH_TORUS_TRANSLATION: Vec3 = Vec3::new(2.5, MESH_CENTER_Y, 0.0);
pub(super) const UNDERSIDE_PLANE_COLOR: Color = Color::srgba(0.0, 0.0, 0.0, 0.0);
pub(super) const UNDERSIDE_PLANE_ROTATION_X: f32 = PI;

// sensitivity
pub(super) const DRAG_SENSITIVITY: f32 = 0.02;

// time
pub(super) const SECONDS_TO_MILLIS: f32 = 1000.0;

// window
pub(super) const PRIMARY_WINDOW_TITLE: &str = "showcase";
