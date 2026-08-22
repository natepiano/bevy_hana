use core::fmt::Debug;
use core::hash::Hash;

use bevy_ecs::entity::Entity;

use super::ArrangementError;

/// Read-only association from a provider's logical members to reserved ECS entities.
///
/// `M` is the provider's logical authoring-time member type, not
/// [`crate::Member`]. Each entry associates one such value with the ECS entity
/// reserved for that member. The association preserves the provider's member
/// order, exposes no mutation, and is the only member/entity association that
/// [`super::ArrangementPlan::try_new`] accepts. The library constructs this
/// value before calling a provider; downstream providers can only inspect it.
///
/// ```compile_fail
/// use hana_valence::ArrangementMemberEntities;
///
/// let _ = ArrangementMemberEntities::<u8>::try_new([]);
/// ```
pub struct ArrangementMemberEntities<M> {
    entries: Vec<(M, Entity)>,
}

impl<M> ArrangementMemberEntities<M>
where
    M: Eq + Hash + Debug,
{
    /// Creates the authoritative logical-member-to-ECS-entity association.
    ///
    /// Entries retain their supplied order. Each logical member and each ECS
    /// entity must occur exactly once, making lookup and plan source/target
    /// membership unambiguous. Library construction reserves the entities
    /// before calling this constructor; this pure type does not access ECS
    /// state.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError::DuplicateLogicalMember`] when a logical
    /// member appears twice, or [`ArrangementError::DuplicateMemberEntity`]
    /// when one ECS entity is associated with more than one logical member.
    pub(super) fn try_new(
        members: impl IntoIterator<Item = (M, Entity)>,
    ) -> Result<Self, ArrangementError> {
        let mut entries = Vec::new();
        for (member, member_entity) in members {
            if entries
                .iter()
                .any(|(listed_member, _)| listed_member == &member)
            {
                return Err(ArrangementError::DuplicateLogicalMember {
                    member: format!("{member:?}"),
                });
            }
            if entries
                .iter()
                .any(|(_, listed_entity)| *listed_entity == member_entity)
            {
                return Err(ArrangementError::DuplicateMemberEntity { member_entity });
            }
            entries.push((member, member_entity));
        }

        Ok(Self { entries })
    }

    /// Returns the ECS entity reserved for `member`.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError::UnlistedMember`] with `member`'s `Debug`
    /// representation when this provider did not enumerate it.
    pub fn entity(&self, member: &M) -> Result<Entity, ArrangementError> {
        self.entries
            .iter()
            .find(|(listed_member, _)| listed_member == member)
            .map_or_else(
                || {
                    Err(ArrangementError::UnlistedMember {
                        member: format!("{member:?}"),
                    })
                },
                |(_, member_entity)| Ok(*member_entity),
            )
    }

    /// Iterates over logical members and their reserved entities in provider order.
    ///
    /// The order is independent of any [`crate::ResolvedAnchorGeometry::frames`]
    /// hash-map iteration and is the ordering used for plan connections.
    pub fn iter(&self) -> impl Iterator<Item = (&M, Entity)> + '_ {
        self.entries
            .iter()
            .map(|(member, member_entity)| (member, *member_entity))
    }

    /// Returns the number of listed logical members.
    #[must_use]
    pub const fn len(&self) -> usize { self.entries.len() }

    /// Returns whether this association contains no logical members.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.entries.is_empty() }

    pub(super) fn index_of_entity(&self, member_entity: Entity) -> Option<usize> {
        self.entries
            .iter()
            .position(|(_, listed_entity)| *listed_entity == member_entity)
    }
}
