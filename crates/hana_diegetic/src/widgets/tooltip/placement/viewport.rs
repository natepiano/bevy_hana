use bevy::prelude::Rect;
use bevy::prelude::Vec2;

use crate::layout::Anchor;
use crate::layout::Px;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachment;
use crate::panel::PanelScreenBounds;
use crate::widgets::constants::DEFAULT_TOOLTIP_GAP;
use crate::widgets::constants::TOOLTIP_VIEWPORT_MARGIN;
#[cfg(test)]
use crate::widgets::tooltip;

// World-to-viewport projection can differ by a few millionths of a pixel
// after a ray is converted back to an attachment offset.
pub(super) const TOOLTIP_PROJECTION_ROUNDING_PIXELS: f32 = 0.01;

pub(in crate::widgets::tooltip) fn usable_viewport_axis(axis: f32) -> Option<f32> {
    let usable = TOOLTIP_VIEWPORT_MARGIN.mul_add(-2.0, axis);
    (usable.is_finite() && usable > 0.0).then_some(usable)
}

/// The full window axis available to the no-room overlay fallback.
pub(super) fn available_window_axis(axis: f32) -> Option<f32> {
    (axis.is_finite() && axis > 0.0).then_some(axis)
}

pub(super) fn valid_screen_size(size: Vec2) -> bool {
    size.is_finite() && size.x > 0.0 && size.y > 0.0
}

pub(super) fn valid_screen_scale(scale: Vec2) -> bool {
    scale.is_finite() && scale.x.abs() > f32::EPSILON && scale.y.abs() > f32::EPSILON
}

pub(super) fn rotate_screen_vector(vector: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(
        vector.y.mul_add(sin, vector.x * cos),
        vector.y.mul_add(cos, -vector.x * sin),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TooltipSide {
    Above,
    Right,
    Below,
    Left,
}

impl TooltipSide {
    const fn opposite(self) -> Self {
        match self {
            Self::Above => Self::Below,
            Self::Right => Self::Left,
            Self::Below => Self::Above,
            Self::Left => Self::Right,
        }
    }

    pub(super) const fn is_vertical(self) -> bool { matches!(self, Self::Above | Self::Below) }

    pub(super) const fn anchors(self) -> (Anchor, Anchor) {
        match self {
            Self::Above => (Anchor::BottomCenter, Anchor::TopCenter),
            Self::Right => (Anchor::CenterLeft, Anchor::CenterRight),
            Self::Below => (Anchor::TopCenter, Anchor::BottomCenter),
            Self::Left => (Anchor::CenterRight, Anchor::CenterLeft),
        }
    }

    /// The offset that keeps `gap` between a tooltip on this side and its control.
    pub(super) const fn gap_offset(self, gap: f32) -> Vec2 {
        match self {
            Self::Above => Vec2::new(0.0, -gap),
            Self::Right => Vec2::new(gap, 0.0),
            Self::Below => Vec2::new(0.0, gap),
            Self::Left => Vec2::new(-gap, 0.0),
        }
    }

    /// The gap `offset` keeps between a tooltip on this side and its control.
    pub(super) const fn gap_kept_by(self, offset: Vec2) -> f32 {
        if self.is_vertical() {
            offset.y.abs()
        } else {
            offset.x.abs()
        }
    }
}

/// `DEFAULT_TOOLTIP_GAP` out from the side `source` and `target` name. A zero
/// offset makes [`preferred_side`] fall back to below for anchors that name no side.
pub(in crate::widgets::tooltip) fn default_gap_offset(
    source: Anchor,
    target: Anchor,
) -> PanelAnchorOffset {
    let side = preferred_side(PanelAttachment::new(source, target), Vec2::ZERO);
    let offset = side.gap_offset(DEFAULT_TOOLTIP_GAP);
    PanelAnchorOffset::new(Px(offset.x), Px(offset.y))
}

pub(super) fn preferred_side(authored: PanelAttachment, offset: Vec2) -> TooltipSide {
    let (source_x, source_y) = authored.source_anchor().offset_fraction();
    let (target_x, target_y) = authored.target_anchor().offset_fraction();
    let delta = Vec2::new(target_x - source_x, target_y - source_y);
    if delta.y.abs() >= delta.x.abs() && delta.y != 0.0 {
        if delta.y > 0.0 {
            TooltipSide::Below
        } else {
            TooltipSide::Above
        }
    } else if delta.x != 0.0 {
        if delta.x > 0.0 {
            TooltipSide::Right
        } else {
            TooltipSide::Left
        }
    } else if offset.y.abs() >= offset.x.abs() {
        if offset.y >= 0.0 {
            TooltipSide::Below
        } else {
            TooltipSide::Above
        }
    } else if offset.x >= 0.0 {
        TooltipSide::Right
    } else {
        TooltipSide::Left
    }
}

pub(super) const fn tooltip_side_order(
    preferred: TooltipSide,
    _target: PanelScreenBounds,
    _viewport: Rect,
) -> [TooltipSide; 4] {
    let remaining = if preferred.is_vertical() {
        [TooltipSide::Right, TooltipSide::Left]
    } else {
        [TooltipSide::Below, TooltipSide::Above]
    };
    [preferred, preferred.opposite(), remaining[0], remaining[1]]
}

/// Screen bounds of a placed tooltip's body and of everything it paints.
#[derive(Clone, Copy, Debug)]
pub(super) struct TooltipProjectedFootprint {
    /// The tooltip body alone; its target anchor stays within this span.
    pub(super) body:    PanelScreenBounds,
    /// The body with its shadow; every boundary and panel check uses this.
    pub(super) painted: PanelScreenBounds,
}

/// What a tooltip slides to stay within along its side, inside the placement
/// area.
#[derive(Clone, Copy, Debug)]
pub(super) enum TooltipShiftBoundary {
    /// The projected marked element holding the target, inset by the viewport
    /// margin.
    Element(Rect),
    /// The upright bounds of the projected host panel holding the target,
    /// inside its border and inset by the viewport margin.
    HostInterior(Rect),
    /// Only the placement area bounds the slide.
    PlacementAreaOnly,
}

/// `bounds` grown by `outset` on every side; a negative `outset` insets it.
fn margin_outset(bounds: PanelScreenBounds, outset: f32) -> Rect {
    let outset = Vec2::splat(outset);
    Rect {
        min: bounds.top_left() - outset,
        max: bounds.top_left() + bounds.size() + outset,
    }
}

/// The four corners of `bounds`.
fn corners(bounds: PanelScreenBounds) -> [Vec2; 4] {
    [
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::BottomLeft,
    ]
    .map(|anchor| bounds.point(anchor))
}

/// A screen rectangle turned with the panel it was projected from.
#[derive(Clone, Copy, Debug)]
pub(super) struct OrientedScreenRect {
    centre:    Vec2,
    /// Half its size along its own axes.
    half_size: Vec2,
    /// Its own x axis on screen, of unit length; its y axis is perpendicular.
    x_axis:    Vec2,
}

impl OrientedScreenRect {
    /// The rectangle whose corners are `top_left`, `top_right` and
    /// `bottom_left`.
    pub(super) fn from_corners(top_left: Vec2, top_right: Vec2, bottom_left: Vec2) -> Self {
        let across = top_right - top_left;
        let down = bottom_left - top_left;
        Self {
            centre:    top_left + 0.5 * (across + down),
            half_size: 0.5 * Vec2::new(across.length(), down.length()),
            x_axis:    across.normalize_or(Vec2::X),
        }
    }

    /// This rectangle grown by `outset` on every side; a negative `outset`
    /// insets it.
    fn outset(self, outset: f32) -> Self {
        Self {
            half_size: self.half_size + outset,
            ..self
        }
    }

    fn y_axis(self) -> Vec2 { self.x_axis.perp() }

    /// How far this rectangle reaches from its centre along `axis`.
    fn reach_along(self, axis: Vec2) -> f32 {
        self.half_size.x.mul_add(
            self.x_axis.dot(axis).abs(),
            self.half_size.y * self.y_axis().dot(axis).abs(),
        )
    }

    /// The upright screen bounds holding this rectangle.
    pub(super) fn upright_bounds(self) -> Rect {
        let extent = Vec2::new(self.reach_along(Vec2::X), self.reach_along(Vec2::Y));
        Rect {
            min: self.centre - extent,
            max: self.centre + extent,
        }
    }

    /// Whether every corner of `painted` lies inside this rectangle.
    fn contains(self, painted: PanelScreenBounds) -> bool {
        let limit = self.half_size + TOOLTIP_PROJECTION_ROUNDING_PIXELS;
        corners(painted).into_iter().all(|corner| {
            let offset = corner - self.centre;
            Vec2::new(offset.dot(self.x_axis), offset.dot(self.y_axis()))
                .abs()
                .cmple(limit)
                .all()
        })
    }

    /// Whether `painted` lies wholly to one side of this rectangle along the
    /// screen's axes or this rectangle's own.
    fn separated_from(self, painted: PanelScreenBounds) -> bool {
        let corners = corners(painted);
        [Vec2::X, Vec2::Y, self.x_axis, self.y_axis()]
            .into_iter()
            .any(|axis| {
                let (low, high) = corners
                    .iter()
                    .map(|corner| corner.dot(axis))
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), along| {
                        (low.min(along), high.max(along))
                    });
                let centre = self.centre.dot(axis);
                let reach = self.reach_along(axis) - TOOLTIP_PROJECTION_ROUNDING_PIXELS;
                high <= centre - reach || low >= centre + reach
            })
    }
}

/// The border a widget target's tooltip keeps clear of: the panel holding the
/// target. A tooltip's painted bounds lie inside its interior or wholly outside
/// its exterior, never across the border.
#[derive(Clone, Copy, Debug)]
pub(super) struct HostBorder {
    /// The border's inner edge, inset by the viewport margin.
    interior: OrientedScreenRect,
    /// The panel's outer edge, outset by the viewport margin.
    exterior: OrientedScreenRect,
}

impl HostBorder {
    /// The border running from the host's `outer` edge in to its `inner` edge.
    pub(super) fn new(outer: OrientedScreenRect, inner: OrientedScreenRect) -> Self {
        Self {
            interior: inner.outset(-TOOLTIP_VIEWPORT_MARGIN),
            exterior: outer.outset(TOOLTIP_VIEWPORT_MARGIN),
        }
    }

    /// Whether `painted` lies inside the interior or wholly outside the
    /// exterior.
    fn clears(self, painted: PanelScreenBounds) -> bool {
        self.interior.contains(painted) || self.exterior.separated_from(painted)
    }

    /// `sides` with the side whose outer edge lies nearest `target` first;
    /// sides at an equal distance keep their order.
    fn sides_nearest_first(
        self,
        mut sides: [TooltipSide; 4],
        target: PanelScreenBounds,
    ) -> [TooltipSide; 4] {
        let exterior = self.exterior.upright_bounds();
        let start = target.top_left();
        let end = start + target.size();
        let distance = |side: TooltipSide| match side {
            TooltipSide::Above => start.y - exterior.min.y,
            TooltipSide::Right => exterior.max.x - end.x,
            TooltipSide::Below => exterior.max.y - end.y,
            TooltipSide::Left => start.x - exterior.min.x,
        };
        sides.sort_by(|first, second| distance(*first).total_cmp(&distance(*second)));
        sides
    }
}

/// How a tooltip on one side of its control moves to keep clear of what bounds
/// it.
#[derive(Clone, Copy, Debug)]
pub(super) enum TooltipSlide {
    /// Beside its control, sliding along the side within the boundary.
    Beside(TooltipShiftBoundary),
    /// Out from its control along the side's normal until its painted bounds
    /// lie past the host's exterior, given as upright screen bounds, then
    /// sliding along the side within the placement area alone.
    OutsideHost(Rect),
}

impl TooltipSlide {
    /// The screen shift this slide gives `footprint`, placed on `side` of
    /// `target_point`, inside `placement_area`.
    pub(super) fn shift(
        self,
        side: TooltipSide,
        footprint: TooltipProjectedFootprint,
        target_point: Vec2,
        placement_area: Rect,
    ) -> Vec2 {
        match self {
            Self::Beside(boundary) => {
                limited_along_edge_shift(side, footprint, target_point, boundary, placement_area)
            },
            Self::OutsideHost(exterior) => {
                let start = footprint.painted.top_left();
                let end = start + footprint.painted.size();
                let outward = match side {
                    TooltipSide::Above => Vec2::new(0.0, (exterior.min.y - end.y).min(0.0)),
                    TooltipSide::Right => Vec2::new((exterior.max.x - start.x).max(0.0), 0.0),
                    TooltipSide::Below => Vec2::new(0.0, (exterior.max.y - start.y).max(0.0)),
                    TooltipSide::Left => Vec2::new((exterior.min.x - end.x).min(0.0), 0.0),
                };
                outward
                    + limited_along_edge_shift(
                        side,
                        footprint,
                        target_point,
                        TooltipShiftBoundary::PlacementAreaOnly,
                        placement_area,
                    )
            },
        }
    }
}

/// What bounds a screen tooltip's slide besides the placement area: the marked
/// element and the host panel holding a widget target.
#[derive(Clone, Copy, Debug)]
pub(super) enum TooltipSlideBounds {
    /// A widget inside a marked element: the tooltip slides within the element,
    /// then within its host, or moves out past the host, and keeps clear of the
    /// host's border.
    MarkedElement {
        /// The projected element, inset by the viewport margin.
        element: Rect,
        /// The host panel holding the element.
        host:    HostBorder,
    },
    /// A screen widget with no marked element, or a widget whose marked
    /// element does not project: the tooltip slides within the border's
    /// interior, or moves out past it, and keeps clear of the border.
    Host(HostBorder),
    /// A panel or screen anchor target, or a world widget with no marked
    /// element: only the placement area bounds the slide.
    PlacementAreaOnly,
}

impl TooltipSlideBounds {
    /// The marked element `bounds` inside `host`.
    pub(super) fn marked_element(bounds: PanelScreenBounds, host: HostBorder) -> Self {
        Self::MarkedElement {
            element: margin_outset(bounds, -TOOLTIP_VIEWPORT_MARGIN),
            host,
        }
    }

    /// The sides and slides in the order tried: on each of `sides` in turn,
    /// beside the control within the marked element, then within the host's
    /// interior, then within the placement area alone; after every side, out
    /// past the host's exterior, through the outer edge nearest `target` first.
    /// In a marked element, the preferred side, the first of `sides`, tries out
    /// past the host's exterior level with the control before the placement
    /// area and every other side, so a spot outside the host beats one inside
    /// it on another side, over the element's other content.
    pub(super) fn slide_order(
        self,
        sides: [TooltipSide; 4],
        target: PanelScreenBounds,
    ) -> impl Iterator<Item = (TooltipSide, TooltipSlide)> {
        let (element, host) = match self {
            Self::MarkedElement { element, host } => {
                (Some(TooltipShiftBoundary::Element(element)), Some(host))
            },
            Self::Host(host) => (None, Some(host)),
            Self::PlacementAreaOnly => (None, None),
        };
        let interior =
            host.map(|host| TooltipShiftBoundary::HostInterior(host.interior.upright_bounds()));
        let exterior = host.map(|host| host.exterior.upright_bounds());
        let preferred = sides[0];
        let beside = move |side: TooltipSide| {
            element
                .into_iter()
                .chain(interior)
                .map(move |boundary| (side, TooltipSlide::Beside(boundary)))
        };
        let anywhere = move |side: TooltipSide| {
            std::iter::once((
                side,
                TooltipSlide::Beside(TooltipShiftBoundary::PlacementAreaOnly),
            ))
        };
        let outside = move |side: TooltipSide| {
            exterior
                .into_iter()
                .map(move |exterior| (side, TooltipSlide::OutsideHost(exterior)))
        };
        let others = host.map_or_else(
            || sides.to_vec(),
            |host| host.sides_nearest_first(sides, target).to_vec(),
        );
        let preferred_outside = exterior.filter(|_| element.is_some());
        beside(preferred)
            .chain(
                preferred_outside.map(|exterior| (preferred, TooltipSlide::OutsideHost(exterior))),
            )
            .chain(anywhere(preferred))
            .chain(
                sides
                    .into_iter()
                    .skip(1)
                    .flat_map(move |side| beside(side).chain(anywhere(side))),
            )
            .chain(
                others
                    .into_iter()
                    .filter(move |&side| preferred_outside.is_none() || side != preferred)
                    .flat_map(outside),
            )
    }

    /// Whether `painted` keeps clear of the host's border, inside or outside.
    pub(super) fn clears_host_border(self, painted: PanelScreenBounds) -> bool {
        match self {
            Self::MarkedElement { host, .. } | Self::Host(host) => host.clears(painted),
            Self::PlacementAreaOnly => true,
        }
    }
}

/// Slides `footprint` along its side so its painted bounds stay inside
/// `boundary` where they can and the placement area always, keeping
/// `target_point` within the body.
///
/// Painted bounds wider than the boundary cross its edge, but only where they
/// still fit the placement area.
pub(super) fn limited_along_edge_shift(
    side: TooltipSide,
    footprint: TooltipProjectedFootprint,
    target_point: Vec2,
    boundary: TooltipShiftBoundary,
    placement_area: Rect,
) -> Vec2 {
    let axis = usize::from(!side.is_vertical());
    let start = footprint.painted.top_left()[axis];
    let length = footprint.painted.size()[axis];
    let area_min = placement_area.min[axis] + TOOLTIP_VIEWPORT_MARGIN;
    let area_max = placement_area.max[axis] - TOOLTIP_VIEWPORT_MARGIN;
    let area_shift = containment_shift(start, length, area_min, area_max);
    let desired = match boundary {
        TooltipShiftBoundary::Element(within) | TooltipShiftBoundary::HostInterior(within)
            if length <= area_max - area_min =>
        {
            containment_shift(start, length, within.min[axis], within.max[axis])
                .clamp(area_min - start, area_max - start - length)
        },
        TooltipShiftBoundary::Element(_)
        | TooltipShiftBoundary::HostInterior(_)
        | TooltipShiftBoundary::PlacementAreaOnly => area_shift,
    };
    let body_start = footprint.body.top_left()[axis];
    let body_end = body_start + footprint.body.size()[axis];
    let mut shift = Vec2::ZERO;
    shift[axis] = desired.clamp(
        target_point[axis] - body_end,
        target_point[axis] - body_start,
    );
    shift
}

pub(super) fn containment_shift(start: f32, length: f32, minimum: f32, maximum: f32) -> f32 {
    if start < minimum {
        minimum - start
    } else if start + length > maximum {
        maximum - start - length
    } else {
        0.0
    }
}

pub(super) fn bounds_intersect_viewport(bounds: PanelScreenBounds, viewport: Rect) -> bool {
    let start = bounds.top_left();
    let end = start + bounds.size();
    start.x < viewport.max.x - TOOLTIP_VIEWPORT_MARGIN
        && end.x > viewport.min.x + TOOLTIP_VIEWPORT_MARGIN
        && start.y < viewport.max.y - TOOLTIP_VIEWPORT_MARGIN
        && end.y > viewport.min.y + TOOLTIP_VIEWPORT_MARGIN
}

pub(super) fn bounds_fit_viewport(top_left: Vec2, size: Vec2, viewport: Rect) -> bool {
    let bottom_right = top_left + size;
    top_left.x >= viewport.min.x + TOOLTIP_VIEWPORT_MARGIN - TOOLTIP_PROJECTION_ROUNDING_PIXELS
        && top_left.y
            >= viewport.min.y + TOOLTIP_VIEWPORT_MARGIN - TOOLTIP_PROJECTION_ROUNDING_PIXELS
        && bottom_right.x
            <= viewport.max.x - TOOLTIP_VIEWPORT_MARGIN + TOOLTIP_PROJECTION_ROUNDING_PIXELS
        && bottom_right.y
            <= viewport.max.y - TOOLTIP_VIEWPORT_MARGIN + TOOLTIP_PROJECTION_ROUNDING_PIXELS
}

/// Whether bounds, including their painted shadow, lie wholly in the window.
pub(super) fn bounds_fit_window(top_left: Vec2, size: Vec2, viewport: Rect) -> bool {
    let bottom_right = top_left + size;
    top_left.x >= viewport.min.x - TOOLTIP_PROJECTION_ROUNDING_PIXELS
        && top_left.y >= viewport.min.y - TOOLTIP_PROJECTION_ROUNDING_PIXELS
        && bottom_right.x <= viewport.max.x + TOOLTIP_PROJECTION_ROUNDING_PIXELS
        && bottom_right.y <= viewport.max.y + TOOLTIP_PROJECTION_ROUNDING_PIXELS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_bounds_accept_subpixel_rounding_at_the_margin() {
        let viewport = tooltip::zero_origin_viewport(Vec2::splat(100.0));
        let margin = TOOLTIP_VIEWPORT_MARGIN;
        let usable = margin.mul_add(-2.0, viewport.width());
        assert!(bounds_fit_viewport(
            Vec2::new(margin - 0.000_002, margin),
            Vec2::splat(usable + 0.000_002),
            viewport,
        ));
        assert!(!bounds_fit_viewport(
            Vec2::new(margin - 0.1, margin),
            Vec2::splat(usable + 0.1),
            viewport,
        ));
    }

    #[test]
    fn keep_visible_side_order_and_tie_breaks_are_deterministic() {
        let target = PanelScreenBounds::new(Vec2::new(40.0, 40.0), Vec2::new(20.0, 20.0));
        assert!(target.is_ok());
        let Some(target) = target.ok() else {
            return;
        };
        let viewport = tooltip::zero_origin_viewport(Vec2::splat(100.0));

        assert_eq!(
            tooltip_side_order(TooltipSide::Above, target, viewport),
            [
                TooltipSide::Above,
                TooltipSide::Below,
                TooltipSide::Right,
                TooltipSide::Left,
            ]
        );
        assert_eq!(
            tooltip_side_order(TooltipSide::Left, target, viewport),
            [
                TooltipSide::Left,
                TooltipSide::Right,
                TooltipSide::Below,
                TooltipSide::Above,
            ]
        );
    }

    #[test]
    fn keep_visible_limits_along_edge_shift_to_target_overlap() {
        let top_left = Vec2::new(-13.0, 20.0);
        let source_size = Vec2::new(40.0, 20.0);
        let target_point = Vec2::new(7.0, 20.0);
        let bounds = PanelScreenBounds::new(top_left, source_size);
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let shift = limited_along_edge_shift(
            TooltipSide::Above,
            TooltipProjectedFootprint {
                body:    bounds,
                painted: bounds,
            },
            target_point,
            TooltipShiftBoundary::PlacementAreaOnly,
            tooltip::zero_origin_viewport(Vec2::splat(100.0)),
        );

        assert_eq!(shift, Vec2::new(20.0, 0.0));
        let shifted_left = top_left.x + shift.x;
        assert!(target_point.x >= shifted_left);
        assert!(target_point.x <= shifted_left + source_size.x);
    }

    /// A tooltip above its target whose body is 24 px wide and whose shadow
    /// reaches 8 px past each side.
    fn shadowed_footprint() -> Option<TooltipProjectedFootprint> {
        Some(TooltipProjectedFootprint {
            body:    PanelScreenBounds::new(Vec2::new(62.0, 20.0), Vec2::new(24.0, 10.0)).ok()?,
            painted: PanelScreenBounds::new(Vec2::new(54.0, 15.0), Vec2::new(40.0, 26.0)).ok()?,
        })
    }

    #[test]
    fn shift_slides_painted_bounds_inside_the_card_inset() {
        let footprint = shadowed_footprint();
        let card = PanelScreenBounds::new(Vec2::new(20.0, 0.0), Vec2::new(60.0, 100.0));
        assert!(footprint.is_some() && card.is_ok());
        let (Some(footprint), Ok(card)) = (footprint, card) else {
            return;
        };
        let shift = limited_along_edge_shift(
            TooltipSide::Above,
            footprint,
            Vec2::new(60.0, 30.0),
            TooltipShiftBoundary::Element(margin_outset(card, -TOOLTIP_VIEWPORT_MARGIN)),
            tooltip::zero_origin_viewport(Vec2::splat(200.0)),
        );

        assert_eq!(shift, Vec2::new(-22.0, 0.0));
        let painted_end = footprint.painted.top_left().x + footprint.painted.size().x + shift.x;
        assert!(painted_end <= 80.0 - TOOLTIP_VIEWPORT_MARGIN);
    }

    /// The upright rectangle `size` across with its top-left corner at
    /// `top_left`.
    fn upright(top_left: Vec2, size: Vec2) -> OrientedScreenRect {
        OrientedScreenRect::from_corners(
            top_left,
            top_left + Vec2::new(size.x, 0.0),
            top_left + Vec2::new(0.0, size.y),
        )
    }

    #[test]
    fn host_border_is_cleared_inside_or_outside_by_the_margin_and_never_across() {
        let host = HostBorder::new(
            upright(Vec2::new(100.0, 100.0), Vec2::new(200.0, 100.0)),
            upright(Vec2::new(102.0, 102.0), Vec2::new(196.0, 96.0)),
        );
        let margin = TOOLTIP_VIEWPORT_MARGIN;
        let painted = |x: f32, y: f32| PanelScreenBounds::new(Vec2::new(x, y), Vec2::splat(40.0));

        let cases = [
            (
                "inside the inner edge",
                painted(102.0 + margin, 102.0 + margin),
                true,
            ),
            (
                "within the margin inside",
                painted(102.0 + margin - 1.0, 120.0),
                false,
            ),
            ("across the border", painted(90.0, 120.0), false),
            (
                "within the margin outside",
                painted(100.0 - margin - 39.0, 120.0),
                false,
            ),
            (
                "left of the outer edge",
                painted(100.0 - margin - 40.0, 120.0),
                true,
            ),
            ("below the outer edge", painted(150.0, 200.0 + margin), true),
        ];
        for (case, bounds, clears) in cases {
            assert!(bounds.is_ok(), "{case}");
            let Ok(bounds) = bounds else {
                return;
            };
            assert_eq!(host.clears(bounds), clears, "{case}");
        }
    }

    #[test]
    fn turned_host_border_is_cleared_inside_or_outside_its_turned_edges() {
        // A 200 px host with a 2 px border, turned 45° about its centre.
        let centre = Vec2::splat(300.0);
        let turn = Vec2::from_angle(std::f32::consts::FRAC_PI_4);
        let turned = |half: f32| {
            let corner = |x: f32, y: f32| centre + turn.rotate(Vec2::new(x, y));
            OrientedScreenRect::from_corners(
                corner(-half, -half),
                corner(half, -half),
                corner(-half, half),
            )
        };
        let host = HostBorder::new(turned(100.0), turned(98.0));
        let painted =
            |x: f32, y: f32| PanelScreenBounds::new(centre + Vec2::new(x, y), Vec2::splat(30.0));

        let cases = [
            ("inside the turned interior", painted(-15.0, -15.0), true),
            (
                "across the turned border, inside its upright bounds",
                painted(60.0, 60.0),
                false,
            ),
            (
                "beyond the turned edge, inside its upright bounds",
                painted(110.0, 110.0),
                true,
            ),
        ];
        for (case, bounds, clears) in cases {
            assert!(bounds.is_ok(), "{case}");
            let Ok(bounds) = bounds else {
                return;
            };
            assert_eq!(host.clears(bounds), clears, "{case}");
        }
    }

    #[test]
    fn shift_lets_painted_bounds_wider_than_the_card_cross_it_inside_the_placement_area() {
        let footprint = shadowed_footprint();
        let card = PanelScreenBounds::new(Vec2::ZERO, Vec2::new(30.0, 100.0));
        assert!(footprint.is_some() && card.is_ok());
        let (Some(footprint), Ok(card)) = (footprint, card) else {
            return;
        };
        let shift = limited_along_edge_shift(
            TooltipSide::Above,
            footprint,
            Vec2::new(30.0, 30.0),
            TooltipShiftBoundary::Element(margin_outset(card, -TOOLTIP_VIEWPORT_MARGIN)),
            tooltip::zero_origin_viewport(Vec2::splat(100.0)),
        );

        assert_eq!(shift, Vec2::new(-46.0, 0.0));
        let painted_start = footprint.painted.top_left().x + shift.x;
        let painted_end = painted_start + footprint.painted.size().x;
        assert!(painted_start >= TOOLTIP_VIEWPORT_MARGIN);
        assert!(painted_end > 30.0 - TOOLTIP_VIEWPORT_MARGIN);
        assert!(painted_end <= 100.0 - TOOLTIP_VIEWPORT_MARGIN);
    }
}
