use bevy::prelude::Color;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;

use crate::fit::overlay::constants::DEFAULT_OVERLAY_LINE_WIDTH;
use crate::fit::overlay::constants::OVERLAY_BALANCED_COLOR;
use crate::fit::overlay::constants::OVERLAY_RECTANGLE_COLOR;
use crate::fit::overlay::constants::OVERLAY_SILHOUETTE_COLOR;
use crate::fit::overlay::constants::OVERLAY_UNBALANCED_COLOR;

/// Configuration for fit target overlay colors and line appearance.
///
/// This resource controls visual style only. It does not select render layers,
/// choose a camera, or override `Camera::order`. Configure visibility on the
/// camera that carries `FitOverlay`: generated retained line visuals copy that
/// camera's effective `RenderLayers` and render in normal Bevy camera passes
/// when camera and visual layers intersect. Overlay labels are Bevy UI nodes
/// targeted through `UiTargetCamera`.
#[derive(Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct FitTargetOverlayConfig {
    /// Color for the screen-aligned bounding rectangle.
    pub rectangle_color:  Color,
    /// Color for the silhouette convex hull.
    pub silhouette_color: Color,
    /// Color for balanced margins (left ≈ right, top ≈ bottom).
    pub balanced_color:   Color,
    /// Color for unbalanced margins.
    pub unbalanced_color: Color,
    /// Line width for retained overlay line meshes, in viewport pixels.
    pub line_width:       f32,
}

impl Default for FitTargetOverlayConfig {
    fn default() -> Self {
        Self {
            rectangle_color:  OVERLAY_RECTANGLE_COLOR,
            silhouette_color: OVERLAY_SILHOUETTE_COLOR,
            balanced_color:   OVERLAY_BALANCED_COLOR,
            unbalanced_color: OVERLAY_UNBALANCED_COLOR,
            line_width:       DEFAULT_OVERLAY_LINE_WIDTH,
        }
    }
}
