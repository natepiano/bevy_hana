//! `PrimitiveBuilder` impls.

use bevy::app::Plugins;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::ecs::system::EntityCommands;
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::AppExit;
use bevy::prelude::Bundle;
use bevy::prelude::Color;
use bevy::prelude::Component;
use bevy::prelude::FromWorld;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamBindings;
use hana_lagrange::OrbitCamPreset;

use super::CameraHomeBuilder;
use super::NoOrbitCam;
use super::SprinkleBuilder;
use super::StudioLightingBuilder;
use super::TitleBarBuilder;
use super::WithOrbitCam;
use crate::OrbitCamPose;
use crate::cube_spin;
use crate::cube_spin::CubeSpinConfig;
use crate::cube_spin::FairyDustCubeSpinTarget;
use crate::primitive;
use crate::primitive::Face;
use crate::primitive::FaceTextSpec;
use crate::primitive::PrimitiveConfig;
use crate::screen_panels::DescriptionPanel;
use crate::screen_panels::TitleBar;

/// Boxed deferred-insert closure applied to a primitive entity at spawn time.
pub(super) type PrimitiveInsert = Box<dyn FnOnce(&mut EntityCommands) + Send + Sync>;

/// Builder returned while configuring a simple scene primitive.
///
/// Calling a non-primitive builder method finalizes the primitive and returns
/// to the normal [`SprinkleBuilder`] chain.
pub struct PrimitiveBuilder<S> {
    pub(super) parent:  SprinkleBuilder<S>,
    pub(super) config:  PrimitiveConfig,
    pub(super) inserts: Vec<PrimitiveInsert>,
}

impl<S> PrimitiveBuilder<S> {
    /// Sets the primitive size.
    ///
    /// For a ground plane this is the square edge length. For a cube this is
    /// the cube edge length. Use [`Self::transform`] with a non-uniform
    /// [`Vec3`](bevy::prelude::Vec3) scale to make a ground plane rectangular.
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.config.set_size(size);
        self
    }

    /// Inserts additional components on the spawned primitive entity.
    ///
    /// Useful for attaching markers or example-specific components without
    /// dropping to a manual `commands.spawn`.
    #[must_use]
    pub fn insert<B: Bundle>(mut self, bundle: B) -> Self {
        self.inserts.push(Box::new(move |entity| {
            entity.insert(bundle);
        }));
        self
    }

    /// Sets the primitive material base color.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.config.set_color(color);
        self
    }

    /// Sets the full primitive material.
    ///
    /// This overrides any color previously configured with [`Self::color`].
    #[must_use]
    pub fn material(mut self, material: StandardMaterial) -> Self {
        self.config = self.config.with_material(material);
        self
    }

    /// Sets the primitive transform.
    #[must_use]
    pub const fn transform(mut self, transform: Transform) -> Self {
        self.config.set_transform(transform);
        self
    }

    /// Adds a centered world-space [`hana_diegetic::DiegeticText`] label to one
    /// face of a cube primitive. The label inherits the cube's `Transform` as parent,
    /// is sized in world meters by `text_size`, and uses one-sided glyphs.
    ///
    /// Only meaningful for cube primitives; on a ground plane the label is
    /// still attached as a child but its placement uses the cube face math
    /// and may not be what you want.
    #[must_use]
    pub fn face_text(
        mut self,
        face: Face,
        text: impl Into<String>,
        text_size: f32,
        color: Color,
    ) -> Self {
        self.config.push_face_text(FaceTextSpec {
            face,
            text: text.into(),
            text_size,
            color,
        });
        self
    }

    /// Adds a canonical blue label to one cube face.
    #[must_use]
    pub fn face_label(mut self, face: Face, text: impl Into<String>) -> Self {
        self.config.push_face_text(FaceTextSpec {
            face,
            text: text.into(),
            text_size: crate::constants::CUBE_FACE_LABEL_SIZE,
            color: crate::constants::CUBE_FACE_PANEL_BLUE,
        });
        self
    }

    /// Rotates this cube with the shared Fairy Dust cube-spin helper.
    ///
    /// This is intended for single-cube examples. For manual or multi-cube
    /// scenes, prefer [`SprinkleBuilder::with_cube_spin`] with an explicit
    /// marker component.
    #[must_use]
    pub fn cube_spin(mut self, config: CubeSpinConfig) -> Self {
        cube_spin::install::<FairyDustCubeSpinTarget>(&mut self.parent.app, config);
        self = self.insert(FairyDustCubeSpinTarget);
        self
    }

    /// Finalizes the current primitive and starts configuring a ground plane.
    #[must_use]
    pub fn with_ground_plane(self) -> Self { self.finish().with_ground_plane() }

    /// Finalizes the current primitive and starts configuring a cube.
    #[must_use]
    pub fn with_cube(self) -> Self { self.finish().with_cube() }

    /// Finalizes the current primitive and adds window position persistence.
    #[must_use]
    pub fn with_save_window_position(self) -> SprinkleBuilder<S> {
        self.finish().with_save_window_position()
    }

    /// Finalizes the current primitive and adds BRP extras.
    #[must_use]
    pub fn with_brp_extras(self) -> SprinkleBuilder<S> { self.finish().with_brp_extras() }

    /// Finalizes the current primitive and adds the smart camera control panel.
    #[must_use]
    pub fn with_camera_control_panel(self) -> SprinkleBuilder<S> {
        self.finish().with_camera_control_panel()
    }

    /// Finalizes the current primitive and adds a marker-scoped cube spin helper.
    #[must_use]
    pub fn with_cube_spin<M: Component>(self) -> SprinkleBuilder<S> {
        self.finish().with_cube_spin::<M>()
    }

    /// Finalizes the current primitive and adds a customized marker-scoped cube spin helper.
    #[must_use]
    pub fn with_cube_spin_config<M: Component>(self, config: CubeSpinConfig) -> SprinkleBuilder<S> {
        self.finish().with_cube_spin_config::<M>(config)
    }

    /// Finalizes the current primitive and adds studio lighting.
    #[must_use]
    pub fn with_studio_lighting(self) -> StudioLightingBuilder<S> {
        self.finish().with_studio_lighting()
    }

    /// Finalizes the current primitive and adds an example description panel.
    #[must_use]
    pub fn with_description_panel(self, panel: DescriptionPanel) -> SprinkleBuilder<S> {
        self.finish().with_description_panel(panel)
    }

    /// Finalizes the current primitive and adds an example title bar.
    #[must_use]
    pub fn with_title_bar(self, title_bar: TitleBar) -> TitleBarBuilder<S> {
        self.finish().with_title_bar(title_bar)
    }

    /// Finalizes the current primitive and starts configuring a camera home pose.
    #[must_use]
    pub fn with_camera_home(self) -> CameraHomeBuilder<S> { self.finish().with_camera_home() }

    /// Finalizes the current primitive and mirrors
    /// [`App::add_plugins`](bevy::app::App::add_plugins).
    #[must_use]
    pub fn add_plugins<M>(self, plugins: impl Plugins<M>) -> SprinkleBuilder<S> {
        self.finish().add_plugins(plugins)
    }

    /// Finalizes the current primitive and mirrors
    /// [`App::add_systems`](bevy::app::App::add_systems).
    #[must_use]
    pub fn add_systems<M>(
        self,
        schedule: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> SprinkleBuilder<S> {
        self.finish().add_systems(schedule, systems)
    }

    /// Finalizes the current primitive and mirrors
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

    /// Finalizes the current primitive and mirrors
    /// [`App::init_resource`](bevy::app::App::init_resource).
    #[must_use]
    pub fn init_resource<R: Resource + FromWorld>(self) -> SprinkleBuilder<S> {
        self.finish().init_resource::<R>()
    }

    /// Finalizes the current primitive and mirrors
    /// [`App::insert_resource`](bevy::app::App::insert_resource).
    #[must_use]
    pub fn insert_resource<R: Resource>(self, resource: R) -> SprinkleBuilder<S> {
        self.finish().insert_resource(resource)
    }

    /// Finalizes the current primitive and runs the configured app.
    pub fn run(self) -> AppExit { self.finish().run() }

    fn finish(mut self) -> SprinkleBuilder<S> {
        primitive::install(&mut self.parent.app, self.config, self.inserts);
        self.parent
    }
}

impl PrimitiveBuilder<NoOrbitCam> {
    /// Finalizes the current primitive, adds `LagrangePlugin`, and spawns an
    /// `OrbitCam` entity.
    pub fn with_orbit_cam_configured<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_configured(configure)
    }

    /// Finalizes the current primitive, adds `LagrangePlugin`, spawns an
    /// `OrbitCam`, and inserts extra camera-side components.
    pub fn with_orbit_cam<F, B>(self, configure: F, bundle: B) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam(configure, bundle)
    }

    /// Finalizes the current primitive, spawns an `OrbitCam`, and installs one
    /// built-in input preset.
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

    /// Finalizes the current primitive, spawns an `OrbitCam` with an explicit
    /// startup pose, and installs one built-in input preset.
    pub fn with_orbit_cam_preset_pose(
        self,
        pose: OrbitCamPose,
        preset: impl Into<OrbitCamPreset>,
    ) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_orbit_cam_preset_pose(pose, preset)
    }

    /// Finalizes the current primitive, spawns an `OrbitCam`, installs one
    /// built-in input preset, and inserts extra camera-side components.
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

    /// Finalizes the current primitive, spawns an `OrbitCam` with an explicit
    /// startup pose, installs one built-in input preset, and inserts extra
    /// camera-side components.
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

    /// Finalizes the current primitive, spawns an `OrbitCam`, and installs
    /// app-owned input bindings.
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

    /// Finalizes the current primitive, spawns an `OrbitCam`, installs
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

    /// Finalizes the current primitive and spawns a manually driven `OrbitCam`.
    pub fn with_orbit_cam_manual<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_manual(configure)
    }

    /// Finalizes the current primitive, spawns a manually driven `OrbitCam`,
    /// and inserts extra camera-side components.
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

impl PrimitiveBuilder<WithOrbitCam> {
    /// Finalizes the current primitive and makes the restart camera animation
    /// available through [`crate::RestoreWindowAnimation`].
    #[must_use]
    pub fn with_restore_camera_on_restart(self) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_restore_camera_on_restart()
    }

    /// Finalizes the current primitive and adds stable transparency to the
    /// spawned `OrbitCam`.
    #[must_use]
    pub fn with_stable_transparency(self) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_stable_transparency()
    }
}
