use core::ops::Index;
use core::slice::Iter;
use std::vec;
use std::vec::IntoIter;

use bevy_ecs::entity::Entity;

use super::FoldAuthorError;
use crate::ArrangementConnection;

/// One nonempty ordered set of arrangement members folded as a unit.
///
/// A group is internally unique: an entity appears at most once in it. Vector
/// position is the semantic group-local index, so a recipe can give the first
/// member a different treatment from the last one without a second index type.
/// The same entity may appear in several different groups, which is how one
/// arrangement expresses overlapping fold alternatives.
///
/// Construction is the only place membership is set. There is no mutable
/// accessor, no unchecked constructor, and no empty state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldGroup {
    members: Vec<Entity>,
}

impl FoldGroup {
    /// Creates a group from one required member plus any additional members.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::DuplicateFoldGroupMember`] when an entity
    /// occurs more than once.
    pub fn try_new(
        first: Entity,
        remaining: impl IntoIterator<Item = Entity>,
    ) -> Result<Self, FoldAuthorError> {
        Self::try_from_iter(core::iter::once(first).chain(remaining))
    }

    /// Creates a group from a dynamically produced member sequence.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::EmptyFoldGroup`] when the sequence is empty
    /// or [`FoldAuthorError::DuplicateFoldGroupMember`] when an entity occurs
    /// more than once.
    pub fn try_from_iter(
        members: impl IntoIterator<Item = Entity>,
    ) -> Result<Self, FoldAuthorError> {
        let mut group_members = Vec::<Entity>::new();
        for member_entity in members {
            if group_members.contains(&member_entity) {
                return Err(FoldAuthorError::DuplicateFoldGroupMember { member_entity });
            }
            group_members.push(member_entity);
        }
        if group_members.is_empty() {
            return Err(FoldAuthorError::EmptyFoldGroup);
        }

        Ok(Self {
            members: group_members,
        })
    }

    /// Concatenates several groups into one group in their supplied order.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::EmptyFoldGroups`] when no group is supplied
    /// or [`FoldAuthorError::DuplicateFoldGroupMember`] when the supplied
    /// groups overlap, because a single group stays internally unique.
    pub fn combine<'a>(
        groups: impl IntoIterator<Item = &'a Self>,
    ) -> Result<Self, FoldAuthorError> {
        let mut combined = Vec::new();
        let mut group_count = 0_usize;
        for group in groups {
            group_count += 1;
            combined.extend(group.members.iter().copied());
        }
        if group_count == 0 {
            return Err(FoldAuthorError::EmptyFoldGroups);
        }

        Self::try_from_iter(combined)
    }

    /// Iterates over this group's members in group-local index order.
    pub fn iter(&self) -> impl Iterator<Item = &Entity> { self.members.iter() }

    /// Yields this group's connections in member order.
    ///
    /// A group holds only connection sources by construction, so a recipe
    /// never has to join group members to creases by hand and never has to
    /// handle a member without a connection.
    pub fn connections<'a>(
        &self,
        connections: &'a [ArrangementConnection],
    ) -> impl Iterator<Item = &'a ArrangementConnection> {
        self.members.iter().filter_map(|member_entity| {
            connections
                .iter()
                .find(|connection| connection.member_entity == *member_entity)
        })
    }

    /// Returns whether `member_entity` belongs to this group.
    #[must_use]
    pub fn contains(&self, member_entity: Entity) -> bool { self.members.contains(&member_entity) }
}

impl From<Entity> for FoldGroup {
    fn from(member_entity: Entity) -> Self {
        Self {
            members: vec![member_entity],
        }
    }
}

impl TryFrom<Vec<Entity>> for FoldGroup {
    type Error = FoldAuthorError;

    fn try_from(members: Vec<Entity>) -> Result<Self, Self::Error> { Self::try_from_iter(members) }
}

impl<'group> IntoIterator for &'group FoldGroup {
    type IntoIter = Iter<'group, Entity>;
    type Item = &'group Entity;

    fn into_iter(self) -> Self::IntoIter { self.members.iter() }
}

/// Nonempty ordered fold groups describing one complete fold alternative.
///
/// Group order is the outer fold order; member order inside each
/// [`FoldGroup`] is the group-local order. Groups may overlap, and one member
/// may appear in several groups. Like a single group, this collection has no
/// empty state, no unchecked constructor, and no mutable accessor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldGroups {
    groups: Vec<FoldGroup>,
}

impl FoldGroups {
    /// Creates fold groups from one required group plus any additional groups.
    #[must_use]
    pub fn new(first: FoldGroup, remaining: impl IntoIterator<Item = FoldGroup>) -> Self {
        Self {
            groups: core::iter::once(first).chain(remaining).collect(),
        }
    }

    /// Creates fold groups from a dynamically produced group sequence.
    ///
    /// # Errors
    ///
    /// Returns [`FoldAuthorError::EmptyFoldGroups`] when the sequence is empty.
    pub fn try_from_iter(
        groups: impl IntoIterator<Item = FoldGroup>,
    ) -> Result<Self, FoldAuthorError> {
        let groups = groups.into_iter().collect::<Vec<_>>();
        if groups.is_empty() {
            return Err(FoldAuthorError::EmptyFoldGroups);
        }

        Ok(Self { groups })
    }

    /// Returns every group in fold order.
    #[must_use]
    pub fn as_slice(&self) -> &[FoldGroup] { &self.groups }

    /// Iterates over every group in fold order.
    pub fn iter(&self) -> impl Iterator<Item = &FoldGroup> { self.groups.iter() }
}

impl From<FoldGroup> for FoldGroups {
    fn from(group: FoldGroup) -> Self { Self::new(group, []) }
}

impl TryFrom<Vec<FoldGroup>> for FoldGroups {
    type Error = FoldAuthorError;

    fn try_from(groups: Vec<FoldGroup>) -> Result<Self, Self::Error> { Self::try_from_iter(groups) }
}

impl Index<usize> for FoldGroups {
    type Output = FoldGroup;

    /// Returns the group at `index`, panicking on an out-of-range index like
    /// any other slice index.
    fn index(&self, index: usize) -> &Self::Output { &self.groups[index] }
}

impl IntoIterator for FoldGroups {
    type IntoIter = IntoIter<FoldGroup>;
    type Item = FoldGroup;

    fn into_iter(self) -> Self::IntoIter { self.groups.into_iter() }
}

impl<'groups> IntoIterator for &'groups FoldGroups {
    type IntoIter = Iter<'groups, FoldGroup>;
    type Item = &'groups FoldGroup;

    fn into_iter(self) -> Self::IntoIter { self.groups.iter() }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use bevy_ecs::entity::Entity;
    use bevy_ecs::world::World;

    use super::FoldGroup;
    use super::FoldGroups;
    use crate::AnchorSite;
    use crate::AnchoredTo;
    use crate::Angle;
    use crate::ArrangementConnection;
    use crate::Edge;
    use crate::FoldAuthorError;
    use crate::HingeClearance;

    fn entities(count: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..count).map(|_| world.spawn_empty().id()).collect()
    }

    /// Builds a fixture group, naming the rejected members on failure.
    fn valid_group<const N: usize>(root: Entity, rest: [Entity; N]) -> FoldGroup {
        match FoldGroup::try_new(root, rest) {
            Ok(group) => group,
            Err(error) => {
                panic!("test fixture group {root:?} + {rest:?} was rejected: {error:?}")
            },
        }
    }

    #[test]
    fn a_group_keeps_supplied_member_order_as_its_group_local_index() {
        let members = entities(3);
        let group = valid_group(members[2], [members[0], members[1]]);

        assert_eq!(
            group.iter().copied().collect::<Vec<_>>(),
            vec![members[2], members[0], members[1]],
        );
        assert!(group.contains(members[0]));
    }

    #[test]
    fn one_entity_is_the_single_member_case() {
        let members = entities(1);
        let group = FoldGroup::from(members[0]);

        assert_eq!(group.iter().copied().collect::<Vec<_>>(), vec![members[0]]);
    }

    #[test]
    fn empty_and_repeated_group_construction_is_rejected() {
        let members = entities(2);

        assert_eq!(
            FoldGroup::try_from_iter([]),
            Err(FoldAuthorError::EmptyFoldGroup),
        );
        assert_eq!(
            FoldGroup::try_from(Vec::new()),
            Err(FoldAuthorError::EmptyFoldGroup),
        );
        assert_eq!(
            FoldGroup::try_new(members[0], [members[1], members[0]]),
            Err(FoldAuthorError::DuplicateFoldGroupMember {
                member_entity: members[0],
            }),
        );
        assert_eq!(
            FoldGroup::try_from(vec![members[1], members[1]]),
            Err(FoldAuthorError::DuplicateFoldGroupMember {
                member_entity: members[1],
            }),
        );
    }

    #[test]
    fn combining_disjoint_groups_concatenates_them_and_overlap_is_rejected() {
        let members = entities(3);
        let first = valid_group(members[0], [members[1]]);
        let second = FoldGroup::from(members[2]);

        let combined = match FoldGroup::combine([&first, &second]) {
            Ok(combined) => combined,
            Err(error) => {
                panic!("test fixture combine of {first:?} and {second:?} was rejected: {error:?}")
            },
        };
        assert_eq!(
            combined.iter().copied().collect::<Vec<_>>(),
            vec![members[0], members[1], members[2]],
        );
        assert_eq!(
            FoldGroup::combine([&first, &first]),
            Err(FoldAuthorError::DuplicateFoldGroupMember {
                member_entity: members[0],
            }),
        );
        assert_eq!(
            FoldGroup::combine([]),
            Err(FoldAuthorError::EmptyFoldGroups),
        );
    }

    #[test]
    fn groups_may_overlap_and_expose_ordered_slice_index_and_iteration() {
        let members = entities(3);
        let first = valid_group(members[0], [members[1]]);
        let second = valid_group(members[1], [members[2]]);
        let groups = FoldGroups::new(first.clone(), [second.clone()]);

        assert_eq!(groups.as_slice(), [first.clone(), second.clone()]);
        assert_eq!(groups[1], second);
        assert_eq!(groups.iter().count(), 2);
        assert_eq!((&groups).into_iter().count(), 2);
        assert_eq!(groups.into_iter().collect::<Vec<_>>(), vec![first, second]);
    }

    #[test]
    fn group_connections_follow_member_order_and_skip_unconnected_members() {
        let members = entities(3);
        let group = valid_group(members[2], [members[1], members[0]]);
        // Plan order deliberately disagrees with member order, and members[1]
        // — a group member — has no connection in the plan at all.
        let connections = [test_connection(members[0]), test_connection(members[2])];

        assert_eq!(
            group
                .connections(&connections)
                .map(|connection| connection.member_entity)
                .collect::<Vec<_>>(),
            vec![members[2], members[0]],
        );
    }

    fn test_connection(member_entity: Entity) -> ArrangementConnection {
        ArrangementConnection {
            member_entity,
            anchored_to: AnchoredTo::new(
                Entity::PLACEHOLDER,
                AnchorSite::Center,
                AnchorSite::Center,
            ),
            member_edge: Edge {
                start: AnchorSite::Vertex(0),
                end:   AnchorSite::Vertex(1),
            },
            base_angle: Angle::default(),
            hinge_clearance: HingeClearance::CENTERED,
        }
    }

    #[test]
    fn dynamic_group_collection_conversion_rejects_an_empty_sequence() {
        assert_eq!(
            FoldGroups::try_from_iter([]),
            Err(FoldAuthorError::EmptyFoldGroups),
        );
        assert_eq!(
            FoldGroups::try_from(Vec::new()),
            Err(FoldAuthorError::EmptyFoldGroups),
        );
    }
}
