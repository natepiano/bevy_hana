//! Panel coordinate space and companion effects.

#![allow(
    clippy::used_underscore_binding,
    reason = "false positive on enum variant fields"
)]

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Insert;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::Vec2;
use bevy::window::WindowRef;

use super::DiegeticPanel;
use crate::layout;
use crate::layout::Dimension;
use crate::layout::LayoutTree;
use crate::layout::ShadowCasting;
use crate::layout::Sizing;

/// Where a screen-space panel is placed within the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub enum ScreenPosition {
    /// Pin to the window edge/corner that matches the panel's
    /// [`Anchor`](crate::Anchor). `Anchor::TopLeft` pins to the window's
    /// top-left corner, `Anchor::Center` pins to the window's center, etc.
    #[default]
    Screen,
    /// Place at an explicit pixel position (top-left origin, y-down).
    /// The panel's [`Anchor`](crate::Anchor) determines which point of the
    /// panel sits at this position.
    At(Vec2),
}

/// Compatibility adapter for the old panel-surface shadow API.
///
/// New code should author [`ShadowCasting`] so fills, borders, panel shapes,
/// text, and images share one cascade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum SurfaceShadow {
    /// The panel surface does not cast shadows.
    #[default]
    Off,
    /// The panel surface participates in shadow casting.
    On,
}

impl From<SurfaceShadow> for ShadowCasting {
    fn from(value: SurfaceShadow) -> Self {
        match value {
            SurfaceShadow::Off => Self::Off,
            SurfaceShadow::On => Self::On,
        }
    }
}

impl From<ShadowCasting> for SurfaceShadow {
    fn from(value: ShadowCasting) -> Self {
        match value {
            ShadowCasting::Off => Self::Off,
            ShadowCasting::On => Self::On,
        }
    }
}

/// Whether the panel is placed in 3D world space or rendered as a 2D screen
/// overlay.
///
/// `World` panels are positioned and scaled in 3D space.
/// `Screen` panels render via an orthographic overlay camera.
#[derive(Clone, Debug, Reflect)]
pub enum CoordinateSpace {
    /// Panel is placed in 3D world space.
    World {
        /// Panel width, expressed with the layout engine's [`Sizing`] enum.
        /// `Fixed` is a physical value in the panel's layout unit;
        /// `Fit { min, max }` shrink-wraps content (bounded by `max`).
        /// `Grow` / `Percent` are screen-only and rejected by the world
        /// builder at compile time.
        width:  Sizing,
        /// Panel height, same semantics as `width`.
        height: Sizing,
    },
    /// Panel renders as a 2D screen overlay.
    Screen {
        /// Where to place the panel within the window.
        position:      ScreenPosition,
        /// Panel width, expressed with the layout engine's [`Sizing`] enum.
        /// `Fixed` is a pixel value; `Percent(f)` is a fraction of the
        /// window; `Fit { min, max }` grows to content (bounded by `max` if
        /// set); `Grow { min, max }` fills the window clamped to `[min, max]`.
        width:         Sizing,
        /// Panel height, same semantics as `width`.
        height:        Sizing,
        /// Camera render order. Higher orders render on top. Default: `1`.
        camera_order:  isize,
        /// Identifies panels that share an overlay view when their camera
        /// order and target window also match. The overlay view receives a
        /// private render layer so other windows cannot draw these panels.
        /// Default: `RenderLayers::layer(31)`.
        render_layers: RenderLayers,
        /// Window this panel renders into. Defaults to `WindowRef::Primary`.
        /// Use `WindowRef::Entity(..)` to pin a panel to a specific window.
        window:        WindowRef,
    },
}

impl Default for CoordinateSpace {
    fn default() -> Self {
        Self::World {
            width:  Sizing::Fixed(Dimension {
                value: 0.0,
                unit:  None,
            }),
            height: Sizing::Fixed(Dimension {
                value: 0.0,
                unit:  None,
            }),
        }
    }
}

impl CoordinateSpace {
    /// Returns `true` if this is a screen-space panel.
    #[must_use]
    pub const fn is_screen(&self) -> bool { matches!(self, Self::Screen { .. }) }
}

/// Queryable, reflectable mirror of a panel's [`CoordinateSpace`] discriminant.
///
/// [`DiegeticPanel`] keeps [`CoordinateSpace`] as an internal field — the
/// authoritative source for layout and conversion math, carrying the sizing and
/// screen configuration. That field reflects but is not independently
/// queryable, and mutating it in place through `&mut DiegeticPanel` fires no
/// component hook, so a world<->screen conversion cannot be observed.
///
/// `PanelSpace` duplicates only the `World`/`Screen` discriminant as a
/// standalone component. It lowers the query requirement (`Query<&PanelSpace>`
/// instead of pulling the whole panel) and gives cheap reflect/BRP inspection
/// of a panel's space. Attachment and conversion mutations validate
/// `DiegeticPanel.coordinate_space` directly rather than using this mirror as
/// an authoring boundary.
///
/// The cost is one duplicated discriminant kept in sync when a whole panel
/// component is inserted or replaced (`sync_panel_space_on_insert`) and at the
/// coordinate-space conversion apply points. `PanelSpace` never carries sizing or screen config;
/// the field stays the single source for geometry. Removing the field entirely
/// (true single source) would thread the space through the panel's geometry and
/// conversion hot paths and is deliberately out of scope.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Component, Default, PartialEq, Debug)]
pub enum PanelSpace {
    /// Panel is placed in 3D world space.
    #[default]
    World,
    /// Panel renders as a 2D screen overlay.
    Screen,
}

impl From<&CoordinateSpace> for PanelSpace {
    fn from(space: &CoordinateSpace) -> Self {
        if space.is_screen() {
            Self::Screen
        } else {
            Self::World
        }
    }
}

pub(crate) fn constrain_fit_width(
    panel: &mut DiegeticPanel,
    blueprint: &LayoutTree,
    max_width: f32,
) {
    let unit = panel.layout_unit();
    let min = Dimension {
        value: 0.0,
        unit:  Some(unit),
    };
    let max = Dimension {
        value: max_width,
        unit:  Some(unit),
    };
    match &mut panel.coordinate_space {
        CoordinateSpace::World { width, .. } | CoordinateSpace::Screen { width, .. } => {
            *width = Sizing::Fit { min, max };
        },
    }
    let mut tree = blueprint.clone();
    layout::set_root_fit_width(&mut tree, min, max);
    panel.replace_tree_full_rebuild(tree);
}

/// Synchronizes the [`PanelSpace`] mirror after a panel insert or replacement.
///
/// In-place conversions re-insert `PanelSpace` at their apply points; this
/// observer covers initial insertion and whole-component replacement.
pub(super) fn sync_panel_space_on_insert(
    inserted: On<Insert, DiegeticPanel>,
    panels: Query<&DiegeticPanel>,
    mut commands: Commands,
) {
    let entity = inserted.entity;
    let Ok(panel) = panels.get(entity) else {
        return;
    };
    commands
        .entity(entity)
        .insert(PanelSpace::from(panel.coordinate_space()));
}
