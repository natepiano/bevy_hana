//! Capability: spawn an `OrbitCam` entity tagged with [`FairyDustOrbitCam`]
//! so camera-attached capabilities (e.g. stable-transparency) can find it.
//! `LagrangePlugin` itself is installed by the deferred Fairy Dust baseline
//! when the first builder operation is applied.

use std::sync::Mutex;

use bevy::prelude::App;
use bevy::prelude::Bundle;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Startup;
use bevy::prelude::Vec3;
use hana_lagrange::AnglePairLimit;
use hana_lagrange::Initialization;
use hana_lagrange::OrbitCam;
use hana_lagrange::ScalarLimit;
use hana_lagrange::UpsideDownPolicy;

use crate::constants::EXAMPLE_ORBIT_CAM_PITCH_LIMIT;
use crate::constants::EXAMPLE_ORBIT_CAM_ZOOM_LOWER_LIMIT;
use crate::constants::EXAMPLE_ORBIT_CAM_ZOOM_UPPER_LIMIT;

/// Marker identifying the `OrbitCam` that `fairy_dust` capabilities target.
///
/// [`crate::SprinkleBuilder::with_orbit_cam_configured`],
/// [`crate::SprinkleBuilder::with_orbit_cam`],
/// [`crate::SprinkleBuilder::with_orbit_cam_preset`],
/// [`crate::SprinkleBuilder::with_orbit_cam_bindings`], and
/// [`crate::SprinkleBuilder::with_orbit_cam_manual`] add it for you. An example
/// that spawns its own `OrbitCam` (for instance to insert a swapped `CameraBasis`)
/// attaches it by hand, so [`crate::SprinkleBuilder::with_camera_home`],
/// [`crate::SprinkleBuilder::with_stable_transparency`], and
/// [`crate::SprinkleBuilder::with_restore_camera_on_restart`] still find the
/// camera. Capabilities locate it via `On<Add, FairyDustOrbitCam>` observers or
/// `With<FairyDustOrbitCam>` queries rather than scanning every `OrbitCam` in
/// the world.
///
/// [`crate::SprinkleBuilder::with_camera_control_panel`] does not need this
/// marker — it follows the routed `OrbitCam` directly.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct FairyDustOrbitCam;

/// Explicit startup pose for Fairy Dust's spawned `OrbitCam`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamPose {
    /// World-space point the orbit camera looks at.
    pub focus:  Vec3,
    /// Orbit yaw in radians.
    pub yaw:    f32,
    /// Orbit pitch in radians.
    pub pitch:  f32,
    /// Orbit radius from `focus`.
    pub radius: f32,
}

impl OrbitCamPose {
    /// Seeds an existing `OrbitCam` with this pose and makes the first controller
    /// pass write the matching `Transform`.
    pub fn apply_to(self, camera: &mut OrbitCam) {
        camera.pan.snap_to(self.focus);
        camera.zoom.snap_to(self.radius);
        camera.orbit.snap_to((self.yaw, self.pitch));
        camera.initialization = Initialization::FromPose;
    }
}

impl From<OrbitCamPose> for OrbitCam {
    fn from(pose: OrbitCamPose) -> Self {
        let mut camera = Self::default();
        pose.apply_to(&mut camera);
        camera
    }
}

/// Applies the canonical example camera limits used by manual `OrbitCam` spawns.
pub fn apply_example_orbit_cam_limits(camera: &mut OrbitCam) {
    *camera.orbit.limit_mut() = AnglePairLimit {
        yaw:   ScalarLimit::None,
        pitch: ScalarLimit::Clamp {
            min: -EXAMPLE_ORBIT_CAM_PITCH_LIMIT,
            max: EXAMPLE_ORBIT_CAM_PITCH_LIMIT,
        },
    };
    *camera.zoom.limit_mut() = ScalarLimit::Clamp {
        min: EXAMPLE_ORBIT_CAM_ZOOM_LOWER_LIMIT,
        max: EXAMPLE_ORBIT_CAM_ZOOM_UPPER_LIMIT,
    };
    camera.upside_down_policy = UpsideDownPolicy::Allow;
}

pub(crate) fn install_with(
    app: &mut App,
    configure: impl FnOnce(&mut OrbitCam) + Send + Sync + 'static,
) {
    install_with_bundle(app, configure, ());
}

pub(crate) fn install_with_bundle<B>(
    app: &mut App,
    configure: impl FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    bundle: B,
) where
    B: Bundle + Send + Sync + 'static,
{
    let configure = Mutex::new(Some(configure));
    let bundle = Mutex::new(Some(bundle));
    app.add_systems(Startup, move |mut commands: Commands| {
        let mut orbit_cam = OrbitCam::default();
        apply_example_orbit_cam_limits(&mut orbit_cam);
        let configure_fn = configure.lock().ok().and_then(|mut g| g.take());
        if let Some(f) = configure_fn {
            f(&mut orbit_cam);
        }
        let mut camera = commands.spawn((orbit_cam, FairyDustOrbitCam));
        if let Some(bundle) = bundle.lock().ok().and_then(|mut g| g.take()) {
            camera.insert(bundle);
        }
    });
}

pub(crate) fn install_pose_with_bundle<B>(app: &mut App, pose: OrbitCamPose, bundle: B)
where
    B: Bundle + Send + Sync + 'static,
{
    let bundle = Mutex::new(Some(bundle));
    app.add_systems(Startup, move |mut commands: Commands| {
        let mut orbit_cam = OrbitCam::from(pose);
        apply_example_orbit_cam_limits(&mut orbit_cam);
        let mut camera = commands.spawn((orbit_cam, FairyDustOrbitCam));
        if let Some(bundle) = bundle.lock().ok().and_then(|mut g| g.take()) {
            camera.insert(bundle);
        }
    });
}
