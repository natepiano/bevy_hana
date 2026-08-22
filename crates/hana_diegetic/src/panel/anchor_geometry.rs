//! Read-only panel anchor geometry.

use bevy::ecs::system::SystemParam;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Query;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::transform::helper::ComputeGlobalTransformError;
use bevy::transform::helper::TransformHelper;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;

use super::CoordinateSpace;
use super::DiegeticPanel;
use super::ResolvedScreenPanelPosition;
use super::ScreenPosition;
use crate::layout::Anchor;

const ORTHONORMAL_EPSILON: f32 = 1e-4;

/// Read-only access to the current anchor geometry for a panel entity.
///
/// Screen panels resolve to logical pixels in their target window: top-left
/// origin, x right, y down. World panels resolve to world-space meters using
/// Bevy's [`TransformHelper`], so transform changes written earlier in the same
/// frame are visible even before transform propagation.
///
/// If the same system also writes [`Transform`](bevy::prelude::Transform)
/// components, place this
/// [`SystemParam`] in a [`ParamSet`](bevy::ecs::system::ParamSet) so geometry
/// reads happen before transform writes.
#[derive(SystemParam)]
pub struct PanelAnchorGeometryParam<'w, 's> {
    panels: Query<
        'w,
        's,
        (
            &'static DiegeticPanel,
            Option<&'static ResolvedScreenPanelPosition>,
        ),
    >,
    windows:    Query<'w, 's, &'static Window>,
    primary:    Query<'w, 's, Entity, With<PrimaryWindow>>,
    transforms: TransformHelper<'w, 's>,
}

impl PanelAnchorGeometryParam<'_, '_> {
    /// Resolves anchor geometry for `entity`.
    ///
    /// # Errors
    ///
    /// Returns [`PanelAnchorGeometryError`] when the entity is not a panel, the
    /// screen panel's window is missing or zero-sized, the world panel lacks a
    /// usable transform, or the panel dimensions cannot produce finite geometry.
    pub fn get(
        &self,
        entity: Entity,
    ) -> Result<ResolvedPanelAnchorGeometry, PanelAnchorGeometryError> {
        let (panel, resolved_position) = self
            .panels
            .get(entity)
            .map_err(|_| PanelAnchorGeometryError::PanelMissing)?;
        match panel.coordinate_space() {
            CoordinateSpace::Screen { window, .. } => {
                let (window, window_size) = self.resolve_window(*window)?;
                ResolvedPanelAnchorGeometry::from_screen_panel(
                    panel,
                    window,
                    window_size,
                    resolved_position,
                )
            },
            CoordinateSpace::World { .. } => {
                let transform = self
                    .transforms
                    .compute_global_transform(entity)
                    .map_err(PanelAnchorGeometryError::from)?;
                ResolvedPanelAnchorGeometry::from_world_panel(panel, &transform)
            },
        }
    }

    fn resolve_window(
        &self,
        window_ref: WindowRef,
    ) -> Result<(Entity, Vec2), PanelAnchorGeometryError> {
        let window_entity = match window_ref {
            WindowRef::Primary => self
                .primary
                .single()
                .map_err(|_| PanelAnchorGeometryError::WindowMissing)?,
            WindowRef::Entity(entity) => entity,
        };
        let window = self
            .windows
            .get(window_entity)
            .map_err(|_| PanelAnchorGeometryError::WindowMissing)?;
        let size = Vec2::new(window.width(), window.height());
        if !valid_size(size) {
            return Err(PanelAnchorGeometryError::WindowZeroSized);
        }
        Ok((window_entity, size))
    }
}

/// Current anchor geometry for one panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedPanelAnchorGeometry {
    points: PanelAnchorPoints,
}

impl ResolvedPanelAnchorGeometry {
    /// Builds screen-space geometry in logical pixels.
    fn from_screen_panel(
        panel: &DiegeticPanel,
        window: Entity,
        window_size: Vec2,
        resolved_position: Option<&ResolvedScreenPanelPosition>,
    ) -> Result<Self, PanelAnchorGeometryError> {
        let anchor_position = screen_anchor_position(
            panel,
            window_size,
            resolved_position.and_then(|p| p.anchor_position),
        )?;
        let bounds = PanelScreenBounds::from_anchor_position(
            anchor_position,
            panel.anchor(),
            Vec2::new(panel.width(), panel.height()),
        )?;
        Ok(Self {
            points: PanelAnchorPoints::Screen { window, bounds },
        })
    }

    /// Builds world-space geometry in meters.
    ///
    /// # Errors
    ///
    /// Returns [`PanelAnchorGeometryError::InvalidPanelSize`] for invalid panel
    /// dimensions and [`PanelAnchorGeometryError::InvalidPanelPlane`] for
    /// degenerate or sheared transform axes, propagated from
    /// [`PanelPlane::from_panel`].
    pub fn from_world_panel(
        panel: &DiegeticPanel,
        transform: &GlobalTransform,
    ) -> Result<Self, PanelAnchorGeometryError> {
        Ok(Self {
            points: PanelAnchorPoints::World {
                plane: PanelPlane::from_panel(panel, transform)?,
            },
        })
    }

    /// Anchor points for this panel.
    #[must_use]
    pub const fn points(&self) -> &PanelAnchorPoints { &self.points }

    /// Resolves one anchor point.
    #[must_use]
    pub fn point(&self, anchor: Anchor) -> PanelAnchorPoint { self.points.point(anchor) }

    /// Resolves one panel edge.
    #[must_use]
    pub fn edge(&self, edge: PanelAnchorEdge) -> PanelAnchorEdgeEndpoints { self.points.edge(edge) }
}

/// Coordinate-space-specific anchor geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelAnchorPoints {
    /// Screen-space panel bounds in logical pixels.
    Screen {
        /// Window that owns these logical-pixel coordinates.
        window: Entity,
        /// Panel bounds in the window.
        bounds: PanelScreenBounds,
    },
    /// World-space panel plane in meters.
    World {
        /// Panel plane with top-left origin and resolved meter size.
        plane: PanelPlane,
    },
}

impl PanelAnchorPoints {
    /// Resolves one anchor point.
    #[must_use]
    pub fn point(self, anchor: Anchor) -> PanelAnchorPoint {
        match self {
            Self::Screen { bounds, .. } => PanelAnchorPoint::Screen(bounds.point(anchor)),
            Self::World { plane } => PanelAnchorPoint::World(plane.point(anchor)),
        }
    }

    /// Resolves one panel edge.
    #[must_use]
    pub fn edge(self, edge: PanelAnchorEdge) -> PanelAnchorEdgeEndpoints {
        match self {
            Self::Screen { bounds, .. } => PanelAnchorEdgeEndpoints::Screen(bounds.edge(edge)),
            Self::World { plane } => PanelAnchorEdgeEndpoints::World(plane.edge(edge)),
        }
    }

    /// Returns screen bounds, if this is screen-space geometry.
    #[must_use]
    pub const fn screen_bounds(self) -> Option<PanelScreenBounds> {
        match self {
            Self::Screen { bounds, .. } => Some(bounds),
            Self::World { .. } => None,
        }
    }

    /// Returns the world plane, if this is world-space geometry.
    #[must_use]
    pub const fn world_plane(self) -> Option<PanelPlane> {
        match self {
            Self::Screen { .. } => None,
            Self::World { plane } => Some(plane),
        }
    }
}

/// One resolved anchor point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelAnchorPoint {
    /// Screen point in logical pixels.
    Screen(Vec2),
    /// World point in meters.
    World(Vec3),
}

impl PanelAnchorPoint {
    /// Returns the screen point, if this is screen geometry.
    #[must_use]
    pub const fn as_screen(self) -> Option<Vec2> {
        match self {
            Self::Screen(point) => Some(point),
            Self::World(_) => None,
        }
    }

    /// Returns the world point, if this is world geometry.
    #[must_use]
    pub const fn as_world(self) -> Option<Vec3> {
        match self {
            Self::Screen(_) => None,
            Self::World(point) => Some(point),
        }
    }
}

/// One resolved panel edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelAnchorEdgeEndpoints {
    /// Screen edge endpoints in logical pixels.
    Screen([Vec2; 2]),
    /// World edge endpoints in meters.
    World([Vec3; 2]),
}

impl PanelAnchorEdgeEndpoints {
    /// Returns screen endpoints, if this is screen geometry.
    #[must_use]
    pub const fn as_screen(self) -> Option<[Vec2; 2]> {
        match self {
            Self::Screen(points) => Some(points),
            Self::World(_) => None,
        }
    }

    /// Returns world endpoints, if this is world geometry.
    #[must_use]
    pub const fn as_world(self) -> Option<[Vec3; 2]> {
        match self {
            Self::Screen(_) => None,
            Self::World(points) => Some(points),
        }
    }
}

/// Panel edge selector for anchor geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelAnchorEdge {
    /// Top edge.
    Top,
    /// Right edge.
    Right,
    /// Bottom edge.
    Bottom,
    /// Left edge.
    Left,
}

/// Screen-space panel bounds in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelScreenBounds {
    top_left: Vec2,
    size:     Vec2,
}

impl PanelScreenBounds {
    /// Creates bounds from a top-left point and size.
    ///
    /// # Errors
    ///
    /// Returns [`PanelAnchorGeometryError::InvalidPanelSize`] for non-finite or
    /// non-positive sizes.
    pub fn new(top_left: Vec2, size: Vec2) -> Result<Self, PanelAnchorGeometryError> {
        if !top_left.is_finite() || !valid_size(size) {
            return Err(PanelAnchorGeometryError::InvalidPanelSize);
        }
        Ok(Self { top_left, size })
    }

    /// Creates bounds from a panel anchor position.
    ///
    /// # Errors
    ///
    /// Returns [`PanelAnchorGeometryError::InvalidPanelSize`] for non-finite or
    /// non-positive sizes, propagated from [`PanelScreenBounds::new`].
    pub fn from_anchor_position(
        anchor_position: Vec2,
        anchor: Anchor,
        size: Vec2,
    ) -> Result<Self, PanelAnchorGeometryError> {
        Self::new(anchor_position - anchor_offset(anchor, size), size)
    }

    /// Top-left point in logical pixels.
    #[must_use]
    pub const fn top_left(self) -> Vec2 { self.top_left }

    /// Size in logical pixels.
    #[must_use]
    pub const fn size(self) -> Vec2 { self.size }

    /// Offset of `anchor` from the top-left point.
    #[must_use]
    pub fn anchor_offset(self, anchor: Anchor) -> Vec2 { anchor_offset(anchor, self.size) }

    /// Resolves one anchor point in logical pixels.
    #[must_use]
    pub fn point(self, anchor: Anchor) -> Vec2 { self.top_left + self.anchor_offset(anchor) }

    /// Resolves one edge in logical pixels.
    #[must_use]
    pub fn edge(self, edge: PanelAnchorEdge) -> [Vec2; 2] {
        let top_left = self.top_left;
        let top_right = top_left + Vec2::new(self.size.x, 0.0);
        let bottom_left = top_left + Vec2::new(0.0, self.size.y);
        let bottom_right = top_left + self.size;
        match edge {
            PanelAnchorEdge::Top => [top_left, top_right],
            PanelAnchorEdge::Right => [top_right, bottom_right],
            PanelAnchorEdge::Bottom => [bottom_left, bottom_right],
            PanelAnchorEdge::Left => [top_left, bottom_left],
        }
    }
}

/// World-space panel plane in meters.
///
/// `origin` is the panel's top-left corner. `right`, `up`, and `normal` are
/// unit world directions. `size` is the resolved world-space panel size in
/// meters after panel sizing and transform scale are applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelPlane {
    origin: Vec3,
    right:  Vec3,
    up:     Vec3,
    normal: Vec3,
    size:   Vec2,
}

impl PanelPlane {
    /// Creates a world-space panel plane from a panel and current transform.
    ///
    /// # Errors
    ///
    /// Returns [`PanelAnchorGeometryError::InvalidPanelSize`] for invalid panel
    /// dimensions and [`PanelAnchorGeometryError::InvalidPanelPlane`] for
    /// degenerate or sheared transform axes.
    pub fn from_panel(
        panel: &DiegeticPanel,
        transform: &GlobalTransform,
    ) -> Result<Self, PanelAnchorGeometryError> {
        let affine = transform.affine();
        let right_axis = affine.transform_vector3(Vec3::X);
        let up_axis = affine.transform_vector3(Vec3::Y);
        let right_scale = right_axis.length();
        let up_scale = up_axis.length();
        if !right_scale.is_finite() || !up_scale.is_finite() {
            return Err(PanelAnchorGeometryError::InvalidPanelPlane);
        }

        let size = Vec2::new(
            panel.world_width() * right_scale,
            panel.world_height() * up_scale,
        );
        if !valid_size(size) {
            return Err(PanelAnchorGeometryError::InvalidPanelSize);
        }

        let right = normalize_axis(right_axis)?;
        let up = normalize_axis(up_axis)?;
        let normal = checked_normal(right, up)?;
        let panel_anchor_offset = anchor_offset(panel.anchor(), size);
        let anchor_point = transform.transform_point(Vec3::ZERO);
        let origin = anchor_point - right * panel_anchor_offset.x + up * panel_anchor_offset.y;
        Self::from_top_left(origin, right, up, size).map(|plane| Self { normal, ..plane })
    }

    /// Creates a plane from a top-left point, basis, and size.
    ///
    /// # Errors
    ///
    /// Returns [`PanelAnchorGeometryError::InvalidPanelSize`] for a non-finite
    /// origin or invalid size and [`PanelAnchorGeometryError::InvalidPanelPlane`]
    /// for degenerate or sheared basis axes.
    pub fn from_top_left(
        origin: Vec3,
        right: Vec3,
        up: Vec3,
        size: Vec2,
    ) -> Result<Self, PanelAnchorGeometryError> {
        if !origin.is_finite() || !valid_size(size) {
            return Err(PanelAnchorGeometryError::InvalidPanelSize);
        }
        let right = normalize_axis(right)?;
        let up = normalize_axis(up)?;
        let normal = checked_normal(right, up)?;
        Ok(Self {
            origin,
            right,
            up,
            normal,
            size,
        })
    }

    /// Top-left point in world meters.
    #[must_use]
    pub const fn origin(self) -> Vec3 { self.origin }

    /// Unit right direction.
    #[must_use]
    pub const fn right(self) -> Vec3 { self.right }

    /// Unit up direction.
    #[must_use]
    pub const fn up(self) -> Vec3 { self.up }

    /// Unit normal direction.
    #[must_use]
    pub const fn normal(self) -> Vec3 { self.normal }

    /// Resolved world-space size in meters.
    #[must_use]
    pub const fn size(self) -> Vec2 { self.size }

    /// Resolves one anchor point in world meters.
    #[must_use]
    pub fn point(self, anchor: Anchor) -> Vec3 {
        let offset = anchor_offset(anchor, self.size);
        self.origin + self.right * offset.x - self.up * offset.y
    }

    /// Resolves one edge in world meters.
    #[must_use]
    pub fn edge(self, edge: PanelAnchorEdge) -> [Vec3; 2] {
        let top_left = self.origin;
        let top_right = top_left + self.right * self.size.x;
        let bottom_left = top_left - self.up * self.size.y;
        let bottom_right = top_right - self.up * self.size.y;
        match edge {
            PanelAnchorEdge::Top => [top_left, top_right],
            PanelAnchorEdge::Right => [top_right, bottom_right],
            PanelAnchorEdge::Bottom => [bottom_left, bottom_right],
            PanelAnchorEdge::Left => [top_left, bottom_left],
        }
    }
}

/// Why panel anchor geometry could not be resolved.
#[derive(thiserror::Error, Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelAnchorGeometryError {
    /// The entity has no [`DiegeticPanel`].
    #[error("panel is missing")]
    PanelMissing,
    /// The target window could not be found.
    #[error("window is missing")]
    WindowMissing,
    /// The target window has a zero-sized axis.
    #[error("window is zero-sized")]
    WindowZeroSized,
    /// A world panel transform could not be computed.
    #[error("transform is unavailable")]
    TransformUnavailable,
    /// Panel dimensions were non-finite or non-positive.
    #[error("panel size is invalid")]
    InvalidPanelSize,
    /// The world transform did not produce a usable orthonormal panel plane.
    #[error("panel plane is invalid")]
    InvalidPanelPlane,
}

impl From<ComputeGlobalTransformError> for PanelAnchorGeometryError {
    fn from(_: ComputeGlobalTransformError) -> Self { Self::TransformUnavailable }
}

pub(crate) fn screen_anchor_position(
    panel: &DiegeticPanel,
    window_size: Vec2,
    resolved_anchor_position: Option<Vec2>,
) -> Result<Vec2, PanelAnchorGeometryError> {
    if !valid_size(window_size) {
        return Err(PanelAnchorGeometryError::WindowZeroSized);
    }
    if let Some(anchor_position) = resolved_anchor_position {
        if anchor_position.is_finite() {
            return Ok(anchor_position);
        }
        return Err(PanelAnchorGeometryError::InvalidPanelSize);
    }

    let CoordinateSpace::Screen { position, .. } = panel.coordinate_space() else {
        return Err(PanelAnchorGeometryError::PanelMissing);
    };
    match *position {
        ScreenPosition::Screen => {
            let (x, y) = panel.anchor().offset_fraction();
            Ok(Vec2::new(x * window_size.x, y * window_size.y))
        },
        ScreenPosition::At(position) => {
            if position.is_finite() {
                Ok(position)
            } else {
                Err(PanelAnchorGeometryError::InvalidPanelSize)
            }
        },
    }
}

fn anchor_offset(anchor: Anchor, size: Vec2) -> Vec2 {
    let (x, y) = anchor.offset(size.x, size.y);
    Vec2::new(x, y)
}

fn valid_size(size: Vec2) -> bool { size.is_finite() && size.x > 0.0 && size.y > 0.0 }

fn normalize_axis(axis: Vec3) -> Result<Vec3, PanelAnchorGeometryError> {
    axis.try_normalize()
        .filter(|axis| axis.is_finite())
        .ok_or(PanelAnchorGeometryError::InvalidPanelPlane)
}

fn checked_normal(right: Vec3, up: Vec3) -> Result<Vec3, PanelAnchorGeometryError> {
    if right.dot(up).abs() > ORTHONORMAL_EPSILON {
        return Err(PanelAnchorGeometryError::InvalidPanelPlane);
    }
    normalize_axis(right.cross(up))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::error::Error;

    use bevy::ecs::system::SystemState;
    use bevy::prelude::App;
    use bevy::prelude::Component;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::ParamSet;
    use bevy::prelude::Quat;
    use bevy::prelude::Query;
    use bevy::prelude::Res;
    use bevy::prelude::Resource;
    use bevy::prelude::Transform;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::Window;
    use bevy::prelude::With;
    use bevy::transform::helper::ComputeGlobalTransformError;
    use bevy::window::PrimaryWindow;

    use super::PanelAnchorEdge;
    use super::PanelAnchorEdgeEndpoints;
    use super::PanelAnchorGeometryError;
    use super::PanelAnchorGeometryParam;
    use super::PanelAnchorPoint;
    use super::PanelAnchorPoints;
    use super::PanelPlane;
    use super::PanelScreenBounds;
    use crate::DiegeticTextMeasurer;
    use crate::HeadlessLayoutPlugin;
    use crate::Mm;
    use crate::Px;
    use crate::layout::Anchor;
    use crate::panel::DiegeticPanel;
    use crate::panel::ResolvedScreenPanelPosition;

    const SCREEN_TOP_LEFT: Vec2 = Vec2::new(10.0, 20.0);
    const SCREEN_SIZE: Vec2 = Vec2::new(100.0, 40.0);

    #[derive(Component)]
    struct Mover;

    #[derive(Resource)]
    struct Target(Entity);

    #[test]
    fn panel_anchor_geometry_error_messages_are_stable() {
        let cases = [
            (PanelAnchorGeometryError::PanelMissing, "panel is missing"),
            (PanelAnchorGeometryError::WindowMissing, "window is missing"),
            (
                PanelAnchorGeometryError::WindowZeroSized,
                "window is zero-sized",
            ),
            (
                PanelAnchorGeometryError::TransformUnavailable,
                "transform is unavailable",
            ),
            (
                PanelAnchorGeometryError::InvalidPanelSize,
                "panel size is invalid",
            ),
            (
                PanelAnchorGeometryError::InvalidPanelPlane,
                "panel plane is invalid",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn transform_error_conversion_is_lossy() {
        let error = PanelAnchorGeometryError::from(ComputeGlobalTransformError::MissingTransform(
            Entity::PLACEHOLDER,
        ));

        assert_eq!(error, PanelAnchorGeometryError::TransformUnavailable);
        assert!(error.source().is_none());
    }

    fn assert_close_2d(actual: Vec2, expected: Vec2) {
        assert!(
            actual.abs_diff_eq(expected, 1e-4),
            "expected {expected:?}, got {actual:?}",
        );
    }

    fn assert_close_3d(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-4),
            "expected {expected:?}, got {actual:?}",
        );
    }

    fn fixed_screen_panel(size: Vec2, anchor: Anchor, screen_position: Vec2) -> DiegeticPanel {
        DiegeticPanel::screen()
            .size(Px(size.x), Px(size.y))
            .anchor(anchor)
            .screen_position(screen_position.x, screen_position.y)
            .layout(|_| {})
            .build()
            .expect("screen panel builds")
    }

    fn world_panel(anchor: Anchor) -> DiegeticPanel {
        DiegeticPanel::world()
            .size(Mm(200.0), Mm(100.0))
            .world_width(2.0)
            .anchor(anchor)
            .layout(|_| {})
            .build()
            .expect("world panel builds")
    }

    fn app_with_window() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(DiegeticTextMeasurer::default());
        app.add_plugins(HeadlessLayoutPlugin);
        app.world_mut().spawn((
            Window {
                resolution: (800_u32, 600_u32).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        app
    }

    #[test]
    fn screen_bounds_points_return_literal_coordinates_for_all_anchors() {
        let bounds =
            PanelScreenBounds::new(SCREEN_TOP_LEFT, SCREEN_SIZE).expect("screen bounds are valid");
        let cases = [
            (Anchor::TopLeft, Vec2::new(10.0, 20.0)),
            (Anchor::TopCenter, Vec2::new(60.0, 20.0)),
            (Anchor::TopRight, Vec2::new(110.0, 20.0)),
            (Anchor::CenterLeft, Vec2::new(10.0, 40.0)),
            (Anchor::Center, Vec2::new(60.0, 40.0)),
            (Anchor::CenterRight, Vec2::new(110.0, 40.0)),
            (Anchor::BottomLeft, Vec2::new(10.0, 60.0)),
            (Anchor::BottomCenter, Vec2::new(60.0, 60.0)),
            (Anchor::BottomRight, Vec2::new(110.0, 60.0)),
        ];

        for (anchor, expected) in cases {
            assert_close_2d(bounds.point(anchor), expected);
        }
    }

    #[test]
    fn screen_bounds_edges_return_literal_endpoints() {
        let bounds =
            PanelScreenBounds::new(SCREEN_TOP_LEFT, SCREEN_SIZE).expect("screen bounds are valid");
        let cases = [
            (
                PanelAnchorEdge::Top,
                [Vec2::new(10.0, 20.0), Vec2::new(110.0, 20.0)],
            ),
            (
                PanelAnchorEdge::Right,
                [Vec2::new(110.0, 20.0), Vec2::new(110.0, 60.0)],
            ),
            (
                PanelAnchorEdge::Bottom,
                [Vec2::new(10.0, 60.0), Vec2::new(110.0, 60.0)],
            ),
            (
                PanelAnchorEdge::Left,
                [Vec2::new(10.0, 20.0), Vec2::new(10.0, 60.0)],
            ),
        ];

        for (edge, expected) in cases {
            let [first, second] = bounds.edge(edge);
            assert_close_2d(first, expected[0]);
            assert_close_2d(second, expected[1]);
        }
    }

    #[test]
    fn screen_geometry_param_reads_resolved_attachment_position() {
        let mut app = app_with_window();
        let panel = app
            .world_mut()
            .spawn(fixed_screen_panel(
                SCREEN_SIZE,
                Anchor::Center,
                Vec2::new(400.0, 300.0),
            ))
            .id();
        app.world_mut()
            .entity_mut(panel)
            .insert(ResolvedScreenPanelPosition {
                anchor_position: Some(Vec2::new(120.0, 80.0)),
                ..Default::default()
            });

        let mut state = SystemState::<PanelAnchorGeometryParam>::new(app.world_mut());
        let geometry = state
            .get(app.world())
            .expect("geometry param validates")
            .get(panel)
            .expect("screen geometry resolves");
        let PanelAnchorPoints::Screen { window: _, bounds } = *geometry.points() else {
            panic!("expected screen geometry");
        };

        assert_close_2d(bounds.top_left(), Vec2::new(70.0, 60.0));
        assert_close_2d(
            geometry
                .point(Anchor::BottomRight)
                .as_screen()
                .expect("screen point"),
            Vec2::new(170.0, 100.0),
        );
    }

    #[test]
    fn screen_geometry_param_reports_missing_window() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(HeadlessLayoutPlugin);
        let panel = app
            .world_mut()
            .spawn(fixed_screen_panel(
                SCREEN_SIZE,
                Anchor::TopLeft,
                Vec2::new(0.0, 0.0),
            ))
            .id();

        let mut state = SystemState::<PanelAnchorGeometryParam>::new(app.world_mut());
        let error = state
            .get(app.world())
            .expect("geometry param validates")
            .get(panel)
            .expect_err("screen geometry needs a window");

        assert_eq!(error, PanelAnchorGeometryError::WindowMissing);
    }

    #[test]
    fn world_plane_basis_is_unit_orthonormal_with_resolved_meter_size() {
        let panel = world_panel(Anchor::Center);
        let transform = GlobalTransform::from(
            Transform::from_translation(Vec3::new(1.0, 2.0, 3.0))
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        );

        let plane = PanelPlane::from_panel(&panel, &transform).expect("world plane resolves");

        assert_close_3d(plane.right(), Vec3::Y);
        assert_close_3d(plane.up(), Vec3::NEG_X);
        assert_close_3d(plane.normal(), Vec3::Z);
        assert_close_2d(plane.size(), Vec2::new(2.0, 1.0));
        assert_close_3d(plane.point(Anchor::Center), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn world_geometry_param_reads_same_frame_transform_changes() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(HeadlessLayoutPlugin);
        let panel = app
            .world_mut()
            .spawn((
                world_panel(Anchor::Center),
                Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            ))
            .id();

        let mut state = SystemState::<PanelAnchorGeometryParam>::new(app.world_mut());
        let initial = state
            .get(app.world())
            .expect("geometry param validates")
            .get(panel)
            .expect("world geometry resolves")
            .point(Anchor::Center)
            .as_world()
            .expect("world point");
        assert_close_3d(initial, Vec3::new(1.0, 0.0, 0.0));

        app.world_mut()
            .get_mut::<Transform>(panel)
            .expect("panel has transform")
            .translation = Vec3::new(5.0, 0.0, 0.0);
        let current = state
            .get(app.world())
            .expect("geometry param validates")
            .get(panel)
            .expect("world geometry resolves")
            .point(Anchor::Center)
            .as_world()
            .expect("world point");

        assert_close_3d(current, Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn world_plane_rejects_sheared_basis() {
        let error =
            PanelPlane::from_top_left(Vec3::ZERO, Vec3::X, Vec3::new(0.25, 1.0, 0.0), Vec2::ONE)
                .expect_err("sheared basis is rejected");

        assert_eq!(error, PanelAnchorGeometryError::InvalidPanelPlane);
    }

    #[test]
    fn consumer_can_move_toward_anchor_point_without_attachment() {
        fn move_mover_to_target_anchor(
            mut params: ParamSet<(PanelAnchorGeometryParam, Query<&mut Transform, With<Mover>>)>,
            target: Res<Target>,
        ) {
            let target_point = params
                .p0()
                .get(target.0)
                .expect("target geometry resolves")
                .point(Anchor::BottomRight);
            let PanelAnchorPoint::Screen(target_point) = target_point else {
                panic!("target should be screen geometry");
            };

            let mut movers = params.p1();
            for mut transform in &mut movers {
                transform.translation.x = target_point.x;
                transform.translation.y = target_point.y;
            }
        }

        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                SCREEN_SIZE,
                Anchor::TopLeft,
                Vec2::new(30.0, 40.0),
            ))
            .id();
        let mover = app.world_mut().spawn((Mover, Transform::default())).id();
        app.insert_resource(Target(target));
        app.add_systems(Update, move_mover_to_target_anchor);

        app.update();

        let transform = app
            .world()
            .get::<Transform>(mover)
            .expect("mover has transform");
        assert_close_2d(
            transform.translation.truncate(),
            Vec2::new(30.0 + SCREEN_SIZE.x, 40.0 + SCREEN_SIZE.y),
        );
    }

    #[test]
    fn resolved_geometry_edge_dispatches_by_coordinate_space() {
        let bounds =
            PanelScreenBounds::new(SCREEN_TOP_LEFT, SCREEN_SIZE).expect("screen bounds are valid");
        let points = PanelAnchorPoints::Screen {
            window: Entity::PLACEHOLDER,
            bounds,
        };

        let PanelAnchorEdgeEndpoints::Screen(edge) = points.edge(PanelAnchorEdge::Top) else {
            panic!("expected screen edge");
        };

        assert_close_2d(edge[0], Vec2::new(10.0, 20.0));
        assert_close_2d(edge[1], Vec2::new(110.0, 20.0));
    }
}
