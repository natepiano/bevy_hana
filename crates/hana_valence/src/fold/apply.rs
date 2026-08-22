//! Deferred application of one fold recipe onto retained arrangement hinges.

use core::fmt::Debug;
use core::hash::Hash;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_platform::collections::HashMap;

use super::FoldAssignment;
use super::FoldGroup;
use super::FoldGroups;
use super::FoldRecipe;
use super::FoldRecipeCapability;
use crate::ArrangementConnection;
use crate::ArrangementError;
use crate::Hinge;
use crate::RetainedProviderKnowledge;
use crate::arrangement;

/// Replaces every selected fold endpoint from one transient recipe.
///
/// Reads run first and completely: retained groups, retained connections, and
/// the recipe's declared capability. The selection is then split into connected
/// pieces so an ordering-sensitive recipe never counts across a boundary with
/// no physical meaning, and the recipe runs once per piece. The concatenated
/// result is validated as a whole set before any hinge is written, so a
/// coverage, duplicate, foreign, non-finite, or away-from-base failure leaves
/// every current hinge in place.
///
/// # Errors
///
/// Returns the exact selection, capability, recipe, assignment, or base-endpoint
/// failure. Every one of them happens before the first hinge write.
pub(crate) fn apply_fold_recipe<S, R>(
    world: &mut World,
    arrangement_entity: Entity,
    selection: &S,
    recipe: &R,
) -> Result<(), ArrangementError>
where
    S: Eq + Hash + Debug + Send + Sync + 'static,
    R: FoldRecipe,
{
    let assignments = {
        let knowledge = RetainedProviderKnowledge::for_arrangement(world, arrangement_entity)?;
        let groups = knowledge.groups(selection)?;
        let capability = R::RequiredCapability::retrieve(&knowledge, selection)?;
        let connections = arrangement::retained_connections(world, arrangement_entity)?;

        let mut assignments = Vec::<FoldAssignment>::new();
        for piece in connected_pieces(groups, connections) {
            assignments.extend(
                recipe
                    .fold_assignments(&piece, connections, capability)
                    .map_err(|source| ArrangementError::FoldRecipe {
                        arrangement: arrangement_entity,
                        source:      Box::new(source),
                    })?,
            );
        }
        validate_assignments(&assignments, groups)?;
        assignments
    };

    reject_hinges_away_from_base(world, &assignments)?;
    for assignment in assignments {
        // Application-owned entities may already be gone; a later missing
        // member is best effort once writing has begun.
        if let Some(mut member) = world.get_entity_mut(assignment.member_entity).ok()
            && let Some(hinge) = member.get::<Hinge>().copied()
        {
            member.insert(hinge.refolded(assignment.folded_angle, assignment.pivot_offset));
        }
    }

    Ok(())
}

/// Splits the selected groups into physically connected pieces.
///
/// Connectivity is followed over every retained connection, not only over group
/// members, because a forest root is never in a group yet is exactly what joins
/// two members. Each returned [`FoldGroups`] keeps the selection's group order
/// and drops the groups that piece does not reach, so a recipe's first group is
/// the first group of the piece it is folding.
fn connected_pieces(groups: &FoldGroups, connections: &[ArrangementConnection]) -> Vec<FoldGroups> {
    let mut piece_of = HashMap::<Entity, usize>::default();
    let mut next_piece = 0_usize;
    for connection in connections {
        let target = connection.anchored_to.target();
        let joined = match (
            piece_of.get(&connection.member_entity).copied(),
            piece_of.get(&target).copied(),
        ) {
            (None, None) => {
                next_piece += 1;
                next_piece - 1
            },
            (Some(piece), None) | (None, Some(piece)) => piece,
            (Some(source_piece), Some(target_piece)) => {
                for piece in piece_of.values_mut() {
                    if *piece == target_piece {
                        *piece = source_piece;
                    }
                }
                source_piece
            },
        };
        piece_of.insert(connection.member_entity, joined);
        piece_of.insert(target, joined);
    }

    let mut ordered_pieces = Vec::<usize>::new();
    for group in groups {
        for member_entity in group {
            // A group holds only connection sources by construction, so a
            // member absent from the connectivity map has no connection and
            // therefore no piece. Excluding it here leaves it uncovered, which
            // `validate_assignments` reports as a missing assignment rather
            // than folding it with every other unconnected member.
            let Some(piece) = piece_of.get(member_entity).copied() else {
                continue;
            };
            if !ordered_pieces.contains(&piece) {
                ordered_pieces.push(piece);
            }
        }
    }

    ordered_pieces
        .into_iter()
        .filter_map(|piece| {
            let reached = groups.iter().filter_map(|group| {
                FoldGroup::try_from_iter(
                    group
                        .iter()
                        .filter(|member_entity| piece_of.get(*member_entity) == Some(&piece))
                        .copied(),
                )
                .ok()
            });
            FoldGroups::try_from_iter(reached).ok()
        })
        .collect()
}

/// Rejects an incomplete, repeated, foreign, or non-finite assignment set.
fn validate_assignments(
    assignments: &[FoldAssignment],
    groups: &FoldGroups,
) -> Result<(), ArrangementError> {
    for (position, assignment) in assignments.iter().enumerate() {
        if !assignment.pivot_offset.is_finite() {
            return Err(ArrangementError::NonFiniteFoldAssignmentPivot {
                member_entity: assignment.member_entity,
            });
        }
        if assignments[..position]
            .iter()
            .any(|earlier| earlier.member_entity == assignment.member_entity)
        {
            return Err(ArrangementError::DuplicateFoldAssignment {
                member_entity: assignment.member_entity,
            });
        }
        if !groups
            .iter()
            .any(|group| group.contains(assignment.member_entity))
        {
            return Err(ArrangementError::ForeignFoldAssignment {
                member_entity: assignment.member_entity,
            });
        }
    }

    for group in groups {
        for member_entity in group {
            if !assignments
                .iter()
                .any(|assignment| assignment.member_entity == *member_entity)
            {
                return Err(ArrangementError::MissingFoldAssignment {
                    member_entity: *member_entity,
                });
            }
        }
    }

    Ok(())
}

/// Rejects replacement anywhere but the shared base endpoint.
///
/// A hinge already carrying a distinct folded endpoint is mid-recipe: swapping
/// its target would move geometry that playback is holding. A member whose
/// hinge is absent is left to the best-effort write pass.
fn reject_hinges_away_from_base(
    world: &World,
    assignments: &[FoldAssignment],
) -> Result<(), ArrangementError> {
    for assignment in assignments {
        if world
            .get::<Hinge>(assignment.member_entity)
            .is_some_and(|hinge| !hinge.rests_at_base())
        {
            return Err(ArrangementError::FoldRecipeAwayFromBase {
                member_entity: assignment.member_entity,
            });
        }
    }

    Ok(())
}
