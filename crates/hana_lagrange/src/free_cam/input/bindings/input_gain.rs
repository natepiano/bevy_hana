//! Per-action free-camera input gain, layered over the shared [`InputGain`] vocabulary.

use bevy::prelude::Reflect;

use crate::input::BindingsError;
use crate::input::InputGain;

/// Per-action free-camera `InputGain` values.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct FreeCamInputGain {
    translate: InputGain,
    look:      InputGain,
    roll:      InputGain,
}

impl FreeCamInputGain {
    /// Creates an input gain set with all actions enabled at the default multiplier.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            translate: InputGain::DEFAULT,
            look:      InputGain::DEFAULT,
            roll:      InputGain::DEFAULT,
        }
    }

    /// Creates an input gain set using the same multiplier for every action.
    #[must_use]
    pub const fn uniform(value: f32) -> Self {
        let input_gain = InputGain(value);
        Self {
            translate: input_gain,
            look:      input_gain,
            roll:      input_gain,
        }
    }

    /// Sets translate input gain.
    #[must_use]
    pub const fn translate(mut self, value: f32) -> Self {
        self.translate = InputGain(value);
        self
    }

    /// Sets look input gain.
    #[must_use]
    pub const fn look(mut self, value: f32) -> Self {
        self.look = InputGain(value);
        self
    }

    /// Sets roll input gain.
    #[must_use]
    pub const fn roll(mut self, value: f32) -> Self {
        self.roll = InputGain(value);
        self
    }

    /// Returns translate input gain.
    #[must_use]
    pub const fn translate_input_gain(self) -> InputGain { self.translate }

    /// Returns look input gain.
    #[must_use]
    pub const fn look_input_gain(self) -> InputGain { self.look }

    /// Returns roll input gain.
    #[must_use]
    pub const fn roll_input_gain(self) -> InputGain { self.roll }

    pub(super) fn validate(self) -> Result<(), BindingsError> {
        self.translate.validate()?;
        self.look.validate()?;
        self.roll.validate()
    }
}

impl Default for FreeCamInputGain {
    fn default() -> Self { Self::new() }
}
