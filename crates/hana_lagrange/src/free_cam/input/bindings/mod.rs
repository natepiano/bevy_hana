//! Free-camera binding model: preset, builder, validated bindings, and the free-flight per-action
//! newtypes layered over the shared `crate::input` binding vocabulary, consumed by the
//! `FreeCam` input adapter and `crate::input::control_summary`.
//!
//! Submodules:
//! - [`preset`] — built-in [`FreeCamPreset`] keymaps plus the [`FreeCamLookPitch`] setting.
//! - [`builder`] — [`FreeCamBindingsBuilder`], the per-action binding wrappers, and the user-facing
//!   source types (translate keys, mouse look).
//! - [`action_set`] — per-action binding-set newtypes written by the validator and read by the
//!   adapter.
//! - [`input_gain`] — the [`FreeCamInputGain`] per-action gain layered over
//!   [`crate::input::InputGain`].
//! - [`validate`] — builder descriptor lists → [`FreeCamBindings`] lowering.
//!
//! This file holds the validated runtime [`FreeCamBindings`] value and the cross-cutting
//! integration tests.

mod action_set;
mod builder;
mod input_gain;
mod preset;
mod validate;

pub use action_set::FreeCamHomeActionBindings;
pub use action_set::FreeCamLookActionBindings;
pub use action_set::FreeCamRollActionBindings;
pub use action_set::FreeCamTranslateActionBindings;
use bevy::prelude::Reflect;
pub use builder::FreeCamBindingsBuilder;
pub use builder::FreeCamLookBinding;
pub use builder::FreeCamMouseLook;
pub use builder::FreeCamRollBinding;
pub use builder::FreeCamTranslateBinding;
pub use builder::FreeCamTranslateKeys;
pub use input_gain::FreeCamInputGain;
pub use preset::FreeCamGamepadLayout;
pub use preset::FreeCamGamepadPreset;
pub use preset::FreeCamKeyboardMousePreset;
pub use preset::FreeCamLookPitch;
pub use preset::FreeCamPreset;
pub use preset::FreeCamPresetKind;

use crate::input::CameraInputGamepadSelectionPolicy;
use crate::input::CameraSlowMode;
use crate::input::FreeCamHomeAction;
use crate::input::ImpulseActionBindingEntry;

/// Validated runtime binding specification for a `FreeCam`.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct FreeCamBindings {
    pub(super) translate:  FreeCamTranslateActionBindings,
    pub(super) look:       FreeCamLookActionBindings,
    pub(super) roll:       FreeCamRollActionBindings,
    pub(super) look_pitch: FreeCamLookPitch,
    pub(super) slow_mode:  Option<CameraSlowMode>,
    pub(super) gamepad:    CameraInputGamepadSelectionPolicy,
    pub(super) home:       FreeCamHomeActionBindings,
}

impl FreeCamBindings {
    /// Creates a `FreeCamBindings` builder.
    #[must_use]
    pub fn builder() -> FreeCamBindingsBuilder { FreeCamBindingsBuilder::default() }

    /// Returns translate action bindings.
    #[must_use]
    pub const fn translate(&self) -> &FreeCamTranslateActionBindings { &self.translate }

    /// Returns look action bindings.
    #[must_use]
    pub const fn look(&self) -> &FreeCamLookActionBindings { &self.look }

    /// Returns roll action bindings.
    #[must_use]
    pub const fn roll(&self) -> &FreeCamRollActionBindings { &self.roll }

    /// Returns whether mouse Y is passed through or inverted for look input.
    #[must_use]
    pub const fn look_pitch(&self) -> FreeCamLookPitch { self.look_pitch }

    /// Returns the slow-mode policy.
    #[must_use]
    pub const fn slow_mode(&self) -> Option<&CameraSlowMode> { self.slow_mode.as_ref() }

    /// Replaces the pitch-axis direction for look input.
    #[must_use]
    pub const fn with_look_pitch(mut self, look_pitch: FreeCamLookPitch) -> Self {
        self.look_pitch = look_pitch;
        self
    }

    /// Replaces the slow-mode policy.
    #[must_use]
    pub const fn with_slow_mode(mut self, slow_mode: CameraSlowMode) -> Self {
        self.slow_mode = Some(slow_mode);
        self
    }

    /// Removes the slow-mode policy.
    #[must_use]
    pub const fn without_slow_mode(mut self) -> Self {
        self.slow_mode = None;
        self
    }

    /// Returns the gamepad routing policy.
    #[must_use]
    pub const fn gamepad(&self) -> CameraInputGamepadSelectionPolicy { self.gamepad }

    /// Returns the sources that reset the camera to its home pose.
    #[must_use]
    pub const fn home(&self) -> &FreeCamHomeActionBindings { &self.home }

    pub(super) fn enabled_home_entries(
        &self,
    ) -> impl Iterator<Item = &ImpulseActionBindingEntry<FreeCamHomeAction>> {
        self.home.enabled_entries()
    }

    /// Replaces the gamepad routing policy.
    #[must_use]
    pub const fn with_gamepad(mut self, gamepad: CameraInputGamepadSelectionPolicy) -> Self {
        self.gamepad = gamepad;
        self
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::GamepadAxis;
    use bevy::prelude::GamepadButton;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MouseButton;
    use bevy_enhanced_input::prelude::Binding;
    use bevy_enhanced_input::prelude::ModKeys;

    use super::*;
    use crate::input::BindingsError;
    use crate::input::CameraInputScalePolicy;
    use crate::input::HeldActionBindingEntry;
    use crate::input::HeldCameraAction;
    use crate::input::InputBinding;
    use crate::input::InputBindingEntry;
    use crate::input::InputGain;
    use crate::input::InteractionSources;

    const CUSTOM_LOOK_INPUT_GAIN: f32 = 0.5;
    const CUSTOM_ROLL_INPUT_GAIN: f32 = 0.25;
    const CUSTOM_TRANSLATE_INPUT_GAIN: f32 = 0.75;

    fn motion_bindings<A>(entry: &HeldActionBindingEntry<A>) -> Vec<Binding>
    where
        A: HeldCameraAction,
    {
        entry
            .motion_descriptor()
            .entries_slice()
            .iter()
            .map(InputBindingEntry::binding)
            .collect()
    }

    fn first_motion_input_gain<A>(entry: &HeldActionBindingEntry<A>) -> Option<InputGain>
    where
        A: HeldCameraAction,
    {
        entry
            .motion_descriptor()
            .entries_slice()
            .first()
            .map(InputBindingEntry::input_gain)
    }

    #[test]
    fn keyboard_mouse_preset_builds_default_layout() -> Result<(), BindingsError> {
        let bindings = FreeCamPreset::keyboard_mouse().to_bindings()?;

        assert_eq!(bindings.translate().len(), 1);
        assert_eq!(bindings.look().len(), 1);
        assert_eq!(bindings.roll().len(), 1);
        assert_eq!(bindings.look_pitch(), FreeCamLookPitch::Normal);
        assert_eq!(
            bindings
                .slow_mode()
                .map(|slow_mode| (slow_mode.toggle_key, slow_mode.mod_keys)),
            Some((KeyCode::KeyS, ModKeys::ALT))
        );

        let [translate] = bindings.translate().entries() else {
            assert_eq!(bindings.translate().entries().len(), 1);
            return Ok(());
        };
        let keys = motion_bindings(translate);
        for key in [
            KeyCode::KeyW,
            KeyCode::KeyS,
            KeyCode::KeyA,
            KeyCode::KeyD,
            KeyCode::Space,
            KeyCode::ControlLeft,
        ] {
            assert!(keys.contains(&Binding::from(key)));
        }

        let [look] = bindings.look().entries() else {
            assert_eq!(bindings.look().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            look.engagement_descriptor().mouse_button_engagement(),
            Some((MouseButton::Right, ModKeys::empty()))
        );
        assert_eq!(look.sources(), InteractionSources::MOUSE);

        let [roll] = bindings.roll().entries() else {
            assert_eq!(bindings.roll().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(roll.sources(), InteractionSources::KEYBOARD);

        Ok(())
    }

    #[test]
    fn builder_starts_empty() -> Result<(), BindingsError> {
        let bindings = FreeCamBindings::builder().build()?;

        assert!(bindings.translate().is_empty());
        assert!(bindings.look().is_empty());
        assert!(bindings.roll().is_empty());
        assert_eq!(bindings.look_pitch(), FreeCamLookPitch::Normal);
        assert!(bindings.slow_mode().is_none());

        Ok(())
    }

    #[test]
    fn builder_uses_custom_translate_keys() -> Result<(), BindingsError> {
        let translate_keys = FreeCamTranslateKeys::default()
            .with_forward(KeyCode::ArrowUp)
            .with_backward(KeyCode::ArrowDown)
            .with_left(KeyCode::ArrowLeft)
            .with_right(KeyCode::ArrowRight)
            .with_up(KeyCode::PageUp)
            .with_down(KeyCode::PageDown);
        let bindings = FreeCamBindings::builder()
            .translate(translate_keys)
            .build()?;

        let [translate] = bindings.translate().entries() else {
            assert_eq!(bindings.translate().entries().len(), 1);
            return Ok(());
        };
        let keys = motion_bindings(translate);
        for key in [
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ] {
            assert!(keys.contains(&Binding::from(key)));
        }

        Ok(())
    }

    #[test]
    fn builder_preserves_authored_input_gain() -> Result<(), BindingsError> {
        let bindings = FreeCamBindings::builder()
            .translate(FreeCamTranslateKeys::default().with_input_gain(CUSTOM_TRANSLATE_INPUT_GAIN))
            .look(
                FreeCamMouseLook::button(MouseButton::Right)
                    .with_input_gain(CUSTOM_LOOK_INPUT_GAIN),
            )
            .roll(
                FreeCamRollBinding::from(InputBinding::bidirectional_keys(
                    KeyCode::KeyQ,
                    KeyCode::KeyE,
                ))
                .with_input_gain(CUSTOM_ROLL_INPUT_GAIN),
            )
            .build()?;

        let [translate] = bindings.translate().entries() else {
            assert_eq!(bindings.translate().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            first_motion_input_gain(translate),
            Some(InputGain(CUSTOM_TRANSLATE_INPUT_GAIN))
        );

        let [look] = bindings.look().entries() else {
            assert_eq!(bindings.look().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            first_motion_input_gain(look),
            Some(InputGain(CUSTOM_LOOK_INPUT_GAIN))
        );

        let [roll] = bindings.roll().entries() else {
            assert_eq!(bindings.roll().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            first_motion_input_gain(roll),
            Some(InputGain(CUSTOM_ROLL_INPUT_GAIN))
        );
        for engagement in roll.engagement_descriptor().entries_slice() {
            assert_eq!(engagement.input_gain(), InputGain::DEFAULT);
        }

        Ok(())
    }

    #[test]
    fn builder_preserves_look_pitch() -> Result<(), BindingsError> {
        let bindings = FreeCamBindings::builder()
            .look(FreeCamMouseLook::button(MouseButton::Right))
            .look_pitch(FreeCamLookPitch::Inverted)
            .build()?;

        assert_eq!(bindings.look_pitch(), FreeCamLookPitch::Inverted);
        assert_eq!(
            bindings
                .with_look_pitch(FreeCamLookPitch::Normal)
                .look_pitch(),
            FreeCamLookPitch::Normal
        );

        Ok(())
    }

    #[test]
    fn builder_rejects_invalid_slow_scale() {
        let bindings = FreeCamBindings::builder()
            .slow_mode(CameraSlowMode {
                toggle_key: KeyCode::KeyS,
                mod_keys:   ModKeys::ALT,
                scale:      CameraInputScalePolicy {
                    normal: 1.0,
                    slow:   f32::NAN,
                },
            })
            .build();

        assert_eq!(bindings, Err(BindingsError::InvalidScale));
    }

    #[test]
    fn without_slow_mode_clears_slow_mode() -> Result<(), BindingsError> {
        let bindings = FreeCamPreset::keyboard_mouse().to_bindings()?;
        assert!(bindings.slow_mode().is_some());
        assert!(bindings.without_slow_mode().slow_mode().is_none());

        Ok(())
    }

    #[test]
    fn gamepad_axis_binds_to_look() -> Result<(), BindingsError> {
        let bindings = FreeCamBindings::builder()
            .look(InputBinding::gamepad_axes_2d(
                GamepadAxis::RightStickX,
                GamepadAxis::RightStickY,
            ))
            .build()?;

        let [look] = bindings.look().entries() else {
            assert_eq!(bindings.look().entries().len(), 1);
            return Ok(());
        };
        assert!(look.sources().contains(InteractionSources::GAMEPAD));

        Ok(())
    }

    #[test]
    fn gamepad_buttons_bind_to_roll() -> Result<(), BindingsError> {
        let bindings = FreeCamBindings::builder()
            .roll(InputBinding::bidirectional_gamepad_buttons(
                GamepadButton::RightTrigger2,
                GamepadButton::LeftTrigger2,
            ))
            .build()?;

        let [roll] = bindings.roll().entries() else {
            assert_eq!(bindings.roll().entries().len(), 1);
            return Ok(());
        };
        assert!(roll.sources().contains(InteractionSources::GAMEPAD));

        Ok(())
    }
}
