use core::marker::PhantomData;
use core::ops::AddAssign;

use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::reflect::FromReflect;
use bevy::reflect::TypePath;

use super::ControlSpeed;
use super::InteractionSources;
use crate::CameraKind;

mod sealed {
    pub trait Sealed {}
}

use sealed::Sealed;

/// Input-stack type family for a lagrange camera kind.
///
/// This trait is sealed; implementers are the crate-defined [`OrbitCamKind`]
/// and [`FreeCamKind`]. Camera kinds are defined by this crate.
///
/// [`FreeCamKind`]: crate::FreeCamKind
/// [`OrbitCamKind`]: crate::OrbitCamKind
pub trait CameraInputKind: CameraKind + TypePath + Sealed {
    /// The enhanced-input context component for this camera kind.
    type Context: Component;
    /// The per-frame input intent component for this camera kind.
    type Input: Component;
    /// The named input channels this camera kind consumes.
    type Channels: IntentChannels + Copy + Default + FromReflect + Reflect + TypePath;
}

/// Named input channels for one camera kind.
///
/// This trait is sealed; implementers are the crate-defined `OrbitCamChannels`
/// and `FreeCamChannels`. Camera kinds are defined by this crate.
pub trait IntentChannels: Sealed {
    /// Clears every channel for the frame.
    fn clear(&mut self);

    /// Returns `true` when any channel carries intent this frame.
    fn has_input(&self) -> bool;

    /// Returns all active sources for this frame.
    fn sources(&self) -> InteractionSources;
}

/// Generic per-frame input intent consumed by a camera controller.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, where K::Channels: FromReflect + TypePath)]
pub struct InputIntent<K: CameraInputKind> {
    channels: K::Channels,
    #[reflect(ignore)]
    marker:   PhantomData<fn() -> K>,
}

impl<K: CameraInputKind> InputIntent<K> {
    /// Returns the named input channels.
    #[must_use]
    pub const fn channels(&self) -> &K::Channels { &self.channels }

    /// Returns the named input channels for mutation.
    pub const fn channels_mut(&mut self) -> &mut K::Channels { &mut self.channels }

    /// Clears every input channel for the frame.
    pub fn clear(&mut self) -> &mut Self {
        self.channels.clear();
        self
    }

    /// Returns `true` when any channel carries intent this frame.
    #[must_use]
    pub fn has_input(&self) -> bool { self.channels.has_input() }

    /// Returns all active sources for this frame.
    #[must_use]
    pub fn sources(&self) -> InteractionSources { self.channels.sources() }
}

impl<K: CameraInputKind> Default for InputIntent<K> {
    fn default() -> Self {
        Self {
            channels: K::Channels::default(),
            marker:   PhantomData,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
enum IntentChannelActivity {
    #[default]
    Inactive,
    Active,
}

impl IntentChannelActivity {
    const fn is_active(self) -> bool { matches!(self, Self::Active) }
}

/// One per-frame input lane, with its delta, source attribution, and speed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct IntentChannel<D> {
    delta:    D,
    sources:  InteractionSources,
    speed:    ControlSpeed,
    activity: IntentChannelActivity,
}

impl<D: Copy> IntentChannel<D> {
    /// Returns the channel's accumulated delta.
    #[must_use]
    pub const fn delta(self) -> D { self.delta }

    /// Returns active sources for this channel.
    #[must_use]
    pub const fn sources(self) -> InteractionSources { self.sources }

    /// Returns the speed variant for this channel.
    #[must_use]
    pub const fn speed(self) -> ControlSpeed { self.speed }

    /// Returns `true` when this channel carries intent this frame.
    #[must_use]
    pub const fn is_active(self) -> bool { self.activity.is_active() }
}

impl<D: AddAssign> IntentChannel<D> {
    pub(crate) fn add_delta(
        &mut self,
        delta: impl Into<D>,
        sources: InteractionSources,
    ) -> &mut Self {
        self.delta += delta.into();
        self.activity = IntentChannelActivity::Active;
        self.sources = self.sources.union(sources);
        self
    }
}

impl<D> IntentChannel<D> {
    pub(crate) const fn set_speed(&mut self, speed: ControlSpeed) { self.speed = speed; }

    pub(crate) const fn mark_active_with_sources(
        &mut self,
        sources: InteractionSources,
    ) -> &mut Self {
        self.activity = IntentChannelActivity::Active;
        self.sources = self.sources.union(sources);
        self
    }
}

impl<D: Default> IntentChannel<D> {
    pub(crate) fn clear(&mut self) -> &mut Self {
        *self = Self::default();
        self
    }
}

impl Sealed for crate::OrbitCamKind {}
impl Sealed for crate::FreeCamKind {}
impl Sealed for crate::OrbitCamChannels {}
impl Sealed for crate::FreeCamChannels {}
