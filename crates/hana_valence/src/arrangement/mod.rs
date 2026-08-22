//! Pure provider authoring for arrangement connection forests.
//!
//! [`ArrangementProvider`] describes a reusable logical arrangement. The
//! library turns its one deterministic member enumeration into
//! [`ArrangementMemberEntities`], then the provider creates one typed
//! [`ArrangementPlan`] against that association. Neither step reads or writes
//! a Bevy [`bevy_ecs::world::World`]; later materialization owns ECS work.

mod capabilities;
mod commands;
mod materialize;
mod member_entities;
mod plan;
mod plugin;
mod sequence_adapter;
#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests;

use core::fmt::Debug;
use core::hash::Hash;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Component;
use bevy_ecs::prelude::FromWorld;
use bevy_ecs::prelude::ReflectFromWorld;
use bevy_ecs::prelude::World;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;
pub use capabilities::RetainedProviderKnowledge;
pub use commands::ArrangementCommandsExt;
pub(crate) use materialize::retained_connections;
pub use member_entities::ArrangementMemberEntities;
pub use plan::ArrangementConnection;
pub use plan::ArrangementConnectionDisplacement;
pub use plan::ArrangementError;
pub use plan::ArrangementPlan;
pub use plan::HingeClearance;
pub use plan::PlannedFoldSequence;
pub use plugin::ArrangementPlugin;
use sequence_adapter::FoldSequenceProvider;
use sequence_adapter::stage_per_group;

use crate::FoldAuthorError;
use crate::FoldGroups;
use crate::FoldSequence;
use crate::FoldTiming;

/// Identity of one non-spatial arrangement controller.
///
/// An `Arrangement` is the owner of arrangement membership and retained
/// construction state, not a member root and not a physical attachment. It
/// may have no members and it does not imply a fold sequence. Member roots
/// point back to this controller through [`Member`], while physical placement
/// remains independently represented by [`crate::AnchoredTo`].
#[derive(Component, Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
#[component(immutable)]
#[reflect(PartialEq, Debug, Default, Clone, opaque)]
pub struct Arrangement;

/// Membership relationship stored on a member root.
///
/// `Member` lives on the root entity that belongs to an arrangement. Its
/// [`Self::arrangement_entity`] field points to the non-spatial
/// [`Arrangement`] controller. It is not a physical parent and it never
/// determines an [`crate::AnchoredTo`] target: forests may have multiple
/// physical roots, branches, disconnected subgraphs, and repeated targets.
///
/// The relationship is immutable. Retargeting replaces the entire component,
/// which lets Bevy remove the source from the old controller's [`Members`]
/// collection before adding it to the new controller's collection.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq, Reflect)]
#[component(immutable)]
#[reflect(PartialEq, Debug, FromWorld, Clone, opaque)]
#[relationship(relationship_target = Members)]
pub struct Member {
    /// Arrangement controller that this member root belongs to.
    #[relationship]
    #[entities]
    #[reflect(ignore)]
    pub arrangement_entity: Entity,
}

impl Member {
    /// Creates membership of `arrangement_entity` for one member root.
    ///
    /// Inserting the returned component on a member root updates the
    /// controller's Bevy-maintained [`Members`] reverse collection. Replacing
    /// it is the supported retargeting operation.
    #[must_use]
    pub const fn new(arrangement_entity: Entity) -> Self { Self { arrangement_entity } }
}

impl FromWorld for Member {
    fn from_world(_: &mut World) -> Self { Self::new(Entity::PLACEHOLDER) }
}

/// Bevy-maintained reverse membership collection on an [`Arrangement`] controller.
///
/// `Members` is the inverse of [`Member`]: it is stored on the controller,
/// not on each member root. Bevy updates it when an immutable `Member`
/// relationship is inserted, replaced, or removed. The collection exposes
/// only ordered read access through its inherent API; applications should
/// create, retarget, or remove a [`Member`] component instead of mutating this
/// collection directly. It intentionally provides no ordinary `Clone`,
/// [`Reflect`](bevy_reflect::Reflect), or
/// [`FromReflect`](bevy_reflect::FromReflect) route. Bevy's public
/// [`RelationshipTarget`](bevy_ecs::relationship::RelationshipTarget) trait
/// necessarily retains its documented
/// [`collection_mut_risky`](bevy_ecs::relationship::RelationshipTarget::collection_mut_risky)
/// and
/// [`from_collection_risky`](bevy_ecs::relationship::RelationshipTarget::from_collection_risky)
/// maintenance escape hatches; using either can violate the reverse
/// collection invariant.
///
/// Iteration follows Bevy's current relationship order. That order is useful
/// when a caller needs enumeration, but it does not express physical parentage
/// and must not be used to infer an [`crate::AnchoredTo`] target.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = Member)]
pub struct Members(Vec<Entity>);

impl Members {
    /// Iterates over current member roots in Bevy relationship order.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ { self.0.iter().copied() }

    /// Returns the number of member roots currently related to this controller.
    #[must_use]
    pub const fn len(&self) -> usize { self.0.len() }

    /// Returns whether this controller currently has no member roots.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.0.is_empty() }
}

/// Result of looking up an existing entity for one provider logical member.
///
/// `Bound` supplies the entity that will receive [`Member`] during successful
/// materialization. `Missing` is a deliberate binding failure: it means the
/// provider's listed member has no entity to bind, rather than an optional
/// member that may be skipped. [`ArrangementCommandsExt::spawn_arrangement_from_members`]
/// consumes every result before it inserts any relationship and rejects this
/// state with [`ArrangementError::MissingMemberBinding`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberBinding {
    /// Existing entity selected for this required logical member.
    Bound(Entity),
    /// Explicit failure to find an entity for this required logical member.
    Missing,
}

/// Downstream extension point for authoring a logical arrangement.
///
/// `Member` is this provider's authoring-time logical member identifier, not
/// the ECS [`crate::Member`] relationship component. The library consumes
/// [`Self::members`] once, preserves that order, and supplies the resulting
/// [`ArrangementMemberEntities`] to [`Self::generate_plan`]. A provider with
/// no fold-group alternatives uses [`core::convert::Infallible`] for
/// `FoldGroupSelection`.
///
/// `FoldGroupSelection` remains part of the returned [`ArrangementPlan`] type
/// until later arrangement materialization erases it. This prevents a
/// provider's fold selection vocabulary from being mixed with another
/// provider's vocabulary in typed APIs.
pub trait ArrangementProvider {
    /// Provider-defined logical member identifier.
    type Member: Eq + Hash + Debug;

    /// Provider-defined choice of fold-group layout.
    type FoldGroupSelection: Eq + Hash + Debug + Send + Sync + 'static;

    /// Enumerates each logical member once in the provider's deterministic order.
    ///
    /// Repeating a member is rejected while the library constructs
    /// [`ArrangementMemberEntities`]. The order becomes the authoritative
    /// logical-member order used by [`ArrangementPlan::try_new`].
    fn members(&self) -> impl Iterator<Item = Self::Member>;

    /// Generates one structurally valid connection forest for this provider.
    ///
    /// `members` is the read-only association produced from this provider's
    /// [`Self::members`] output. Plan construction must use this exact value;
    /// a provider must not collect or submit a second member/entity list.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError`] when provider authoring cannot produce a
    /// valid complete plan. Wrap a provider-specific failure with
    /// [`ArrangementError::provider`] so callers can inspect its source.
    fn generate_plan(
        &self,
        members: &ArrangementMemberEntities<Self::Member>,
    ) -> Result<ArrangementPlan<Self::FoldGroupSelection>, ArrangementError>;

    /// Adds one fold stage per group of `selection`, in the provider's group
    /// order.
    ///
    /// Every stage inherits `default_timing` and folds its members all the way
    /// to their folded endpoint, which is the ordinary staged unfold of a
    /// provider that already orders its groups outward from the root. The
    /// authored sequence travels with the plan and materialization inserts it
    /// on the arrangement controller.
    ///
    /// The returned provider generates the wrapped provider's plan first, so a
    /// provider that retains no alternative under `selection` — a
    /// single-cell [`crate::QuadSheet`] or [`crate::TriangleSheet`] has no
    /// crease and therefore no alternative at all — fails with
    /// [`ArrangementError::UnknownFoldGroupSelection`] and authors nothing.
    fn with_fold_sequence(
        self,
        selection: Self::FoldGroupSelection,
        default_timing: FoldTiming,
    ) -> impl ArrangementProvider<Member = Self::Member, FoldGroupSelection = Self::FoldGroupSelection>
    where
        Self: Sized,
    {
        FoldSequenceProvider::new(self, selection, move |groups: &FoldGroups| {
            Ok(stage_per_group(groups, default_timing.clone()))
        })
    }

    /// Adds the fold sequence `sequence_for_groups` authors from `selection`'s
    /// groups.
    ///
    /// The closure runs once per plan generation, synchronously, after the
    /// wrapped provider's logical members already have entities and before any
    /// scene or ECS write. It may combine, subdivide, reorder, or omit groups
    /// and choose every stage's and member's timing. Its failure aborts the
    /// spawn with the error kept as an [`ArrangementError::Provider`] source.
    ///
    /// `F` is [`Fn`] rather than [`FnOnce`]: [`Self::generate_plan`] takes
    /// `&self` and may run more than once, and a once-only closure would need
    /// hidden consumed state plus a failure path for the second call.
    ///
    /// The unretained-selection failure of [`Self::with_fold_sequence`] applies
    /// here unchanged, and it happens before the closure runs.
    fn with_custom_fold_sequence<F>(
        self,
        selection: Self::FoldGroupSelection,
        sequence_for_groups: F,
    ) -> impl ArrangementProvider<Member = Self::Member, FoldGroupSelection = Self::FoldGroupSelection>
    where
        Self: Sized,
        F: Fn(&FoldGroups) -> Result<FoldSequence, FoldAuthorError>,
    {
        FoldSequenceProvider::new(self, selection, sequence_for_groups)
    }
}

/// Provider knowledge a fold recipe requires but cannot derive itself.
///
/// A provider implements `Provides<C>` for each capability type its recipes
/// need, such as [`crate::WindingClearance`], and calls [`Self::provide`] from
/// inside [`ArrangementProvider::generate_plan`] to build the value it then
/// associates with [`ArrangementPlan::with_capability`]. Application code never
/// calls it: by the time an application holds an arrangement, the capability is
/// already retained with its selected fold groups.
pub trait Provides<C>: ArrangementProvider
where
    C: crate::ProviderCapability,
{
    /// Produces the `C` capability for one selected fold-group alternative.
    ///
    /// `groups` and `connections` are the same values the provider is about to
    /// retain, so a capability can be built against the exact members and
    /// attachments it must cover.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError`] when the provider cannot produce a complete
    /// capability. Wrap a capability-specific failure such as
    /// [`crate::FoldAuthorError`] with [`ArrangementError::provider`] so the
    /// caller keeps its source.
    fn provide(
        &self,
        selection: &Self::FoldGroupSelection,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
    ) -> Result<C, ArrangementError>;
}
