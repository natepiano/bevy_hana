//! World conversion target builder and recipes.

use bevy::math::primitives::InfinitePlane3d;
use bevy::prelude::Camera;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;

use super::PanelProjectionError;
use super::PanelScreenConversion;
use super::projection::PanelScreenProjection;
use super::projection::PanelWorldProjection;
use crate::layout::Anchor;
use crate::layout::Dimension;
use crate::layout::Sizing;
use crate::layout::Unit;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::PanelSizing;
use crate::panel::builder::World;
use crate::panel::sizing::CompatibleUnits;

/// World-space target for converting an existing panel.
#[derive(Clone, Debug, Default)]
pub struct PanelWorldTarget {
    transform:    Option<Transform>,
    width:        Option<Sizing>,
    height:       Option<Sizing>,
    layout_unit:  Option<Unit>,
    world_width:  Option<f32>,
    world_height: Option<f32>,
    anchor:       Option<Anchor>,
}

impl PanelWorldTarget {
    /// Sets the target panel dimensions using world-panel sizing rules.
    #[must_use]
    pub fn size<W, H>(mut self, width: W, height: H) -> Self
    where
        W: PanelSizing<World>,
        H: PanelSizing<World>,
        (W::Unit, H::Unit): CompatibleUnits,
    {
        let width = width.to_sizing();
        let height = height.to_sizing();
        self.layout_unit = sizing_unit(width)
            .or_else(|| sizing_unit(height))
            .or(self.layout_unit);
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    /// Sets the target panel transform.
    #[must_use]
    pub const fn transform(mut self, transform: Transform) -> Self {
        self.transform = Some(transform);
        self
    }

    /// Sets the target panel anchor.
    #[must_use]
    pub const fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = Some(anchor);
        self
    }

    /// Sets the target world width in meters.
    #[must_use]
    pub const fn world_width(mut self, meters: f32) -> Self {
        self.world_width = Some(meters);
        self
    }

    /// Sets the target world height in meters.
    #[must_use]
    pub const fn world_height(mut self, meters: f32) -> Self {
        self.world_height = Some(meters);
        self
    }

    #[cfg(test)]
    pub(super) const fn transform_value(&self) -> Option<Transform> { self.transform }

    #[cfg(test)]
    pub(super) const fn anchor_value(&self) -> Option<Anchor> { self.anchor }

    #[cfg(test)]
    pub(super) const fn world_height_value(&self) -> Option<f32> { self.world_height }

    pub(super) fn resolve(
        self,
        panel: &DiegeticPanel,
        projection: PanelScreenProjection,
        camera: &Camera,
        camera_transform: &GlobalTransform,
    ) -> Result<PanelWorldConversion, PanelProjectionError> {
        self.resolve_screen_placement(
            panel,
            projection.size,
            projection.corners,
            camera,
            camera_transform,
        )
    }

    pub(super) fn resolve_screen_conversion(
        self,
        panel: &DiegeticPanel,
        conversion: PanelScreenConversion,
        camera: &Camera,
        camera_transform: &GlobalTransform,
    ) -> Result<PanelWorldConversion, PanelProjectionError> {
        let anchor = conversion.anchor.unwrap_or_else(|| panel.anchor());
        self.resolve_screen_placement(
            panel,
            conversion.size,
            screen_corners(
                conversion.anchor_position,
                conversion.size,
                anchor,
                conversion.rotation,
            ),
            camera,
            camera_transform,
        )
    }

    fn resolve_screen_placement(
        self,
        panel: &DiegeticPanel,
        screen_size: Vec2,
        screen_corners: [Vec2; 4],
        camera: &Camera,
        camera_transform: &GlobalTransform,
    ) -> Result<PanelWorldConversion, PanelProjectionError> {
        let target_transform = self
            .transform
            .ok_or(PanelProjectionError::InvalidWorldTarget)?;
        let anchor = self.anchor.unwrap_or_else(|| panel.anchor());
        let anchor_position = screen_position_for_anchor(screen_corners, anchor);
        let normal = target_transform.rotation * Vec3::Z;
        let anchor_world = viewport_to_plane(
            camera,
            camera_transform,
            anchor_position,
            target_transform.translation,
            normal,
        )?;
        let projected_size = projected_world_size(
            camera,
            camera_transform,
            screen_corners,
            target_transform.translation,
            normal,
        )
        .unwrap_or(screen_size);
        let layout_unit = self.layout_unit.unwrap_or_else(|| panel.layout_unit());
        let panel_size = Vec2::new(
            self.width.map_or_else(|| panel.width(), initial_panel_size),
            self.height
                .map_or_else(|| panel.height(), initial_panel_size),
        );
        let width = self
            .width
            .unwrap_or_else(|| fixed_sizing(panel_size.x, layout_unit));
        let height = self
            .height
            .unwrap_or_else(|| fixed_sizing(panel_size.y, layout_unit));
        let world_width = self.world_width.or(Some(projected_size.x));
        let world_height = self.world_height.or(Some(projected_size.y));
        let conversion = PanelWorldConversion {
            transform: Transform {
                translation: anchor_world,
                rotation:    target_transform.rotation,
                scale:       Vec3::ONE,
            },
            size: projected_size,
            panel_size,
            layout_unit,
            anchor: Some(anchor),
            width,
            height,
            world_width,
            world_height,
            restore_saved_world: SavedWorldRestoreMode::Skip,
        };
        validate_world_conversion(&conversion)?;
        Ok(conversion)
    }
}

/// World-space conversion recipe for a [`DiegeticPanel`].
#[derive(Clone, Debug)]
pub struct PanelWorldConversion {
    /// Transform to apply to the panel entity.
    pub transform:           Transform,
    /// Resolved world size in meters.
    pub size:                Vec2,
    /// Panel viewport size in `layout_unit`.
    pub panel_size:          Vec2,
    /// Panel layout unit after conversion.
    pub layout_unit:         Unit,
    /// Optional anchor to set before placing the world panel.
    pub anchor:              Option<Anchor>,
    /// World-space width rule.
    pub width:               Sizing,
    /// World-space height rule.
    pub height:              Sizing,
    /// Target world width in meters.
    pub world_width:         Option<f32>,
    /// Target world height in meters.
    pub world_height:        Option<f32>,
    /// Whether applying this conversion restores saved world-authored panel data.
    pub restore_saved_world: SavedWorldRestoreMode,
}

/// Controls whether a world conversion restores saved panel data.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SavedWorldRestoreMode {
    /// Use the current projection directly and skip saved-world restoration.
    #[default]
    Skip,
    /// Restore saved world settings after projection.
    Restore,
}

impl SavedWorldRestoreMode {}

impl From<PanelWorldProjection> for PanelWorldConversion {
    fn from(projection: PanelWorldProjection) -> Self {
        Self {
            transform:           projection.transform,
            size:                projection.size,
            panel_size:          projection.panel_size,
            layout_unit:         projection.layout_unit,
            anchor:              Some(projection.anchor),
            width:               projection.width,
            height:              projection.height,
            world_width:         projection.world_width,
            world_height:        projection.world_height,
            restore_saved_world: projection.restore_saved_world,
        }
    }
}

pub(crate) fn apply_world_conversion(
    panel: &mut DiegeticPanel,
    conversion: PanelWorldConversion,
) -> Result<(), PanelProjectionError> {
    validate_world_conversion(&conversion)?;
    if let Some(anchor) = conversion.anchor {
        panel.anchor = anchor;
    }
    panel.width = conversion.panel_size.x;
    panel.height = conversion.panel_size.y;
    panel.layout_unit = conversion.layout_unit;
    panel.world_width = conversion.world_width;
    panel.world_height = conversion.world_height;
    panel.coordinate_space = CoordinateSpace::World {
        width:  conversion.width,
        height: conversion.height,
    };
    Ok(())
}

pub(crate) fn validate_world_conversion(
    conversion: &PanelWorldConversion,
) -> Result<(), PanelProjectionError> {
    if !conversion.transform.translation.is_finite()
        || !conversion.transform.rotation.is_finite()
        || !conversion.transform.scale.is_finite()
        || !conversion.size.is_finite()
        || !conversion.panel_size.is_finite()
        || conversion.size.x <= 0.0
        || conversion.size.y <= 0.0
        || conversion.panel_size.x <= 0.0
        || conversion.panel_size.y <= 0.0
    {
        return Err(PanelProjectionError::InvalidProjection);
    }
    Ok(())
}

fn projected_world_size(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    corners: [Vec2; 4],
    plane_origin: Vec3,
    normal: Vec3,
) -> Option<Vec2> {
    let top_left =
        viewport_to_plane(camera, camera_transform, corners[0], plane_origin, normal).ok()?;
    let top_right =
        viewport_to_plane(camera, camera_transform, corners[1], plane_origin, normal).ok()?;
    let bottom_right =
        viewport_to_plane(camera, camera_transform, corners[2], plane_origin, normal).ok()?;
    let bottom_left =
        viewport_to_plane(camera, camera_transform, corners[3], plane_origin, normal).ok()?;
    let width = average_positive(
        top_left.distance(top_right),
        bottom_left.distance(bottom_right),
    )
    .ok()?;
    let height = average_positive(
        top_left.distance(bottom_left),
        top_right.distance(bottom_right),
    )
    .ok()?;
    Some(Vec2::new(width, height))
}

fn viewport_to_plane(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    viewport_position: Vec2,
    plane_origin: Vec3,
    normal: Vec3,
) -> Result<Vec3, PanelProjectionError> {
    let ray = camera
        .viewport_to_world(camera_transform, viewport_position)
        .map_err(|_| PanelProjectionError::ProjectionFailed)?;
    ray.plane_intersection_point(plane_origin, InfinitePlane3d::new(normal))
        .ok_or(PanelProjectionError::ProjectionFailed)
}

fn average_positive(a: f32, b: f32) -> Result<f32, PanelProjectionError> {
    if !a.is_finite() || !b.is_finite() || a <= 0.0 || b <= 0.0 {
        return Err(PanelProjectionError::InvalidProjection);
    }
    Ok(f32::midpoint(a, b))
}

const fn initial_panel_size(sizing: Sizing) -> f32 {
    match sizing {
        Sizing::Fixed(size) => size.value,
        Sizing::Fit { min, .. } | Sizing::Grow { min, .. } => min.value,
        Sizing::Percent(_) => 0.0,
    }
}

const fn sizing_unit(sizing: Sizing) -> Option<Unit> {
    match sizing {
        Sizing::Fixed(size) => size.unit,
        Sizing::Fit { min, max } | Sizing::Grow { min, max } => match (min.unit, max.unit) {
            (Some(unit), _) | (None, Some(unit)) => Some(unit),
            (None, None) => None,
        },
        Sizing::Percent(_) => None,
    }
}

const fn fixed_sizing(value: f32, unit: Unit) -> Sizing {
    Sizing::Fixed(Dimension {
        value,
        unit: Some(unit),
    })
}

fn screen_corners(anchor_position: Vec2, size: Vec2, anchor: Anchor, rotation: f32) -> [Vec2; 4] {
    let (fx, fy) = anchor.offset_fraction();
    let top_left = anchor_position - Vec2::new(size.x * fx, size.y * fy);
    let top_right = top_left + Vec2::new(size.x, 0.0);
    let bottom_right = top_left + size;
    let bottom_left = top_left + Vec2::new(0.0, size.y);
    [
        rotate_around_anchor(top_left, anchor_position, rotation),
        rotate_around_anchor(top_right, anchor_position, rotation),
        rotate_around_anchor(bottom_right, anchor_position, rotation),
        rotate_around_anchor(bottom_left, anchor_position, rotation),
    ]
}

fn screen_position_for_anchor(corners: [Vec2; 4], anchor: Anchor) -> Vec2 {
    let (fx, fy) = anchor.offset_fraction();
    let top = corners[0].lerp(corners[1], fx);
    let bottom = corners[3].lerp(corners[2], fx);
    top.lerp(bottom, fy)
}

fn rotate_around_anchor(point: Vec2, anchor_position: Vec2, radians: f32) -> Vec2 {
    let local = point - anchor_position;
    let (sin, cos) = radians.sin_cos();
    anchor_position
        + Vec2::new(
            local.y.mul_add(-sin, local.x * cos),
            local.y.mul_add(cos, local.x * sin),
        )
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;

    use super::PanelWorldTarget;
    use super::screen_position_for_anchor;
    use crate::Anchor;

    #[test]
    fn target_records_transform_anchor_and_world_height() {
        let transform = bevy::prelude::Transform::from_xyz(1.0, 2.0, 3.0);
        let target = PanelWorldTarget::default()
            .transform(transform)
            .anchor(Anchor::Center)
            .world_height(0.5);

        assert_eq!(target.transform_value(), Some(transform));
        assert_eq!(target.anchor_value(), Some(Anchor::Center));
        assert_eq!(target.world_height_value(), Some(0.5));
    }

    #[test]
    fn target_anchor_position_comes_from_projected_corners() {
        let corners = [
            Vec2::new(100.0, 40.0),
            Vec2::new(340.0, 60.0),
            Vec2::new(320.0, 180.0),
            Vec2::new(80.0, 160.0),
        ];

        assert_eq!(
            screen_position_for_anchor(corners, Anchor::TopRight),
            corners[1]
        );
        assert_eq!(
            screen_position_for_anchor(corners, Anchor::BottomLeft),
            corners[3]
        );
        assert_eq!(
            screen_position_for_anchor(corners, Anchor::Center),
            Vec2::new(210.0, 110.0)
        );
    }
}
