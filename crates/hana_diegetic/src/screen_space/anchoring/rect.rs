//! Per-frame screen-space panel rectangles used during attachment resolution.

use bevy::camera::visibility::RenderLayers;
use bevy::platform::collections::HashMap;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::With;
use bevy::window::PrimaryWindow;

use super::projection;
use super::screen_in_plane_angle;
use super::window;
use crate::layout::Anchor;
use crate::layout::Unit;
use crate::panel;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::PanelScreenBounds;
use crate::panel::ResolvedScreenPanelPosition;
use crate::widgets::WidgetAnchorRect;

/// Placement and presentation data for a general screen-space anchor target.
#[derive(Clone, Component, Debug)]
pub struct ScreenAnchorTarget {
    bounds:        PanelScreenBounds,
    window:        Entity,
    camera_order:  isize,
    render_layers: RenderLayers,
    layout_unit:   Unit,
}

impl ScreenAnchorTarget {
    /// Creates general screen-space anchor data.
    #[must_use]
    pub const fn new(
        bounds: PanelScreenBounds,
        window: Entity,
        camera_order: isize,
        render_layers: RenderLayers,
        layout_unit: Unit,
    ) -> Self {
        Self {
            bounds,
            window,
            camera_order,
            render_layers,
            layout_unit,
        }
    }

    /// Current target rectangle in logical pixels.
    #[must_use]
    pub const fn bounds(&self) -> PanelScreenBounds { self.bounds }

    /// Window containing the target rectangle.
    #[must_use]
    pub const fn window(&self) -> Entity { self.window }

    /// Camera order used to present attachments.
    #[must_use]
    pub const fn camera_order(&self) -> isize { self.camera_order }

    /// Render layers used to present attachments.
    #[must_use]
    pub const fn render_layers(&self) -> &RenderLayers { &self.render_layers }

    /// Pixel-unit context for authored offsets and attached panels.
    #[must_use]
    pub const fn layout_unit(&self) -> Unit { self.layout_unit }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ScreenPanelRect {
    pub(super) anchor_position: Vec2,
    pub(super) anchor:          Anchor,
    size:                       Vec2,
    angle:                      f32,
    bounds:                     Option<PanelScreenBounds>,
    layout_scale:               Vec2,
    layout_unit:                Unit,
}

impl ScreenPanelRect {
    fn from_panel(panel: &DiegeticPanel, window_size: Vec2, angle: f32) -> Option<Self> {
        let anchor_position = panel::screen_anchor_position(panel, window_size, None).ok()?;
        let size = Vec2::new(panel.width(), panel.height());
        let bounds =
            PanelScreenBounds::from_anchor_position(anchor_position, panel.anchor(), size).ok()?;
        Some(Self {
            anchor_position,
            anchor: panel.anchor(),
            size,
            angle,
            bounds: Some(bounds),
            layout_scale: Vec2::ONE,
            layout_unit: panel.layout_unit(),
        })
    }

    pub(super) const fn bounds(self) -> Option<PanelScreenBounds> { self.bounds }

    pub(crate) const fn layout_unit(self) -> Unit { self.layout_unit }

    pub(crate) const fn layout_scale(self) -> Vec2 { self.layout_scale }

    pub(crate) const fn angle(self) -> f32 { self.angle }

    pub(crate) fn from_widget(
        owner: Self,
        widget: WidgetAnchorRect,
        owner_transform: &Transform,
    ) -> Option<Self> {
        let scale = projection::screen_panel_scale(owner_transform)?;
        let panel_offset = widget.panel_offset().truncate();
        let anchor_position = owner.anchor_position
            + projection::project_panel_local_offset(panel_offset, scale, owner.angle);
        let size = widget.size() * scale.abs();
        let bounds =
            PanelScreenBounds::from_anchor_position(anchor_position, Anchor::Center, size).ok()?;
        Some(Self {
            anchor_position,
            anchor: Anchor::Center,
            size,
            angle: owner.angle,
            bounds: Some(bounds),
            layout_scale: scale,
            layout_unit: owner.layout_unit,
        })
    }

    pub(crate) fn from_screen_target(target: &ScreenAnchorTarget) -> Self {
        Self {
            anchor_position: target.bounds().point(Anchor::Center),
            anchor:          Anchor::Center,
            size:            target.bounds().size(),
            angle:           0.0,
            bounds:          Some(target.bounds()),
            layout_scale:    Vec2::ONE,
            layout_unit:     target.layout_unit(),
        }
    }

    pub(crate) fn oriented_anchor_point(self, anchor: Anchor) -> Option<Vec2> {
        let bounds = self.bounds?;
        let resolved_anchor_offset = bounds.anchor_offset(anchor);
        let panel_offset = bounds.anchor_offset(self.anchor);
        let authored_anchor_offset =
            (resolved_anchor_offset - panel_offset) * self.layout_scale.signum();
        Some(
            self.anchor_position
                + projection::rotate_screen_offset(authored_anchor_offset, self.angle),
        )
    }

    pub(crate) fn projected_bounds(self) -> Option<PanelScreenBounds> {
        let mut minimum = Vec2::splat(f32::INFINITY);
        let mut maximum = Vec2::splat(f32::NEG_INFINITY);
        for anchor in [
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomRight,
            Anchor::BottomLeft,
        ] {
            let point = self.oriented_anchor_point(anchor)?;
            minimum = minimum.min(point);
            maximum = maximum.max(point);
        }
        PanelScreenBounds::new(minimum, maximum - minimum).ok()
    }

    pub(crate) fn placed_bounds(
        self,
        source_anchor: Anchor,
        source_anchor_position: Vec2,
    ) -> Option<PanelScreenBounds> {
        let bounds = self.bounds?;
        let source_offset = bounds.anchor_offset(source_anchor);
        let mut minimum = Vec2::splat(f32::INFINITY);
        let mut maximum = Vec2::splat(f32::NEG_INFINITY);
        for anchor in [
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomRight,
            Anchor::BottomLeft,
        ] {
            let corner_offset = bounds.anchor_offset(anchor) - source_offset;
            let point = source_anchor_position
                + projection::rotate_screen_offset(corner_offset, self.angle);
            minimum = minimum.min(point);
            maximum = maximum.max(point);
        }
        PanelScreenBounds::new(minimum, maximum - minimum).ok()
    }

    pub(super) fn with_anchor_position_and_angle(
        self,
        anchor_position: Vec2,
        angle: Option<f32>,
    ) -> Self {
        let bounds =
            PanelScreenBounds::from_anchor_position(anchor_position, self.anchor, self.size).ok();
        Self {
            anchor_position,
            angle: angle.unwrap_or(self.angle),
            bounds,
            ..self
        }
    }
}

pub(crate) fn screen_panel_rect(
    panel: &DiegeticPanel,
    resolved_position: Option<&ResolvedScreenPanelPosition>,
    transform: Option<&Transform>,
    window_size: Vec2,
) -> Option<ScreenPanelRect> {
    let resolved_rotation =
        resolved_position.and_then(|resolved_position| resolved_position.rotation);
    let angle = resolved_rotation.unwrap_or_else(|| {
        transform.map_or(0.0, |transform| screen_in_plane_angle(transform.rotation))
    });
    let rect = ScreenPanelRect::from_panel(panel, window_size, angle)?;
    Some(
        resolved_position
            .and_then(|resolved_position| resolved_position.anchor_position)
            .map_or(rect, |anchor_position| {
                rect.with_anchor_position_and_angle(anchor_position, None)
            }),
    )
}

pub(super) fn screen_panel_rects(
    panels: &Query<(Entity, &DiegeticPanel), With<ResolvedScreenPanelPosition>>,
    screen_targets: &Query<(Entity, &ScreenAnchorTarget)>,
    resolved_positions: &Query<&mut ResolvedScreenPanelPosition>,
    transforms: &Query<&Transform>,
    primary: &Query<Entity, With<PrimaryWindow>>,
    window_sizes: &HashMap<Entity, Vec2>,
) -> HashMap<Entity, ScreenPanelRect> {
    let mut rects = HashMap::default();
    for (entity, panel) in panels {
        let CoordinateSpace::Screen { window, .. } = panel.coordinate_space() else {
            continue;
        };
        let Ok((_, window_size)) = window::resolve_window(*window, primary, window_sizes) else {
            continue;
        };
        let authored_rotation = resolved_positions
            .get(entity)
            .ok()
            .and_then(|resolved_position| resolved_position.authored_rotation);
        let angle = authored_rotation.unwrap_or_else(|| {
            transforms
                .get(entity)
                .map_or(0.0, |transform| screen_in_plane_angle(transform.rotation))
        });
        if let Some(rect) = ScreenPanelRect::from_panel(panel, window_size, angle) {
            rects.insert(entity, rect);
        }
    }
    for (entity, target) in screen_targets {
        rects.insert(entity, ScreenPanelRect::from_screen_target(target));
    }
    rects
}
