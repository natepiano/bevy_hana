use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::Vec2;

/// Logical surface metric used to scale semantic camera input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum CameraInputMetricKind {
    /// Camera view size in logical pixels.
    CameraViewSize,
    /// Input surface size in logical pixels.
    InputSurfaceSize,
}

/// Logical surface metrics for cameras whose input surface cannot be inferred.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Component, Default)]
pub struct CameraInputSurfaceMetrics {
    /// Camera view size in logical pixels.
    pub camera_view_size:   Option<Vec2>,
    /// Input surface size in logical pixels.
    pub input_surface_size: Option<Vec2>,
}

impl CameraInputSurfaceMetrics {
    /// Creates surface metrics from optional logical sizes.
    #[must_use]
    pub const fn new(camera_view_size: Option<Vec2>, input_surface_size: Option<Vec2>) -> Self {
        Self {
            camera_view_size,
            input_surface_size,
        }
    }

    /// Creates surface metrics with only a camera view size.
    #[must_use]
    pub const fn camera_view(camera_view_size: Vec2) -> Self {
        Self {
            camera_view_size:   Some(camera_view_size),
            input_surface_size: None,
        }
    }

    /// Creates surface metrics with both camera view and input surface sizes.
    #[must_use]
    pub const fn camera_view_and_input_surface(
        camera_view_size: Vec2,
        input_surface_size: Vec2,
    ) -> Self {
        Self {
            camera_view_size:   Some(camera_view_size),
            input_surface_size: Some(input_surface_size),
        }
    }
}
