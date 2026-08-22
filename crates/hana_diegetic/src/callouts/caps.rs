use bevy::color::Color;

use crate::layout::Dimension;

/// Visual style for arrow end caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowStyle {
    /// Open chevron made from two line segments.
    Open,
    /// Solid triangular arrowhead with a sharp point.
    Solid,
}

/// Which side of an open arrow cap a wing occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CalloutCapWing {
    /// Positive perpendicular side.
    Positive,
    /// Negative perpendicular side.
    Negative,
}

impl CalloutCapWing {
    /// Returns the signed multiplier for this wing's perpendicular offset.
    #[must_use]
    pub(crate) const fn sign(self) -> f32 {
        match self {
            Self::Positive => 1.0,
            Self::Negative => -1.0,
        }
    }
}

/// Low-level cap primitive produced by the shared cap resolver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CalloutCapPrimitiveKind {
    /// One segment of an open arrow cap.
    OpenArrowWing(CalloutCapWing),
    /// Filled triangular arrow cap.
    Triangle,
    /// Filled circular cap.
    Circle,
    /// Filled square cap.
    Square,
    /// Filled diamond cap.
    Diamond,
}

/// A resolved cap primitive in point-space or world-space caller units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedCalloutCapPrimitive {
    /// Primitive shape to render.
    pub(crate) kind:   CalloutCapPrimitiveKind,
    /// Length along the line direction.
    pub(crate) length: f32,
    /// Width across the line direction.
    pub(crate) width:  f32,
    /// Resolved primitive color.
    pub(crate) color:  Color,
    /// Stable ordering within this cap.
    part_order:        usize,
}

/// Resolved cap data shared by standalone callouts and panel lines.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedCalloutCap {
    /// Amount the adjoining shaft moves inward to avoid the cap.
    pub(crate) shaft_inset: f32,
    primitives:             Vec<ResolvedCalloutCapPrimitive>,
}

impl ResolvedCalloutCap {
    /// Returns resolved cap primitives in rendering order.
    #[must_use]
    pub(crate) fn primitives(&self) -> &[ResolvedCalloutCapPrimitive] { &self.primitives }
}

/// Arrow-cap configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ArrowCap {
    pub(super) style:  ArrowStyle,
    pub(super) length: Option<Dimension>,
    pub(super) width:  Option<Dimension>,
    pub(super) color:  Option<Color>,
}

impl ArrowCap {
    /// Creates a default arrow cap.
    #[must_use]
    pub(super) const fn new() -> Self {
        Self {
            style:  ArrowStyle::Open,
            length: None,
            width:  None,
            color:  None,
        }
    }

    /// Uses the open chevron arrow style.
    #[must_use]
    pub(super) const fn open(mut self) -> Self {
        self.style = ArrowStyle::Open;
        self
    }

    /// Uses the solid triangular arrow style.
    #[must_use]
    pub(super) const fn solid(mut self) -> Self {
        self.style = ArrowStyle::Solid;
        self
    }

    /// Sets the cap length along the line direction.
    #[must_use]
    pub(super) const fn length(mut self, length: f32) -> Self {
        self.length = Some(raw_dimension(length));
        self
    }

    /// Sets the cap length along the line direction with a typed dimension.
    #[must_use]
    pub(super) fn length_dimension(mut self, length: impl Into<Dimension>) -> Self {
        self.length = Some(length.into());
        self
    }

    /// Sets the cap width across the line direction.
    #[must_use]
    pub(super) const fn width(mut self, width: f32) -> Self {
        self.width = Some(raw_dimension(width));
        self
    }

    /// Sets the cap width across the line direction with a typed dimension.
    #[must_use]
    pub(super) fn width_dimension(mut self, width: impl Into<Dimension>) -> Self {
        self.width = Some(width.into());
        self
    }

    /// Overrides the cap color. Defaults to the line color.
    #[must_use]
    pub(super) const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    fn scaled_dimensions(self, default_scale: f32) -> Self {
        Self {
            style:  self.style,
            length: self
                .length
                .map(|dimension| scaled_dimension(dimension, default_scale)),
            width:  self
                .width
                .map(|dimension| scaled_dimension(dimension, default_scale)),
            color:  self.color,
        }
    }
}

/// Circle-cap configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CircleCap {
    pub(super) radius: Option<Dimension>,
    pub(super) color:  Option<Color>,
}

impl CircleCap {
    /// Creates a default circle cap.
    #[must_use]
    pub(super) const fn new() -> Self {
        Self {
            radius: None,
            color:  None,
        }
    }

    /// Sets the circle radius.
    #[must_use]
    pub(super) const fn radius(mut self, radius: f32) -> Self {
        self.radius = Some(raw_dimension(radius));
        self
    }

    /// Sets the circle radius with a typed dimension.
    #[must_use]
    pub(super) fn radius_dimension(mut self, radius: impl Into<Dimension>) -> Self {
        self.radius = Some(radius.into());
        self
    }

    /// Overrides the cap color. Defaults to the line color.
    #[must_use]
    pub(super) const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    fn scaled_dimensions(self, default_scale: f32) -> Self {
        Self {
            radius: self
                .radius
                .map(|dimension| scaled_dimension(dimension, default_scale)),
            color:  self.color,
        }
    }
}

/// Square-cap configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SquareCap {
    pub(super) size:  Option<Dimension>,
    pub(super) color: Option<Color>,
}

impl SquareCap {
    /// Creates a default square cap.
    #[must_use]
    pub(super) const fn new() -> Self {
        Self {
            size:  None,
            color: None,
        }
    }

    /// Sets the full square size.
    #[must_use]
    pub(super) const fn size(mut self, size: f32) -> Self {
        self.size = Some(raw_dimension(size));
        self
    }

    /// Sets the full square size with a typed dimension.
    #[must_use]
    pub(super) fn size_dimension(mut self, size: impl Into<Dimension>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// Overrides the cap color. Defaults to the line color.
    #[must_use]
    pub(super) const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    fn scaled_dimensions(self, default_scale: f32) -> Self {
        Self {
            size:  self
                .size
                .map(|dimension| scaled_dimension(dimension, default_scale)),
            color: self.color,
        }
    }
}

/// Diamond-cap configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DiamondCap {
    pub(super) width:  Option<Dimension>,
    pub(super) height: Option<Dimension>,
    pub(super) color:  Option<Color>,
}

impl DiamondCap {
    /// Creates a default diamond cap.
    #[must_use]
    pub(super) const fn new() -> Self {
        Self {
            width:  None,
            height: None,
            color:  None,
        }
    }

    /// Sets the full diamond width.
    #[must_use]
    pub(super) const fn width(mut self, width: f32) -> Self {
        self.width = Some(raw_dimension(width));
        self
    }

    /// Sets the full diamond width with a typed dimension.
    #[must_use]
    pub(super) fn width_dimension(mut self, width: impl Into<Dimension>) -> Self {
        self.width = Some(width.into());
        self
    }

    /// Sets the full diamond height.
    #[must_use]
    pub(super) const fn height(mut self, height: f32) -> Self {
        self.height = Some(raw_dimension(height));
        self
    }

    /// Sets the full diamond height with a typed dimension.
    #[must_use]
    pub(super) fn height_dimension(mut self, height: impl Into<Dimension>) -> Self {
        self.height = Some(height.into());
        self
    }

    /// Overrides the cap color. Defaults to the line color.
    #[must_use]
    pub(super) const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    fn scaled_dimensions(self, default_scale: f32) -> Self {
        Self {
            width:  self
                .width
                .map(|dimension| scaled_dimension(dimension, default_scale)),
            height: self
                .height
                .map(|dimension| scaled_dimension(dimension, default_scale)),
            color:  self.color,
        }
    }
}

/// Decoration that can appear at either end of a [`PanelLine`](crate::PanelLine).
#[derive(Clone, Copy, Debug, PartialEq)]
#[expect(
    private_interfaces,
    reason = "variant payload structs are crate-internal; users configure caps via CalloutCap builder methods"
)]
pub enum CalloutCap {
    /// No end cap.
    None,
    /// Arrow end cap with the given style.
    Arrow(ArrowCap),
    /// Filled circular cap.
    Circle(CircleCap),
    /// Filled square cap.
    Square(SquareCap),
    /// Filled diamond cap.
    Diamond(DiamondCap),
}

impl CalloutCap {
    /// Creates an arrow cap with default open styling.
    #[must_use]
    pub const fn arrow() -> Self { Self::Arrow(ArrowCap::new()) }

    /// Creates a circular cap.
    #[must_use]
    pub const fn circle() -> Self { Self::Circle(CircleCap::new()) }

    /// Creates a square cap.
    #[must_use]
    pub const fn square() -> Self { Self::Square(SquareCap::new()) }

    /// Creates a diamond cap.
    #[must_use]
    pub const fn diamond() -> Self { Self::Diamond(DiamondCap::new()) }

    /// Sets an arrow cap to the open chevron style.
    #[must_use]
    pub const fn open(self) -> Self {
        match self {
            Self::Arrow(cap) => Self::Arrow(cap.open()),
            other => other,
        }
    }

    /// Sets an arrow cap to the solid triangular style.
    #[must_use]
    pub const fn solid(self) -> Self {
        match self {
            Self::Arrow(cap) => Self::Arrow(cap.solid()),
            other => other,
        }
    }

    /// Sets the cap length along the line direction.
    #[must_use]
    pub const fn length(self, length: f32) -> Self {
        match self {
            Self::Arrow(cap) => Self::Arrow(cap.length(length)),
            other => other,
        }
    }

    /// Sets the cap length along the line direction with a typed dimension.
    #[must_use]
    pub fn length_dimension(self, length: impl Into<Dimension>) -> Self {
        match self {
            Self::Arrow(cap) => Self::Arrow(cap.length_dimension(length)),
            other => other,
        }
    }

    /// Sets the cap width across the line direction.
    #[must_use]
    pub const fn width(self, width: f32) -> Self {
        match self {
            Self::Arrow(cap) => Self::Arrow(cap.width(width)),
            Self::Diamond(cap) => Self::Diamond(cap.width(width)),
            other => other,
        }
    }

    /// Sets the cap width across the line direction with a typed dimension.
    #[must_use]
    pub fn width_dimension(self, width: impl Into<Dimension>) -> Self {
        match self {
            Self::Arrow(cap) => Self::Arrow(cap.width_dimension(width)),
            Self::Diamond(cap) => Self::Diamond(cap.width_dimension(width)),
            other => other,
        }
    }

    /// Sets the cap height for cap variants that support an explicit height.
    #[must_use]
    pub const fn height(self, height: f32) -> Self {
        match self {
            Self::Diamond(cap) => Self::Diamond(cap.height(height)),
            other => other,
        }
    }

    /// Sets the cap height with a typed dimension.
    #[must_use]
    pub fn height_dimension(self, height: impl Into<Dimension>) -> Self {
        match self {
            Self::Diamond(cap) => Self::Diamond(cap.height_dimension(height)),
            other => other,
        }
    }

    /// Sets the cap radius for circular caps.
    #[must_use]
    pub const fn radius(self, radius: f32) -> Self {
        match self {
            Self::Circle(cap) => Self::Circle(cap.radius(radius)),
            other => other,
        }
    }

    /// Sets the cap radius with a typed dimension.
    #[must_use]
    pub fn radius_dimension(self, radius: impl Into<Dimension>) -> Self {
        match self {
            Self::Circle(cap) => Self::Circle(cap.radius_dimension(radius)),
            other => other,
        }
    }

    /// Sets the cap size for square caps.
    #[must_use]
    pub const fn size(self, size: f32) -> Self {
        match self {
            Self::Square(cap) => Self::Square(cap.size(size)),
            other => other,
        }
    }

    /// Sets the cap size with a typed dimension.
    #[must_use]
    pub fn size_dimension(self, size: impl Into<Dimension>) -> Self {
        match self {
            Self::Square(cap) => Self::Square(cap.size_dimension(size)),
            other => other,
        }
    }

    /// Overrides the cap color. Defaults to the callout line color.
    #[must_use]
    pub const fn color(self, color: Color) -> Self {
        match self {
            Self::Arrow(cap) => Self::Arrow(cap.color(color)),
            Self::Circle(cap) => Self::Circle(cap.color(color)),
            Self::Square(cap) => Self::Square(cap.color(color)),
            Self::Diamond(cap) => Self::Diamond(cap.color(color)),
            Self::None => Self::None,
        }
    }

    pub(crate) fn scaled_dimensions(self, default_scale: f32) -> Self {
        match self {
            Self::None => Self::None,
            Self::Arrow(cap) => Self::Arrow(cap.scaled_dimensions(default_scale)),
            Self::Circle(cap) => Self::Circle(cap.scaled_dimensions(default_scale)),
            Self::Square(cap) => Self::Square(cap.scaled_dimensions(default_scale)),
            Self::Diamond(cap) => Self::Diamond(cap.scaled_dimensions(default_scale)),
        }
    }

    pub(crate) fn resolved_primitives(
        self,
        default_size: f32,
        fallback_color: Color,
        resolve_dimension: impl Fn(Dimension) -> f32,
    ) -> ResolvedCalloutCap {
        match self {
            Self::None => ResolvedCalloutCap {
                shaft_inset: 0.0,
                primitives:  Vec::new(),
            },
            Self::Arrow(cap) => {
                let length = resolve_or_default(cap.length, default_size, &resolve_dimension);
                let width = resolve_or_default(cap.width, length, &resolve_dimension);
                let color = cap.color.unwrap_or(fallback_color);
                let primitives = match cap.style {
                    ArrowStyle::Open => vec![
                        ResolvedCalloutCapPrimitive {
                            kind: CalloutCapPrimitiveKind::OpenArrowWing(CalloutCapWing::Positive),
                            length,
                            width,
                            color,
                            part_order: 0,
                        },
                        ResolvedCalloutCapPrimitive {
                            kind: CalloutCapPrimitiveKind::OpenArrowWing(CalloutCapWing::Negative),
                            length,
                            width,
                            color,
                            part_order: 1,
                        },
                    ],
                    ArrowStyle::Solid => vec![ResolvedCalloutCapPrimitive {
                        kind: CalloutCapPrimitiveKind::Triangle,
                        length,
                        width,
                        color,
                        part_order: 0,
                    }],
                };
                ResolvedCalloutCap {
                    shaft_inset: length,
                    primitives,
                }
            },
            Self::Circle(cap) => {
                let radius = resolve_or_default(cap.radius, default_size * 0.5, &resolve_dimension);
                let diameter = radius * 2.0;
                ResolvedCalloutCap {
                    shaft_inset: radius,
                    primitives:  vec![ResolvedCalloutCapPrimitive {
                        kind:       CalloutCapPrimitiveKind::Circle,
                        length:     diameter,
                        width:      diameter,
                        color:      cap.color.unwrap_or(fallback_color),
                        part_order: 0,
                    }],
                }
            },
            Self::Square(cap) => {
                let size = resolve_or_default(cap.size, default_size, &resolve_dimension);
                ResolvedCalloutCap {
                    shaft_inset: size * 0.5,
                    primitives:  vec![ResolvedCalloutCapPrimitive {
                        kind:       CalloutCapPrimitiveKind::Square,
                        length:     size,
                        width:      size,
                        color:      cap.color.unwrap_or(fallback_color),
                        part_order: 0,
                    }],
                }
            },
            Self::Diamond(cap) => {
                let width = resolve_or_default(cap.width, default_size, &resolve_dimension);
                let height = resolve_or_default(cap.height, width, &resolve_dimension);
                ResolvedCalloutCap {
                    shaft_inset: width * 0.5,
                    primitives:  vec![ResolvedCalloutCapPrimitive {
                        kind:       CalloutCapPrimitiveKind::Diamond,
                        length:     width,
                        width:      height,
                        color:      cap.color.unwrap_or(fallback_color),
                        part_order: 0,
                    }],
                }
            },
        }
    }
}

fn resolve_or_default(
    dimension: Option<Dimension>,
    default_value: f32,
    resolve_dimension: &impl Fn(Dimension) -> f32,
) -> f32 {
    dimension.map_or(default_value, resolve_dimension)
}

const fn raw_dimension(value: f32) -> Dimension { Dimension { value, unit: None } }

fn scaled_dimension(dimension: Dimension, default_scale: f32) -> Dimension {
    Dimension {
        value: dimension.to_points(default_scale),
        unit:  None,
    }
}
