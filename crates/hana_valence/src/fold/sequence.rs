use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Component;
use bevy_ecs::prelude::ReflectComponent;
use bevy_reflect::Reflect;
use hana_kana::SequenceStageSpan;
use hana_kana::SequenceStages;
use hana_kana::SequenceTime;

use super::FoldLedger;
use super::FoldMemberSample;
use super::FoldMemberTrack;
use super::FoldSegment;
use super::FoldStage;
use super::FoldTarget;
use super::FoldTiming;
use super::constants::SEQUENCE_END;
use super::constants::SEQUENCE_START;
use super::ledger;

/// An authored fold sequence: ordered stages plus the timing they inherit.
///
/// Construction resolves everything playback needs and nothing changes
/// afterwards: the exact stage extents as a
/// [`SequenceStages`] description, one ordered [`FoldMemberTrack`] per unique
/// member, and the ordered [`FoldLedger`]. A member repeated across stages gets
/// one segment per appearance, each starting at the target its previous segment
/// reached, so sampling is history independent. A sequence with no stage is
/// valid.
///
/// This is also the component arrangement materialization inserts on the
/// arrangement controller when a provider authored a sequence. Reflection is
/// opaque because the resolved tracks and ledger inside are derived from the
/// authored stages during construction and must never be reached structurally.
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(opaque)]
#[reflect(Component)]
pub struct FoldSequence {
    stages:          Vec<FoldStage>,
    default_timing:  FoldTiming,
    sequence_stages: Box<SequenceStages>,
    tracks:          Vec<FoldMemberTrack>,
    ledger:          FoldLedger,
}

impl FoldSequence {
    /// Returns the authored stages in fold order.
    #[must_use]
    pub fn stages(&self) -> &[FoldStage] { &self.stages }

    /// Returns the timing stages and members inherit when they do not override
    /// it.
    #[must_use]
    pub const fn default_timing(&self) -> &FoldTiming { &self.default_timing }

    /// Returns the shared stage description transport and tools resolve scopes
    /// against.
    ///
    /// The description is built once here, so its stage identities stay stable
    /// for the lifetime of this value. Replacing the authored definition
    /// produces a new description with a new revision.
    #[must_use]
    pub const fn sequence_stages(&self) -> &SequenceStages { &self.sequence_stages }

    /// Returns the exact sum of every authored stage duration.
    #[must_use]
    pub const fn total(&self) -> SequenceTime { self.sequence_stages.total() }

    /// Returns one track per unique member, in first-appearance order.
    #[must_use]
    pub fn tracks(&self) -> &[FoldMemberTrack] { &self.tracks }

    /// Returns the ordered boundary ledger.
    #[must_use]
    pub const fn ledger(&self) -> &FoldLedger { &self.ledger }

    /// Samples one member at a normalized sequence position.
    ///
    /// A non-finite position names no place in the sequence and samples the
    /// start. An entity this sequence never folds samples
    /// [`FoldMemberSample::Unauthored`].
    #[must_use]
    pub fn sample_member(
        &self,
        member_entity: Entity,
        normalized_position: f64,
    ) -> FoldMemberSample<'_> {
        let normalized_position = if normalized_position.is_finite() {
            normalized_position.clamp(SEQUENCE_START, SEQUENCE_END)
        } else {
            SEQUENCE_START
        };
        self.tracks
            .iter()
            .find(|track| track.member_entity() == member_entity)
            .map_or(FoldMemberSample::Unauthored, |track| {
                track.sample(normalized_position)
            })
    }

    fn new(default_timing: FoldTiming, stages: Vec<FoldStage>) -> Self {
        let sequence_stages = SequenceStages::new(stages.iter().map(|stage| {
            stage
                .longest_member_timing(&default_timing)
                .stage_duration()
        }));
        let total = sequence_stages.total();
        let placed_stages = place_stages(&stages, &sequence_stages);

        let segments = resolve_segments(&placed_stages, &default_timing, total);
        let ledger = FoldLedger::new(&placed_stages, &segments, total);
        let tracks = resolve_tracks(segments);

        Self {
            stages,
            default_timing,
            sequence_stages: Box::new(sequence_stages),
            tracks,
            ledger,
        }
    }
}

/// Collects ordered stages into a [`FoldSequence`].
///
/// The builder owns no `Commands` and reaches no `World`; every constituent
/// value enforces its own invariants, so [`Self::build`] cannot fail.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldSequenceBuilder {
    default_timing: FoldTiming,
    stages:         Vec<FoldStage>,
}

impl FoldSequenceBuilder {
    /// Starts a sequence whose stages inherit `default_timing`.
    #[must_use]
    pub const fn new(default_timing: FoldTiming) -> Self {
        Self {
            default_timing,
            stages: Vec::new(),
        }
    }

    /// Appends one stage.
    #[must_use]
    pub fn stage(mut self, stage: impl Into<FoldStage>) -> Self {
        self.stages.push(stage.into());
        self
    }

    /// Appends several stages in order.
    #[must_use]
    pub fn stages<I, S>(mut self, stages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<FoldStage>,
    {
        self.stages.extend(stages.into_iter().map(Into::into));
        self
    }

    /// Resolves the authored stages into an immutable sequence.
    #[must_use]
    pub fn build(self) -> FoldSequence { FoldSequence::new(self.default_timing, self.stages) }
}

/// One authored stage carrying the exact extent the published description gave
/// it.
///
/// A stage's authored position and its extent travel with the stage, so nothing
/// downstream addresses either by indexing a parallel vector.
#[derive(Clone, Copy, Debug)]
pub(super) struct PlacedStage<'stages> {
    pub(super) stage:         &'stages FoldStage,
    pub(super) stage_ordinal: usize,
    pub(super) span:          SequenceStageSpan,
}

/// Places every authored stage at the extent the published description gave it.
///
/// [`SequenceStages::new`] accumulated one span per authored stage a moment
/// earlier and in the same order, so the description describes exactly
/// `stages`. [`SequenceStages::stage_ids_with_spans`] reads each identity and
/// its extent from the same position in that description, so placement cannot
/// be rejected and every authored stage is placed.
fn place_stages<'stages>(
    stages: &'stages [FoldStage],
    sequence_stages: &SequenceStages,
) -> Vec<PlacedStage<'stages>> {
    stages
        .iter()
        .zip(sequence_stages.stage_ids_with_spans())
        .map(|(stage, (stage_id, span))| PlacedStage {
            stage,
            stage_ordinal: stage_id.ordinal(),
            span,
        })
        .collect()
}

/// Resolves every stage member into one segment with exact absolute times.
fn resolve_segments(
    placed_stages: &[PlacedStage<'_>],
    default_timing: &FoldTiming,
    total: SequenceTime,
) -> Vec<FoldSegment> {
    let mut segments = Vec::new();
    let mut reached = Vec::<(Entity, FoldTarget)>::new();
    for placed in placed_stages {
        let PlacedStage {
            stage,
            stage_ordinal,
            span,
        } = *placed;
        let stage_start = span.start();
        for (member_index, member_entity) in stage.group().iter().copied().enumerate() {
            let timing = stage.resolved_timing(member_index, default_timing);
            let begin = stage_start.advanced_by(timing.start_offset);
            let end = stage_start.advanced_by(timing.stage_duration());
            let from = reached
                .iter()
                .find(|(reached_entity, _)| *reached_entity == member_entity)
                .map_or(FoldTarget::BASE, |(_, target)| *target);
            let to = stage.targets()[member_index];

            segments.push(FoldSegment {
                member_entity,
                stage_ordinal,
                member_index,
                from,
                to,
                begin,
                end,
                normalized_begin: ledger::normalized(begin, total),
                normalized_end: ledger::normalized(end, total),
                timing: timing.clone(),
            });

            match reached
                .iter_mut()
                .find(|(reached_entity, _)| *reached_entity == member_entity)
            {
                Some((_, target)) => *target = to,
                None => reached.push((member_entity, to)),
            }
        }
    }
    segments
}

/// Groups segments into one track per unique member, in first-appearance order.
fn resolve_tracks(segments: Vec<FoldSegment>) -> Vec<FoldMemberTrack> {
    let mut member_entities = Vec::<Entity>::new();
    let mut tracked = Vec::<Vec<FoldSegment>>::new();
    for segment in segments {
        let member_entity = segment.member_entity();
        if let Some(track_index) = member_entities
            .iter()
            .position(|tracked_entity| *tracked_entity == member_entity)
        {
            tracked[track_index].push(segment);
        } else {
            member_entities.push(member_entity);
            tracked.push(vec![segment]);
        }
    }

    member_entities
        .into_iter()
        .zip(tracked)
        .map(|(member_entity, segments)| FoldMemberTrack::new(member_entity, segments))
        .collect()
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
    use hana_kana::SequenceStageId;

    use super::*;
    use crate::FoldAuthorError;
    use crate::FoldBoundary;
    use crate::FoldEndpoint;
    use crate::FoldGroup;

    const HALF_FOLDED: f32 = 0.5;
    const SECOND: Duration = Duration::from_secs(1);
    const SIX_SECONDS: Duration = Duration::from_secs(6);
    const THREE_SECONDS: Duration = Duration::from_secs(3);
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

    fn stagger(member_index: usize) -> Duration {
        SECOND * u32::try_from(member_index).unwrap_or_default()
    }

    /// What one sample reports, freed from the borrow of the segment it read.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum SampledPlace {
        Unauthored,
        Resting {
            target: FoldTarget,
        },
        Moving {
            stage_ordinal: usize,
            from:          FoldTarget,
            to:            FoldTarget,
            raw_progress:  f32,
        },
    }

    const fn moving(
        stage_ordinal: usize,
        from: FoldTarget,
        to: FoldTarget,
        raw_progress: f32,
    ) -> SampledPlace {
        SampledPlace::Moving {
            stage_ordinal,
            from,
            to,
            raw_progress,
        }
    }

    const fn resting(target: FoldTarget) -> SampledPlace { SampledPlace::Resting { target } }

    fn sampled_place(
        sequence: &FoldSequence,
        member_entity: Entity,
        normalized_position: f64,
    ) -> SampledPlace {
        match sequence.sample_member(member_entity, normalized_position) {
            FoldMemberSample::Unauthored => SampledPlace::Unauthored,
            FoldMemberSample::Resting { target } => resting(target),
            FoldMemberSample::Moving {
                segment,
                raw_progress,
            } => moving(
                segment.stage_ordinal(),
                segment.from(),
                segment.to(),
                raw_progress.normalized(),
            ),
        }
    }

    #[test]
    fn a_sequence_with_no_stage_is_valid_and_holds_only_its_endpoints() {
        let sequence = FoldSequenceBuilder::new(timing(SECOND)).build();
        let unauthored = members(1)[0];

        assert!(sequence.stages().is_empty());
        assert!(sequence.tracks().is_empty());
        assert!(sequence.sequence_stages().is_empty());
        assert_eq!(sequence.total(), SequenceTime::ZERO);
        assert_eq!(sequence.ledger().records().len(), 2);
        assert_eq!(
            sequence.sample_member(unauthored, 0.5),
            FoldMemberSample::Unauthored
        );
    }

    #[test]
    fn each_stage_keeps_a_stable_identity_and_its_exact_authored_span() {
        let member_entities = members(3);
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stages([
                stage(&member_entities[0..1]),
                stage(&member_entities[1..2]).override_timing(timing(TWO_SECONDS)),
                stage(&member_entities[2..3]).override_timing(timing(THREE_SECONDS)),
            ])
            .build();
        let sequence_stages = sequence.sequence_stages();

        assert_eq!(sequence_stages.len(), 3);
        assert_eq!(sequence.total(), SequenceTime::from(SIX_SECONDS));
        assert_eq!(
            sequence_stages
                .stage_ids()
                .map(SequenceStageId::ordinal)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(
            sequence_stages
                .stage_ids()
                .all(|stage_id| stage_id.revision() == sequence_stages.revision())
        );

        let authored_spans = [
            (Duration::ZERO, SECOND),
            (SECOND, THREE_SECONDS),
            (THREE_SECONDS, SIX_SECONDS),
        ];
        for (ordinal, (start, end)) in authored_spans.into_iter().enumerate() {
            let Ok(stage_id) = sequence_stages.stage_id(ordinal) else {
                panic!("stage {ordinal} has no identity");
            };
            let Ok(span) = sequence_stages.span(stage_id) else {
                panic!("stage {ordinal} has no span");
            };
            assert_eq!(span.start(), SequenceTime::from(start));
            assert_eq!(span.end(), SequenceTime::from(end));
        }
    }

    #[test]
    fn a_repeated_member_starts_each_segment_where_its_previous_one_ended()
    -> Result<(), FoldAuthorError> {
        let member_entities = members(2);
        let half_folded = FoldTarget::try_new(HALF_FOLDED)?;
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stage(stage(&member_entities).with_member_target(member_entities[0], half_folded)?)
            .stage(stage(&member_entities[0..1]))
            .build();
        let tracks = sequence.tracks();

        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].member_entity(), member_entities[0]);
        assert_eq!(tracks[1].member_entity(), member_entities[1]);

        let repeated = tracks[0].segments();
        assert_eq!(repeated.len(), 2);
        assert_eq!(
            (repeated[0].from(), repeated[0].to()),
            (FoldTarget::BASE, half_folded)
        );
        assert_eq!(
            (repeated[1].from(), repeated[1].to()),
            (half_folded, FoldTarget::FOLDED)
        );
        assert_eq!(tracks[1].segments().len(), 1);
        Ok(())
    }

    #[test]
    fn every_sampled_position_of_a_repeated_member_reports_the_same_place_in_any_order()
    -> Result<(), FoldAuthorError> {
        let member_entities = members(1);
        let member_entity = member_entities[0];
        let half_folded = FoldTarget::try_new(HALF_FOLDED)?;
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stage(stage(&member_entities).with_member_target(member_entity, half_folded)?)
            .stage(stage(&member_entities))
            .build();
        let first_segment = moving(0, FoldTarget::BASE, half_folded, 0.0);
        let sampled = [
            (0.0, first_segment),
            (0.25, moving(0, FoldTarget::BASE, half_folded, 0.5)),
            (0.5, moving(1, half_folded, FoldTarget::FOLDED, 0.0)),
            (0.75, moving(1, half_folded, FoldTarget::FOLDED, 0.5)),
            (1.0, resting(FoldTarget::FOLDED)),
        ];

        for (position, place) in sampled {
            assert_eq!(
                sampled_place(&sequence, member_entity, position),
                place,
                "sampling {position} in order"
            );
        }
        for index in [3_usize, 0, 4, 2, 1] {
            let (position, place) = sampled[index];
            assert_eq!(
                sampled_place(&sequence, member_entity, position),
                place,
                "sampling {position} out of order"
            );
        }

        assert_eq!(
            sampled_place(&sequence, member_entity, f64::NAN),
            first_segment
        );
        assert_eq!(sampled_place(&sequence, member_entity, -1.0), first_segment);
        assert_eq!(
            sampled_place(&sequence, member_entity, 2.0),
            resting(FoldTarget::FOLDED)
        );
        Ok(())
    }

    #[test]
    fn sampling_inside_a_segment_reports_its_stage_and_raw_progress() {
        let member_entities = members(1);
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stage(stage(&member_entities))
            .stage(stage(&member_entities))
            .build();

        let FoldMemberSample::Moving {
            segment,
            raw_progress,
        } = sequence.sample_member(member_entities[0], 0.25)
        else {
            panic!("a quarter of the way in the first stage is still moving");
        };
        assert_eq!(segment.stage_ordinal(), 0);
        assert_eq!(raw_progress.normalized(), 0.5);
    }

    #[test]
    fn simultaneous_members_share_one_stage_extent() {
        let member_entities = members(3);
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stage(stage(&member_entities))
            .build();

        assert_eq!(sequence.total(), SequenceTime::from(SECOND));
        for track in sequence.tracks() {
            let segment = &track.segments()[0];
            assert_eq!(segment.begin(), SequenceTime::ZERO);
            assert_eq!(segment.end(), SequenceTime::from(SECOND));
        }
    }

    #[test]
    fn sequential_stages_run_one_after_another() {
        let member_entities = members(2);
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stage(stage(&member_entities[0..1]))
            .stage(stage(&member_entities[1..2]))
            .build();
        let second_stage = &sequence.tracks()[1].segments()[0];

        assert_eq!(sequence.total(), SequenceTime::from(TWO_SECONDS));
        assert_eq!(second_stage.begin(), SequenceTime::from(SECOND));
        assert_eq!(second_stage.normalized_begin(), 0.5);
    }

    #[test]
    fn staggered_members_wave_inside_one_stage_without_splitting_it() {
        let member_entities = members(3);
        let sequence = FoldSequenceBuilder::new(timing(SECOND))
            .stage(
                stage(&member_entities).override_member_timings_with(|member_index, _| {
                    timing(SECOND).with_start_offset(stagger(member_index))
                }),
            )
            .build();

        assert_eq!(sequence.sequence_stages().len(), 1);
        assert_eq!(sequence.total(), SequenceTime::from(THREE_SECONDS));
        for (member_index, track) in sequence.tracks().iter().enumerate() {
            let segment = &track.segments()[0];
            assert_eq!(segment.begin(), SequenceTime::from(stagger(member_index)));
            assert_eq!(
                segment.end(),
                SequenceTime::from(stagger(member_index) + SECOND)
            );
        }
    }

    #[test]
    fn snap_timing_completes_its_member_where_the_stage_begins() {
        let member_entities = members(1);
        let sequence = FoldSequenceBuilder::new(FoldTiming::snap())
            .stage(stage(&member_entities))
            .build();

        assert_eq!(sequence.total(), SequenceTime::ZERO);
        assert_eq!(
            sequence.sample_member(member_entities[0], 0.0),
            FoldMemberSample::Resting {
                target: FoldTarget::FOLDED,
            }
        );
    }

    #[test]
    fn maximum_stage_durations_sum_exactly_across_the_sequence() {
        let member_entities = members(2);
        let sequence =
            FoldSequenceBuilder::new(FoldTiming::new(Duration::MAX, EaseFunction::Linear))
                .stage(stage(&member_entities[0..1]))
                .stage(stage(&member_entities[1..2]))
                .build();
        let sequence_stages = sequence.sequence_stages();

        assert_eq!(
            sequence.total().whole_seconds(),
            u128::from(u64::MAX) * 2 + 1
        );
        assert_eq!(sequence.total().subsec_nanoseconds(), 999_999_998);

        let Ok(stage_id) = sequence_stages.stage_id(1) else {
            panic!("the second stage has no identity");
        };
        let Ok(span) = sequence_stages.span(stage_id) else {
            panic!("the second stage has no span");
        };
        assert_eq!(span.start(), SequenceTime::from(Duration::MAX));
        assert_eq!(span.end(), sequence.total());
    }

    #[test]
    fn a_member_whose_timing_saturates_still_ends_inside_its_own_stage() {
        let member_entities = members(1);
        let sequence = FoldSequenceBuilder::new(
            FoldTiming::new(Duration::MAX, EaseFunction::Linear).with_start_offset(Duration::MAX),
        )
        .stage(stage(&member_entities))
        .build();
        let sequence_stages = sequence.sequence_stages();
        let segment = &sequence.tracks()[0].segments()[0];

        let Ok(stage_id) = sequence_stages.stage_id(0) else {
            panic!("the only stage has no identity");
        };
        let Ok(span) = sequence_stages.span(stage_id) else {
            panic!("the only stage has no span");
        };
        assert_eq!(span.end(), SequenceTime::from(Duration::MAX));
        assert_eq!(segment.end(), span.end());

        let Some(last) = sequence.ledger().records().last() else {
            panic!("every ledger holds its two endpoints");
        };
        assert_eq!(
            last.boundary(),
            FoldBoundary::Endpoint(FoldEndpoint::Folded)
        );
    }
}
