//! `OrbitCam` component, systems, and helpers.

mod constants;
pub(crate) mod controller;
mod drag_state;
mod input;
mod intent;
mod orbit_transform;
mod presets;

use bevy::camera::CameraUpdateSystems;
use bevy::prelude::App;
use bevy::prelude::Camera3d;
use bevy::prelude::Component;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::SystemSet;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::transform::TransformSystems;
use bevy_enhanced_input::prelude::InputContextAppExt;
use constants::DEFAULT_ORBIT_SMOOTHNESS;
use constants::DEFAULT_PAN_SMOOTHNESS;
use constants::DEFAULT_ZOOM_LOWER_LIMIT;
use constants::DEFAULT_ZOOM_SMOOTHNESS;
use controller::orbit_cam;
use drag_state::OrbitDragState;
pub use input::CameraInputGamepadSelectionPolicy;
pub use input::GamepadInputGain;
pub use input::MouseInputGain;
pub use input::OrbitCamBindingWithInputGain;
pub use input::OrbitCamBindings;
pub use input::OrbitCamBindingsBuilder;
pub use input::OrbitCamBlenderLikeKeyboardPreset;
pub use input::OrbitCamBlenderLikePreset;
pub use input::OrbitCamButtonDragZoom;
pub use input::OrbitCamButtonDragZoomAxis;
pub use input::OrbitCamGamepadPreset;
pub use input::OrbitCamGamepadPresetBuilder;
pub use input::OrbitCamHomeActionBindings;
pub(super) use input::OrbitCamInputAdapterPlugin;
pub use input::OrbitCamInputGain;
pub use input::OrbitCamKeyboardPreset;
pub use input::OrbitCamLineScroll;
pub use input::OrbitCamMouseDrag;
pub use input::OrbitCamMouseWheelZoom;
pub use input::OrbitCamOrbitActionBindings;
pub use input::OrbitCamOrbitBinding;
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
pub use input::OrbitCamZoomCoarseActionBindings;
pub use input::OrbitCamZoomSmoothActionBindings;
pub use input::PinchGestureZoom;
pub use input::SmoothScrollInputGain;
pub use input::ZoomInversion;
pub use intent::CoarseZoomDelta;
pub use intent::OrbitCamChannels;
pub use intent::OrbitCamInput;
pub use intent::OrbitDelta;
pub use intent::PanDelta;
pub use intent::SmoothZoomDelta;
pub use intent::ZoomDelta;
pub(crate) use orbit_transform::transform_from_orbit;

use super::input::IntentChannel;
use super::input::OrbitCamInputContext;
use super::input::OrbitCamInputMode;
use crate::CameraBasis;
use crate::CameraHomeKind;
use crate::CameraKind;
use crate::Initialization;
use crate::camera_home;
use crate::constants::DEFAULT_ORBIT_ANGLE;
use crate::constants::DEFAULT_TARGET_RADIUS;
use crate::fit::UnifiedFitRequestObserversPlugin;
use crate::input::DEFAULT_INPUT_GAIN;
use crate::input::OrbitCamInteractionStarted;
use crate::operation::AnglePairLimit;
use crate::operation::Focus;
use crate::operation::Operation;
use crate::operation::OrbitAngles;
use crate::operation::Radius;
use crate::operation::RegionLimit;
use crate::operation::ScalarLimit;
use crate::system_sets::CameraControllerSystemSet;
use crate::time_source::TimeSource;

/// Registers the `OrbitCam` controller and its input pipeline.
pub(super) struct OrbitCamPlugin;

impl Plugin for OrbitCamPlugin {
    fn build(&self, app: &mut App) { OrbitCamKind::add_camera_kind_systems(app); }
}

/// Compile-time type-family key for `OrbitCam`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct OrbitCamKind;

impl CameraKind for OrbitCamKind {
    type Camera = OrbitCam;

    fn add_controller_systems(app: &mut App) {
        app.register_type::<Operation<OrbitAngles>>()
            .register_type::<Operation<Focus>>()
            .register_type::<Operation<Radius>>()
            .register_type::<OrbitCamInput>()
            .register_type::<OrbitCamInputMode>()
            .register_type::<IntentChannel<OrbitDelta>>()
            .register_type::<IntentChannel<PanDelta>>()
            .register_type::<IntentChannel<ZoomDelta>>()
            .add_input_context::<OrbitCamInputContext>()
            .add_plugins(OrbitCamInputAdapterPlugin)
            .add_systems(
                PostUpdate,
                orbit_cam
                    .in_set(OrbitCamSystemSet)
                    .in_set(CameraControllerSystemSet)
                    .before(TransformSystems::Propagate)
                    .before(CameraUpdateSystems),
            );
        camera_home::add_home_systems::<Self>(app);
        camera_home::add_orbit_cam_home_reset_systems(app);
    }

    fn add_animate_to_fit_systems(app: &mut App) {
        if !app.is_plugin_added::<UnifiedFitRequestObserversPlugin>() {
            app.add_plugins(UnifiedFitRequestObserversPlugin);
        }
    }

    fn add_zoom_to_fit_systems(app: &mut App) {
        if !app.is_plugin_added::<UnifiedFitRequestObserversPlugin>() {
            app.add_plugins(UnifiedFitRequestObserversPlugin);
        }
    }

    fn add_look_at_systems(app: &mut App) {
        if !app.is_plugin_added::<UnifiedFitRequestObserversPlugin>() {
            app.add_plugins(UnifiedFitRequestObserversPlugin);
        }
    }
}

impl CameraHomeKind for OrbitCamKind {
    type HomePose = OrbitCamHomePose;
    type InteractionStarted = OrbitCamInteractionStarted;

    fn capture_home(camera: &Self::Camera) -> Self::HomePose {
        OrbitCamHomePose::from_current(camera)
    }

    fn apply_home(camera: &mut Self::Camera, home: &Self::HomePose) {
        camera.orbit.set_target(home.orbit);
        camera.pan.set_target(home.pan);
        camera.zoom.set_target(home.zoom);
    }
}

/// Schedule label for the private `OrbitCam` controller system.
///
/// Use this in `PostUpdate` to run systems before the controller reads
/// `OrbitCam` and `OrbitCamInput`, or after it writes the camera `Transform`.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct OrbitCamSystemSet;

/// Whether the camera is allowed to orbit past the poles into an upside-down orientation.
#[derive(Clone, PartialEq, Eq, Debug, Reflect, Copy, Default)]
pub enum UpsideDownPolicy {
    /// Camera may orbit upside down.
    Allow,
    /// Camera pitch is clamped to prevent going upside down.
    #[default]
    Prevent,
}

/// One-shot controller request for recalculating the camera transform.
#[doc(hidden)]
#[derive(Clone, PartialEq, Eq, Debug, Reflect, Copy, Default)]
pub enum OrbitCamUpdateRequest {
    /// No forced update was requested.
    #[default]
    None,
    /// Force one transform update on the next controller pass.
    ForceUpdate,
}

/// Tags an entity as capable of panning and orbiting.
///
/// The camera's driven state lives in three [`Operation`]s — `orbit`
/// (yaw/pitch), `pan` (focus), and `zoom` (radius) — each pairing a smoothed
/// current/target value with its sensitivity, damping, and limit.
///
/// Use preset constructors such as [`OrbitCam::blender_like`] for default-pose
/// input modes. Preset bundle constructors and pose constructors stay separate;
/// to start from an explicit pose and use a preset, insert both components.
///
/// # Example
/// ```no_run
/// # use bevy::prelude::*;
/// # use hana_lagrange::{LagrangePlugin, OrbitCam, OrbitCamInputMode, OrbitCamPreset};
/// # fn main() {
/// #     App::new()
/// #         .add_plugins(DefaultPlugins)
/// #         .add_plugins(LagrangePlugin)
/// #         .add_systems(Startup, setup)
/// #         .run();
/// # }
/// fn setup(mut commands: Commands) {
///     commands.spawn((
///         Transform::default(),
///         OrbitCam::from_pose(Vec3::ZERO, (0.0, 0.4), 5.0),
///         OrbitCamInputMode::with_preset(OrbitCamPreset::blender_like()),
///     ));
/// }
/// ```
#[derive(Component, Reflect, Copy, Clone, Debug, PartialEq)]
#[reflect(Component)]
#[require(
    Camera3d,
    Transform,
    CameraBasis,
    OrbitDragState,
    OrbitCamInput,
    OrbitCamInputContext,
    OrbitCamInputMode,
    TimeSource
)]
pub struct OrbitCam {
    /// The orbit angles (yaw, pitch) eased toward their target, with the orbit
    /// motion's sensitivity, damping, and per-axis angle limits. Set the target
    /// directly (`orbit.set_target((yaw, pitch))`) for programmatic
    /// orbiting.
    pub orbit:              Operation<OrbitAngles>,
    /// The focus point eased toward its target, with the pan motion's
    /// sensitivity, damping, and region limit. Set the target directly
    /// (`pan.set_target(focus)`) for programmatic panning or follow behavior.
    pub pan:                Operation<Focus>,
    /// The orbit radius eased toward its target, with the zoom motion's
    /// sensitivity, damping, and scalar limit. For an orthographic camera this
    /// drives the projection scale instead of a distance. Damping applies only to
    /// line-based scroll; pixel-based scroll is already smooth. Set the target
    /// directly (`zoom.set_target(radius)`) for programmatic zooming.
    pub zoom:               Operation<Radius>,
    /// Whether to allow the camera to go upside down.
    /// Defaults to `UpsideDownPolicy::Prevent`.
    pub upside_down_policy: UpsideDownPolicy,
    /// How the start pose is established on the first controller pass. Defaults to
    /// [`Initialization::FromTransform`]; [`OrbitCam::from_pose`] sets
    /// [`Initialization::FromPose`].
    pub initialization:     Initialization,
    /// One-shot update request used by [`OrbitCam::force_update`].
    #[doc(hidden)]
    pub update_request:     OrbitCamUpdateRequest,
}

impl OrbitCam {
    /// Returns an `OrbitCam` whose start pose comes from explicit angles, focus,
    /// and radius rather than the entity's `Transform`. Seeds the three
    /// operations and sets [`Initialization::FromPose`]; the controller writes the
    /// `Transform` to match on its first pass.
    #[must_use]
    pub fn from_pose(
        focus: impl Into<Focus>,
        angles: impl Into<OrbitAngles>,
        radius: impl Into<Radius>,
    ) -> Self {
        let mut camera = Self::default();
        camera.orbit.snap_to(angles);
        camera.pan.snap_to(focus);
        camera.zoom.snap_to(radius);
        camera.initialization = Initialization::FromPose;
        camera
    }

    /// Requests one transform update on the next controller pass.
    ///
    /// Use this after mutating current camera state or projection state directly,
    /// when no target-value change would otherwise make the controller recalculate
    /// the transform.
    pub const fn force_update(&mut self) {
        self.update_request = OrbitCamUpdateRequest::ForceUpdate;
    }

    fn consume_update_request(&mut self) -> OrbitCamUpdateRequest {
        core::mem::take(&mut self.update_request)
    }
}

impl Default for OrbitCam {
    fn default() -> Self {
        Self {
            orbit:              Operation::new(
                OrbitAngles {
                    yaw:   DEFAULT_ORBIT_ANGLE,
                    pitch: DEFAULT_ORBIT_ANGLE,
                },
                DEFAULT_INPUT_GAIN,
                DEFAULT_ORBIT_SMOOTHNESS,
                AnglePairLimit::default(),
            ),
            pan:                Operation::new(
                Focus(Vec3::ZERO),
                DEFAULT_INPUT_GAIN,
                DEFAULT_PAN_SMOOTHNESS,
                RegionLimit::default(),
            ),
            zoom:               Operation::new(
                Radius(DEFAULT_TARGET_RADIUS),
                DEFAULT_INPUT_GAIN,
                DEFAULT_ZOOM_SMOOTHNESS,
                ScalarLimit::Clamp {
                    min: DEFAULT_ZOOM_LOWER_LIMIT,
                    max: f32::INFINITY,
                },
            ),
            upside_down_policy: UpsideDownPolicy::Prevent,
            initialization:     Initialization::FromTransform,
            update_request:     OrbitCamUpdateRequest::None,
        }
    }
}

/// The `OrbitCam` pose restored by the home/reset action.
///
/// Captured from the camera's current operation values. Insert one before spawn to define a
/// fixed custom home pose instead.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub struct OrbitCamHomePose {
    /// Home orbit angles the camera returns to.
    pub orbit: OrbitAngles,
    /// Home focus point the camera returns to.
    pub pan:   Focus,
    /// Home radius the camera returns to.
    pub zoom:  Radius,
}

impl OrbitCamHomePose {
    pub(crate) const fn from_current(orbit_cam: &OrbitCam) -> Self {
        Self {
            orbit: orbit_cam.orbit.current(),
            pan:   orbit_cam.pan.current(),
            zoom:  orbit_cam.zoom.current(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET_FOCUS: Vec3 = Vec3::new(1.0, 2.0, 3.0);
    const TARGET_PITCH: f32 = 0.75;
    const TARGET_RADIUS: Radius = Radius(12.0);
    const TARGET_YAW: f32 = 1.5;

    #[test]
    fn home_pose_reads_operation_currents() {
        let mut orbit_cam = OrbitCam::default();
        orbit_cam.orbit.set_target(OrbitAngles {
            yaw:   TARGET_YAW,
            pitch: TARGET_PITCH,
        });
        orbit_cam.pan.set_target(Focus(TARGET_FOCUS));
        orbit_cam.zoom.set_target(TARGET_RADIUS);

        let home = OrbitCamHomePose::from_current(&orbit_cam);

        assert_ne!(home.orbit, orbit_cam.orbit.target());
        assert_ne!(home.pan, orbit_cam.pan.target());
        assert_ne!(home.zoom, orbit_cam.zoom.target());
        assert_eq!(home.orbit, orbit_cam.orbit.current());
        assert_eq!(home.pan, orbit_cam.pan.current());
        assert_eq!(home.zoom, orbit_cam.zoom.current());
    }
}
