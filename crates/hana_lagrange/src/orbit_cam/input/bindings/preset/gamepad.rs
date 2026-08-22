use bevy::prelude::GamepadAxis;
use bevy::prelude::GamepadButton;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy::prelude::Vec2;
use bevy_enhanced_input::prelude::Binding;

use super::config::OrbitCamPresetConfig;
use crate::input::ControlSpeed;
use crate::input::GamepadInputGain;
use crate::orbit_cam::input::bindings::BindingsError;
use crate::orbit_cam::input::bindings::CameraInputGamepadSelectionPolicy;
use crate::orbit_cam::input::bindings::HeldBinding;
use crate::orbit_cam::input::bindings::InputBinding;
use crate::orbit_cam::input::bindings::InputDeadZone;
use crate::orbit_cam::input::bindings::OrbitCamBindings;
use crate::orbit_cam::input::bindings::OrbitCamBindingsBuilder;
use crate::orbit_cam::input::bindings::OrbitCamInputGain;

/// Tunable gamepad preset descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Default)]
pub struct OrbitCamGamepadPreset {
    input_gain:       OrbitCamInputGain,
    orbit_scale:      f32,
    slow_orbit_scale: f32,
    pan_scale:        f32,
    slow_pan_scale:   f32,
    zoom_scale:       f32,
    slow_zoom_scale:  f32,
    stick_dead_zone:  InputDeadZone,
    home:             [Option<Binding>; 2],
}

impl OrbitCamGamepadPreset {
    const DEFAULT_ORBIT_SCALE: f32 = 1200.0;
    const DEFAULT_PAN_SCALE: f32 = 800.0;
    const DEFAULT_SLOW_ORBIT_SCALE: f32 = 120.0;
    const DEFAULT_SLOW_PAN_SCALE: f32 = 80.0;
    const DEFAULT_SLOW_ZOOM_SCALE: f32 = 0.6;
    const DEFAULT_STICK_DEAD_ZONE_LOWER: f32 = 0.18;
    const DEFAULT_STICK_DEAD_ZONE_UPPER: f32 = 1.0;
    const DEFAULT_ZOOM_SCALE: f32 = 7.0;
    const MAX_DEAD_ZONE: f32 = 1.0;
    const MIN_DEAD_ZONE: f32 = 0.0;
    const MIN_SCALE: f32 = 0.0;

    /// Starts a tuning builder from this preset.
    #[must_use]
    pub const fn customize(self) -> OrbitCamGamepadPresetBuilder {
        OrbitCamGamepadPresetBuilder { preset: self }
    }

    /// Builds the zero-config gamepad preset.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when generated descriptors fail
    /// validation.
    pub fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        <Self as OrbitCamPresetConfig>::build(self)
    }

    /// Sets source input gain for generated gamepad bindings.
    #[must_use]
    pub const fn gamepad_input_gain(mut self, input_gain: OrbitCamInputGain) -> Self {
        self.input_gain = input_gain;
        self
    }

    /// Sets the fast orbit scale.
    #[must_use]
    pub const fn orbit_scale(mut self, orbit_scale: f32) -> Self {
        self.orbit_scale = orbit_scale;
        self
    }

    /// Sets the slow orbit scale.
    #[must_use]
    pub const fn slow_orbit_scale(mut self, slow_orbit_scale: f32) -> Self {
        self.slow_orbit_scale = slow_orbit_scale;
        self
    }

    /// Sets the fast pan scale.
    #[must_use]
    pub const fn pan_scale(mut self, pan_scale: f32) -> Self {
        self.pan_scale = pan_scale;
        self
    }

    /// Sets the slow pan scale.
    #[must_use]
    pub const fn slow_pan_scale(mut self, slow_pan_scale: f32) -> Self {
        self.slow_pan_scale = slow_pan_scale;
        self
    }

    /// Sets the fast zoom scale.
    #[must_use]
    pub const fn zoom_scale(mut self, zoom_scale: f32) -> Self {
        self.zoom_scale = zoom_scale;
        self
    }

    /// Sets the slow zoom scale.
    #[must_use]
    pub const fn slow_zoom_scale(mut self, slow_zoom_scale: f32) -> Self {
        self.slow_zoom_scale = slow_zoom_scale;
        self
    }

    /// Sets the axial stick dead-zone thresholds.
    #[must_use]
    pub const fn stick_dead_zone(mut self, stick_dead_zone: InputDeadZone) -> Self {
        self.stick_dead_zone = stick_dead_zone;
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
        self.input_gain.validate()?;
        Self::validate_scale_pair(self.orbit_scale, self.slow_orbit_scale)?;
        Self::validate_scale_pair(self.pan_scale, self.slow_pan_scale)?;
        Self::validate_scale_pair(self.zoom_scale, self.slow_zoom_scale)?;
        self.validate_stick_dead_zone()
    }

    fn validate_scale_pair(fast: f32, slow: f32) -> Result<(), BindingsError> {
        if !fast.is_finite()
            || !slow.is_finite()
            || fast < Self::MIN_SCALE
            || slow < Self::MIN_SCALE
            || slow > fast
        {
            return Err(BindingsError::InvalidScale);
        }
        Ok(())
    }

    fn validate_stick_dead_zone(&self) -> Result<(), BindingsError> {
        let lower = self.stick_dead_zone.lower_threshold;
        let upper = self.stick_dead_zone.upper_threshold;
        if !lower.is_finite()
            || !upper.is_finite()
            || lower < Self::MIN_DEAD_ZONE
            || upper > Self::MAX_DEAD_ZONE
            || lower >= upper
        {
            return Err(BindingsError::InvalidDeadZone);
        }
        Ok(())
    }

    fn add_to(self, builder: OrbitCamBindingsBuilder) -> OrbitCamBindingsBuilder {
        let orbit_input_gain = self.input_gain.orbit_input_gain().value();
        let pan_input_gain = self.input_gain.pan_input_gain().value();
        let zoom_input_gain = self.input_gain.zoom_input_gain().value();
        let fast_orbit = gamepad_stick(
            GamepadAxis::RightStickX,
            GamepadAxis::RightStickY,
            self.orbit_scale,
            self.stick_dead_zone,
        );
        let slow_orbit = gamepad_stick(
            GamepadAxis::RightStickX,
            GamepadAxis::RightStickY,
            self.slow_orbit_scale,
            self.stick_dead_zone,
        );
        let fast_pan = gamepad_stick(
            GamepadAxis::LeftStickX,
            GamepadAxis::LeftStickY,
            self.pan_scale,
            self.stick_dead_zone,
        );
        let slow_pan = gamepad_stick(
            GamepadAxis::LeftStickX,
            GamepadAxis::LeftStickY,
            self.slow_pan_scale,
            self.stick_dead_zone,
        );

        let builder = builder
            .orbit(
                HeldBinding::same(fast_orbit)
                    .with_input_gain(orbit_input_gain)
                    .with_blocked_gate(GamepadButton::RightTrigger),
            )
            .orbit(
                HeldBinding::same(slow_orbit)
                    .with_input_gain(orbit_input_gain)
                    .with_required_gate(GamepadButton::RightTrigger)
                    .speed(ControlSpeed::Slow),
            )
            .pan(
                HeldBinding::same(fast_pan)
                    .with_input_gain(pan_input_gain)
                    .with_blocked_gate(GamepadButton::LeftTrigger),
            )
            .pan(
                HeldBinding::same(slow_pan)
                    .with_input_gain(pan_input_gain)
                    .with_required_gate(GamepadButton::LeftTrigger)
                    .speed(ControlSpeed::Slow),
            )
            .zoom(
                HeldBinding::same(gamepad_trigger(
                    GamepadButton::RightTrigger2,
                    self.zoom_scale,
                ))
                .with_input_gain(zoom_input_gain)
                .with_blocked_gate(GamepadButton::RightTrigger),
            )
            .zoom(
                HeldBinding::same(gamepad_trigger(
                    GamepadButton::LeftTrigger2,
                    -self.zoom_scale,
                ))
                .with_input_gain(zoom_input_gain)
                .with_blocked_gate(GamepadButton::LeftTrigger),
            )
            .zoom(
                HeldBinding::same(gamepad_trigger(
                    GamepadButton::RightTrigger2,
                    self.slow_zoom_scale,
                ))
                .with_input_gain(zoom_input_gain)
                .with_required_gate(GamepadButton::RightTrigger)
                .speed(ControlSpeed::Slow),
            )
            .zoom(
                HeldBinding::same(gamepad_trigger(
                    GamepadButton::LeftTrigger2,
                    -self.slow_zoom_scale,
                ))
                .with_input_gain(zoom_input_gain)
                .with_required_gate(GamepadButton::LeftTrigger)
                .speed(ControlSpeed::Slow),
            )
            .gamepad(CameraInputGamepadSelectionPolicy::Active);
        self.home
            .into_iter()
            .flatten()
            .fold(builder, OrbitCamBindingsBuilder::home)
    }
}

impl Default for OrbitCamGamepadPreset {
    fn default() -> Self {
        Self {
            input_gain:       OrbitCamInputGain::default(),
            orbit_scale:      Self::DEFAULT_ORBIT_SCALE,
            slow_orbit_scale: Self::DEFAULT_SLOW_ORBIT_SCALE,
            pan_scale:        Self::DEFAULT_PAN_SCALE,
            slow_pan_scale:   Self::DEFAULT_SLOW_PAN_SCALE,
            zoom_scale:       Self::DEFAULT_ZOOM_SCALE,
            slow_zoom_scale:  Self::DEFAULT_SLOW_ZOOM_SCALE,
            stick_dead_zone:  InputDeadZone::new(
                Self::DEFAULT_STICK_DEAD_ZONE_LOWER,
                Self::DEFAULT_STICK_DEAD_ZONE_UPPER,
            ),
            home:             [None; 2],
        }
    }
}

impl GamepadInputGain for OrbitCamGamepadPreset {
    type Gain = OrbitCamInputGain;

    fn gamepad_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::gamepad_input_gain(self, input_gain)
    }
}

impl OrbitCamPresetConfig for OrbitCamGamepadPreset {
    fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        self.build_into(OrbitCamBindings::builder())?.build()
    }
}

/// Fluent tuning builder for [`OrbitCamGamepadPreset`].
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct OrbitCamGamepadPresetBuilder {
    preset: OrbitCamGamepadPreset,
}

impl OrbitCamGamepadPresetBuilder {
    /// Sets the fast orbit scale.
    #[must_use]
    pub const fn orbit_scale(mut self, orbit_scale: f32) -> Self {
        self.preset.orbit_scale = orbit_scale;
        self
    }

    /// Sets the slow orbit scale.
    #[must_use]
    pub const fn slow_orbit_scale(mut self, slow_orbit_scale: f32) -> Self {
        self.preset.slow_orbit_scale = slow_orbit_scale;
        self
    }

    /// Sets the fast pan scale.
    #[must_use]
    pub const fn pan_scale(mut self, pan_scale: f32) -> Self {
        self.preset.pan_scale = pan_scale;
        self
    }

    /// Sets the slow pan scale.
    #[must_use]
    pub const fn slow_pan_scale(mut self, slow_pan_scale: f32) -> Self {
        self.preset.slow_pan_scale = slow_pan_scale;
        self
    }

    /// Sets the fast zoom scale.
    #[must_use]
    pub const fn zoom_scale(mut self, zoom_scale: f32) -> Self {
        self.preset.zoom_scale = zoom_scale;
        self
    }

    /// Sets the slow zoom scale.
    #[must_use]
    pub const fn slow_zoom_scale(mut self, slow_zoom_scale: f32) -> Self {
        self.preset.slow_zoom_scale = slow_zoom_scale;
        self
    }

    /// Sets the axial stick dead-zone thresholds.
    #[must_use]
    pub const fn stick_dead_zone(mut self, stick_dead_zone: InputDeadZone) -> Self {
        self.preset.stick_dead_zone = stick_dead_zone;
        self
    }

    /// Adds a binding that returns the camera to its home pose.
    ///
    /// No home input is bound unless this method is called. The preset holds
    /// up to two home bindings (e.g. a key plus a gamepad button); a third
    /// call replaces the second binding.
    #[must_use]
    pub fn home(mut self, home: impl Into<Binding>) -> Self {
        self.preset = self.preset.home(home);
        self
    }

    /// Sets source input gain for generated gamepad bindings.
    #[must_use]
    pub const fn gamepad_input_gain(mut self, input_gain: OrbitCamInputGain) -> Self {
        self.preset.input_gain = input_gain;
        self
    }

    /// Returns the tuned preset payload.
    #[must_use]
    pub const fn into_preset(self) -> OrbitCamGamepadPreset { self.preset }

    /// Builds tuned gamepad bindings.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when generated descriptors fail
    /// validation.
    pub fn build(self) -> Result<OrbitCamBindings, BindingsError> {
        self.preset.build_into(OrbitCamBindings::builder())?.build()
    }
}

fn gamepad_stick(
    x_axis: GamepadAxis,
    y_axis: GamepadAxis,
    scale: f32,
    dead_zone: InputDeadZone,
) -> InputBinding {
    InputBinding::gamepad_axes_2d(x_axis, y_axis)
        .with_dead_zone(dead_zone)
        .with_scale(Vec2::splat(scale))
        .with_delta_scale()
}

fn gamepad_trigger(button: GamepadButton, scale: f32) -> InputBinding {
    InputBinding::gamepad_button_axis(button, scale).with_delta_scale()
}

#[cfg(test)]
mod tests {
    use bevy::prelude::KeyCode;

    use super::*;

    #[test]
    fn default_gamepad_preset_binds_no_home() -> Result<(), BindingsError> {
        let preset = OrbitCamGamepadPreset::default();

        assert!(!preset.has_home());
        assert!(preset.build()?.home().is_empty());
        Ok(())
    }

    #[test]
    fn gamepad_preset_home_setter_rebinds_the_button() -> Result<(), BindingsError> {
        let preset = OrbitCamGamepadPreset::default().home(GamepadButton::North);

        assert!(preset.has_home());
        assert_eq!(
            preset.build()?.home().to_vec(),
            vec![Binding::from(GamepadButton::North)]
        );
        Ok(())
    }

    #[test]
    fn gamepad_preset_builder_home_setter_binds_select() -> Result<(), BindingsError> {
        let bindings = OrbitCamGamepadPreset::default()
            .customize()
            .home(GamepadButton::Select)
            .build()?;

        assert_eq!(
            bindings.home().to_vec(),
            vec![Binding::from(GamepadButton::Select)]
        );
        Ok(())
    }

    #[test]
    fn gamepad_preset_home_setter_binds_two_inputs() -> Result<(), BindingsError> {
        let preset = OrbitCamGamepadPreset::default()
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
