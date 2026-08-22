use bevy::prelude::Reflect;
use bevy::prelude::ReflectDefault;
use bevy_enhanced_input::prelude::Binding;

use super::OrbitCamBlenderLikeKeyboardPreset;
use super::OrbitCamBlenderLikePreset;
use super::OrbitCamGamepadPreset;
use super::OrbitCamKeyboardPreset;
use super::OrbitCamSimpleMouseKeyboardPreset;
use super::OrbitCamSimpleMousePreset;
use crate::orbit_cam::input::bindings::BindingsError;
use crate::orbit_cam::input::bindings::OrbitCamBindings;

/// Setting-insensitive identity for a built-in orbit-camera input preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
#[non_exhaustive]
pub enum OrbitCamPresetKind {
    /// Mouse-oriented default controls.
    #[default]
    SimpleMouse,
    /// Editor-oriented controls modeled after Blender navigation.
    BlenderLike,
    /// Keyboard-only camera controls.
    Keyboard,
    /// Simple mouse controls plus keyboard camera controls.
    SimpleMouseKeyboard,
    /// Blender-like pointer controls plus keyboard camera controls.
    BlenderLikeKeyboard,
    /// Gamepad camera controls.
    Gamepad,
}

impl OrbitCamPresetKind {
    /// Returns the preset kind's display name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::SimpleMouse => "SimpleMouse",
            Self::BlenderLike => "BlenderLike",
            Self::Keyboard => "Keyboard",
            Self::SimpleMouseKeyboard => "SimpleMouseKeyboard",
            Self::BlenderLikeKeyboard => "BlenderLikeKeyboard",
            Self::Gamepad => "Gamepad",
        }
    }
}

/// Built-in orbit-camera input presets.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(Default)]
#[non_exhaustive]
pub enum OrbitCamPreset {
    /// Mouse-oriented default controls.
    SimpleMouse(OrbitCamSimpleMousePreset),
    /// Editor-oriented controls modeled after Blender navigation.
    BlenderLike(OrbitCamBlenderLikePreset),
    /// Keyboard-only camera controls.
    Keyboard(OrbitCamKeyboardPreset),
    /// Simple mouse controls plus keyboard camera controls.
    SimpleMouseKeyboard(OrbitCamSimpleMouseKeyboardPreset),
    /// Blender-like pointer controls plus keyboard camera controls.
    BlenderLikeKeyboard(OrbitCamBlenderLikeKeyboardPreset),
    /// Gamepad camera controls.
    Gamepad(OrbitCamGamepadPreset),
}

impl OrbitCamPreset {
    /// Builds the simple mouse input preset.
    #[must_use]
    pub fn simple_mouse() -> Self { OrbitCamSimpleMousePreset::default().into() }

    /// Builds the Blender-like input preset.
    #[must_use]
    pub fn blender_like() -> Self { OrbitCamBlenderLikePreset::default().into() }

    /// Builds the keyboard input preset.
    #[must_use]
    pub fn keyboard() -> Self { OrbitCamKeyboardPreset::default().into() }

    /// Builds the simple mouse plus keyboard input preset.
    #[must_use]
    pub fn simple_mouse_keyboard() -> Self { OrbitCamSimpleMouseKeyboardPreset::default().into() }

    /// Builds the Blender-like plus keyboard input preset.
    #[must_use]
    pub fn blender_like_keyboard() -> Self { OrbitCamBlenderLikeKeyboardPreset::default().into() }

    /// Builds the gamepad input preset.
    #[must_use]
    pub fn gamepad() -> Self { OrbitCamGamepadPreset::default().into() }

    /// Returns the preset's setting-insensitive identity.
    #[must_use]
    pub const fn kind(&self) -> OrbitCamPresetKind {
        match self {
            Self::SimpleMouse(_) => OrbitCamPresetKind::SimpleMouse,
            Self::BlenderLike(_) => OrbitCamPresetKind::BlenderLike,
            Self::Keyboard(_) => OrbitCamPresetKind::Keyboard,
            Self::SimpleMouseKeyboard(_) => OrbitCamPresetKind::SimpleMouseKeyboard,
            Self::BlenderLikeKeyboard(_) => OrbitCamPresetKind::BlenderLikeKeyboard,
            Self::Gamepad(_) => OrbitCamPresetKind::Gamepad,
        }
    }

    /// Returns the preset's display name.
    #[must_use]
    pub const fn name(&self) -> &'static str { self.kind().name() }

    /// Adds a binding that returns the camera to its home pose.
    ///
    /// No home input is bound unless this method is called. Each preset holds
    /// up to two home bindings (e.g. a key plus a gamepad button); a third
    /// call replaces the second binding.
    #[must_use]
    pub fn home(self, home: impl Into<Binding>) -> Self {
        let home = home.into();
        match self {
            Self::SimpleMouse(preset) => preset.home(home).into(),
            Self::BlenderLike(preset) => preset.home(home).into(),
            Self::Keyboard(preset) => preset.home(home).into(),
            Self::SimpleMouseKeyboard(preset) => preset.home(home).into(),
            Self::BlenderLikeKeyboard(preset) => preset.home(home).into(),
            Self::Gamepad(preset) => preset.home(home).into(),
        }
    }

    /// Returns whether this preset binds home input.
    #[must_use]
    pub const fn has_home(&self) -> bool {
        match self {
            Self::SimpleMouse(preset) => preset.has_home(),
            Self::BlenderLike(preset) => preset.has_home(),
            Self::Keyboard(preset) => preset.has_home(),
            Self::SimpleMouseKeyboard(preset) => preset.has_home(),
            Self::BlenderLikeKeyboard(preset) => preset.has_home(),
            Self::Gamepad(preset) => preset.has_home(),
        }
    }

    /// Converts this preset into validated custom bindings.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] if the preset construction violates a
    /// binding invariant.
    pub fn to_bindings(&self) -> Result<OrbitCamBindings, BindingsError> {
        match self {
            Self::SimpleMouse(preset) => preset.build(),
            Self::BlenderLike(preset) => preset.build(),
            Self::Keyboard(preset) => preset.build(),
            Self::SimpleMouseKeyboard(preset) => preset.build(),
            Self::BlenderLikeKeyboard(preset) => preset.build(),
            Self::Gamepad(preset) => preset.build(),
        }
    }
}

impl Default for OrbitCamPreset {
    fn default() -> Self { Self::simple_mouse() }
}

impl From<OrbitCamSimpleMousePreset> for OrbitCamPreset {
    fn from(preset: OrbitCamSimpleMousePreset) -> Self { Self::SimpleMouse(preset) }
}

impl From<OrbitCamBlenderLikePreset> for OrbitCamPreset {
    fn from(preset: OrbitCamBlenderLikePreset) -> Self { Self::BlenderLike(preset) }
}

impl From<OrbitCamKeyboardPreset> for OrbitCamPreset {
    fn from(preset: OrbitCamKeyboardPreset) -> Self { Self::Keyboard(preset) }
}

impl From<OrbitCamSimpleMouseKeyboardPreset> for OrbitCamPreset {
    fn from(preset: OrbitCamSimpleMouseKeyboardPreset) -> Self { Self::SimpleMouseKeyboard(preset) }
}

impl From<OrbitCamBlenderLikeKeyboardPreset> for OrbitCamPreset {
    fn from(preset: OrbitCamBlenderLikeKeyboardPreset) -> Self { Self::BlenderLikeKeyboard(preset) }
}

impl From<OrbitCamGamepadPreset> for OrbitCamPreset {
    fn from(preset: OrbitCamGamepadPreset) -> Self { Self::Gamepad(preset) }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::KeyCode;

    use super::*;

    #[test]
    fn constructors_return_default_payload_variants() {
        assert_eq!(
            OrbitCamPreset::simple_mouse(),
            OrbitCamPreset::SimpleMouse(OrbitCamSimpleMousePreset::default())
        );
        assert_eq!(
            OrbitCamPreset::blender_like(),
            OrbitCamPreset::BlenderLike(OrbitCamBlenderLikePreset::default())
        );
        assert_eq!(
            OrbitCamPreset::keyboard(),
            OrbitCamPreset::Keyboard(OrbitCamKeyboardPreset::default())
        );
        assert_eq!(
            OrbitCamPreset::simple_mouse_keyboard(),
            OrbitCamPreset::SimpleMouseKeyboard(OrbitCamSimpleMouseKeyboardPreset::default())
        );
        assert_eq!(
            OrbitCamPreset::blender_like_keyboard(),
            OrbitCamPreset::BlenderLikeKeyboard(OrbitCamBlenderLikeKeyboardPreset::default())
        );
        assert_eq!(
            OrbitCamPreset::gamepad(),
            OrbitCamPreset::Gamepad(OrbitCamGamepadPreset::default())
        );
    }

    #[test]
    fn blender_like_kind_reports_identity_name() {
        let preset = OrbitCamPreset::blender_like();

        assert_eq!(preset.kind(), OrbitCamPresetKind::BlenderLike);
        assert_eq!(preset.kind().name(), "BlenderLike");
        assert_eq!(preset.name(), "BlenderLike");
    }

    fn assert_preset_home_round_trip(preset: OrbitCamPreset) -> Result<(), BindingsError> {
        assert!(!preset.has_home());

        let preset = preset.home(KeyCode::KeyH);

        assert!(preset.has_home());
        assert_eq!(
            preset.to_bindings()?.home().to_vec(),
            vec![Binding::from(KeyCode::KeyH)]
        );
        Ok(())
    }

    #[test]
    fn enum_home_dispatch_round_trips_for_every_variant() -> Result<(), BindingsError> {
        assert_preset_home_round_trip(OrbitCamPreset::simple_mouse())?;
        assert_preset_home_round_trip(OrbitCamPreset::blender_like())?;
        assert_preset_home_round_trip(OrbitCamPreset::keyboard())?;
        assert_preset_home_round_trip(OrbitCamPreset::simple_mouse_keyboard())?;
        assert_preset_home_round_trip(OrbitCamPreset::blender_like_keyboard())?;
        assert_preset_home_round_trip(OrbitCamPreset::gamepad())?;
        Ok(())
    }
}
