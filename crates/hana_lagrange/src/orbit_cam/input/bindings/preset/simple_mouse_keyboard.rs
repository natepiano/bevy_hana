use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy_enhanced_input::prelude::Binding;

use super::config::OrbitCamPresetConfig;
use super::keyboard::OrbitCamKeyboardPreset;
use super::simple_mouse::OrbitCamSimpleMousePreset;
use crate::input::MouseInputGain;
use crate::input::SmoothScrollInputGain;
use crate::orbit_cam::input::bindings::BindingsError;
use crate::orbit_cam::input::bindings::OrbitCamBindings;
use crate::orbit_cam::input::bindings::OrbitCamBindingsBuilder;
use crate::orbit_cam::input::bindings::OrbitCamInputGain;

/// Configures simple mouse controls plus keyboard camera controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct OrbitCamSimpleMouseKeyboardPreset {
    pointer:  OrbitCamSimpleMousePreset,
    keyboard: OrbitCamKeyboardPreset,
}

impl OrbitCamSimpleMouseKeyboardPreset {
    /// Builds the simple mouse plus keyboard preset.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when generated descriptors fail
    /// validation.
    pub fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        <Self as OrbitCamPresetConfig>::build(self)
    }

    /// Replaces the simple mouse child preset.
    #[must_use]
    pub const fn simple_mouse(mut self, preset: OrbitCamSimpleMousePreset) -> Self {
        self.pointer = preset;
        self
    }

    /// Replaces the keyboard child preset.
    #[must_use]
    pub const fn keyboard(mut self, preset: OrbitCamKeyboardPreset) -> Self {
        self.keyboard = preset;
        self
    }

    /// Adds a keyboard-child binding that returns the camera to its home pose.
    ///
    /// No home input is bound unless this method is called.
    #[must_use]
    pub fn home(mut self, home: impl Into<Binding>) -> Self {
        self.keyboard = self.keyboard.home(home);
        self
    }

    /// Returns whether either child preset binds home input.
    #[must_use]
    pub const fn has_home(&self) -> bool { self.pointer.has_home() || self.keyboard.has_home() }

    /// Sets source input gain for mouse-drag and line-wheel input.
    #[must_use]
    pub const fn mouse_input_gain(mut self, input_gain: OrbitCamInputGain) -> Self {
        self.pointer = self.pointer.mouse_input_gain(input_gain);
        self
    }

    /// Sets source input gain for Bevy pixel-scroll input.
    #[must_use]
    pub const fn smooth_scroll_input_gain(mut self, input_gain: OrbitCamInputGain) -> Self {
        self.pointer = self.pointer.smooth_scroll_input_gain(input_gain);
        self
    }

    pub(super) fn build_into(
        self,
        builder: OrbitCamBindingsBuilder,
    ) -> Result<OrbitCamBindingsBuilder, BindingsError> {
        let builder = self.pointer.build_into(builder)?;
        Ok(self.keyboard.build_into(builder))
    }
}

impl MouseInputGain for OrbitCamSimpleMouseKeyboardPreset {
    type Gain = OrbitCamInputGain;

    fn mouse_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::mouse_input_gain(self, input_gain)
    }
}

impl SmoothScrollInputGain for OrbitCamSimpleMouseKeyboardPreset {
    type Gain = OrbitCamInputGain;

    fn smooth_scroll_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::smooth_scroll_input_gain(self, input_gain)
    }
}

impl OrbitCamPresetConfig for OrbitCamSimpleMouseKeyboardPreset {
    fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        self.build_into(OrbitCamBindings::builder())?.build()
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::KeyCode;

    use super::*;

    #[test]
    fn simple_mouse_keyboard_home_setter_binds_one_key() -> Result<(), BindingsError> {
        let preset = OrbitCamSimpleMouseKeyboardPreset::default().home(KeyCode::KeyH);

        assert!(preset.has_home());
        assert_eq!(
            preset.build()?.home().to_vec(),
            vec![Binding::from(KeyCode::KeyH)]
        );
        Ok(())
    }
}
