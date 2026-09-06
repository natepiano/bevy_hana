use std::time::Duration;

use bevy::ecs::component::Component;
use bevy::ecs::reflect::ReflectComponent;
use bevy::reflect::Reflect;
use thiserror::Error;

use super::time::SequenceTime;
use super::traversal::RangeCrossings;
use super::traversal::SequenceTraversal;
use super::traversal::SequenceUpdate;

/// Direction of travel through an ordered sequence boundary ledger.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Reflect)]
pub enum SequenceDirection {
    /// Travel from normalized start toward normalized end.
    #[default]
    Forward,
    /// Travel from normalized end toward normalized start.
    Backward,
}

/// Result of applying a playback command that arbitration permitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceCommandOutcome {
    /// The command changed the native journey.
    Applied,
    /// The command was valid but the requested state was already effective.
    NoChange,
}

/// Validated normalized position within one sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Reflect)]
#[reflect(opaque)]
pub struct SequencePosition(f32);

impl SequencePosition {
    /// Normalized sequence end.
    pub const END: Self = Self(1.0);
    /// Normalized sequence start.
    pub const START: Self = Self(0.0);

    /// Creates a position from finite normalized progress within `0..=1`.
    ///
    /// # Errors
    ///
    /// Returns [`SequencePositionError::NonFinite`] or
    /// [`SequencePositionError::OutOfRange`] for an invalid value.
    pub fn try_new(normalized: f32) -> Result<Self, SequencePositionError> {
        if !normalized.is_finite() {
            return Err(SequencePositionError::NonFinite);
        }
        if !(Self::START.0..=Self::END.0).contains(&normalized) {
            return Err(SequencePositionError::OutOfRange { normalized });
        }
        Ok(Self(normalized))
    }

    /// Returns the normalized position.
    #[must_use]
    pub const fn normalized(self) -> f32 { self.0 }

    /// Clamps normalized progress into the valid range.
    ///
    /// An infinite value clamps to the bound it exceeds. `f32::clamp` returns
    /// NaN for a NaN input, so NaN is checked first and mapped to
    /// [`Self::START`], keeping the finite invariant [`Self::try_new`]
    /// enforces.
    pub(crate) const fn clamped(normalized: f32) -> Self {
        if normalized.is_nan() {
            return Self::START;
        }
        Self(normalized.clamp(Self::START.0, Self::END.0))
    }

    /// Clamps `f64` scalar progress into the valid range before narrowing it.
    ///
    /// Clamping first means the narrowed value is always within `0..=1`, so the
    /// conversion rounds and never overflows. NaN clamps to [`Self::START`], as
    /// in [`Self::clamped`].
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the value is clamped into 0..=1 before it narrows, leaving only rounding"
    )]
    fn clamped_from_f64(normalized: f64) -> Self {
        if normalized.is_nan() {
            return Self::START;
        }
        Self(normalized.clamp(f64::from(Self::START.0), f64::from(Self::END.0)) as f32)
    }
}

/// One producer's complete normalized movement for the current update.
///
/// The value carries no writer identity, clock, easing, target, or domain
/// output. A producer that crossed no selected-range boundary passes
/// [`RangeCrossings::NONE`], so every consumer reads one total movement.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(opaque)]
#[reflect(Component)]
pub struct SequenceMovement {
    position:          SequencePosition,
    direction:         SequenceDirection,
    whole_repetitions: i64,
    range_crossings:   RangeCrossings,
}

impl SequenceMovement {
    /// Creates one update's complete movement.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceMovementError::DirectionContradictsRepetitions`] when
    /// the signed repetition count disagrees with `direction`,
    /// [`SequenceMovementError::CrossingContradictsDirection`] when the final
    /// crossing was not crossed in `direction`, and
    /// [`SequenceMovementError::CrossingsContradictRepetitions`] when fewer
    /// crossings were recorded than complete repetitions require.
    pub fn try_new(
        position: SequencePosition,
        direction: SequenceDirection,
        whole_repetitions: i64,
        range_crossings: RangeCrossings,
    ) -> Result<Self, SequenceMovementError> {
        let repetitions_agree = match direction {
            SequenceDirection::Forward => whole_repetitions >= 0,
            SequenceDirection::Backward => whole_repetitions <= 0,
        };
        if !repetitions_agree {
            return Err(SequenceMovementError::DirectionContradictsRepetitions {
                direction,
                whole_repetitions,
            });
        }
        if let Some(last_crossing) = range_crossings.last()
            && last_crossing.direction() != direction
        {
            return Err(SequenceMovementError::CrossingContradictsDirection {
                direction,
                crossing_direction: last_crossing.direction(),
            });
        }
        let required_crossings =
            usize::try_from(whole_repetitions.unsigned_abs()).unwrap_or(usize::MAX);
        if range_crossings.len() < required_crossings {
            return Err(SequenceMovementError::CrossingsContradictRepetitions {
                whole_repetitions,
                crossings: range_crossings.len(),
            });
        }
        Ok(Self {
            position,
            direction,
            whole_repetitions,
            range_crossings,
        })
    }

    /// Returns the final normalized position of this update.
    #[must_use]
    pub const fn position(&self) -> SequencePosition { self.position }

    /// Returns the direction the producer was travelling.
    #[must_use]
    pub const fn direction(&self) -> SequenceDirection { self.direction }

    /// Returns signed complete-sequence repetitions travelled this update.
    #[must_use]
    pub const fn whole_repetitions(&self) -> i64 { self.whole_repetitions }

    /// Returns the ordered selected-range entries and exits of this update.
    #[must_use]
    pub const fn range_crossings(&self) -> RangeCrossings { self.range_crossings }

    /// Replaces only the position portion of this movement.
    ///
    /// Direction, repetition, and range-crossing metadata keep the values the
    /// producer set, so a scalar producer such as a tween writes no traversal
    /// metadata.
    pub const fn set_position(&mut self, position: SequencePosition) { self.position = position; }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SequenceTarget {
    normalized_position: f64,
    gap:                 usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NativeJourney {
    Idle,
    Playing {
        direction: SequenceDirection,
        target:    SequenceTarget,
    },
    Paused {
        direction: SequenceDirection,
        target:    SequenceTarget,
    },
}

/// History-independent normalized playback over an ordered boundary ledger.
///
/// Domains keep their boundary metadata in an immutable ledger and give this
/// type the ledger's normalized positions in the same stable ordinal order.
/// Coincident records therefore retain domain-defined ordering. Forward
/// traversal yields those ordinals in ledger order and backward traversal
/// yields them in reverse order.
///
/// Every mutation is reached through
/// [`SequenceCommands`](super::SequenceCommands) so a producer selected as the
/// target's [`SequenceDriver`](super::SequenceDriver) is the only writer of
/// local position while it holds the target.
///
/// For coincident fold records, domain ledgers place exiting member records
/// before the old stage end, the old stage end before the new stage begin,
/// stage begin before its member begins, member ends before stage end, adjacent
/// begin/end records for zero-duration members in group order, and a reached
/// endpoint last. [`SequenceTraversal::boundaries`] reverses that complete
/// ordering for backward travel.
#[derive(Clone, Debug, PartialEq)]
pub struct SequencePlayback {
    normalized_position: f64,
    boundary_positions:  Vec<f64>,
    gap:                 usize,
    native_journey:      NativeJourney,
}

impl SequencePlayback {
    /// Normalized sequence end.
    pub const END: f64 = 1.0;
    /// Normalized sequence start.
    pub const START: f64 = 0.0;

    /// Creates idle native playback at [`SequencePlayback::START`].
    ///
    /// Boundary positions must be finite, within the inclusive normalized
    /// range, and ordered from lowest to highest. Equal positions are retained
    /// as distinct ordered records. The initial gap is before every record, so
    /// zero-duration records at `START` remain traversable.
    ///
    /// # Errors
    ///
    /// Returns the exact [`SequencePlaybackError`] identifying a non-finite,
    /// out-of-range, or out-of-order boundary.
    pub fn try_new(
        boundary_positions: impl IntoIterator<Item = f64>,
    ) -> Result<Self, SequencePlaybackError> {
        let mut validated_positions = Vec::new();
        for (ordinal, normalized_position) in boundary_positions.into_iter().enumerate() {
            let normalized_position = Self::validate_boundary(ordinal, normalized_position)?;
            if validated_positions
                .last()
                .is_some_and(|previous| *previous > normalized_position)
            {
                return Err(SequencePlaybackError::BoundaryOutOfOrder { ordinal });
            }
            validated_positions.push(normalized_position);
        }

        Ok(Self {
            normalized_position: Self::START,
            boundary_positions:  validated_positions,
            gap:                 0,
            native_journey:      NativeJourney::Idle,
        })
    }

    /// Returns the current normalized position.
    #[must_use]
    pub const fn normalized_position(&self) -> f64 { self.normalized_position }

    /// Returns the current position as a validated shared value.
    #[must_use]
    pub fn position(&self) -> SequencePosition {
        SequencePosition::clamped_from_f64(self.normalized_position)
    }

    /// Returns the stable ordered boundary positions supplied at construction.
    #[must_use]
    pub fn boundary_positions(&self) -> &[f64] { &self.boundary_positions }

    /// Returns the current gap between ordered boundary records.
    #[must_use]
    pub const fn gap(&self) -> usize { self.gap }

    /// Returns whether a native journey is currently moving.
    #[must_use]
    pub const fn is_playing(&self) -> bool {
        matches!(self.native_journey, NativeJourney::Playing { .. })
    }

    /// Returns whether a native journey is paused with its destination retained.
    #[must_use]
    pub const fn is_paused(&self) -> bool {
        matches!(self.native_journey, NativeJourney::Paused { .. })
    }

    /// Selects an absolute normalized endpoint and begins native playback.
    ///
    /// Forward playback targets `END`; backward playback targets `START`.
    /// Calling this method from an interior position replaces the prior native
    /// destination without changing the current position.
    pub(super) fn play(&mut self, direction: SequenceDirection) -> SequenceCommandOutcome {
        let target = self.endpoint_target(direction);
        self.set_native_target(direction, target)
    }

    /// Begins native travel to an absolute interior destination.
    pub(super) fn play_to(&mut self, destination: SequencePosition) -> SequenceCommandOutcome {
        let normalized_position = f64::from(destination.normalized());
        let direction = if normalized_position < self.normalized_position {
            SequenceDirection::Backward
        } else {
            SequenceDirection::Forward
        };
        let target = SequenceTarget {
            normalized_position,
            gap: self.gap_at(normalized_position, direction),
        };
        self.set_native_target(direction, target)
    }

    /// Pauses an active native journey while retaining its destination.
    pub(super) const fn pause(&mut self) -> SequenceCommandOutcome {
        match self.native_journey {
            NativeJourney::Playing { direction, target } => {
                self.native_journey = NativeJourney::Paused { direction, target };
                SequenceCommandOutcome::Applied
            },
            NativeJourney::Idle | NativeJourney::Paused { .. } => SequenceCommandOutcome::NoChange,
        }
    }

    /// Resumes a paused native journey.
    pub(super) const fn resume(&mut self) -> SequenceCommandOutcome {
        match self.native_journey {
            NativeJourney::Paused { direction, target } => {
                self.native_journey = NativeJourney::Playing { direction, target };
                SequenceCommandOutcome::Applied
            },
            NativeJourney::Idle | NativeJourney::Playing { .. } => SequenceCommandOutcome::NoChange,
        }
    }

    /// Stops any native journey at the current local position.
    pub(super) const fn cancel(&mut self) -> SequenceCommandOutcome {
        match self.native_journey {
            NativeJourney::Idle => SequenceCommandOutcome::NoChange,
            NativeJourney::Playing { .. } | NativeJourney::Paused { .. } => {
                self.native_journey = NativeJourney::Idle;
                SequenceCommandOutcome::Applied
            },
        }
    }

    /// Begins native travel to the adjacent distinct boundary position.
    ///
    /// All records coincident at that position are one step destination. When
    /// the destination is already at the current scalar position, the next
    /// call to [`SequencePlayback::advance_native`] moves only the gap cursor.
    pub(super) fn step(&mut self, direction: SequenceDirection) -> SequenceCommandOutcome {
        let target = self.step_target(direction);
        self.set_native_target(direction, target)
    }

    /// Advances an active native journey by wall-clock `delta`.
    ///
    /// `total` is the exact sum of authored sequence durations. A zero total
    /// reaches the retained destination immediately. A nonzero total maps
    /// `delta` into normalized progress at one complete sequence per `total`.
    /// Paused and idle playback do not change.
    pub(super) fn advance_native(
        &mut self,
        delta: Duration,
        total: SequenceTime,
    ) -> SequenceUpdate {
        let NativeJourney::Playing { direction, target } = self.native_journey else {
            return SequenceUpdate::NoTraversal;
        };

        let scalar_destination_reached =
            Self::positions_match(self.normalized_position, target.normalized_position);
        let normalized_position =
            if total.is_zero() || scalar_destination_reached {
                target.normalized_position
            } else {
                let normalized_delta = delta.as_secs_f64() / total.as_seconds_f64();
                match direction {
                    SequenceDirection::Forward => (self.normalized_position + normalized_delta)
                        .min(target.normalized_position),
                    SequenceDirection::Backward => (self.normalized_position - normalized_delta)
                        .max(target.normalized_position),
                }
            };
        let destination_reached =
            Self::positions_match(normalized_position, target.normalized_position);
        let gap = if destination_reached {
            target.gap
        } else {
            self.gap_at(normalized_position, direction)
        };
        let sequence_update = self.apply_position(normalized_position, gap, direction, 0);

        if destination_reached {
            self.native_journey = NativeJourney::Idle;
        }

        sequence_update
    }

    /// Applies a selected producer's complete movement to local position.
    ///
    /// # Errors
    ///
    /// Returns [`SequencePlaybackError::TraversalDirectionMismatch`] when the
    /// movement contradicts the current local position, leaving playback
    /// unchanged.
    pub(super) fn apply_movement(
        &mut self,
        movement: &SequenceMovement,
    ) -> Result<SequenceUpdate, SequencePlaybackError> {
        self.try_seek_with_repetitions(
            f64::from(movement.position().normalized()),
            movement.direction(),
            movement.whole_repetitions(),
        )
    }

    /// Seeks directly without traversing additional complete ledger repetitions.
    ///
    /// The requested direction must agree with the scalar movement. Equal
    /// scalar positions are accepted so coincident records can move only the
    /// gap cursor.
    ///
    /// # Errors
    ///
    /// Returns [`SequencePlaybackError::NonFinitePosition`] or
    /// [`SequencePlaybackError::PositionOutOfRange`] for an invalid position,
    /// and [`SequencePlaybackError::TraversalDirectionMismatch`] when the
    /// direction contradicts the requested movement.
    pub(super) fn try_seek(
        &mut self,
        normalized_position: f64,
        direction: SequenceDirection,
    ) -> Result<SequenceUpdate, SequencePlaybackError> {
        self.try_seek_with_repetitions(normalized_position, direction, 0)
    }

    /// Seeks with signed complete-ledger repetitions retained in the traversal.
    ///
    /// Positive repetitions require forward travel and negative repetitions
    /// require backward travel. A zero repetition count requires the scalar
    /// destination itself to agree with `direction`.
    ///
    /// # Errors
    ///
    /// Returns a position-validation error or
    /// [`SequencePlaybackError::TraversalDirectionMismatch`] when the signed
    /// repetition count and scalar movement do not describe `direction`.
    pub(super) fn try_seek_with_repetitions(
        &mut self,
        normalized_position: f64,
        direction: SequenceDirection,
        whole_ledger_repetitions: i64,
    ) -> Result<SequenceUpdate, SequencePlaybackError> {
        let normalized_position = Self::validate_position(normalized_position)?;
        self.validate_direction(normalized_position, direction, whole_ledger_repetitions)?;

        let gap = self.gap_at(normalized_position, direction);
        let sequence_update = self.apply_position(
            normalized_position,
            gap,
            direction,
            whole_ledger_repetitions,
        );
        if self.native_journey != NativeJourney::Idle {
            self.native_journey = NativeJourney::Idle;
        }
        Ok(sequence_update)
    }

    fn validate_boundary(
        ordinal: usize,
        normalized_position: f64,
    ) -> Result<f64, SequencePlaybackError> {
        if !normalized_position.is_finite() {
            return Err(SequencePlaybackError::NonFiniteBoundary { ordinal });
        }
        if !(Self::START..=Self::END).contains(&normalized_position) {
            return Err(SequencePlaybackError::BoundaryOutOfRange {
                ordinal,
                normalized_position,
            });
        }
        Ok(Self::canonicalize_zero(normalized_position))
    }

    fn validate_position(normalized_position: f64) -> Result<f64, SequencePlaybackError> {
        if !normalized_position.is_finite() {
            return Err(SequencePlaybackError::NonFinitePosition);
        }
        if !(Self::START..=Self::END).contains(&normalized_position) {
            return Err(SequencePlaybackError::PositionOutOfRange {
                normalized_position,
            });
        }
        Ok(Self::canonicalize_zero(normalized_position))
    }

    fn validate_direction(
        &self,
        normalized_position: f64,
        direction: SequenceDirection,
        whole_ledger_repetitions: i64,
    ) -> Result<(), SequencePlaybackError> {
        let repetitions_agree = match direction {
            SequenceDirection::Forward => whole_ledger_repetitions >= 0,
            SequenceDirection::Backward => whole_ledger_repetitions <= 0,
        };
        let scalar_agrees = match direction {
            SequenceDirection::Forward => normalized_position >= self.normalized_position,
            SequenceDirection::Backward => normalized_position <= self.normalized_position,
        };
        if repetitions_agree && (whole_ledger_repetitions != 0 || scalar_agrees) {
            Ok(())
        } else {
            Err(SequencePlaybackError::TraversalDirectionMismatch {
                direction,
                whole_ledger_repetitions,
            })
        }
    }

    const fn canonicalize_zero(normalized_position: f64) -> f64 {
        if normalized_position == Self::START {
            Self::START
        } else {
            normalized_position
        }
    }

    const fn positions_match(left: f64, right: f64) -> bool { left.to_bits() == right.to_bits() }

    const fn endpoint_target(&self, direction: SequenceDirection) -> SequenceTarget {
        match direction {
            SequenceDirection::Forward => SequenceTarget {
                normalized_position: Self::END,
                gap:                 self.boundary_positions.len(),
            },
            SequenceDirection::Backward => SequenceTarget {
                normalized_position: Self::START,
                gap:                 0,
            },
        }
    }

    fn step_target(&self, direction: SequenceDirection) -> SequenceTarget {
        match direction {
            SequenceDirection::Forward if self.gap < self.boundary_positions.len() => {
                let normalized_position = self.boundary_positions[self.gap];
                SequenceTarget {
                    normalized_position,
                    gap: self.upper_gap(normalized_position),
                }
            },
            SequenceDirection::Backward if self.gap > 0 => {
                let normalized_position = self.boundary_positions[self.gap - 1];
                SequenceTarget {
                    normalized_position,
                    gap: self.lower_gap(normalized_position),
                }
            },
            _ => self.endpoint_target(direction),
        }
    }

    fn set_native_target(
        &mut self,
        direction: SequenceDirection,
        target: SequenceTarget,
    ) -> SequenceCommandOutcome {
        if Self::positions_match(self.normalized_position, target.normalized_position)
            && self.gap == target.gap
        {
            if self.native_journey == NativeJourney::Idle {
                return SequenceCommandOutcome::NoChange;
            }
            self.native_journey = NativeJourney::Idle;
            return SequenceCommandOutcome::Applied;
        }

        let native_journey = NativeJourney::Playing { direction, target };
        if self.native_journey == native_journey {
            SequenceCommandOutcome::NoChange
        } else {
            self.native_journey = native_journey;
            SequenceCommandOutcome::Applied
        }
    }

    fn gap_at(&self, normalized_position: f64, direction: SequenceDirection) -> usize {
        match direction {
            SequenceDirection::Forward => self.upper_gap(normalized_position),
            SequenceDirection::Backward => self.lower_gap(normalized_position),
        }
    }

    fn lower_gap(&self, normalized_position: f64) -> usize {
        self.boundary_positions
            .partition_point(|boundary| *boundary < normalized_position)
    }

    fn upper_gap(&self, normalized_position: f64) -> usize {
        self.boundary_positions
            .partition_point(|boundary| *boundary <= normalized_position)
    }

    const fn apply_position(
        &mut self,
        normalized_position: f64,
        gap: usize,
        direction: SequenceDirection,
        whole_ledger_repetitions: i64,
    ) -> SequenceUpdate {
        let position_changed =
            !Self::positions_match(self.normalized_position, normalized_position);
        let gap_changed = self.gap != gap;
        let repeated_nonempty_ledger =
            whole_ledger_repetitions != 0 && !self.boundary_positions.is_empty();
        if !position_changed && !gap_changed && !repeated_nonempty_ledger {
            return SequenceUpdate::NoTraversal;
        }

        let traversal = SequenceTraversal::new(
            self.gap,
            gap,
            direction,
            whole_ledger_repetitions,
            self.boundary_positions.len(),
        );
        self.normalized_position = normalized_position;
        self.gap = gap;

        SequenceUpdate::Traversed {
            normalized_position,
            direction,
            traversal,
        }
    }
}

/// Errors returned while creating a [`SequencePosition`].
#[derive(Clone, Copy, Debug, Error, PartialEq)]
pub enum SequencePositionError {
    /// The value was NaN or infinite.
    #[error("sequence position must be finite")]
    NonFinite,
    /// The value was outside the inclusive normalized range.
    #[error("sequence position {normalized} must be within 0..=1")]
    OutOfRange {
        /// Invalid normalized value.
        normalized: f32,
    },
}

/// Errors returned while creating a [`SequenceMovement`].
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SequenceMovementError {
    /// Direction and signed repetition count disagreed.
    #[error("movement direction {direction:?} contradicts {whole_repetitions} repetitions")]
    DirectionContradictsRepetitions {
        /// Requested direction.
        direction:         SequenceDirection,
        /// Requested signed complete-sequence repetitions.
        whole_repetitions: i64,
    },
    /// The final range crossing was not crossed in the movement's direction.
    #[error(
        "final range crossing direction {crossing_direction:?} contradicts movement direction \
         {direction:?}"
    )]
    CrossingContradictsDirection {
        /// Requested direction.
        direction:          SequenceDirection,
        /// Direction of the final recorded crossing.
        crossing_direction: SequenceDirection,
    },
    /// Fewer crossings were recorded than complete repetitions require.
    #[error("{crossings} range crossings cannot describe {whole_repetitions} repetitions")]
    CrossingsContradictRepetitions {
        /// Requested signed complete-sequence repetitions.
        whole_repetitions: i64,
        /// Recorded crossing count.
        crossings:         usize,
    },
}

/// Errors returned while constructing or moving [`SequencePlayback`].
#[derive(Clone, Copy, Debug, Error, PartialEq)]
pub enum SequencePlaybackError {
    /// One boundary was NaN or infinite.
    #[error("sequence boundary {ordinal} must be finite")]
    NonFiniteBoundary {
        /// Stable ordinal of the invalid boundary.
        ordinal: usize,
    },
    /// One boundary was outside the inclusive normalized range.
    #[error("sequence boundary {ordinal} position {normalized_position} must be within 0..=1")]
    BoundaryOutOfRange {
        /// Stable ordinal of the invalid boundary.
        ordinal:             usize,
        /// Invalid normalized position.
        normalized_position: f64,
    },
    /// One boundary followed a greater normalized position.
    #[error("sequence boundary {ordinal} is out of order")]
    BoundaryOutOfOrder {
        /// Stable ordinal of the first out-of-order boundary.
        ordinal: usize,
    },
    /// A seek position was NaN or infinite.
    #[error("sequence position must be finite")]
    NonFinitePosition,
    /// A seek position was outside the inclusive normalized range.
    #[error("sequence position {normalized_position} must be within 0..=1")]
    PositionOutOfRange {
        /// Invalid normalized position.
        normalized_position: f64,
    },
    /// Seek direction contradicted its scalar movement or signed repetitions.
    #[error(
        "sequence direction {direction:?} contradicts {whole_ledger_repetitions} ledger repetitions"
    )]
    TraversalDirectionMismatch {
        /// Requested direction.
        direction:                SequenceDirection,
        /// Requested signed complete-ledger repetitions.
        whole_ledger_repetitions: i64,
    },
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exactly representable sentinel positions"
)]
mod tests {
    use super::*;
    use crate::sequence::traversal::RangeCrossing;
    use crate::sequence::traversal::RangeEdge;

    const FIRST_BOUNDARY: f64 = 0.25;
    const MIDDLE_BOUNDARY: f64 = 0.5;
    const MIDDLE_POSITION: f32 = 0.5;

    #[test]
    fn position_construction_reports_every_exact_error_variant() {
        assert_eq!(
            SequencePosition::try_new(f32::NAN),
            Err(SequencePositionError::NonFinite)
        );
        assert_eq!(
            SequencePosition::try_new(-MIDDLE_POSITION),
            Err(SequencePositionError::OutOfRange {
                normalized: -MIDDLE_POSITION,
            })
        );
        assert_eq!(
            SequencePosition::try_new(MIDDLE_POSITION).map(SequencePosition::normalized),
            Ok(MIDDLE_POSITION)
        );
        assert_eq!(SequencePosition::START.normalized(), 0.0);
        assert_eq!(SequencePosition::END.normalized(), 1.0);
    }

    #[test]
    fn clamping_a_non_finite_value_still_yields_a_finite_position() {
        for clamped in [
            SequencePosition::clamped(f32::NAN),
            SequencePosition::clamped(f32::INFINITY),
            SequencePosition::clamped(f32::NEG_INFINITY),
            SequencePosition::clamped_from_f64(f64::NAN),
        ] {
            assert!(
                clamped.normalized().is_finite(),
                "clamping never breaches the finite invariant"
            );
            assert_eq!(
                SequencePosition::try_new(clamped.normalized()),
                Ok(clamped),
                "a clamped position is one try_new would have accepted"
            );
        }
        assert_eq!(SequencePosition::clamped(f32::NAN), SequencePosition::START);
        assert_eq!(
            SequencePosition::clamped(f32::INFINITY),
            SequencePosition::END
        );
    }

    #[test]
    fn movement_accessors_return_every_constructed_part() -> Result<(), SequenceMovementError> {
        let range_crossings = RangeCrossings::try_new([
            RangeCrossing::new(RangeEdge::Start, SequenceDirection::Forward),
            RangeCrossing::new(RangeEdge::End, SequenceDirection::Forward),
        ])
        .expect("alternating forward crossings are valid");
        let movement = SequenceMovement::try_new(
            SequencePosition::END,
            SequenceDirection::Forward,
            2,
            range_crossings,
        )?;

        assert_eq!(movement.position(), SequencePosition::END);
        assert_eq!(movement.direction(), SequenceDirection::Forward);
        assert_eq!(movement.whole_repetitions(), 2);
        assert_eq!(movement.range_crossings(), range_crossings);
        Ok(())
    }

    #[test]
    fn movement_construction_rejects_contradictory_direction_and_crossings() {
        assert_eq!(
            SequenceMovement::try_new(
                SequencePosition::END,
                SequenceDirection::Backward,
                1,
                RangeCrossings::NONE,
            ),
            Err(SequenceMovementError::DirectionContradictsRepetitions {
                direction:         SequenceDirection::Backward,
                whole_repetitions: 1,
            })
        );
        let backward_crossing = RangeCrossings::try_new([RangeCrossing::new(
            RangeEdge::End,
            SequenceDirection::Backward,
        )])
        .expect("a single crossing is valid");
        assert_eq!(
            SequenceMovement::try_new(
                SequencePosition::END,
                SequenceDirection::Forward,
                0,
                backward_crossing,
            ),
            Err(SequenceMovementError::CrossingContradictsDirection {
                direction:          SequenceDirection::Forward,
                crossing_direction: SequenceDirection::Backward,
            })
        );
        assert_eq!(
            SequenceMovement::try_new(
                SequencePosition::START,
                SequenceDirection::Forward,
                3,
                RangeCrossings::NONE,
            ),
            Err(SequenceMovementError::CrossingsContradictRepetitions {
                whole_repetitions: 3,
                crossings:         0,
            })
        );
    }

    #[test]
    fn movement_records_a_round_trip_whose_endpoints_are_equal() -> Result<(), SequenceMovementError>
    {
        let round_trip = RangeCrossings::try_new([
            RangeCrossing::new(RangeEdge::Start, SequenceDirection::Forward),
            RangeCrossing::new(RangeEdge::End, SequenceDirection::Forward),
            RangeCrossing::new(RangeEdge::End, SequenceDirection::Backward),
            RangeCrossing::new(RangeEdge::Start, SequenceDirection::Backward),
        ])
        .expect("alternating round-trip crossings are valid");
        let movement = SequenceMovement::try_new(
            SequencePosition::START,
            SequenceDirection::Backward,
            0,
            round_trip,
        )?;

        assert_eq!(movement.position(), SequencePosition::START);
        assert_eq!(movement.range_crossings().len(), 4);
        assert!(!movement.range_crossings().is_none());
        Ok(())
    }

    #[test]
    fn tween_producers_replace_only_the_position_portion() -> Result<(), SequenceMovementError> {
        let mut movement = SequenceMovement::try_new(
            SequencePosition::START,
            SequenceDirection::Forward,
            1,
            RangeCrossings::try_new([RangeCrossing::new(
                RangeEdge::End,
                SequenceDirection::Forward,
            )])
            .expect("a single crossing is valid"),
        )?;
        let before = movement;

        movement.set_position(SequencePosition::END);

        assert_eq!(movement.position(), SequencePosition::END);
        assert_eq!(movement.direction(), before.direction());
        assert_eq!(movement.whole_repetitions(), before.whole_repetitions());
        assert_eq!(movement.range_crossings(), before.range_crossings());
        Ok(())
    }

    #[test]
    fn playback_constructor_accepts_ordered_coincident_boundaries() {
        let playback = SequencePlayback::try_new([
            SequencePlayback::START,
            MIDDLE_BOUNDARY,
            MIDDLE_BOUNDARY,
            SequencePlayback::END,
        ]);

        assert!(playback.is_ok());
    }

    #[test]
    fn playback_constructor_reports_every_exact_error_variant() {
        assert_eq!(
            SequencePlayback::try_new([f64::NAN]),
            Err(SequencePlaybackError::NonFiniteBoundary { ordinal: 0 })
        );
        assert_eq!(
            SequencePlayback::try_new([SequencePlayback::END.next_up()]),
            Err(SequencePlaybackError::BoundaryOutOfRange {
                ordinal:             0,
                normalized_position: SequencePlayback::END.next_up(),
            })
        );
        assert_eq!(
            SequencePlayback::try_new([MIDDLE_BOUNDARY, FIRST_BOUNDARY]),
            Err(SequencePlaybackError::BoundaryOutOfOrder { ordinal: 1 })
        );
    }

    #[test]
    fn pause_resume_and_interior_reversal_retain_absolute_destinations()
    -> Result<(), SequencePlaybackError> {
        let mut playback = SequencePlayback::try_new([MIDDLE_BOUNDARY])?;
        let total = SequenceTime::from(Duration::from_secs(10));

        assert_eq!(
            playback.play(SequenceDirection::Forward),
            SequenceCommandOutcome::Applied
        );
        let forward = playback.advance_native(Duration::from_secs(4), total);
        assert_position_eq(forward_position(forward), 0.4);

        assert_eq!(playback.pause(), SequenceCommandOutcome::Applied);
        let paused_position = playback.normalized_position();
        assert_eq!(
            playback.advance_native(Duration::from_secs(4), total),
            SequenceUpdate::NoTraversal
        );
        assert_position_eq(playback.normalized_position(), paused_position);
        assert_eq!(playback.resume(), SequenceCommandOutcome::Applied);
        let resumed = playback.advance_native(Duration::from_secs(1), total);
        assert_eq!(crossed_boundaries(resumed), vec![0]);

        assert_eq!(
            playback.play(SequenceDirection::Backward),
            SequenceCommandOutcome::Applied
        );
        let reversed = playback.advance_native(Duration::from_secs(1), total);
        assert_eq!(crossed_boundaries(reversed), vec![0]);
        assert_position_eq(playback.normalized_position(), 0.4);
        Ok(())
    }

    #[test]
    fn cancel_stops_at_the_current_position_and_resume_needs_a_paused_journey()
    -> Result<(), SequencePlaybackError> {
        let mut playback = SequencePlayback::try_new([MIDDLE_BOUNDARY])?;
        let total = SequenceTime::from(Duration::from_secs(10));

        assert_eq!(playback.resume(), SequenceCommandOutcome::NoChange);
        assert_eq!(playback.cancel(), SequenceCommandOutcome::NoChange);
        assert_eq!(
            playback.play(SequenceDirection::Forward),
            SequenceCommandOutcome::Applied
        );
        playback.advance_native(Duration::from_secs(2), total);
        let cancelled_position = playback.normalized_position();

        assert_eq!(playback.cancel(), SequenceCommandOutcome::Applied);
        assert!(!playback.is_playing());
        assert_eq!(
            playback.advance_native(Duration::from_secs(2), total),
            SequenceUpdate::NoTraversal
        );
        assert_position_eq(playback.normalized_position(), cancelled_position);
        Ok(())
    }

    #[test]
    fn native_play_to_reaches_an_absolute_interior_destination() -> Result<(), SequencePlaybackError>
    {
        let mut playback = SequencePlayback::try_new([MIDDLE_BOUNDARY])?;
        let total = SequenceTime::from(Duration::from_secs(10));
        let destination =
            SequencePosition::try_new(MIDDLE_POSITION).expect("0.5 is a valid position");

        assert_eq!(
            playback.play_to(destination),
            SequenceCommandOutcome::Applied
        );
        playback.advance_native(Duration::from_secs(10), total);

        assert_position_eq(playback.normalized_position(), MIDDLE_BOUNDARY);
        assert_eq!(
            playback.play_to(destination),
            SequenceCommandOutcome::NoChange
        );
        Ok(())
    }

    #[test]
    fn applied_movement_preserves_multi_wrap_traversal() -> Result<(), SequencePlaybackError> {
        let mut playback = SequencePlayback::try_new([FIRST_BOUNDARY, MIDDLE_BOUNDARY])?;
        let range_crossings = RangeCrossings::try_new([
            RangeCrossing::new(RangeEdge::End, SequenceDirection::Forward),
            RangeCrossing::new(RangeEdge::Start, SequenceDirection::Forward),
        ])
        .expect("alternating forward crossings are valid");
        let movement = SequenceMovement::try_new(
            SequencePosition::try_new(MIDDLE_POSITION).expect("0.5 is a valid position"),
            SequenceDirection::Forward,
            2,
            range_crossings,
        )
        .expect("forward repetitions agree with forward crossings");

        let sequence_update = playback.apply_movement(&movement)?;

        assert_eq!(crossed_boundaries(sequence_update), vec![0, 1, 0, 1, 0, 1]);
        assert_position_eq(playback.normalized_position(), MIDDLE_BOUNDARY);
        Ok(())
    }

    #[test]
    fn adjacent_steps_traverse_coincident_records_at_one_scalar_position()
    -> Result<(), SequencePlaybackError> {
        let mut playback = SequencePlayback::try_new([
            SequencePlayback::START,
            SequencePlayback::START,
            MIDDLE_BOUNDARY,
            SequencePlayback::END,
        ])?;
        let total = SequenceTime::from(Duration::from_secs(1));

        assert_eq!(
            playback.step(SequenceDirection::Forward),
            SequenceCommandOutcome::Applied
        );
        let coincident = playback.advance_native(Duration::ZERO, total);
        assert_position_eq(playback.normalized_position(), SequencePlayback::START);
        assert_eq!(crossed_boundaries(coincident), vec![0, 1]);

        assert_eq!(
            playback.step(SequenceDirection::Forward),
            SequenceCommandOutcome::Applied
        );
        let middle = playback.advance_native(Duration::from_millis(500), total);
        assert_eq!(crossed_boundaries(middle), vec![2]);

        assert_eq!(
            playback.step(SequenceDirection::Backward),
            SequenceCommandOutcome::Applied
        );
        let reverse_at_middle = playback.advance_native(Duration::ZERO, total);
        assert_position_eq(playback.normalized_position(), MIDDLE_BOUNDARY);
        assert_eq!(crossed_boundaries(reverse_at_middle), vec![2]);
        Ok(())
    }

    #[test]
    fn zero_total_sequence_crosses_every_record_in_both_directions()
    -> Result<(), SequencePlaybackError> {
        let mut playback = SequencePlayback::try_new([
            SequencePlayback::START,
            SequencePlayback::START,
            SequencePlayback::START,
        ])?;

        assert_eq!(
            playback.play(SequenceDirection::Forward),
            SequenceCommandOutcome::Applied
        );
        let forward = playback.advance_native(Duration::ZERO, SequenceTime::ZERO);
        assert_position_eq(playback.normalized_position(), SequencePlayback::END);
        assert_eq!(crossed_boundaries(forward), vec![0, 1, 2]);

        assert_eq!(
            playback.play(SequenceDirection::Backward),
            SequenceCommandOutcome::Applied
        );
        let backward = playback.advance_native(Duration::ZERO, SequenceTime::ZERO);
        assert_position_eq(playback.normalized_position(), SequencePlayback::START);
        assert_eq!(crossed_boundaries(backward), vec![2, 1, 0]);
        Ok(())
    }

    #[test]
    fn equal_idle_seek_without_boundaries_is_write_free() -> Result<(), SequencePlaybackError> {
        let mut playback = SequencePlayback::try_new([])?;
        let before = playback.clone();

        assert_eq!(
            playback.try_seek(SequencePlayback::START, SequenceDirection::Forward)?,
            SequenceUpdate::NoTraversal
        );
        assert_eq!(playback, before);
        Ok(())
    }

    #[test]
    fn seek_validation_preserves_state_on_every_error() -> Result<(), SequencePlaybackError> {
        let mut playback = SequencePlayback::try_new([MIDDLE_BOUNDARY])?;
        let positioned = playback.try_seek(MIDDLE_BOUNDARY, SequenceDirection::Forward)?;
        assert_eq!(crossed_boundaries(positioned), vec![0]);
        let before = playback.clone();

        assert_eq!(
            playback.try_seek(f64::INFINITY, SequenceDirection::Forward),
            Err(SequencePlaybackError::NonFinitePosition)
        );
        assert_eq!(
            playback.try_seek(-FIRST_BOUNDARY, SequenceDirection::Backward),
            Err(SequencePlaybackError::PositionOutOfRange {
                normalized_position: -FIRST_BOUNDARY,
            })
        );
        assert_eq!(
            playback.try_seek(FIRST_BOUNDARY, SequenceDirection::Forward),
            Err(SequencePlaybackError::TraversalDirectionMismatch {
                direction:                SequenceDirection::Forward,
                whole_ledger_repetitions: 0,
            })
        );
        assert_eq!(playback, before);
        Ok(())
    }

    fn crossed_boundaries(sequence_update: SequenceUpdate) -> Vec<usize> {
        match sequence_update {
            SequenceUpdate::NoTraversal => Vec::new(),
            SequenceUpdate::Traversed { traversal, .. } => traversal.boundaries().collect(),
        }
    }

    fn assert_position_eq(actual: f64, expected: f64) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }

    fn forward_position(sequence_update: SequenceUpdate) -> f64 {
        match sequence_update {
            SequenceUpdate::NoTraversal => SequencePlayback::START,
            SequenceUpdate::Traversed {
                normalized_position,
                ..
            } => normalized_position,
        }
    }
}
