//! Provider adapters that author a fold sequence from a plan's own groups.
//!
//! Both adapters wrap an existing [`ArrangementProvider`] and add one authored
//! [`FoldSequence`] to the plan that provider already produces. The wrapper
//! types stay private: the trait conveniences return
//! `impl ArrangementProvider`, so an application names only the provider it
//! started with.

use super::ArrangementError;
use super::ArrangementMemberEntities;
use super::ArrangementPlan;
use super::ArrangementProvider;
use crate::FoldAuthorError;
use crate::FoldGroups;
use crate::FoldSequence;
use crate::FoldSequenceBuilder;
use crate::FoldTiming;

/// An [`ArrangementProvider`] that also authors one fold sequence.
///
/// `sequence_for_groups` runs once per plan generation, after the wrapped
/// provider produced a valid plan and that plan yielded the groups retained
/// under `selection`. It is [`Fn`] rather than [`FnOnce`] because
/// [`ArrangementProvider::generate_plan`] takes `&self` and may run more than
/// once; a once-only closure would need consumed state and a failure path for
/// the second call.
pub(super) struct FoldSequenceProvider<P, F>
where
    P: ArrangementProvider,
{
    provider:            P,
    selection:           P::FoldGroupSelection,
    sequence_for_groups: F,
}

impl<P, F> FoldSequenceProvider<P, F>
where
    P: ArrangementProvider,
    F: Fn(&FoldGroups) -> Result<FoldSequence, FoldAuthorError>,
{
    pub(super) const fn new(
        provider: P,
        selection: P::FoldGroupSelection,
        sequence_for_groups: F,
    ) -> Self {
        Self {
            provider,
            selection,
            sequence_for_groups,
        }
    }
}

impl<P, F> ArrangementProvider for FoldSequenceProvider<P, F>
where
    P: ArrangementProvider,
    F: Fn(&FoldGroups) -> Result<FoldSequence, FoldAuthorError>,
{
    type FoldGroupSelection = P::FoldGroupSelection;
    type Member = P::Member;

    fn members(&self) -> impl Iterator<Item = Self::Member> { self.provider.members() }

    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
        let plan = self.provider.generate_plan(members)?;
        let fold_sequence = {
            let groups = plan.fold_groups(&self.selection)?;
            (self.sequence_for_groups)(groups).map_err(ArrangementError::provider)?
        };

        plan.with_authored_fold_sequence(fold_sequence)
    }
}

/// Authors one stage per selected group, in the provider's group order.
///
/// Every stage inherits `default_timing` and folds each of its members to
/// [`FoldTarget::FOLDED`](crate::FoldTarget::FOLDED), which is the
/// ordinary "unfold this sheet one group at a time" sequence. A provider that
/// wants anything else authors it through
/// [`ArrangementProvider::with_custom_fold_sequence`].
pub(super) fn stage_per_group(groups: &FoldGroups, default_timing: FoldTiming) -> FoldSequence {
    FoldSequenceBuilder::new(default_timing)
        .stages(groups.iter().cloned())
        .build()
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use core::cell::Cell;
    use std::time::Duration;

    use bevy_ecs::entity::Entity;
    use bevy_ecs::world::World;
    use bevy_kana::SequenceTime;
    use bevy_math::curve::EaseFunction;

    use super::*;
    use crate::AnchorSite;
    use crate::AnchoredTo;
    use crate::Angle;
    use crate::ArrangementConnection;
    use crate::Edge;
    use crate::FoldGroup;
    use crate::HingeClearance;
    use crate::PlannedFoldSequence;

    const SECOND: Duration = Duration::from_secs(1);

    /// Which fold-group alternative a [`Chain`] retains.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum ChainSelection {
        /// One group per hinged member, innermost first.
        OnePerMember,
        /// A selection the provider never retains.
        Unauthored,
    }

    /// Whether a [`Chain`] retains a fold-group alternative in its plan.
    #[derive(Clone, Copy, Debug)]
    enum ChainGroups {
        /// The plan retains one group per hinged member.
        Retained,
        /// The plan retains no alternative at all.
        Unretained,
    }

    /// A root plus `hinged` members chained onto it.
    struct Chain {
        hinged: usize,
        groups: ChainGroups,
    }

    impl ArrangementProvider for Chain {
        type FoldGroupSelection = ChainSelection;
        type Member = usize;

        fn members(&self) -> impl Iterator<Item = Self::Member> { 0..=self.hinged }

        fn generate_plan(
            &self,
            members: &ArrangementMemberEntities<Self::Member>,
        ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError> {
            let mut connections = Vec::new();
            for member in 1..=self.hinged {
                connections.push(ArrangementConnection {
                    member_entity:   members.entity(&member)?,
                    anchored_to:     AnchoredTo::new(
                        members.entity(&(member - 1))?,
                        AnchorSite::Vertex(0),
                        AnchorSite::Vertex(1),
                    ),
                    member_edge:     Edge {
                        start: AnchorSite::Vertex(0),
                        end:   AnchorSite::Vertex(1),
                    },
                    base_angle:      Angle::default(),
                    hinge_clearance: HingeClearance::CENTERED,
                });
            }
            let plan = ArrangementPlan::try_new(members, connections)?;
            let ChainGroups::Retained = self.groups else {
                return Ok(plan);
            };
            let groups = (1..=self.hinged)
                .map(|member| members.entity(&member).map(FoldGroup::from))
                .collect::<Result<Vec<_>, _>>()?;
            let Ok(groups) = FoldGroups::try_from_iter(groups) else {
                return Ok(plan);
            };

            plan.with_fold_groups(ChainSelection::OnePerMember, groups)
        }
    }

    fn entities(count: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..count).map(|_| world.spawn_empty().id()).collect()
    }

    fn member_entities(count: usize) -> ArrangementMemberEntities<usize> {
        match ArrangementMemberEntities::try_new(entities(count).into_iter().enumerate()) {
            Ok(members) => members,
            Err(error) => panic!("distinct member entities were rejected: {error:?}"),
        }
    }

    fn timing() -> FoldTiming { FoldTiming::new(SECOND, EaseFunction::Linear) }

    #[test]
    fn one_stage_per_selected_group_inherits_the_default_timing() {
        let members = member_entities(3);
        let provider = Chain {
            hinged: 2,
            groups: ChainGroups::Retained,
        }
        .with_fold_sequence(ChainSelection::OnePerMember, timing());

        let Ok(plan) = provider.generate_plan(&members) else {
            panic!("the standard adapter rejected a provider that retains its selection");
        };
        let PlannedFoldSequence::Authored(sequence) = plan.fold_sequence() else {
            panic!("the standard adapter authored no sequence");
        };

        assert_eq!(sequence.stages().len(), 2);
        assert_eq!(sequence.default_timing(), &timing());
        assert_eq!(sequence.total(), SequenceTime::from(SECOND * 2));
    }

    #[test]
    fn a_custom_closure_may_reshape_every_group_and_runs_once_per_plan_generation() {
        let members = member_entities(4);
        let runs = Cell::new(0_usize);
        let provider = Chain {
            hinged: 3,
            groups: ChainGroups::Retained,
        }
        .with_custom_fold_sequence(ChainSelection::OnePerMember, |groups| {
            runs.set(runs.get() + 1);
            let combined = FoldGroup::combine(groups.iter())?;
            Ok(
                FoldSequenceBuilder::new(FoldTiming::new(SECOND, EaseFunction::Linear))
                    .stage(combined)
                    .build(),
            )
        });

        for expected_runs in 1..=2 {
            let Ok(plan) = provider.generate_plan(&members) else {
                panic!("the custom adapter rejected its own closure output");
            };
            let PlannedFoldSequence::Authored(sequence) = plan.fold_sequence() else {
                panic!("the custom adapter authored no sequence");
            };
            assert_eq!(sequence.stages().len(), 1);
            assert_eq!(sequence.tracks().len(), 3);
            assert_eq!(runs.get(), expected_runs);
        }
    }

    #[test]
    fn a_closure_failure_leaves_the_plan_unauthored_and_keeps_its_source() {
        let members = member_entities(2);
        let provider = Chain {
            hinged: 1,
            groups: ChainGroups::Retained,
        }
        .with_custom_fold_sequence(ChainSelection::OnePerMember, |_| {
            Err(FoldAuthorError::EmptyFoldGroups)
        });

        let Err(error) = provider.generate_plan(&members) else {
            panic!("a failing closure must fail the plan");
        };
        let ArrangementError::Provider { source } = &error else {
            panic!("a closure failure keeps its source: {error:?}");
        };
        assert_eq!(
            source.downcast_ref::<FoldAuthorError>(),
            Some(&FoldAuthorError::EmptyFoldGroups),
        );
    }

    #[test]
    fn a_custom_sequence_that_tracks_an_entity_outside_the_arrangement_authors_nothing() {
        let spawned = entities(4);
        let Some((&outside, listed)) = spawned.split_last() else {
            panic!("four spawned entities always split");
        };
        let members = match ArrangementMemberEntities::try_new(listed.iter().copied().enumerate()) {
            Ok(members) => members,
            Err(error) => panic!("distinct member entities were rejected: {error:?}"),
        };
        let provider = Chain {
            hinged: 2,
            groups: ChainGroups::Retained,
        }
        .with_custom_fold_sequence(ChainSelection::OnePerMember, move |_| {
            Ok(FoldSequenceBuilder::new(timing())
                .stage(FoldGroup::from(outside))
                .build())
        });

        let error = provider.generate_plan(&members);
        let Err(ArrangementError::ForeignFoldGroupMember { member_entity }) = error else {
            panic!("a tracked member outside the arrangement must be named exactly: {error:?}");
        };
        assert_eq!(member_entity, outside);
    }

    #[test]
    fn stacking_both_conveniences_rejects_the_second_authored_sequence() {
        let members = member_entities(3);
        let provider = Chain {
            hinged: 2,
            groups: ChainGroups::Retained,
        }
        .with_fold_sequence(ChainSelection::OnePerMember, timing())
        .with_custom_fold_sequence(ChainSelection::OnePerMember, |groups| {
            let combined = FoldGroup::combine(groups.iter())?;
            Ok(FoldSequenceBuilder::new(timing()).stage(combined).build())
        });

        let error = provider.generate_plan(&members);
        let Err(ArrangementError::DuplicateAuthoredFoldSequence) = error else {
            panic!("a second authored sequence must be rejected: {error:?}");
        };
    }

    #[test]
    fn a_selection_the_provider_never_retained_authors_nothing() {
        let members = member_entities(2);
        let unretained = Chain {
            hinged: 1,
            groups: ChainGroups::Unretained,
        }
        .with_fold_sequence(ChainSelection::OnePerMember, timing());
        let unknown = Chain {
            hinged: 1,
            groups: ChainGroups::Retained,
        }
        .with_custom_fold_sequence(ChainSelection::Unauthored, |_| {
            panic!("an unknown selection must fail before the closure runs")
        });

        for error in [
            unretained.generate_plan(&members),
            unknown.generate_plan(&members),
        ] {
            let Err(ArrangementError::UnknownFoldGroupSelection { .. }) = error else {
                panic!("an unretained selection must be named exactly: {error:?}");
            };
        }
    }
}
