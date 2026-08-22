//! Per-action orbit-camera input gain, layered over the shared [`InputGain`] vocabulary.

use bevy::prelude::Reflect;

use crate::input::BindingsError;
use crate::input::InputGain;

/// Per-action orbit-camera `InputGain` values.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct OrbitCamInputGain {
    orbit: InputGain,
    pan:   InputGain,
    zoom:  InputGain,
}

impl OrbitCamInputGain {
    /// Creates an input gain set with all actions enabled at the default multiplier.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            orbit: InputGain::DEFAULT,
            pan:   InputGain::DEFAULT,
            zoom:  InputGain::DEFAULT,
        }
    }

    /// Creates an input gain set using the same multiplier for every action.
    #[must_use]
    pub const fn uniform(value: f32) -> Self {
        let input_gain = InputGain(value);
        Self {
            orbit: input_gain,
            pan:   input_gain,
            zoom:  input_gain,
        }
    }

    /// Sets orbit input gain.
    #[must_use]
    pub const fn orbit(mut self, value: f32) -> Self {
        self.orbit = InputGain(value);
        self
    }

    /// Sets pan input gain.
    #[must_use]
    pub const fn pan(mut self, value: f32) -> Self {
        self.pan = InputGain(value);
        self
    }

    /// Sets zoom input gain.
    #[must_use]
    pub const fn zoom(mut self, value: f32) -> Self {
        self.zoom = InputGain(value);
        self
    }

    /// Returns orbit input gain.
    #[must_use]
    pub const fn orbit_input_gain(self) -> InputGain { self.orbit }

    /// Returns pan input gain.
    #[must_use]
    pub const fn pan_input_gain(self) -> InputGain { self.pan }

    /// Returns zoom input gain.
    #[must_use]
    pub const fn zoom_input_gain(self) -> InputGain { self.zoom }

    pub(super) fn validate(self) -> Result<(), BindingsError> {
        self.orbit.validate()?;
        self.pan.validate()?;
        self.zoom.validate()
    }
}

impl Default for OrbitCamInputGain {
    fn default() -> Self { Self::new() }
}
