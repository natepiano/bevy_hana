use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ModKeys;

use super::config::OrbitCamPresetConfig;
use crate::input::MouseInputGain;
use crate::input::SmoothScrollInputGain;
use crate::orbit_cam::input::bindings::BindingsError;
use crate::orbit_cam::input::bindings::CameraInputScalePolicy;
use crate::orbit_cam::input::bindings::CameraSlowMode;
use crate::orbit_cam::input::bindings::OrbitCamBindings;
use crate::orbit_cam::input::bindings::OrbitCamBindingsBuilder;
use crate::orbit_cam::input::bindings::OrbitCamInputGain;
use crate::orbit_cam::input::bindings::OrbitCamMouseDrag;
use crate::orbit_cam::input::bindings::OrbitCamMouseWheelZoom;
use crate::orbit_cam::input::bindings::OrbitCamPinchZoom;
use crate::orbit_cam::input::bindings::OrbitCamTrackpadScroll;

/// Configures Blender-style pointer and smooth-scroll camera controls.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Default)]
pub struct OrbitCamBlenderLikePreset {
    mouse_input_gain:         OrbitCamInputGain,
    smooth_scroll_input_gain: OrbitCamInputGain,
    zoom_mod_keys:            ModKeys,
    slow_toggle_key:          Option<KeyCode>,
    slow_toggle_mod_keys:     ModKeys,
    slow_scale:               f32,
    home:                     [Option<Binding>; 2],
}

impl OrbitCamBlenderLikePreset {
    const DEFAULT_NORMAL_SCALE: f32 = 1.0;
    const DEFAULT_SLOW_SCALE: f32 = 0.05;
    const MAX_SLOW_SCALE: f32 = 1.0;
    const MIN_SLOW_SCALE: f32 = 0.0;

    /// Builds the Blender-like preset.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when generated descriptors fail
    /// validation.
    pub fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        <Self as OrbitCamPresetConfig>::build(self)
    }

    /// Sets the keyboard modifiers required for trackpad zoom.
    #[must_use]
    pub const fn zoom_mod_keys(mut self, zoom_mod_keys: ModKeys) -> Self {
        self.zoom_mod_keys = zoom_mod_keys;
        self
    }

    /// Sets the key that toggles slow mode on or off for this camera.
    #[must_use]
    pub const fn slow_toggle_key(mut self, slow_toggle_key: Option<KeyCode>) -> Self {
        self.slow_toggle_key = slow_toggle_key;
        self
    }

    /// Sets the modifier keys held with the toggle key to fire the slow-mode toggle.
    #[must_use]
    pub const fn slow_toggle_mod_keys(mut self, slow_toggle_mod_keys: ModKeys) -> Self {
        self.slow_toggle_mod_keys = slow_toggle_mod_keys;
        self
    }

    /// Sets the scale applied to all inputs when slow mode is active.
    #[must_use]
    pub const fn slow_scale(mut self, slow_scale: f32) -> Self {
        self.slow_scale = slow_scale;
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

    pub(super) fn build_into(
        self,
        builder: OrbitCamBindingsBuilder,
    ) -> Result<OrbitCamBindingsBuilder, BindingsError> {
        self.validate()?;
        Ok(self.add_to(builder))
    }

    fn validate(&self) -> Result<(), BindingsError> {
        self.mouse_input_gain.validate()?;
        self.smooth_scroll_input_gain.validate()?;
        if self.slow_toggle_key.is_some()
            && (!self.slow_scale.is_finite()
                || self.slow_scale <= Self::MIN_SLOW_SCALE
                || self.slow_scale > Self::MAX_SLOW_SCALE)
        {
            return Err(BindingsError::InvalidScale);
        }
        Ok(())
    }

    fn add_to(self, builder: OrbitCamBindingsBuilder) -> OrbitCamBindingsBuilder {
        let builder = if let Some(toggle_key) = self.slow_toggle_key {
            builder.slow_mode(CameraSlowMode {
                toggle_key,
                mod_keys: self.slow_toggle_mod_keys,
                scale: CameraInputScalePolicy {
                    normal: Self::DEFAULT_NORMAL_SCALE,
                    slow:   self.slow_scale,
                },
            })
        } else {
            builder
        };

        let builder = builder
            .orbit(
                OrbitCamMouseDrag::new(MouseButton::Middle)
                    .with_input_gain(self.mouse_input_gain.orbit_input_gain().value()),
            )
            .orbit(
                OrbitCamTrackpadScroll::default()
                    .with_input_gain(self.smooth_scroll_input_gain.orbit_input_gain().value()),
            )
            .pan(
                OrbitCamMouseDrag::new(MouseButton::Middle)
                    .with_mod_keys(ModKeys::SHIFT)
                    .with_input_gain(self.mouse_input_gain.pan_input_gain().value()),
            )
            .pan(
                OrbitCamTrackpadScroll::default()
                    .with_mod_keys(ModKeys::SHIFT)
                    .with_input_gain(self.smooth_scroll_input_gain.pan_input_gain().value()),
            )
            .zoom(
                OrbitCamMouseWheelZoom
                    .with_input_gain(self.mouse_input_gain.zoom_input_gain().value()),
            )
            .zoom(
                OrbitCamTrackpadScroll::default()
                    .with_mod_keys(self.zoom_mod_keys)
                    .with_input_gain(self.smooth_scroll_input_gain.zoom_input_gain().value()),
            )
            .zoom(OrbitCamPinchZoom);
        self.home
            .into_iter()
            .flatten()
            .fold(builder, OrbitCamBindingsBuilder::home)
    }
}

impl Default for OrbitCamBlenderLikePreset {
    fn default() -> Self {
        Self {
            mouse_input_gain:         OrbitCamInputGain::default(),
            smooth_scroll_input_gain: OrbitCamInputGain::default(),
            zoom_mod_keys:            ModKeys::CONTROL,
            slow_toggle_key:          Some(KeyCode::KeyS),
            slow_toggle_mod_keys:     ModKeys::ALT,
            slow_scale:               Self::DEFAULT_SLOW_SCALE,
            home:                     [None; 2],
        }
    }
}

impl MouseInputGain for OrbitCamBlenderLikePreset {
    type Gain = OrbitCamInputGain;

    fn mouse_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::mouse_input_gain(self, input_gain)
    }
}

impl SmoothScrollInputGain for OrbitCamBlenderLikePreset {
    type Gain = OrbitCamInputGain;

    fn smooth_scroll_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::smooth_scroll_input_gain(self, input_gain)
    }
}

impl OrbitCamPresetConfig for OrbitCamBlenderLikePreset {
    fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        self.build_into(OrbitCamBindings::builder())?.build()
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::GamepadButton;

    use super::*;

    #[test]
    fn default_blender_like_preset_binds_no_home() -> Result<(), BindingsError> {
        let preset = OrbitCamBlenderLikePreset::default();

        assert!(!preset.has_home());
        assert!(preset.build()?.home().is_empty());
        Ok(())
    }

    #[test]
    fn blender_like_preset_home_setter_binds_the_key() -> Result<(), BindingsError> {
        let preset = OrbitCamBlenderLikePreset::default().home(KeyCode::KeyH);

        assert!(preset.has_home());
        assert_eq!(
            preset.build()?.home().to_vec(),
            vec![Binding::from(KeyCode::KeyH)]
        );
        Ok(())
    }

    #[test]
    fn blender_like_preset_home_setter_binds_two_inputs() -> Result<(), BindingsError> {
        let preset = OrbitCamBlenderLikePreset::default()
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
