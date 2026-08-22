use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;

use crate::fit::overlay::geometry::Edge;

/// Generated fit-overlay visual owned by a source camera.
///
/// The `camera` identifies the entity with `FitOverlay` that requested this
/// visual. Retained line visuals copy that camera's effective `RenderLayers`;
/// Bevy UI label visuals use `UiTargetCamera` instead.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Eq)]
#[reflect(Component)]
pub struct FitOverlayVisual {
    /// Camera entity that owns update and cleanup for this visual.
    pub(super) camera: Entity,
    /// Stable identity for the visual within its source camera's overlay.
    pub(super) kind:   FitOverlayVisualKind,
}

/// Stable identity for a generated fit-overlay visual.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FitOverlayVisualKind {
    /// Screen-aligned fit target rectangle.
    Rectangle,
    /// Convex hull silhouette of the current fit target.
    Silhouette,
    /// Margin line from a projected target edge to the viewport edge.
    MarginLine {
        /// Margin edge represented by this visual.
        edge: Edge,
    },
    /// Margin percentage label for a viewport edge.
    MarginLabel {
        /// Margin edge represented by this label.
        edge: Edge,
    },
    /// Label for the projected screen-space bounds.
    BoundsLabel,
}
