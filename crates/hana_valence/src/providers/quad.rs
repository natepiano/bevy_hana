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

/// One logical cell of a [`QuadSheet`], addressed by row and column.
///
/// Row and column are zero based. Row zero is the top row and column zero is
/// the leftmost column, matching the cell geometry the sheet publishes: `+X`
/// runs along a row and `-Y` runs down the rows. `QuadCell::new(0, 0)` is the
/// sheet's fixed root cell.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct QuadCell {
    row:    usize,
    column: usize,
}

impl QuadCell {
    /// Creates the logical cell at `row` and `column`.
    #[must_use]
    pub const fn new(row: usize, column: usize) -> Self { Self { row, column } }

    /// Returns this cell's zero-based row, counted downward from the top.
    #[must_use]
    pub const fn row(&self) -> usize { self.row }

    /// Returns this cell's zero-based column, counted rightward from the left.
    #[must_use]
    pub const fn column(&self) -> usize { self.column }
}

/// The fold alternatives a [`QuadSheet`] offers.
///
/// Both alternatives cover every non-root cell exactly once; they differ in
/// how those cells are gathered into groups. Group order runs outward from the
/// fixed root cell `QuadCell::new(0, 0)`: group index zero is the row or
/// column that touches the root, and the last group is the far edge of the
/// sheet. Member order inside a group runs outward the same way, so a recipe
/// can treat position zero as the innermost crease and the last position as
/// the outermost one without a second index type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum QuadFoldGroupSelection {
    /// One group per row, each holding that row's hinged cells in column order.
    Rows,
    /// One group per column, each holding that column's hinged cells in row order.
    Columns,
}

impl QuadFoldGroupSelection {
    /// Every alternative a [`QuadSheet`] with at least one crease retains.
    const ALL: [Self; 2] = [Self::Rows, Self::Columns];
}

/// Which shared quad edge seats one cell against the neighbor it attaches to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuadCreaseEdge {
    /// The cell's left edge meets the right edge of the previous column.
    Left,
    /// The cell's top edge meets the bottom edge of the previous row.
    Top,
}

impl QuadCreaseEdge {
    /// Returns the ordered source-local edge that becomes the hinge axis.
    const fn member_edge(self) -> Edge {
        match self {
            Self::Left => QuadSheet::LEFT_EDGE,
            Self::Top => QuadSheet::TOP_EDGE,
        }
    }

    /// Returns the attachment site on the cell that carries the connection.
    const fn source_anchor(self) -> AnchorSite {
        match self {
            Self::Left => QuadSheet::LEFT_MIDPOINT,
            Self::Top => QuadSheet::TOP_MIDPOINT,
        }
    }

    /// Returns the facing attachment site on the neighbor being attached to.
    const fn target_anchor(self) -> AnchorSite {
        match self {
            Self::Left => QuadSheet::RIGHT_MIDPOINT,
            Self::Top => QuadSheet::BOTTOM_MIDPOINT,
        }
    }
}

/// A rectangular sheet of unit quad cells with row and column fold alternatives.
///
/// The sheet enumerates its cells in row-major order and authors one
/// connection forest rooted at `QuadCell::new(0, 0)`. Row zero chains leftward
/// along its shared vertical edges, and every cell below row zero attaches to
/// the cell directly above it through their shared horizontal edge. No cell
/// assumes it follows the previously enumerated member, so a sheet wider than
/// one column is a branching forest rather than a chain.
///
/// One row or one column is a normal degenerate sheet: a one-row sheet is a
/// horizontal strip whose creases are all vertical, and a one-column sheet is
/// a vertical strip whose creases are all horizontal. Both keep the same
/// row/column selection vocabulary. A sheet with a single cell, or with no
/// cells at all, has no crease, so it retains no fold alternative.
///
/// ```rust,ignore
/// let arrangement = commands.spawn_arrangement(QuadSheet::new(3, 4), |cell| {
///     bsn! { QuadPanel { row: cell.row(), column: cell.column() } }
/// })?;
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuadSheet {
    rows:    usize,
    columns: usize,
}

impl QuadSheet {
    /// Authored edge length of one logical cell, in arrangement units.
    pub const CELL_SIZE: f32 = 1.0;
    /// Authored thickness of one cell layer, in arrangement units.
    ///
    /// [`Provides<WindingClearance>`] gives every hinged cell exactly this much
    /// clearance along the cell normal, which is how thick one wrapped layer of
    /// this sheet is. Stacking those layers belongs to the wrapping recipe.
    pub const LAYER_THICKNESS: f32 = Self::CELL_SIZE / 100.0;
    const BOTTOM_EDGE: Edge = Edge {
        start: AnchorSite::Vertex(2),
        end:   AnchorSite::Vertex(3),
    };
    const BOTTOM_MIDPOINT: AnchorSite = AnchorSite::EdgeMidpoint(2);
    const LEFT_EDGE: Edge = Edge {
        start: AnchorSite::Vertex(3),
        end:   AnchorSite::Vertex(0),
    };
    const LEFT_MIDPOINT: AnchorSite = AnchorSite::EdgeMidpoint(3);
    const RIGHT_EDGE: Edge = Edge {
        start: AnchorSite::Vertex(1),
        end:   AnchorSite::Vertex(2),
    };
    const RIGHT_MIDPOINT: AnchorSite = AnchorSite::EdgeMidpoint(1);
    const TOP_EDGE: Edge = Edge {
        start: AnchorSite::Vertex(0),
        end:   AnchorSite::Vertex(1),
    };
    const TOP_MIDPOINT: AnchorSite = AnchorSite::EdgeMidpoint(0);

    /// Creates a sheet of `rows` by `columns` unit quad cells.
    #[must_use]
    pub const fn new(rows: usize, columns: usize) -> Self { Self { rows, columns } }

    /// Returns the number of cell rows this sheet enumerates.
    #[must_use]
    pub const fn rows(&self) -> usize { self.rows }

    /// Returns the number of cell columns this sheet enumerates.
    #[must_use]
    pub const fn columns(&self) -> usize { self.columns }

    /// Returns the local anchor geometry every cell of this sheet shares.
    ///
    /// Every quad cell is identical, so one value serves every member scene.
    /// The cell is a [`Self::CELL_SIZE`] square centered on its own origin in
    /// the XY plane, with `+Z` as its outward normal. Vertices run clockwise
    /// from the top-left corner — `Vertex(0)` top-left, `Vertex(1)` top-right,
    /// `Vertex(2)` bottom-right, `Vertex(3)` bottom-left — and
    /// `EdgeMidpoint(0..=3)` are the top, right, bottom, and left midpoints in
    /// that order. [`AnchorSite::Center`] is the cell origin. The four edges
    /// are authored top, right, bottom, then left; iterating
    /// [`ResolvedAnchorGeometry::edges`] keeps that order, while
    /// [`ResolvedAnchorGeometry::frames`] is an unordered map.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] when the constructed frames or edges are
    /// rejected, which a finite [`Self::CELL_SIZE`] cannot cause.
    pub fn cell_geometry(&self) -> Result<ResolvedAnchorGeometry, GeometryError> {
        let half = Self::CELL_SIZE / 2.0;
        let top_left = Vec3::new(-half, half, 0.0);
        let top_right = Vec3::new(half, half, 0.0);
        let bottom_right = Vec3::new(half, -half, 0.0);
        let bottom_left = Vec3::new(-half, -half, 0.0);

        ResolvedAnchorGeometry::try_new(
            [
                (AnchorSite::Center, sheet::anchor_frame(Vec3::ZERO)?),
                (Self::TOP_EDGE.start, sheet::anchor_frame(top_left)?),
                (Self::TOP_EDGE.end, sheet::anchor_frame(top_right)?),
                (Self::BOTTOM_EDGE.start, sheet::anchor_frame(bottom_right)?),
                (Self::BOTTOM_EDGE.end, sheet::anchor_frame(bottom_left)?),
                (
                    Self::TOP_MIDPOINT,
                    sheet::anchor_frame((top_left + top_right) / 2.0)?,
                ),
                (
                    Self::RIGHT_MIDPOINT,
                    sheet::anchor_frame((top_right + bottom_right) / 2.0)?,
                ),
                (
                    Self::BOTTOM_MIDPOINT,
                    sheet::anchor_frame((bottom_right + bottom_left) / 2.0)?,
                ),
                (
                    Self::LEFT_MIDPOINT,
                    sheet::anchor_frame((bottom_left + top_left) / 2.0)?,
                ),
            ],
            [
                Self::TOP_EDGE,
                Self::RIGHT_EDGE,
                Self::BOTTOM_EDGE,
                Self::LEFT_EDGE,
            ],
        )
    }

    /// Authors this sheet's connection forest in row-major cell order.
    fn connections(
        &self,
        members: &ArrangementMemberEntities<QuadCell>,
    ) -> Result<Vec<ArrangementConnection>, ArrangementError> {
        let mut connections = Vec::new();
        for row in 0..self.rows {
            for column in 0..self.columns {
                let (target, crease_edge) = if row == 0 {
                    if column == 0 {
                        continue;
                    }
                    (QuadCell::new(row, column - 1), QuadCreaseEdge::Left)
                } else {
                    (QuadCell::new(row - 1, column), QuadCreaseEdge::Top)
                };
                let member_entity = members.entity(&QuadCell::new(row, column))?;
                let target_entity = members.entity(&target)?;
                connections.push(ArrangementConnection {
                    member_entity,
                    anchored_to: AnchoredTo::new(
                        target_entity,
                        crease_edge.source_anchor(),
                        crease_edge.target_anchor(),
                    ),
                    member_edge: crease_edge.member_edge(),
                    base_angle: Angle::default(),
                    hinge_clearance: HingeClearance::CENTERED,
                });
            }
        }

        Ok(connections)
    }

    /// Lists every cell of every row or column, outward from the root cell.
    fn fold_lines(
        &self,
        selection: QuadFoldGroupSelection,
        members: &ArrangementMemberEntities<QuadCell>,
    ) -> Result<Vec<Vec<Entity>>, ArrangementError> {
        let (lines, cells_per_line) = match selection {
            QuadFoldGroupSelection::Rows => (self.rows, self.columns),
            QuadFoldGroupSelection::Columns => (self.columns, self.rows),
        };

        let mut fold_lines = Vec::with_capacity(lines);
        for line in 0..lines {
            let mut cells = Vec::with_capacity(cells_per_line);
            for cell in 0..cells_per_line {
                let cell = match selection {
                    QuadFoldGroupSelection::Rows => QuadCell::new(line, cell),
                    QuadFoldGroupSelection::Columns => QuadCell::new(cell, line),
                };
                cells.push(members.entity(&cell)?);
            }
            fold_lines.push(cells);
        }

        Ok(fold_lines)
    }
}

impl ArrangementProvider for QuadSheet {
    type FoldGroupSelection = QuadFoldGroupSelection;
    type Member = QuadCell;

    fn members(&self) -> impl Iterator<Item = Self::Member> {
        let columns = self.columns;
        (0..self.rows)
            .flat_map(move |row| (0..columns).map(move |column| QuadCell::new(row, column)))
    }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let mut plan = ArrangementPlan::try_new(members, self.connections(members)?)?;
        if plan.connections().is_empty() {
            return Ok(plan);
        }

        for selection in QuadFoldGroupSelection::ALL {
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

impl Provides<WindingClearance> for QuadSheet {
    fn provide(
        &self,
        _: &Self::FoldGroupSelection,
        groups: &FoldGroups,
        _: &[ArrangementConnection],
    ) -> Result<WindingClearance, ArrangementError> {
        sheet::wound_layer_clearance(groups, Self::LAYER_THICKNESS)
    }
}
