use bevy::prelude::Reflect;

/// Screen-space anchor used when placing fitted bounds inside the viewport.
///
/// The fraction matches common top-left UI coordinates: `TopLeft` is `(0, 0)`,
/// `Center` is `(0.5, 0.5)`, and `BottomRight` is `(1, 1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum FitAnchor {
    /// Align the fitted bounds' top-left point to the viewport's top-left point.
    TopLeft,
    /// Align the fitted bounds' top-center point to the viewport's top-center point.
    TopCenter,
    /// Align the fitted bounds' top-right point to the viewport's top-right point.
    TopRight,
    /// Align the fitted bounds' center-left point to the viewport's center-left point.
    CenterLeft,
    /// Align the fitted bounds' center point to the viewport's center point.
    #[default]
    Center,
    /// Align the fitted bounds' center-right point to the viewport's center-right point.
    CenterRight,
    /// Align the fitted bounds' bottom-left point to the viewport's bottom-left point.
    BottomLeft,
    /// Align the fitted bounds' bottom-center point to the viewport's bottom-center point.
    BottomCenter,
    /// Align the fitted bounds' bottom-right point to the viewport's bottom-right point.
    BottomRight,
}

impl FitAnchor {
    /// Returns the anchor's fraction from top-left as `(x, y)`.
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
}
