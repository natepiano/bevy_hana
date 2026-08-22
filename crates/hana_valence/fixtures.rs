//! Shared validated geometry fixtures for `hana_valence` examples and tests.
//!
//! The module is included by examples, integration tests, and resolver unit
//! tests so those consumers all construct the same provider-defined sites.

use bevy_math::Vec3;
use hana_valence::AnchorFrame;
use hana_valence::AnchorSite;
use hana_valence::Edge;
use hana_valence::Orientation;
use hana_valence::Position;
use hana_valence::ResolvedAnchorGeometry;

const TRIANGLE_HEIGHT: f32 = 0.866_025_4;
const TRIANGLE_HALF_SIDE: f32 = 0.5;
const TRIANGLE_TWO_THIRDS: f32 = 2.0 / 3.0;

/// Top edge of [`quad_geometry`].
pub const QUAD_TOP_EDGE: Edge = Edge {
    start: AnchorSite::Vertex(0),
    end:   AnchorSite::Vertex(1),
};

/// Right edge of [`quad_geometry`].
pub const QUAD_RIGHT_EDGE: Edge = Edge {
    start: AnchorSite::Vertex(1),
    end:   AnchorSite::Vertex(2),
};

/// Bottom edge of [`quad_geometry`].
pub const QUAD_BOTTOM_EDGE: Edge = Edge {
    start: AnchorSite::Vertex(2),
    end:   AnchorSite::Vertex(3),
};

/// Left edge of [`quad_geometry`].
pub const QUAD_LEFT_EDGE: Edge = Edge {
    start: AnchorSite::Vertex(3),
    end:   AnchorSite::Vertex(0),
};

/// Right edge of [`triangle_geometry`].
const TRIANGLE_RIGHT_EDGE: Edge = Edge {
    start: AnchorSite::Vertex(0),
    end:   AnchorSite::Vertex(1),
};

/// Bottom edge of [`triangle_geometry`].
const TRIANGLE_BOTTOM_EDGE: Edge = Edge {
    start: AnchorSite::Vertex(1),
    end:   AnchorSite::Vertex(2),
};

/// Left edge of [`triangle_geometry`].
const TRIANGLE_LEFT_EDGE: Edge = Edge {
    start: AnchorSite::Vertex(2),
    end:   AnchorSite::Vertex(0),
};

/// Equilateral triangle anchor geometry with unit side length.
#[must_use]
pub fn triangle_geometry() -> ResolvedAnchorGeometry {
    let vertices = triangle_vertices();
    valid_geometry(ResolvedAnchorGeometry::try_new(
        [
            anchor_frame(AnchorSite::Vertex(0), vertices[0]),
            anchor_frame(AnchorSite::Vertex(1), vertices[1]),
            anchor_frame(AnchorSite::Vertex(2), vertices[2]),
            anchor_frame(
                AnchorSite::EdgeMidpoint(0),
                edge_midpoint(TRIANGLE_RIGHT_EDGE, vertices),
            ),
            anchor_frame(
                AnchorSite::EdgeMidpoint(1),
                edge_midpoint(TRIANGLE_BOTTOM_EDGE, vertices),
            ),
            anchor_frame(
                AnchorSite::EdgeMidpoint(2),
                edge_midpoint(TRIANGLE_LEFT_EDGE, vertices),
            ),
            anchor_frame(AnchorSite::Center, Vec3::ZERO),
        ],
        [
            TRIANGLE_RIGHT_EDGE,
            TRIANGLE_BOTTOM_EDGE,
            TRIANGLE_LEFT_EDGE,
        ],
    ))
}

/// Axis-aligned rectangle anchor geometry centered at the origin in the XY plane.
#[must_use]
pub fn quad_geometry(width: f32, height: f32) -> ResolvedAnchorGeometry {
    let vertices = quad_vertices(width, height);
    valid_geometry(ResolvedAnchorGeometry::try_new(
        [
            anchor_frame(AnchorSite::Vertex(0), vertices[0]),
            anchor_frame(AnchorSite::Vertex(1), vertices[1]),
            anchor_frame(AnchorSite::Vertex(2), vertices[2]),
            anchor_frame(AnchorSite::Vertex(3), vertices[3]),
            anchor_frame(
                AnchorSite::EdgeMidpoint(0),
                edge_midpoint(QUAD_TOP_EDGE, vertices),
            ),
            anchor_frame(
                AnchorSite::EdgeMidpoint(1),
                edge_midpoint(QUAD_RIGHT_EDGE, vertices),
            ),
            anchor_frame(
                AnchorSite::EdgeMidpoint(2),
                edge_midpoint(QUAD_BOTTOM_EDGE, vertices),
            ),
            anchor_frame(
                AnchorSite::EdgeMidpoint(3),
                edge_midpoint(QUAD_LEFT_EDGE, vertices),
            ),
            anchor_frame(AnchorSite::Center, Vec3::ZERO),
        ],
        [
            QUAD_TOP_EDGE,
            QUAD_RIGHT_EDGE,
            QUAD_BOTTOM_EDGE,
            QUAD_LEFT_EDGE,
        ],
    ))
}

/// Returns the midpoint site for a quad edge.
#[must_use]
pub fn quad_edge_anchor(edge: Edge) -> Option<AnchorSite> {
    if edge == QUAD_TOP_EDGE {
        Some(AnchorSite::EdgeMidpoint(0))
    } else if edge == QUAD_RIGHT_EDGE {
        Some(AnchorSite::EdgeMidpoint(1))
    } else if edge == QUAD_BOTTOM_EDGE {
        Some(AnchorSite::EdgeMidpoint(2))
    } else if edge == QUAD_LEFT_EDGE {
        Some(AnchorSite::EdgeMidpoint(3))
    } else {
        None
    }
}

/// Returns the midpoint site for a triangle edge.
#[must_use]
pub fn triangle_edge_anchor(edge: Edge) -> Option<AnchorSite> {
    if edge == TRIANGLE_RIGHT_EDGE {
        Some(AnchorSite::EdgeMidpoint(0))
    } else if edge == TRIANGLE_BOTTOM_EDGE {
        Some(AnchorSite::EdgeMidpoint(1))
    } else if edge == TRIANGLE_LEFT_EDGE {
        Some(AnchorSite::EdgeMidpoint(2))
    } else {
        None
    }
}

/// Seating edge for member `index` in a straight triangle strip.
#[must_use]
pub const fn triangle_edge(index: usize) -> Edge {
    match index % 3 {
        1 => TRIANGLE_BOTTOM_EDGE,
        2 => TRIANGLE_RIGHT_EDGE,
        _ => TRIANGLE_LEFT_EDGE,
    }
}

fn anchor_frame(site: AnchorSite, position: Vec3) -> (AnchorSite, AnchorFrame) {
    (
        site,
        valid_frame(AnchorFrame::try_new(
            Position::from(position),
            Orientation::default(),
        )),
    )
}

fn valid_frame<E>(result: Result<AnchorFrame, E>) -> AnchorFrame {
    assert!(result.is_ok(), "fixture position must be finite");
    result.unwrap_or_default()
}

fn valid_geometry<E>(result: Result<ResolvedAnchorGeometry, E>) -> ResolvedAnchorGeometry {
    assert!(result.is_ok(), "fixed fixture geometry must be valid");
    result.unwrap_or_default()
}

const fn triangle_vertices() -> [Vec3; 3] {
    [
        Vec3::new(0.0, TRIANGLE_HEIGHT * TRIANGLE_TWO_THIRDS, 0.0),
        Vec3::new(TRIANGLE_HALF_SIDE, -TRIANGLE_HEIGHT / 3.0, 0.0),
        Vec3::new(-TRIANGLE_HALF_SIDE, -TRIANGLE_HEIGHT / 3.0, 0.0),
    ]
}

const fn quad_vertices(width: f32, height: f32) -> [Vec3; 4] {
    let half_width = width / 2.0;
    let half_height = height / 2.0;
    [
        Vec3::new(-half_width, half_height, 0.0),
        Vec3::new(half_width, half_height, 0.0),
        Vec3::new(half_width, -half_height, 0.0),
        Vec3::new(-half_width, -half_height, 0.0),
    ]
}

fn edge_midpoint<const N: usize>(edge: Edge, vertices: [Vec3; N]) -> Vec3 {
    let Some(start) = vertex_position(edge.start, vertices) else {
        return Vec3::ZERO;
    };
    let Some(end) = vertex_position(edge.end, vertices) else {
        return Vec3::ZERO;
    };
    (start + end) / 2.0
}

fn vertex_position<const N: usize>(site: AnchorSite, vertices: [Vec3; N]) -> Option<Vec3> {
    match site {
        AnchorSite::Vertex(index) => usize::try_from(index)
            .ok()
            .and_then(|index| vertices.get(index).copied()),
        _ => None,
    }
}
