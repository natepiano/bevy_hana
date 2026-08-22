//! Free-flight `FreeCam` camera kind.

mod constants;
pub(crate) mod controller;
mod input;
mod intent;
mod presets;

use bevy::camera::CameraUpdateSystems;
use bevy::prelude::App;
use bevy::prelude::Camera3d;
use bevy::prelude::Component;
use bevy::prelude::EulerRot;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Projection;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Transform;
use bevy::prelude::Vec3;
use bevy::transform::TransformSystems;
use bevy_enhanced_input::prelude::InputContextAppExt;
pub use input::FreeCamBindings;
pub use input::FreeCamBindingsBuilder;
pub use input::FreeCamGamepadLayout;
pub use input::FreeCamGamepadPreset;
pub use input::FreeCamHomeActionBindings;
pub(super) use input::FreeCamInputAdapterPlugin;
pub use input::FreeCamInputGain;
pub use input::FreeCamKeyboardMousePreset;
pub use input::FreeCamLookActionBindings;
pub use input::FreeCamLookBinding;
pub use input::FreeCamLookPitch;
pub use input::FreeCamMouseLook;
pub use input::FreeCamPreset;
pub use input::FreeCamPresetKind;
pub use input::FreeCamRollActionBindings;
pub use input::FreeCamRollBinding;
pub use input::FreeCamTranslateActionBindings;
pub use input::FreeCamTranslateBinding;
pub use input::FreeCamTranslateKeys;
pub use intent::FreeCamChannels;
pub use intent::FreeCamInput;
pub use intent::LookDelta;
pub use intent::RollDelta;
pub use intent::TranslateDelta;

use self::constants::DEFAULT_FREE_LOOK_SENSITIVITY;
use self::constants::DEFAULT_FREE_LOOK_SMOOTHNESS;
use self::constants::DEFAULT_FREE_PITCH_LIMIT;
use self::constants::DEFAULT_FREE_ROLL_SENSITIVITY;
use self::constants::DEFAULT_FREE_ROLL_SMOOTHNESS;
use self::constants::DEFAULT_FREE_TRANSLATE_SENSITIVITY;
use self::constants::DEFAULT_FREE_TRANSLATE_SMOOTHNESS;
use self::controller::free_cam;
use crate::CameraBasis;
use crate::CameraHomeKind;
use crate::CameraKind;
use crate::Initialization;
use crate::camera_home;
use crate::fit::UnifiedFitRequestObserversPlugin;
use crate::input::FreeCamInputContext;
use crate::input::FreeCamInputMode;
use crate::input::FreeCamInteractionStarted;
use crate::input::IntentChannel;
use crate::operation::AnglePairLimit;
use crate::operation::LookAngles;
use crate::operation::Operation;
use crate::operation::Position;
use crate::operation::RegionLimit;
use crate::operation::Roll;
use crate::operation::ScalarLimit;
use crate::orbit_cam;
use crate::orbit_cam::OrbitCamHomePose;
use crate::time_source::TimeSource;

/// Registers `FreeCam` systems and its enhanced-input context.
pub(super) struct FreeCamPlugin;

impl Plugin for FreeCamPlugin {
    fn build(&self, app: &mut App) { FreeCamKind::add_camera_kind_systems(app); }
}

/// Compile-time type-family key for `FreeCam`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct FreeCamKind;

impl CameraKind for FreeCamKind {
    type Camera = FreeCam;

    fn add_controller_systems(app: &mut App) {
        app.register_type::<Operation<LookAngles>>()
            .register_type::<Operation<Position>>()
            .register_type::<Operation<Roll>>()
            .register_type::<FreeCamInput>()
            .register_type::<FreeCamInputMode>()
            .register_type::<IntentChannel<TranslateDelta>>()
            .register_type::<IntentChannel<LookDelta>>()
            .register_type::<IntentChannel<RollDelta>>()
            .add_input_context::<FreeCamInputContext>()
            .add_plugins(FreeCamInputAdapterPlugin)
            .add_systems(
                PostUpdate,
                free_cam
                    .in_set(crate::CameraControllerSystemSet)
                    .before(TransformSystems::Propagate)
                    .before(CameraUpdateSystems),
            );
        camera_home::add_home_systems::<Self>(app);
        camera_home::add_free_cam_home_reset_systems(app);
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

impl CameraHomeKind for FreeCamKind {
    type HomePose = FreeCamHomePose;
    type InteractionStarted = FreeCamInteractionStarted;

    fn capture_home(camera: &Self::Camera) -> Self::HomePose {
        FreeCamHomePose::from_current(camera)
    }

    fn apply_home(camera: &mut Self::Camera, home: &Self::HomePose) {
        camera.translate.set_target(home.position);
        camera.look.set_target(home.look);
        camera.roll.set_target(home.roll);
    }
}

/// One-shot controller request for recalculating the `FreeCam` transform.
#[doc(hidden)]
#[derive(Clone, PartialEq, Eq, Debug, Reflect, Copy, Default)]
pub enum FreeCamUpdateRequest {
    /// No forced update was requested.
    #[default]
    None,
    /// Force one transform update on the next controller pass.
    ForceUpdate,
}

/// Tags an entity as a free-flight camera that can turn and translate freely.
///
/// Use [`FreeCam::with_preset`] for a default-pose camera with a built-in input
/// preset. Preset bundle constructors and pose constructors stay separate; to
/// start from an explicit pose and use a preset, insert both components.
///
/// # Example
/// ```no_run
/// # use bevy::prelude::*;
/// # use hana_lagrange::{FreeCam, FreeCamInputMode, FreeCamPreset, LagrangePlugin};
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
///         FreeCam::from_pose(Vec3::ZERO, (0.0, 0.0), 0.0),
///         FreeCamInputMode::with_preset(FreeCamPreset::keyboard_mouse()),
///     ));
/// }
/// ```
#[derive(Component, Reflect, Copy, Clone, Debug, PartialEq)]
#[reflect(Component)]
#[require(
    Camera3d,
    Transform,
    CameraBasis,
    FreeCamInput,
    FreeCamInputContext,
    FreeCamInputMode,
    TimeSource
)]
pub struct FreeCam {
    /// The camera position eased toward its target, with translation motion's
    /// sensitivity, damping, and region limit. Set the target directly
    /// (`translate.set_target(Vec3::new(x, y, z))`) for programmatic translation.
    pub translate:      Operation<Position>,
    /// The free-look angles eased toward their target, with look motion's
    /// sensitivity, damping, and per-axis angle limits. Set the target directly
    /// (`look.set_target((yaw, pitch))`) for programmatic turning.
    pub look:           Operation<LookAngles>,
    /// The roll angle eased toward its target, with roll motion's sensitivity,
    /// damping, and scalar limit. Set the target directly (`roll.set_target(angle)`)
    /// for programmatic roll.
    pub roll:           Operation<Roll>,
    /// How the start pose is established on the first controller pass. Defaults
    /// to [`Initialization::FromTransform`]; [`FreeCam::from_pose`] sets
    /// [`Initialization::FromPose`].
    pub initialization: Initialization,
    /// One-shot update request used by [`FreeCam::force_update`].
    #[doc(hidden)]
    pub update_request: FreeCamUpdateRequest,
}

impl FreeCam {
    /// Returns a `FreeCam` whose start pose comes from explicit position, look,
    /// and roll state rather than the entity's `Transform`. Seeds the three
    /// operations and sets [`Initialization::FromPose`] for the controller that
    /// consumes `FreeCam` initialization.
    #[must_use]
    pub fn from_pose(
        position: impl Into<Position>,
        look: impl Into<LookAngles>,
        roll: impl Into<Roll>,
    ) -> Self {
        let mut camera = Self::default();
        camera.translate.snap_to(position);
        camera.look.snap_to(look);
        camera.roll.snap_to(roll);
        camera.initialization = Initialization::FromPose;
        camera
    }

    /// Returns a free-flight camera with yaw unrestricted and pitch clamped
    /// short of vertical.
    #[must_use]
    pub fn pitch_limited() -> Self {
        let mut camera = Self::default();
        camera.look.limit_mut().pitch = ScalarLimit::Clamp {
            min: -DEFAULT_FREE_PITCH_LIMIT,
            max: DEFAULT_FREE_PITCH_LIMIT,
        };
        camera
    }

    /// Returns a pitch-limited camera whose roll is locked to the horizon.
    #[must_use]
    pub fn horizon_locked() -> Self {
        let mut camera = Self::pitch_limited();
        camera.roll.snap_to(Roll::default());
        *camera.roll.limit_mut() = ScalarLimit::Clamp { min: 0.0, max: 0.0 };
        camera
    }
}

impl Default for FreeCam {
    fn default() -> Self {
        Self {
            translate:      Operation::new(
                Position(Vec3::ZERO),
                DEFAULT_FREE_TRANSLATE_SENSITIVITY,
                DEFAULT_FREE_TRANSLATE_SMOOTHNESS,
                RegionLimit::default(),
            ),
            look:           Operation::new(
                LookAngles::default(),
                DEFAULT_FREE_LOOK_SENSITIVITY,
                DEFAULT_FREE_LOOK_SMOOTHNESS,
                AnglePairLimit::default(),
            ),
            roll:           Operation::new(
                Roll::default(),
                DEFAULT_FREE_ROLL_SENSITIVITY,
                DEFAULT_FREE_ROLL_SMOOTHNESS,
                ScalarLimit::default(),
            ),
            initialization: Initialization::FromTransform,
            update_request: FreeCamUpdateRequest::None,
        }
    }
}
impl FreeCam {
    /// Requests one transform update on the next controller pass.
    ///
    /// Use this after mutating current camera state directly, when no
    /// target-value change would otherwise make the controller recalculate the
    /// transform.
    pub const fn force_update(&mut self) {
        self.update_request = FreeCamUpdateRequest::ForceUpdate;
    }

    fn consume_update_request(&mut self) -> FreeCamUpdateRequest {
        core::mem::take(&mut self.update_request)
    }
}

/// The `FreeCam` pose restored by the home/reset action.
///
/// Captured from the start pose on the first controller pass and initially provisional
/// (see [`CameraHomePending`](crate::CameraHomePending)): a completed camera animation upgrades it
/// to the settled landing pose, and the first genuine interaction locks it. Insert one before spawn
/// to define a fixed custom home pose instead; an app-provided pose is authoritative and never
/// upgraded.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub struct FreeCamHomePose {
    /// Home position the camera returns to.
    pub position: Position,
    /// Home look angles the camera returns to.
    pub look:     LookAngles,
    /// Home roll the camera returns to.
    pub roll:     Roll,
}

impl FreeCamHomePose {
    pub(crate) const fn from_current(free_cam: &FreeCam) -> Self {
        Self {
            position: free_cam.translate.current(),
            look:     free_cam.look.current(),
            roll:     free_cam.roll.current(),
        }
    }

    /// Returns the equivalent free-flight home pose for an orbit-camera home
    /// pose in the same camera basis.
    ///
    /// `OrbitCam` represents a view as focus + yaw + pitch + radius. `FreeCam`
    /// represents the same view as position + look + roll, so this conversion is
    /// useful when explicit camera-mode switch policy preserves Home behavior.
    /// The projection is needed because orthographic `OrbitCam` uses zoom as
    /// projection scale, not physical camera distance.
    #[must_use]
    pub fn from_orbit_home(
        home: OrbitCamHomePose,
        basis: CameraBasis,
        projection: &Projection,
    ) -> Self {
        let transform = orbit_cam::transform_from_orbit(
            home.orbit.yaw,
            home.orbit.pitch,
            home.zoom.0,
            home.pan.0,
            projection,
            basis.axes(),
        );
        Self::from_transform(&transform, basis)
    }

    fn from_transform(transform: &Transform, basis: CameraBasis) -> Self {
        let local_rotation = basis.rotation().inverse() * transform.rotation;
        let (yaw, pitch, roll) = local_rotation.to_euler(EulerRot::YXZ);
        Self {
            position: Position(transform.translation),
            look:     LookAngles { yaw, pitch: -pitch },
            roll:     Roll(roll),
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::OrthographicProjection;
    use bevy::prelude::PerspectiveProjection;

    use super::*;
    use crate::operation::Focus;
    use crate::operation::OrbitAngles;
    use crate::operation::Radius;

    const POSE_TOLERANCE: f32 = 0.00001;

    fn assert_vec3_close(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() <= POSE_TOLERANCE,
            "expected {expected:?}, got {actual:?}"
        );
    }

    fn assert_f32_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= POSE_TOLERANCE,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn default_free_cam_is_free_flight() {
        let camera = FreeCam::default();

        assert_eq!(camera.look.limit(), AnglePairLimit::default());
        assert_eq!(camera.roll.limit(), ScalarLimit::default());
    }

    #[test]
    fn pitch_limited_clamps_pitch_only() {
        let camera = FreeCam::pitch_limited();

        assert_eq!(camera.look.limit().yaw, ScalarLimit::None);
        assert_eq!(
            camera.look.limit().pitch,
            ScalarLimit::Clamp {
                min: -DEFAULT_FREE_PITCH_LIMIT,
                max: DEFAULT_FREE_PITCH_LIMIT,
            }
        );
        assert_eq!(camera.roll.limit(), ScalarLimit::None);
    }

    #[test]
    fn horizon_locked_clamps_pitch_and_locks_roll() {
        let camera = FreeCam::horizon_locked();

        assert_eq!(
            camera.look.limit().pitch,
            ScalarLimit::Clamp {
                min: -DEFAULT_FREE_PITCH_LIMIT,
                max: DEFAULT_FREE_PITCH_LIMIT,
            }
        );
        assert_eq!(camera.roll.current(), Roll::default());
        assert_eq!(camera.roll.target(), Roll::default());
        assert_eq!(
            camera.roll.limit(),
            ScalarLimit::Clamp { min: 0.0, max: 0.0 }
        );
    }

    #[test]
    fn free_home_pose_from_orbit_home_matches_perspective_orbit_view() {
        let home = OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   std::f32::consts::FRAC_PI_2,
                pitch: 0.0,
            },
            pan:   Focus(Vec3::new(1.0, 2.0, 3.0)),
            zoom:  Radius(5.0),
        };
        let projection = Projection::Perspective(PerspectiveProjection::default());

        let free_home = FreeCamHomePose::from_orbit_home(home, CameraBasis::Y_UP, &projection);

        assert_vec3_close(free_home.position.0, Vec3::new(6.0, 2.0, 3.0));
        assert_f32_close(free_home.look.yaw, home.orbit.yaw);
        assert_f32_close(free_home.look.pitch, home.orbit.pitch);
        assert_f32_close(free_home.roll.0, 0.0);
    }

    #[test]
    fn free_home_pose_from_orbit_home_uses_orthographic_camera_distance() {
        let home = OrbitCamHomePose {
            orbit: OrbitAngles {
                yaw:   0.0,
                pitch: 0.0,
            },
            pan:   Focus(Vec3::new(1.0, 2.0, 3.0)),
            zoom:  Radius(4.0),
        };
        let projection = Projection::Orthographic(OrthographicProjection {
            near: 10.0,
            far: 30.0,
            ..OrthographicProjection::default_3d()
        });

        let free_home = FreeCamHomePose::from_orbit_home(home, CameraBasis::Y_UP, &projection);

        assert_vec3_close(free_home.position.0, Vec3::new(1.0, 2.0, 23.0));
        assert_f32_close(free_home.look.yaw, 0.0);
        assert_f32_close(free_home.look.pitch, 0.0);
        assert_f32_close(free_home.roll.0, 0.0);
    }
}
