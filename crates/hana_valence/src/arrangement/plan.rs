use core::error::Error as StdError;
use core::fmt::Debug;
use core::hash::Hash;

use bevy_ecs::entity::Entity;
use bevy_platform::collections::HashMap;
use bevy_platform::collections::HashSet;
use hana_kana::Angle;
use hana_kana::Displacement;
use thiserror::Error;

use super::ArrangementMemberEntities;
use super::capabilities::CapabilityAssociation;
use super::capabilities::FoldGroupAlternative;
use crate::AnchoredTo;
use crate::Edge;
use crate::FoldGroups;
use crate::FoldSequence;
use crate::ProviderCapability;

/// One provider-authored physical attachment and its immutable hinge endpoint.
///
/// `member_entity` is the relationship source. [`AnchoredTo`] supplies its
/// target and the source/target attachment sites. `member_edge` is the ordered,
/// source-local hinge axis: reversing its endpoints reverses the fold sign.
/// `base_angle` is a finite, immutable resting endpoint that may span multiple
/// turns. [`ArrangementPlan::try_new`] accepts this connection only when both
/// entities belong to its authoritative [`ArrangementMemberEntities`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrangementConnection {
    /// ECS entity that will become the physical relationship source.
    pub member_entity:   Entity,
    /// Physical target and attachment sites for `member_entity`.
    pub anchored_to:     AnchoredTo,
    /// Ordered source-local edge used as this connection's hinge axis.
    pub member_edge:     Edge,
    /// Immutable finite provider resting angle for the connection hinge.
    pub base_angle:      Angle,
    /// Source-local physical pivot clearances for positive and negative folding.
    pub hinge_clearance: HingeClearance,
}

/// Source-local physical pivot displacements for each hinge direction.
///
/// `positive` and `negative` are signed semantic [`Displacement`] values from
/// the shared `ArrangementConnection::member_edge` to the physical pivot. The
/// positive value applies to a positive relative fold angle; the negative
/// value applies to a negative relative fold angle. They are independent so a
/// provider can describe asymmetric physical clearance. [`Self::CENTERED`]
/// keeps both directional pivots on the shared edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HingeClearance {
    positive: Displacement,
    negative: Displacement,
}

impl HingeClearance {
    /// Zero clearance for both hinge directions.
    pub const CENTERED: Self = Self::new(
        Displacement::new(0.0, 0.0, 0.0),
        Displacement::new(0.0, 0.0, 0.0),
    );

    /// Creates directional physical-pivot clearances.
    ///
    /// The values are checked for finite coordinates by
    /// [`ArrangementPlan::try_new`], which keeps construction of a connection
    /// value itself infallible and pure.
    #[must_use]
    pub const fn new(positive: Displacement, negative: Displacement) -> Self {
        Self { positive, negative }
    }

    /// Returns the source-local pivot displacement for positive fold direction.
    #[must_use]
    pub const fn positive(&self) -> Displacement { self.positive }

    /// Returns the source-local pivot displacement for negative fold direction.
    #[must_use]
    pub const fn negative(&self) -> Displacement { self.negative }
}

/// Identifies which connection displacement had a non-finite coordinate.
///
/// [`ArrangementError::NonFiniteDisplacement`] returns this value so callers
/// can distinguish the attachment offset from either directional
/// [`HingeClearance`] value without interpreting a boolean flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArrangementConnectionDisplacement {
    /// The target-frame offset stored by [`AnchoredTo`].
    AttachmentOffset,
    /// The source-local pivot displacement for positive fold direction.
    PositiveHingeClearance,
    /// The source-local pivot displacement for negative fold direction.
    NegativeHingeClearance,
}

/// A complete, structurally valid provider connection forest.
///
/// `S` is the provider's typed fold-group selection vocabulary. One `S` value
/// selects one complete fold-group alternative plus the provider capabilities
/// that alternative needs, and the type stays part of the plan until later ECS
/// materialization erases it; the plan is never a component and contains no
/// world state. Connections are stored in [`ArrangementMemberEntities`]
/// provider order, not their input iterator order or a geometry frame-map
/// iteration order.
#[derive(Debug)]
pub struct ArrangementPlan<S> {
    connections:   Vec<ArrangementConnection>,
    alternatives:  Vec<FoldGroupAlternative<S>>,
    fold_sequence: PlannedFoldSequence,
}

/// Whether a plan carries a provider-authored fold sequence to materialize.
///
/// A provider authors at most one sequence per plan, and most providers author
/// none: an arrangement is a connection forest first, and folding it is a
/// separate decision. `Unauthored` is that ordinary case: materialization
/// writes no fold sequence and the arrangement materializes exactly as it would
/// without this field.
/// `Authored` carries the one resolved sequence
/// [`ArrangementProvider::with_fold_sequence`](super::ArrangementProvider::with_fold_sequence)
/// or
/// [`ArrangementProvider::with_custom_fold_sequence`](super::ArrangementProvider::with_custom_fold_sequence)
/// built while the plan was still pure.
#[derive(Debug)]
pub enum PlannedFoldSequence {
    /// The provider authored no fold sequence for this arrangement.
    Unauthored,
    /// The provider authored this sequence for the arrangement controller.
    Authored(FoldSequence),
}

impl<S> ArrangementPlan<S> {
    /// Validates and stores all provider-authored connections as one forest.
    ///
    /// Every connection source and target must be in `members`; each source
    /// may have one target; and following source-to-target relationships must
    /// terminate. Empty associations, multiple roots, disconnected forests,
    /// and repeated targets are valid. The resulting connections are reordered
    /// by `members`' logical-member order.
    ///
    /// This method performs no ECS access, scene work, relationship insertion,
    /// retained diagnostics, or rollback. An error returns before a plan is
    /// produced and leaves all supplied values unchanged.
    ///
    /// `Edge` validation stops at unequal endpoints: this method
    /// has no [`crate::ResolvedAnchorGeometry`], so endpoint presence,
    /// separation, and usable-axis checks remain owned by
    /// [`crate::ResolvedAnchorGeometry::try_new`]. `Angle` is already finite
    /// by construction and therefore needs no additional validation here.
    ///
    /// # Errors
    ///
    /// Returns an [`ArrangementError`] for every structural failure described
    /// above, a same-site `member_edge`, or a non-finite attachment offset or
    /// hinge-clearance displacement.
    pub fn try_new<M>(
        members: &ArrangementMemberEntities<M>,
        connections: impl IntoIterator<Item = ArrangementConnection>,
    ) -> Result<Self, ArrangementError>
    where
        M: Eq + Hash + Debug,
    {
        let mut sources = HashSet::<Entity>::default();
        let mut ordered_connections = Vec::new();

        for connection in connections {
            let Some(member_index) = members.index_of_entity(connection.member_entity) else {
                return Err(ArrangementError::UnlistedConnectionSource {
                    member_entity: connection.member_entity,
                });
            };
            let target_entity = connection.anchored_to.target();
            if members.index_of_entity(target_entity).is_none() {
                return Err(ArrangementError::UnlistedConnectionTarget { target_entity });
            }
            if connection.member_entity == target_entity {
                return Err(ArrangementError::SelfTarget {
                    member_entity: connection.member_entity,
                });
            }
            if !sources.insert(connection.member_entity) {
                return Err(ArrangementError::DuplicateConnectionSource {
                    member_entity: connection.member_entity,
                });
            }
            if connection.member_edge.start == connection.member_edge.end {
                return Err(ArrangementError::EqualMemberEdgeSites {
                    member_entity: connection.member_entity,
                    member_edge:   connection.member_edge,
                });
            }
            validate_displacement(
                connection.member_entity,
                connection.anchored_to.offset(),
                ArrangementConnectionDisplacement::AttachmentOffset,
            )?;
            validate_displacement(
                connection.member_entity,
                connection.hinge_clearance.positive(),
                ArrangementConnectionDisplacement::PositiveHingeClearance,
            )?;
            validate_displacement(
                connection.member_entity,
                connection.hinge_clearance.negative(),
                ArrangementConnectionDisplacement::NegativeHingeClearance,
            )?;

            ordered_connections.push((member_index, connection));
        }

        ordered_connections.sort_by_key(|(member_index, _)| *member_index);
        let connections = ordered_connections
            .into_iter()
            .map(|(_, connection)| connection)
            .collect::<Vec<_>>();
        validate_forest(&connections)?;

        Ok(Self {
            connections,
            alternatives: Vec::new(),
            fold_sequence: PlannedFoldSequence::Unauthored,
        })
    }

    /// Returns validated connections in authoritative logical-member order.
    ///
    /// Roots have no `ArrangementConnection`; therefore this slice contains
    /// only members that attach to another listed member.
    #[must_use]
    pub fn connections(&self) -> &[ArrangementConnection] { &self.connections }

    /// Returns whether a provider authored a fold sequence for this plan.
    #[must_use]
    pub const fn fold_sequence(&self) -> &PlannedFoldSequence { &self.fold_sequence }

    /// Retains `fold_sequence` as the one sequence this plan materializes.
    ///
    /// The sequence adapters call this once, after the wrapped provider
    /// returned a valid plan and the authored sequence was resolved from that
    /// plan's own retained groups. Every member the sequence tracks must be a
    /// connection source of this plan, for the same reason a fold group's
    /// members must be: a physical root has no hinge to fold.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError::DuplicateAuthoredFoldSequence`] when this
    /// plan already carries an authored sequence, or
    /// [`ArrangementError::ForeignFoldGroupMember`] when a tracked member is
    /// not a connection source of this plan.
    pub(super) fn with_authored_fold_sequence(
        mut self,
        fold_sequence: FoldSequence,
    ) -> Result<Self, ArrangementError> {
        if matches!(self.fold_sequence, PlannedFoldSequence::Authored(_)) {
            return Err(ArrangementError::DuplicateAuthoredFoldSequence);
        }
        for track in fold_sequence.tracks() {
            let member_entity = track.member_entity();
            if !self
                .connections
                .iter()
                .any(|connection| connection.member_entity == member_entity)
            {
                return Err(ArrangementError::ForeignFoldGroupMember { member_entity });
            }
        }
        self.fold_sequence = PlannedFoldSequence::Authored(fold_sequence);

        Ok(self)
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        Vec<ArrangementConnection>,
        Vec<FoldGroupAlternative<S>>,
        PlannedFoldSequence,
    ) {
        (self.connections, self.alternatives, self.fold_sequence)
    }
}

impl<S> ArrangementPlan<S>
where
    S: Eq + Hash + Debug + Send + Sync + 'static,
{
    /// Retains one complete fold-group alternative under `selection`.
    ///
    /// Each group member must already be the source of one of this plan's
    /// connections, because a physical root has no hinge to fold. Groups may
    /// overlap and a member may appear in several groups; that is how a
    /// provider offers alternative fold layouts for the same arrangement.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError::DuplicateFoldGroupSelection`] when
    /// `selection` already selects an alternative, or
    /// [`ArrangementError::ForeignFoldGroupMember`] when a group member is not
    /// a connection source of this plan.
    pub fn with_fold_groups(
        mut self,
        selection: S,
        groups: FoldGroups,
    ) -> Result<Self, ArrangementError> {
        if self
            .alternatives
            .iter()
            .any(|alternative| alternative.selection() == &selection)
        {
            return Err(ArrangementError::DuplicateFoldGroupSelection {
                selection: format!("{selection:?}"),
            });
        }
        for group in &groups {
            for member_entity in group {
                if !self
                    .connections
                    .iter()
                    .any(|connection| connection.member_entity == *member_entity)
                {
                    return Err(ArrangementError::ForeignFoldGroupMember {
                        member_entity: *member_entity,
                    });
                }
            }
        }
        self.alternatives
            .push(FoldGroupAlternative::new(selection, groups));

        Ok(self)
    }

    /// Returns the groups retained for `selection`.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError::UnknownFoldGroupSelection`] when the
    /// provider retained no alternative for this selection value.
    pub(super) fn fold_groups(&self, selection: &S) -> Result<&FoldGroups, ArrangementError> {
        self.alternatives
            .iter()
            .find(|alternative| alternative.selection() == selection)
            .map(FoldGroupAlternative::groups)
            .ok_or_else(|| ArrangementError::UnknownFoldGroupSelection {
                selection: format!("{selection:?}"),
            })
    }

    /// Retains purpose-specific provider knowledge for one selected alternative.
    ///
    /// `C` is the provider's own capability type, such as
    /// [`crate::WindingClearance`]. A later recipe looks its value up by
    /// concrete type, so one selection holds at most one value per capability
    /// type. This association never inspects the invariants inside `C`: a
    /// capability that has invariants enforces them in its own constructor
    /// before reaching this method.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError::UnknownFoldGroupSelection`] when
    /// `selection` has no fold-group alternative yet, or
    /// [`ArrangementError::DuplicateCapability`] when the selection
    /// already holds a value of type `C`.
    pub fn with_capability<C>(
        mut self,
        selection: S,
        capability: C,
    ) -> Result<Self, ArrangementError>
    where
        C: ProviderCapability,
    {
        let Some(alternative) = self
            .alternatives
            .iter_mut()
            .find(|alternative| alternative.selection() == &selection)
        else {
            return Err(ArrangementError::UnknownFoldGroupSelection {
                selection: format!("{selection:?}"),
            });
        };

        match alternative.associate(capability) {
            CapabilityAssociation::Stored => Ok(self),
            CapabilityAssociation::AlreadyAssociated { capability } => {
                Err(ArrangementError::DuplicateCapability {
                    selection: format!("{selection:?}"),
                    capability,
                })
            },
        }
    }
}

/// Errors shared by arrangement validation and construction commands.
///
/// Association and plan-validation variants reject authoring input before any
/// relationship or member scene is queued. Spawn commands may already have
/// reserved entities; synchronous failure queues their cleanup.
/// [`Self::MissingMemberBinding`] reports a command-side binding failure without
/// inserting relationships. [`Self::Provider`] keeps a downstream provider
/// error as its source.
#[derive(Debug, Error)]
pub enum ArrangementError {
    /// A provider-specific authoring operation failed.
    #[error("arrangement provider failed: {source}")]
    Provider {
        /// Original provider error, retained for error-chain inspection.
        #[source]
        source: Box<dyn StdError + Send + Sync + 'static>,
    },
    /// A provider enumerated the same logical member more than once.
    #[error("logical arrangement member {member} appears more than once")]
    DuplicateLogicalMember {
        /// `Debug` representation of the repeated logical member.
        member: String,
    },
    /// Two logical members were associated with the same ECS entity.
    #[error("arrangement member entity {member_entity:?} appears more than once")]
    DuplicateMemberEntity {
        /// Entity that cannot represent more than one logical member.
        member_entity: Entity,
    },
    /// A caller reported [`MemberBinding::Missing`](super::MemberBinding::Missing)
    /// for a listed member.
    #[error("logical arrangement member {member} has no bound entity")]
    MissingMemberBinding {
        /// `Debug` representation of the member for which binding failed.
        member: String,
    },
    /// A provider requested the entity for a logical member it did not list.
    #[error("logical arrangement member {member} was not listed")]
    UnlistedMember {
        /// `Debug` representation of the requested logical member.
        member: String,
    },
    /// A connection source was not in the authoritative member association.
    #[error("arrangement connection source {member_entity:?} was not listed")]
    UnlistedConnectionSource {
        /// Foreign relationship source entity.
        member_entity: Entity,
    },
    /// A connection target was not in the authoritative member association.
    #[error("arrangement connection target {target_entity:?} was not listed")]
    UnlistedConnectionTarget {
        /// Foreign relationship target entity.
        target_entity: Entity,
    },
    /// A connection named one member as both its source and its target.
    #[error("arrangement connection {member_entity:?} cannot target itself")]
    SelfTarget {
        /// Source entity that also appeared as its target.
        member_entity: Entity,
    },
    /// More than one connection used the same relationship source.
    #[error("arrangement connection source {member_entity:?} appears more than once")]
    DuplicateConnectionSource {
        /// Source entity shared by the conflicting connections.
        member_entity: Entity,
    },
    /// A connection edge used the same source-local anchor site twice.
    #[error("arrangement connection {member_entity:?} has equal member-edge sites {member_edge:?}")]
    EqualMemberEdgeSites {
        /// Source entity whose edge was invalid without geometry inspection.
        member_entity: Entity,
        /// Ordered edge whose endpoints were equal.
        member_edge:   Edge,
    },
    /// An attachment offset or hinge-clearance displacement had NaN or infinity.
    #[error("arrangement connection {member_entity:?} has a non-finite {displacement:?}")]
    NonFiniteDisplacement {
        /// Connection source containing the invalid displacement.
        member_entity: Entity,
        /// Connection field whose coordinates were non-finite.
        displacement:  ArrangementConnectionDisplacement,
    },
    /// Following source-to-target connections returned to an earlier source.
    #[error("arrangement connection forest contains a physical cycle through {member_entity:?}")]
    PhysicalCycle {
        /// First repeated source entity encountered while following the cycle.
        member_entity: Entity,
    },
    /// One fold-group selection was given more than one group alternative.
    #[error("fold-group selection {selection} already selects an alternative")]
    DuplicateFoldGroupSelection {
        /// `Debug` representation of the repeated selection value.
        selection: String,
    },
    /// A selection had no fold-group alternative when one was required.
    #[error("fold-group selection {selection} has no group alternative")]
    UnknownFoldGroupSelection {
        /// `Debug` representation of the requested selection value.
        selection: String,
    },
    /// A typed read used a different selection type than the provider's.
    #[error("arrangement retains {expected} fold-group selections, not {found}")]
    MismatchedFoldGroupSelectionType {
        /// Selection type name recorded when the arrangement materialized.
        expected: &'static str,
        /// Selection type name supplied by the read.
        found:    &'static str,
    },
    /// A fold group listed a member that is not a connection source.
    #[error("fold group member {member_entity:?} is not an arrangement connection source")]
    ForeignFoldGroupMember {
        /// Group member without a connection of its own.
        member_entity: Entity,
    },
    /// One plan was given more than one authored fold sequence.
    #[error("arrangement plan already has an authored fold sequence")]
    DuplicateAuthoredFoldSequence,
    /// One selection was given two values of the same capability type.
    #[error("fold-group selection {selection} already has a {capability} capability")]
    DuplicateCapability {
        /// `Debug` representation of the selection value.
        selection:  String,
        /// Type name of the capability supplied twice.
        capability: &'static str,
    },
    /// A required provider capability was never associated with a selection.
    #[error("fold-group selection {selection} has no {capability} capability")]
    MissingCapability {
        /// `Debug` representation of the selection value.
        selection:  String,
        /// Type name of the capability the caller requires.
        capability: &'static str,
    },
    /// A retained read named an entity that is not a materialized arrangement.
    #[error("entity {arrangement:?} is not a materialized arrangement controller")]
    UnmaterializedArrangement {
        /// Entity that carried no retained arrangement state.
        arrangement: Entity,
    },
    /// A fold recipe rejected the selected groups.
    #[error("fold recipe failed for arrangement {arrangement:?}: {source}")]
    FoldRecipe {
        /// Arrangement controller whose hinges were left unchanged.
        arrangement: Entity,
        /// Original recipe error, retained for error-chain inspection.
        #[source]
        source:      Box<dyn StdError + Send + Sync + 'static>,
    },
    /// A recipe left one selected fold-group member without an endpoint.
    #[error("fold recipe assigned no endpoint to selected member {member_entity:?}")]
    MissingFoldAssignment {
        /// Selected member the recipe did not cover.
        member_entity: Entity,
    },
    /// A recipe assigned two endpoints to the same member.
    #[error("fold recipe assigned member {member_entity:?} more than one endpoint")]
    DuplicateFoldAssignment {
        /// Member that received conflicting endpoints.
        member_entity: Entity,
    },
    /// A recipe assigned an endpoint to an entity outside the selected groups.
    #[error("fold recipe assigned an endpoint to unselected member {member_entity:?}")]
    ForeignFoldAssignment {
        /// Entity named by an assignment that the selection does not contain.
        member_entity: Entity,
    },
    /// A recipe assignment carried a NaN or infinite pivot displacement.
    #[error("fold recipe assigned member {member_entity:?} a non-finite pivot offset")]
    NonFiniteFoldAssignmentPivot {
        /// Member whose assigned pivot coordinates were not finite.
        member_entity: Entity,
    },
    /// A recipe named a member whose folded endpoint already differs from its base endpoint.
    #[error("member {member_entity:?} is not resting at its base endpoint")]
    FoldRecipeAwayFromBase {
        /// Member already carrying a distinct folded endpoint.
        member_entity: Entity,
    },
}

impl ArrangementError {
    /// Wraps a downstream provider error without discarding its error source.
    #[must_use]
    pub fn provider(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Provider {
            source: Box::new(source),
        }
    }
}

fn validate_displacement(
    member_entity: Entity,
    displacement: Displacement,
    role: ArrangementConnectionDisplacement,
) -> Result<(), ArrangementError> {
    if displacement.is_finite() {
        Ok(())
    } else {
        Err(ArrangementError::NonFiniteDisplacement {
            member_entity,
            displacement: role,
        })
    }
}

fn validate_forest(connections: &[ArrangementConnection]) -> Result<(), ArrangementError> {
    let targets = connections
        .iter()
        .map(|connection| (connection.member_entity, connection.anchored_to.target()))
        .collect::<HashMap<_, _>>();

    for connection in connections {
        let mut path = HashSet::<Entity>::default();
        let mut member_entity = connection.member_entity;
        loop {
            if !path.insert(member_entity) {
                return Err(ArrangementError::PhysicalCycle { member_entity });
            }
            let Some(target_entity) = targets.get(&member_entity).copied() else {
                break;
            };
            member_entity = target_entity;
        }
    }

    Ok(())
}
