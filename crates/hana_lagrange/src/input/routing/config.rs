//! Public routing configuration for camera input.
//!
//! Types:
//! - [`CameraInputRouting`] — selects between cursor-hit-test and explicit-camera modes.
//! - [`NoPositionFallback`] — policy for keyboard/gamepad input that lacks pointer coordinates.
//! - [`CameraInputRoutingConfig`] — resource holding the active mode, optional explicit target, and
//!   no-position fallback. Constructed with [`CameraInputRoutingConfig::cursor_hit_test`] or
//!   [`CameraInputRoutingConfig::explicit`].

use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;

/// Camera input routing mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum CameraInputRouting {
    /// Choose the active camera by cursor/touch hit testing.
    #[default]
    CursorHitTest,
    /// Use the configured explicit camera entity.
    Explicit,
}

/// Fallback policy for input without pointer position metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum NoPositionFallback {
    /// Drop input unless a latch, explicit route, or unambiguous hit test identifies a camera.
    #[default]
    NoInput,
    /// Route to the only eligible `OrbitCam` when exactly one exists.
    OnlyEligibleCamera,
}

/// Routing mode, explicit target, and no-position fallback applied to preset
/// and custom camera input.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Resource, Default)]
pub struct CameraInputRoutingConfig {
    /// Routing mode.
    pub mode:                 CameraInputRouting,
    /// Explicit target camera used when `mode` is [`CameraInputRouting::Explicit`].
    pub explicit_camera:      Option<Entity>,
    /// Fallback policy for keyboard/gamepad style input without pointer position.
    pub no_position_fallback: NoPositionFallback,
}

impl CameraInputRoutingConfig {
    /// Creates cursor-hit-test routing with default no-position fallback.
    #[must_use]
    pub const fn cursor_hit_test() -> Self {
        Self {
            mode:                 CameraInputRouting::CursorHitTest,
            explicit_camera:      None,
            no_position_fallback: NoPositionFallback::NoInput,
        }
    }

    /// Creates explicit routing to `camera`.
    #[must_use]
    pub const fn explicit(camera: Entity) -> Self {
        Self {
            mode:                 CameraInputRouting::Explicit,
            explicit_camera:      Some(camera),
            no_position_fallback: NoPositionFallback::NoInput,
        }
    }

    /// Sets the no-position fallback policy.
    #[must_use]
    pub const fn with_no_position_fallback(mut self, fallback: NoPositionFallback) -> Self {
        self.no_position_fallback = fallback;
        self
    }
}
