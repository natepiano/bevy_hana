use bevy_ecs::entity::Entity;
use hana_kana::Displacement;

use super::FoldAuthorError;
use super::FoldGroups;
use super::ProviderCapability;

/// Physical clearance that keeps each wrapped layer off the layer beneath it.
///
/// A winding clearance is a provider capability: member thickness is the
/// provider's own data, and a wrapping recipe needs that thickness to place
/// every layer. The mapping from member to displacement stays private so a
/// recipe cannot receive partial coverage, an unrelated member, or a mixed
/// winding sense.
///
/// Each stored [`Displacement`] is the canonical positive winding direction for
/// its member, measured in the same source-local frame as
/// [`crate::HingeClearance`]. The opposite winding direction is exactly its
/// negation, which is why one member has one value: a wrap cannot be authored
/// with asymmetric winding.
#[derive(Clone, Debug, PartialEq)]
pub struct WindingClearance {
    clearances: Vec<(Entity, Displacement)>,
}

impl WindingClearance {
    /// Creates a clearance that exactly covers the members of `groups`.
    ///
    /// Coverage is validated here, before the value can be associated with a
    /// plan selection, so a wrapping recipe never has to check for a missing
    /// member while it writes hinges. A member repeated across groups needs one
    /// displacement, not one per group.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::NonFiniteWindingClearance`] for NaN or
    /// infinite coordinates, [`FoldAuthorError::DuplicateWindingClearance`]
    /// when a member is supplied twice,
    /// [`FoldAuthorError::ForeignWindingClearance`] when a supplied member is
    /// outside `groups`, and [`FoldAuthorError::MissingWindingClearance`] when
    /// a member of `groups` has no displacement.
    pub fn try_new(
        groups: &FoldGroups,
        clearances: impl IntoIterator<Item = (Entity, Displacement)>,
    ) -> Result<Self, FoldAuthorError> {
        let mut entries = Vec::<(Entity, Displacement)>::new();
        for (member_entity, clearance) in clearances {
            if !clearance.is_finite() {
                return Err(FoldAuthorError::NonFiniteWindingClearance { member_entity });
            }
            if entries
                .iter()
                .any(|(listed_entity, _)| *listed_entity == member_entity)
            {
                return Err(FoldAuthorError::DuplicateWindingClearance { member_entity });
            }
            if !groups.iter().any(|group| group.contains(member_entity)) {
                return Err(FoldAuthorError::ForeignWindingClearance { member_entity });
            }
            entries.push((member_entity, clearance));
        }

        for group in groups {
            for member_entity in group {
                if !entries
                    .iter()
                    .any(|(listed_entity, _)| listed_entity == member_entity)
                {
                    return Err(FoldAuthorError::MissingWindingClearance {
                        member_entity: *member_entity,
                    });
                }
            }
        }

        Ok(Self {
            clearances: entries,
        })
    }

    /// Returns the canonical positive winding displacement for one member.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::MissingWindingClearance`] when this clearance
    /// does not cover `member_entity`.
    pub fn clearance_for(&self, member_entity: Entity) -> Result<Displacement, FoldAuthorError> {
        self.clearances
            .iter()
            .find(|(listed_entity, _)| *listed_entity == member_entity)
            .map_or(
                Err(FoldAuthorError::MissingWindingClearance { member_entity }),
                |(_, clearance)| Ok(*clearance),
            )
    }
}

impl ProviderCapability for WindingClearance {}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy_ecs::entity::Entity;
    use bevy_ecs::world::World;
    use hana_kana::Displacement;

    use super::WindingClearance;
    use crate::FoldAuthorError;
    use crate::FoldGroup;
    use crate::FoldGroups;

    fn entities(count: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..count).map(|_| world.spawn_empty().id()).collect()
    }

    fn overlapping_groups(members: &[Entity]) -> FoldGroups {
        let Ok(first) = FoldGroup::try_new(members[0], [members[1]]) else {
            panic!("FoldGroup::try_new rejected the first overlapping group");
        };
        let Ok(second) = FoldGroup::try_new(members[1], [members[2]]) else {
            panic!("FoldGroup::try_new rejected the second overlapping group");
        };
        FoldGroups::new(first, [second])
    }

    #[test]
    fn one_displacement_per_member_covers_repeated_group_membership() {
        let members = entities(3);
        let groups = overlapping_groups(&members);
        let step = Displacement::new(0.0, 0.25, 0.0);

        let Ok(clearance) = WindingClearance::try_new(
            &groups,
            [
                (members[0], step),
                (members[1], step),
                (members[2], step * 2.0),
            ],
        ) else {
            panic!("WindingClearance::try_new rejected one displacement per member");
        };

        assert_eq!(clearance.clearance_for(members[1]), Ok(step));
        assert_eq!(clearance.clearance_for(members[2]), Ok(step * 2.0));
    }

    #[test]
    fn incomplete_coverage_is_rejected_before_the_value_exists() {
        let members = entities(3);
        let groups = overlapping_groups(&members);
        let step = Displacement::new(0.0, 0.25, 0.0);

        assert_eq!(
            WindingClearance::try_new(&groups, [(members[0], step), (members[1], step)]),
            Err(FoldAuthorError::MissingWindingClearance {
                member_entity: members[2],
            }),
        );
    }

    #[test]
    fn duplicate_foreign_and_non_finite_clearances_are_rejected() {
        let members = entities(4);
        let groups = overlapping_groups(&members);
        let step = Displacement::new(0.0, 0.25, 0.0);

        assert_eq!(
            WindingClearance::try_new(&groups, [(members[0], step), (members[0], step)]),
            Err(FoldAuthorError::DuplicateWindingClearance {
                member_entity: members[0],
            }),
        );
        assert_eq!(
            WindingClearance::try_new(&groups, [(members[3], step)]),
            Err(FoldAuthorError::ForeignWindingClearance {
                member_entity: members[3],
            }),
        );
        assert_eq!(
            WindingClearance::try_new(
                &groups,
                [(members[0], Displacement::new(f32::NAN, 0.0, 0.0))],
            ),
            Err(FoldAuthorError::NonFiniteWindingClearance {
                member_entity: members[0],
            }),
        );
    }

    #[test]
    fn a_lookup_outside_the_covered_members_names_the_member() {
        let members = entities(4);
        let groups = overlapping_groups(&members);
        let step = Displacement::new(0.0, 0.25, 0.0);
        let Ok(clearance) = WindingClearance::try_new(
            &groups,
            [(members[0], step), (members[1], step), (members[2], step)],
        ) else {
            panic!("WindingClearance::try_new rejected a uniform displacement set");
        };

        assert_eq!(
            clearance.clearance_for(members[3]),
            Err(FoldAuthorError::MissingWindingClearance {
                member_entity: members[3],
            }),
        );
    }
}
