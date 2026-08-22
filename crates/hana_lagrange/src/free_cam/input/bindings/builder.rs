//! Programmatic construction of [`super::FreeCamBindings`] plus the user-facing input-kind types
//! accepted by the builder.
//!
//! Types:
//! - [`FreeCamTranslateKeys`] — the six-key WASD/Space/Ctrl translate helper.
//! - [`FreeCamMouseLook`] — mouse-motion look binding gated by a mouse button.
//! - [`FreeCamTranslateBinding`] / [`FreeCamLookBinding`] / [`FreeCamRollBinding`] — per-action
//!   wrappers over a [`HeldBinding`] accepted by the builder's `.translate()` / `.look()` /
//!   `.roll()` methods.
//! - [`FreeCamBindingsBuilder`] — accumulates per-action [`HeldBindingDescriptor`]s and validates
//!   them into [`super::FreeCamBindings`].

use bevy::prelude::KeyCode;
use bevy::prelude::MouseButton;
use bevy::prelude::Reflect;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ModKeys;

use super::FreeCamBindings;
use super::preset::FreeCamLookPitch;
use super::validate;
use crate::input::ActionBindingDescriptor;
use crate::input::BindingRoutePolicy;
use crate::input::BindingsError;
use crate::input::CameraInputGamepadSelectionPolicy;
use crate::input::CameraSlowMode;
use crate::input::HeldBinding;
use crate::input::HeldBindingDescriptor;
use crate::input::InputBinding;
use crate::input::InteractionSources;

/// Named translation-key bindings for a `FreeCam`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub struct FreeCamTranslateKeys {
    /// Key that moves the camera forward along its look direction.
    pub forward:  KeyCode,
    /// Key that moves the camera backward along its look direction.
    pub backward: KeyCode,
    /// Key that moves the camera left in its local basis.
    pub left:     KeyCode,
    /// Key that moves the camera right in its local basis.
    pub right:    KeyCode,
    /// Key that moves the camera up in its local basis.
    pub up:       KeyCode,
    /// Key that moves the camera down in its local basis.
    pub down:     KeyCode,
}

impl FreeCamTranslateKeys {
    /// Sets the forward translation key.
    #[must_use]
    pub const fn with_forward(mut self, forward: KeyCode) -> Self {
        self.forward = forward;
        self
    }

    /// Sets the backward translation key.
    #[must_use]
    pub const fn with_backward(mut self, backward: KeyCode) -> Self {
        self.backward = backward;
        self
    }

    /// Sets the left translation key.
    #[must_use]
    pub const fn with_left(mut self, left: KeyCode) -> Self {
        self.left = left;
        self
    }

    /// Sets the right translation key.
    #[must_use]
    pub const fn with_right(mut self, right: KeyCode) -> Self {
        self.right = right;
        self
    }

    /// Sets the upward translation key.
    #[must_use]
    pub const fn with_up(mut self, up: KeyCode) -> Self {
        self.up = up;
        self
    }

    /// Sets the downward translation key.
    #[must_use]
    pub const fn with_down(mut self, down: KeyCode) -> Self {
        self.down = down;
        self
    }

    const fn into_binding(self) -> InputBinding {
        InputBinding::vec3_keys(
            self.forward,
            self.backward,
            self.left,
            self.right,
            self.up,
            self.down,
        )
    }

    /// Sets the authored input gain for this translate-key binding.
    #[must_use]
    pub fn with_input_gain(self, input_gain: f32) -> FreeCamTranslateBinding {
        HeldBinding::same(self.into_binding())
            .with_input_gain(input_gain)
            .into()
    }
}

impl Default for FreeCamTranslateKeys {
    fn default() -> Self {
        Self {
            forward:  KeyCode::KeyW,
            backward: KeyCode::KeyS,
            left:     KeyCode::KeyA,
            right:    KeyCode::KeyD,
            up:       KeyCode::Space,
            down:     KeyCode::ControlLeft,
        }
    }
}

/// Mouse-motion look binding gated by a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeCamMouseLook {
    button: MouseButton,
}

impl FreeCamMouseLook {
    /// Creates a mouse-look binding engaged while `button` is held.
    #[must_use]
    pub const fn button(button: MouseButton) -> Self { Self { button } }

    /// Sets the authored input gain for this mouse-look binding.
    #[must_use]
    pub fn with_input_gain(self, input_gain: f32) -> FreeCamLookBinding {
        HeldBinding::from(self).with_input_gain(input_gain).into()
    }
}

impl From<FreeCamMouseLook> for HeldBinding {
    fn from(value: FreeCamMouseLook) -> Self {
        Self::new(
            Binding::mouse_motion(),
            Binding::MouseButton {
                button:   value.button,
                mod_keys: ModKeys::empty(),
            },
        )
        .with_sources(InteractionSources::MOUSE)
        .with_route(BindingRoutePolicy::CursorPosition)
    }
}

/// Binding that produces `FreeCam` translate intent.
#[derive(Clone, Debug, PartialEq)]
pub struct FreeCamTranslateBinding(HeldBinding);

impl FreeCamTranslateBinding {
    /// Sets the authored input gain for this translate binding.
    #[must_use]
    pub fn with_input_gain(mut self, input_gain: f32) -> Self {
        self.0 = self.0.with_input_gain(input_gain);
        self
    }
}

impl From<HeldBinding> for FreeCamTranslateBinding {
    fn from(binding: HeldBinding) -> Self { Self(binding) }
}

impl From<InputBinding> for FreeCamTranslateBinding {
    fn from(binding: InputBinding) -> Self { Self(HeldBinding::same(binding)) }
}

impl From<FreeCamTranslateKeys> for FreeCamTranslateBinding {
    fn from(keys: FreeCamTranslateKeys) -> Self { Self(HeldBinding::same(keys.into_binding())) }
}

/// Binding that produces `FreeCam` look intent.
#[derive(Clone, Debug, PartialEq)]
pub struct FreeCamLookBinding(HeldBinding);

impl FreeCamLookBinding {
    /// Sets the authored input gain for this look binding.
    #[must_use]
    pub fn with_input_gain(mut self, input_gain: f32) -> Self {
        self.0 = self.0.with_input_gain(input_gain);
        self
    }
}

impl From<HeldBinding> for FreeCamLookBinding {
    fn from(binding: HeldBinding) -> Self { Self(binding) }
}

impl From<InputBinding> for FreeCamLookBinding {
    fn from(binding: InputBinding) -> Self { Self(HeldBinding::same(binding)) }
}

impl From<FreeCamMouseLook> for FreeCamLookBinding {
    fn from(look: FreeCamMouseLook) -> Self { Self(look.into()) }
}

/// Binding that produces `FreeCam` roll intent.
#[derive(Clone, Debug, PartialEq)]
pub struct FreeCamRollBinding(HeldBinding);

impl FreeCamRollBinding {
    /// Sets the authored input gain for this roll binding.
    #[must_use]
    pub fn with_input_gain(mut self, input_gain: f32) -> Self {
        self.0 = self.0.with_input_gain(input_gain);
        self
    }
}

impl From<HeldBinding> for FreeCamRollBinding {
    fn from(binding: HeldBinding) -> Self { Self(binding) }
}

impl From<InputBinding> for FreeCamRollBinding {
    fn from(binding: InputBinding) -> Self { Self(HeldBinding::same(binding)) }
}

/// Builder for validated `FreeCam` bindings.
#[derive(Clone, Debug, Default)]
pub struct FreeCamBindingsBuilder {
    translate:  Vec<HeldBindingDescriptor>,
    look:       Vec<HeldBindingDescriptor>,
    roll:       Vec<HeldBindingDescriptor>,
    look_pitch: FreeCamLookPitch,
    slow_mode:  Option<CameraSlowMode>,
    gamepad:    CameraInputGamepadSelectionPolicy,
    home:       Vec<ActionBindingDescriptor>,
}

impl FreeCamBindingsBuilder {
    /// Adds a binding that produces translate intent.
    #[must_use]
    pub fn translate(mut self, binding: impl Into<FreeCamTranslateBinding>) -> Self {
        self.translate.push(binding.into().0.into());
        self
    }

    /// Adds a binding that produces look intent.
    #[must_use]
    pub fn look(mut self, binding: impl Into<FreeCamLookBinding>) -> Self {
        self.look.push(binding.into().0.into());
        self
    }

    /// Adds a binding that produces roll intent.
    #[must_use]
    pub fn roll(mut self, binding: impl Into<FreeCamRollBinding>) -> Self {
        self.roll.push(binding.into().0.into());
        self
    }

    /// Sets whether mouse Y is passed through or inverted for look input.
    #[must_use]
    pub const fn look_pitch(mut self, look_pitch: FreeCamLookPitch) -> Self {
        self.look_pitch = look_pitch;
        self
    }

    /// Sets the slow-mode policy.
    #[must_use]
    pub const fn slow_mode(mut self, slow_mode: CameraSlowMode) -> Self {
        self.slow_mode = Some(slow_mode);
        self
    }

    /// Removes the slow-mode policy.
    #[must_use]
    pub const fn without_slow_mode(mut self) -> Self {
        self.slow_mode = None;
        self
    }

    /// Sets the gamepad routing policy.
    #[must_use]
    pub const fn gamepad(mut self, gamepad: CameraInputGamepadSelectionPolicy) -> Self {
        self.gamepad = gamepad;
        self
    }

    /// Adds a source (key or gamepad button) that resets the camera to its home pose.
    #[must_use]
    pub fn home(mut self, home: impl Into<Binding>) -> Self {
        self.home.push(ActionBindingDescriptor::from(home.into()));
        self
    }

    /// Builds validated `FreeCam` bindings.
    ///
    /// # Errors
    ///
    /// Returns [`BindingsError`] when the builder violates a binding invariant.
    pub fn build(self) -> Result<FreeCamBindings, BindingsError> {
        let Self {
            translate,
            look,
            roll,
            look_pitch,
            slow_mode,
            gamepad,
            home,
        } = self;
        validate::validate_free_cam_bindings(
            &translate, &look, &roll, look_pitch, slow_mode, gamepad, &home,
        )
    }
}
