use bevy::prelude::MouseButton;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy_enhanced_input::prelude::Binding;

use super::config::OrbitCamPresetConfig;
use crate::input::MouseInputGain;
use crate::input::SmoothScrollInputGain;
use crate::orbit_cam::input::bindings::BindingsError;
use crate::orbit_cam::input::bindings::OrbitCamBindings;
use crate::orbit_cam::input::bindings::OrbitCamBindingsBuilder;
use crate::orbit_cam::input::bindings::OrbitCamInputGain;
use crate::orbit_cam::input::bindings::OrbitCamMouseDrag;
use crate::orbit_cam::input::bindings::OrbitCamMouseWheelZoom;
use crate::orbit_cam::input::bindings::OrbitCamPinchZoom;
use crate::orbit_cam::input::bindings::OrbitCamTrackpadScroll;

/// Configures the default mouse-oriented orbit-camera preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct OrbitCamSimpleMousePreset {
    mouse_input_gain:         OrbitCamInputGain,
    smooth_scroll_input_gain: OrbitCamInputGain,
    home:                     [Option<Binding>; 2],
}

impl OrbitCamSimpleMousePreset {
    /// Builds the simple mouse preset.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when generated descriptors fail
    /// validation.
    pub fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        <Self as OrbitCamPresetConfig>::build(self)
    }

    /// Sets source input gain for mouse-drag and line-wheel input.
    #[must_use]
    pub const fn mouse_input_gain(mut self, input_gain: OrbitCamInputGain) -> Self {
        self.mouse_input_gain = input_gain;
        self
    }

    /// Sets source input gain for Bevy pixel-scroll input.
    #[must_use]
    pub const fn smooth_scroll_input_gain(mut self, input_gain: OrbitCamInputGain) -> Self {
        self.smooth_scroll_input_gain = input_gain;
        self
    }

    /// Adds a binding that returns the camera to its home pose.
    ///
    /// No home input is bound unless this method is called. The preset holds
    /// up to two home bindings (e.g. a key plus a gamepad button); a third
    /// call replaces the second binding.
    #[must_use]
    pub fn home(mut self, home: impl Into<Binding>) -> Self {
        let home = Some(home.into());
        match &mut self.home {
            [first @ None, _] => *first = home,
            [_, second] => *second = home,
        }
        self
    }

    /// Returns whether this preset binds home input.
    #[must_use]
    pub const fn has_home(&self) -> bool { matches!(self.home, [Some(_), _] | [_, Some(_)]) }

    pub(super) fn build_into(
        self,
        builder: OrbitCamBindingsBuilder,
    ) -> Result<OrbitCamBindingsBuilder, BindingsError> {
        self.validate()?;
        Ok(self.add_to(builder))
    }

    fn validate(&self) -> Result<(), BindingsError> {
        self.mouse_input_gain.validate()?;
        self.smooth_scroll_input_gain.validate()
    }

    fn add_to(self, builder: OrbitCamBindingsBuilder) -> OrbitCamBindingsBuilder {
        let builder = builder
            .orbit(
                OrbitCamMouseDrag::new(MouseButton::Left)
                    .with_input_gain(self.mouse_input_gain.orbit_input_gain().value()),
            )
            .pan(
                OrbitCamMouseDrag::new(MouseButton::Right)
                    .with_input_gain(self.mouse_input_gain.pan_input_gain().value()),
            )
            .zoom(
                OrbitCamMouseWheelZoom
                    .with_input_gain(self.mouse_input_gain.zoom_input_gain().value()),
            )
            .zoom(
                OrbitCamTrackpadScroll::default()
                    .with_input_gain(self.smooth_scroll_input_gain.zoom_input_gain().value()),
            );
        let builder = self
            .home
            .into_iter()
            .flatten()
            .fold(builder, OrbitCamBindingsBuilder::home);
        builder.zoom(OrbitCamPinchZoom)
    }
}

impl MouseInputGain for OrbitCamSimpleMousePreset {
    type Gain = OrbitCamInputGain;

    fn mouse_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::mouse_input_gain(self, input_gain)
    }
}

impl SmoothScrollInputGain for OrbitCamSimpleMousePreset {
    type Gain = OrbitCamInputGain;

    fn smooth_scroll_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::smooth_scroll_input_gain(self, input_gain)
    }
}

impl OrbitCamPresetConfig for OrbitCamSimpleMousePreset {
    fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        self.build_into(OrbitCamBindings::builder())?.build()
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::GamepadButton;
    use bevy::prelude::KeyCode;

    use super::*;

    #[test]
    fn default_simple_mouse_preset_binds_no_home() -> Result<(), BindingsError> {
        let preset = OrbitCamSimpleMousePreset::default();

        assert!(!preset.has_home());
        assert!(preset.build()?.home().is_empty());
        Ok(())
    }

    #[test]
    fn simple_mouse_preset_home_setter_binds_the_key() -> Result<(), BindingsError> {
        let preset = OrbitCamSimpleMousePreset::default().home(KeyCode::KeyH);

        assert!(preset.has_home());
        assert_eq!(
            preset.build()?.home().to_vec(),
            vec![Binding::from(KeyCode::KeyH)]
        );
        Ok(())
    }

    #[test]
    fn simple_mouse_preset_home_setter_binds_two_inputs() -> Result<(), BindingsError> {
        let preset = OrbitCamSimpleMousePreset::default()
            .home(KeyCode::KeyH)
            .home(GamepadButton::Select);

        assert_eq!(
            preset.build()?.home().to_vec(),
            vec![
                Binding::from(KeyCode::KeyH),
                Binding::from(GamepadButton::Select)
            ]
        );
        Ok(())
    }
}
