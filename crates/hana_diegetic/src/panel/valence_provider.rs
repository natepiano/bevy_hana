//! Provider that writes `hana_valence` anchor geometry for diegetic panels.

use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use hana_valence::AnchorFrame;
use hana_valence::AnchorSite;
use hana_valence::Edge;
use hana_valence::GeometryError;
use hana_valence::Orientation;
use hana_valence::Position;
use hana_valence::ResolvedAnchorGeometry;

use super::CoordinateSpace;
use super::DiegeticPanel;
use super::lifecycle;
use crate::layout::Anchor;

const BOTTOM_CENTER_EDGE: u32 = 2;
const BOTTOM_LEFT_VERTEX: u32 = 3;
const BOTTOM_RIGHT_VERTEX: u32 = 2;
const CENTER_LEFT_EDGE: u32 = 3;
const CENTER_RIGHT_EDGE: u32 = 1;
const QUAD_ANCHOR_COUNT: usize = 9;
const QUAD_EDGE_COUNT: usize = 4;
const TOP_CENTER_EDGE: u32 = 0;
const TOP_LEFT_VERTEX: u32 = 0;
const TOP_RIGHT_VERTEX: u32 = 1;

impl From<Anchor> for AnchorSite {
    fn from(anchor: Anchor) -> Self {
        match anchor {
            Anchor::TopLeft => Self::Vertex(TOP_LEFT_VERTEX),
            Anchor::TopRight => Self::Vertex(TOP_RIGHT_VERTEX),
            Anchor::BottomRight => Self::Vertex(BOTTOM_RIGHT_VERTEX),
            Anchor::BottomLeft => Self::Vertex(BOTTOM_LEFT_VERTEX),
            Anchor::TopCenter => Self::EdgeMidpoint(TOP_CENTER_EDGE),
            Anchor::CenterRight => Self::EdgeMidpoint(CENTER_RIGHT_EDGE),
            Anchor::BottomCenter => Self::EdgeMidpoint(BOTTOM_CENTER_EDGE),
            Anchor::CenterLeft => Self::EdgeMidpoint(CENTER_LEFT_EDGE),
            Anchor::Center => Self::Center,
        }
    }
}

impl TryFrom<AnchorSite> for Anchor {
    type Error = AnchorSite;

    fn try_from(anchor_id: AnchorSite) -> Result<Self, Self::Error> {
        match anchor_id {
            AnchorSite::Vertex(TOP_LEFT_VERTEX) => Ok(Self::TopLeft),
            AnchorSite::Vertex(TOP_RIGHT_VERTEX) => Ok(Self::TopRight),
            AnchorSite::Vertex(BOTTOM_RIGHT_VERTEX) => Ok(Self::BottomRight),
            AnchorSite::Vertex(BOTTOM_LEFT_VERTEX) => Ok(Self::BottomLeft),
            AnchorSite::EdgeMidpoint(TOP_CENTER_EDGE) => Ok(Self::TopCenter),
            AnchorSite::EdgeMidpoint(CENTER_RIGHT_EDGE) => Ok(Self::CenterRight),
            AnchorSite::EdgeMidpoint(BOTTOM_CENTER_EDGE) => Ok(Self::BottomCenter),
            AnchorSite::EdgeMidpoint(CENTER_LEFT_EDGE) => Ok(Self::CenterLeft),
            AnchorSite::Center => Ok(Self::Center),
            unmapped => Err(unmapped),
        }
    }
}

pub(super) fn write_panel_anchor_geometry(
    mut commands: Commands,
    panels: Query<
        (Entity, &DiegeticPanel, Option<&ResolvedAnchorGeometry>),
        Changed<DiegeticPanel>,
    >,
) {
    for (entity, panel, geometry) in &panels {
        if !matches!(panel.coordinate_space(), CoordinateSpace::World { .. }) {
            continue;
        }
        let Ok(next_geometry) = panel_anchor_geometry(panel) else {
            continue;
        };
        if geometry.is_none_or(|geometry| !same_geometry(geometry, &next_geometry)) {
            lifecycle::write_owned_component(&mut commands, entity, entity, next_geometry);
        }
    }
}

fn same_geometry(left: &ResolvedAnchorGeometry, right: &ResolvedAnchorGeometry) -> bool {
    left.frames().count() == right.frames().count()
        && left.frames().all(|(site, left_frame)| {
            right
                .frame(*site)
                .is_ok_and(|right_frame| left_frame == right_frame)
        })
        && left.edges() == right.edges()
}

/// Builds quad anchor geometry for `panel` in its local frame.
pub(super) fn panel_anchor_geometry(
    panel: &DiegeticPanel,
) -> Result<ResolvedAnchorGeometry, GeometryError> {
    write_geometry(
        Vec2::new(panel.world_width(), panel.world_height()),
        panel.anchor(),
    )
}

fn write_geometry(
    size: Vec2,
    panel_anchor: Anchor,
) -> Result<ResolvedAnchorGeometry, GeometryError> {
    let mut frames = Vec::with_capacity(QUAD_ANCHOR_COUNT);
    for (anchor, position) in quad_anchor_points(size, panel_anchor) {
        let frame = AnchorFrame::try_new(Position::from(position), Orientation::default())?;
        frames.push((AnchorSite::from(anchor), frame));
    }
    // A panel can be point-sized while content layout is settling. Its named
    // sites still provide a meaningful point attachment, but no quad edge has
    // a usable direction until the panel gains extent. Validate each retained
    // edge through `ResolvedAnchorGeometry::try_new` rather than publishing a
    // degenerate edge or loosening that constructor's invariant.
    let sites_only = ResolvedAnchorGeometry::try_new(frames.clone(), Vec::<Edge>::new())?;
    let edges = quad_edges()
        .into_iter()
        .filter(|edge| edge.axis(&sites_only).is_ok())
        .collect::<Vec<_>>();
    ResolvedAnchorGeometry::try_new(frames, edges)
}

fn quad_anchor_points(size: Vec2, panel_anchor: Anchor) -> [(Anchor, Vec3); QUAD_ANCHOR_COUNT] {
    let panel_offset = anchor_offset(panel_anchor, size);
    [
        (
            Anchor::TopLeft,
            anchor_position(Anchor::TopLeft, size, panel_offset),
        ),
        (
            Anchor::TopRight,
            anchor_position(Anchor::TopRight, size, panel_offset),
        ),
        (
            Anchor::BottomRight,
            anchor_position(Anchor::BottomRight, size, panel_offset),
        ),
        (
            Anchor::BottomLeft,
            anchor_position(Anchor::BottomLeft, size, panel_offset),
        ),
        (
            Anchor::TopCenter,
            anchor_position(Anchor::TopCenter, size, panel_offset),
        ),
        (
            Anchor::CenterRight,
            anchor_position(Anchor::CenterRight, size, panel_offset),
        ),
        (
            Anchor::BottomCenter,
            anchor_position(Anchor::BottomCenter, size, panel_offset),
        ),
        (
            Anchor::CenterLeft,
            anchor_position(Anchor::CenterLeft, size, panel_offset),
        ),
        (
            Anchor::Center,
            anchor_position(Anchor::Center, size, panel_offset),
        ),
    ]
}

fn anchor_position(anchor: Anchor, size: Vec2, panel_offset: Vec2) -> Vec3 {
    let anchor_offset = anchor_offset(anchor, size);
    Vec3::new(
        anchor_offset.x - panel_offset.x,
        panel_offset.y - anchor_offset.y,
        0.0,
    )
}

fn anchor_offset(anchor: Anchor, size: Vec2) -> Vec2 {
    let (x, y) = anchor.offset(size.x, size.y);
    Vec2::new(x, y)
}

const fn quad_edges() -> [Edge; QUAD_EDGE_COUNT] {
    [
        Edge {
            start: AnchorSite::Vertex(TOP_LEFT_VERTEX),
            end:   AnchorSite::Vertex(TOP_RIGHT_VERTEX),
        },
        Edge {
            start: AnchorSite::Vertex(TOP_RIGHT_VERTEX),
            end:   AnchorSite::Vertex(BOTTOM_RIGHT_VERTEX),
        },
        Edge {
            start: AnchorSite::Vertex(BOTTOM_RIGHT_VERTEX),
            end:   AnchorSite::Vertex(BOTTOM_LEFT_VERTEX),
        },
        Edge {
            start: AnchorSite::Vertex(BOTTOM_LEFT_VERTEX),
            end:   AnchorSite::Vertex(TOP_LEFT_VERTEX),
        },
    ]
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::schedule::Schedule;
    use bevy::ecs::world::World;
    use bevy::prelude::Entity;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use hana_valence::AnchorSite;
    use hana_valence::ResolvedAnchorGeometry;

    use super::quad_edges;
    use super::write_geometry;
    use super::write_panel_anchor_geometry;
    use crate::layout::Anchor;
    use crate::panel::DiegeticPanel;

    const EXPECTED_EDGE_COUNT: usize = 4;
    const EXPECTED_POINT_COUNT: usize = 9;
    const GEOMETRY_EPSILON: f32 = 1e-5;
    const HALF_EXTENT_FACTOR: f32 = 0.5;
    const PANEL_HEIGHT: f32 = 1.0;
    const PANEL_WIDTH: f32 = 2.0;
    const RESIZED_PANEL_HEIGHT: f32 = 2.0;
    const RESIZED_PANEL_WIDTH: f32 = 4.0;
    const UNMAPPED_VERTEX: u32 = 99;

    #[test]
    fn spawned_world_panel_gets_local_quad_geometry() {
        let mut world = World::new();
        let panel = world.spawn(world_panel()).id();
        let mut schedule = provider_schedule();

        schedule.run(&mut world);

        let geometry = geometry(&world, panel);
        assert_eq!(geometry.frames().count(), EXPECTED_POINT_COUNT);
        assert_eq!(geometry.edges().len(), EXPECTED_EDGE_COUNT);
        assert_expected_positions(geometry, PANEL_WIDTH, PANEL_HEIGHT);
        assert_eq!(geometry.edges(), quad_edges().as_slice());
    }

    #[test]
    fn point_sized_panel_keeps_named_sites_without_degenerate_edges() {
        let geometry = write_geometry(Vec2::ZERO, Anchor::Center)
            .expect("a point-sized panel still has valid point attachment sites");

        assert_eq!(geometry.frames().count(), EXPECTED_POINT_COUNT);
        assert!(geometry.edges().is_empty());
    }

    #[test]
    fn resizing_panel_updates_existing_geometry() {
        let mut world = World::new();
        let panel = world.spawn(world_panel()).id();
        let mut schedule = provider_schedule();
        schedule.run(&mut world);

        let initial_keys = sorted_anchor_ids(geometry(&world, panel));
        world
            .get_mut::<DiegeticPanel>(panel)
            .expect("panel exists")
            .set_width(RESIZED_PANEL_WIDTH);
        world
            .get_mut::<DiegeticPanel>(panel)
            .expect("panel exists")
            .set_height(RESIZED_PANEL_HEIGHT);

        schedule.run(&mut world);

        let geometry = geometry(&world, panel);
        assert_eq!(sorted_anchor_ids(geometry), initial_keys);
        assert_expected_positions(geometry, RESIZED_PANEL_WIDTH, RESIZED_PANEL_HEIGHT);
    }

    #[test]
    fn anchor_site_mapping_round_trips_known_panel_anchors() {
        for anchor in all_anchors() {
            let anchor_id = AnchorSite::from(anchor);

            assert_eq!(Anchor::try_from(anchor_id), Ok(anchor));
        }

        let unmapped = AnchorSite::Vertex(UNMAPPED_VERTEX);
        assert_eq!(Anchor::try_from(unmapped), Err(unmapped));
    }

    fn provider_schedule() -> Schedule {
        let mut schedule = Schedule::default();
        schedule.add_systems(write_panel_anchor_geometry);
        schedule
    }

    fn world_panel() -> DiegeticPanel {
        DiegeticPanel::world()
            .size(PANEL_WIDTH, PANEL_HEIGHT)
            .anchor(Anchor::TopLeft)
            .layout(|_| {})
            .build()
            .expect("world panel builds")
    }

    fn geometry(world: &World, entity: Entity) -> &ResolvedAnchorGeometry {
        world
            .get::<ResolvedAnchorGeometry>(entity)
            .expect("panel has valence geometry")
    }

    fn sorted_anchor_ids(geometry: &ResolvedAnchorGeometry) -> Vec<AnchorSite> {
        let mut anchor_ids: Vec<_> = geometry.frames().map(|(site, _)| *site).collect();
        anchor_ids.sort_by_key(|anchor_id| anchor_sort_key(*anchor_id));
        anchor_ids
    }

    fn anchor_sort_key(anchor_id: AnchorSite) -> (u8, u32) {
        match anchor_id {
            AnchorSite::Vertex(index) => (0, index),
            AnchorSite::EdgeMidpoint(index) => (1, index),
            AnchorSite::Center => (2, 0),
            _ => (3, 0),
        }
    }

    fn assert_expected_positions(
        geometry: &ResolvedAnchorGeometry,
        expected_width: f32,
        expected_height: f32,
    ) {
        let cases = [
            (Anchor::TopLeft, Vec3::ZERO),
            (Anchor::TopRight, Vec3::new(expected_width, 0.0, 0.0)),
            (
                Anchor::BottomRight,
                Vec3::new(expected_width, -expected_height, 0.0),
            ),
            (Anchor::BottomLeft, Vec3::new(0.0, -expected_height, 0.0)),
            (
                Anchor::TopCenter,
                Vec3::new(expected_width * HALF_EXTENT_FACTOR, 0.0, 0.0),
            ),
            (
                Anchor::CenterRight,
                Vec3::new(expected_width, -expected_height * HALF_EXTENT_FACTOR, 0.0),
            ),
            (
                Anchor::BottomCenter,
                Vec3::new(expected_width * HALF_EXTENT_FACTOR, -expected_height, 0.0),
            ),
            (
                Anchor::CenterLeft,
                Vec3::new(0.0, -expected_height * HALF_EXTENT_FACTOR, 0.0),
            ),
            (
                Anchor::Center,
                Vec3::new(
                    expected_width * HALF_EXTENT_FACTOR,
                    -expected_height * HALF_EXTENT_FACTOR,
                    0.0,
                ),
            ),
        ];

        for (anchor, expected) in cases {
            let point = geometry
                .frame(AnchorSite::from(anchor))
                .expect("anchor point exists");
            assert!(
                point
                    .position()
                    .into_inner()
                    .abs_diff_eq(expected, GEOMETRY_EPSILON),
                "expected {expected:?}, got {:?}",
                point.position().into_inner(),
            );
        }
    }

    fn all_anchors() -> [Anchor; EXPECTED_POINT_COUNT] {
        [
            Anchor::TopLeft,
            Anchor::TopCenter,
            Anchor::TopRight,
            Anchor::CenterLeft,
            Anchor::Center,
            Anchor::CenterRight,
            Anchor::BottomLeft,
            Anchor::BottomCenter,
            Anchor::BottomRight,
        ]
    }
}
