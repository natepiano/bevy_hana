#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

mod animation;
mod camera_basis;
mod camera_home;
mod camera_kind;
mod constants;
mod fit;
#[macro_use]
mod input;
mod free_cam;
mod initialization;
mod interpolation;
mod operation;
mod orbit_cam;
mod system_sets;
mod time_source;

pub use animation::AnimationBegin;
pub use animation::AnimationConflictPolicy;
pub use animation::AnimationEnd;
use animation::AnimationPlugin;
pub use animation::AnimationReason;
pub use animation::AnimationRejected;
pub use animation::AnimationRejectionReason;
pub use animation::AnimationSource;
pub use animation::CameraCommands;
pub use animation::CameraEvaluationError;
pub use animation::CameraEventTiming;
pub use animation::CameraInputInterruptBehavior;
pub use animation::CameraMove;
pub use animation::CameraMoveBegin;
pub use animation::CameraMoveDestination;
pub use animation::CameraMoveEnd;
pub use animation::CameraMoveError;
pub use animation::CameraPlaybackObservation;
pub use animation::CameraRequestPreparationError;
pub use animation::CameraSequence;
pub use animation::CameraSequenceError;
pub use animation::FreeCamRollTarget;
pub use animation::PlayAnimation;
use bevy::prelude::App;
use bevy::prelude::Plugin;
pub use camera_basis::CameraBasis;
pub use camera_home::CameraHomeKind;
pub use camera_home::CameraHomePending;
pub use camera_kind::CameraKind;
pub use fit::AnimateToFit;
pub use fit::CurrentFitTarget;
pub use fit::FitAnchor;
#[cfg(feature = "fit_overlay")]
pub use fit::FitOverlay;
use fit::FitPlugin;
#[cfg(feature = "fit_overlay")]
pub use fit::FitTargetOverlayConfig;
pub use fit::LookAt;
pub use fit::LookAtAndZoomToFit;
pub use fit::SetFitTarget;
pub use fit::ZoomBegin;
pub use fit::ZoomContext;
pub use fit::ZoomEnd;
pub use fit::ZoomReason;
pub use fit::ZoomToFit;
pub use free_cam::FreeCam;
pub use free_cam::FreeCamHomePose;
pub use free_cam::FreeCamKind;
use free_cam::FreeCamPlugin;
#[doc(hidden)]
pub use free_cam::FreeCamUpdateRequest;
pub use hana_kana::Easing;
pub use hana_kana::EasingCurve;
pub use hana_kana::EasingCurveBuilder;
pub use hana_kana::EasingCurveError;
pub use hana_kana::EasingInput;
pub use hana_kana::EasingInterpolation;
pub use hana_kana::EasingKnot;
pub use hana_kana::EasingOutput;
pub use hana_kana::EasingSlope;
pub use hana_kana::EasingSlopes;
pub use hana_kana::SequenceCommand;
pub use hana_kana::SequenceCommandOutcome;
pub use hana_kana::SequenceCommandRejected;
pub use hana_kana::SequenceCommandResponse;
pub use hana_kana::SequenceDirection;
pub use hana_kana::SequenceDriver;
pub use hana_kana::SequenceDriverTakeover;
pub use hana_kana::SequenceEasing;
pub use hana_kana::SequenceEasingError;
pub use hana_kana::SequenceEvaluation;
pub use hana_kana::SequenceMovement;
pub use hana_kana::SequenceOwner;
pub use hana_kana::SequenceOwnership;
pub use hana_kana::SequencePosition;
pub use hana_kana::SequenceScope;
pub use hana_kana::SequenceSourceState;
pub use hana_kana::SequenceStageId;
pub use hana_kana::SequenceStageSpan;
pub use hana_kana::SequenceStages;
pub use hana_kana::SequenceTime;
pub use initialization::Initialization;
pub use input::ActionBindingDescriptor;
pub use input::AxisResponse;
pub use input::BindingEngagement;
pub use input::BindingGate;
pub use input::BindingGates;
pub use input::BindingRoutePolicy;
pub use input::BindingsError;
pub use input::CameraControlAction;
pub use input::CameraControlActivation;
pub use input::CameraControlBinding;
pub use input::CameraControlBindingKind;
pub use input::CameraControlSummary;
pub use input::CameraHomed;
pub use input::CameraInputDisabled;
pub use input::CameraInputGamepadSelectionPolicy;
pub use input::CameraInputKind;
pub use input::CameraInputMetricKind;
pub use input::CameraInputMetricsMissing;
pub use input::CameraInputModeKind;
pub use input::CameraInputReportingDebounce;
pub use input::CameraInputRouting;
pub use input::CameraInputRoutingConfig;
pub use input::CameraInputScalePolicy;
pub use input::CameraInputSurfaceMetrics;
pub use input::CameraSemanticAction;
pub use input::CameraSlowMode;
pub use input::CameraSlowModeState;
pub use input::CoarseZoomDelta;
pub use input::ControlSpeed;
pub use input::Damping;
pub use input::FreeCamActiveDirections;
pub use input::FreeCamBindings;
pub use input::FreeCamBindingsBuilder;
pub use input::FreeCamChannels;
pub use input::FreeCamControlDirection;
pub use input::FreeCamGamepadLayout;
pub use input::FreeCamGamepadPreset;
pub use input::FreeCamHomeActionBindings;
pub use input::FreeCamInput;
pub use input::FreeCamInputContext;
pub use input::FreeCamInputGain;
pub use input::FreeCamInputMode;
pub use input::FreeCamInteractionEnded;
pub use input::FreeCamInteractionKind;
pub use input::FreeCamInteractionSourcesChanged;
pub use input::FreeCamInteractionSpeedChanged;
pub use input::FreeCamInteractionStarted;
pub use input::FreeCamInteractionState;
pub use input::FreeCamKeyboardMousePreset;
pub use input::FreeCamLookAction;
pub use input::FreeCamLookActionBindings;
pub use input::FreeCamLookBinding;
pub use input::FreeCamLookPitch;
pub use input::FreeCamManualInput;
pub use input::FreeCamManualInputWriter;
pub use input::FreeCamMouseLook;
pub use input::FreeCamPreset;
pub use input::FreeCamPresetKind;
pub use input::FreeCamRollAction;
pub use input::FreeCamRollActionBindings;
pub use input::FreeCamRollBinding;
pub use input::FreeCamTranslateAction;
pub use input::FreeCamTranslateActionBindings;
pub use input::FreeCamTranslateBinding;
pub use input::FreeCamTranslateKeys;
pub use input::GateInput;
pub use input::GatePolarity;
pub use input::HeldBinding;
pub use input::HeldCameraAction;
pub use input::ImpulseCameraAction;
pub use input::InputAxisTransform;
pub use input::InputBinding;
pub use input::InputBindingModifiers;
pub use input::InputBindingScale;
pub use input::InputDeadZone;
pub use input::InputDeltaScale;
pub use input::InputGain;
pub use input::InputIntent;
pub use input::InputMode;
use input::InputPlugin;
pub use input::IntentChannel;
pub use input::IntentChannels;
pub use input::InteractionSources;
pub use input::LookDelta;
pub use input::ManualInputSource;
pub use input::NoPositionFallback;
pub use input::OrbitCamBindingWithInputGain;
pub use input::OrbitCamBindings;
pub use input::OrbitCamBindingsBuilder;
pub use input::OrbitCamBlenderLikeKeyboardPreset;
pub use input::OrbitCamBlenderLikePreset;
pub use input::OrbitCamButtonDragZoom;
pub use input::OrbitCamButtonDragZoomAxis;
pub use input::OrbitCamControlRow;
pub use input::OrbitCamControlSummary;
pub use input::OrbitCamGamepadPreset;
pub use input::OrbitCamGamepadPresetBuilder;
pub use input::OrbitCamHomeActionBindings;
pub use input::OrbitCamInput;
pub use input::OrbitCamInputContext;
pub use input::OrbitCamInputGain;
pub use input::OrbitCamInputMode;
pub use input::OrbitCamInteractionEnded;
pub use input::OrbitCamInteractionKind;
pub use input::OrbitCamInteractionSourcesChanged;
pub use input::OrbitCamInteractionSpeedChanged;
pub use input::OrbitCamInteractionStarted;
pub use input::OrbitCamInteractionState;
pub use input::OrbitCamKeyboardPreset;
pub use input::OrbitCamManualInput;
pub use input::OrbitCamManualInputWriter;
pub use input::OrbitCamMouseDrag;
pub use input::OrbitCamMouseWheelZoom;
pub use input::OrbitCamOrbitAction;
pub use input::OrbitCamOrbitActionBindings;
pub use input::OrbitCamOrbitBinding;
pub use input::OrbitCamPanAction;
pub use input::OrbitCamPanActionBindings;
pub use input::OrbitCamPanBinding;
pub use input::OrbitCamPinchZoom;
pub use input::OrbitCamPreset;
pub use input::OrbitCamPresetKind;
pub use input::OrbitCamSimpleMouseKeyboardPreset;
pub use input::OrbitCamSimpleMousePreset;
pub use input::OrbitCamTouchBinding;
pub use input::OrbitCamTouchBindingConfig;
pub use input::OrbitCamTrackpadScroll;
pub use input::OrbitCamZoomBinding;
pub use input::OrbitCamZoomCoarseAction;
pub use input::OrbitCamZoomCoarseActionBindings;
pub use input::OrbitCamZoomSmoothAction;
pub use input::OrbitCamZoomSmoothActionBindings;
pub use input::OrbitDelta;
pub use input::PanDelta;
pub use input::PinchGestureZoom;
pub use input::ResetFreeCamToHome;
pub use input::ResetOrbitCamToHome;
pub use input::ResolvedCameraInputRoute;
pub use input::RollDelta;
pub use input::Sensitivity;
pub use input::SmoothZoomDelta;
pub use input::TranslateDelta;
pub use input::ZoomDelta;
pub use input::ZoomDirection;
pub use input::ZoomInversion;
pub use input::describe_controls;
pub use input::describe_controls_for;
pub use input::describe_orbit_cam_controls;
pub use operation::AnglePairLimit;
pub use operation::Focus;
pub use operation::Limit;
pub use operation::LookAngles;
pub use operation::Operation;
pub use operation::OrbitAngles;
pub use operation::Position;
pub use operation::Radius;
pub use operation::RegionLimit;
pub use operation::Roll;
pub use operation::ScalarLimit;
pub use operation::Smoothable;
pub use orbit_cam::GamepadInputGain;
pub use orbit_cam::MouseInputGain;
pub use orbit_cam::OrbitCam;
pub use orbit_cam::OrbitCamChannels;
pub use orbit_cam::OrbitCamHomePose;
pub use orbit_cam::OrbitCamKind;
use orbit_cam::OrbitCamPlugin;
pub use orbit_cam::OrbitCamSystemSet;
#[doc(hidden)]
pub use orbit_cam::OrbitCamUpdateRequest;
pub use orbit_cam::SmoothScrollInputGain;
pub use orbit_cam::UpsideDownPolicy;
pub use system_sets::CameraControllerSystemSet;
pub use system_sets::CameraInputPhase;
use system_sets::LagrangeSystemSetsPlugin;
pub use time_source::TimeSource;

/// Bevy plugin for the lagrange cameras. Registers shared camera
/// infrastructure and both camera kinds, `OrbitCam` and `FreeCam`.
/// # Example
/// ```no_run
/// # use bevy::prelude::*;
/// # use hana_lagrange::{LagrangePlugin, OrbitCam};
/// fn main() {
///     App::new()
///         .add_plugins(DefaultPlugins)
///         .add_plugins(LagrangePlugin)
///         .run();
/// }
/// ```
pub struct LagrangePlugin;

impl Plugin for LagrangePlugin {
    fn build(&self, app: &mut App) {
        // Shared camera infrastructure, used across all camera kinds. Each
        // camera kind registers its own enhanced-input context in its plugin.
        app.add_plugins((
            LagrangeSystemSetsPlugin,
            AnimationPlugin,
            InputPlugin,
            FitPlugin,
        ));

        // Per-camera-kind plugins.
        app.add_plugins((OrbitCamPlugin, FreeCamPlugin));
    }
}
