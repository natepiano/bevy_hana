use bevy::reflect::Reflect;
use thiserror::Error;

use super::playback::SequenceDirection;

/// Which boundary of a selected stage range a producer crossed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
pub enum RangeEdge {
    /// The lower normalized boundary of the selected range.
    Start,
    /// The upper normalized boundary of the selected range.
    End,
}

/// Whether crossing a range edge moved into or out of the selected range.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
pub enum RangeTransition {
    /// Travel moved into the selected range.
    Entered,
    /// Travel moved out of the selected range.
    Exited,
}

/// One crossing of a selected-range boundary in the direction it was crossed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
pub struct RangeCrossing {
    edge:      RangeEdge,
    direction: SequenceDirection,
}

impl RangeCrossing {
    /// Creates a crossing of `edge` travelling in `direction`.
    #[must_use]
    pub const fn new(edge: RangeEdge, direction: SequenceDirection) -> Self {
        Self { edge, direction }
    }

    /// Returns the crossed boundary.
    #[must_use]
    pub const fn edge(self) -> RangeEdge { self.edge }

    /// Returns the direction of travel through the boundary.
    #[must_use]
    pub const fn direction(self) -> SequenceDirection { self.direction }

    /// Returns whether this crossing entered or exited the selected range.
    #[must_use]
    pub const fn transition(self) -> RangeTransition {
        match (self.edge, self.direction) {
            (RangeEdge::Start, SequenceDirection::Forward)
            | (RangeEdge::End, SequenceDirection::Backward) => RangeTransition::Entered,
            (RangeEdge::Start, SequenceDirection::Backward)
            | (RangeEdge::End, SequenceDirection::Forward) => RangeTransition::Exited,
        }
    }

    const fn encode(self) -> u64 {
        let edge = match self.edge {
            RangeEdge::Start => 0,
            RangeEdge::End => 1,
        };
        let direction = match self.direction {
            SequenceDirection::Forward => 0,
            SequenceDirection::Backward => 1,
        };
        edge | (direction << 1)
    }

    const fn decode(bits: u64) -> Self {
        let edge = if bits & 1 == 0 {
            RangeEdge::Start
        } else {
            RangeEdge::End
        };
        let direction = if (bits >> 1) & 1 == 0 {
            SequenceDirection::Forward
        } else {
            SequenceDirection::Backward
        };
        Self { edge, direction }
    }
}

/// Ordered selected-range entries and exits recorded within one update.
///
/// A producer that crossed no selected-range boundary passes
/// [`RangeCrossings::NONE`] rather than omitting the value, so a consumer never
/// interprets absence. The encoding is a fixed-size bit field, so recording a
/// crossing allocates nothing. A round trip that returns to its starting
/// position keeps its crossings even though its start and end positions are
/// equal.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct RangeCrossings {
    bits: u64,
    len:  u8,
}

impl RangeCrossings {
    /// Most crossings one update can record.
    pub const CAPACITY: usize = 32;
    /// The explicit empty value for an update that crossed no boundary.
    pub const NONE: Self = Self { bits: 0, len: 0 };

    /// Records ordered crossings for one update.
    ///
    /// # Errors
    ///
    /// Returns [`RangeCrossingsError::CapacityExceeded`] beyond
    /// [`RangeCrossings::CAPACITY`] crossings and
    /// [`RangeCrossingsError::RepeatedTransition`] when two consecutive
    /// crossings both enter or both exit the selected range.
    pub fn try_new(
        crossings: impl IntoIterator<Item = RangeCrossing>,
    ) -> Result<Self, RangeCrossingsError> {
        let mut range_crossings = Self::NONE;
        let mut previous = Self::NONE;
        for (ordinal, range_crossing) in crossings.into_iter().enumerate() {
            if ordinal == Self::CAPACITY {
                return Err(RangeCrossingsError::CapacityExceeded {
                    capacity: Self::CAPACITY,
                });
            }
            if previous.len == 1 && previous.crossing(0).transition() == range_crossing.transition()
            {
                return Err(RangeCrossingsError::RepeatedTransition {
                    ordinal,
                    transition: range_crossing.transition(),
                });
            }
            range_crossings.bits |= range_crossing.encode() << (ordinal * Self::BITS_PER_CROSSING);
            range_crossings.len += 1;
            previous = Self {
                bits: range_crossing.encode(),
                len:  1,
            };
        }
        Ok(range_crossings)
    }

    /// Returns whether this update crossed no selected-range boundary.
    #[must_use]
    pub const fn is_none(self) -> bool { self.len == 0 }

    /// Returns whether this update crossed no selected-range boundary.
    #[must_use]
    pub const fn is_empty(self) -> bool { self.is_none() }

    /// Returns how many crossings this update recorded.
    #[must_use]
    pub const fn len(self) -> usize { self.len as usize }

    /// Iterates the recorded crossings in the order they happened.
    pub fn ordered(self) -> impl Iterator<Item = RangeCrossing> {
        (0..self.len()).map(move |ordinal| self.crossing(ordinal))
    }

    /// Returns the last recorded crossing, or `None` when this update crossed
    /// nothing.
    pub(super) const fn last(self) -> Option<RangeCrossing> {
        if self.len == 0 {
            None
        } else {
            Some(self.crossing(self.len as usize - 1))
        }
    }

    const BITS_PER_CROSSING: usize = 2;
    const CROSSING_MASK: u64 = 0b11;

    const fn crossing(self, ordinal: usize) -> RangeCrossing {
        RangeCrossing::decode(
            (self.bits >> (ordinal * Self::BITS_PER_CROSSING)) & Self::CROSSING_MASK,
        )
    }
}

/// Errors returned while recording [`RangeCrossings`].
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RangeCrossingsError {
    /// More crossings than one update can record.
    #[error("one update records at most {capacity} range crossings")]
    CapacityExceeded {
        /// Maximum recordable crossings.
        capacity: usize,
    },
    /// Two consecutive crossings both entered or both exited the range.
    #[error("range crossing {ordinal} repeats transition {transition:?}")]
    RepeatedTransition {
        /// Ordinal of the repeating crossing.
        ordinal:    usize,
        /// Repeated transition.
        transition: RangeTransition,
    },
}

/// Compact traversal between two ordered-ledger gaps.
///
/// The traversal retains its movement direction and uses it when counting or
/// iterating crossed records. A domain can emit its metadata lazily without a
/// boundary collection allocated per update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SequenceTraversal {
    old_gap:                  usize,
    new_gap:                  usize,
    direction:                SequenceDirection,
    whole_ledger_repetitions: i64,
    boundary_count:           usize,
}

impl SequenceTraversal {
    pub(super) const fn new(
        old_gap: usize,
        new_gap: usize,
        direction: SequenceDirection,
        whole_ledger_repetitions: i64,
        boundary_count: usize,
    ) -> Self {
        Self {
            old_gap,
            new_gap,
            direction,
            whole_ledger_repetitions,
            boundary_count,
        }
    }

    /// Returns the gap before traversal.
    #[must_use]
    pub const fn old_gap(self) -> usize { self.old_gap }

    /// Returns the gap after traversal.
    #[must_use]
    pub const fn new_gap(self) -> usize { self.new_gap }

    /// Returns signed complete-ledger repetitions between the two gaps.
    #[must_use]
    pub const fn whole_ledger_repetitions(self) -> i64 { self.whole_ledger_repetitions }

    /// Returns the ledger length used to create this traversal.
    #[must_use]
    pub const fn boundary_count(self) -> usize { self.boundary_count }

    /// Returns the exact number of boundary records crossed in the retained
    /// traversal direction.
    #[must_use]
    pub fn boundary_crossing_count(self) -> u128 {
        let complete_crossings =
            u128::from(self.whole_ledger_repetitions.unsigned_abs()) * self.boundary_count as u128;
        let old_gap = self.old_gap as u128;
        let new_gap = self.new_gap as u128;
        match self.direction {
            SequenceDirection::Forward => complete_crossings + new_gap - old_gap,
            SequenceDirection::Backward => complete_crossings + old_gap - new_gap,
        }
    }

    /// Iterates crossed boundary ordinals without allocating.
    ///
    /// Forward traversal yields stable ledger order. Backward traversal yields
    /// exact reverse order, including every coincident record and complete
    /// ledger repetition. The iterator uses the direction retained by this
    /// traversal.
    pub fn boundaries(self) -> impl Iterator<Item = usize> { BoundaryOrdinals::new(self) }
}

/// Result of applying native advancement or an arbitrary seek.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequenceUpdate {
    /// Neither normalized position nor the ordered gap cursor changed.
    NoTraversal,
    /// Scalar position, ordered cursor, or complete-ledger repetition changed.
    Traversed {
        /// Final normalized position after the update.
        normalized_position: f64,
        /// Direction associated with this update for domain-specific state changes.
        direction:           SequenceDirection,
        /// Compact descriptor retaining every crossed ledger record and its direction.
        traversal:           SequenceTraversal,
    },
}

struct BoundaryOrdinals {
    direction:      SequenceDirection,
    next_ordinal:   usize,
    remaining:      u128,
    boundary_count: usize,
}

impl BoundaryOrdinals {
    fn new(traversal: SequenceTraversal) -> Self {
        let direction = traversal.direction;
        let remaining = if traversal.boundary_count == 0 {
            0
        } else {
            traversal.boundary_crossing_count()
        };
        let next_ordinal = match direction {
            SequenceDirection::Forward if traversal.old_gap == traversal.boundary_count => 0,
            SequenceDirection::Forward => traversal.old_gap,
            SequenceDirection::Backward if traversal.old_gap == 0 => {
                traversal.boundary_count.saturating_sub(1)
            },
            SequenceDirection::Backward => traversal.old_gap - 1,
        };
        Self {
            direction,
            next_ordinal,
            remaining,
            boundary_count: traversal.boundary_count,
        }
    }
}

impl Iterator for BoundaryOrdinals {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let ordinal = self.next_ordinal;
        self.next_ordinal = match self.direction {
            SequenceDirection::Forward if ordinal + 1 == self.boundary_count => 0,
            SequenceDirection::Forward => ordinal + 1,
            SequenceDirection::Backward if ordinal == 0 => self.boundary_count - 1,
            SequenceDirection::Backward => ordinal - 1,
        };
        self.remaining -= 1;
        Some(ordinal)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        usize::try_from(self.remaining)
            .map_or((usize::MAX, None), |remaining| (remaining, Some(remaining)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::SequencePlayback;
    use crate::sequence::SequencePlaybackError;

    const FIRST_BOUNDARY: f64 = 0.25;
    const MIDDLE_BOUNDARY: f64 = 0.5;
    const LAST_BOUNDARY: f64 = 0.75;

    #[test]
    fn traversal_retains_forward_and_backward_direction_for_coincident_records()
    -> Result<(), SequencePlaybackError> {
        let mut playback = SequencePlayback::try_new([
            FIRST_BOUNDARY,
            MIDDLE_BOUNDARY,
            MIDDLE_BOUNDARY,
            LAST_BOUNDARY,
        ])?;

        let forward = playback.try_seek(LAST_BOUNDARY, SequenceDirection::Forward)?;
        assert_eq!(boundary_crossing_count(forward), 4);
        assert_eq!(crossed_boundaries(forward), vec![0, 1, 2, 3]);

        let backward = playback.try_seek(FIRST_BOUNDARY, SequenceDirection::Backward)?;
        assert_eq!(boundary_crossing_count(backward), 4);
        assert_eq!(crossed_boundaries(backward), vec![3, 2, 1, 0]);
        Ok(())
    }

    #[test]
    fn traversal_retains_direction_and_signed_multiple_wraps_without_a_transition_buffer()
    -> Result<(), SequencePlaybackError> {
        let mut playback =
            SequencePlayback::try_new([FIRST_BOUNDARY, MIDDLE_BOUNDARY, LAST_BOUNDARY])?;
        let initial = playback.try_seek(LAST_BOUNDARY, SequenceDirection::Forward)?;
        assert_eq!(boundary_crossing_count(initial), 3);
        assert_eq!(crossed_boundaries(initial), vec![0, 1, 2]);

        let forward =
            playback.try_seek_with_repetitions(FIRST_BOUNDARY, SequenceDirection::Forward, 3)?;
        assert_eq!(boundary_crossing_count(forward), 7);
        assert_eq!(crossed_boundaries(forward), vec![0, 1, 2, 0, 1, 2, 0]);
        assert_eq!(whole_ledger_repetitions(forward), 3);

        let backward =
            playback.try_seek_with_repetitions(LAST_BOUNDARY, SequenceDirection::Backward, -3)?;
        assert_eq!(boundary_crossing_count(backward), 8);
        assert_eq!(crossed_boundaries(backward), vec![0, 2, 1, 0, 2, 1, 0, 2]);
        assert_eq!(whole_ledger_repetitions(backward), -3);
        Ok(())
    }

    fn crossed_boundaries(sequence_update: SequenceUpdate) -> Vec<usize> {
        match sequence_update {
            SequenceUpdate::NoTraversal => Vec::new(),
            SequenceUpdate::Traversed { traversal, .. } => traversal.boundaries().collect(),
        }
    }

    fn boundary_crossing_count(sequence_update: SequenceUpdate) -> u128 {
        match sequence_update {
            SequenceUpdate::NoTraversal => 0,
            SequenceUpdate::Traversed { traversal, .. } => traversal.boundary_crossing_count(),
        }
    }

    fn whole_ledger_repetitions(sequence_update: SequenceUpdate) -> i64 {
        match sequence_update {
            SequenceUpdate::NoTraversal => 0,
            SequenceUpdate::Traversed { traversal, .. } => traversal.whole_ledger_repetitions(),
        }
    }
}
