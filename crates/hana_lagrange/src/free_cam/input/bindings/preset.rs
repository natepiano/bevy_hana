//! Built-in `FreeCam` input presets and the look-pitch setting they carry.
//!
//! Types:
//! - [`FreeCamLookPitch`] — mouse-Y pitch direction shared by presets and validated bindings.
//! - [`FreeCamPresetKind`] — setting-insensitive identity of a built-in preset.
//! - [`FreeCamKeyboardMousePreset`] — the mouse-and-keyboard preset payload.
//! - [`FreeCamPreset`] — the built-in preset enum lowered into [`super::FreeCamBindings`].

use bevy::prelude::GamepadAxis;
use bevy::prelude::GamepadButton;
use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ModKeys;

use super::FreeCamBindings;
use super::FreeCamBindingsBuilder;
use super::FreeCamInputGain;
use super::builder::FreeCamMouseLook;
use super::builder::FreeCamTrackpadLook;
use super::builder::FreeCamTranslateKeys;
use crate::input::BindingsError;
use crate::input::CameraInputScalePolicy;
use crate::input::CameraSlowMode;
use crate::input::GamepadInputGain;
use crate::input::HeldBinding;
use crate::input::InputBinding;
use crate::input::InputDeadZone;
use crate::input::MouseInputGain;
use crate::orbit_cam::CameraInputGamepadSelectionPolicy;

/// Pitch-axis direction for `FreeCam` look input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
pub enum FreeCamLookPitch {
    /// Mouse Y is passed through unchanged.
    #[default]
    Normal,
    /// Mouse Y is negated before it reaches the `FreeCam` look channel.
    Inverted,
}

impl FreeCamLookPitch {
    /// Returns the opposite pitch-axis direction.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Normal => Self::Inverted,
            Self::Inverted => Self::Normal,
        }
    }
}

/// Setting-insensitive identity for a built-in `FreeCam` input preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
#[non_exhaustive]
pub enum FreeCamPresetKind {
    /// Mouse look plus keyboard translate and roll controls.
    #[default]
    KeyboardMouse,
    /// Twin-stick gamepad controls.
    Gamepad,
}

impl FreeCamPresetKind {
    /// Returns the preset kind's display name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::KeyboardMouse => "Mouse and keyboard",
            Self::Gamepad => "Gamepad",
        }
    }
}

/// How the mouse-and-keyboard preset exposes a slow-mode toggle.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
enum SlowModeToggle {
    Bound(KeyCode),
    Unbound,
}

/// How the mouse-and-keyboard preset exposes trackpad look.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
enum TrackpadLookBinding {
    BoundWithDefaultGain,
    BoundWithInputGain(f32),
    Unbound,
}

/// Home bindings retained by the mouse-and-keyboard preset.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
enum HomeBindings {
    Unbound,
    One(Binding),
    Two(Binding, Binding),
}

impl HomeBindings {
    const fn has_home(self) -> bool { !matches!(self, Self::Unbound) }

    const fn with_home(self, binding: Binding) -> Self {
        match self {
            Self::Unbound => Self::One(binding),
            Self::One(first) | Self::Two(first, _) => Self::Two(first, binding),
        }
    }
}

/// Mouse-and-keyboard `FreeCam` preset payload.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Default)]
pub struct FreeCamKeyboardMousePreset {
    toggle_key:      SlowModeToggle,
    toggle_mod_keys: ModKeys,
    scale:           f32,
    input_gain:      FreeCamInputGain,
    home:            HomeBindings,
    look_pitch:      FreeCamLookPitch,
    trackpad_look:   TrackpadLookBinding,
}

impl FreeCamKeyboardMousePreset {
    const DEFAULT_SLOW_SCALE: f32 = 0.25;
    const NORMAL_SCALE: f32 = 1.0;

    /// Converts this preset into validated custom bindings.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] if the preset construction violates a binding
    /// invariant.
    pub fn to_bindings(self) -> Result<FreeCamBindings, BindingsError> {
        self.input_gain.validate()?;
        let builder = FreeCamBindings::builder()
            .translate(
                FreeCamTranslateKeys::default()
                    .with_input_gain(self.input_gain.translate_input_gain().value()),
            )
            .look(
                FreeCamMouseLook::button(MouseButton::Right)
                    .with_input_gain(self.input_gain.look_input_gain().value()),
            )
            .roll(
                HeldBinding::same(InputBinding::bidirectional_keys(
                    KeyCode::KeyQ,
                    KeyCode::KeyE,
                ))
                .with_input_gain(self.input_gain.roll_input_gain().value()),
            )
            .look_pitch(self.look_pitch);
        let builder = match self.trackpad_look {
            TrackpadLookBinding::BoundWithDefaultGain => {
                builder.look(FreeCamTrackpadLook::default())
            },
            TrackpadLookBinding::BoundWithInputGain(input_gain) => {
                builder.look(FreeCamTrackpadLook::default().with_input_gain(input_gain))
            },
            TrackpadLookBinding::Unbound => builder,
        };
        let builder = match self.home {
            HomeBindings::Unbound => builder,
            HomeBindings::One(first) => builder.home(first),
            HomeBindings::Two(first, second) => builder.home(first).home(second),
        };
        match self.toggle_key {
            SlowModeToggle::Bound(toggle_key) => builder
                .slow_mode(CameraSlowMode {
                    toggle_key,
                    mod_keys: self.toggle_mod_keys,
                    scale: CameraInputScalePolicy {
                        normal: Self::NORMAL_SCALE,
                        slow:   self.scale,
                    },
                })
                .build(),
            SlowModeToggle::Unbound => builder.without_slow_mode().build(),
        }
    }

    /// Sets the key that toggles slow mode on or off for this camera.
    #[must_use]
    pub const fn with_slow_toggle_key(mut self, slow_toggle_key: KeyCode) -> Self {
        self.toggle_key = SlowModeToggle::Bound(slow_toggle_key);
        self
    }

    /// Removes the slow-mode toggle from this camera preset.
    #[must_use]
    pub const fn without_slow_toggle(mut self) -> Self {
        self.toggle_key = SlowModeToggle::Unbound;
        self
    }

    /// Sets the modifier keys held with the toggle key to fire the slow-mode toggle.
    #[must_use]
    pub const fn slow_toggle_mod_keys(mut self, slow_toggle_mod_keys: ModKeys) -> Self {
        self.toggle_mod_keys = slow_toggle_mod_keys;
        self
    }

    /// Sets the scale applied to all inputs when slow mode is active.
    #[must_use]
    pub const fn slow_scale(mut self, slow_scale: f32) -> Self {
        self.scale = slow_scale;
        self
    }

    /// Sets source-side input gain for the keyboard/mouse preset.
    #[must_use]
    pub const fn mouse_input_gain(mut self, input_gain: FreeCamInputGain) -> Self {
        self.input_gain = input_gain;
        self
    }

    /// Removes smooth-scroll trackpad look from this camera preset.
    #[must_use]
    pub const fn without_trackpad_look(mut self) -> Self {
        self.trackpad_look = TrackpadLookBinding::Unbound;
        self
    }

    /// Sets source-side input gain for smooth-scroll trackpad look.
    #[must_use]
    pub const fn trackpad_input_gain(mut self, input_gain: f32) -> Self {
        self.trackpad_look = TrackpadLookBinding::BoundWithInputGain(input_gain);
        self
    }

    /// Adds a binding that returns the camera to its home pose.
    ///
    /// No home input is bound unless this method is called. The preset holds
    /// up to two home bindings (e.g. a key plus a gamepad button); a third
    /// call replaces the second binding.
    #[must_use]
    pub fn with_home(mut self, home: impl Into<Binding>) -> Self {
        self.home = self.home.with_home(home.into());
        self
    }

    /// Returns whether this preset binds home input.
    #[must_use]
    pub const fn has_home(&self) -> bool { self.home.has_home() }

    /// Sets whether mouse Y is passed through or inverted for look input.
    #[must_use]
    pub const fn with_look_pitch(mut self, look_pitch: FreeCamLookPitch) -> Self {
        self.look_pitch = look_pitch;
        self
    }

    /// Returns the pitch-axis direction for look input.
    #[must_use]
    pub const fn look_pitch(self) -> FreeCamLookPitch { self.look_pitch }
}

impl Default for FreeCamKeyboardMousePreset {
    fn default() -> Self {
        Self {
            toggle_key:      SlowModeToggle::Bound(KeyCode::KeyS),
            toggle_mod_keys: ModKeys::ALT,
            scale:           Self::DEFAULT_SLOW_SCALE,
            input_gain:      FreeCamInputGain::new(),
            home:            HomeBindings::Unbound,
            look_pitch:      FreeCamLookPitch::Normal,
            trackpad_look:   TrackpadLookBinding::BoundWithDefaultGain,
        }
    }
}

impl MouseInputGain for FreeCamKeyboardMousePreset {
    type Gain = FreeCamInputGain;

    fn mouse_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::mouse_input_gain(self, input_gain)
    }
}

/// Built-in `FreeCam` input presets.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(Default)]
#[non_exhaustive]
pub enum FreeCamPreset {
    /// Mouse look plus keyboard translate and roll controls.
    KeyboardMouse(FreeCamKeyboardMousePreset),
    /// Twin-stick gamepad controls.
    Gamepad(FreeCamGamepadPreset),
}

impl FreeCamPreset {
    /// Builds the mouse-and-keyboard input preset.
    #[must_use]
    pub const fn keyboard_mouse() -> Self {
        Self::KeyboardMouse(FreeCamKeyboardMousePreset {
            toggle_key:      SlowModeToggle::Bound(KeyCode::KeyS),
            toggle_mod_keys: ModKeys::ALT,
            scale:           FreeCamKeyboardMousePreset::DEFAULT_SLOW_SCALE,
            input_gain:      FreeCamInputGain::new(),
            home:            HomeBindings::Unbound,
            look_pitch:      FreeCamLookPitch::Normal,
            trackpad_look:   TrackpadLookBinding::BoundWithDefaultGain,
        })
    }

    /// Builds the gamepad input preset: left stick moves, right stick looks.
    #[must_use]
    pub fn gamepad() -> Self { Self::Gamepad(FreeCamGamepadPreset::default()) }

    /// Builds the southpaw gamepad input preset: right stick moves, left stick looks.
    #[must_use]
    pub fn gamepad_southpaw() -> Self {
        Self::Gamepad(FreeCamGamepadPreset::default().with_layout(FreeCamGamepadLayout::Southpaw))
    }

    /// Returns the preset's setting-insensitive identity.
    #[must_use]
    pub const fn kind(&self) -> FreeCamPresetKind {
        match self {
            Self::KeyboardMouse(_) => FreeCamPresetKind::KeyboardMouse,
            Self::Gamepad(_) => FreeCamPresetKind::Gamepad,
        }
    }

    /// Returns the preset's display name, including the gamepad stick layout so
    /// the standard and southpaw gamepad layouts read differently in a control
    /// panel. Use this for display; [`Self::kind`] is the setting-insensitive
    /// identity for matching and equality.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::KeyboardMouse(_) => FreeCamPresetKind::KeyboardMouse.name(),
            Self::Gamepad(preset) => match preset.layout {
                FreeCamGamepadLayout::Standard => FreeCamPresetKind::Gamepad.name(),
                FreeCamGamepadLayout::Southpaw => "Gamepad, sticks swapped",
            },
        }
    }

    /// Adds a binding that returns the camera to its home pose.
    ///
    /// No home input is bound unless this method is called. Each preset holds
    /// up to two home bindings (e.g. a key plus a gamepad button); a third
    /// call replaces the second binding.
    #[must_use]
    pub fn with_home(self, home: impl Into<Binding>) -> Self {
        let home = home.into();
        match self {
            Self::KeyboardMouse(preset) => preset.with_home(home).into(),
            Self::Gamepad(preset) => preset.with_home(home).into(),
        }
    }

    /// Returns whether this preset binds home input.
    #[must_use]
    pub const fn has_home(&self) -> bool {
        match self {
            Self::KeyboardMouse(preset) => preset.has_home(),
            Self::Gamepad(preset) => preset.has_home(),
        }
    }

    /// Converts this preset into validated custom bindings.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] if the preset construction violates a binding
    /// invariant.
    pub fn to_bindings(&self) -> Result<FreeCamBindings, BindingsError> {
        match self {
            Self::KeyboardMouse(preset) => preset.to_bindings(),
            Self::Gamepad(preset) => preset.to_bindings(),
        }
    }
}

impl Default for FreeCamPreset {
    fn default() -> Self { Self::keyboard_mouse() }
}

impl From<FreeCamKeyboardMousePreset> for FreeCamPreset {
    fn from(preset: FreeCamKeyboardMousePreset) -> Self { Self::KeyboardMouse(preset) }
}

impl From<FreeCamGamepadPreset> for FreeCamPreset {
    fn from(preset: FreeCamGamepadPreset) -> Self { Self::Gamepad(preset) }
}

/// Stick assignment for a [`FreeCamGamepadPreset`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
#[non_exhaustive]
pub enum FreeCamGamepadLayout {
    /// Left stick moves, right stick looks.
    #[default]
    Standard,
    /// Right stick moves, left stick looks.
    Southpaw,
}

impl FreeCamGamepadLayout {
    const fn move_axes(self) -> (GamepadAxis, GamepadAxis) {
        match self {
            Self::Standard => (GamepadAxis::LeftStickX, GamepadAxis::LeftStickY),
            Self::Southpaw => (GamepadAxis::RightStickX, GamepadAxis::RightStickY),
        }
    }

    const fn look_axes(self) -> (GamepadAxis, GamepadAxis) {
        match self {
            Self::Standard => (GamepadAxis::RightStickX, GamepadAxis::RightStickY),
            Self::Southpaw => (GamepadAxis::LeftStickX, GamepadAxis::LeftStickY),
        }
    }
}

/// Twin-stick gamepad `FreeCam` preset.
///
/// One stick moves and the other looks (see [`FreeCamGamepadLayout`]); the triggers raise and
/// lower, and the bumpers roll. The left stick click boosts move speed while held.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Default)]
pub struct FreeCamGamepadPreset {
    layout:          FreeCamGamepadLayout,
    input_gain:      FreeCamInputGain,
    move_scale:      f32,
    boost_scale:     f32,
    look_scale:      f32,
    roll_scale:      f32,
    stick_dead_zone: InputDeadZone,
    home:            [Option<Binding>; 2],
    look_pitch:      FreeCamLookPitch,
}

impl FreeCamGamepadPreset {
    const DEFAULT_BOOST_SCALE: f32 = 4.0;
    const DEFAULT_LOOK_SCALE: f32 = 800.0;
    const DEFAULT_MOVE_SCALE: f32 = 1.0;
    const DEFAULT_ROLL_SCALE: f32 = 1.0;
    const DEFAULT_STICK_DEAD_ZONE_LOWER: f32 = 0.18;
    const DEFAULT_STICK_DEAD_ZONE_UPPER: f32 = 1.0;

    const BOOST_BUTTON: GamepadButton = GamepadButton::LeftThumb;
    const DOWN_BUTTON: GamepadButton = GamepadButton::LeftTrigger2;
    const ROLL_LEFT_BUTTON: GamepadButton = GamepadButton::LeftTrigger;
    const ROLL_RIGHT_BUTTON: GamepadButton = GamepadButton::RightTrigger;
    const UP_BUTTON: GamepadButton = GamepadButton::RightTrigger2;

    /// Sets the stick layout.
    #[must_use]
    pub const fn with_layout(mut self, layout: FreeCamGamepadLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Sets source-side input gain for the gamepad preset.
    #[must_use]
    pub const fn gamepad_input_gain(mut self, input_gain: FreeCamInputGain) -> Self {
        self.input_gain = input_gain;
        self
    }

    /// Sets the move speed applied to the move stick.
    #[must_use]
    pub const fn with_move_scale(mut self, move_scale: f32) -> Self {
        self.move_scale = move_scale;
        self
    }

    /// Sets the speed multiplier applied to move input while the boost button is held.
    #[must_use]
    pub const fn with_boost_scale(mut self, boost_scale: f32) -> Self {
        self.boost_scale = boost_scale;
        self
    }

    /// Sets the look rate applied to the look stick.
    #[must_use]
    pub const fn with_look_scale(mut self, look_scale: f32) -> Self {
        self.look_scale = look_scale;
        self
    }

    /// Sets the roll speed applied to the roll buttons.
    #[must_use]
    pub const fn with_roll_scale(mut self, roll_scale: f32) -> Self {
        self.roll_scale = roll_scale;
        self
    }

    /// Sets the dead-zone thresholds applied to both sticks.
    #[must_use]
    pub const fn with_stick_dead_zone(mut self, stick_dead_zone: InputDeadZone) -> Self {
        self.stick_dead_zone = stick_dead_zone;
        self
    }

    /// Adds a binding that returns the camera to its home pose.
    ///
    /// No home input is bound unless this method is called. The preset holds
    /// up to two home bindings (e.g. a key plus a gamepad button); a third
    /// call replaces the second binding.
    #[must_use]
    pub fn with_home(mut self, home: impl Into<Binding>) -> Self {
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

    /// Sets whether stick Y is passed through or inverted for look input.
    #[must_use]
    pub const fn with_look_pitch(mut self, look_pitch: FreeCamLookPitch) -> Self {
        self.look_pitch = look_pitch;
        self
    }

    /// Returns the pitch-axis direction for look input.
    #[must_use]
    pub const fn look_pitch(self) -> FreeCamLookPitch { self.look_pitch }

    /// Converts this preset into validated custom bindings.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] if the preset construction violates a binding
    /// invariant.
    pub fn to_bindings(self) -> Result<FreeCamBindings, BindingsError> {
        self.input_gain.validate()?;
        let (move_x, move_y) = self.layout.move_axes();
        let (look_x, look_y) = self.layout.look_axes();
        let dead_zone = self.stick_dead_zone;
        let translate_input_gain = self.input_gain.translate_input_gain().value();
        let look_input_gain = self.input_gain.look_input_gain().value();
        let roll_input_gain = self.input_gain.roll_input_gain().value();
        let move_binding = |scale: f32| {
            InputBinding::gamepad_vec3(move_x, move_y, Self::UP_BUTTON, Self::DOWN_BUTTON)
                .with_dead_zone(dead_zone)
                .with_scale(scale)
        };

        let builder = FreeCamBindings::builder()
            .translate(
                HeldBinding::same(move_binding(self.move_scale))
                    .with_input_gain(translate_input_gain)
                    .with_blocked_gate(Self::BOOST_BUTTON),
            )
            .translate(
                HeldBinding::same(move_binding(self.move_scale * self.boost_scale))
                    .with_input_gain(translate_input_gain)
                    .with_required_gate(Self::BOOST_BUTTON),
            )
            .look(
                HeldBinding::same(
                    InputBinding::gamepad_axes_2d(look_x, look_y)
                        .with_dead_zone(dead_zone)
                        .with_scale(self.look_scale)
                        .with_delta_scale(),
                )
                .with_input_gain(look_input_gain),
            )
            .roll(
                HeldBinding::same(
                    InputBinding::bidirectional_gamepad_buttons(
                        Self::ROLL_RIGHT_BUTTON,
                        Self::ROLL_LEFT_BUTTON,
                    )
                    .with_scale(self.roll_scale),
                )
                .with_input_gain(roll_input_gain),
            )
            .gamepad(CameraInputGamepadSelectionPolicy::Active)
            .look_pitch(self.look_pitch);
        self.home
            .into_iter()
            .flatten()
            .fold(builder, FreeCamBindingsBuilder::home)
            .build()
    }
}

impl Default for FreeCamGamepadPreset {
    fn default() -> Self {
        Self {
            layout:          FreeCamGamepadLayout::Standard,
            input_gain:      FreeCamInputGain::new(),
            move_scale:      Self::DEFAULT_MOVE_SCALE,
            boost_scale:     Self::DEFAULT_BOOST_SCALE,
            look_scale:      Self::DEFAULT_LOOK_SCALE,
            roll_scale:      Self::DEFAULT_ROLL_SCALE,
            stick_dead_zone: InputDeadZone::new(
                Self::DEFAULT_STICK_DEAD_ZONE_LOWER,
                Self::DEFAULT_STICK_DEAD_ZONE_UPPER,
            ),
            home:            [None; 2],
            look_pitch:      FreeCamLookPitch::Normal,
        }
    }
}

impl GamepadInputGain for FreeCamGamepadPreset {
    type Gain = FreeCamInputGain;

    fn gamepad_input_gain(self, input_gain: Self::Gain) -> Self {
        Self::gamepad_input_gain(self, input_gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::HeldActionBindingEntry;
    use crate::input::HeldCameraAction;
    use crate::input::InputBindingEntry;
    use crate::input::InputGain;
    use crate::input::InteractionSources;

    const GAMEPAD_LOOK_INPUT_GAIN: f32 = 0.5;
    const GAMEPAD_ROLL_INPUT_GAIN: f32 = 0.25;
    const GAMEPAD_TRANSLATE_INPUT_GAIN: f32 = 0.75;
    const INVALID_INPUT_GAIN: f32 = -0.01;
    const MOUSE_LOOK_INPUT_GAIN: f32 = 0.5;
    const MOUSE_ROLL_INPUT_GAIN: f32 = 0.25;
    const MOUSE_TRANSLATE_INPUT_GAIN: f32 = 0.75;
    const TRACKPAD_LOOK_INPUT_GAIN: f32 = 0.625;

    fn first_move_axis(bindings: &FreeCamBindings) -> Binding {
        bindings.translate().entries()[0]
            .motion_descriptor()
            .entries_slice()[0]
            .binding()
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

    fn first_motion_install_scale<A>(entry: &HeldActionBindingEntry<A>) -> Option<f32>
    where
        A: HeldCameraAction,
    {
        entry
            .motion_descriptor()
            .entries_slice()
            .first()
            .and_then(|entry| entry.install_modifiers().scale())
    }

    #[test]
    fn keyboard_mouse_preset_reports_identity() {
        let preset = FreeCamPreset::keyboard_mouse();

        assert_eq!(preset.kind(), FreeCamPresetKind::KeyboardMouse);
        assert_eq!(preset.name(), "Mouse and keyboard");
    }

    #[test]
    fn preset_names_describe_both_kinds_and_southpaw_layout() {
        assert_eq!(
            FreeCamPresetKind::KeyboardMouse.name(),
            "Mouse and keyboard"
        );
        assert_eq!(FreeCamPresetKind::Gamepad.name(), "Gamepad");
        assert_eq!(
            FreeCamPreset::gamepad_southpaw().name(),
            "Gamepad, sticks swapped"
        );
    }

    #[test]
    fn without_slow_toggle_builds_no_slow_mode() -> Result<(), BindingsError> {
        let bindings = FreeCamKeyboardMousePreset::default()
            .without_slow_toggle()
            .to_bindings()?;
        assert!(bindings.slow_mode().is_none());
        Ok(())
    }

    #[test]
    fn with_slow_toggle_key_rebinds_slow_mode() -> Result<(), BindingsError> {
        let bindings = FreeCamKeyboardMousePreset::default()
            .without_slow_toggle()
            .with_slow_toggle_key(KeyCode::KeyT)
            .to_bindings()?;
        assert_eq!(
            bindings.slow_mode().map(|mode| mode.toggle_key),
            Some(KeyCode::KeyT)
        );
        Ok(())
    }

    #[test]
    fn keyboard_mouse_preset_preserves_look_pitch() {
        let preset =
            FreeCamKeyboardMousePreset::default().with_look_pitch(FreeCamLookPitch::Inverted);

        assert_eq!(
            preset.to_bindings().map(|bindings| bindings.look_pitch()),
            Ok(FreeCamLookPitch::Inverted)
        );
    }

    #[test]
    fn keyboard_mouse_preset_binds_trackpad_look_by_default() -> Result<(), BindingsError> {
        let bindings = FreeCamKeyboardMousePreset::default().to_bindings()?;

        assert_eq!(bindings.trackpad_look.len(), 1);
        assert_eq!(
            bindings.trackpad_look[0].input_gain(),
            FreeCamTrackpadLook::default().input_gain()
        );
        Ok(())
    }

    #[test]
    fn keyboard_mouse_preset_can_remove_trackpad_look() -> Result<(), BindingsError> {
        let bindings = FreeCamKeyboardMousePreset::default()
            .without_trackpad_look()
            .to_bindings()?;

        assert_eq!(bindings.trackpad_look.len(), 0);
        Ok(())
    }

    #[test]
    fn keyboard_mouse_preset_sets_trackpad_look_input_gain() -> Result<(), BindingsError> {
        let bindings = FreeCamKeyboardMousePreset::default()
            .trackpad_input_gain(TRACKPAD_LOOK_INPUT_GAIN)
            .to_bindings()?;

        assert_eq!(bindings.trackpad_look.len(), 1);
        assert_eq!(
            bindings.trackpad_look[0].input_gain(),
            InputGain(TRACKPAD_LOOK_INPUT_GAIN)
        );
        Ok(())
    }

    #[test]
    fn keyboard_mouse_source_input_gain_scales_bindings() -> Result<(), BindingsError> {
        let input_gain = FreeCamInputGain::new()
            .translate(MOUSE_TRANSLATE_INPUT_GAIN)
            .look(MOUSE_LOOK_INPUT_GAIN)
            .roll(MOUSE_ROLL_INPUT_GAIN);
        let bindings = FreeCamKeyboardMousePreset::default()
            .mouse_input_gain(input_gain)
            .to_bindings()?;

        let [translate] = bindings.translate().entries() else {
            assert_eq!(bindings.translate().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            first_motion_input_gain(translate),
            Some(InputGain(MOUSE_TRANSLATE_INPUT_GAIN))
        );
        assert_eq!(
            first_motion_install_scale(translate),
            Some(MOUSE_TRANSLATE_INPUT_GAIN)
        );

        let [look] = bindings.look().entries() else {
            assert_eq!(bindings.look().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            first_motion_input_gain(look),
            Some(InputGain(MOUSE_LOOK_INPUT_GAIN))
        );
        assert_eq!(
            first_motion_install_scale(look),
            Some(MOUSE_LOOK_INPUT_GAIN)
        );

        let [roll] = bindings.roll().entries() else {
            assert_eq!(bindings.roll().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            first_motion_input_gain(roll),
            Some(InputGain(MOUSE_ROLL_INPUT_GAIN))
        );
        assert_eq!(
            first_motion_install_scale(roll),
            Some(MOUSE_ROLL_INPUT_GAIN)
        );

        Ok(())
    }

    #[test]
    fn keyboard_mouse_preset_binds_no_home_by_default() {
        let preset = FreeCamKeyboardMousePreset::default();

        assert!(!preset.has_home());
        assert_eq!(
            preset
                .to_bindings()
                .map(|bindings| bindings.home().to_vec()),
            Ok(Vec::new())
        );
    }

    #[test]
    fn keyboard_mouse_preset_with_home_rebinds_the_key() {
        let preset = FreeCamKeyboardMousePreset::default().with_home(KeyCode::KeyR);

        assert!(preset.has_home());
        assert_eq!(
            preset
                .to_bindings()
                .map(|bindings| bindings.home().to_vec()),
            Ok(vec![Binding::from(KeyCode::KeyR)])
        );
    }

    #[test]
    fn keyboard_mouse_preset_with_home_twice_binds_both_inputs() {
        let preset = FreeCamKeyboardMousePreset::default()
            .with_home(KeyCode::KeyH)
            .with_home(GamepadButton::Select);

        assert_eq!(
            preset
                .to_bindings()
                .map(|bindings| bindings.home().to_vec()),
            Ok(vec![
                Binding::from(KeyCode::KeyH),
                Binding::from(GamepadButton::Select)
            ])
        );
    }

    #[test]
    fn keyboard_mouse_preset_third_home_replaces_the_second() {
        let preset = FreeCamKeyboardMousePreset::default()
            .with_home(KeyCode::KeyH)
            .with_home(GamepadButton::Select)
            .with_home(GamepadButton::North);

        assert_eq!(
            preset
                .to_bindings()
                .map(|bindings| bindings.home().to_vec()),
            Ok(vec![
                Binding::from(KeyCode::KeyH),
                Binding::from(GamepadButton::North)
            ])
        );
    }

    #[test]
    fn gamepad_preset_reports_identity() {
        let preset = FreeCamPreset::gamepad();

        assert_eq!(preset.kind(), FreeCamPresetKind::Gamepad);
        assert_eq!(preset.name(), "Gamepad");
    }

    #[test]
    fn southpaw_shares_gamepad_identity_but_has_distinct_name() {
        let preset = FreeCamPreset::gamepad_southpaw();

        assert_eq!(preset.kind(), FreeCamPresetKind::Gamepad);
        assert_eq!(preset.name(), "Gamepad, sticks swapped");
    }

    #[test]
    fn gamepad_preset_builds_twin_stick_layout() -> Result<(), BindingsError> {
        let bindings = FreeCamPreset::gamepad().to_bindings()?;

        // A normal move binding plus a boost-gated move binding.
        assert_eq!(bindings.translate().len(), 2);
        assert_eq!(bindings.look().len(), 1);
        assert_eq!(bindings.roll().len(), 1);
        assert!(bindings.home().is_empty());
        assert_eq!(
            bindings.gamepad(),
            CameraInputGamepadSelectionPolicy::Active
        );

        assert!(
            bindings
                .translate()
                .entries()
                .iter()
                .all(|entry| entry.sources().contains(InteractionSources::GAMEPAD))
        );
        assert!(
            bindings
                .look()
                .entries()
                .iter()
                .all(|entry| entry.sources().contains(InteractionSources::GAMEPAD))
        );

        Ok(())
    }

    #[test]
    fn gamepad_source_input_gain_scales_binding_modifiers() -> Result<(), BindingsError> {
        let default = FreeCamGamepadPreset::default().to_bindings()?;
        let input_gain = FreeCamInputGain::new()
            .translate(GAMEPAD_TRANSLATE_INPUT_GAIN)
            .look(GAMEPAD_LOOK_INPUT_GAIN)
            .roll(GAMEPAD_ROLL_INPUT_GAIN);
        let tuned = FreeCamGamepadPreset::default()
            .gamepad_input_gain(input_gain)
            .to_bindings()?;

        let [default_move, default_boost] = default.translate().entries() else {
            assert_eq!(default.translate().entries().len(), 2);
            return Ok(());
        };
        let [tuned_move, tuned_boost] = tuned.translate().entries() else {
            assert_eq!(tuned.translate().entries().len(), 2);
            return Ok(());
        };
        for (tuned, default) in [(tuned_move, default_move), (tuned_boost, default_boost)] {
            assert_eq!(
                first_motion_install_scale(tuned),
                first_motion_install_scale(default)
                    .map(|scale| scale * GAMEPAD_TRANSLATE_INPUT_GAIN)
            );
        }

        let [default_look] = default.look().entries() else {
            assert_eq!(default.look().entries().len(), 1);
            return Ok(());
        };
        let [tuned_look] = tuned.look().entries() else {
            assert_eq!(tuned.look().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            first_motion_install_scale(tuned_look),
            first_motion_install_scale(default_look).map(|scale| scale * GAMEPAD_LOOK_INPUT_GAIN)
        );

        let [default_roll] = default.roll().entries() else {
            assert_eq!(default.roll().entries().len(), 1);
            return Ok(());
        };
        let [tuned_roll] = tuned.roll().entries() else {
            assert_eq!(tuned.roll().entries().len(), 1);
            return Ok(());
        };
        assert_eq!(
            first_motion_install_scale(tuned_roll),
            first_motion_install_scale(default_roll).map(|scale| scale * GAMEPAD_ROLL_INPUT_GAIN)
        );

        Ok(())
    }

    #[test]
    fn source_input_gain_validates_before_bindings_build() {
        assert_eq!(
            FreeCamKeyboardMousePreset::default()
                .mouse_input_gain(FreeCamInputGain::uniform(INVALID_INPUT_GAIN))
                .to_bindings(),
            Err(BindingsError::InvalidScale)
        );
        assert_eq!(
            FreeCamGamepadPreset::default()
                .gamepad_input_gain(FreeCamInputGain::uniform(INVALID_INPUT_GAIN))
                .to_bindings(),
            Err(BindingsError::InvalidScale)
        );
    }

    #[test]
    fn gamepad_preset_with_home_binds_select() -> Result<(), BindingsError> {
        let preset = FreeCamGamepadPreset::default().with_home(GamepadButton::Select);

        assert!(preset.has_home());
        assert_eq!(
            preset.to_bindings()?.home().to_vec(),
            vec![Binding::from(GamepadButton::Select)]
        );
        Ok(())
    }

    #[test]
    fn gamepad_preset_with_home_twice_binds_key_and_button() -> Result<(), BindingsError> {
        let preset = FreeCamGamepadPreset::default()
            .with_home(KeyCode::KeyH)
            .with_home(GamepadButton::Select);

        assert_eq!(
            preset.to_bindings()?.home().to_vec(),
            vec![
                Binding::from(KeyCode::KeyH),
                Binding::from(GamepadButton::Select)
            ]
        );
        Ok(())
    }

    fn assert_preset_home_round_trip(preset: FreeCamPreset) -> Result<(), BindingsError> {
        assert!(!preset.has_home());

        let preset = preset.with_home(KeyCode::KeyH);

        assert!(preset.has_home());
        assert_eq!(
            preset.to_bindings()?.home().to_vec(),
            vec![Binding::from(KeyCode::KeyH)]
        );
        Ok(())
    }

    #[test]
    fn enum_with_home_dispatch_round_trips_for_every_variant() -> Result<(), BindingsError> {
        assert_preset_home_round_trip(FreeCamPreset::keyboard_mouse())?;
        assert_preset_home_round_trip(FreeCamPreset::gamepad())?;
        Ok(())
    }

    #[test]
    fn gamepad_tuning_setters_update_binding_modifiers() -> Result<(), BindingsError> {
        const MOVE_SCALE: f32 = 2.5;
        const ROLL_SCALE: f32 = 0.5;
        const STICK_DEAD_ZONE: InputDeadZone = InputDeadZone::new(0.31, 0.92);

        let bindings = FreeCamGamepadPreset::default()
            .with_move_scale(MOVE_SCALE)
            .with_roll_scale(ROLL_SCALE)
            .with_stick_dead_zone(STICK_DEAD_ZONE)
            .to_bindings()?;

        let [move_binding, ..] = bindings.translate().entries() else {
            assert_eq!(bindings.translate().entries().len(), 2);
            return Ok(());
        };
        assert!(
            move_binding
                .motion_descriptor()
                .entries_slice()
                .iter()
                .all(|entry| entry.modifiers().scale() == Some(MOVE_SCALE))
        );
        assert!(
            move_binding
                .motion_descriptor()
                .entries_slice()
                .iter()
                .all(|entry| entry.modifiers().dead_zone() == Some(STICK_DEAD_ZONE))
        );

        let [look_binding] = bindings.look().entries() else {
            assert_eq!(bindings.look().entries().len(), 1);
            return Ok(());
        };
        assert!(
            look_binding
                .motion_descriptor()
                .entries_slice()
                .iter()
                .all(|entry| entry.modifiers().dead_zone() == Some(STICK_DEAD_ZONE))
        );

        let [roll_binding] = bindings.roll().entries() else {
            assert_eq!(bindings.roll().entries().len(), 1);
            return Ok(());
        };
        assert!(
            roll_binding
                .motion_descriptor()
                .entries_slice()
                .iter()
                .all(|entry| entry.modifiers().scale() == Some(ROLL_SCALE))
        );

        Ok(())
    }

    #[test]
    fn gamepad_layout_swaps_move_stick() -> Result<(), BindingsError> {
        let standard = FreeCamPreset::gamepad().to_bindings()?;
        let southpaw = FreeCamPreset::gamepad_southpaw().to_bindings()?;

        assert_eq!(
            first_move_axis(&standard),
            Binding::GamepadAxis(GamepadAxis::LeftStickX)
        );
        assert_eq!(
            first_move_axis(&southpaw),
            Binding::GamepadAxis(GamepadAxis::RightStickX)
        );

        Ok(())
    }
}
