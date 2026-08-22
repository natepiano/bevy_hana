use bevy::prelude::KeyCode;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy_enhanced_input::prelude::Binding;

use super::config::OrbitCamPresetConfig;
use crate::orbit_cam::input::bindings::BindingsError;
use crate::orbit_cam::input::bindings::InputBinding;
use crate::orbit_cam::input::bindings::OrbitCamBindings;
use crate::orbit_cam::input::bindings::OrbitCamBindingsBuilder;

/// Configures keyboard-only orbit-camera controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct OrbitCamKeyboardPreset {
    home: [Option<Binding>; 2],
}

impl OrbitCamKeyboardPreset {
    /// Builds the keyboard preset.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when generated descriptors fail
    /// validation.
    pub fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        <Self as OrbitCamPresetConfig>::build(self)
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

    pub(super) fn build_into(self, builder: OrbitCamBindingsBuilder) -> OrbitCamBindingsBuilder {
        self.add_to(builder)
    }

    fn add_to(self, builder: OrbitCamBindingsBuilder) -> OrbitCamBindingsBuilder {
        let orbit_keys = InputBinding::cardinal_keys(
            KeyCode::ArrowUp,
            KeyCode::ArrowRight,
            KeyCode::ArrowDown,
            KeyCode::ArrowLeft,
        );
        let pan_keys =
            InputBinding::cardinal_keys(KeyCode::KeyW, KeyCode::KeyD, KeyCode::KeyS, KeyCode::KeyA);
        let zoom_keys = InputBinding::bidirectional_keys(KeyCode::Equal, KeyCode::Minus);
        let builder = builder.orbit(orbit_keys).pan(pan_keys).zoom(zoom_keys);
        self.home
            .into_iter()
            .flatten()
            .fold(builder, OrbitCamBindingsBuilder::home)
    }
}

impl OrbitCamPresetConfig for OrbitCamKeyboardPreset {
    fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        self.build_into(OrbitCamBindings::builder()).build()
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::GamepadButton;

    use super::*;

    #[test]
    fn default_keyboard_preset_binds_no_home() -> Result<(), BindingsError> {
        let preset = OrbitCamKeyboardPreset::default();

        assert!(!preset.has_home());
        assert!(preset.build()?.home().is_empty());
        Ok(())
    }

    #[test]
    fn keyboard_preset_home_setter_binds_the_key() -> Result<(), BindingsError> {
        let preset = OrbitCamKeyboardPreset::default().home(KeyCode::KeyH);

        assert!(preset.has_home());
        assert_eq!(
            preset.build()?.home().to_vec(),
            vec![Binding::from(KeyCode::KeyH)]
        );
        Ok(())
    }

    #[test]
    fn keyboard_preset_home_setter_binds_two_inputs() -> Result<(), BindingsError> {
        let preset = OrbitCamKeyboardPreset::default()
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
