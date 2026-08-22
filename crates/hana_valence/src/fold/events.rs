//! Stage, member, and endpoint events one fold sequence emits from raw
//! traversal.
//!
//! Every event here is derived from the immutable ledger
//! [`FoldSequence`](super::FoldSequence) built once at authoring time and from
//! the [`SequenceTraversal`](bevy_kana::SequenceTraversal) the shared layer
//! returned for this update's movement. Eased output never reaches an event:
//! an owned curve changes only what a member's pose interpolates, while a
//! rejected curve or non-finite sample holds the pose. Each leaves raw position,
//! raw traversal, and these events alone.

use bevy_ecs::entity::Entity;
use bevy_ecs::event::EntityEvent;
use bevy_ecs::reflect::ReflectEvent;
use bevy_ecs::system::Commands;
use bevy_kana::SequenceDirection;
use bevy_kana::SequencePosition;
use bevy_kana::SequenceStageId;
use bevy_kana::SequenceTime;
use bevy_kana::SequenceUpdate;
use bevy_kana::ToF32;
use bevy_reflect::Reflect;

use super::FoldBoundary;
use super::FoldBoundaryRecord;
use super::FoldEndpoint;
use super::FoldMemberBoundary;
use super::FoldSegment;
use super::FoldSequence;
use super::FoldTiming;
use super::constants::SEQUENCE_END;
use super::constants::SEQUENCE_START;
use super::playback::SequenceReport;

/// Exactly when one crossed boundary sits inside its own sequence.
///
/// The three values are the raw ledger's, not eased output: `elapsed` is the
/// boundary's authored time, `total` is the whole sequence's authored extent,
/// and `position` is the boundary's normalized place. Position is carried
/// rather than divided out of the other two, because a sequence whose stages
/// are all zero-duration has `elapsed` and `total` both zero at every boundary
/// and only `position` still separates its two endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct FoldEventTiming {
    elapsed:  SequenceTime,
    total:    SequenceTime,
    position: SequencePosition,
}

impl FoldEventTiming {
    /// Returns the exact authored time of the crossed boundary.
    #[must_use]
    pub const fn elapsed(self) -> SequenceTime { self.elapsed }

    /// Returns the exact authored extent of the whole sequence.
    #[must_use]
    pub const fn total(self) -> SequenceTime { self.total }

    /// Returns the normalized position of the crossed boundary.
    #[must_use]
    pub const fn position(self) -> SequencePosition { self.position }

    /// Reads one ledger record's exact place within a sequence of `total`
    /// extent.
    ///
    /// Every record's normalized position is finite and already clamped into
    /// `SEQUENCE_START..=SEQUENCE_END` when the ledger is built, so a value
    /// outside that range means the ledger and playback disagree and the
    /// debug assertion fails the test that produced it. A release build clamps
    /// to the bound the value overshot, which keeps the endpoint an observer
    /// reads from `position` the one the record actually names. Only NaN, which
    /// no clamp resolves, reads as the sequence start.
    fn of(record: &FoldBoundaryRecord, total: SequenceTime) -> Self {
        let normalized_position = record.normalized_position();
        debug_assert!(
            (SEQUENCE_START..=SEQUENCE_END).contains(&normalized_position),
            "every FoldBoundaryRecord position is built within the normalized sequence",
        );
        Self {
            elapsed: record.elapsed(),
            total,
            position: SequencePosition::try_new(
                normalized_position
                    .clamp(SEQUENCE_START, SEQUENCE_END)
                    .to_f32(),
            )
            .unwrap_or(SequencePosition::START),
        }
    }
}

/// A stage started travelling.
///
/// Travelling backward through a stage's authored end starts that stage, so
/// this names what the sequence did rather than which record it crossed.
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Event)]
pub struct FoldStageBegin {
    /// Arrangement whose retained sequence crossed the boundary.
    #[event_target]
    pub arrangement: Entity,
    /// Stage that began.
    pub stage:       SequenceStageId,
    /// Direction the sequence was travelling.
    pub direction:   SequenceDirection,
    /// Exact place of the crossed boundary.
    pub timing:      FoldEventTiming,
}

/// A stage finished travelling.
///
/// Travelling backward through a stage's authored start finishes that stage.
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Event)]
pub struct FoldStageEnd {
    /// Arrangement whose retained sequence crossed the boundary.
    #[event_target]
    pub arrangement: Entity,
    /// Stage that ended.
    pub stage:       SequenceStageId,
    /// Direction the sequence was travelling.
    pub direction:   SequenceDirection,
    /// Exact place of the crossed boundary.
    pub timing:      FoldEventTiming,
}

/// One member started travelling its segment inside one stage.
///
/// The event targets the member, so an observer watching a single hinged entity
/// receives only its own movement. Reflection is opaque because
/// [`FoldTiming`] carries an authored [`Easing`](bevy_kana::Easing) that is not
/// structurally reflected.
#[derive(EntityEvent, Clone, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
#[reflect(Event)]
pub struct FoldMemberBegin {
    /// Member whose segment started.
    #[event_target]
    pub member:        Entity,
    /// Arrangement whose retained sequence crossed the boundary.
    pub arrangement:   Entity,
    /// Stage that owns the segment.
    pub stage:         SequenceStageId,
    /// Direction the sequence was travelling.
    pub direction:     SequenceDirection,
    /// Exact place of the crossed boundary.
    pub timing:        FoldEventTiming,
    /// Delay, extent, and curve resolved for this member in that stage.
    pub member_timing: FoldTiming,
}

/// One member finished travelling its segment inside one stage.
///
/// Reflection is opaque for the reason [`FoldMemberBegin`] states.
#[derive(EntityEvent, Clone, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
#[reflect(Event)]
pub struct FoldMemberEnd {
    /// Member whose segment finished.
    #[event_target]
    pub member:        Entity,
    /// Arrangement whose retained sequence crossed the boundary.
    pub arrangement:   Entity,
    /// Stage that owns the segment.
    pub stage:         SequenceStageId,
    /// Direction the sequence was travelling.
    pub direction:     SequenceDirection,
    /// Exact place of the crossed boundary.
    pub timing:        FoldEventTiming,
    /// Delay, extent, and curve resolved for this member in that stage.
    pub member_timing: FoldTiming,
}

/// Travel crossed one of the sequence's two endpoints, arriving or departing.
///
/// An endpoint record is crossed in both directions, so a forward play that
/// starts at the base emits `endpoint: FoldEndpoint::Base` as it leaves and a
/// backward play emits `FoldEndpoint::Folded` as it leaves. `endpoint` and
/// `direction` together name which of the two happened: travel that reaches an
/// endpoint moves toward it — forward to
/// [`FoldEndpoint::Folded`](super::FoldEndpoint::Folded), backward to
/// [`FoldEndpoint::Base`](super::FoldEndpoint::Base) — and the other two
/// pairings name a departure.
#[derive(EntityEvent, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Event)]
pub struct FoldEndpointReached {
    /// Arrangement whose retained sequence crossed the endpoint.
    #[event_target]
    pub arrangement: Entity,
    /// Endpoint that was crossed.
    pub endpoint:    FoldEndpoint,
    /// Direction the sequence was travelling.
    pub direction:   SequenceDirection,
    /// Exact place of the crossed boundary.
    pub timing:      FoldEventTiming,
}

/// Which side of a stage's or member's authored span one record marks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpanEdge {
    Start,
    End,
}

/// What crossing a span edge did to the span, in the direction it was crossed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpanTransition {
    Entered,
    Exited,
}

impl SpanTransition {
    /// Reads what crossing `edge` while travelling `direction` did.
    const fn of(edge: SpanEdge, direction: SequenceDirection) -> Self {
        match (edge, direction) {
            (SpanEdge::Start, SequenceDirection::Forward)
            | (SpanEdge::End, SequenceDirection::Backward) => Self::Entered,
            (SpanEdge::Start, SequenceDirection::Backward)
            | (SpanEdge::End, SequenceDirection::Forward) => Self::Exited,
        }
    }
}

/// Which authored lookup a traversed boundary failed, leaving it unemitted.
///
/// Each names one of the three lookups
/// [`emit_fold_boundaries`] makes against the same authored
/// [`FoldSequence`] the traversal indexed: the ledger record at a crossed
/// ordinal, the [`SequenceStageId`] of a record's stage ordinal, and the
/// [`FoldSegment`] a member boundary was built from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnansweredBoundaryLookup {
    LedgerRecord,
    StageIdentity,
    MemberSegment,
}

/// Emits one event per boundary this update's raw traversal crossed.
///
/// [`SequenceUpdate::NoTraversal`] emits nothing, which covers a hold, an
/// unchanged position, and movement the shared layer refused to apply. Every
/// other update yields its crossed ledger ordinals in the traversal's own
/// direction, so a large seek emits every boundary between the two positions
/// and a multi-wrap traversal emits every crossing of every repetition.
///
/// `boundary_report` is the caller's warn-once latch for the one condition
/// this cannot repair: a crossed ordinal the authored ledger does not answer.
pub(super) fn emit_fold_boundaries(
    commands: &mut Commands,
    arrangement: Entity,
    sequence: &FoldSequence,
    update: SequenceUpdate,
    boundary_report: &mut SequenceReport,
) {
    let SequenceUpdate::Traversed {
        traversal,
        direction,
        ..
    } = update
    else {
        return;
    };
    let records = sequence.ledger().records();
    let total = sequence.total();
    let mut emitter = FoldBoundaryEmitter {
        arrangement,
        sequence,
        direction,
        boundary_report,
    };
    for ordinal in traversal.boundaries() {
        let Some(record) = records.get(ordinal) else {
            emitter.report_skipped(UnansweredBoundaryLookup::LedgerRecord);
            continue;
        };
        let timing = FoldEventTiming::of(record, total);
        match record.boundary() {
            FoldBoundary::StageBegin { stage_ordinal } => {
                emitter.emit_stage(commands, stage_ordinal, SpanEdge::Start, timing);
            },
            FoldBoundary::StageEnd { stage_ordinal } => {
                emitter.emit_stage(commands, stage_ordinal, SpanEdge::End, timing);
            },
            FoldBoundary::MemberBegin(member_boundary) => {
                emitter.emit_member(commands, member_boundary, SpanEdge::Start, timing);
            },
            FoldBoundary::MemberEnd(member_boundary) => {
                emitter.emit_member(commands, member_boundary, SpanEdge::End, timing);
            },
            FoldBoundary::Endpoint(endpoint) => commands.trigger(FoldEndpointReached {
                arrangement,
                endpoint,
                direction,
                timing,
            }),
        }
    }
}

/// One update's crossed boundaries, and what every one of them is emitted
/// against.
///
/// Arrangement, authored sequence, and travel direction are the same for every
/// boundary of one update. `boundary_report` is the
/// [`FoldSequencePlayback`](super::FoldSequencePlayback) latch that warns once
/// per sequence when an authored lookup goes unanswered.
struct FoldBoundaryEmitter<'a> {
    arrangement:     Entity,
    sequence:        &'a FoldSequence,
    direction:       SequenceDirection,
    boundary_report: &'a mut SequenceReport,
}

impl FoldBoundaryEmitter<'_> {
    /// Emits one stage event on the arrangement.
    fn emit_stage(
        &mut self,
        commands: &mut Commands,
        stage_ordinal: usize,
        edge: SpanEdge,
        timing: FoldEventTiming,
    ) {
        let Some(stage) = self.stage_id(stage_ordinal) else {
            return;
        };
        let arrangement = self.arrangement;
        let direction = self.direction;
        match SpanTransition::of(edge, direction) {
            SpanTransition::Entered => commands.trigger(FoldStageBegin {
                arrangement,
                stage,
                direction,
                timing,
            }),
            SpanTransition::Exited => commands.trigger(FoldStageEnd {
                arrangement,
                stage,
                direction,
                timing,
            }),
        }
    }

    /// Emits one member event on the member itself.
    fn emit_member(
        &mut self,
        commands: &mut Commands,
        member_boundary: FoldMemberBoundary,
        edge: SpanEdge,
        timing: FoldEventTiming,
    ) {
        let sequence = self.sequence;
        let Some(stage) = self.stage_id(member_boundary.stage_ordinal) else {
            return;
        };
        let Some(segment) = crossed_segment(sequence, member_boundary) else {
            self.report_skipped(UnansweredBoundaryLookup::MemberSegment);
            return;
        };
        let arrangement = self.arrangement;
        let direction = self.direction;
        let member = member_boundary.member_entity;
        let member_timing = segment.timing().clone();
        match SpanTransition::of(edge, direction) {
            SpanTransition::Entered => commands.trigger(FoldMemberBegin {
                member,
                arrangement,
                stage,
                direction,
                timing,
                member_timing,
            }),
            SpanTransition::Exited => commands.trigger(FoldMemberEnd {
                member,
                arrangement,
                stage,
                direction,
                timing,
                member_timing,
            }),
        }
    }

    /// Reads the stage identity `stage_ordinal` names, reporting a miss.
    ///
    /// The identity comes from the same description the ledger's stage
    /// ordinals were built from, so this answers every record the authored
    /// sequence produced.
    fn stage_id(&mut self, stage_ordinal: usize) -> Option<SequenceStageId> {
        let stage = self.sequence.sequence_stages().stage_id(stage_ordinal).ok();
        if stage.is_none() {
            self.report_skipped(UnansweredBoundaryLookup::StageIdentity);
        }
        stage
    }

    /// Warns once per sequence that a crossed boundary emitted no event.
    ///
    /// Every [`UnansweredBoundaryLookup`] is answered by the same authored
    /// [`FoldSequence`] the traversal indexed, so a miss means retained
    /// playback and that ledger disagree rather than that authoring was
    /// rejected. The latch is the one the other
    /// [`FoldSequencePlayback`](super::FoldSequencePlayback) reports use: one
    /// warning per sequence.
    fn report_skipped(&mut self, lookup: UnansweredBoundaryLookup) {
        if self.boundary_report.report() {
            tracing::warn!(
                arrangement = ?self.arrangement,
                lookup = ?lookup,
                "fold traversal crossed a boundary the authored ledger does not answer",
            );
        }
    }
}

/// Finds the one segment a member boundary was built from.
///
/// A member repeated across stages owns one segment per stage, and one stage
/// places it at one group-local index, so the stage and index a boundary
/// carries name exactly one of that member's segments.
fn crossed_segment(
    sequence: &FoldSequence,
    member_boundary: FoldMemberBoundary,
) -> Option<&FoldSegment> {
    sequence
        .tracks()
        .iter()
        .find(|track| track.member_entity() == member_boundary.member_entity)
        .and_then(|track| {
            track.segments().iter().find(|segment| {
                segment.stage_ordinal() == member_boundary.stage_ordinal
                    && segment.member_index() == member_boundary.member_index
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backward_travel_swaps_what_each_span_edge_means() {
        assert_eq!(
            SpanTransition::of(SpanEdge::Start, SequenceDirection::Forward),
            SpanTransition::Entered
        );
        assert_eq!(
            SpanTransition::of(SpanEdge::End, SequenceDirection::Forward),
            SpanTransition::Exited
        );
        assert_eq!(
            SpanTransition::of(SpanEdge::Start, SequenceDirection::Backward),
            SpanTransition::Exited
        );
        assert_eq!(
            SpanTransition::of(SpanEdge::End, SequenceDirection::Backward),
            SpanTransition::Entered
        );
    }
}
