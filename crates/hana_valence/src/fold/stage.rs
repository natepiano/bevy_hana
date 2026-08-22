use bevy_ecs::entity::Entity;
use bevy_kana::Cascade;
use bevy_kana::SequenceTime;
use bevy_kana::resolve_cascade_ref;

use super::FoldTarget;
use super::FoldTiming;
use crate::FoldAuthorError;
use crate::FoldGroup;

/// One synchronization step of an authored fold sequence.
///
/// A stage owns one [`FoldGroup`], one [`FoldTarget`] per group member, and the
/// authored timing overrides that sit between the sequence default and each
/// member. Every member of the group moves within the stage; overlapping
/// [`FoldTiming::start_offset`] values produce a wave inside one stage rather
/// than splitting it into several.
#[derive(Clone, Debug, PartialEq)]
pub struct FoldStage {
    group:          FoldGroup,
    targets:        Vec<FoldTarget>,
    timing:         Cascade<FoldTiming>,
    member_timings: Vec<Cascade<FoldTiming>>,
}

impl FoldStage {
    /// Replaces one member's destination for this stage.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::UnknownStageMember`] when `member_entity` is
    /// not in this stage's group.
    pub fn with_member_target(
        mut self,
        member_entity: Entity,
        fold_target: FoldTarget,
    ) -> Result<Self, FoldAuthorError> {
        let member_index = self.member_index(member_entity)?;
        self.targets[member_index] = fold_target;
        Ok(self)
    }

    /// Replaces the timing every member of this stage inherits.
    #[must_use]
    pub fn override_timing(mut self, fold_timing: FoldTiming) -> Self {
        self.timing = Cascade::Override(fold_timing);
        self
    }

    /// Returns this stage to inheriting the sequence default timing.
    #[must_use]
    pub fn inherit_timing(mut self) -> Self {
        self.timing = Cascade::Inherit;
        self
    }

    /// Replaces one member's timing for this stage.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::UnknownStageMember`] when `member_entity` is
    /// not in this stage's group.
    pub fn override_member_timing(
        mut self,
        member_entity: Entity,
        fold_timing: FoldTiming,
    ) -> Result<Self, FoldAuthorError> {
        let member_index = self.member_index(member_entity)?;
        self.member_timings[member_index] = Cascade::Override(fold_timing);
        Ok(self)
    }

    /// Returns one member to inheriting the stage timing.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::UnknownStageMember`] when `member_entity` is
    /// not in this stage's group.
    pub fn inherit_member_timing(mut self, member_entity: Entity) -> Result<Self, FoldAuthorError> {
        let member_index = self.member_index(member_entity)?;
        self.member_timings[member_index] = Cascade::Inherit;
        Ok(self)
    }

    /// Overrides every member's timing from its group-local index and entity.
    ///
    /// This is how a staggered stage is authored: `override_for` receives each
    /// member in group order and returns that member's complete timing.
    #[must_use]
    pub fn override_member_timings_with(
        mut self,
        mut override_for: impl FnMut(usize, Entity) -> FoldTiming,
    ) -> Self {
        for (member_index, member_entity) in self.group.iter().copied().enumerate() {
            self.member_timings[member_index] =
                Cascade::Override(override_for(member_index, member_entity));
        }
        self
    }

    /// Returns the members this stage folds, in group-local index order.
    #[must_use]
    pub const fn group(&self) -> &FoldGroup { &self.group }

    /// Returns each member's destination in group-local index order.
    #[must_use]
    pub fn targets(&self) -> &[FoldTarget] { &self.targets }

    /// Resolves one member's timing: member override, then stage override, then
    /// the sequence default.
    pub(super) fn resolved_timing<'timing>(
        &'timing self,
        member_index: usize,
        default_timing: &'timing FoldTiming,
    ) -> &'timing FoldTiming {
        resolve_cascade_ref(
            [&self.member_timings[member_index], &self.timing],
            default_timing,
        )
    }

    /// Returns the timing of the member this stage lasts longest for.
    ///
    /// Members are compared by their exact [`FoldTiming::end`], so a later
    /// starting member with a short movement can still decide the stage extent.
    pub(super) fn longest_member_timing<'timing>(
        &'timing self,
        default_timing: &'timing FoldTiming,
    ) -> &'timing FoldTiming {
        (1..self.targets.len())
            .map(|member_index| self.resolved_timing(member_index, default_timing))
            .fold(
                self.resolved_timing(0, default_timing),
                |longest, candidate| {
                    if exact_parts(candidate.end()) > exact_parts(longest.end()) {
                        candidate
                    } else {
                        longest
                    }
                },
            )
    }

    fn member_index(&self, member_entity: Entity) -> Result<usize, FoldAuthorError> {
        self.group
            .iter()
            .position(|group_member| *group_member == member_entity)
            .ok_or(FoldAuthorError::UnknownStageMember { member_entity })
    }
}

impl From<FoldGroup> for FoldStage {
    /// Folds every group member to [`FoldTarget::FOLDED`] with inherited timing.
    fn from(group: FoldGroup) -> Self {
        let member_count = group.iter().count();
        Self {
            group,
            targets: vec![FoldTarget::FOLDED; member_count],
            timing: Cascade::Inherit,
            member_timings: vec![Cascade::Inherit; member_count],
        }
    }
}

impl From<Entity> for FoldStage {
    fn from(member_entity: Entity) -> Self { Self::from(FoldGroup::from(member_entity)) }
}

/// Returns the exact comparable parts of a sequence time.
///
/// [`SequenceTime`] is `Eq` but not `Ord`, and its `as_seconds_f64` conversion
/// rounds, so ordering compares the exact whole-second and nanosecond parts.
pub(super) const fn exact_parts(sequence_time: SequenceTime) -> (u128, u32) {
    (
        sequence_time.whole_seconds(),
        sequence_time.subsec_nanoseconds(),
    )
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::time::Duration;

    use bevy_ecs::world::World;
    use bevy_math::curve::EaseFunction;

    use super::*;

    const LONG: Duration = Duration::from_secs(4);
    const OFFSET: Duration = Duration::from_secs(3);
    const SHORT: Duration = Duration::from_secs(1);

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

    fn default_timing() -> FoldTiming { FoldTiming::new(SHORT, EaseFunction::Linear) }

    #[test]
    fn a_stage_from_a_group_folds_every_member_and_inherits_timing() {
        let member_entities = members(2);
        let fold_stage = stage(&member_entities);
        let sequence_default = default_timing();

        assert_eq!(fold_stage.targets(), [FoldTarget::FOLDED; 2]);
        assert_eq!(
            fold_stage.resolved_timing(0, &sequence_default),
            &sequence_default
        );
        assert_eq!(
            FoldStage::from(member_entities[0]).group(),
            &FoldGroup::from(member_entities[0])
        );
    }

    #[test]
    fn member_timing_overrides_stage_timing_which_overrides_the_sequence_default()
    -> Result<(), FoldAuthorError> {
        let member_entities = members(2);
        let sequence_default = default_timing();
        let stage_override = FoldTiming::new(LONG, EaseFunction::Linear);
        let member_override = FoldTiming::new(OFFSET, EaseFunction::QuadraticIn);
        let fold_stage = stage(&member_entities)
            .override_timing(stage_override.clone())
            .override_member_timing(member_entities[0], member_override.clone())?;

        assert_eq!(
            fold_stage.resolved_timing(0, &sequence_default),
            &member_override
        );
        assert_eq!(
            fold_stage.resolved_timing(1, &sequence_default),
            &stage_override
        );

        let inherited = fold_stage
            .inherit_member_timing(member_entities[0])?
            .inherit_timing();
        assert_eq!(
            inherited.resolved_timing(0, &sequence_default),
            &sequence_default
        );
        Ok(())
    }

    #[test]
    fn the_stage_lasts_through_its_last_finishing_member() {
        let member_entities = members(3);
        let sequence_default = default_timing();
        let fold_stage = stage(&member_entities).override_member_timings_with(|index, _| {
            FoldTiming::new(SHORT, EaseFunction::Linear).with_start_offset(Duration::from_secs(
                u64::try_from(index).unwrap_or_default(),
            ))
        });

        assert_eq!(
            fold_stage
                .longest_member_timing(&sequence_default)
                .stage_duration(),
            Duration::from_secs(3)
        );
    }

    #[test]
    fn a_late_starting_short_member_can_still_decide_the_stage_extent() {
        let member_entities = members(2);
        let sequence_default = default_timing();
        let fold_stage = stage(&member_entities).override_member_timings_with(|index, _| {
            if index == 0 {
                FoldTiming::new(LONG, EaseFunction::Linear)
            } else {
                FoldTiming::new(SHORT, EaseFunction::Linear).with_start_offset(OFFSET)
            }
        });

        assert_eq!(
            fold_stage
                .longest_member_timing(&sequence_default)
                .stage_duration(),
            OFFSET + SHORT
        );
    }

    #[test]
    fn targets_and_timings_reject_an_entity_outside_the_group() {
        let member_entities = members(3);
        let foreign = member_entities[2];
        let fold_stage = stage(&member_entities[0..2]);

        assert_eq!(
            fold_stage
                .clone()
                .with_member_target(foreign, FoldTarget::BASE),
            Err(FoldAuthorError::UnknownStageMember {
                member_entity: foreign,
            })
        );
        assert_eq!(
            fold_stage
                .clone()
                .override_member_timing(foreign, default_timing()),
            Err(FoldAuthorError::UnknownStageMember {
                member_entity: foreign,
            })
        );
        assert_eq!(
            fold_stage.inherit_member_timing(foreign),
            Err(FoldAuthorError::UnknownStageMember {
                member_entity: foreign,
            })
        );
    }
}
