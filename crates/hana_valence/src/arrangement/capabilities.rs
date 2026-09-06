use core::any::Any;
use core::any::TypeId;
use core::any::type_name;
use core::fmt::Debug;
use core::fmt::Formatter;
use core::hash::Hash;

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Component;
use bevy_ecs::world::World;

use super::ArrangementError;
use crate::FoldGroups;

/// Provider capability values for one selection, keyed by concrete Rust type.
///
/// The table stores at most one value per concrete capability type, so a
/// recipe's lookup for its required capability resolves to a single value or to
/// none. Values stay type-erased; nothing here inspects or validates the
/// invariants inside an arbitrary provider capability.
pub(super) struct ArrangementCapabilities {
    entries: Vec<CapabilityEntry>,
}

struct CapabilityEntry {
    type_id:   TypeId,
    type_name: &'static str,
    value:     Box<dyn Any + Send + Sync>,
}

/// Result of associating one concrete capability type with a selection.
pub(super) enum CapabilityAssociation {
    /// The capability became the selection's value for its concrete type.
    Stored,
    /// The selection already had a value of that concrete capability type.
    AlreadyAssociated {
        /// Type name of the capability that was already associated.
        capability: &'static str,
    },
}

impl ArrangementCapabilities {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn associate<C>(&mut self, capability: C) -> CapabilityAssociation
    where
        C: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<C>();
        if self.entries.iter().any(|entry| entry.type_id == type_id) {
            return CapabilityAssociation::AlreadyAssociated {
                capability: type_name::<C>(),
            };
        }
        self.entries.push(CapabilityEntry {
            type_id,
            type_name: type_name::<C>(),
            value: Box::new(capability),
        });

        CapabilityAssociation::Stored
    }

    fn capability<S, C>(&self, selection: &S) -> Result<&C, ArrangementError>
    where
        S: Debug,
        C: Send + Sync + 'static,
    {
        self.entries
            .iter()
            .find(|entry| entry.type_id == TypeId::of::<C>())
            .and_then(|entry| entry.value.downcast_ref::<C>())
            .ok_or_else(|| ArrangementError::MissingCapability {
                selection:  format!("{selection:?}"),
                capability: type_name::<C>(),
            })
    }
}

impl Debug for ArrangementCapabilities {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_list()
            .entries(self.entries.iter().map(|entry| entry.type_name))
            .finish()
    }
}

/// One provider fold-group alternative while its selection type is still known.
///
/// [`super::ArrangementPlan`] owns these values. Materialization turns each one
/// into its erased [`RetainedFoldGroupAlternative`] counterpart.
#[derive(Debug)]
pub(super) struct FoldGroupAlternative<S> {
    selection:    S,
    groups:       FoldGroups,
    capabilities: ArrangementCapabilities,
}

impl<S> FoldGroupAlternative<S>
where
    S: Eq + Hash + Debug + Send + Sync + 'static,
{
    pub(super) const fn new(selection: S, groups: FoldGroups) -> Self {
        Self {
            selection,
            groups,
            capabilities: ArrangementCapabilities::new(),
        }
    }

    pub(super) const fn selection(&self) -> &S { &self.selection }

    pub(super) const fn groups(&self) -> &FoldGroups { &self.groups }

    pub(super) fn associate<C>(&mut self, capability: C) -> CapabilityAssociation
    where
        C: Send + Sync + 'static,
    {
        self.capabilities.associate(capability)
    }

    fn into_retained(self) -> RetainedFoldGroupAlternative {
        RetainedFoldGroupAlternative {
            selection:    Box::new(self.selection),
            groups:       self.groups,
            capabilities: self.capabilities,
        }
    }
}

/// One retained fold-group alternative after its selection type was erased.
struct RetainedFoldGroupAlternative {
    selection:    Box<dyn Any + Send + Sync>,
    groups:       FoldGroups,
    capabilities: ArrangementCapabilities,
}

/// Retained fold-group alternatives and the provider selection type identity.
///
/// This component is the sole retained home for a provider's fold-group
/// alternatives and capabilities. It keeps the selection type identity that
/// materialization erased, so a later typed read is rejected with a named type
/// mismatch instead of silently missing every alternative. It is private,
/// carries no reflection route, and is read only through
/// [`RetainedProviderKnowledge`].
#[derive(Component)]
pub(super) struct RetainedFoldGroupAlternatives {
    selection_type:      TypeId,
    selection_type_name: &'static str,
    alternatives:        Vec<RetainedFoldGroupAlternative>,
}

impl RetainedFoldGroupAlternatives {
    pub(super) fn erase<S>(alternatives: Vec<FoldGroupAlternative<S>>) -> Self
    where
        S: Eq + Hash + Debug + Send + Sync + 'static,
    {
        Self {
            selection_type:      TypeId::of::<S>(),
            selection_type_name: type_name::<S>(),
            alternatives:        alternatives
                .into_iter()
                .map(FoldGroupAlternative::into_retained)
                .collect(),
        }
    }

    pub(super) const fn selection_type_id(&self) -> TypeId { self.selection_type }

    pub(super) const fn alternative_count(&self) -> usize { self.alternatives.len() }

    fn matching<S>(&self, selection: &S) -> Result<&RetainedFoldGroupAlternative, ArrangementError>
    where
        S: Eq + Hash + Debug + Send + Sync + 'static,
    {
        if self.selection_type != TypeId::of::<S>() {
            return Err(ArrangementError::MismatchedFoldGroupSelectionType {
                expected: self.selection_type_name,
                found:    type_name::<S>(),
            });
        }

        self.alternatives
            .iter()
            .find(|alternative| {
                alternative
                    .selection
                    .downcast_ref::<S>()
                    .is_some_and(|retained| retained == selection)
            })
            .ok_or_else(|| ArrangementError::UnknownFoldGroupSelection {
                selection: format!("{selection:?}"),
            })
    }
}

/// Typed read access to everything a provider retained for one arrangement.
///
/// A fold recipe reads its group alternative through [`Self::groups`] and the
/// purpose-specific knowledge that alternative needs through
/// [`Self::capability`]. Both reads are typed by the provider's selection
/// vocabulary: a selection of the wrong Rust type and an unknown selection
/// value are distinct, named failures rather than one empty answer. The
/// underlying retained table stays private, so no read here can mutate
/// arrangement state or bypass a capability's own constructor.
pub struct RetainedProviderKnowledge<'world> {
    alternatives: &'world RetainedFoldGroupAlternatives,
}

impl<'world> RetainedProviderKnowledge<'world> {
    /// Borrows the provider knowledge retained by one materialized arrangement.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError::UnmaterializedArrangement`] when
    /// `arrangement` is not an arrangement controller whose construction
    /// command has already applied.
    pub fn for_arrangement(
        world: &'world World,
        arrangement: Entity,
    ) -> Result<Self, ArrangementError> {
        world
            .get::<RetainedFoldGroupAlternatives>(arrangement)
            .map_or(
                Err(ArrangementError::UnmaterializedArrangement { arrangement }),
                |alternatives| Ok(Self { alternatives }),
            )
    }

    /// Returns the complete group alternative authored for `selection`.
    ///
    /// # Errors
    ///
    /// Returns [`ArrangementError::MismatchedFoldGroupSelectionType`] when `S`
    /// is not the provider's selection type, or
    /// [`ArrangementError::UnknownFoldGroupSelection`] when the provider
    /// authored no alternative for this selection value.
    pub fn groups<S>(&self, selection: &S) -> Result<&FoldGroups, ArrangementError>
    where
        S: Eq + Hash + Debug + Send + Sync + 'static,
    {
        self.alternatives
            .matching(selection)
            .map(|alternative| &alternative.groups)
    }

    /// Returns the `C` capability the provider associated with `selection`.
    ///
    /// # Errors
    ///
    /// Returns the same selection failures as [`Self::groups`], or
    /// [`ArrangementError::MissingCapability`] when the provider
    /// associated no value of type `C` with this selection.
    pub fn capability<S, C>(&self, selection: &S) -> Result<&C, ArrangementError>
    where
        S: Eq + Hash + Debug + Send + Sync + 'static,
        C: Send + Sync + 'static,
    {
        self.alternatives
            .matching(selection)?
            .capabilities
            .capability::<S, C>(selection)
    }
}
