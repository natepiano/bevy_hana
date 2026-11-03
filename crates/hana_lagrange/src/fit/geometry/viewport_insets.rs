use bevy::prelude::Reflect;

/// Window space, in logical pixels, that a fit keeps its target out of.
///
/// Each side is the distance from that window edge to the region the fit
/// frames into, so an overlay panel along an edge can reserve its space. The
/// fit sizes and anchors the target inside the region that remains: `margin`
/// is a fraction of that region and `offset_px` moves within it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct FitViewportInsets {
    left:   f32,
    top:    f32,
    right:  f32,
    bottom: f32,
}

impl FitViewportInsets {
    /// No reserved space: the fit frames into the whole viewport.
    pub const NONE: Self = Self {
        left:   0.0,
        top:    0.0,
        right:  0.0,
        bottom: 0.0,
    };

    /// Creates insets from the four window edges, in logical pixels.
    ///
    /// Negative or non-finite values reserve nothing on that side.
    #[must_use]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left:   reserved_pixels(left),
            top:    reserved_pixels(top),
            right:  reserved_pixels(right),
            bottom: reserved_pixels(bottom),
        }
    }

    /// Distance from the left window edge, in logical pixels.
    #[must_use]
    pub(super) const fn left(self) -> f32 { self.left }

    /// Distance from the top window edge, in logical pixels.
    #[must_use]
    pub(super) const fn top(self) -> f32 { self.top }

    /// Distance from the right window edge, in logical pixels.
    #[must_use]
    pub(super) const fn right(self) -> f32 { self.right }

    /// Distance from the bottom window edge, in logical pixels.
    #[must_use]
    pub(super) const fn bottom(self) -> f32 { self.bottom }

    /// Whether any side reserves space.
    #[must_use]
    pub(super) fn reserves_space(self) -> bool { self != Self::NONE }
}

const fn reserved_pixels(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
