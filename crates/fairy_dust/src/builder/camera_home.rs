//! `CameraHomeBuilder` impls.

use bevy::app::Plugins;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::AppExit;
use bevy::prelude::Bundle;
use bevy::prelude::Component;
use bevy::prelude::FromWorld;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamBindings;
use hana_lagrange::OrbitCamPreset;

use super::NoOrbitCam;
use super::PrimitiveBuilder;
use super::SprinkleBuilder;
use super::StudioLightingBuilder;
use super::TitleBarBuilder;
use super::WithOrbitCam;
use crate::Anchor;
use crate::OrbitCamPose;
use crate::camera_home;
use crate::camera_home::CameraHomeConfig;
use crate::camera_home::HomeTitleBarControl;
use crate::constants::HOME_CONTROL;
use crate::cube_spin::CubeSpinConfig;
use crate::restart_camera;
use crate::screen_panels;
use crate::screen_panels::DescriptionPanel;
use crate::screen_panels::TitleBar;

/// Builder returned while configuring a camera "home" pose.
///
/// Calling a non-home builder method finalizes the home registration and
/// returns to the normal [`SprinkleBuilder`] chain.
pub struct CameraHomeBuilder<S> {
    pub(super) parent: SprinkleBuilder<S>,
    pub(super) config: CameraHomeConfig,
}

impl<S> CameraHomeBuilder<S> {
    /// Sets the home pose yaw in radians.
    #[must_use]
    pub const fn yaw(mut self, yaw: f32) -> Self {
        self.config.yaw = yaw;
        self
    }

    /// Sets the home pose pitch in radians.
    #[must_use]
    pub const fn pitch(mut self, pitch: f32) -> Self {
        self.config.pitch = pitch;
        self
    }

    /// Sets the screen-fraction margin used when framing the home region.
    #[must_use]
    pub const fn margin(mut self, margin: f32) -> Self {
        self.config.margin = margin;
        self
    }

    /// Sets where the fitted home bounds should land in the viewport.
    #[must_use]
    pub const fn anchor(mut self, anchor: Anchor) -> Self {
        self.config.anchor = anchor;
        self
    }

    /// Sets a pixel offset from the selected home anchor.
    ///
    /// Positive x moves the fitted home bounds right. Positive y moves them
    /// down, matching Bevy's screen-space coordinate convention.
    #[must_use]
    pub const fn offset_px(mut self, offset_px: Vec2) -> Self {
        self.config.offset_px = offset_px;
        self
    }

    /// Keeps the home behavior installed but does not add the automatic
    /// `H Home` chip to Fairy Dust title bars.
    #[must_use]
    pub const fn without_title_bar_control(mut self) -> Self {
        self.config.title_bar_control = HomeTitleBarControl::Hidden;
        self
    }

    /// Finalizes the current home registration and starts configuring another.
    #[must_use]
    pub fn with_camera_home(self) -> Self { self.finish().with_camera_home() }

    /// Finalizes the current home registration and starts configuring a ground plane.
    #[must_use]
    pub fn with_ground_plane(self) -> PrimitiveBuilder<S> { self.finish().with_ground_plane() }

    /// Finalizes the current home registration and starts configuring a cube.
    #[must_use]
    pub fn with_cube(self) -> PrimitiveBuilder<S> { self.finish().with_cube() }

    /// Finalizes the current home registration and adds window position persistence.
    #[must_use]
    pub fn with_save_window_position(self) -> SprinkleBuilder<S> {
        self.finish().with_save_window_position()
    }

    /// Finalizes the current home registration and adds BRP extras.
    #[must_use]
    pub fn with_brp_extras(self) -> SprinkleBuilder<S> { self.finish().with_brp_extras() }

    /// Finalizes the current home registration and adds the camera control panel.
    #[must_use]
    pub fn with_camera_control_panel(self) -> SprinkleBuilder<S> {
        self.finish().with_camera_control_panel()
    }

    /// Finalizes the current home registration and adds a marker-scoped cube spin helper.
    #[must_use]
    pub fn with_cube_spin<M: Component>(self) -> SprinkleBuilder<S> {
        self.finish().with_cube_spin::<M>()
    }

    /// Finalizes the current home registration and adds a customized marker-scoped cube spin
    /// helper.
    #[must_use]
    pub fn with_cube_spin_config<M: Component>(self, config: CubeSpinConfig) -> SprinkleBuilder<S> {
        self.finish().with_cube_spin_config::<M>(config)
    }

    /// Finalizes the current home registration and adds studio lighting.
    #[must_use]
    pub fn with_studio_lighting(self) -> StudioLightingBuilder<S> {
        self.finish().with_studio_lighting()
    }

    /// Finalizes the current home registration and adds an example description panel.
    #[must_use]
    pub fn with_description_panel(self, panel: DescriptionPanel) -> SprinkleBuilder<S> {
        self.finish().with_description_panel(panel)
    }

    /// Finalizes the current home registration and adds an example title bar.
    #[must_use]
    pub fn with_title_bar(self, title_bar: TitleBar) -> TitleBarBuilder<S> {
        self.finish().with_title_bar(title_bar)
    }

    /// Finalizes the current home registration and mirrors
    /// [`App::add_plugins`](bevy::app::App::add_plugins).
    #[must_use]
    pub fn add_plugins<M>(self, plugins: impl Plugins<M>) -> SprinkleBuilder<S> {
        self.finish().add_plugins(plugins)
    }

    /// Finalizes the current home registration and mirrors
    /// [`App::add_systems`](bevy::app::App::add_systems).
    #[must_use]
    pub fn add_systems<M>(
        self,
        schedule: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> SprinkleBuilder<S> {
        self.finish().add_systems(schedule, systems)
    }

    /// Finalizes the current home registration and mirrors
    /// [`App::add_observer`](bevy::app::App::add_observer).
    #[must_use]
    pub fn add_observer<E, B, M, I>(self, observer: I) -> SprinkleBuilder<S>
    where
        E: bevy::ecs::event::Event,
        B: Bundle,
        I: bevy::ecs::system::IntoObserverSystem<E, B, M>,
    {
        self.finish().add_observer(observer)
    }

    /// Finalizes the current home registration and mirrors
    /// [`App::init_resource`](bevy::app::App::init_resource).
    #[must_use]
    pub fn init_resource<R: Resource + FromWorld>(self) -> SprinkleBuilder<S> {
        self.finish().init_resource::<R>()
    }

    /// Finalizes the current home registration and mirrors
    /// [`App::insert_resource`](bevy::app::App::insert_resource).
    #[must_use]
    pub fn insert_resource<R: Resource>(self, resource: R) -> SprinkleBuilder<S> {
        self.finish().insert_resource(resource)
    }

    /// Finalizes the current home registration and runs the configured app.
    pub fn run(self) -> AppExit { self.finish().run() }

    fn finish(mut self) -> SprinkleBuilder<S> {
        if matches!(self.config.title_bar_control, HomeTitleBarControl::Shown) {
            screen_panels::register_title_control(&mut self.parent.app, HOME_CONTROL);
        }
        camera_home::install(&mut self.parent.app, self.config);
        self.parent
    }
}

impl CameraHomeBuilder<NoOrbitCam> {
    /// Finalizes the current home registration, adds `LagrangePlugin`, and spawns an
    /// `OrbitCam` entity.
    pub fn with_orbit_cam_configured<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_configured(configure)
    }

    /// Finalizes the current home registration, adds `LagrangePlugin`, spawns an
    /// `OrbitCam`, and inserts extra camera-side components.
    pub fn with_orbit_cam<F, B>(self, configure: F, bundle: B) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam(configure, bundle)
    }

    /// Finalizes the current home registration, spawns an `OrbitCam`, and
    /// installs one built-in input preset.
    pub fn with_orbit_cam_preset<F>(
        self,
        configure: F,
        preset: impl Into<OrbitCamPreset>,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_preset(configure, preset)
    }

    /// Finalizes the current home registration, spawns an `OrbitCam` with an
    /// explicit startup pose, and installs one built-in input preset.
    pub fn with_orbit_cam_preset_pose(
        self,
        pose: OrbitCamPose,
        preset: impl Into<OrbitCamPreset>,
    ) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_orbit_cam_preset_pose(pose, preset)
    }

    /// Finalizes the current home registration, spawns an `OrbitCam`, installs
    /// one built-in input preset, and inserts extra camera-side components.
    pub fn with_orbit_cam_preset_bundle<F, B>(
        self,
        configure: F,
        preset: impl Into<OrbitCamPreset>,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish()
            .with_orbit_cam_preset_bundle(configure, preset, bundle)
    }

    /// Finalizes the current home registration, spawns an `OrbitCam` with an
    /// explicit startup pose, installs one built-in input preset, and inserts
    /// extra camera-side components.
    pub fn with_orbit_cam_preset_pose_bundle<B>(
        self,
        pose: OrbitCamPose,
        preset: impl Into<OrbitCamPreset>,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        B: Bundle + Send + Sync + 'static,
    {
        self.finish()
            .with_orbit_cam_preset_pose_bundle(pose, preset, bundle)
    }

    /// Finalizes the current home registration, spawns an `OrbitCam`, and
    /// installs app-owned input bindings.
    pub fn with_orbit_cam_bindings<F>(
        self,
        configure: F,
        bindings: OrbitCamBindings,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_bindings(configure, bindings)
    }

    /// Finalizes the current home registration, spawns an `OrbitCam`, installs
    /// app-owned input bindings, and inserts extra camera-side components.
    pub fn with_orbit_cam_bindings_bundle<F, B>(
        self,
        configure: F,
        bindings: OrbitCamBindings,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish()
            .with_orbit_cam_bindings_bundle(configure, bindings, bundle)
    }

    /// Finalizes the current home registration and spawns a manually driven
    /// `OrbitCam`.
    pub fn with_orbit_cam_manual<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_manual(configure)
    }

    /// Finalizes the current home registration, spawns a manually driven
    /// `OrbitCam`, and inserts extra camera-side components.
    pub fn with_orbit_cam_manual_bundle<F, B>(
        self,
        configure: F,
        bundle: B,
    ) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish()
            .with_orbit_cam_manual_bundle(configure, bundle)
    }
}

impl CameraHomeBuilder<WithOrbitCam> {
    /// Finalizes the current home registration and makes the restart camera
    /// animation available through [`crate::RestoreWindowAnimation`].
    #[must_use]
    pub fn with_restore_camera_on_restart(mut self) -> SprinkleBuilder<WithOrbitCam> {
        restart_camera::install(&mut self.parent.app);
        self.finish()
    }

    /// Finalizes the current home registration and adds stable transparency to the
    /// spawned `OrbitCam`.
    #[must_use]
    pub fn with_stable_transparency(self) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_stable_transparency()
    }
}
