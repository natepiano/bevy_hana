//! Camera input API.
//!
//! # Quick Start
//!
//! `OrbitCam` defaults to
//! `OrbitCamInputMode::with_preset(OrbitCamPreset::simple_mouse())`. Insert
//! `OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like())` for editor-style
//! navigation, [`OrbitCamInputMode::Bindings`] when your app owns a keymap or
//! gamepad mapping, or [`OrbitCamInputMode::Manual`] when your app computes
//! camera intent itself. `FreeCam` defaults to
//! `FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse())`, with
//! [`FreeCamInputMode::Manual`] available when your app computes free-flight
//! camera intent itself.
//!
//! App-authored manual camera input should write through
//! [`OrbitCamManualInputWriter`] or [`FreeCamManualInputWriter`] in
//! [`CameraInputPhase::WriteManual`].
//! Preset and custom binding input is finalized before the controller runs and
//! emits [`OrbitCamInteractionStarted`], [`OrbitCamInteractionEnded`], and
//! [`OrbitCamInteractionSourcesChanged`] for `OrbitCam`, or
//! [`FreeCamInteractionStarted`], [`FreeCamInteractionEnded`], and
//! [`FreeCamInteractionSourcesChanged`] for `FreeCam`, with source attribution.
//!
//! Surface metrics are derived into the resolved input route each frame.
//! An explicit [`CameraInputSurfaceMetrics`] component overrides only the
//! fields it provides, which is useful for render-to-texture and editor-panel
//! cameras whose logical input surface differs from the rendered camera view.
//! Mouse-like and keyboard held interactions keep their owner while held.
//! Gamepad and touch sources are named in the reported attribution, but no
//! latch holds them: `CameraInputSourceLatches` latches only the mouse, wheel,
//! and smooth-scroll sources, plus the keyboard.
//!
//! [`CameraInputPhase::WriteManual`]: crate::CameraInputPhase::WriteManual

mod action_resolution;
mod actions;
mod axis_response;
#[macro_use]
mod bindings;
mod constants;
mod context;
mod control_summary;
mod disabled;
mod events;
mod install;
mod intent;
mod interaction_state;
mod lifecycle;
mod manual;
mod metrics;
mod mode_reconciliation;
mod modes;
mod routing;
mod source_input_gain;
mod sources;
mod touch;

pub(super) use action_resolution::CameraActionResolutionContext;
pub(super) use action_resolution::CameraActionResolutionKind;
pub(super) use action_resolution::NoActionFrameState;
pub(super) use action_resolution::resolve_actions_into_camera_input;
pub use actions::CameraSemanticAction;
pub(super) use actions::FreeCamGateAction;
pub(super) use actions::FreeCamHomeAction;
pub use actions::FreeCamLookAction;
pub(super) use actions::FreeCamLookButtonAction;
pub use actions::FreeCamRollAction;
pub(super) use actions::FreeCamRollEngagedAction;
pub(super) use actions::FreeCamSlowModeToggleAction;
pub use actions::FreeCamTranslateAction;
pub(super) use actions::FreeCamTranslateEngagedAction;
pub use actions::HeldCameraAction;
pub use actions::ImpulseCameraAction;
pub(super) use actions::OrbitCamAdapterOrbitAction;
pub(super) use actions::OrbitCamAdapterPanAction;
pub(super) use actions::OrbitCamAdapterZoomCoarseAction;
pub(super) use actions::OrbitCamAdapterZoomSmoothAction;
pub(super) use actions::OrbitCamGateAction;
pub(super) use actions::OrbitCamHomeAction;
pub use actions::OrbitCamOrbitAction;
pub(super) use actions::OrbitCamOrbitEngagedAction;
pub(super) use actions::OrbitCamOrbitSlowAction;
pub use actions::OrbitCamPanAction;
pub(super) use actions::OrbitCamPanEngagedAction;
pub(super) use actions::OrbitCamPanSlowAction;
pub(super) use actions::OrbitCamSlowModeToggleAction;
pub use actions::OrbitCamZoomCoarseAction;
pub(super) use actions::OrbitCamZoomEngagedAction;
pub use actions::OrbitCamZoomSmoothAction;
pub(super) use actions::OrbitCamZoomSmoothSlowAction;
pub use axis_response::AxisResponse;
pub use axis_response::Damping;
pub use axis_response::Sensitivity;
use bevy::input::gestures::PinchGesture;
use bevy::input::touch::Touches;
use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PreUpdate;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
pub use bindings::ActionBindingDescriptor;
pub use bindings::BindingEngagement;
pub use bindings::BindingGate;
pub use bindings::BindingGates;
pub use bindings::BindingRoutePolicy;
pub use bindings::BindingsError;
pub use bindings::CameraInputScalePolicy;
pub use bindings::CameraSlowMode;
pub use bindings::GateInput;
pub use bindings::GatePolarity;
pub(crate) use bindings::HeldActionBindingEntry;
pub(super) use bindings::HeldActionBindingSet;
pub use bindings::HeldBinding;
pub(super) use bindings::HeldBindingDescriptor;
pub(crate) use bindings::ImpulseActionBindingEntry;
pub(crate) use bindings::ImpulseActionBindingSet;
pub use bindings::InputAxisTransform;
pub use bindings::InputBinding;
pub(super) use bindings::InputBindingDescriptor;
pub(super) use bindings::InputBindingEntry;
pub use bindings::InputBindingModifiers;
pub use bindings::InputBindingScale;
pub use bindings::InputDeadZone;
pub use bindings::InputDeltaScale;
pub use bindings::InputGain;
pub(super) use bindings::LiveInputs;
pub(super) use bindings::action_descriptor_to_entry;
pub(super) use bindings::attributed_sources;
pub(super) use bindings::held_descriptors_to_set;
pub(super) use bindings::mod_keys_pressed;
pub(super) use bindings::validate_held_entries;
pub(super) use bindings::validate_impulse_entries;
pub(super) use bindings::validate_slow_mode;
pub(super) use constants::BUTTON_ZOOM_SCALE;
#[cfg(test)]
pub(super) use constants::CUSTOM_SLOW_SCALE;
pub(crate) use constants::DEFAULT_INPUT_GAIN;
#[cfg(test)]
pub(super) use constants::DISABLED_INPUT_GAIN;
pub(super) use constants::FREE_CAM_HOME_ACTION_NAME;
pub(super) use constants::FREE_CAM_LOOK_ACTION_NAME;
pub(super) use constants::FREE_CAM_ROLL_ACTION_NAME;
pub(super) use constants::FREE_CAM_TRANSLATE_ACTION_NAME;
#[cfg(test)]
pub(super) use constants::INVALID_SOURCE_INPUT_GAIN;
pub(super) use constants::ORBIT_ACTION_NAME;
pub(super) use constants::ORBIT_HOME_ACTION_NAME;
pub(super) use constants::PAN_ACTION_NAME;
pub(super) use constants::PINCH_GESTURE_AMPLIFICATION;
#[cfg(test)]
pub(super) use constants::PINCH_INPUT_GAIN;
pub(super) use constants::PIXEL_SCROLL_SCALE;
pub(super) use constants::TOUCH_PINCH_SCALE;
#[cfg(test)]
pub(super) use constants::WHEEL_INPUT_GAIN;
pub(super) use constants::ZOOM_COARSE_ACTION_NAME;
pub(super) use constants::ZOOM_SMOOTH_ACTION_NAME;
pub use context::FreeCamInputContext;
pub use context::OrbitCamInputContext;
pub use control_summary::CameraControlAction;
pub use control_summary::CameraControlActivation;
pub use control_summary::CameraControlBinding;
pub use control_summary::CameraControlBindingKind;
pub use control_summary::CameraControlSummary;
pub use control_summary::ControlSpeed;
pub use control_summary::OrbitCamControlRow;
pub use control_summary::OrbitCamControlSummary;
pub use control_summary::ZoomDirection;
pub use control_summary::describe_controls;
pub use control_summary::describe_controls_for;
pub use control_summary::describe_orbit_cam_controls;
pub use disabled::CameraInputDisabled;
pub use events::CameraHomed;
pub use events::CameraInputMetricsMissing;
pub use events::FreeCamInteractionEnded;
pub use events::FreeCamInteractionKind;
pub use events::FreeCamInteractionSourcesChanged;
pub use events::FreeCamInteractionSpeedChanged;
pub use events::FreeCamInteractionStarted;
pub use events::OrbitCamInteractionEnded;
pub use events::OrbitCamInteractionKind;
pub use events::OrbitCamInteractionSourcesChanged;
pub use events::OrbitCamInteractionSpeedChanged;
pub use events::OrbitCamInteractionStarted;
pub use events::ResetFreeCamToHome;
pub use events::ResetOrbitCamToHome;
pub(super) use install::CameraBindingGateCondition;
pub(super) use install::CameraInstallKind;
pub(super) use install::GateActionCache;
pub(super) use install::MotionActions;
pub(super) use install::action_sources;
pub(super) use install::held_sources;
pub(super) use install::spawn_action;
pub(super) use install::spawn_binding;
pub(super) use install::spawn_held_bindings;
pub(super) use install::spawn_single_binding;
pub use intent::CameraInputKind;
pub use intent::InputIntent;
pub use intent::IntentChannel;
pub use intent::IntentChannels;
pub use interaction_state::FreeCamActiveDirections;
pub use interaction_state::FreeCamControlDirection;
pub use interaction_state::FreeCamInteractionState;
pub use interaction_state::OrbitCamInteractionState;
pub(super) use lifecycle::CameraInputLifecyclePlugin;
pub use lifecycle::CameraInputReportingDebounce;
pub use manual::FreeCamManualInput;
pub use manual::FreeCamManualInputWriter;
pub use manual::OrbitCamManualInput;
pub use manual::OrbitCamManualInputWriter;
pub use metrics::CameraInputMetricKind;
pub use metrics::CameraInputSurfaceMetrics;
pub(super) use mode_reconciliation::CameraInputModeReplaced;
pub(super) use mode_reconciliation::CameraInputModesPlugin;
pub(super) use mode_reconciliation::CameraInstalledBindings;
#[cfg(test)]
pub(super) use mode_reconciliation::CameraResolvedBindings;
pub(super) use mode_reconciliation::FreeCamResolvedBindings;
pub(super) use mode_reconciliation::OrbitCamInputInstallationOf;
pub(super) use mode_reconciliation::OrbitCamResolvedBindings;
pub(super) use mode_reconciliation::input_installation_has_placeholder_for;
pub(super) use mode_reconciliation::installed_input_entities_for;
pub(super) use mode_reconciliation::replace_installed_input_entities_for;
pub use modes::CameraInputModeKind;
pub(super) use modes::CameraManual;
pub use modes::FreeCamInputMode;
pub use modes::InputMode;
pub use modes::OrbitCamInputMode;
pub(super) use routing::CameraInputBlockers;
pub(super) use routing::CameraInputContextGated;
pub use routing::CameraInputRouting;
pub use routing::CameraInputRoutingConfig;
pub(super) use routing::CameraInputRoutingPlugin;
pub(super) use routing::CameraInputSourceLatches;
pub(crate) use routing::CameraSlowModeLatches;
pub use routing::CameraSlowModeState;
pub use routing::NoPositionFallback;
pub use routing::ResolvedCameraInputRoute;
pub use source_input_gain::GamepadInputGain;
pub use source_input_gain::MouseInputGain;
pub use source_input_gain::SmoothScrollInputGain;
pub use sources::InteractionSources;
pub use sources::ManualInputSource;
#[cfg(test)]
pub(super) use touch::OneFingerGestures;
pub(super) use touch::TouchGestures;
pub(super) use touch::TouchTracker;
#[cfg(test)]
pub(super) use touch::TwoFingerGestures;

pub use crate::free_cam::FreeCamBindings;
pub use crate::free_cam::FreeCamBindingsBuilder;
pub use crate::free_cam::FreeCamChannels;
pub use crate::free_cam::FreeCamGamepadLayout;
pub use crate::free_cam::FreeCamGamepadPreset;
pub use crate::free_cam::FreeCamHomeActionBindings;
pub use crate::free_cam::FreeCamInput;
pub use crate::free_cam::FreeCamInputGain;
pub use crate::free_cam::FreeCamKeyboardMousePreset;
pub use crate::free_cam::FreeCamLookActionBindings;
pub use crate::free_cam::FreeCamLookBinding;
pub use crate::free_cam::FreeCamLookPitch;
pub use crate::free_cam::FreeCamMouseLook;
pub use crate::free_cam::FreeCamPreset;
pub use crate::free_cam::FreeCamPresetKind;
pub use crate::free_cam::FreeCamRollActionBindings;
pub use crate::free_cam::FreeCamRollBinding;
pub use crate::free_cam::FreeCamTranslateActionBindings;
pub use crate::free_cam::FreeCamTranslateBinding;
pub use crate::free_cam::FreeCamTranslateKeys;
pub use crate::free_cam::LookDelta;
pub use crate::free_cam::RollDelta;
pub use crate::free_cam::TranslateDelta;
pub use crate::orbit_cam::CameraInputGamepadSelectionPolicy;
pub use crate::orbit_cam::CoarseZoomDelta;
pub use crate::orbit_cam::OrbitCamBindingWithInputGain;
pub use crate::orbit_cam::OrbitCamBindings;
pub use crate::orbit_cam::OrbitCamBindingsBuilder;
pub use crate::orbit_cam::OrbitCamBlenderLikeKeyboardPreset;
pub use crate::orbit_cam::OrbitCamBlenderLikePreset;
pub use crate::orbit_cam::OrbitCamButtonDragZoom;
pub use crate::orbit_cam::OrbitCamButtonDragZoomAxis;
pub use crate::orbit_cam::OrbitCamGamepadPreset;
pub use crate::orbit_cam::OrbitCamGamepadPresetBuilder;
pub use crate::orbit_cam::OrbitCamHomeActionBindings;
pub use crate::orbit_cam::OrbitCamInput;
pub use crate::orbit_cam::OrbitCamInputGain;
pub use crate::orbit_cam::OrbitCamKeyboardPreset;
pub use crate::orbit_cam::OrbitCamLineScroll;
pub use crate::orbit_cam::OrbitCamMouseDrag;
pub use crate::orbit_cam::OrbitCamMouseWheelZoom;
pub use crate::orbit_cam::OrbitCamOrbitActionBindings;
pub use crate::orbit_cam::OrbitCamOrbitBinding;
pub use crate::orbit_cam::OrbitCamPanActionBindings;
pub use crate::orbit_cam::OrbitCamPanBinding;
pub use crate::orbit_cam::OrbitCamPinchZoom;
pub use crate::orbit_cam::OrbitCamPreset;
pub use crate::orbit_cam::OrbitCamPresetKind;
pub use crate::orbit_cam::OrbitCamSimpleMouseKeyboardPreset;
pub use crate::orbit_cam::OrbitCamSimpleMousePreset;
pub use crate::orbit_cam::OrbitCamTouchBinding;
pub use crate::orbit_cam::OrbitCamTouchBindingConfig;
pub use crate::orbit_cam::OrbitCamTrackpadScroll;
pub use crate::orbit_cam::OrbitCamZoomBinding;
pub use crate::orbit_cam::OrbitCamZoomCoarseActionBindings;
pub use crate::orbit_cam::OrbitCamZoomSmoothActionBindings;
pub use crate::orbit_cam::OrbitDelta;
pub use crate::orbit_cam::PanDelta;
pub use crate::orbit_cam::PinchGestureZoom;
pub use crate::orbit_cam::SmoothZoomDelta;
pub use crate::orbit_cam::ZoomDelta;
pub use crate::orbit_cam::ZoomInversion;
use crate::system_sets::CameraInputInternalSet;
use crate::system_sets::CameraInputPhase;

/// Registers shared camera input infrastructure used by every camera kind.
///
/// Each camera kind registers its own enhanced-input context in its own
/// plugin; this owns the enhanced-input core, the bevy touch/gesture inputs,
/// and the touch tracker that feeds the input adapter.
pub(super) struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<EnhancedInputPlugin>() {
            app.add_plugins(EnhancedInputPlugin);
        }
        app.add_plugins((
            CameraInputModesPlugin,
            CameraInputRoutingPlugin,
            CameraInputLifecyclePlugin,
        ))
        .init_resource::<TouchTracker>()
        .init_resource::<Touches>()
        .add_message::<PinchGesture>()
        .add_systems(
            PreUpdate,
            touch::touch_tracker
                .in_set(CameraInputPhase::PreInput)
                .before(CameraInputInternalSet::AdapterInjection),
        );
    }
}
