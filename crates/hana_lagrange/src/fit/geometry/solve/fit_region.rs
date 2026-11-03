use bevy::prelude::Vec2;
use bevy::prelude::warn;

use super::fit_solution::FitError;
use super::focus;
use crate::fit::constants::MIN_FIT_REGION_FRACTION;
use crate::fit::geometry::projection::ScreenSpaceBounds;
use crate::fit::geometry::viewport_insets::FitViewportInsets;

/// The part of the viewport a fit frames into, as fractions of the viewport
/// size reserved at each edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FitRegion {
    left:   f32,
    top:    f32,
    right:  f32,
    bottom: f32,
}

impl FitRegion {
    /// The whole viewport.
    pub(super) const FULL_VIEWPORT: Self = Self {
        left:   0.0,
        top:    0.0,
        right:  0.0,
        bottom: 0.0,
    };

    /// Converts pixel insets into viewport fractions.
    ///
    /// Insets that leave less than [`MIN_FIT_REGION_FRACTION`] of either
    /// dimension are ignored with a warning, so a fit still frames its target.
    pub(super) fn from_insets(
        insets: FitViewportInsets,
        viewport_size: Option<Vec2>,
    ) -> Result<Self, FitError> {
        if !insets.reserves_space() {
            return Ok(Self::FULL_VIEWPORT);
        }
        let Some(size) = viewport_size.filter(|size| focus::viewport_can_map_pixels(Some(*size)))
        else {
            return Err(FitError::NoViewport);
        };
        let region = Self {
            left:   insets.left() / size.x,
            top:    insets.top() / size.y,
            right:  insets.right() / size.x,
            bottom: insets.bottom() / size.y,
        };
        if region.width_fraction() < MIN_FIT_REGION_FRACTION
            || region.height_fraction() < MIN_FIT_REGION_FRACTION
        {
            warn!(
                "calculate_fit: viewport insets {insets:?} leave too little of the {size} viewport; framing the whole viewport"
            );
            return Ok(Self::FULL_VIEWPORT);
        }
        Ok(region)
    }

    fn width_fraction(self) -> f32 { 1.0 - self.left - self.right }

    fn height_fraction(self) -> f32 { 1.0 - self.top - self.bottom }

    /// The region's size in logical pixels for a viewport of `viewport_size`.
    pub(super) fn pixel_size(self, viewport_size: Vec2) -> Vec2 {
        Vec2::new(
            viewport_size.x * self.width_fraction(),
            viewport_size.y * self.height_fraction(),
        )
    }

    /// Re-expresses viewport bounds relative to this region: the region's
    /// center becomes the origin, its half size becomes the half extent, and
    /// each margin measures the distance to the region's edge.
    pub(super) fn bounds_within(self, bounds: &ScreenSpaceBounds) -> ScreenSpaceBounds {
        let center_x = bounds.half_extent_x * (self.left - self.right);
        let center_y = bounds.half_extent_y * (self.bottom - self.top);
        let half_extent_x = bounds.half_extent_x * self.width_fraction();
        let half_extent_y = bounds.half_extent_y * self.height_fraction();
        let min_normalized_x = bounds.min_normalized_x - center_x;
        let max_normalized_x = bounds.max_normalized_x - center_x;
        let min_normalized_y = bounds.min_normalized_y - center_y;
        let max_normalized_y = bounds.max_normalized_y - center_y;
        ScreenSpaceBounds {
            left_margin: min_normalized_x + half_extent_x,
            right_margin: half_extent_x - max_normalized_x,
            top_margin: half_extent_y - max_normalized_y,
            bottom_margin: min_normalized_y + half_extent_y,
            min_normalized_x,
            max_normalized_x,
            min_normalized_y,
            max_normalized_y,
            half_extent_x,
            half_extent_y,
        }
    }
}
