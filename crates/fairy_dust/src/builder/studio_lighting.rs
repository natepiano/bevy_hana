//! `StudioLightingBuilder` impls.

use bevy::prelude::Bundle;
use bevy::prelude::Vec3;
use hana_lagrange::OrbitCam;
use hana_lagrange::OrbitCamBindings;
use hana_lagrange::OrbitCamPreset;

use super::CameraHomeBuilder;
use super::NoOrbitCam;
use super::PrimitiveBuilder;
use super::SprinkleBuilder;
use super::TitleBarBuilder;
use super::WithOrbitCam;
use crate::OrbitCamPose;
use crate::lighting;
use crate::lighting::StudioLightingConfig;
use crate::primitive::PrimitiveConfig;
use crate::screen_panels::TitleBar;

/// Builder returned by [`SprinkleBuilder::with_studio_lighting`] for tweaking
/// the studio rig before it spawns.
///
/// Lighting tweak methods are only reachable through this type, so calling
/// [`Self::aim_at`] or [`Self::key_light_pos`] is a compile error when no
/// studio lighting has been installed. Calling a non-lighting builder method
/// finalizes the configuration and returns to the normal [`SprinkleBuilder`]
/// (or [`PrimitiveBuilder`]) chain.
pub struct StudioLightingBuilder<S> {
    pub(super) parent: SprinkleBuilder<S>,
    pub(super) config: StudioLightingConfig,
}

impl<S> StudioLightingBuilder<S> {
    /// Sets the world-space target the key and fill lights aim at.
    ///
    /// Defaults to `(0.0, 0.45, 0.0)`. Set this to the position of the
    /// scene's primary subject so the cascade shadow map stays tight around
    /// what is being shadowed.
    #[must_use]
    pub const fn aim_at(mut self, target: Vec3) -> Self {
        self.config.aim_at = target;
        self
    }

    /// Sets the key (shadow-casting) directional light position in world
    /// space.
    ///
    /// Defaults to `(-3.5, 7.0, 4.8)`. The shadow direction is
    /// `(aim_at - key_light_pos).normalize()`; place the light in front of
    /// and above the subject (relative to the camera) to cast shadows that
    /// trail back behind it.
    #[must_use]
    pub const fn key_light_pos(mut self, pos: Vec3) -> Self {
        self.config.key_light_pos = pos;
        self
    }

    /// Sets the key light illuminance in lux.
    ///
    /// Defaults to the studio rig's key-light value of 13,500 lux. Use this for
    /// an example that keeps the studio direction, fill, point light, and shadow
    /// settings but needs a specific key-light brightness for comparison.
    #[must_use]
    pub const fn key_light_illuminance(mut self, illuminance: f32) -> Self {
        self.config.key_light_illuminance = illuminance;
        self
    }

    /// Finalizes the studio lighting configuration and starts configuring a
    /// ground plane.
    #[must_use]
    pub fn with_ground_plane(self) -> PrimitiveBuilder<S> {
        PrimitiveBuilder {
            parent:  self.finish(),
            config:  PrimitiveConfig::ground_plane(),
            inserts: Vec::new(),
        }
    }

    /// Finalizes the studio lighting configuration and starts configuring a
    /// cube.
    #[must_use]
    pub fn with_cube(self) -> PrimitiveBuilder<S> {
        PrimitiveBuilder {
            parent:  self.finish(),
            config:  PrimitiveConfig::cube(),
            inserts: Vec::new(),
        }
    }

    /// Finalizes the studio lighting configuration and starts configuring a
    /// camera home pose.
    #[must_use]
    pub fn with_camera_home(self) -> CameraHomeBuilder<S> { self.finish().with_camera_home() }

    /// Finalizes the studio lighting configuration and adds an example title
    /// bar.
    #[must_use]
    pub fn with_title_bar(self, title_bar: TitleBar) -> TitleBarBuilder<S> {
        self.finish().with_title_bar(title_bar)
    }

    fn finish(mut self) -> SprinkleBuilder<S> {
        lighting::install(&mut self.parent.app, self.config);
        self.parent
    }
}

impl StudioLightingBuilder<NoOrbitCam> {
    /// Finalizes the studio lighting configuration, adds `LagrangePlugin`, and
    /// spawns an `OrbitCam` entity.
    pub fn with_orbit_cam_configured<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_configured(configure)
    }

    /// Finalizes the studio lighting configuration, spawns an `OrbitCam`, and
    /// inserts extra camera-side components.
    pub fn with_orbit_cam<F, B>(self, configure: F, bundle: B) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam(configure, bundle)
    }

    /// Finalizes the studio lighting configuration, spawns an `OrbitCam`, and
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

    /// Finalizes the studio lighting configuration, spawns an `OrbitCam` with an
    /// explicit startup pose, and installs one built-in input preset.
    pub fn with_orbit_cam_preset_pose(
        self,
        pose: OrbitCamPose,
        preset: impl Into<OrbitCamPreset>,
    ) -> SprinkleBuilder<WithOrbitCam> {
        self.finish().with_orbit_cam_preset_pose(pose, preset)
    }

    /// Finalizes the studio lighting configuration, spawns an `OrbitCam`,
    /// installs one built-in input preset, and inserts extra camera-side
    /// components.
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

    /// Finalizes the studio lighting configuration, spawns an `OrbitCam` with an
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

    /// Finalizes the studio lighting configuration, spawns an `OrbitCam`, and
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

    /// Finalizes the studio lighting configuration, spawns an `OrbitCam`,
    /// installs app-owned input bindings, and inserts extra camera-side
    /// components.
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

    /// Finalizes the studio lighting configuration and spawns a manually
    /// driven `OrbitCam`.
    pub fn with_orbit_cam_manual<F>(self, configure: F) -> SprinkleBuilder<WithOrbitCam>
    where
        F: FnOnce(&mut OrbitCam) + Send + Sync + 'static,
    {
        self.finish().with_orbit_cam_manual(configure)
    }

    /// Finalizes the studio lighting configuration, spawns a manually driven
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
