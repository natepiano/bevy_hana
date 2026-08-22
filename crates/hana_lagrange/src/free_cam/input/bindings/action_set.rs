//! Per-action free-camera binding-set newtypes stored on [`super::FreeCamBindings`].
//!
//! Each newtype wraps the shared generic [`ImpulseActionBindingSet`] / [`HeldActionBindingSet`]
//! for one free-flight semantic action and exposes the read-only accessors consumed by the
//! `FreeCam` input
//! adapter and `crate::input::control_summary`.

use bevy_enhanced_input::prelude::Binding;

use crate::input::FreeCamHomeAction;
use crate::input::FreeCamLookAction;
use crate::input::FreeCamLookButtonAction;
use crate::input::FreeCamRollAction;
use crate::input::FreeCamRollEngagedAction;
use crate::input::FreeCamTranslateAction;
use crate::input::FreeCamTranslateEngagedAction;
use crate::input::HeldActionBindingEntry;
use crate::input::HeldActionBindingSet;
use crate::input::ImpulseActionBindingEntry;
use crate::input::ImpulseActionBindingSet;
use crate::input::InputBindingEntry;

/// Translate action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FreeCamTranslateActionBindings(
    pub(super) HeldActionBindingSet<FreeCamTranslateAction, FreeCamTranslateEngagedAction>,
);

/// Look action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FreeCamLookActionBindings(
    pub(super) HeldActionBindingSet<FreeCamLookAction, FreeCamLookButtonAction>,
);

/// Roll action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FreeCamRollActionBindings(
    pub(super) HeldActionBindingSet<FreeCamRollAction, FreeCamRollEngagedAction>,
);

/// Home action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FreeCamHomeActionBindings(pub(super) ImpulseActionBindingSet<FreeCamHomeAction>);

impl_binding_forwards!(
    FreeCamTranslateActionBindings,
    HeldActionBindingSet<FreeCamTranslateAction, FreeCamTranslateEngagedAction>,
    HeldActionBindingEntry<FreeCamTranslateAction>,
    entries(pub),
    enabled_entries(pub)
);

impl_binding_forwards!(
    FreeCamLookActionBindings,
    HeldActionBindingSet<FreeCamLookAction, FreeCamLookButtonAction>,
    HeldActionBindingEntry<FreeCamLookAction>,
    entries(pub),
    enabled_entries(pub)
);

impl_binding_forwards!(
    FreeCamRollActionBindings,
    HeldActionBindingSet<FreeCamRollAction, FreeCamRollEngagedAction>,
    HeldActionBindingEntry<FreeCamRollAction>,
    entries(pub),
    enabled_entries(pub)
);

impl_binding_forwards!(
    FreeCamHomeActionBindings,
    ImpulseActionBindingSet<FreeCamHomeAction>,
    ImpulseActionBindingEntry<FreeCamHomeAction>,
    entries(),
    enabled_entries(pub(super))
);

impl FreeCamHomeActionBindings {
    /// Returns the BEI bindings that can trigger home.
    pub fn bindings(&self) -> impl Iterator<Item = Binding> + '_ {
        self.entries()
            .iter()
            .flat_map(|entry| entry.binding_descriptor().entries_slice())
            .map(InputBindingEntry::binding)
    }

    /// Returns the BEI bindings that can trigger home as a vector.
    #[must_use]
    pub fn to_vec(&self) -> Vec<Binding> { self.bindings().collect() }
}
