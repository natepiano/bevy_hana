use std::time::Duration;

// adapter scales
/// Conversion factor from mouse drag delta to scroll-equivalent zoom input.
pub(crate) const BUTTON_ZOOM_SCALE: f32 = 0.03;
/// Amplification factor for trackpad pinch gesture input.
pub(crate) const PINCH_GESTURE_AMPLIFICATION: f32 = 10.0;
/// Scale factor for converting pixel-based scroll events to zoom input.
pub(crate) const PIXEL_SCROLL_SCALE: f32 = 0.005;
/// Conversion factor from two-finger touch pinch to zoom input.
pub(crate) const TOUCH_PINCH_SCALE: f32 = 0.015;

// action names
pub(crate) const FREE_CAM_LOOK_ACTION_NAME: &str = "FreeCamLookAction";
pub(crate) const FREE_CAM_ROLL_ACTION_NAME: &str = "FreeCamRollAction";
pub(crate) const FREE_CAM_TRANSLATE_ACTION_NAME: &str = "FreeCamTranslateAction";
pub(crate) const FREE_CAM_HOME_ACTION_NAME: &str = "FreeCamHomeAction";
pub(crate) const ORBIT_ACTION_NAME: &str = "OrbitCamOrbitAction";
pub(crate) const ORBIT_HOME_ACTION_NAME: &str = "OrbitCamHomeAction";
pub(crate) const PAN_ACTION_NAME: &str = "OrbitCamPanAction";
pub(crate) const ZOOM_COARSE_ACTION_NAME: &str = "OrbitCamZoomCoarseAction";
pub(crate) const ZOOM_SMOOTH_ACTION_NAME: &str = "OrbitCamZoomSmoothAction";

// operation defaults
/// Neutral input gain multiplier for user-controlled camera input.
pub(crate) const DEFAULT_INPUT_GAIN: f32 = 1.0;

// camera labels
pub(super) const FREE_CAM_CAMERA_LABEL: &str = "FreeCam";
pub(super) const ORBIT_CAM_CAMERA_LABEL: &str = "OrbitCam";

// debounce durations
pub(super) const DEFAULT_REPORTING_DEBOUNCE: Duration = Duration::from_millis(100);

// interaction lifecycle
pub(super) const INTERACTION_CHANNEL_COUNT: usize = 3;

// mode labels
pub(super) const INPUT_MODE_LABEL: &str = "Input";
pub(super) const PRESET_MODE_LABEL: &str = "Preset";

// mode values
pub(super) const CUSTOM_BINDINGS_MODE_VALUE: &str = "custom bindings";
pub(super) const MANUAL_MODE_VALUE: &str = "manual input";

// setting row labels
pub(super) const INVERT_Y_BINDING_LABEL: &str = "alt-i";
pub(super) const INVERT_Y_STATUS_LABEL: &str = "Invert Y";

// row labels
pub(super) const APP_AUTHORED_INPUT_ROW_LABEL: &str = "app-authored input";
pub(super) const CUSTOM_INPUT_ROW_LABEL: &str = "custom input";
pub(super) const GAMEPAD_BINDINGS_ROW_LABEL: &str = "gamepad bindings";
pub(super) const ONE_FINGER_TOUCH_ROW_LABEL: &str = "one finger touch";
pub(super) const ROLL_DISABLED_ROW_LABEL: &str = "Roll disabled";
pub(super) const TWO_FINGER_TOUCH_ROW_LABEL: &str = "two finger touch";

// source-stem labels
pub(super) const GAMEPAD_BINDING_SOURCE_LABEL: &str = "gamepad binding";
pub(super) const INPUT_BINDING_SOURCE_LABEL: &str = "input binding";
pub(super) const KEYBOARD_BINDING_SOURCE_LABEL: &str = "keyboard binding";
pub(super) const MANUAL_INPUT_SOURCE_LABEL: &str = "manual input";
pub(super) const MOUSE_BINDING_SOURCE_LABEL: &str = "mouse binding";
pub(super) const MOUSE_DESCRIPTOR_LABEL: &str = "mouse";
pub(super) const PINCH_SOURCE_LABEL: &str = "pinch";
pub(super) const TOUCH_SOURCE_LABEL: &str = "touch";
pub(super) const TRACKPAD_SOURCE_LABEL: &str = "smooth-scroll";
pub(super) const WHEEL_SOURCE_LABEL: &str = "wheel";

// split zoom-direction row labels — each bidirectional zoom source shows one
// row per direction, labeled by the physical gesture that drives it.
pub(super) const WHEEL_ZOOM_IN_LABEL: &str = "wheel ↑";
pub(super) const WHEEL_ZOOM_OUT_LABEL: &str = "wheel ↓";
pub(super) const PINCH_ZOOM_IN_LABEL: &str = "pinch out";
pub(super) const PINCH_ZOOM_OUT_LABEL: &str = "pinch in";
pub(super) const SMOOTH_SCROLL_ZOOM_IN_LABEL: &str = "scroll ↑";
pub(super) const SMOOTH_SCROLL_ZOOM_OUT_LABEL: &str = "scroll ↓";

// free-cam gamepad row action labels — right-column overrides for the
// decomposed gamepad translate rows (stick, boost gate, vertical triggers) and
// the split roll rows, shown in place of the plain action name.
pub(super) const ROLL_LEFT_ACTION_LABEL: &str = "Roll ←";
pub(super) const ROLL_RIGHT_ACTION_LABEL: &str = "Roll →";
pub(super) const TRANSLATE_BOOST_ACTION_LABEL: &str = "Boost";
pub(super) const TRANSLATE_DOWN_ACTION_LABEL: &str = "Down";
pub(super) const TRANSLATE_UP_ACTION_LABEL: &str = "Up";

// shared test constants
#[cfg(test)]
pub(crate) const CUSTOM_SLOW_SCALE: f32 = 0.25;
#[cfg(test)]
pub(crate) const DISABLED_INPUT_GAIN: f32 = super::InputGain::DISABLED.0;
#[cfg(test)]
pub(crate) const INVALID_SOURCE_INPUT_GAIN: f32 = -1.0;
#[cfg(test)]
pub(crate) const PINCH_INPUT_GAIN: f32 = 0.5;
#[cfg(test)]
pub(crate) const WHEEL_INPUT_GAIN: f32 = 0.25;
