use bevy_ecs::entity::Entity;
use bevy_math::Vec3;
use hana_kana::Angle;

use super::sheet;
use crate::AnchorSite;
use crate::AnchoredTo;
use crate::ArrangementConnection;
use crate::ArrangementError;
use crate::ArrangementMemberEntities;
use crate::ArrangementPlan;
use crate::ArrangementProvider;
use crate::Edge;
use crate::FoldGroups;
use crate::GeometryError;
use crate::HingeClearance;
use crate::Provides;
use crate::ResolvedAnchorGeometry;
use crate::WindingClearance;

/// Which way one [`TriangleCell`] points inside its row.
///
/// Cells alternate along a row and between rows so the sheet tiles without
/// gaps. An `Upward` cell carries its horizontal base edge at the bottom; a
/// `Downward` cell carries it at the top, which is why only a `Downward` cell
/// can attach to the row above it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriangleCellOrientation {
    /// Apex at the top, horizontal base edge at the bottom.
    Upward,
    /// Apex at the bottom, horizontal base edge at the top.
    Downward,
}

/// One logical cell of a [`TriangleSheet`], addressed by row and column.
///
/// Row and column are zero based. Row zero is the top row and column zero is
/// the leftmost column. Consecutive columns overlap by half a cell width, so
/// column `c` and column `c + 1` share one slanted edge.
/// `TriangleCell::new(0, 0)` is the sheet's first forest root.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TriangleCell {
    row:    usize,
    column: usize,
}

impl TriangleCell {
    /// Creates the logical cell at `row` and `column`.
    #[must_use]
    pub const fn new(row: usize, column: usize) -> Self { Self { row, column } }

    /// Returns this cell's zero-based row, counted downward from the top.
    #[must_use]
    pub const fn row(&self) -> usize { self.row }

    /// Returns this cell's zero-based column, counted rightward from the left.
    #[must_use]
    pub const fn column(&self) -> usize { self.column }

    /// Returns which way this cell points.
    ///
    /// A cell points [`TriangleCellOrientation::Upward`] when its row and
    /// column sum is even and [`TriangleCellOrientation::Downward`] when that
    /// sum is odd, which is the alternation a gap-free triangular tiling needs.
    #[must_use]
    pub const fn orientation(&self) -> TriangleCellOrientation {
        if (self.row + self.column).is_multiple_of(2) {
            TriangleCellOrientation::Upward
        } else {
            TriangleCellOrientation::Downward
        }
    }
}

/// The fold alternatives a [`TriangleSheet`] offers.
///
/// Both alternatives cover every non-root cell exactly once; they differ in
/// how those cells are gathered into groups. Group order runs outward from the
/// sheet's first forest root `TriangleCell::new(0, 0)`: group index zero is
/// the row or column that touches that root, and the last group is the far
/// edge of the sheet. Member order inside a group runs outward the same way,
/// so a recipe can treat position zero as the innermost crease and the last
/// position as the outermost one without a second index type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TriangleFoldGroupSelection {
    /// One group per row, each holding that row's hinged cells in column order.
    Rows,
    /// One group per column, each holding that column's hinged cells in row order.
    Columns,
}

impl TriangleFoldGroupSelection {
    /// Every alternative a [`TriangleSheet`] with at least one crease retains.
    const ALL: [Self; 2] = [Self::Rows, Self::Columns];
}

/// How one row of a [`TriangleSheet`] reaches the row above it.
///
/// A row either links upward through the one cell whose horizontal base edge
/// faces that row, or carries no such cell and roots its own subgraph. Every
/// other cell of the row chains sideways toward whichever of the two it has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TriangleRowLink {
    /// The cell at `link_column` attaches upward; the rest chain toward it.
    UpwardAt {
        /// Column of the row's leftmost downward-pointing cell.
        link_column: usize,
    },
    /// No cell attaches upward; column zero is a forest root and the rest of
    /// the row chains leftward from it.
    ForestRoot,
}

/// Which shared triangle edge seats one cell against the neighbor it attaches to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TriangleCreaseEdge {
    /// The cell's left slanted edge meets the previous column in the same row.
    Leftward,
    /// The cell's right slanted edge meets the next column in the same row.
    Rightward,
    /// The cell's horizontal base edge meets the same column in the row above.
    Upward,
}

impl TriangleCreaseEdge {
    /// Returns the provider edge index this crease uses on `orientation`.
    ///
    /// The neighbor always carries the opposite orientation and meets this cell
    /// from the opposite direction, so the neighbor's crease index is the same
    /// number: an upward cell's left slanted edge is index two, and so is the
    /// right slanted edge of the downward cell beside it. One lookup therefore
    /// serves both the source and the target site, and callers must pass the
    /// source's orientation for both.
    const fn edge_index(self, orientation: TriangleCellOrientation) -> usize {
        match (self, orientation) {
            (Self::Upward, _) => 1,
            (Self::Leftward, TriangleCellOrientation::Upward)
            | (Self::Rightward, TriangleCellOrientation::Downward) => 2,
            (Self::Leftward, TriangleCellOrientation::Downward)
            | (Self::Rightward, TriangleCellOrientation::Upward) => 0,
        }
    }
}

/// A rectangular triangle sheet with row and column fold alternatives.
///
/// The sheet enumerates its cells in row-major order. Every cell alternates
/// orientation with its neighbors, so a row is a strip of triangles sharing
/// slanted edges and a cell attaches upward only where its horizontal base
/// edge faces the row above. Row zero chains leftward from its root at
/// `TriangleCell::new(0, 0)`. Each later row attaches to the row above through
/// its leftmost downward-pointing cell — column zero on odd rows, column one
/// on even rows — and chains outward from that link in both directions. Each
/// cell's target comes from its own row and column, not from the enumeration
/// order.
///
/// One row or one column is a normal degenerate sheet and keeps the same
/// row/column selection vocabulary. A single-column sheet has no downward cell
/// on its even rows, so each even row below the first becomes its own forest
/// root; multiple roots and disconnected subgraphs are valid. A sheet with a
/// single cell, or with no cells at all, has no crease, so it retains no fold
/// alternative.
///
/// ```rust,ignore
/// let arrangement = commands.spawn_arrangement(TriangleSheet::new(2, 6), |cell| {
///     bsn! { TriangleTile { orientation: cell.orientation() } }
/// })?;
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TriangleSheet {
    rows:    usize,
    columns: usize,
}

impl TriangleSheet {
    /// Authored side length of one logical cell, in arrangement units.
    pub const CELL_SIDE: f32 = 1.0;
    /// Authored height of one logical cell, in arrangement units.
    pub const CELL_HEIGHT: f32 = 0.866_025_4;
    /// Authored thickness of one cell layer, in arrangement units.
    ///
    /// [`Provides<WindingClearance>`] gives every hinged cell exactly this much
    /// clearance along the cell normal, which is how thick one wrapped layer of
    /// this sheet is. Stacking those layers belongs to the wrapping recipe.
    pub const LAYER_THICKNESS: f32 = Self::CELL_SIDE / 100.0;
    /// Ordered cell edges, indexed the way [`TriangleCreaseEdge`] reports.
    const EDGES: [Edge; 3] = [
        Edge {
            start: AnchorSite::Vertex(0),
            end:   AnchorSite::Vertex(1),
        },
        Edge {
            start: AnchorSite::Vertex(1),
            end:   AnchorSite::Vertex(2),
        },
        Edge {
            start: AnchorSite::Vertex(2),
            end:   AnchorSite::Vertex(0),
        },
    ];
    /// Edge midpoint sites in the same order as [`Self::EDGES`].
    const MIDPOINTS: [AnchorSite; 3] = [
        AnchorSite::EdgeMidpoint(0),
        AnchorSite::EdgeMidpoint(1),
        AnchorSite::EdgeMidpoint(2),
    ];

    /// Creates a sheet of `rows` by `columns` unit triangle cells.
    #[must_use]
    pub const fn new(rows: usize, columns: usize) -> Self { Self { rows, columns } }

    /// Returns the number of cell rows this sheet enumerates.
    #[must_use]
    pub const fn rows(&self) -> usize { self.rows }

    /// Returns the number of cell columns this sheet enumerates.
    #[must_use]
    pub const fn columns(&self) -> usize { self.columns }

    /// Returns the local anchor geometry for one cell of this sheet.
    ///
    /// Geometry depends only on [`TriangleCell::orientation`]: a downward cell
    /// is the upward cell turned a half turn about its own `+Z` normal, so both
    /// orientations share that normal and their centroid origin. Vertices run
    /// `Vertex(0)` apex, then the other two clockwise from the apex, and
    /// `EdgeMidpoint(0..=2)` are the midpoints of edges `V0→V1`, `V1→V2`, and
    /// `V2→V0` in that order. Edge index one is always the horizontal base
    /// edge. [`AnchorSite::Center`] is the centroid. Iterating
    /// [`ResolvedAnchorGeometry::edges`] keeps the authored index order, while
    /// [`ResolvedAnchorGeometry::frames`] is an unordered map.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] when the constructed frames or edges are
    /// rejected, which a finite [`Self::CELL_SIDE`] cannot cause.
    pub fn cell_geometry(
        &self,
        cell: TriangleCell,
    ) -> Result<ResolvedAnchorGeometry, GeometryError> {
        let half_side = Self::CELL_SIDE / 2.0;
        let apex_reach = Self::CELL_HEIGHT * 2.0 / 3.0;
        let base_reach = Self::CELL_HEIGHT / 3.0;
        let vertices = match cell.orientation() {
            TriangleCellOrientation::Upward => [
                Vec3::new(0.0, apex_reach, 0.0),
                Vec3::new(half_side, -base_reach, 0.0),
                Vec3::new(-half_side, -base_reach, 0.0),
            ],
            TriangleCellOrientation::Downward => [
                Vec3::new(0.0, -apex_reach, 0.0),
                Vec3::new(-half_side, base_reach, 0.0),
                Vec3::new(half_side, base_reach, 0.0),
            ],
        };

        let mut frames = vec![
            (AnchorSite::Center, sheet::anchor_frame(Vec3::ZERO)?),
            (AnchorSite::Vertex(0), sheet::anchor_frame(vertices[0])?),
            (AnchorSite::Vertex(1), sheet::anchor_frame(vertices[1])?),
            (AnchorSite::Vertex(2), sheet::anchor_frame(vertices[2])?),
        ];
        for (index, midpoint) in Self::MIDPOINTS.into_iter().enumerate() {
            let start = vertices[index];
            let end = vertices[(index + 1) % vertices.len()];
            frames.push((midpoint, sheet::anchor_frame((start + end) / 2.0)?));
        }

        ResolvedAnchorGeometry::try_new(frames, Self::EDGES)
    }

    /// Returns how `row` reaches the row above it.
    ///
    /// Only a downward-pointing cell faces the row above with its horizontal
    /// base edge, and the leftmost downward cell of a row sits at column zero
    /// on odd rows and column one on even rows. Row zero has no row above it,
    /// and a sheet too narrow to hold that leftmost downward cell has no such
    /// cell to link through, so either row begins its own forest instead.
    const fn row_link(&self, row: usize) -> TriangleRowLink {
        if row == 0 {
            return TriangleRowLink::ForestRoot;
        }
        let link_column = 1 - row % 2;
        if link_column < self.columns {
            TriangleRowLink::UpwardAt { link_column }
        } else {
            TriangleRowLink::ForestRoot
        }
    }

    /// Authors this sheet's connection forest in row-major cell order.
    fn connections(
        &self,
        members: &ArrangementMemberEntities<TriangleCell>,
    ) -> Result<Vec<ArrangementConnection>, ArrangementError> {
        let mut connections = Vec::new();
        for row in 0..self.rows {
            let row_link = self.row_link(row);
            for column in 0..self.columns {
                let cell = TriangleCell::new(row, column);
                let (target, crease_edge) = match row_link {
                    TriangleRowLink::UpwardAt { link_column } if column == link_column => (
                        TriangleCell::new(row - 1, column),
                        TriangleCreaseEdge::Upward,
                    ),
                    TriangleRowLink::UpwardAt { link_column } if column < link_column => (
                        TriangleCell::new(row, column + 1),
                        TriangleCreaseEdge::Rightward,
                    ),
                    TriangleRowLink::ForestRoot if column == 0 => continue,
                    TriangleRowLink::UpwardAt { .. } | TriangleRowLink::ForestRoot => (
                        TriangleCell::new(row, column - 1),
                        TriangleCreaseEdge::Leftward,
                    ),
                };
                let edge_index = crease_edge.edge_index(cell.orientation());
                connections.push(ArrangementConnection {
                    member_entity:   members.entity(&cell)?,
                    anchored_to:     AnchoredTo::new(
                        members.entity(&target)?,
                        Self::MIDPOINTS[edge_index],
                        Self::MIDPOINTS[edge_index],
                    ),
                    member_edge:     Self::EDGES[edge_index],
                    base_angle:      Angle::default(),
                    hinge_clearance: HingeClearance::CENTERED,
                });
            }
        }

        Ok(connections)
    }

    /// Lists every cell of every row or column, outward from the first root.
    fn fold_lines(
        &self,
        selection: TriangleFoldGroupSelection,
        members: &ArrangementMemberEntities<TriangleCell>,
    ) -> Result<Vec<Vec<Entity>>, ArrangementError> {
        let (lines, cells_per_line) = match selection {
            TriangleFoldGroupSelection::Rows => (self.rows, self.columns),
            TriangleFoldGroupSelection::Columns => (self.columns, self.rows),
        };

        let mut fold_lines = Vec::with_capacity(lines);
        for line in 0..lines {
            let mut cells = Vec::with_capacity(cells_per_line);
            for position in 0..cells_per_line {
                let cell = match selection {
                    TriangleFoldGroupSelection::Rows => TriangleCell::new(line, position),
                    TriangleFoldGroupSelection::Columns => TriangleCell::new(position, line),
                };
                cells.push(members.entity(&cell)?);
            }
            fold_lines.push(cells);
        }

        Ok(fold_lines)
    }
}

impl ArrangementProvider for TriangleSheet {
    type FoldGroupSelection = TriangleFoldGroupSelection;
    type Member = TriangleCell;

    fn members(&self) -> impl Iterator<Item = Self::Member> {
        let columns = self.columns;
        (0..self.rows)
            .flat_map(move |row| (0..columns).map(move |column| TriangleCell::new(row, column)))
    }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let mut plan = ArrangementPlan::try_new(members, self.connections(members)?)?;
        if plan.connections().is_empty() {
            return Ok(plan);
        }

        for selection in TriangleFoldGroupSelection::ALL {
            let groups = sheet::fold_groups_from_lines(
                self.fold_lines(selection, members)?,
                plan.connections(),
            )?;
            let winding_clearance = self.provide(&selection, &groups, plan.connections())?;
            plan = plan
                .with_fold_groups(selection, groups)?
                .with_capability(selection, winding_clearance)?;
        }

        Ok(plan)
    }
}

impl Provides<WindingClearance> for TriangleSheet {
    fn provide(
        &self,
        _: &Self::FoldGroupSelection,
        groups: &FoldGroups,
        _: &[ArrangementConnection],
    ) -> Result<WindingClearance, ArrangementError> {
        sheet::wound_layer_clearance(groups, Self::LAYER_THICKNESS)
    }
}
