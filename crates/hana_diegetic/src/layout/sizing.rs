//! Layout sizing and child-flow types.

use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;

use super::Dimension;

/// `Sizing` behavior for a layout element along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Default, PartialEq, Debug)]
pub enum Sizing {
    /// Shrink-wrap to content, clamped to `[min, max]`.
    ///
    /// The element's content size is computed first (e.g. via text measurement
    /// or children accumulation), then clamped to the `[min, max]` range.
    /// If content is smaller than `min`, the element grows to `min`.
    /// If content is larger than `max`, the element is capped at `max`.
    Fit {
        /// Minimum size.
        min: Dimension,
        /// Maximum size.
        max: Dimension,
    },
    /// Expand to fill remaining parent space, clamped to `[min, max]`.
    ///
    /// After all non-`Grow` siblings are sized, remaining space is distributed
    /// among `Grow` siblings using a smallest-first equalising heuristic:
    /// the smallest `Grow` elements receive space first until they match the
    /// next-smallest, then all are grown together, repeating until space is
    /// exhausted or every element hits its `max`.
    ///
    /// `min` acts as a guaranteed floor -- the element never shrinks below it.
    /// `max` caps expansion -- the element stops growing once it reaches `max`.
    Grow {
        /// Minimum size.
        min: Dimension,
        /// Maximum size.
        max: Dimension,
    },
    /// Exact size.
    Fixed(Dimension),
    /// Fraction of the parent's size along this axis (0.0--1.0).
    ///
    /// Along the parent's layout direction, padding and child gaps are
    /// subtracted before computing the fraction.
    Percent(f32),
}

impl Default for Sizing {
    fn default() -> Self {
        Self::Fit {
            min: Dimension {
                value: 0.0,
                unit:  None,
            },
            max: Dimension {
                value: f32::INFINITY,
                unit:  None,
            },
        }
    }
}

impl Sizing {
    /// Shrink-wrap to content with no size constraints.
    ///
    /// The element's content size is computed first (e.g. via text measurement
    /// or children accumulation) and used as-is — there is no minimum floor
    /// and no maximum cap.
    pub const FIT: Self = Self::Fit {
        min: Dimension {
            value: 0.0,
            unit:  None,
        },
        max: Dimension {
            value: f32::MAX,
            unit:  None,
        },
    };

    /// Expand to fill available space with no size constraints.
    ///
    /// After all non-`Grow` siblings are sized, remaining space is distributed
    /// among `Grow` siblings using a smallest-first equalising heuristic.
    /// With no constraints, this element will absorb as much space as possible.
    pub const GROW: Self = Self::Grow {
        min: Dimension {
            value: 0.0,
            unit:  None,
        },
        max: Dimension {
            value: f32::MAX,
            unit:  None,
        },
    };

    /// Shrink-wrap to content with a minimum floor.
    ///
    /// The element will never be smaller than `min`, even if content is smaller.
    #[must_use]
    pub fn fit_min(min: impl Into<Dimension>) -> Self {
        Self::Fit {
            min: min.into(),
            max: Dimension {
                value: f32::MAX,
                unit:  None,
            },
        }
    }

    /// Shrink-wrap to content, clamped to `[min, max]`.
    ///
    /// Content smaller than `min` grows to `min`; content larger than `max`
    /// is capped at `max`.
    #[must_use]
    pub fn fit_range(min: impl Into<Dimension>, max: impl Into<Dimension>) -> Self {
        Self::Fit {
            min: min.into(),
            max: max.into(),
        }
    }

    /// Expand to fill available space with a minimum floor.
    ///
    /// The element is guaranteed at least `min` even if no space remains.
    #[must_use]
    pub fn grow_min(min: impl Into<Dimension>) -> Self {
        Self::Grow {
            min: min.into(),
            max: Dimension {
                value: f32::MAX,
                unit:  None,
            },
        }
    }

    /// Expand to fill available space, clamped to `[min, max]`.
    ///
    /// `min` is a guaranteed floor; `max` caps expansion.
    #[must_use]
    pub fn grow_range(min: impl Into<Dimension>, max: impl Into<Dimension>) -> Self {
        Self::Grow {
            min: min.into(),
            max: max.into(),
        }
    }

    /// Exact size, ignoring content and siblings.
    #[must_use]
    pub fn fixed(size: impl Into<Dimension>) -> Self { Self::Fixed(size.into()) }

    /// Fraction of the parent's content area (0.0–1.0).
    #[must_use]
    pub const fn percent(fraction: f32) -> Self { Self::Percent(fraction) }

    /// Returns the minimum bound for this sizing rule (resolved `.value`).
    #[must_use]
    pub const fn min_size(&self) -> f32 {
        match self {
            Self::Fit { min, .. } | Self::Grow { min, .. } => min.value,
            Self::Fixed(size) => size.value,
            Self::Percent(_) => 0.0,
        }
    }

    /// Returns the maximum bound for this sizing rule (resolved `.value`).
    #[must_use]
    pub const fn max_size(&self) -> f32 {
        match self {
            Self::Fit { max, .. } | Self::Grow { max, .. } => max.value,
            Self::Fixed(size) => size.value,
            Self::Percent(_) => f32::INFINITY,
        }
    }

    /// Returns `true` if this is a `Grow` variant.
    #[must_use]
    pub const fn is_grow(&self) -> bool { matches!(self, Self::Grow { .. }) }

    /// Returns `true` if this is a `Fit` variant.
    #[must_use]
    pub const fn is_fit(&self) -> bool { matches!(self, Self::Fit { .. }) }

    /// Returns `true` if this element can be compressed during overflow.
    #[must_use]
    pub const fn is_resizable(&self) -> bool {
        matches!(self, Self::Fit { .. } | Self::Grow { .. })
    }

    /// Resolves all dimensions to points and returns a copy with plain values.
    ///
    /// Dimensions with an explicit unit convert via `unit.to_points()`.
    /// Dimensions without a unit (bare `f32`) use `default_scale`.
    /// `Percent` is unchanged (it's a fraction, not a spatial value).
    #[must_use]
    pub fn resolved(self, default_scale: f32) -> Self {
        match self {
            Self::Fit { min, max } => Self::Fit {
                min: Dimension {
                    value: min.to_points(default_scale),
                    unit:  None,
                },
                max: Dimension {
                    value: max.to_points(default_scale),
                    unit:  None,
                },
            },
            Self::Grow { min, max } => Self::Grow {
                min: Dimension {
                    value: min.to_points(default_scale),
                    unit:  None,
                },
                max: Dimension {
                    value: max.to_points(default_scale),
                    unit:  None,
                },
            },
            Self::Fixed(size) => Self::Fixed(Dimension {
                value: size.to_points(default_scale),
                unit:  None,
            }),
            Self::Percent(frac) => Self::Percent(frac),
        }
    }
}

/// `Direction` in which children are laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// Children flow left to right.
    #[default]
    LeftToRight,
    /// Children flow top to bottom.
    TopToBottom,
}

/// Horizontal alignment of children within their parent.
///
/// When [`Direction::LeftToRight`], this controls main-axis alignment (distributes
/// extra space before/after the row of children). When [`Direction::TopToBottom`],
/// this controls cross-axis alignment (positions each child horizontally within
/// the parent's content area).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlignX {
    /// Align to the left edge.
    #[default]
    Left,
    /// Center horizontally.
    Center,
    /// Align to the right edge.
    Right,
}

/// Vertical alignment of children within their parent.
///
/// When [`Direction::TopToBottom`], this controls main-axis alignment (distributes
/// extra space before/after the column of children). When [`Direction::LeftToRight`],
/// this controls cross-axis alignment (positions each child vertically within
/// the parent's content area).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlignY {
    /// Align to the top edge.
    #[default]
    Top,
    /// Center vertically.
    Center,
    /// Align to the bottom edge.
    Bottom,
}

/// Interior padding between an element's edges and its children.
///
/// Note: [`Sizing::Percent`] on child elements is computed against the parent's
/// content area (i.e., after this padding and child gap are subtracted).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Padding {
    /// Left padding.
    pub left:   Dimension,
    /// Right padding.
    pub right:  Dimension,
    /// Top padding.
    pub top:    Dimension,
    /// Bottom padding.
    pub bottom: Dimension,
}

impl Default for Padding {
    fn default() -> Self {
        Self {
            left:   Dimension {
                value: 0.0,
                unit:  None,
            },
            right:  Dimension {
                value: 0.0,
                unit:  None,
            },
            top:    Dimension {
                value: 0.0,
                unit:  None,
            },
            bottom: Dimension {
                value: 0.0,
                unit:  None,
            },
        }
    }
}

impl Padding {
    /// Uniform padding on all sides.
    #[must_use]
    pub fn all(value: impl Into<Dimension>) -> Self {
        let value = value.into();
        Self {
            left:   value,
            right:  value,
            top:    value,
            bottom: value,
        }
    }

    /// Symmetric padding: `x` for left/right, `y` for top/bottom.
    #[must_use]
    pub fn xy(x: impl Into<Dimension>, y: impl Into<Dimension>) -> Self {
        let x = x.into();
        let y = y.into();
        Self {
            left:   x,
            right:  x,
            top:    y,
            bottom: y,
        }
    }

    /// Individual padding per side.
    #[must_use]
    pub fn new(
        left: impl Into<Dimension>,
        right: impl Into<Dimension>,
        top: impl Into<Dimension>,
        bottom: impl Into<Dimension>,
    ) -> Self {
        Self {
            left:   left.into(),
            right:  right.into(),
            top:    top.into(),
            bottom: bottom.into(),
        }
    }

    /// Total horizontal padding (left + right) in resolved units.
    #[must_use]
    pub const fn horizontal(&self) -> f32 { self.left.value + self.right.value }

    /// Total vertical padding (top + bottom) in resolved units.
    #[must_use]
    pub const fn vertical(&self) -> f32 { self.top.value + self.bottom.value }

    /// Resolves all dimensions to points and returns a copy with plain values.
    ///
    /// Dimensions with an explicit unit convert via `unit.to_points()`.
    /// Dimensions without a unit (bare f32) use `default_scale` (typically
    /// `layout_to_points`).
    #[must_use]
    pub fn resolved(self, default_scale: f32) -> Self {
        Self {
            left:   Dimension {
                value: self.left.to_points(default_scale),
                unit:  None,
            },
            right:  Dimension {
                value: self.right.to_points(default_scale),
                unit:  None,
            },
            top:    Dimension {
                value: self.top.to_points(default_scale),
                unit:  None,
            },
            bottom: Dimension {
                value: self.bottom.to_points(default_scale),
                unit:  None,
            },
        }
    }
}
