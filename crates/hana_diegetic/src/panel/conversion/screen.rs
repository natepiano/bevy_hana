//! Screen conversion target builder and recipes.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Vec2;
use bevy::window::WindowRef;

use super::PanelProjectionError;
use super::projection::PanelScreenProjection;
use crate::layout;
use crate::layout::Anchor;
use crate::layout::Dimension;
use crate::layout::LayoutTree;
use crate::layout::Px;
use crate::layout::Sizing;
use crate::layout::Unit;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::ScreenPosition;
use crate::panel::builder::Screen;
use crate::panel::constants::DEFAULT_SCREEN_SPACE_CAMERA_ORDER;
use crate::panel::constants::DEFAULT_SCREEN_SPACE_RENDER_LAYER;
use crate::panel::sizing::PanelSizing;
use crate::screen_space;

/// Screen-space landing target for converting an existing panel.
///
/// The builder mirrors the screen-panel placement vocabulary without requiring a
/// replacement layout tree. Unset fields inherit the panel's current projection:
/// projected anchor position, projected size, current anchor, and the camera's
/// target window.
#[derive(Clone, Debug, Default)]
pub struct PanelScreenTarget {
    position: Option<ScreenPosition>,
    width:    Option<Sizing>,
    height:   Option<Sizing>,
    anchor:   Option<Anchor>,
}

impl PanelScreenTarget {
    /// Sets the screen-space target dimensions.
    #[must_use]
    pub fn size<W, H>(mut self, width: W, height: H) -> Self
    where
        W: PanelSizing<Screen>,
        H: PanelSizing<Screen>,
    {
        self.width = Some(width.to_sizing());
        self.height = Some(height.to_sizing());
        self
    }

    /// Places the panel at an explicit pixel position.
    ///
    /// The panel's target [`Anchor`] determines which point of the panel lands at
    /// this position.
    #[must_use]
    pub const fn screen_position(mut self, x: f32, y: f32) -> Self {
        self.position = Some(ScreenPosition::At(Vec2::new(x, y)));
        self
    }

    /// Pins the panel to the window edge or corner matching its target anchor.
    ///
    /// With [`Anchor::Center`], this places the converted panel at the center of
    /// the target window.
    #[must_use]
    pub const fn screen(mut self) -> Self {
        self.position = Some(ScreenPosition::Screen);
        self
    }

    /// Sets the converted panel's anchor.
    #[must_use]
    pub const fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = Some(anchor);
        self
    }

    pub(super) fn resolve(
        self,
        panel: &DiegeticPanel,
        computed: &ComputedDiegeticPanel,
        projection: PanelScreenProjection,
        window_size: Vec2,
    ) -> Result<PanelScreenConversion, PanelProjectionError> {
        let anchor = self.anchor.unwrap_or_else(|| panel.anchor());
        let position = self
            .position
            .unwrap_or(ScreenPosition::At(projection.anchor_position));
        let anchor_position = match position {
            ScreenPosition::Screen => {
                let (fx, fy) = anchor.offset_fraction();
                Vec2::new(fx * window_size.x, fy * window_size.y)
            },
            ScreenPosition::At(position) => position,
        };
        let width = self
            .width
            .unwrap_or_else(|| Sizing::fixed(Px(projection.size.x)));
        let height = self
            .height
            .unwrap_or_else(|| Sizing::fixed(Px(projection.size.y)));
        let content = projected_content_size(panel, computed, projection);
        let size = Vec2::new(
            screen_space::resolve_screen_axis(width, window_size.x, content.x, projection.size.x),
            screen_space::resolve_screen_axis(height, window_size.y, content.y, projection.size.y),
        );
        let conversion = PanelScreenConversion::at_pixels(anchor_position, size)
            .anchor(anchor)
            .sizing(width, height)
            .window(WindowRef::Entity(projection.window));
        validate_screen_conversion(&conversion)?;
        Ok(conversion)
    }
}

/// Screen-space conversion recipe for a [`DiegeticPanel`].
#[derive(Clone, Debug)]
pub struct PanelScreenConversion {
    /// Position for the converted panel's configured anchor, in logical pixels.
    pub anchor_position: Vec2,
    /// Converted panel size in logical pixels.
    pub size:            Vec2,
    /// Optional anchor to set before placing the screen panel.
    pub anchor:          Option<Anchor>,
    /// Screen-space width rule.
    pub width:           Sizing,
    /// Screen-space height rule.
    pub height:          Sizing,
    /// Authored screen-space rotation in radians.
    pub rotation:        f32,
    /// Screen-space overlay camera render order.
    pub camera_order:    isize,
    /// Render layers for screen-space camera isolation.
    pub render_layers:   RenderLayers,
    /// Window the converted panel renders into.
    pub window:          WindowRef,
}

impl PanelScreenConversion {
    /// Creates a resolved screen conversion from logical-pixel placement.
    #[must_use]
    pub const fn at_pixels(anchor_position: Vec2, size: Vec2) -> Self {
        Self {
            anchor_position,
            size,
            anchor: None,
            width: Sizing::Fixed(Dimension {
                value: size.x,
                unit:  Some(Unit::Pixels),
            }),
            height: Sizing::Fixed(Dimension {
                value: size.y,
                unit:  Some(Unit::Pixels),
            }),
            rotation: 0.0,
            camera_order: DEFAULT_SCREEN_SPACE_CAMERA_ORDER,
            render_layers: RenderLayers::layer(DEFAULT_SCREEN_SPACE_RENDER_LAYER),
            window: WindowRef::Primary,
        }
    }

    /// Sets the converted panel's anchor.
    #[must_use]
    pub const fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = Some(anchor);
        self
    }

    /// Sets the screen-space sizing rules while keeping the already-resolved
    /// conversion size for the initial layout scale.
    #[must_use]
    pub const fn sizing(mut self, width: Sizing, height: Sizing) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Sets the authored in-plane screen rotation.
    #[must_use]
    pub const fn rotation(mut self, rotation: f32) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets the overlay camera render order.
    #[must_use]
    pub const fn camera_order(mut self, order: isize) -> Self {
        self.camera_order = order;
        self
    }

    /// Sets the render layers for camera isolation.
    #[must_use]
    pub fn render_layers(mut self, layers: RenderLayers) -> Self {
        self.render_layers = layers;
        self
    }

    /// Pins the converted panel to a specific window.
    #[must_use]
    pub const fn window(mut self, window: WindowRef) -> Self {
        self.window = window;
        self
    }
}

impl From<PanelScreenProjection> for PanelScreenConversion {
    fn from(projection: PanelScreenProjection) -> Self {
        Self::at_pixels(projection.anchor_position, projection.size)
            .rotation(projection.rotation)
            .window(WindowRef::Entity(projection.window))
    }
}

pub(crate) fn apply_screen_conversion(
    panel: &mut DiegeticPanel,
    conversion: PanelScreenConversion,
) -> Result<(), PanelProjectionError> {
    validate_screen_conversion(&conversion)?;
    if let Some(anchor) = conversion.anchor {
        panel.anchor = anchor;
    }
    panel.width = conversion.size.x;
    panel.height = conversion.size.y;
    panel.layout_unit = Unit::Pixels;
    panel.world_width = None;
    panel.world_height = None;
    panel.coordinate_space = CoordinateSpace::Screen {
        position:      ScreenPosition::At(conversion.anchor_position),
        width:         conversion.width,
        height:        conversion.height,
        camera_order:  conversion.camera_order,
        render_layers: conversion.render_layers,
        window:        conversion.window,
    };
    Ok(())
}

pub(crate) fn validate_screen_conversion(
    conversion: &PanelScreenConversion,
) -> Result<(), PanelProjectionError> {
    if !conversion.anchor_position.is_finite()
        || !conversion.size.is_finite()
        || !conversion.rotation.is_finite()
        || conversion.size.x <= 0.0
        || conversion.size.y <= 0.0
    {
        return Err(PanelProjectionError::InvalidProjection);
    }
    Ok(())
}

pub(crate) fn apply_screen_root_sizing(tree: &mut LayoutTree, width: Sizing, height: Sizing) {
    match width {
        Sizing::Fit { min, max } => layout::set_root_fit_width(tree, min, max),
        Sizing::Grow { min, max } => layout::set_root_grow_width(tree, min, max),
        Sizing::Percent(_) => layout::set_root_grow_width(
            tree,
            Dimension {
                value: 0.0,
                unit:  None,
            },
            Dimension {
                value: f32::MAX,
                unit:  None,
            },
        ),
        Sizing::Fixed(_) => {},
    }
    match height {
        Sizing::Fit { min, max } => layout::set_root_fit_height(tree, min, max),
        Sizing::Grow { min, max } => layout::set_root_grow_height(tree, min, max),
        Sizing::Percent(_) => layout::set_root_grow_height(
            tree,
            Dimension {
                value: 0.0,
                unit:  None,
            },
            Dimension {
                value: f32::MAX,
                unit:  None,
            },
        ),
        Sizing::Fixed(_) => {},
    }
}

fn projected_content_size(
    panel: &DiegeticPanel,
    computed: &ComputedDiegeticPanel,
    projection: PanelScreenProjection,
) -> Vec2 {
    if matches!(panel.coordinate_space(), CoordinateSpace::Screen { .. }) {
        return Vec2::new(computed.content_width(), computed.content_height());
    }
    let world_height = panel.world_height();
    if !world_height.is_finite() || world_height <= 0.0 {
        return Vec2::ZERO;
    }
    let world_to_pixels = projection.size.y / world_height;
    Vec2::new(
        computed.content_width() * world_to_pixels,
        computed.content_height() * world_to_pixels,
    )
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact expected layout values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use super::PanelScreenConversion;
    use crate::layout::Sizing;
    use crate::layout::Unit;

    #[test]
    fn at_pixels_uses_fixed_pixel_sizing() {
        let conversion = PanelScreenConversion::at_pixels(
            bevy::math::Vec2::new(10.0, 20.0),
            bevy::math::Vec2::new(30.0, 40.0),
        );

        let Sizing::Fixed(width) = conversion.width else {
            panic!("width should be fixed");
        };
        let Sizing::Fixed(height) = conversion.height else {
            panic!("height should be fixed");
        };

        assert_eq!(width.value, conversion.size.x);
        assert_eq!(width.unit, Some(Unit::Pixels));
        assert_eq!(height.value, conversion.size.y);
        assert_eq!(height.unit, Some(Unit::Pixels));
    }
}
