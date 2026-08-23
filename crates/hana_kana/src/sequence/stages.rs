use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bevy::ecs::component::Component;
use bevy::ecs::reflect::ReflectComponent;
use bevy::reflect::Reflect;
use thiserror::Error;

use super::playback::SequencePosition;
use super::time::SequenceTime;

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

/// Identity of one immutable authored stage description.
///
/// Replacing a target's [`SequenceStages`] produces a new revision, so a scope
/// resolved against the previous description is detectably stale.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct SequenceStagesRevision(u64);

/// Stable identity of one stage within one authored description.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct SequenceStageId {
    revision: SequenceStagesRevision,
    ordinal:  usize,
}

impl SequenceStageId {
    /// Returns the description revision this identity belongs to.
    #[must_use]
    pub const fn revision(self) -> SequenceStagesRevision { self.revision }

    /// Returns the authored order of this stage.
    #[must_use]
    pub const fn ordinal(self) -> usize { self.ordinal }
}

/// Exact authored extent of one stage.
///
/// A zero-duration stage keeps equal start and end times, so coincident
/// boundaries survive in authored order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct SequenceStageSpan {
    start: SequenceTime,
    end:   SequenceTime,
}

impl SequenceStageSpan {
    /// Returns the exact elapsed time where this stage begins.
    #[must_use]
    pub const fn start(self) -> SequenceTime { self.start }

    /// Returns the exact elapsed time where this stage ends.
    #[must_use]
    pub const fn end(self) -> SequenceTime { self.end }
}

/// Normalized extent a resolved [`SequenceScope`] selects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequenceRange {
    revision: SequenceStagesRevision,
    start:    SequencePosition,
    end:      SequencePosition,
    span:     SequenceStageSpan,
}

impl SequenceRange {
    /// Returns the description revision this range was resolved against.
    #[must_use]
    pub const fn revision(self) -> SequenceStagesRevision { self.revision }

    /// Returns the normalized start of the selected extent.
    #[must_use]
    pub const fn start(self) -> SequencePosition { self.start }

    /// Returns the normalized end of the selected extent.
    #[must_use]
    pub const fn end(self) -> SequencePosition { self.end }

    /// Returns the exact authored extent this range covers.
    #[must_use]
    pub const fn span(self) -> SequenceStageSpan { self.span }

    /// Returns `position` as progress within this range.
    ///
    /// A zero-width range has no interior, so every position maps to `0.0`.
    #[must_use]
    pub fn progress(self, position: SequencePosition) -> f32 {
        let start = self.start.normalized();
        let width = self.end.normalized() - start;
        if width <= 0.0 {
            return 0.0;
        }
        ((position.normalized() - start) / width).clamp(0.0, 1.0)
    }
}

/// Which stages of a sequence an evaluation selects.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Reflect)]
pub enum SequenceScope {
    /// Every authored stage.
    #[default]
    WholeSequence,
    /// Exactly one authored stage.
    Stage(SequenceStageId),
    /// A contiguous inclusive run of authored stages.
    StageRange {
        /// First selected stage.
        first: SequenceStageId,
        /// Last selected stage.
        last:  SequenceStageId,
    },
}

/// Immutable ordered stage description shared by tools and transport.
///
/// Domains keep their own authored values and evaluators and publish this
/// description on the sequence entity so shared arbitration, transport, and
/// tools can resolve a [`SequenceScope`] without domain knowledge. An empty
/// description and an all-zero description are both valid.
#[derive(Component, Clone, Debug, Eq, PartialEq, Reflect)]
#[reflect(opaque)]
#[reflect(Component)]
pub struct SequenceStages {
    revision: SequenceStagesRevision,
    spans:    Vec<SequenceStageSpan>,
    total:    SequenceTime,
}

impl SequenceStages {
    /// Describes stages from authored durations in stage order.
    #[must_use]
    pub fn new(durations: impl IntoIterator<Item = Duration>) -> Self {
        let mut elapsed = SequenceTime::ZERO;
        let spans = durations
            .into_iter()
            .map(|duration| {
                let start = elapsed;
                elapsed = elapsed.advanced_by(duration);
                SequenceStageSpan {
                    start,
                    end: elapsed,
                }
            })
            .collect();
        Self {
            revision: SequenceStagesRevision(NEXT_REVISION.fetch_add(1, Ordering::Relaxed)),
            spans,
            total: elapsed,
        }
    }

    /// Returns this description's identity.
    #[must_use]
    pub const fn revision(&self) -> SequenceStagesRevision { self.revision }

    /// Returns the exact sum of every authored stage duration.
    #[must_use]
    pub const fn total(&self) -> SequenceTime { self.total }

    /// Returns how many stages this description contains.
    #[must_use]
    pub const fn len(&self) -> usize { self.spans.len() }

    /// Returns whether this description contains no stage.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.spans.is_empty() }

    /// Returns the ordered stage identities of this description.
    pub fn stage_ids(&self) -> impl Iterator<Item = SequenceStageId> {
        let revision = self.revision;
        (0..self.spans.len()).map(move |ordinal| SequenceStageId { revision, ordinal })
    }

    /// Returns each stage's identity paired with its exact authored extent, in
    /// stage order.
    ///
    /// The pairing is infallible where [`Self::span`] is not: both values are
    /// built from the same position in this description's own stages, so no
    /// identity makes the round trip out and back that a stale revision or an
    /// unknown ordinal could fail.
    pub fn stage_ids_with_spans(
        &self,
    ) -> impl Iterator<Item = (SequenceStageId, SequenceStageSpan)> {
        let revision = self.revision;
        self.spans
            .iter()
            .copied()
            .enumerate()
            .map(move |(ordinal, span)| (SequenceStageId { revision, ordinal }, span))
    }

    /// Returns the identity of the stage at `ordinal`.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceScopeError::UnknownStage`] when `ordinal` is beyond the
    /// described stages.
    pub const fn stage_id(&self, ordinal: usize) -> Result<SequenceStageId, SequenceScopeError> {
        if ordinal >= self.spans.len() {
            return Err(SequenceScopeError::UnknownStage {
                ordinal,
                stages: self.spans.len(),
            });
        }
        Ok(SequenceStageId {
            revision: self.revision,
            ordinal,
        })
    }

    /// Returns the exact authored extent of one stage.
    ///
    /// # Errors
    ///
    /// Returns [`SequenceScopeError::StaleRevision`] for an identity from a
    /// replaced description and [`SequenceScopeError::UnknownStage`] for an
    /// ordinal this description does not contain.
    pub fn span(&self, stage_id: SequenceStageId) -> Result<SequenceStageSpan, SequenceScopeError> {
        self.validate(stage_id)?;
        Ok(self.spans[stage_id.ordinal])
    }

    /// Resolves the normalized extent a scope selects.
    ///
    /// # Errors
    ///
    /// Returns the exact [`SequenceScopeError`] for a stale identity, an unknown
    /// stage, or a reversed stage range.
    pub fn resolve(&self, scope: SequenceScope) -> Result<SequenceRange, SequenceScopeError> {
        let span = match scope {
            SequenceScope::WholeSequence => SequenceStageSpan {
                start: SequenceTime::ZERO,
                end:   self.total,
            },
            SequenceScope::Stage(stage_id) => self.span(stage_id)?,
            SequenceScope::StageRange { first, last } => {
                if first.ordinal > last.ordinal {
                    return Err(SequenceScopeError::RangeOutOfOrder {
                        first: first.ordinal,
                        last:  last.ordinal,
                    });
                }
                SequenceStageSpan {
                    start: self.span(first)?.start,
                    end:   self.span(last)?.end,
                }
            },
        };

        let (start, end) = match scope {
            SequenceScope::WholeSequence => (SequencePosition::START, SequencePosition::END),
            SequenceScope::Stage(_) | SequenceScope::StageRange { .. } => (
                SequencePosition::clamped(span.start.fraction_of(self.total)),
                SequencePosition::clamped(span.end.fraction_of(self.total)),
            ),
        };
        Ok(SequenceRange {
            revision: self.revision,
            start,
            end,
            span,
        })
    }

    fn validate(&self, stage_id: SequenceStageId) -> Result<(), SequenceScopeError> {
        if stage_id.revision != self.revision {
            return Err(SequenceScopeError::StaleRevision {
                scope:       stage_id.revision,
                description: self.revision,
            });
        }
        if stage_id.ordinal >= self.spans.len() {
            return Err(SequenceScopeError::UnknownStage {
                ordinal: stage_id.ordinal,
                stages:  self.spans.len(),
            });
        }
        Ok(())
    }
}

/// Errors returned while resolving a [`SequenceScope`].
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SequenceScopeError {
    /// The scope named a stage from a replaced description.
    #[error("scope revision {scope:?} was replaced by description revision {description:?}")]
    StaleRevision {
        /// Revision the scope was resolved against.
        scope:       SequenceStagesRevision,
        /// Revision the description currently carries.
        description: SequenceStagesRevision,
    },
    /// The scope named a stage this description does not contain.
    #[error("stage {ordinal} is beyond the {stages} described stages")]
    UnknownStage {
        /// Requested stage ordinal.
        ordinal: usize,
        /// Number of described stages.
        stages:  usize,
    },
    /// A stage range ended before it began.
    #[error("stage range {first}..={last} is reversed")]
    RangeOutOfOrder {
        /// First selected stage ordinal.
        first: usize,
        /// Last selected stage ordinal.
        last:  usize,
    },
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exactly representable normalized extents"
)]
mod tests {
    use bevy::reflect::FromReflect;
    use bevy::reflect::PartialReflect;
    use bevy::reflect::ReflectKind;
    use bevy::reflect::structs::DynamicStruct;

    use super::*;

    const FIRST_SECONDS: u64 = 1;
    const LAST_SECONDS: u64 = 3;

    fn three_stages() -> SequenceStages {
        SequenceStages::new([
            Duration::from_secs(FIRST_SECONDS),
            Duration::ZERO,
            Duration::from_secs(LAST_SECONDS),
        ])
    }

    #[test]
    fn empty_and_zero_duration_descriptions_resolve_the_whole_sequence() {
        let empty = SequenceStages::new([]);
        let whole = empty
            .resolve(SequenceScope::WholeSequence)
            .expect("the whole sequence always resolves");

        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert!(empty.total().is_zero());
        assert_eq!(whole.start(), SequencePosition::START);
        assert_eq!(whole.end(), SequencePosition::END);

        let all_zero = SequenceStages::new([Duration::ZERO, Duration::ZERO]);
        let coincident = all_zero
            .resolve(SequenceScope::Stage(
                all_zero.stage_id(1).expect("stage 1 exists"),
            ))
            .expect("a described stage resolves");
        assert_eq!(coincident.start(), coincident.end());
        assert_eq!(coincident.progress(SequencePosition::END), 0.0);
    }

    #[test]
    fn identities_paired_with_spans_agree_with_reading_each_one_separately() {
        for stages in [
            SequenceStages::new([]),
            SequenceStages::new([Duration::ZERO]),
            SequenceStages::new([Duration::from_secs(FIRST_SECONDS)]),
            three_stages(),
        ] {
            let paired = stages.stage_ids_with_spans().collect::<Vec<_>>();
            let separate = stages
                .stage_ids()
                .map(|stage_id| (stage_id, stages.span(stage_id).expect("a described stage")))
                .collect::<Vec<_>>();

            assert_eq!(paired, separate);
            assert_eq!(paired.len(), stages.len());
        }

        assert_eq!(SequenceStages::new([]).stage_ids_with_spans().count(), 0);
    }

    #[test]
    fn maximum_duration_spans_stay_exact() {
        let stages = SequenceStages::new([Duration::MAX, Duration::MAX]);
        let span = stages
            .span(stages.stage_id(1).expect("stage 1 exists"))
            .expect("stage 1 has a span");

        assert_eq!(span.start(), SequenceTime::from(Duration::MAX));
        assert_eq!(stages.total().whole_seconds(), u128::from(u64::MAX) * 2 + 1);
        assert!(span.end().as_seconds_f64().is_finite());
    }

    #[test]
    fn stage_and_range_scopes_resolve_normalized_extents() {
        let stages = three_stages();
        let first = stages.stage_id(0).expect("stage 0 exists");
        let last = stages.stage_id(2).expect("stage 2 exists");

        let one_stage = stages
            .resolve(SequenceScope::Stage(first))
            .expect("stage 0 resolves");
        assert_eq!(one_stage.start(), SequencePosition::START);
        assert_eq!(one_stage.end().normalized(), 0.25);

        let stage_range = stages
            .resolve(SequenceScope::StageRange { first, last })
            .expect("an ordered stage range resolves");
        assert_eq!(stage_range.start(), SequencePosition::START);
        assert_eq!(stage_range.end(), SequencePosition::END);
        assert_eq!(stage_range.progress(SequencePosition::END), 1.0);
    }

    #[test]
    fn scope_resolution_reports_every_exact_error_variant() {
        let stages = three_stages();
        let replaced = three_stages();
        let stale_stage = stages.stage_id(0).expect("stage 0 exists");
        let first = replaced.stage_id(0).expect("stage 0 exists");
        let last = replaced.stage_id(2).expect("stage 2 exists");

        assert_eq!(
            replaced.resolve(SequenceScope::Stage(stale_stage)),
            Err(SequenceScopeError::StaleRevision {
                scope:       stages.revision(),
                description: replaced.revision(),
            })
        );
        assert_eq!(
            replaced.stage_id(3),
            Err(SequenceScopeError::UnknownStage {
                ordinal: 3,
                stages:  3,
            })
        );
        assert_eq!(
            replaced.resolve(SequenceScope::StageRange {
                first: last,
                last:  first,
            }),
            Err(SequenceScopeError::RangeOutOfOrder { first: 2, last: 0 })
        );
    }

    #[test]
    fn stage_description_reflection_is_opaque() {
        let mut stages = three_stages();
        let described_total = stages.total();
        let described_len = stages.len();

        let mut structural_patch = DynamicStruct::default();
        structural_patch.insert("total", SequenceTime::ZERO);
        structural_patch.insert("spans", Vec::<SequenceStageSpan>::new());

        let first_span = stages
            .span(stages.stage_id(0).expect("stage 0 exists"))
            .expect("stage 0 has a span");

        assert_eq!(stages.reflect_kind(), ReflectKind::Opaque);
        assert_eq!(first_span.reflect_kind(), ReflectKind::Opaque);
        assert!(stages.try_apply(&structural_patch).is_err());
        assert!(SequenceStages::from_reflect(&structural_patch).is_none());
        assert_eq!(stages.total(), described_total);
        assert_eq!(stages.len(), described_len);
    }
}
