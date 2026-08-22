use bevy_ecs::entity::Entity;
use bevy_kana::SequenceTime;
use bevy_kana::ToF32;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;

use super::FoldTarget;
use super::FoldTiming;
use super::constants::SEQUENCE_END;
use super::constants::SEQUENCE_START;
use super::sequence::PlacedStage;
use super::stage;

/// Which of a fold sequence's two endpoints a value names.
///
/// A boundary record names the endpoint each side of a stage transition holds,
/// so a consumer can ask what a member is doing at a boundary without
/// re-deriving it from segment math. Retained playback always starts a new
/// [`FoldSequencePlayback`](crate::FoldSequencePlayback) at normalized position
/// zero, which is [`Self::Base`]; a sequence that should present folded is
/// authored that way or moved there by its issuer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
#[reflect(PartialEq, Debug, Default, Clone)]
pub enum FoldEndpoint {
    /// Normalized position zero, where every member holds
    /// [`FoldTarget::BASE`] and the hinge holds
    /// [`Hinge::base_angle`](crate::Hinge::base_angle).
    #[default]
    Base,
    /// Normalized position one, where every member holds the last target its
    /// own segments reached.
    Folded,
}

/// How far a member has travelled through one authored segment.
///
/// Normalized to `0..=1` and finite: [`FoldSegment::raw_progress`] is the only
/// producer and it clamps every sampled position into that range, so no reader
/// has to test the value before using it. A position that names no place —
/// NaN — orders against neither end of the segment, so it reads as
/// [`Self::ENTERED`] rather than escaping the range.
///
/// Easing consumes [`Self::normalized`] and its output is never stored back
/// here: a single-stage `ReplacedBy` curve may carry that output past either
/// end, which is an evaluation result rather than a position in a segment.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct FoldSegmentProgress(f32);

impl FoldSegmentProgress {
    /// A segment just entered and not yet travelled.
    pub const ENTERED: Self = Self(0.0);
    /// A segment travelled to its end.
    pub const FULLY_TRAVELLED: Self = Self(1.0);

    /// Returns the normalized progress.
    #[must_use]
    pub const fn normalized(self) -> f32 { self.0 }

    /// Clamps `f64` progress into the valid range before narrowing it.
    ///
    /// Clamping first leaves only rounding for the narrowing to do. NaN orders
    /// against nothing and so names no position within the segment; it reads as
    /// [`Self::ENTERED`], as
    /// [`SequencePosition`](bevy_kana::SequencePosition) does for a sequence.
    fn clamped_from_f64(normalized: f64) -> Self {
        if normalized.is_nan() {
            return Self::ENTERED;
        }
        Self(
            normalized
                .clamp(
                    f64::from(Self::ENTERED.0),
                    f64::from(Self::FULLY_TRAVELLED.0),
                )
                .to_f32(),
        )
    }
}

/// Where a sampled position sits relative to one member's authored segments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FoldMemberSample<'sequence> {
    /// The sequence authors no segment for this entity, so it never moves it.
    Unauthored,
    /// The member holds a target it has already reached: [`FoldTarget::BASE`]
    /// before its first segment begins, or the destination of the last segment
    /// the sampled position entered.
    Resting {
        /// Destination the member holds.
        target: FoldTarget,
    },
    /// The sampled position lies inside one authored segment.
    Moving {
        /// Segment containing the sampled position.
        segment:      &'sequence FoldSegment,
        /// Position within `segment`, produced by
        /// [`FoldSegment::raw_progress`].
        raw_progress: FoldSegmentProgress,
    },
}

/// One member's movement inside one stage, resolved to exact sequence time.
///
/// A member repeated across stages owns one segment per stage it appears in.
/// Each segment starts at the target the member's previous segment reached, so
/// sampling never consults prior output.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldSegment {
    pub(super) member_entity:    Entity,
    pub(super) stage_ordinal:    usize,
    pub(super) member_index:     usize,
    pub(super) from:             FoldTarget,
    pub(super) to:               FoldTarget,
    pub(super) begin:            SequenceTime,
    pub(super) end:              SequenceTime,
    pub(super) normalized_begin: f64,
    pub(super) normalized_end:   f64,
    pub(super) timing:           FoldTiming,
}

impl FoldSegment {
    /// Returns the member this segment moves.
    #[must_use]
    pub const fn member_entity(&self) -> Entity { self.member_entity }

    /// Returns the authored position of the stage that owns this segment.
    #[must_use]
    pub const fn stage_ordinal(&self) -> usize { self.stage_ordinal }

    /// Returns the member's group-local index within that stage.
    #[must_use]
    pub const fn member_index(&self) -> usize { self.member_index }

    /// Returns the target this segment starts from.
    #[must_use]
    pub const fn from(&self) -> FoldTarget { self.from }

    /// Returns the target this segment ends at.
    #[must_use]
    pub const fn to(&self) -> FoldTarget { self.to }

    /// Returns the exact elapsed sequence time where this segment starts.
    #[must_use]
    pub const fn begin(&self) -> SequenceTime { self.begin }

    /// Returns the exact elapsed sequence time where this segment ends.
    #[must_use]
    pub const fn end(&self) -> SequenceTime { self.end }

    /// Returns the normalized sequence position where this segment starts.
    #[must_use]
    pub const fn normalized_begin(&self) -> f64 { self.normalized_begin }

    /// Returns the normalized sequence position where this segment ends.
    #[must_use]
    pub const fn normalized_end(&self) -> f64 { self.normalized_end }

    /// Returns the resolved timing this segment was built from.
    #[must_use]
    pub const fn timing(&self) -> &FoldTiming { &self.timing }

    /// Returns `normalized_position` as progress across this segment.
    ///
    /// A zero-duration segment has no interior, so reaching it completes it.
    #[must_use]
    pub fn raw_progress(&self, normalized_position: f64) -> FoldSegmentProgress {
        let extent = self.normalized_end - self.normalized_begin;
        if extent <= 0.0 {
            return FoldSegmentProgress::FULLY_TRAVELLED;
        }
        FoldSegmentProgress::clamped_from_f64(
            (normalized_position - self.normalized_begin) / extent,
        )
    }
}

/// Every segment one member travels, in stage order.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldMemberTrack {
    member_entity: Entity,
    segments:      Vec<FoldSegment>,
}

impl FoldMemberTrack {
    pub(super) const fn new(member_entity: Entity, segments: Vec<FoldSegment>) -> Self {
        Self {
            member_entity,
            segments,
        }
    }

    /// Returns the member this track moves.
    #[must_use]
    pub const fn member_entity(&self) -> Entity { self.member_entity }

    /// Returns this member's segments in stage order.
    #[must_use]
    pub fn segments(&self) -> &[FoldSegment] { &self.segments }

    /// Samples this track at a normalized sequence position.
    ///
    /// The result depends only on `normalized_position`, so forward, backward,
    /// and arbitrary sampling agree. When segments of one member overlap, the
    /// last one the position has entered wins.
    ///
    /// A position that names no place — NaN — sits before no segment start and
    /// after no segment end, so it enters every segment and reads as the last
    /// one: just entered when that segment has extent, fully travelled when it
    /// has none, because a zero-extent segment is completed by reaching it.
    /// [`FoldSequence::sample_member`](super::FoldSequence::sample_member) maps
    /// such a position to the sequence start before it reaches here.
    #[must_use]
    pub fn sample(&self, normalized_position: f64) -> FoldMemberSample<'_> {
        let mut sampled = FoldMemberSample::Resting {
            target: FoldTarget::BASE,
        };
        for segment in &self.segments {
            if segment.normalized_begin() > normalized_position {
                break;
            }
            sampled = if normalized_position >= segment.normalized_end() {
                FoldMemberSample::Resting {
                    target: segment.to(),
                }
            } else {
                FoldMemberSample::Moving {
                    segment,
                    raw_progress: segment.raw_progress(normalized_position),
                }
            };
        }
        sampled
    }
}

/// One member's entry into or exit from an authored segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FoldMemberBoundary {
    /// Authored position of the stage that owns the segment.
    pub stage_ordinal: usize,
    /// Member's group-local index within that stage.
    pub member_index:  usize,
    /// Member whose segment starts or ends here.
    pub member_entity: Entity,
}

/// What one ordered ledger record marks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldBoundary {
    /// A stage starts.
    StageBegin {
        /// Authored position of the stage.
        stage_ordinal: usize,
    },
    /// One member's segment starts.
    MemberBegin(FoldMemberBoundary),
    /// One member's segment ends.
    MemberEnd(FoldMemberBoundary),
    /// A stage ends.
    StageEnd {
        /// Authored position of the stage.
        stage_ordinal: usize,
    },
    /// The sequence reached one of its two endpoints.
    Endpoint(FoldEndpoint),
}

/// One ordered boundary of an authored fold sequence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldBoundaryRecord {
    boundary:            FoldBoundary,
    elapsed:             SequenceTime,
    normalized_position: f64,
}

impl FoldBoundaryRecord {
    /// Returns what this record marks.
    #[must_use]
    pub const fn boundary(&self) -> FoldBoundary { self.boundary }

    /// Returns the exact authored time of this record.
    #[must_use]
    pub const fn elapsed(&self) -> SequenceTime { self.elapsed }

    /// Returns the normalized sequence position of this record.
    #[must_use]
    pub const fn normalized_position(&self) -> f64 { self.normalized_position }
}

/// The ordered boundary records of one authored fold sequence.
///
/// Record order is the one
/// [`SequencePlayback`](bevy_kana::SequencePlayback) documents for coincident
/// fold boundaries: the exiting stage's member ends, that stage's end, the next
/// stage's begin, then its member begins in group order, with a zero-duration
/// member's begin and end kept adjacent and a reached endpoint last.
/// [`SequenceTraversal::boundaries`](bevy_kana::SequenceTraversal::boundaries)
/// reverses exactly this order for backward travel.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldLedger {
    records: Vec<FoldBoundaryRecord>,
}

impl FoldLedger {
    /// Returns every record in ledger order.
    #[must_use]
    pub fn records(&self) -> &[FoldBoundaryRecord] { &self.records }

    /// Returns the normalized positions
    /// [`SequencePlayback::try_new`](bevy_kana::SequencePlayback::try_new)
    /// accepts, in the same ordinal order as [`Self::records`].
    pub fn boundary_positions(&self) -> impl Iterator<Item = f64> {
        self.records
            .iter()
            .map(FoldBoundaryRecord::normalized_position)
    }

    /// Builds the ledger from placed stages and resolved member segments.
    pub(super) fn new(
        placed_stages: &[PlacedStage<'_>],
        segments: &[FoldSegment],
        total: SequenceTime,
    ) -> Self {
        let mut records = Vec::with_capacity(placed_stages.len() * 2 + segments.len() * 2 + 2);
        for placed in placed_stages {
            let stage_ordinal = placed.stage_ordinal;
            let span = placed.span;
            records.push(FoldBoundaryRecord {
                boundary:            FoldBoundary::StageBegin { stage_ordinal },
                elapsed:             span.start(),
                normalized_position: normalized(span.start(), total),
            });
            records.push(FoldBoundaryRecord {
                boundary:            FoldBoundary::StageEnd { stage_ordinal },
                elapsed:             span.end(),
                normalized_position: normalized(span.end(), total),
            });
        }
        for segment in segments {
            let member_boundary = FoldMemberBoundary {
                stage_ordinal: segment.stage_ordinal(),
                member_index:  segment.member_index(),
                member_entity: segment.member_entity(),
            };
            records.push(FoldBoundaryRecord {
                boundary:            FoldBoundary::MemberBegin(member_boundary),
                elapsed:             segment.begin(),
                normalized_position: segment.normalized_begin(),
            });
            records.push(FoldBoundaryRecord {
                boundary:            FoldBoundary::MemberEnd(member_boundary),
                elapsed:             segment.end(),
                normalized_position: segment.normalized_end(),
            });
        }
        records.push(FoldBoundaryRecord {
            boundary:            FoldBoundary::Endpoint(FoldEndpoint::Base),
            elapsed:             SequenceTime::ZERO,
            normalized_position: SEQUENCE_START,
        });
        records.push(FoldBoundaryRecord {
            boundary:            FoldBoundary::Endpoint(FoldEndpoint::Folded),
            elapsed:             total,
            normalized_position: SEQUENCE_END,
        });
        records.sort_by_key(ledger_order);

        Self { records }
    }
}

/// Returns `elapsed` as a normalized position within `total`.
///
/// A sequence with no extent has no interior, so every authored time maps to
/// the start.
pub(super) fn normalized(elapsed: SequenceTime, total: SequenceTime) -> f64 {
    if total.is_zero() {
        return SEQUENCE_START;
    }
    (elapsed.as_seconds_f64() / total.as_seconds_f64()).clamp(SEQUENCE_START, SEQUENCE_END)
}

/// Sort key that produces the documented coincident-record order.
///
/// Field order is the tie-break order: exact time, then stage, then what the
/// record marks within that stage, then group-local member index, then a
/// member's begin before its end.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LedgerOrder {
    whole_seconds:      u128,
    subsec_nanoseconds: u32,
    stage:              LedgerStage,
    phase:              BoundaryPhase,
    member_index:       usize,
    edge:               SegmentEdge,
}

/// Which stage's records a coincident position belongs to.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum LedgerStage {
    /// Records of the stage at this authored position.
    Stage(usize),
    /// Records that follow every stage sharing this position.
    AfterEveryStage,
}

/// Order of coincident records within one stage.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum BoundaryPhase {
    StageBegin,
    Member,
    StageEnd,
    BaseEndpoint,
    FoldedEndpoint,
}

/// Order of one member's two records at one position.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SegmentEdge {
    Begin,
    End,
}

const fn ledger_order(record: &FoldBoundaryRecord) -> LedgerOrder {
    let (whole_seconds, subsec_nanoseconds) = stage::exact_parts(record.elapsed);
    let (stage, phase, member_index, edge) = match record.boundary {
        FoldBoundary::StageBegin { stage_ordinal } => (
            LedgerStage::Stage(stage_ordinal),
            BoundaryPhase::StageBegin,
            0,
            SegmentEdge::Begin,
        ),
        FoldBoundary::MemberBegin(member_boundary) => (
            LedgerStage::Stage(member_boundary.stage_ordinal),
            BoundaryPhase::Member,
            member_boundary.member_index,
            SegmentEdge::Begin,
        ),
        FoldBoundary::MemberEnd(member_boundary) => (
            LedgerStage::Stage(member_boundary.stage_ordinal),
            BoundaryPhase::Member,
            member_boundary.member_index,
            SegmentEdge::End,
        ),
        FoldBoundary::StageEnd { stage_ordinal } => (
            LedgerStage::Stage(stage_ordinal),
            BoundaryPhase::StageEnd,
            0,
            SegmentEdge::End,
        ),
        FoldBoundary::Endpoint(FoldEndpoint::Base) => (
            LedgerStage::AfterEveryStage,
            BoundaryPhase::BaseEndpoint,
            0,
            SegmentEdge::Begin,
        ),
        FoldBoundary::Endpoint(FoldEndpoint::Folded) => (
            LedgerStage::AfterEveryStage,
            BoundaryPhase::FoldedEndpoint,
            0,
            SegmentEdge::End,
        ),
    };

    LedgerOrder {
        whole_seconds,
        subsec_nanoseconds,
        stage,
        phase,
        member_index,
        edge,
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exactly representable normalized positions"
)]
mod tests {
    use std::time::Duration;

    use bevy_ecs::world::World;
    use bevy_math::curve::EaseFunction;

    use super::*;
    use crate::FoldGroup;
    use crate::FoldSequence;
    use crate::FoldSequenceBuilder;
    use crate::FoldStage;

    const FOUR_SECONDS: Duration = Duration::from_secs(4);
    const SECOND: Duration = Duration::from_secs(1);
    const TWO_SECONDS: Duration = Duration::from_secs(2);

    fn members(count: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..count).map(|_| world.spawn_empty().id()).collect()
    }

    fn stage(member_entities: &[Entity]) -> FoldStage {
        match FoldGroup::try_from_iter(member_entities.iter().copied()) {
            Ok(group) => FoldStage::from(group),
            Err(error) => panic!("test fixture group {member_entities:?} was rejected: {error:?}"),
        }
    }

    fn timing(duration: Duration) -> FoldTiming { FoldTiming::new(duration, EaseFunction::Linear) }

    fn moved(
        stage_ordinal: usize,
        member_index: usize,
        member_entity: Entity,
    ) -> FoldMemberBoundary {
        FoldMemberBoundary {
            stage_ordinal,
            member_index,
            member_entity,
        }
    }

    fn boundaries(sequence: &FoldSequence) -> Vec<FoldBoundary> {
        sequence
            .ledger()
            .records()
            .iter()
            .map(FoldBoundaryRecord::boundary)
            .collect()
    }

    fn assert_positions_never_go_backwards(sequence: &FoldSequence) {
        let positions = sequence.ledger().boundary_positions().collect::<Vec<_>>();
        assert!(
            positions.windows(2).all(|pair| pair[0] <= pair[1]),
            "ledger positions ran backwards: {positions:?}"
        );
    }

    #[test]
    fn a_stage_change_orders_the_exiting_stage_before_the_entering_one() {
        let member_entities = members(1);
        let member_entity = member_entities[0];
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stage(stage(&member_entities))
            .stage(stage(&member_entities))
            .build();

        assert_eq!(
            boundaries(&sequence),
            [
                FoldBoundary::StageBegin { stage_ordinal: 0 },
                FoldBoundary::MemberBegin(moved(0, 0, member_entity)),
                FoldBoundary::Endpoint(FoldEndpoint::Base),
                FoldBoundary::MemberEnd(moved(0, 0, member_entity)),
                FoldBoundary::StageEnd { stage_ordinal: 0 },
                FoldBoundary::StageBegin { stage_ordinal: 1 },
                FoldBoundary::MemberBegin(moved(1, 0, member_entity)),
                FoldBoundary::MemberEnd(moved(1, 0, member_entity)),
                FoldBoundary::StageEnd { stage_ordinal: 1 },
                FoldBoundary::Endpoint(FoldEndpoint::Folded),
            ]
        );
        assert_positions_never_go_backwards(&sequence);
    }

    #[test]
    fn a_staggered_stage_keeps_its_member_boundaries_inside_its_own_extent() {
        let member_entities = members(3);
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stage(
                stage(&member_entities).override_member_timings_with(|member_index, _| {
                    timing(SECOND)
                        .with_start_offset(SECOND * u32::try_from(member_index).unwrap_or_default())
                }),
            )
            .build();

        assert_eq!(
            boundaries(&sequence),
            [
                FoldBoundary::StageBegin { stage_ordinal: 0 },
                FoldBoundary::MemberBegin(moved(0, 0, member_entities[0])),
                FoldBoundary::Endpoint(FoldEndpoint::Base),
                FoldBoundary::MemberEnd(moved(0, 0, member_entities[0])),
                FoldBoundary::MemberBegin(moved(0, 1, member_entities[1])),
                FoldBoundary::MemberEnd(moved(0, 1, member_entities[1])),
                FoldBoundary::MemberBegin(moved(0, 2, member_entities[2])),
                FoldBoundary::MemberEnd(moved(0, 2, member_entities[2])),
                FoldBoundary::StageEnd { stage_ordinal: 0 },
                FoldBoundary::Endpoint(FoldEndpoint::Folded),
            ]
        );
        assert_positions_never_go_backwards(&sequence);
    }

    #[test]
    fn a_zero_duration_member_at_a_stage_end_keeps_its_pair_before_that_stage_end() {
        let member_entities = members(2);
        let sequence = FoldSequenceBuilder::new(timing(TWO_SECONDS))
            .stage(
                stage(&member_entities).override_member_timings_with(|member_index, _| {
                    if member_index == 0 {
                        timing(TWO_SECONDS)
                    } else {
                        FoldTiming::snap().with_start_offset(TWO_SECONDS)
                    }
                }),
            )
            .build();

        assert_eq!(sequence.total(), SequenceTime::from(TWO_SECONDS));
        assert_eq!(
            boundaries(&sequence),
            [
                FoldBoundary::StageBegin { stage_ordinal: 0 },
                FoldBoundary::MemberBegin(moved(0, 0, member_entities[0])),
                FoldBoundary::Endpoint(FoldEndpoint::Base),
                FoldBoundary::MemberEnd(moved(0, 0, member_entities[0])),
                FoldBoundary::MemberBegin(moved(0, 1, member_entities[1])),
                FoldBoundary::MemberEnd(moved(0, 1, member_entities[1])),
                FoldBoundary::StageEnd { stage_ordinal: 0 },
                FoldBoundary::Endpoint(FoldEndpoint::Folded),
            ]
        );
        assert_positions_never_go_backwards(&sequence);
    }

    #[test]
    fn a_sequence_with_no_extent_maps_every_authored_time_to_its_start() {
        assert_eq!(
            normalized(SequenceTime::from(SECOND), SequenceTime::ZERO),
            SEQUENCE_START
        );
        assert_eq!(
            normalized(SequenceTime::from(SECOND), SequenceTime::from(FOUR_SECONDS)),
            0.25
        );
        assert_eq!(
            normalized(SequenceTime::from(FOUR_SECONDS), SequenceTime::from(SECOND)),
            SEQUENCE_END
        );
    }
}
