//! `FreeCam` input-intent vocabulary and channel access.

use core::ops::AddAssign;

use bevy::prelude::Reflect;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;

use super::FreeCamKind;
use crate::input::CameraInputKind;
use crate::input::ControlSpeed;
use crate::input::FreeCamActiveDirections;
use crate::input::FreeCamInputContext;
use crate::input::InputIntent;
use crate::input::IntentChannel;
use crate::input::IntentChannels;
use crate::input::InteractionSources;
use crate::input::ManualInputSource;

/// `FreeCam` translation intent in controller-local axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct TranslateDelta(Vec3);

impl AddAssign for TranslateDelta {
    fn add_assign(&mut self, delta: Self) { self.0 += delta.0; }
}

impl From<Vec3> for TranslateDelta {
    fn from(value: Vec3) -> Self { Self(value) }
}

impl TranslateDelta {
    /// Returns the translation intent vector.
    #[must_use]
    pub const fn vector(self) -> Vec3 { self.0 }
}

/// `FreeCam` look motion expressed in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct LookDelta(Vec2);

impl AddAssign for LookDelta {
    fn add_assign(&mut self, delta: Self) { self.0 += delta.0; }
}

impl From<Vec2> for LookDelta {
    fn from(value: Vec2) -> Self { Self(value) }
}

impl LookDelta {
    /// Returns the logical-pixel delta.
    #[must_use]
    pub const fn pixels(self) -> Vec2 { self.0 }
}

/// `FreeCam` roll intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct RollDelta(f32);

impl AddAssign for RollDelta {
    fn add_assign(&mut self, delta: Self) { self.0 += delta.0; }
}

impl From<f32> for RollDelta {
    fn from(value: f32) -> Self { Self(value) }
}

impl RollDelta {
    /// Returns the roll amount.
    #[must_use]
    pub const fn amount(self) -> f32 { self.0 }
}

impl CameraInputKind for FreeCamKind {
    type Context = FreeCamInputContext;
    type Input = FreeCamInput;
    type Channels = FreeCamChannels;
}

/// Named input channels consumed by the `FreeCam` controller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub struct FreeCamChannels {
    translate:  IntentChannel<TranslateDelta>,
    look:       IntentChannel<LookDelta>,
    roll:       IntentChannel<RollDelta>,
    directions: FreeCamActiveDirections,
}

impl FreeCamChannels {
    const fn has_input(self) -> bool {
        self.translate.is_active() || self.look.is_active() || self.roll.is_active()
    }

    const fn sources(self) -> InteractionSources {
        self.translate
            .sources()
            .union(self.look.sources())
            .union(self.roll.sources())
    }
}

impl IntentChannels for FreeCamChannels {
    fn clear(&mut self) { *self = Self::default(); }

    fn has_input(&self) -> bool { (*self).has_input() }

    fn sources(&self) -> InteractionSources { (*self).sources() }
}

/// Semantic per-frame camera input consumed by the `FreeCam` controller.
pub type FreeCamInput = InputIntent<FreeCamKind>;

impl InputIntent<FreeCamKind> {
    /// Returns the translation delta.
    #[must_use]
    pub const fn translate(&self) -> TranslateDelta { self.channels().translate.delta() }

    /// Returns the look delta.
    #[must_use]
    pub const fn look(&self) -> LookDelta { self.channels().look.delta() }

    /// Returns the roll delta.
    #[must_use]
    pub const fn roll(&self) -> RollDelta { self.channels().roll.delta() }

    /// Returns active sources for translation input this frame.
    #[must_use]
    pub const fn translate_sources(&self) -> InteractionSources {
        self.channels().translate.sources()
    }

    /// Returns active sources for look input this frame.
    #[must_use]
    pub const fn look_sources(&self) -> InteractionSources { self.channels().look.sources() }

    /// Returns active sources for roll input this frame.
    #[must_use]
    pub const fn roll_sources(&self) -> InteractionSources { self.channels().roll.sources() }

    /// Returns the speed variant of the translation input this frame.
    #[must_use]
    pub const fn translate_speed(&self) -> ControlSpeed { self.channels().translate.speed() }

    /// Returns the speed variant of the look input this frame.
    #[must_use]
    pub const fn look_speed(&self) -> ControlSpeed { self.channels().look.speed() }

    /// Returns the speed variant of the roll input this frame.
    #[must_use]
    pub const fn roll_speed(&self) -> ControlSpeed { self.channels().roll.speed() }

    /// Returns the decomposed move directions engaged this frame.
    #[must_use]
    pub const fn directions(&self) -> FreeCamActiveDirections { self.channels().directions }

    /// Returns `true` when the frame carries translation intent.
    #[must_use]
    pub const fn has_translate(&self) -> bool { self.channels().translate.is_active() }

    /// Returns `true` when the frame carries look intent.
    #[must_use]
    pub const fn has_look(&self) -> bool { self.channels().look.is_active() }

    /// Returns `true` when the frame carries roll intent.
    #[must_use]
    pub const fn has_roll(&self) -> bool { self.channels().roll.is_active() }
}

impl InputIntent<FreeCamKind> {
    pub(super) const fn set_translate_speed(&mut self, speed: ControlSpeed) {
        self.channels_mut().translate.set_speed(speed);
    }

    pub(super) const fn set_look_speed(&mut self, speed: ControlSpeed) {
        self.channels_mut().look.set_speed(speed);
    }

    pub(super) const fn set_roll_speed(&mut self, speed: ControlSpeed) {
        self.channels_mut().roll.set_speed(speed);
    }

    pub(super) const fn set_directions(&mut self, directions: FreeCamActiveDirections) {
        self.channels_mut().directions = directions;
    }

    pub(crate) fn add_translate_from_source(
        &mut self,
        delta: impl Into<TranslateDelta>,
        source: ManualInputSource,
    ) -> &mut Self {
        self.add_translate_with_sources(delta, source.sources())
    }

    pub(super) fn add_translate_with_sources(
        &mut self,
        delta: impl Into<TranslateDelta>,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().translate.add_delta(delta, sources);
        self
    }

    pub(crate) const fn mark_translate_active_with_sources(
        &mut self,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut()
            .translate
            .mark_active_with_sources(sources);
        self
    }

    pub(crate) fn add_look_from_source(
        &mut self,
        delta: impl Into<LookDelta>,
        source: ManualInputSource,
    ) -> &mut Self {
        self.add_look_with_sources(delta, source.sources())
    }

    pub(super) fn add_look_with_sources(
        &mut self,
        delta: impl Into<LookDelta>,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().look.add_delta(delta, sources);
        self
    }

    pub(crate) const fn mark_look_active_with_sources(
        &mut self,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().look.mark_active_with_sources(sources);
        self
    }

    pub(crate) fn add_roll_from_source(
        &mut self,
        delta: impl Into<RollDelta>,
        source: ManualInputSource,
    ) -> &mut Self {
        self.add_roll_with_sources(delta, source.sources())
    }

    pub(super) fn add_roll_with_sources(
        &mut self,
        delta: impl Into<RollDelta>,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().roll.add_delta(delta, sources);
        self
    }

    pub(crate) const fn mark_roll_active_with_sources(
        &mut self,
        sources: InteractionSources,
    ) -> &mut Self {
        self.channels_mut().roll.mark_active_with_sources(sources);
        self
    }
}
