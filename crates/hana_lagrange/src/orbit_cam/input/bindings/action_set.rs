//! Per-action orbit-camera binding-set newtypes stored on [`super::OrbitCamBindings`].
//!
//! Each newtype wraps the shared generic storage
//! ([`ImpulseActionBindingSet`] / [`HeldActionBindingSet`]) for one orbit semantic action and
//! exposes the
//! read-only accessors consumed by the `OrbitCam` input adapter and
//! `crate::input::control_summary`.

use bevy_enhanced_input::prelude::Binding;

use crate::input::HeldActionBindingEntry;
use crate::input::HeldActionBindingSet;
use crate::input::ImpulseActionBindingEntry;
use crate::input::ImpulseActionBindingSet;
use crate::input::InputBindingEntry;
use crate::input::OrbitCamHomeAction;
use crate::input::OrbitCamOrbitAction;
use crate::input::OrbitCamOrbitEngagedAction;
use crate::input::OrbitCamPanAction;
use crate::input::OrbitCamPanEngagedAction;
use crate::input::OrbitCamZoomCoarseAction;
use crate::input::OrbitCamZoomEngagedAction;
use crate::input::OrbitCamZoomSmoothAction;

/// Orbit action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrbitCamOrbitActionBindings(
    pub(super) HeldActionBindingSet<OrbitCamOrbitAction, OrbitCamOrbitEngagedAction>,
);

/// Pan action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrbitCamPanActionBindings(
    pub(super) HeldActionBindingSet<OrbitCamPanAction, OrbitCamPanEngagedAction>,
);

/// Smooth zoom action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrbitCamZoomSmoothActionBindings(
    pub(super) HeldActionBindingSet<OrbitCamZoomSmoothAction, OrbitCamZoomEngagedAction>,
);

/// Coarse zoom action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrbitCamZoomCoarseActionBindings(
    pub(super) ImpulseActionBindingSet<OrbitCamZoomCoarseAction>,
);

/// Home action binding set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrbitCamHomeActionBindings(pub(super) ImpulseActionBindingSet<OrbitCamHomeAction>);

impl_binding_forwards!(
    OrbitCamOrbitActionBindings,
    HeldActionBindingSet<OrbitCamOrbitAction, OrbitCamOrbitEngagedAction>,
    HeldActionBindingEntry<OrbitCamOrbitAction>,
    entries(pub),
    enabled_entries(pub)
);

impl_binding_forwards!(
    OrbitCamPanActionBindings,
    HeldActionBindingSet<OrbitCamPanAction, OrbitCamPanEngagedAction>,
    HeldActionBindingEntry<OrbitCamPanAction>,
    entries(pub),
    enabled_entries(pub)
);

impl_binding_forwards!(
    OrbitCamZoomSmoothActionBindings,
    HeldActionBindingSet<OrbitCamZoomSmoothAction, OrbitCamZoomEngagedAction>,
    HeldActionBindingEntry<OrbitCamZoomSmoothAction>,
    entries(pub),
    enabled_entries(pub)
);

impl_binding_forwards!(
    OrbitCamZoomCoarseActionBindings,
    ImpulseActionBindingSet<OrbitCamZoomCoarseAction>,
    ImpulseActionBindingEntry<OrbitCamZoomCoarseAction>,
    entries(pub),
    enabled_entries(pub)
);

impl_binding_forwards!(
    OrbitCamHomeActionBindings,
    ImpulseActionBindingSet<OrbitCamHomeAction>,
    ImpulseActionBindingEntry<OrbitCamHomeAction>,
    entries(),
    enabled_entries(pub(super))
);

impl OrbitCamHomeActionBindings {
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
