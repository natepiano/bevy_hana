//! Semantic IME session targets.

use bevy::prelude::Entity;
use bevy::prelude::Rect;
use bevy::prelude::Vec2;

use super::PanelElementId;

/// Semantic backing target for an IME session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImeTarget {
    /// Editable field authored on a world-space diegetic panel.
    WorldPanelField {
        /// Panel entity that owns the field id.
        panel:    Entity,
        /// Panel-local semantic field id.
        field_id: PanelElementId,
    },
    /// Editable field authored on a screen-space diegetic panel.
    ScreenPanelField {
        /// Panel entity that owns the field id.
        panel:    Entity,
        /// Panel-local semantic field id.
        field_id: PanelElementId,
    },
    /// Caller-owned session with app-specific backing state.
    AppOwned {
        /// Entity that owns the app-side field state.
        owner:    Entity,
        /// Owner-local semantic field id.
        field_id: PanelElementId,
    },
}

/// Screen-space placement supplied by caller-owned IME sessions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImeSessionAnchor {
    /// Anchor to a screen-space rectangle in logical pixels.
    ScreenRect(Rect),
    /// Anchor to a screen-space point in logical pixels.
    ScreenPoint(Vec2),
}

impl ImeSessionAnchor {
    /// Creates an anchor from a screen-space rectangle.
    #[must_use]
    pub const fn screen_rect(rect: Rect) -> Self { Self::ScreenRect(rect) }

    /// Creates an anchor from a screen-space point.
    #[must_use]
    pub const fn screen_point(point: Vec2) -> Self { Self::ScreenPoint(point) }
}
