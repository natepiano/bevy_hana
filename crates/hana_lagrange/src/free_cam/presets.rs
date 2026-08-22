use bevy::prelude::Bundle;

use super::FreeCam;
use super::FreeCamBindings;
use crate::input::FreeCamInputMode;
use crate::input::FreeCamPreset;

impl FreeCam {
    /// Returns a `FreeCam` bundle using a built-in input preset.
    #[must_use]
    pub fn with_preset(preset: impl Into<FreeCamPreset>) -> impl Bundle {
        (Self::default(), FreeCamInputMode::with_preset(preset))
    }

    /// Returns a `FreeCam` bundle using app-owned validated bindings.
    #[must_use]
    pub fn with_bindings(bindings: FreeCamBindings) -> impl Bundle {
        (Self::default(), FreeCamInputMode::Bindings(bindings))
    }

    /// Returns a `FreeCam` bundle using manual app-authored input.
    #[must_use]
    pub fn manual() -> impl Bundle { (Self::default(), FreeCamInputMode::Manual) }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::World;

    use super::*;
    use crate::input::BindingsError;
    use crate::input::FreeCamGamepadPreset;
    use crate::input::FreeCamInputGain;

    const TUNED_GAMEPAD_INPUT_GAIN: f32 = 0.5;
    const TUNED_MOVE_SCALE: f32 = 2.0;

    #[test]
    fn presets_insert_requested_input_modes() {
        let mut world = World::new();
        let keyboard_mouse = world
            .spawn(FreeCam::with_preset(FreeCamPreset::keyboard_mouse()))
            .id();
        let gamepad = world
            .spawn(FreeCam::with_preset(FreeCamPreset::gamepad()))
            .id();
        let manual = world.spawn(FreeCam::manual()).id();

        assert_eq!(
            world.get::<FreeCamInputMode>(keyboard_mouse),
            Some(&FreeCamInputMode::with_preset(
                FreeCamPreset::keyboard_mouse()
            ))
        );
        assert_eq!(
            world.get::<FreeCamInputMode>(gamepad),
            Some(&FreeCamInputMode::with_preset(FreeCamPreset::gamepad()))
        );
        assert_eq!(
            world.get::<FreeCamInputMode>(manual),
            Some(&FreeCamInputMode::Manual)
        );
    }

    #[test]
    fn with_bindings_inserts_bindings_input_mode() -> Result<(), BindingsError> {
        let mut world = World::new();
        let bindings = FreeCamPreset::keyboard_mouse().to_bindings()?;
        let camera = world.spawn(FreeCam::with_bindings(bindings)).id();

        assert!(matches!(
            world.get::<FreeCamInputMode>(camera),
            Some(FreeCamInputMode::Bindings(_))
        ));

        Ok(())
    }

    #[test]
    fn with_preset_accepts_tuned_payloads() {
        let tuned = FreeCamGamepadPreset::default().with_move_scale(TUNED_MOVE_SCALE);
        let expected_mode = FreeCamInputMode::with_preset(FreeCamPreset::from(tuned));
        let mut world = World::new();
        let camera = world.spawn(FreeCam::with_preset(tuned)).id();

        assert_eq!(world.get::<FreeCamInputMode>(camera), Some(&expected_mode));
    }

    #[test]
    fn with_preset_accepts_gain_tuned_payloads() {
        let tuned = FreeCamGamepadPreset::default()
            .gamepad_input_gain(FreeCamInputGain::uniform(TUNED_GAMEPAD_INPUT_GAIN));
        let expected_mode = FreeCamInputMode::with_preset(FreeCamPreset::from(tuned));
        let mut world = World::new();
        let camera = world.spawn(FreeCam::with_preset(tuned)).id();

        assert_eq!(world.get::<FreeCamInputMode>(camera), Some(&expected_mode));
    }
}
