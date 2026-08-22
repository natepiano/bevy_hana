use super::AlignX;
use super::AlignY;
use super::ChildDivider;
use super::Dimension;

/// How a parent layout treats a specific axis for child sizing and positioning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AxisRole {
    RowMain,
    ColumnMain,
    Cross,
    Overlay,
}

/// Internal child layout mode stored on [`Element`](super::element::Element).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChildLayout {
    /// Children are laid out from left to right.
    Row {
        /// Main-axis spacing between adjacent children.
        gap:     Dimension,
        /// Horizontal child alignment.
        align_x: AlignX,
        /// Vertical child alignment.
        align_y: AlignY,
        /// Optional separator between adjacent child slots.
        divider: Option<ChildDivider>,
    },
    /// Children are laid out from top to bottom.
    Column {
        /// Main-axis spacing between adjacent children.
        gap:     Dimension,
        /// Horizontal child alignment.
        align_x: AlignX,
        /// Vertical child alignment.
        align_y: AlignY,
        /// Optional separator between adjacent child slots.
        divider: Option<ChildDivider>,
    },
    /// Children are independently positioned inside the parent content box.
    Overlay {
        /// Horizontal child alignment.
        align_x: AlignX,
        /// Vertical child alignment.
        align_y: AlignY,
    },
}

impl ChildLayout {
    /// Returns horizontal child alignment.
    #[must_use]
    pub(super) const fn align_x(&self) -> AlignX {
        match self {
            Self::Row { align_x, .. }
            | Self::Column { align_x, .. }
            | Self::Overlay { align_x, .. } => *align_x,
        }
    }

    /// Returns vertical child alignment.
    #[must_use]
    pub(super) const fn align_y(&self) -> AlignY {
        match self {
            Self::Row { align_y, .. }
            | Self::Column { align_y, .. }
            | Self::Overlay { align_y, .. } => *align_y,
        }
    }

    /// Returns the optional row or column child divider.
    #[must_use]
    pub(super) const fn divider(&self) -> Option<ChildDivider> {
        match self {
            Self::Row { divider, .. } | Self::Column { divider, .. } => *divider,
            Self::Overlay { .. } => None,
        }
    }

    /// Returns the X-axis role for this child layout.
    #[must_use]
    pub(super) const fn x_axis_role(&self) -> AxisRole {
        match self {
            Self::Row { .. } => AxisRole::RowMain,
            Self::Column { .. } => AxisRole::Cross,
            Self::Overlay { .. } => AxisRole::Overlay,
        }
    }

    /// Returns the Y-axis role for this child layout.
    #[must_use]
    pub(super) const fn y_axis_role(&self) -> AxisRole {
        match self {
            Self::Row { .. } => AxisRole::Cross,
            Self::Column { .. } => AxisRole::ColumnMain,
            Self::Overlay { .. } => AxisRole::Overlay,
        }
    }

    /// Returns the main-axis gap for row or column children.
    #[must_use]
    pub(super) const fn main_gap(&self) -> Option<Dimension> {
        match self {
            Self::Row { gap, .. } | Self::Column { gap, .. } => Some(*gap),
            Self::Overlay { .. } => None,
        }
    }

    /// Resolves row/column dimensions to points while preserving alignment.
    #[must_use]
    pub(super) fn to_points(self, layout_scale: f32) -> Self {
        match self {
            Self::Row {
                gap,
                align_x,
                align_y,
                divider,
            } => Self::Row {
                gap: Dimension {
                    value: gap.to_points(layout_scale),
                    unit:  None,
                },
                align_x,
                align_y,
                divider: divider.map(|divider| divider.to_points(layout_scale)),
            },
            Self::Column {
                gap,
                align_x,
                align_y,
                divider,
            } => Self::Column {
                gap: Dimension {
                    value: gap.to_points(layout_scale),
                    unit:  None,
                },
                align_x,
                align_y,
                divider: divider.map(|divider| divider.to_points(layout_scale)),
            },
            Self::Overlay { align_x, align_y } => Self::Overlay { align_x, align_y },
        }
    }
}

impl Default for ChildLayout {
    fn default() -> Self {
        Self::Row {
            gap:     Dimension::default(),
            align_x: AlignX::default(),
            align_y: AlignY::default(),
            divider: None,
        }
    }
}
