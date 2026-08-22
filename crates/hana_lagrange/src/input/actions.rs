use bevy::prelude::Reflect;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy_enhanced_input::prelude::InputAction;

mod sealed {
    pub trait Sealed {}
}

/// Marker trait for semantic Lagrange camera actions.
///
/// This trait is sealed; downstream crates cannot implement it for their own
/// action types.
pub trait CameraSemanticAction: InputAction + sealed::Sealed {}

/// Marker trait for held Lagrange camera actions.
///
/// This trait is sealed. Held action types are defined by this crate for
/// [`OrbitCamKind`] and [`FreeCamKind`].
///
/// [`FreeCamKind`]: crate::FreeCamKind
/// [`OrbitCamKind`]: crate::OrbitCamKind
pub trait HeldCameraAction: CameraSemanticAction {}

/// Marker trait for impulse Lagrange camera actions.
pub trait ImpulseCameraAction: CameraSemanticAction {}

/// Enhanced-input action for orbit intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(Vec2)]
pub struct OrbitCamOrbitAction;

/// Enhanced-input action for pan intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(Vec2)]
pub struct OrbitCamPanAction;

/// Enhanced-input action for coarse zoom intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(f32)]
pub struct OrbitCamZoomCoarseAction;

/// Enhanced-input action for smooth zoom intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(f32)]
pub struct OrbitCamZoomSmoothAction;

/// Enhanced-input action for `FreeCam` translation intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(Vec3)]
pub struct FreeCamTranslateAction;

/// Enhanced-input action for `FreeCam` look intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(Vec2)]
pub struct FreeCamLookAction;

/// Enhanced-input action for `FreeCam` roll intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(f32)]
pub struct FreeCamRollAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct FreeCamLookButtonAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct FreeCamTranslateEngagedAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct FreeCamRollEngagedAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct FreeCamSlowModeToggleAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct FreeCamGateAction;

/// Enhanced-input action that resets a `FreeCam` to its home pose while active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct FreeCamHomeAction;

/// Enhanced-input action that resets an `OrbitCam` to its home pose while active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct OrbitCamHomeAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct OrbitCamOrbitEngagedAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct OrbitCamPanEngagedAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct OrbitCamZoomEngagedAction;

/// Slow (gated) orbit motion — routed separately so the active speed falls out
/// of which motion action is firing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(Vec2)]
pub(crate) struct OrbitCamOrbitSlowAction;

/// Slow (gated) pan motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(Vec2)]
pub(crate) struct OrbitCamPanSlowAction;

/// Slow (gated) smooth zoom motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction, Reflect)]
#[action_output(f32)]
pub(crate) struct OrbitCamZoomSmoothSlowAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct OrbitCamGateAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(Vec2)]
pub(crate) struct OrbitCamAdapterOrbitAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(Vec2)]
pub(crate) struct OrbitCamAdapterPanAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(f32)]
pub(crate) struct OrbitCamAdapterZoomCoarseAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(f32)]
pub(crate) struct OrbitCamAdapterZoomSmoothAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, InputAction)]
#[action_output(bool)]
pub(crate) struct OrbitCamSlowModeToggleAction;

macro_rules! impl_camera_action {
    ($action:ty) => {
        impl sealed::Sealed for $action {}
        impl CameraSemanticAction for $action {}
    };
}

impl_camera_action!(OrbitCamOrbitAction);
impl_camera_action!(OrbitCamPanAction);
impl_camera_action!(OrbitCamZoomCoarseAction);
impl_camera_action!(OrbitCamZoomSmoothAction);
impl_camera_action!(FreeCamTranslateAction);
impl_camera_action!(FreeCamLookAction);
impl_camera_action!(FreeCamRollAction);
impl_camera_action!(FreeCamLookButtonAction);
impl_camera_action!(FreeCamTranslateEngagedAction);
impl_camera_action!(FreeCamRollEngagedAction);
impl_camera_action!(FreeCamSlowModeToggleAction);
impl_camera_action!(FreeCamHomeAction);
impl_camera_action!(OrbitCamOrbitEngagedAction);
impl_camera_action!(OrbitCamPanEngagedAction);
impl_camera_action!(OrbitCamZoomEngagedAction);
impl_camera_action!(OrbitCamHomeAction);
impl_camera_action!(OrbitCamOrbitSlowAction);
impl_camera_action!(OrbitCamPanSlowAction);
impl_camera_action!(OrbitCamZoomSmoothSlowAction);
impl_camera_action!(OrbitCamAdapterOrbitAction);
impl_camera_action!(OrbitCamAdapterPanAction);
impl_camera_action!(OrbitCamAdapterZoomCoarseAction);
impl_camera_action!(OrbitCamAdapterZoomSmoothAction);

impl HeldCameraAction for OrbitCamOrbitAction {}
impl HeldCameraAction for OrbitCamPanAction {}
impl HeldCameraAction for OrbitCamZoomSmoothAction {}
impl HeldCameraAction for FreeCamTranslateAction {}
impl HeldCameraAction for FreeCamLookAction {}
impl HeldCameraAction for FreeCamRollAction {}

impl ImpulseCameraAction for OrbitCamZoomCoarseAction {}
impl ImpulseCameraAction for FreeCamHomeAction {}
impl ImpulseCameraAction for OrbitCamHomeAction {}
