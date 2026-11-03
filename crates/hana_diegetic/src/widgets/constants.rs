use std::time::Duration;

use crate::layout::Unit;

// slider pointer interaction
pub(super) const THUMB_CLICK_JITTER_PIXELS: f32 = 4.0;
pub(super) const THUMB_CLICK_JITTER_PIXELS_SQUARED: f32 =
    THUMB_CLICK_JITTER_PIXELS * THUMB_CLICK_JITTER_PIXELS;
pub(super) const THUMB_HIT_SLOP_POINTS: f32 = 4.0;

// wheel scrolling
/// Lines of a scrolling element's measured line pitch one wheel notch moves.
pub(super) const WHEEL_LINES_PER_NOTCH: f32 = 3.0;

// tooltip
pub(super) const DEFAULT_SHOW_DELAY: Duration = Duration::from_millis(500);
pub(super) const DEFAULT_TOOLTIP_GAP: f32 = 8.0;
pub(super) const QUAD_ANCHOR_COUNT: usize = 9;
pub(super) const QUAD_EDGE_COUNT: usize = 4;
pub(super) const TOOLTIP_VIEWPORT_MARGIN: f32 = 8.0;

/// How far a tooltip's soft shadow paints past its body, in tooltip layout
/// pixels.
///
/// The shadow is the body's rounded rectangle moved down by
/// `downward_offset_px` and blurred over `blur_support_px` on each side of its
/// edge, so nothing paints past the body grown by `blur_support_px` on every
/// side and by `downward_offset_px` more below. The SDF shadow record, its
/// mesh and clip, batch culling bounds, and tooltip placement all read this one
/// extent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TooltipShadowExtent {
    /// Distance from the shadow's edge to where its blur reaches zero.
    blur_support_px:    f32,
    /// Distance the shadow's rectangle sits below the body.
    downward_offset_px: f32,
}

impl TooltipShadowExtent {
    /// This extent in the local units of a panel with this point scale, where
    /// its shadow is drawn.
    pub(crate) fn panel_local(self, points_to_world: f32) -> TooltipShadowLocalExtent {
        let local_per_px = Unit::Pixels.to_points() * points_to_world;
        TooltipShadowLocalExtent {
            blur_support:    self.blur_support_px * local_per_px,
            downward_offset: self.downward_offset_px * local_per_px,
        }
    }

    /// The painted bounds' reach past the body, in the same local units: the
    /// blur on every side, and the downward offset more below.
    pub(crate) fn painted_reach(self, points_to_world: f32) -> TooltipPaintedReach {
        let local = self.panel_local(points_to_world);
        TooltipPaintedReach {
            sides: local.blur_support,
            above: local.blur_support,
            below: local.blur_support + local.downward_offset,
        }
    }
}

/// A tooltip shadow's extent in the local units of the panel it is drawn on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TooltipShadowLocalExtent {
    /// Distance from the shadow's edge to where its blur reaches zero.
    pub(crate) blur_support:    f32,
    /// Distance the shadow's rectangle sits below the body.
    pub(crate) downward_offset: f32,
}

/// How far a tooltip's shadow paints past its body on each side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TooltipPaintedReach {
    /// Reach past the left and right edges.
    pub(crate) sides: f32,
    /// Reach above the top edge: the blur.
    pub(crate) above: f32,
    /// Reach below the bottom edge: the blur plus the downward offset.
    pub(crate) below: f32,
}

/// Every tooltip's shadow extent.
pub(crate) const TOOLTIP_SHADOW_EXTENT: TooltipShadowExtent = TooltipShadowExtent {
    blur_support_px:    8.0,
    downward_offset_px: 3.0,
};
