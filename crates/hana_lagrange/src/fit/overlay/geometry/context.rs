use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Camera;
use bevy::prelude::Entity;
use bevy::prelude::Rect;
use bevy::prelude::Vec2;

/// Empty-frame reason for a `FitOverlay` camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::fit::overlay) enum FitOverlayEmptyReason {
    /// The camera is inactive.
    InactiveCamera,
    /// The camera render target could not be normalized.
    MissingRenderTarget,
    /// The camera viewport is unavailable or has no positive area.
    MissingViewport,
    /// The camera has no `CurrentFitTarget`.
    MissingCurrentFitTarget,
    /// The target has no extractable mesh vertices.
    MissingMesh,
    /// The projection kind is not supported by the overlay layout.
    UnsupportedProjection,
    /// Projected target vertices did not produce usable screen-space bounds.
    UnprojectableBounds,
    /// Projected depth data was empty.
    MissingDepths,
}

/// Render and viewport context resolved from a source `FitOverlay` camera.
pub(in crate::fit::overlay) struct FitOverlayCameraContext {
    /// Camera entity that owns the overlay frame.
    pub camera:            Entity,
    /// Normalized render target used for layout identity.
    pub normalized_target: NormalizedRenderTarget,
    /// Logical viewport rectangle for the source camera.
    pub logical_viewport:  Rect,
    /// Effective render layers copied from the source camera.
    pub layers:            RenderLayers,
    /// Source camera pass order.
    pub order:             isize,
    /// Whether the source camera is active.
    pub is_active:         bool,
}

impl FitOverlayCameraContext {
    pub(in crate::fit::overlay) fn resolve(
        camera: Entity,
        camera_component: &Camera,
        render_target: &RenderTarget,
        render_layers: Option<&RenderLayers>,
        primary_window: Option<Entity>,
    ) -> Result<Self, FitOverlayEmptyReason> {
        let normalized_target = render_target
            .normalize(primary_window)
            .ok_or(FitOverlayEmptyReason::MissingRenderTarget)?;

        let logical_viewport = camera_component
            .logical_viewport_rect()
            .ok_or(FitOverlayEmptyReason::MissingViewport)?;
        if logical_viewport.width() <= 0.0 || logical_viewport.height() <= 0.0 {
            return Err(FitOverlayEmptyReason::MissingViewport);
        }

        let context = Self {
            camera,
            normalized_target,
            logical_viewport,
            layers: effective_render_layers(render_layers),
            order: camera_component.order,
            is_active: camera_component.is_active,
        };

        if context.is_active {
            Ok(context)
        } else {
            Err(FitOverlayEmptyReason::InactiveCamera)
        }
    }

    pub(in crate::fit::overlay) const fn viewport_size(&self) -> Vec2 {
        self.logical_viewport.size()
    }
}

fn effective_render_layers(render_layers: Option<&RenderLayers>) -> RenderLayers {
    render_layers.cloned().unwrap_or_default()
}
