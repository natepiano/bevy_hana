//! Downstream extension point for replacing every selected fold endpoint.

use core::error::Error as StdError;
use core::fmt::Debug;
use core::hash::Hash;

use bevy_ecs::entity::Entity;
use hana_kana::Angle;
use hana_kana::Displacement;

use super::FoldGroups;
use crate::ArrangementConnection;
use crate::ArrangementError;
use crate::RetainedProviderKnowledge;

/// One recipe-authored replacement for a single connection's folded endpoint.
///
/// An assignment deliberately omits the member's [`crate::Edge`] and base
/// angle: those stay provider geometry that a recipe reads but never rewrites.
/// Assignments are returned as one plain vector so an omitted or repeated
/// member is visible while the whole result is validated, before any hinge is
/// written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldAssignment {
    /// Arrangement member whose connection hinge receives this endpoint.
    pub member_entity: Entity,
    /// Resting angle the member reaches at the folded endpoint.
    pub folded_angle:  Angle,
    /// Source-anchor-frame displacement to the physical pivot while folding.
    pub pivot_offset:  Displacement,
}

/// A recipe that turns selected fold groups into replacement fold endpoints.
///
/// The library supplies the selected [`FoldGroups`], the arrangement's
/// validated connections, and the one provider capability the recipe declares.
/// [`FoldGroups`] carries selected group order and
/// [`FoldGroup::connections`](super::FoldGroup::connections) carries member
/// order, so a recipe never joins groups to creases by hand.
///
/// A recipe is a transient extension value: it is consumed once by
/// [`crate::ArrangementCommandsExt::apply_fold_recipe`], owns no ECS state, and
/// is never retained.
pub trait FoldRecipe {
    /// Provider knowledge this recipe cannot derive from groups alone.
    ///
    /// Use [`NoCapability`] for a recipe that needs none.
    type RequiredCapability: FoldRecipeCapability;

    /// Concrete failure this recipe reports, preserved as an error source.
    type Error: StdError + Send + Sync + 'static;

    /// Produces one replacement endpoint per selected connection.
    ///
    /// The result must cover every member of `groups` exactly once and name no
    /// other entity; the caller validates the complete vector before it writes
    /// any hinge.
    ///
    /// # Errors
    ///
    /// Returns this recipe's own error when the selected groups cannot be
    /// folded, such as two groups demanding opposite directions for one member
    /// or an endpoint that is not representable.
    fn fold_assignments(
        &self,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
        required_capability: &Self::RequiredCapability,
    ) -> Result<Vec<FoldAssignment>, Self::Error>;
}

/// A value a provider files against one selected fold-group alternative.
///
/// Implementing this trait is what makes a capability storable through
/// [`crate::ArrangementPlan::with_capability`] and
/// [`crate::Provides`], and it is also what gives the type its
/// [`FoldRecipeCapability`] table lookup. A capability that no recipe can ask
/// for therefore fails to compile at the line that files it.
pub trait ProviderCapability: Send + Sync + 'static {}

/// How a recipe's declared capability is retrieved for one selection.
///
/// Every stored [`ProviderCapability`] inherits the retained-table lookup, so
/// the lookup exists once for every present and future capability.
/// [`NoCapability`] implements this trait directly and never consults the
/// table, which keeps the applying command free of any type-identity branch.
pub trait FoldRecipeCapability: Send + Sync + 'static {
    /// Retrieves this capability for `selection` from retained provider knowledge.
    ///
    /// # Errors
    ///
    /// Returns the selection-typing failures of
    /// [`RetainedProviderKnowledge::groups`], or
    /// [`ArrangementError::MissingCapability`] when the provider filed no value
    /// of this type.
    fn retrieve<'knowledge, S>(
        knowledge: &'knowledge RetainedProviderKnowledge<'_>,
        selection: &S,
    ) -> Result<&'knowledge Self, ArrangementError>
    where
        S: Eq + Hash + Debug + Send + Sync + 'static;
}

impl<C> FoldRecipeCapability for C
where
    C: ProviderCapability,
{
    fn retrieve<'knowledge, S>(
        knowledge: &'knowledge RetainedProviderKnowledge<'_>,
        selection: &S,
    ) -> Result<&'knowledge Self, ArrangementError>
    where
        S: Eq + Hash + Debug + Send + Sync + 'static,
    {
        knowledge.capability::<S, Self>(selection)
    }
}

/// The declaration that a recipe needs no provider capability at all.
///
/// A recipe using `NoCapability` applies against an arrangement whose provider
/// filed nothing, because retrieval answers from this type itself instead of
/// the retained table. No value of it is ever stored.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoCapability;

impl NoCapability {
    const REQUIRED: Self = Self;
}

impl FoldRecipeCapability for NoCapability {
    fn retrieve<'knowledge, S>(
        _: &'knowledge RetainedProviderKnowledge<'_>,
        _: &S,
    ) -> Result<&'knowledge Self, ArrangementError>
    where
        S: Eq + Hash + Debug + Send + Sync + 'static,
    {
        Ok(&Self::REQUIRED)
    }
}
