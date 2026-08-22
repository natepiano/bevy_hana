//! Read-only panel projection helpers.

use bevy::camera::RenderTarget;
use bevy::ecs::system::SystemParam;
use bevy::prelude::Camera;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Query;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use bevy::transform::helper::TransformHelper;
use bevy::window::PrimaryWindow;
use bevy::window::Window;
use bevy::window::WindowRef;

use super::PanelProjectionError;
use super::PanelScreenConversion;
use super::PanelScreenHandoff;
use super::PanelScreenTarget;
use super::PanelWorldConversion;
use super::PanelWorldTarget;
use super::SavedPanelWorldState;
use super::SavedWorldRestoreMode;
use crate::layout::Anchor;
use crate::layout::Sizing;
use crate::layout::Unit;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::PanelPlane;
use crate::panel::PanelScreenBounds;
use crate::panel::ResolvedScreenPanelPosition;
use crate::panel::ScreenPosition;
use crate::screen_space;

/// Read-only access to panel projection helpers.
///
/// `project_to_screen` answers where a panel currently appears in logical
/// screen pixels. `project_to_world` answers where that current screen footprint
/// would sit on a supplied world target plane.
#[derive(SystemParam)]
pub struct PanelProjectionParam<'w, 's> {
    panels: Query<
        'w,
        's,
        (
            &'static DiegeticPanel,
            &'static ComputedDiegeticPanel,
            Option<&'static ResolvedScreenPanelPosition>,
            Option<&'static Transform>,
            Option<&'static PanelScreenHandoff>,
            Option<&'static SavedPanelWorldState>,
        ),
    >,
    cameras:    Query<'w, 's, (&'static Camera, Option<&'static RenderTarget>)>,
    windows:    Query<'w, 's, &'static Window>,
    primary:    Query<'w, 's, Entity, With<PrimaryWindow>>,
    transforms: TransformHelper<'w, 's>,
}

impl PanelProjectionParam<'_, '_> {
    /// Projects `panel` into logical screen coordinates for `camera`.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when either entity is missing, the camera
    /// does not render to a window, transforms are unavailable, the camera cannot
    /// project the panel, or the projected panel has no positive finite size.
    pub fn project_to_screen(
        &self,
        panel: Entity,
        camera: Entity,
    ) -> Result<PanelScreenProjection, PanelProjectionError> {
        let (panel_ref, _, resolved_position, transform, _, _) = self
            .panels
            .get(panel)
            .map_err(|_| PanelProjectionError::PanelMissing)?;
        let (camera_ref, camera_target) = self
            .cameras
            .get(camera)
            .map_err(|_| PanelProjectionError::CameraMissing)?;
        let window = self.camera_window(camera_target)?;
        match panel_ref.coordinate_space() {
            CoordinateSpace::Screen { .. } => {
                self.project_screen_panel(panel, panel_ref, resolved_position, transform)
            },
            CoordinateSpace::World { .. } => {
                self.project_world_panel(panel, window, panel_ref, camera_ref, camera)
            },
        }
    }

    /// Resolves a custom screen target for `panel` after projecting it through `camera`.
    ///
    /// This answers where a conversion would land without mutating the panel.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when the panel cannot be projected or the
    /// target cannot be resolved to a positive finite screen size.
    pub fn project_to_screen_target(
        &self,
        panel: Entity,
        camera: Entity,
        target: PanelScreenTarget,
    ) -> Result<PanelScreenConversion, PanelProjectionError> {
        let projection = self.project_to_screen(panel, camera)?;
        self.conversion_for_screen_projection(panel, projection, target)
    }

    /// Projects the panel's current screen footprint onto `target`.
    ///
    /// This is the no-jump bridge for moving a screen-space panel into world
    /// space: the returned projection uses the target plane and orientation, but
    /// derives world size from the panel's current screen footprint unless the
    /// target supplies explicit world dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when the panel cannot be projected or the
    /// target plane cannot be resolved.
    pub fn project_to_world(
        &self,
        panel: Entity,
        camera: Entity,
        target: PanelWorldTarget,
    ) -> Result<PanelWorldProjection, PanelProjectionError> {
        let (panel_ref, _, resolved_position, transform, _, _) = self
            .panels
            .get(panel)
            .map_err(|_| PanelProjectionError::PanelMissing)?;
        let (camera_ref, _) = self
            .cameras
            .get(camera)
            .map_err(|_| PanelProjectionError::CameraMissing)?;
        let camera_transform = self
            .transforms
            .compute_global_transform(camera)
            .map_err(PanelProjectionError::from)?;
        let screen_projection = match panel_ref.coordinate_space() {
            CoordinateSpace::Screen { .. } => {
                self.project_screen_panel(panel, panel_ref, resolved_position, transform)?
            },
            CoordinateSpace::World { .. } => {
                let (_, camera_target) = self
                    .cameras
                    .get(camera)
                    .map_err(|_| PanelProjectionError::CameraMissing)?;
                let window = self.camera_window(camera_target)?;
                self.project_world_panel(panel, window, panel_ref, camera_ref, camera)?
            },
        };
        let conversion =
            target.resolve(panel_ref, screen_projection, camera_ref, &camera_transform)?;
        Ok(PanelWorldProjection::from_conversion(panel, conversion))
    }

    /// Projects a screen panel back to its saved world handoff.
    ///
    /// The returned projection restores the saved world-authored units and size,
    /// while choosing a camera-facing world pose that preserves the panel's
    /// current screen footprint.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when the panel has no saved handoff, no
    /// saved world state, or the projection cannot be resolved.
    pub fn project_to_saved_world(
        &self,
        panel: Entity,
    ) -> Result<PanelWorldProjection, PanelProjectionError> {
        let (panel_ref, _, resolved_position, transform, handoff, saved) =
            self.panels
                .get(panel)
                .map_err(|_| PanelProjectionError::PanelMissing)?;
        let handoff = handoff.ok_or(PanelProjectionError::ScreenHandoffMissing)?;
        let saved = saved.ok_or(PanelProjectionError::SavedWorldStateMissing)?;
        let (camera_ref, camera_target) = self
            .cameras
            .get(handoff.camera)
            .map_err(|_| PanelProjectionError::CameraMissing)?;
        let camera_transform = self
            .transforms
            .compute_global_transform(handoff.camera)
            .map_err(PanelProjectionError::from)?;
        let screen_projection = match panel_ref.coordinate_space() {
            CoordinateSpace::Screen { .. } => {
                self.project_screen_panel(panel, panel_ref, resolved_position, transform)?
            },
            CoordinateSpace::World { .. } => {
                let window = self.camera_window(camera_target)?;
                self.project_world_panel(panel, window, panel_ref, camera_ref, handoff.camera)?
            },
        };
        let target = PanelWorldTarget::default()
            .transform(handoff_world_transform(handoff, &camera_transform)?)
            .anchor(saved.anchor);
        let conversion =
            target.resolve(panel_ref, screen_projection, camera_ref, &camera_transform)?;
        let projection = PanelWorldProjection::from_conversion(panel, conversion);
        Ok(PanelWorldProjection::from_conversion(
            panel,
            saved.world_conversion(projection),
        ))
    }

    /// Projects a resolved screen conversion onto `target`.
    ///
    /// This is useful before applying a screen conversion: a world panel can
    /// animate to the world pose that already matches the final screen landing,
    /// then switch coordinate spaces without a visible jump.
    ///
    /// # Errors
    ///
    /// Returns [`PanelProjectionError`] when the panel or camera is missing, or
    /// the target plane cannot be resolved.
    pub fn project_screen_to_world(
        &self,
        panel: Entity,
        camera: Entity,
        screen: PanelScreenConversion,
        target: PanelWorldTarget,
    ) -> Result<PanelWorldProjection, PanelProjectionError> {
        let (panel_ref, _, _, _, _, _) = self
            .panels
            .get(panel)
            .map_err(|_| PanelProjectionError::PanelMissing)?;
        let (camera_ref, _) = self
            .cameras
            .get(camera)
            .map_err(|_| PanelProjectionError::CameraMissing)?;
        let camera_transform = self
            .transforms
            .compute_global_transform(camera)
            .map_err(PanelProjectionError::from)?;
        let conversion =
            target.resolve_screen_conversion(panel_ref, screen, camera_ref, &camera_transform)?;
        Ok(PanelWorldProjection::from_conversion(panel, conversion))
    }

    pub(super) fn conversion_for_screen_projection(
        &self,
        panel: Entity,
        projection: PanelScreenProjection,
        target: PanelScreenTarget,
    ) -> Result<PanelScreenConversion, PanelProjectionError> {
        let (panel_ref, computed, _, _, _, _) = self
            .panels
            .get(panel)
            .map_err(|_| PanelProjectionError::PanelMissing)?;
        let window_size = self.window_size(projection.window)?;
        target.resolve(panel_ref, computed, projection, window_size)
    }

    fn camera_window(
        &self,
        camera_target: Option<&RenderTarget>,
    ) -> Result<Entity, PanelProjectionError> {
        match camera_target {
            None | Some(RenderTarget::Window(WindowRef::Primary)) => self
                .primary
                .single()
                .map_err(|_| PanelProjectionError::WindowMissing),
            Some(RenderTarget::Window(WindowRef::Entity(entity))) => Ok(*entity),
            _ => Err(PanelProjectionError::UnsupportedCameraTarget),
        }
    }

    fn project_screen_panel(
        &self,
        panel_entity: Entity,
        panel: &DiegeticPanel,
        resolved_position: Option<&ResolvedScreenPanelPosition>,
        transform: Option<&Transform>,
    ) -> Result<PanelScreenProjection, PanelProjectionError> {
        let CoordinateSpace::Screen { window, .. } = *panel.coordinate_space() else {
            return Err(PanelProjectionError::InvalidProjection);
        };
        let (window, window_size) = self.resolve_window(window)?;
        let anchor_position = screen_anchor_position(panel, window_size, resolved_position)?;
        let size = Vec2::new(panel.width(), panel.height());
        let bounds =
            PanelScreenBounds::from_anchor_position(anchor_position, panel.anchor(), size)?;
        let rotation = transform.map_or(0.0, |transform| {
            screen_space::screen_in_plane_angle(transform.rotation)
        });
        PanelScreenProjection::from_parts(
            panel_entity,
            window,
            anchor_position,
            size,
            rotation,
            rotated_screen_corners(bounds, panel.anchor(), anchor_position, rotation),
        )
    }

    fn project_world_panel(
        &self,
        panel_entity: Entity,
        window: Entity,
        panel: &DiegeticPanel,
        camera: &Camera,
        camera_entity: Entity,
    ) -> Result<PanelScreenProjection, PanelProjectionError> {
        let panel_transform = self
            .transforms
            .compute_global_transform(panel_entity)
            .map_err(PanelProjectionError::from)?;
        let camera_transform = self
            .transforms
            .compute_global_transform(camera_entity)
            .map_err(PanelProjectionError::from)?;
        let plane = PanelPlane::from_panel(panel, &panel_transform)?;
        let corners = project_panel_corners(camera, &camera_transform, plane)?;
        let anchor_position = camera
            .world_to_viewport(&camera_transform, plane.point(panel.anchor()))
            .map_err(|_| PanelProjectionError::ProjectionFailed)?;
        let top_width = corners[0].distance(corners[1]);
        let bottom_width = corners[3].distance(corners[2]);
        let left_height = corners[0].distance(corners[3]);
        let right_height = corners[1].distance(corners[2]);
        let size = Vec2::new(
            average_positive(top_width, bottom_width)?,
            average_positive(left_height, right_height)?,
        );
        let top_edge = corners[1] - corners[0];
        let rotation = top_edge.y.atan2(top_edge.x);
        PanelScreenProjection::from_parts(
            panel_entity,
            window,
            anchor_position,
            size,
            rotation,
            corners,
        )
    }

    fn resolve_window(
        &self,
        window_ref: WindowRef,
    ) -> Result<(Entity, Vec2), PanelProjectionError> {
        let entity = match window_ref {
            WindowRef::Primary => self
                .primary
                .single()
                .map_err(|_| PanelProjectionError::WindowMissing)?,
            WindowRef::Entity(entity) => entity,
        };
        let size = self.window_size(entity)?;
        Ok((entity, size))
    }

    fn window_size(&self, entity: Entity) -> Result<Vec2, PanelProjectionError> {
        let window = self
            .windows
            .get(entity)
            .map_err(|_| PanelProjectionError::WindowMissing)?;
        let size = Vec2::new(window.width(), window.height());
        if !size.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
            return Err(PanelProjectionError::NoViewportSize);
        }
        Ok(size)
    }
}

/// Current logical-pixel placement of a panel in a camera's screen space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelScreenProjection {
    /// Projected panel entity.
    pub panel:           Entity,
    /// Window that owns the logical-pixel coordinates.
    pub window:          Entity,
    /// Projected position of the panel's configured anchor.
    pub anchor_position: Vec2,
    /// Axis-aligned bounds containing the projected corners.
    pub bounds:          PanelScreenBounds,
    /// Approximate screen-space panel size used for conversion.
    pub size:            Vec2,
    /// Approximate in-plane screen rotation in radians.
    pub rotation:        f32,
    /// Projected corners in top-left, top-right, bottom-right, bottom-left order.
    pub corners:         [Vec2; 4],
}

impl PanelScreenProjection {
    fn from_parts(
        panel: Entity,
        window: Entity,
        anchor_position: Vec2,
        size: Vec2,
        rotation: f32,
        corners: [Vec2; 4],
    ) -> Result<Self, PanelProjectionError> {
        if !anchor_position.is_finite() || !size.is_finite() || !rotation.is_finite() {
            return Err(PanelProjectionError::InvalidProjection);
        }
        for corner in corners {
            if !corner.is_finite() {
                return Err(PanelProjectionError::InvalidProjection);
            }
        }
        let bounds = bounds_from_corners(corners)?;
        Ok(Self {
            panel,
            window,
            anchor_position,
            bounds,
            size,
            rotation,
            corners,
        })
    }
}

/// World-space placement of a panel converted from a screen footprint.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelWorldProjection {
    /// Projected panel entity.
    pub panel:               Entity,
    /// Transform to apply to the panel entity.
    pub transform:           Transform,
    /// Resolved world size in meters.
    pub size:                Vec2,
    /// Panel viewport size in `layout_unit`.
    pub panel_size:          Vec2,
    /// Panel layout unit after conversion.
    pub layout_unit:         Unit,
    /// Target anchor.
    pub anchor:              Anchor,
    /// World-space width rule.
    pub width:               Sizing,
    /// World-space height rule.
    pub height:              Sizing,
    /// Target world width in meters.
    pub world_width:         Option<f32>,
    /// Target world height in meters.
    pub world_height:        Option<f32>,
    /// Whether applying this projection should restore saved world-authored panel data.
    pub restore_saved_world: SavedWorldRestoreMode,
}

impl PanelWorldProjection {
    fn from_conversion(panel: Entity, conversion: PanelWorldConversion) -> Self {
        Self {
            panel,
            transform: conversion.transform,
            size: conversion.size,
            panel_size: conversion.panel_size,
            layout_unit: conversion.layout_unit,
            anchor: conversion.anchor.unwrap_or(Anchor::TopLeft),
            width: conversion.width,
            height: conversion.height,
            world_width: conversion.world_width,
            world_height: conversion.world_height,
            restore_saved_world: conversion.restore_saved_world,
        }
    }
}

fn handoff_world_transform(
    handoff: &PanelScreenHandoff,
    camera_transform: &GlobalTransform,
) -> Result<Transform, PanelProjectionError> {
    if !handoff.distance.is_finite() || handoff.distance <= 0.0 {
        return Err(PanelProjectionError::InvalidWorldTarget);
    }
    let camera_transform = camera_transform.compute_transform();
    let forward = camera_transform.rotation * Vec3::NEG_Z;
    Ok(
        Transform::from_translation(camera_transform.translation + forward * handoff.distance)
            .with_rotation(camera_transform.rotation),
    )
}

fn screen_anchor_position(
    panel: &DiegeticPanel,
    window_size: Vec2,
    resolved_position: Option<&ResolvedScreenPanelPosition>,
) -> Result<Vec2, PanelProjectionError> {
    if let Some(anchor_position) = resolved_position.and_then(|position| position.anchor_position) {
        return Ok(anchor_position);
    }
    let CoordinateSpace::Screen { position, .. } = panel.coordinate_space() else {
        return Err(PanelProjectionError::InvalidProjection);
    };
    match *position {
        ScreenPosition::Screen => {
            let (fx, fy) = panel.anchor().offset_fraction();
            Ok(Vec2::new(fx * window_size.x, fy * window_size.y))
        },
        ScreenPosition::At(position) => Ok(position),
    }
}

fn project_panel_corners(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    plane: PanelPlane,
) -> Result<[Vec2; 4], PanelProjectionError> {
    let top_left = plane.origin();
    let top_right = top_left + plane.right() * plane.size().x;
    let bottom_right = top_right - plane.up() * plane.size().y;
    let bottom_left = top_left - plane.up() * plane.size().y;
    Ok([
        camera
            .world_to_viewport(camera_transform, top_left)
            .map_err(|_| PanelProjectionError::ProjectionFailed)?,
        camera
            .world_to_viewport(camera_transform, top_right)
            .map_err(|_| PanelProjectionError::ProjectionFailed)?,
        camera
            .world_to_viewport(camera_transform, bottom_right)
            .map_err(|_| PanelProjectionError::ProjectionFailed)?,
        camera
            .world_to_viewport(camera_transform, bottom_left)
            .map_err(|_| PanelProjectionError::ProjectionFailed)?,
    ])
}

fn rotated_screen_corners(
    bounds: PanelScreenBounds,
    anchor: Anchor,
    anchor_position: Vec2,
    rotation: f32,
) -> [Vec2; 4] {
    let top_left = bounds.top_left();
    let top_right = top_left + Vec2::new(bounds.size().x, 0.0);
    let bottom_right = top_left + bounds.size();
    let bottom_left = top_left + Vec2::new(0.0, bounds.size().y);
    let anchor_offset = bounds.anchor_offset(anchor);
    [
        rotate_around_anchor(
            top_left,
            top_left + anchor_offset,
            anchor_position,
            rotation,
        ),
        rotate_around_anchor(
            top_right,
            top_left + anchor_offset,
            anchor_position,
            rotation,
        ),
        rotate_around_anchor(
            bottom_right,
            top_left + anchor_offset,
            anchor_position,
            rotation,
        ),
        rotate_around_anchor(
            bottom_left,
            top_left + anchor_offset,
            anchor_position,
            rotation,
        ),
    ]
}

fn rotate_around_anchor(
    point: Vec2,
    source_anchor: Vec2,
    target_anchor: Vec2,
    radians: f32,
) -> Vec2 {
    let local = point - source_anchor;
    let (sin, cos) = radians.sin_cos();
    target_anchor
        + Vec2::new(
            local.y.mul_add(-sin, local.x * cos),
            local.y.mul_add(cos, local.x * sin),
        )
}

fn bounds_from_corners(corners: [Vec2; 4]) -> Result<PanelScreenBounds, PanelProjectionError> {
    let mut min = corners[0];
    let mut max = corners[0];
    for corner in corners.into_iter().skip(1) {
        min = min.min(corner);
        max = max.max(corner);
    }
    PanelScreenBounds::new(min, max - min).map_err(PanelProjectionError::from)
}

fn average_positive(a: f32, b: f32) -> Result<f32, PanelProjectionError> {
    if !a.is_finite() || !b.is_finite() || a <= 0.0 || b <= 0.0 {
        return Err(PanelProjectionError::InvalidProjection);
    }
    Ok(f32::midpoint(a, b))
}
