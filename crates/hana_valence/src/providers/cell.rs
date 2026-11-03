//! Sparse square cells joined into clearable folding strips.

use core::cmp::Reverse;

use bevy_math::Vec3;
use bevy_platform::collections::HashMap;
use hana_kana::Angle;
use hana_kana::Displacement;
use thiserror::Error;

use super::quad;
use crate::AnchorSite;
use crate::AnchoredTo;
use crate::ArrangementConnection;
use crate::ArrangementError;
use crate::ArrangementMemberEntities;
use crate::ArrangementPlan;
use crate::ArrangementProvider;
use crate::Edge;
use crate::FoldGroup;
use crate::FoldGroups;
use crate::FoldStackPivotClearance;
use crate::GeometryError;
use crate::HingeClearance;
use crate::Provides;
use crate::QuadCell;
use crate::ResolvedAnchorGeometry;
use crate::fold;

/// Gap between two layers in a folded stack, as a share of the cell pitch.
const FOLD_STACK_LAYER_GAP_SHARE: f32 = 1.0 / 2000.0;

/// The fold alternative retained by a [`CellSheet`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CellFoldGroupSelection {
    /// Each connected piece follows its row-running strip from root to tips.
    Strips,
}

/// Invalid input to [`CellSheet::try_new`].
#[derive(Clone, Copy, Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum CellSheetError {
    /// The cell pitch was zero, negative, NaN, or infinite.
    #[error("cell size must be finite and positive, got {cell_size}")]
    InvalidCellSize {
        /// Rejected cell pitch.
        cell_size: f32,
    },
    /// One logical location occurred more than once.
    #[error("cell {cell:?} occurs more than once")]
    DuplicateCell {
        /// Repeated logical cell.
        cell: QuadCell,
    },
    /// A cell row cannot name the adjacent row below it.
    #[error("cell {cell:?} has no representable successor row")]
    RowSuccessorOverflow {
        /// Cell whose row is `usize::MAX`.
        cell: QuadCell,
    },
    /// A cell column cannot name the adjacent column to its right.
    #[error("cell {cell:?} has no representable successor column")]
    ColumnSuccessorOverflow {
        /// Cell whose column is `usize::MAX`.
        cell: QuadCell,
    },
    /// A hinged cell received no strip-fold pivot assignment.
    #[error("cell {cell:?} has no strip-fold pivot clearance")]
    MissingStripFoldPivotClearance {
        /// Hinged cell without a pivot assignment.
        cell: QuadCell,
    },
    /// A hinged cell received more than one strip-fold pivot assignment.
    #[error("cell {cell:?} received strip-fold pivot clearance twice")]
    DuplicateStripFoldPivotClearance {
        /// Hinged cell with repeated pivot assignments.
        cell: QuadCell,
    },
    /// A strip-fold pivot assignment named a root or an absent cell.
    #[error("strip-fold pivot clearance names foreign cell {cell:?}")]
    ForeignStripFoldPivotClearance {
        /// Cell outside the sheet's hinged members.
        cell: QuadCell,
    },
    /// Stack simulation produced a pivot that cannot be represented.
    #[error("cell {cell:?} needs a non-finite strip-fold pivot clearance")]
    NonFiniteStripFoldPivotClearance {
        /// Cell whose computed pivot was invalid.
        cell: QuadCell,
    },
    /// Adding the strip half-turn to a cell's base angle overflowed.
    #[error("cell {cell:?} has an unrepresentable strip-fold endpoint")]
    UnrepresentableStripFoldEndpoint {
        /// Cell whose folded endpoint was invalid.
        cell: QuadCell,
    },
}

/// A sparse collection of square cells connected into row-running strips.
///
/// Cells are canonicalized into row-major order. Every edge-connected piece
/// gets one root at its top-left cell. A strip runs to both ends of its row,
/// enters each adjacent run at the shared column nearest the preceding strip
/// end, and exposes each piece's depth-first unfold order as one fold group.
/// Disconnected pieces therefore form an ordinary arrangement forest.
#[derive(Clone, Debug, PartialEq)]
pub struct CellSheet {
    cells:     Vec<QuadCell>,
    cell_size: f32,
    strip:     StripTree,
}

impl CellSheet {
    /// Builds a sparse cell sheet at the given finite, positive pitch.
    ///
    /// # Errors
    ///
    /// Returns [`CellSheetError::InvalidCellSize`] for a zero, negative, NaN,
    /// or infinite pitch, [`CellSheetError::DuplicateCell`] when one logical
    /// location appears twice, a successor-overflow variant when a coordinate
    /// cannot name its adjacent cell, or a strip-fold variant when the complete
    /// stack recipe cannot represent exactly one finite pivot and folded
    /// endpoint per hinged member.
    pub fn try_new(
        cells: impl IntoIterator<Item = QuadCell>,
        cell_size: f32,
    ) -> Result<Self, CellSheetError> {
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return Err(CellSheetError::InvalidCellSize { cell_size });
        }

        let mut cells = cells.into_iter().collect::<Vec<_>>();
        cells.sort_unstable_by_key(|cell| (cell.row(), cell.column()));
        if let Some(cell) = cells
            .windows(2)
            .find_map(|pair| (pair[0] == pair[1]).then_some(pair[0]))
        {
            return Err(CellSheetError::DuplicateCell { cell });
        }
        if let Some(cell) = cells.iter().find(|cell| cell.row() == usize::MAX) {
            return Err(CellSheetError::RowSuccessorOverflow { cell: *cell });
        }
        if let Some(cell) = cells.iter().find(|cell| cell.column() == usize::MAX) {
            return Err(CellSheetError::ColumnSuccessorOverflow { cell: *cell });
        }

        let strip = StripTree::build(&cells, cell_size)?;
        Ok(Self {
            cells,
            cell_size,
            strip,
        })
    }

    /// Returns the canonical row-major cells this provider covers.
    #[must_use]
    pub fn cells(&self) -> &[QuadCell] { &self.cells }

    /// Returns the finite, positive pitch between adjacent cell centres.
    #[must_use]
    pub const fn cell_size(&self) -> f32 { self.cell_size }

    /// Returns whether the sheet contains at least one hinge.
    #[must_use]
    pub fn has_hinges(&self) -> bool { self.strip.links.iter().any(CellLink::is_hinged) }

    /// Returns the local anchor geometry every cell shares.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] if the constructor-validated pitch cannot be
    /// represented by the common quad geometry contract.
    pub fn cell_geometry(&self) -> Result<ResolvedAnchorGeometry, GeometryError> {
        quad::sized_cell_geometry(self.cell_size)
    }

    fn connections(
        &self,
        members: &ArrangementMemberEntities<QuadCell>,
    ) -> Result<Vec<ArrangementConnection>, ArrangementError> {
        self.cells
            .iter()
            .enumerate()
            .filter_map(|(cell_index, cell)| {
                let CellLink::Hinged { parent, side } = self.strip.links[cell_index] else {
                    return None;
                };
                Some((cell, parent, side))
            })
            .map(|(cell, parent, side)| {
                let (source_anchor, target_anchor, member_edge) = side.joint();
                Ok(ArrangementConnection {
                    member_entity: members.entity(cell)?,
                    anchored_to: AnchoredTo::new(
                        members.entity(&self.cells[parent])?,
                        source_anchor,
                        target_anchor,
                    ),
                    member_edge,
                    base_angle: Angle::default(),
                    hinge_clearance: HingeClearance::CENTERED,
                })
            })
            .collect()
    }

    fn fold_groups(
        &self,
        members: &ArrangementMemberEntities<QuadCell>,
    ) -> Result<FoldGroups, ArrangementError> {
        let groups = self
            .strip
            .pieces
            .iter()
            .filter(|piece| !piece.is_empty())
            .map(|piece| {
                FoldGroup::try_from_iter(
                    piece
                        .iter()
                        .map(|cell_index| members.entity(&self.cells[*cell_index]))
                        .collect::<Result<Vec<_>, _>>()?,
                )
                .map_err(ArrangementError::provider)
            })
            .collect::<Result<Vec<_>, _>>()?;
        FoldGroups::try_from_iter(groups).map_err(ArrangementError::provider)
    }
}

impl ArrangementProvider for CellSheet {
    type FoldGroupSelection = CellFoldGroupSelection;
    type Member = QuadCell;

    fn members(&self) -> impl Iterator<Item = Self::Member> { self.cells.iter().copied() }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let mut plan = ArrangementPlan::try_new(members, self.connections(members)?)?;
        if !self.has_hinges() {
            return Ok(plan);
        }

        let selection = CellFoldGroupSelection::Strips;
        let groups = self.fold_groups(members)?;
        let clearance = self.provide(&selection, &groups, plan.connections())?;
        plan = plan
            .with_fold_groups(selection, groups)?
            .with_capability(selection, clearance)?;
        Ok(plan)
    }
}

impl Provides<FoldStackPivotClearance> for CellSheet {
    fn provide(
        &self,
        _: &Self::FoldGroupSelection,
        groups: &FoldGroups,
        _: &[ArrangementConnection],
    ) -> Result<FoldStackPivotClearance, ArrangementError> {
        let mut pivots = Vec::new();
        for (group_position, piece) in self
            .strip
            .pieces
            .iter()
            .filter(|piece| !piece.is_empty())
            .enumerate()
        {
            let Some(group) = groups.iter().nth(group_position) else {
                let cell = self.cells[piece[0]];
                return Err(ArrangementError::provider(
                    CellSheetError::MissingStripFoldPivotClearance { cell },
                ));
            };
            for (member_index, cell_index) in piece.iter().copied().enumerate() {
                let cell = self.cells[cell_index];
                let Some(member_entity) = group.iter().nth(member_index).copied() else {
                    return Err(ArrangementError::provider(
                        CellSheetError::MissingStripFoldPivotClearance { cell },
                    ));
                };
                let pivot = self
                    .strip
                    .stack_pivots
                    .pivot_for(cell)
                    .map_err(ArrangementError::provider)?;
                pivots.push((member_entity, pivot.direction, pivot.displacement));
            }
        }
        FoldStackPivotClearance::try_new(groups, pivots).map_err(ArrangementError::provider)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StackFoldDirection {
    Positive,
    Negative,
}

impl StackFoldDirection {
    const fn from_face(face: StackFace) -> Self {
        match face {
            StackFace::Front => Self::Negative,
            StackFace::Back => Self::Positive,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StackFace {
    Front,
    Back,
}

impl StackFace {
    const fn opposite(self) -> Self {
        match self {
            Self::Front => Self::Back,
            Self::Back => Self::Front,
        }
    }

    const fn sign(self) -> f32 {
        match self {
            Self::Front => 1.0,
            Self::Back => -1.0,
        }
    }

    const fn slot(self) -> usize {
        match self {
            Self::Front => 0,
            Self::Back => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParentSide {
    Left,
    Right,
    Above,
    Below,
}

impl ParentSide {
    const fn joint(self) -> (AnchorSite, AnchorSite, Edge) {
        let (source, target, start) = match self {
            Self::Above => (0, 2, 0),
            Self::Right => (1, 3, 1),
            Self::Below => (2, 0, 2),
            Self::Left => (3, 1, 3),
        };
        (
            AnchorSite::EdgeMidpoint(source),
            AnchorSite::EdgeMidpoint(target),
            Edge {
                start: AnchorSite::Vertex(start),
                end:   AnchorSite::Vertex((start + 1) % 4),
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellLink {
    Root,
    Hinged { parent: usize, side: ParentSide },
}

impl CellLink {
    const fn is_hinged(&self) -> bool { matches!(self, Self::Hinged { .. }) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StackPivot {
    direction:    StackFoldDirection,
    displacement: Displacement,
}

#[derive(Clone, Debug, PartialEq)]
struct StripTree {
    links:        Vec<CellLink>,
    pieces:       Vec<Vec<usize>>,
    stack_pivots: StackPivotAssignments,
}

#[derive(Clone, Debug, PartialEq)]
struct StackPivotAssignments {
    by_cell: HashMap<QuadCell, StackPivot>,
}

impl StackPivotAssignments {
    fn try_new(
        cells: &[QuadCell],
        links: &[CellLink],
        assignments: impl IntoIterator<Item = (QuadCell, StackPivot)>,
    ) -> Result<Self, CellSheetError> {
        let mut by_cell = HashMap::default();
        for (cell, pivot) in assignments {
            if by_cell.contains_key(&cell) {
                return Err(CellSheetError::DuplicateStripFoldPivotClearance { cell });
            }
            let Some(cell_index) = cells.iter().position(|listed| *listed == cell) else {
                return Err(CellSheetError::ForeignStripFoldPivotClearance { cell });
            };
            if matches!(links[cell_index], CellLink::Root) {
                return Err(CellSheetError::ForeignStripFoldPivotClearance { cell });
            }
            if !pivot.displacement.is_finite() {
                return Err(CellSheetError::NonFiniteStripFoldPivotClearance { cell });
            }
            if fold::strip_fold_endpoint(Angle::default(), pivot.direction).is_err() {
                return Err(CellSheetError::UnrepresentableStripFoldEndpoint { cell });
            }
            by_cell.insert(cell, pivot);
        }
        for (cell, link) in cells.iter().copied().zip(links) {
            if link.is_hinged() && !by_cell.contains_key(&cell) {
                return Err(CellSheetError::MissingStripFoldPivotClearance { cell });
            }
        }
        Ok(Self { by_cell })
    }

    fn pivot_for(&self, cell: QuadCell) -> Result<StackPivot, CellSheetError> {
        self.by_cell
            .get(&cell)
            .copied()
            .ok_or(CellSheetError::MissingStripFoldPivotClearance { cell })
    }
}

impl StripTree {
    fn build(cells: &[QuadCell], cell_size: f32) -> Result<Self, CellSheetError> {
        let (links, strip_order) = strip_links(cells);
        let mut children = vec![Vec::<usize>::new(); cells.len()];
        for (cell, link) in links.iter().enumerate() {
            if let CellLink::Hinged { parent, .. } = *link {
                children[parent].push(cell);
            }
        }

        let mut sizes = vec![1_usize; cells.len()];
        for &cell in strip_order.iter().rev() {
            for &child in &children[cell] {
                sizes[cell] = sizes[cell].saturating_add(sizes[child]);
            }
        }

        let mut faces = vec![StackFace::Back; cells.len()];
        for &cell in &strip_order {
            children[cell].sort_by_key(|&child| Reverse(sizes[child]));
            let near = faces[cell];
            for (rank, &child) in children[cell].iter().enumerate() {
                faces[child] = if rank.is_multiple_of(2) {
                    near.opposite()
                } else {
                    near
                };
            }
        }

        let mut stacks = vec![[0.0_f32; 2]; cells.len()];
        let mut stack_pivots = Vec::with_capacity(cells.len().saturating_sub(1));
        for &cell in strip_order.iter().rev() {
            for &child in children[cell].iter().rev() {
                let face = faces[child];
                let carried_near = stacks[child][face.slot()];
                let carried_far = stacks[child][face.opposite().slot()];
                let clearance =
                    stacks[cell][face.slot()].max(carried_near) + FOLD_STACK_LAYER_GAP_SHARE / 2.0;
                let displacement =
                    Displacement::from(Vec3::Z * (face.sign() * clearance * cell_size));
                stack_pivots.push((
                    cells[child],
                    StackPivot {
                        direction: StackFoldDirection::from_face(face),
                        displacement,
                    },
                ));
                stacks[cell][face.slot()] = 2.0f32.mul_add(clearance, carried_far);
            }
        }
        let stack_pivots = StackPivotAssignments::try_new(cells, &links, stack_pivots)?;

        let pieces = strip_order
            .iter()
            .copied()
            .filter(|cell| matches!(links[*cell], CellLink::Root))
            .map(|root| depth_first_hinges(root, &children, &links))
            .collect();
        Ok(Self {
            links,
            pieces,
            stack_pivots,
        })
    }
}

fn depth_first_hinges(root: usize, children: &[Vec<usize>], links: &[CellLink]) -> Vec<usize> {
    let mut order = Vec::new();
    let mut pending = vec![root];
    while let Some(cell) = pending.pop() {
        if matches!(links[cell], CellLink::Hinged { .. }) {
            order.push(cell);
        }
        pending.extend(children[cell].iter().rev());
    }
    order
}

#[derive(Clone, Copy, Debug)]
struct Run {
    row:   usize,
    first: usize,
    last:  usize,
}

impl Run {
    fn strip_ends(self, entry: usize) -> impl Iterator<Item = usize> {
        let left = (entry > self.first).then_some(self.first);
        let right = (entry < self.last).then_some(self.last);
        let lone = (self.first == self.last).then_some(entry);
        left.into_iter().chain(right).chain(lone)
    }
}

fn strip_links(cells: &[QuadCell]) -> (Vec<CellLink>, Vec<usize>) {
    let mut links = vec![CellLink::Root; cells.len()];
    let mut at = HashMap::<(usize, usize), usize>::default();
    let mut runs = Vec::<Run>::new();
    for (index, cell) in cells.iter().enumerate() {
        at.insert((cell.row(), cell.column()), index);
        match runs.last_mut() {
            Some(run) if run.row == cell.row() && run.last + 1 == cell.column() => {
                run.last = cell.column();
            },
            _ => runs.push(Run {
                row:   cell.row(),
                first: cell.column(),
                last:  cell.column(),
            }),
        }
    }

    let cell_at = |row: usize, column: usize| at[&(row, column)];
    let mut reached = vec![false; runs.len()];
    let mut order = Vec::with_capacity(cells.len());
    for root in 0..runs.len() {
        if reached[root] {
            continue;
        }
        reached[root] = true;
        let mut pending = std::collections::VecDeque::from([(root, runs[root].first)]);
        while let Some((run_index, entry)) = pending.pop_front() {
            let run = runs[run_index];
            order.push(cell_at(run.row, entry));
            for column in entry + 1..=run.last {
                links[cell_at(run.row, column)] = CellLink::Hinged {
                    parent: cell_at(run.row, column - 1),
                    side:   ParentSide::Left,
                };
                order.push(cell_at(run.row, column));
            }
            for column in (run.first..entry).rev() {
                links[cell_at(run.row, column)] = CellLink::Hinged {
                    parent: cell_at(run.row, column + 1),
                    side:   ParentSide::Right,
                };
                order.push(cell_at(run.row, column));
            }
            for (next, side) in [
                (run.row + 1, ParentSide::Above),
                (run.row.wrapping_sub(1), ParentSide::Below),
            ] {
                for (child_index, child) in runs.iter().enumerate() {
                    if reached[child_index]
                        || child.row != next
                        || child.last < run.first
                        || child.first > run.last
                    {
                        continue;
                    }
                    let shared = child.first.max(run.first)..=child.last.min(run.last);
                    let hinge_column = shared
                        .min_by_key(|column| {
                            run.strip_ends(entry)
                                .map(|end| column.abs_diff(end))
                                .min()
                                .unwrap_or_default()
                        })
                        .unwrap_or(child.first);
                    links[cell_at(child.row, hinge_column)] = CellLink::Hinged {
                        parent: cell_at(run.row, hinge_column),
                        side,
                    };
                    reached[child_index] = true;
                    pending.push_back((child_index, hinge_column));
                }
            }
        }
    }
    (links, order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pivot(displacement: Displacement) -> StackPivot {
        StackPivot {
            direction: StackFoldDirection::Positive,
            displacement,
        }
    }

    #[test]
    fn stack_pivot_assignments_name_every_clearance_coverage_failure() {
        let root = QuadCell::new(0, 0);
        let hinged = QuadCell::new(0, 1);
        let cells = [root, hinged];
        let links = [
            CellLink::Root,
            CellLink::Hinged {
                parent: 0,
                side:   ParentSide::Left,
            },
        ];
        let finite = pivot(Displacement::new(0.0, 0.0, 0.1));

        assert_eq!(
            StackPivotAssignments::try_new(&cells, &links, []),
            Err(CellSheetError::MissingStripFoldPivotClearance { cell: hinged })
        );
        assert_eq!(
            StackPivotAssignments::try_new(&cells, &links, [(hinged, finite), (hinged, finite)],),
            Err(CellSheetError::DuplicateStripFoldPivotClearance { cell: hinged })
        );
        assert_eq!(
            StackPivotAssignments::try_new(&cells, &links, [(root, finite)]),
            Err(CellSheetError::ForeignStripFoldPivotClearance { cell: root })
        );
        assert_eq!(
            StackPivotAssignments::try_new(
                &cells,
                &links,
                [(hinged, pivot(Displacement::new(0.0, 0.0, f32::NAN)))],
            ),
            Err(CellSheetError::NonFiniteStripFoldPivotClearance { cell: hinged })
        );
    }
}
