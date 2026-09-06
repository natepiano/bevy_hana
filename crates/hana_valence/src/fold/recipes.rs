//! The three fold recipes the library ships.

use bevy_ecs::entity::Entity;
use hana_kana::Angle;
use hana_kana::AngleError;
use hana_kana::Displacement;

use super::FoldAssignment;
use super::FoldAuthorError;
use super::FoldGroups;
use super::FoldRecipe;
use super::NoCapability;
use super::WindingClearance;
use crate::ArrangementConnection;

/// Half a turn, the default accordion offset whose sign every second group
/// reverses.
///
/// Constructed in a `const` so an unrepresentable value is a compile error
/// rather than a silent zero-radian fold.
const HALF_TURN: Angle = match Angle::from_radians(core::f32::consts::PI) {
    Ok(half_turn) => half_turn,
    Err(_) => panic!("HALF_TURN is finite"),
};

/// Which way one connection folds away from its base angle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FoldDirectionSign {
    Positive,
    Negative,
}

impl FoldDirectionSign {
    /// Reads the direction a signed authored offset expresses.
    const fn of(offset: Angle) -> Self {
        if offset.radians().is_sign_negative() {
            Self::Negative
        } else {
            Self::Positive
        }
    }

    /// Alternates direction with outer group position.
    const fn at_group(group_position: usize, offset: Angle) -> Self {
        let forward = Self::of(offset);
        if group_position.is_multiple_of(2) {
            forward
        } else {
            forward.reversed()
        }
    }

    const fn reversed(self) -> Self {
        match self {
            Self::Positive => Self::Negative,
            Self::Negative => Self::Positive,
        }
    }

    /// Returns the offset re-signed to this direction.
    ///
    /// # Errors
    ///
    /// Returns [`AngleError`] when the re-signed magnitude is not
    /// representable. The caller attributes it to a member entity, which this
    /// method does not have.
    fn applied_to(self, offset: Angle) -> Result<Angle, AngleError> {
        let magnitude = offset.radians().abs();
        let signed = match self {
            Self::Positive => magnitude,
            Self::Negative => -magnitude,
        };
        Angle::from_radians(signed)
    }

    /// Signs a winding-layer count for this direction.
    const fn layer_scale(self, layers: f32) -> f32 {
        match self {
            Self::Positive => layers,
            Self::Negative => -layers,
        }
    }

    /// Selects the connection's physical clearance for this direction.
    const fn clearance_of(self, connection: &ArrangementConnection) -> Displacement {
        match self {
            Self::Positive => connection.hinge_clearance.positive(),
            Self::Negative => connection.hinge_clearance.negative(),
        }
    }
}

/// Adds `offset` to `base` with the overflow detected in wider arithmetic.
///
/// Two valid [`Angle`] values are finite, but their `f32` sum need not be. The
/// range test runs in `f64`, where no sum of two finite `f32` values can
/// overflow, and only a sum already proven representable is re-entered through
/// [`Angle::from_radians`].
fn folded_endpoint(
    member_entity: Entity,
    base: Angle,
    offset: Angle,
) -> Result<Angle, FoldAuthorError> {
    let wide = f64::from(base.radians()) + f64::from(offset.radians());
    if wide < f64::from(f32::MIN) || wide > f64::from(f32::MAX) {
        return Err(FoldAuthorError::AngleOverflow { member_entity });
    }

    Angle::from_radians(base.radians() + offset.radians())
        .map_err(|_| FoldAuthorError::AngleOverflow { member_entity })
}

/// Folds alternate groups in opposite directions, like a paper fan.
///
/// Outer group position sets the direction: even positions fold toward
/// [`Self::fold_offset`], odd positions fold away from it, and every connection
/// inside one group shares that direction. A member reached by two groups of
/// matching direction receives one assignment; matching it against two groups
/// of opposite direction is an authoring error, not a silent last-writer-wins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Accordion {
    /// Signed angular displacement from each connection's base angle.
    pub fold_offset: Angle,
}

impl Default for Accordion {
    /// Folds a positive half-turn, which closes a flat sheet onto itself.
    fn default() -> Self {
        Self {
            fold_offset: HALF_TURN,
        }
    }
}

impl FoldRecipe for Accordion {
    type Error = FoldAuthorError;
    type RequiredCapability = NoCapability;

    fn fold_assignments(
        &self,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
        _: &Self::RequiredCapability,
    ) -> Result<Vec<FoldAssignment>, Self::Error> {
        let mut assignments = Vec::<(usize, FoldDirectionSign, FoldAssignment)>::new();
        for (group_position, group) in groups.iter().enumerate() {
            let direction = FoldDirectionSign::at_group(group_position, self.fold_offset);
            for connection in group.connections(connections) {
                let member_entity = connection.member_entity;
                if let Some((first_group, assigned, _)) = assignments
                    .iter()
                    .find(|(_, _, assignment)| assignment.member_entity == member_entity)
                {
                    if *assigned == direction {
                        continue;
                    }
                    return Err(FoldAuthorError::ConflictingAccordionDirections {
                        member_entity,
                        first_group: *first_group,
                        conflicting_group: group_position,
                    });
                }
                assignments.push((
                    group_position,
                    direction,
                    directed_assignment(connection, direction, self.fold_offset)?,
                ));
            }
        }

        Ok(assignments
            .into_iter()
            .map(|(_, _, assignment)| assignment)
            .collect())
    }
}

/// Folds every selected connection the same way, like winding a spring.
///
/// One signed relative offset applies at each connection regardless of group
/// position, so a chain curls in a single sense. A member reached by more than
/// one group receives one assignment, in first-occurrence order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coil {
    /// Signed angular displacement applied at every selected connection.
    pub fold_offset: Angle,
}

impl FoldRecipe for Coil {
    type Error = FoldAuthorError;
    type RequiredCapability = NoCapability;

    fn fold_assignments(
        &self,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
        _: &Self::RequiredCapability,
    ) -> Result<Vec<FoldAssignment>, Self::Error> {
        let direction = FoldDirectionSign::of(self.fold_offset);
        let mut assignments = Vec::<FoldAssignment>::new();
        for connection in wound_order(groups, connections) {
            if assignments
                .iter()
                .any(|assignment| assignment.member_entity == connection.member_entity)
            {
                continue;
            }
            assignments.push(directed_assignment(
                connection,
                direction,
                self.fold_offset,
            )?);
        }

        Ok(assignments)
    }
}

/// Folds every selected connection the same way and stacks the wound layers.
///
/// A provider's [`WindingClearance`] supplies one source-local layer thickness
/// per member. Where that layer lands is a property of the wrap, so this recipe
/// scales the provider's displacement by the member's position in the winding
/// order: the first connection carries one layer, the second two, and so on. A
/// negative offset winds the other way, which negates the canonical positive
/// clearance instead of taking a second value from the provider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wrap {
    /// Signed angular displacement applied at every selected connection.
    pub fold_offset: Angle,
}

impl FoldRecipe for Wrap {
    type Error = FoldAuthorError;
    type RequiredCapability = WindingClearance;

    fn fold_assignments(
        &self,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
        required_capability: &Self::RequiredCapability,
    ) -> Result<Vec<FoldAssignment>, Self::Error> {
        let direction = FoldDirectionSign::of(self.fold_offset);
        let mut assignments = Vec::<FoldAssignment>::new();
        let mut layers = 0.0_f32;
        for connection in wound_order(groups, connections) {
            let member_entity = connection.member_entity;
            if assignments
                .iter()
                .any(|assignment| assignment.member_entity == member_entity)
            {
                continue;
            }
            layers += 1.0;
            let layer = required_capability.clearance_for(member_entity)?;
            assignments.push(FoldAssignment {
                member_entity,
                folded_angle: folded_endpoint(
                    member_entity,
                    connection.base_angle,
                    signed_offset(member_entity, direction, self.fold_offset)?,
                )?,
                pivot_offset: layer * direction.layer_scale(layers),
            });
        }

        Ok(assignments)
    }
}

/// Yields every selected connection once, in group then member order.
fn wound_order<'connections>(
    groups: &'connections FoldGroups,
    connections: &'connections [ArrangementConnection],
) -> impl Iterator<Item = &'connections ArrangementConnection> {
    groups
        .iter()
        .flat_map(move |group| group.connections(connections))
}

fn signed_offset(
    member_entity: Entity,
    direction: FoldDirectionSign,
    offset: Angle,
) -> Result<Angle, FoldAuthorError> {
    direction
        .applied_to(offset)
        .map_err(|_| FoldAuthorError::AngleOverflow { member_entity })
}

fn directed_assignment(
    connection: &ArrangementConnection,
    direction: FoldDirectionSign,
    offset: Angle,
) -> Result<FoldAssignment, FoldAuthorError> {
    let member_entity = connection.member_entity;
    let offset = signed_offset(member_entity, direction, offset)?;

    Ok(FoldAssignment {
        member_entity,
        folded_angle: folded_endpoint(member_entity, connection.base_angle, offset)?,
        pivot_offset: direction.clearance_of(connection),
    })
}
