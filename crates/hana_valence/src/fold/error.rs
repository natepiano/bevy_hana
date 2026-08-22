use bevy_ecs::entity::Entity;
use thiserror::Error;

/// Invalid explicit input supplied to fold authoring values.
///
/// Every variant is a pure authoring rejection: no stage relationship, fold
/// group, or winding clearance exists when one is returned. Stage variants come
/// from [`super::FoldSequenceBuilder`], group variants from
/// [`super::FoldGroup`] and [`super::FoldGroups`] construction, and clearance
/// variants from [`super::WindingClearance`] construction and lookup.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum FoldAuthorError {
    /// One entity occurs in more than one authored position.
    #[error("authored fold member {0:?} occurs more than once")]
    DuplicateMember(Entity),
    /// An authored fold target was NaN or infinite.
    #[error("a fold target must be finite")]
    NonFiniteFoldTarget,
    /// An authored fold target fell outside the normalized range.
    #[error("a fold target must be within 0..=1")]
    FoldTargetOutOfRange,
    /// A stage target or timing named an entity outside the stage's group.
    #[error("fold stage member {member_entity:?} is not in this stage's group")]
    UnknownStageMember {
        /// Entity supplied to a stage whose group does not contain it.
        member_entity: Entity,
    },
    /// A fold group was constructed without any member.
    #[error("a fold group must contain at least one member")]
    EmptyFoldGroup,
    /// One entity occurs more than once inside a single fold group.
    #[error("fold group member {member_entity:?} occurs more than once in one group")]
    DuplicateFoldGroupMember {
        /// Entity repeated inside the group under construction.
        member_entity: Entity,
    },
    /// A fold-group collection was constructed without any group.
    #[error("a fold-group collection must contain at least one group")]
    EmptyFoldGroups,
    /// No winding-clearance displacement covers a requested member.
    #[error("winding clearance has no displacement for member {member_entity:?}")]
    MissingWindingClearance {
        /// Fold-group member without a winding-clearance displacement.
        member_entity: Entity,
    },
    /// One member received more than one winding-clearance displacement.
    #[error("winding clearance for member {member_entity:?} was supplied more than once")]
    DuplicateWindingClearance {
        /// Entity that received conflicting winding clearances.
        member_entity: Entity,
    },
    /// A winding-clearance displacement had a NaN or infinite coordinate.
    #[error("winding clearance for member {member_entity:?} is not finite")]
    NonFiniteWindingClearance {
        /// Entity whose supplied clearance coordinates were not finite.
        member_entity: Entity,
    },
    /// A winding clearance covered a member outside its fold groups.
    #[error("winding clearance member {member_entity:?} is not in the covered fold groups")]
    ForeignWindingClearance {
        /// Entity supplied to a clearance whose fold groups do not contain it.
        member_entity: Entity,
    },
    /// A recipe endpoint fell outside the representable angular range.
    #[error("fold endpoint for member {member_entity:?} is not representable")]
    AngleOverflow {
        /// Entity whose derived folded endpoint could not be represented.
        member_entity: Entity,
    },
    /// One member was reached by two accordion groups of opposite parity.
    #[error(
        "accordion member {member_entity:?} folds one way in group {first_group} and the other \
         way in group {conflicting_group}"
    )]
    ConflictingAccordionDirections {
        /// Entity that two groups would fold in opposite directions.
        member_entity:     Entity,
        /// Group position that first assigned this member a direction.
        first_group:       usize,
        /// Group position that assigned the opposite direction.
        conflicting_group: usize,
    },
}
