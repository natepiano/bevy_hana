//! Validated binding storage: per-semantic-action sets and their entries.
//!
//! Types:
//! - [`ImpulseActionBindingSet`] / [`HeldActionBindingSet`] — generic backing storage parameterized
//!   by [`CameraSemanticAction`] / [`HeldCameraAction`].
//! - [`ImpulseActionBindingEntry`] / [`HeldActionBindingEntry`] — individual binding entries
//!   holding flattened [`InputBindingDescriptor`] data plus source and routing metadata.
//! - [`BindingRoutePolicy`] / [`BindingEngagement`] — routing and engagement enums attached to each
//!   entry.

use std::marker::PhantomData;

use bevy::prelude::Reflect;

use super::descriptor::InputBindingDescriptor;
use super::descriptor::InputBindingEntry;
use super::error::BindingsError;
use super::held_binding::BindingGates;
use super::held_binding::HeldBinding;
use crate::input::CameraSemanticAction;
use crate::input::ControlSpeed;
use crate::input::HeldCameraAction;
use crate::input::InteractionSources;

/// Binding set for one semantic action.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImpulseActionBindingSet<A: CameraSemanticAction> {
    pub(super) entries: Vec<ImpulseActionBindingEntry<A>>,
    pub(super) action:  PhantomData<A>,
}

impl<A: CameraSemanticAction> ImpulseActionBindingSet<A> {
    /// Creates a binding set from validated action entries.
    #[must_use]
    pub const fn from_entries(entries: Vec<ImpulseActionBindingEntry<A>>) -> Self {
        Self {
            entries,
            action: PhantomData,
        }
    }

    /// Returns the number of bindings in the set.
    #[must_use]
    pub const fn len(&self) -> usize { self.entries.len() }

    /// Returns `true` when the set has no bindings.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Returns the binding entries.
    #[must_use]
    pub fn entries(&self) -> &[ImpulseActionBindingEntry<A>] { &self.entries }

    /// Returns binding entries that participate in runtime input.
    pub fn enabled_entries(&self) -> impl Iterator<Item = &ImpulseActionBindingEntry<A>> {
        self.entries.iter().filter(|entry| entry.is_enabled())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeldActionBindingSet<A: HeldCameraAction, E: CameraSemanticAction> {
    pub(super) entries: Vec<HeldActionBindingEntry<A>>,
    pub(super) action:  PhantomData<(A, E)>,
}

impl<A: HeldCameraAction, E: CameraSemanticAction> HeldActionBindingSet<A, E> {
    pub const fn len(&self) -> usize { self.entries.len() }

    pub const fn is_empty(&self) -> bool { self.entries.is_empty() }

    pub fn entries(&self) -> &[HeldActionBindingEntry<A>] { &self.entries }

    pub fn enabled_entries(&self) -> impl Iterator<Item = &HeldActionBindingEntry<A>> {
        self.entries.iter().filter(|entry| entry.is_enabled())
    }
}

/// Binding entry for an impulse camera action.
#[derive(Clone, Debug, PartialEq)]
pub struct ImpulseActionBindingEntry<A: CameraSemanticAction> {
    pub(super) binding:    InputBindingDescriptor,
    pub(super) sources:    InteractionSources,
    pub(super) route:      BindingRoutePolicy,
    pub(super) engagement: BindingEngagement,
    pub(super) action:     PhantomData<A>,
}

impl<A: CameraSemanticAction> ImpulseActionBindingEntry<A> {
    /// Returns the flattened binding descriptor.
    #[must_use]
    pub const fn binding_descriptor(&self) -> &InputBindingDescriptor { &self.binding }

    /// Returns source metadata for this binding.
    #[must_use]
    pub const fn sources(&self) -> InteractionSources { self.sources }

    /// Returns route policy for this binding.
    #[must_use]
    pub const fn route(&self) -> BindingRoutePolicy { self.route }

    /// Returns engagement kind for this binding.
    #[must_use]
    pub const fn engagement(&self) -> BindingEngagement { self.engagement }

    fn is_enabled(&self) -> bool { self.binding.has_enabled_entries() }
}

/// Paired movement and engagement entry for held camera actions.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldActionBindingEntry<A: HeldCameraAction> {
    pub(super) motion:     InputBindingDescriptor,
    pub(super) engagement: InputBindingDescriptor,
    pub(super) gates:      BindingGates,
    pub(super) sources:    InteractionSources,
    pub(super) route:      BindingRoutePolicy,
    pub(super) speed:      ControlSpeed,
    pub(super) action:     PhantomData<A>,
}

impl<A: HeldCameraAction> HeldActionBindingEntry<A> {
    /// Creates a held binding from BEI-style input bindings.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when the source metadata is empty.
    pub fn new(binding: HeldBinding) -> Result<Self, BindingsError> {
        if binding.sources.is_empty() {
            return Err(BindingsError::MissingSources);
        }
        Ok(Self {
            motion:     binding.motion.descriptor(),
            engagement: binding.engagement.descriptor(),
            gates:      binding.gates,
            sources:    binding.sources,
            route:      binding.route,
            speed:      binding.speed,
            action:     PhantomData,
        })
    }

    /// Returns the flattened motion binding descriptor.
    #[must_use]
    pub const fn motion_descriptor(&self) -> &InputBindingDescriptor { &self.motion }

    /// Returns the flattened engagement binding descriptor.
    #[must_use]
    pub const fn engagement_descriptor(&self) -> &InputBindingDescriptor { &self.engagement }

    /// Returns gates applied to both motion and engagement descriptors.
    #[must_use]
    pub const fn gates(&self) -> &BindingGates { &self.gates }

    /// Returns the speed variant this binding represents.
    #[must_use]
    pub const fn speed(&self) -> ControlSpeed { self.speed }

    /// Returns source metadata for this binding.
    #[must_use]
    pub const fn sources(&self) -> InteractionSources { self.sources }

    /// Returns route policy for this binding.
    #[must_use]
    pub const fn route(&self) -> BindingRoutePolicy { self.route }

    fn is_enabled(&self) -> bool {
        self.motion.has_enabled_entries() && self.engagement.has_enabled_entries()
    }

    /// Returns motion binding entries that participate in runtime input.
    pub fn enabled_motion_entries(&self) -> impl Iterator<Item = &InputBindingEntry> {
        self.motion.enabled_entries()
    }
}

/// Route policy attached to a binding entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum BindingRoutePolicy {
    /// Route by cursor or touch position.
    #[default]
    CursorPosition,
    /// Route without position when a latch or explicit owner exists.
    NoPosition,
}

/// Whether a binding is an impulse or held input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum BindingEngagement {
    /// One-frame or impulse binding.
    Impulse,
    /// Held binding with a separate engagement binding.
    Held,
}
