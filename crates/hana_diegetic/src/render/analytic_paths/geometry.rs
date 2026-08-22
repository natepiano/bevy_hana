//! Renderer-owned quadratic path geometry.

use bevy::math::Vec2;

/// A single quadratic Bezier segment in path design-space units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct QuadraticSegment {
    /// Segment start point.
    pub start:   Vec2,
    /// Quadratic control point.
    pub control: Vec2,
    /// Segment end point.
    pub end:     Vec2,
}

/// Axis-aligned path bounds in design-space units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bounds {
    /// Minimum corner.
    pub min: Vec2,
    /// Maximum corner.
    pub max: Vec2,
}

impl Bounds {
    /// Width of the bounds.
    #[must_use]
    pub(crate) fn width(self) -> f32 { self.max.x - self.min.x }

    /// Height of the bounds.
    #[must_use]
    pub(crate) fn height(self) -> f32 { self.max.y - self.min.y }
}

/// One closed analytic path contour expressed as quadratic segments.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PathContour {
    /// Quadratic segments in contour order.
    pub segments:      Vec<QuadraticSegment>,
    /// Narrowest stroke of this contour in design-space units, packed per
    /// curve for hairline dilation. `0.0` disables — text path contours
    /// stay undilated.
    pub min_feature:   f32,
    /// Resolved hairline fade exponent for this contour, packed per curve
    /// (`CurveRecord::fade_exponent`). The shader's winning (nearest) curve
    /// supplies the exponent for each coverage evaluation, so contours with
    /// different fade policies merge into one path without an anti-aliasing
    /// junction. `0.0` renders sub-floor strokes at full alpha
    /// ([`HairlineFade::Full`](crate::HairlineFade::Full)); text path
    /// contours carry `0.0` and are exempt regardless (dilation 0).
    pub fade_exponent: f32,
}

/// Renderer-owned quadratic outline representation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PathOutline {
    /// Outline bounds in local design-space units.
    pub bounds:   Bounds,
    /// Closed contours that make up the filled path.
    pub contours: Vec<PathContour>,
}

impl PathOutline {
    /// Narrowest dilating stroke across all contours, forwarded to
    /// `PackedPathRecord::min_feature` so the shader sizes its distance scan for
    /// the largest dilation in the path. `0.0` when no contour dilates.
    #[must_use]
    pub(crate) fn min_feature(&self) -> f32 {
        let narrowest = self
            .contours
            .iter()
            .map(|contour| contour.min_feature)
            .filter(|min_feature| *min_feature > 0.0)
            .fold(f32::INFINITY, f32::min);
        if narrowest.is_finite() {
            narrowest
        } else {
            0.0
        }
    }

    /// Total quadratic segments across all contours. Caps band splitting so a
    /// sparse path is not divided into bands smaller than the distance scan.
    #[must_use]
    pub(crate) fn curve_count(&self) -> usize {
        self.contours
            .iter()
            .map(|contour| contour.segments.len())
            .sum()
    }

    /// Rescales the outline into a unit-scale frame (origin at `bounds.min`,
    /// the longer extent mapped to `1.0`); returns the normalized outline and
    /// the divided-out scale.
    ///
    /// At REFERENCE-scale coordinates (~1e6 design units) the shader field
    /// normal `normalize(point - closest_point)` collapses to float32 round-off
    /// and every grazing line waves. Unit-scale coordinates keep it exact. The
    /// rebase runs in f64 so the f32 results carry full relative precision at
    /// unit magnitude; coverage divides two distances, so the rescale cancels
    /// out of the rendered result.
    #[must_use]
    pub(crate) fn normalized(self) -> (Self, f32) {
        let extent = self.bounds.width().max(self.bounds.height());
        if !extent.is_finite() || extent <= 0.0 {
            return (self, 1.0);
        }
        let origin = self.bounds.min.as_dvec2();
        let scale = f64::from(extent);
        let rebase = |point: Vec2| ((point.as_dvec2() - origin) / scale).as_vec2();
        let contours = self
            .contours
            .into_iter()
            .map(|contour| PathContour {
                segments:      contour
                    .segments
                    .into_iter()
                    .map(|segment| QuadraticSegment {
                        start:   rebase(segment.start),
                        control: rebase(segment.control),
                        end:     rebase(segment.end),
                    })
                    .collect(),
                min_feature:   contour.min_feature / extent,
                fade_exponent: contour.fade_exponent,
            })
            .collect();
        let bounds = Bounds {
            min: rebase(self.bounds.min),
            max: rebase(self.bounds.max),
        };
        (Self { bounds, contours }, extent)
    }
}
