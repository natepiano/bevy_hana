use bevy::prelude::Reflect;

/// `Anchor` point for standalone text positioning.
///
/// Determines which point of the text block's bounding box is placed
/// at the entity's [`Transform`](bevy::prelude::Transform) position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum Anchor {
    /// Top-left corner at the transform position.
    TopLeft,
    /// Top-center at the transform position.
    TopCenter,
    /// Top-right corner at the transform position.
    TopRight,
    /// Center-left at the transform position.
    CenterLeft,
    /// Center of the text block at the transform position.
    #[default]
    Center,
    /// Center-right at the transform position.
    CenterRight,
    /// Bottom-left corner at the transform position.
    BottomLeft,
    /// Bottom-center at the transform position.
    BottomCenter,
    /// Bottom-right corner at the transform position.
    BottomRight,
}

impl Anchor {
    /// Returns the offset from the top-left corner as a fraction of (width, height).
    ///
    /// For `TopLeft` this is (0, 0). For `Center` it's (0.5, 0.5).
    /// Multiply by the actual width/height to get the offset in whatever units.
    #[must_use]
    pub const fn offset_fraction(self) -> (f32, f32) {
        let x = match self {
            Self::TopLeft | Self::CenterLeft | Self::BottomLeft => 0.0,
            Self::TopCenter | Self::Center | Self::BottomCenter => 0.5,
            Self::TopRight | Self::CenterRight | Self::BottomRight => 1.0,
        };
        let y = match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => 0.0,
            Self::CenterLeft | Self::Center | Self::CenterRight => 0.5,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => 1.0,
        };
        (x, y)
    }

    /// Returns the anchor offset for a bounding box of the given size.
    #[must_use]
    pub fn offset(self, width: f32, height: f32) -> (f32, f32) {
        let (fx, fy) = self.offset_fraction();
        (width * fx, height * fy)
    }
}
