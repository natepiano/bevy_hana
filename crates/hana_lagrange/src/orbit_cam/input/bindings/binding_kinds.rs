//! User-facing concrete binding kinds accepted by the
//! [`OrbitCamBindingsBuilder`](super::builder::OrbitCamBindingsBuilder): mouse drag, trackpad
//! scroll, mouse wheel, pinch, button drag, touch, gamepad policy, and zoom inversion.

use std::ops::Deref;

use bevy::prelude::MouseButton;
use bevy::prelude::Reflect;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::ModKeys;

use super::input_gain::OrbitCamInputGain;
use crate::input::BindingRoutePolicy;
use crate::input::HeldBinding;
use crate::input::InputGain;
use crate::input::InteractionSources;

/// A binding value plus its authored input gain.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct OrbitCamBindingWithInputGain<T> {
    binding:    T,
    input_gain: InputGain,
}

impl<T> OrbitCamBindingWithInputGain<T> {
    /// Creates a binding wrapper with explicit input gain.
    #[must_use]
    pub const fn new(binding: T, input_gain: f32) -> Self {
        Self {
            binding,
            input_gain: InputGain(input_gain),
        }
    }

    /// Returns the wrapped binding.
    #[must_use]
    pub const fn binding(&self) -> &T { &self.binding }

    /// Returns the authored input gain.
    #[must_use]
    pub const fn input_gain(&self) -> InputGain { self.input_gain }

    /// Replaces the authored input gain.
    #[must_use]
    pub const fn with_input_gain(mut self, input_gain: f32) -> Self {
        self.input_gain = InputGain(input_gain);
        self
    }
}

impl<T> Deref for OrbitCamBindingWithInputGain<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target { self.binding() }
}

impl<T> From<T> for OrbitCamBindingWithInputGain<T> {
    fn from(binding: T) -> Self {
        Self {
            binding,
            input_gain: InputGain::DEFAULT,
        }
    }
}

impl OrbitCamBindingWithInputGain<OrbitCamMouseDrag> {
    /// Requires keyboard modifiers on both mouse motion and button engagement.
    #[must_use]
    pub const fn with_mod_keys(mut self, mod_keys: ModKeys) -> Self {
        self.binding = self.binding.with_mod_keys(mod_keys);
        self
    }
}

impl OrbitCamBindingWithInputGain<OrbitCamTrackpadScroll> {
    /// Requires keyboard modifiers on smooth-scroll input.
    #[must_use]
    pub const fn with_mod_keys(mut self, mod_keys: ModKeys) -> Self {
        self.binding = self.binding.with_mod_keys(mod_keys);
        self
    }
}

impl OrbitCamBindingWithInputGain<OrbitCamLineScroll> {
    /// Requires keyboard modifiers on line-scroll input.
    #[must_use]
    pub const fn with_mod_keys(mut self, mod_keys: ModKeys) -> Self {
        self.binding = self.binding.with_mod_keys(mod_keys);
        self
    }
}

impl OrbitCamBindingWithInputGain<OrbitCamButtonDragZoom> {
    /// Sets the axis used for button-drag zoom.
    #[must_use]
    pub const fn with_axis(mut self, axis: OrbitCamButtonDragZoomAxis) -> Self {
        self.binding = self.binding.with_axis(axis);
        self
    }
}

/// Mouse-drag binding for orbit or pan behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub struct OrbitCamMouseDrag {
    /// Mouse button that engages the drag.
    pub button:   MouseButton,
    /// Keyboard modifiers required by both motion and button engagement.
    pub mod_keys: ModKeys,
}

impl OrbitCamMouseDrag {
    /// Creates a mouse-drag binding without keyboard modifiers.
    #[must_use]
    pub const fn new(button: MouseButton) -> Self {
        Self {
            button,
            mod_keys: ModKeys::empty(),
        }
    }

    /// Requires keyboard modifiers on both mouse motion and button engagement.
    #[must_use]
    pub const fn with_mod_keys(mut self, mod_keys: ModKeys) -> Self {
        self.mod_keys = mod_keys;
        self
    }

    /// Sets the authored input gain for this mouse-drag binding.
    #[must_use]
    pub const fn with_input_gain(self, input_gain: f32) -> OrbitCamBindingWithInputGain<Self> {
        OrbitCamBindingWithInputGain::new(self, input_gain)
    }
}

impl From<OrbitCamMouseDrag> for HeldBinding {
    fn from(value: OrbitCamMouseDrag) -> Self {
        Self::new(
            Binding::MouseMotion {
                mod_keys: value.mod_keys,
            },
            Binding::MouseButton {
                button:   value.button,
                mod_keys: value.mod_keys,
            },
        )
        .with_sources(InteractionSources::MOUSE)
        .with_route(BindingRoutePolicy::CursorPosition)
    }
}

impl From<OrbitCamBindingWithInputGain<OrbitCamMouseDrag>> for HeldBinding {
    fn from(value: OrbitCamBindingWithInputGain<OrbitCamMouseDrag>) -> Self {
        Self::from(*value.binding()).with_input_gain(value.input_gain().value())
    }
}

/// Trackpad smooth-scroll binding for orbit, pan, or zoom behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct OrbitCamTrackpadScroll {
    /// Keyboard modifiers required by the smooth-scroll binding.
    pub mod_keys: ModKeys,
}

impl OrbitCamTrackpadScroll {
    /// Requires keyboard modifiers on smooth-scroll input.
    #[must_use]
    pub const fn with_mod_keys(mut self, mod_keys: ModKeys) -> Self {
        self.mod_keys = mod_keys;
        self
    }

    /// Sets the authored input gain for this smooth-scroll binding.
    #[must_use]
    pub const fn with_input_gain(self, input_gain: f32) -> OrbitCamBindingWithInputGain<Self> {
        OrbitCamBindingWithInputGain::new(self, input_gain)
    }
}

/// Line-scroll binding for orbit, pan, or zoom behavior.
///
/// Accepts mouse-wheel events, including trackpads forwarded as wheel input.
/// A matching enabled binding takes precedence over [`OrbitCamMouseWheelZoom`].
/// Orbit and pan gain is measured in pixels per line; zoom gain uses the same
/// pixel-equivalent scale as [`OrbitCamTrackpadScroll`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct OrbitCamLineScroll {
    /// Keyboard modifiers required by the line-scroll binding.
    pub mod_keys: ModKeys,
}

impl OrbitCamLineScroll {
    /// Requires keyboard modifiers on line-scroll input.
    #[must_use]
    pub const fn with_mod_keys(mut self, mod_keys: ModKeys) -> Self {
        self.mod_keys = mod_keys;
        self
    }

    /// Sets the authored input gain for this line-scroll binding.
    #[must_use]
    pub const fn with_input_gain(self, input_gain: f32) -> OrbitCamBindingWithInputGain<Self> {
        OrbitCamBindingWithInputGain::new(self, input_gain)
    }
}

/// Mouse-wheel zoom binding. Zoom direction is governed by the camera's
/// [`ZoomInversion`]; this binding only enables wheel zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct OrbitCamMouseWheelZoom;

impl OrbitCamMouseWheelZoom {
    /// Sets the authored input gain for this mouse-wheel zoom binding.
    #[must_use]
    pub const fn with_input_gain(self, input_gain: f32) -> OrbitCamBindingWithInputGain<Self> {
        OrbitCamBindingWithInputGain::new(self, input_gain)
    }
}

/// Pinch gesture zoom binding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub struct OrbitCamPinchZoom;

impl OrbitCamPinchZoom {
    /// Sets the authored input gain for this pinch zoom binding.
    #[must_use]
    pub const fn with_input_gain(self, input_gain: f32) -> OrbitCamBindingWithInputGain<Self> {
        OrbitCamBindingWithInputGain::new(self, input_gain)
    }
}

/// Button-drag zoom binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub struct OrbitCamButtonDragZoom {
    /// Mouse button that engages button-drag zoom.
    pub button: MouseButton,
    /// Axis used for button-drag zoom.
    pub axis:   OrbitCamButtonDragZoomAxis,
}

impl OrbitCamButtonDragZoom {
    /// Creates a button-drag zoom binding using vertical motion.
    #[must_use]
    pub const fn new(button: MouseButton) -> Self {
        Self {
            button,
            axis: OrbitCamButtonDragZoomAxis::Y,
        }
    }

    /// Sets the axis used for button-drag zoom.
    #[must_use]
    pub const fn with_axis(mut self, axis: OrbitCamButtonDragZoomAxis) -> Self {
        self.axis = axis;
        self
    }

    /// Sets the authored input gain for this button-drag zoom binding.
    #[must_use]
    pub const fn with_input_gain(self, input_gain: f32) -> OrbitCamBindingWithInputGain<Self> {
        OrbitCamBindingWithInputGain::new(self, input_gain)
    }
}

/// Touch gesture policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum OrbitCamTouchBinding {
    /// One finger orbits and two fingers pan and zoom.
    OneFingerOrbit,
    /// One finger pans and two fingers orbit and zoom.
    TwoFingerOrbit,
}

impl OrbitCamTouchBinding {
    /// Sets per-action authored input gain for this touch policy.
    #[must_use]
    pub const fn with_input_gain(
        self,
        input_gain: OrbitCamInputGain,
    ) -> OrbitCamTouchBindingConfig {
        OrbitCamTouchBindingConfig {
            binding: self,
            input_gain,
        }
    }
}

/// Touch gesture policy plus per-action authored input gain.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct OrbitCamTouchBindingConfig {
    binding:    OrbitCamTouchBinding,
    input_gain: OrbitCamInputGain,
}

impl OrbitCamTouchBindingConfig {
    /// Creates a touch policy using default input gain for every action.
    #[must_use]
    pub const fn new(binding: OrbitCamTouchBinding) -> Self {
        Self {
            binding,
            input_gain: OrbitCamInputGain::new(),
        }
    }

    /// Returns the touch policy.
    #[must_use]
    pub const fn binding(self) -> OrbitCamTouchBinding { self.binding }

    /// Returns the authored per-action touch input gain.
    #[must_use]
    pub const fn input_gain(self) -> OrbitCamInputGain { self.input_gain }

    /// Returns whether any touch action participates in runtime input.
    #[must_use]
    pub const fn has_enabled_action(self) -> bool {
        self.orbit_enabled() || self.pan_enabled() || self.zoom_enabled()
    }

    /// Returns whether touch orbit participates in runtime input.
    #[must_use]
    pub const fn orbit_enabled(self) -> bool { self.input_gain.orbit_input_gain().is_enabled() }

    /// Returns whether touch pan participates in runtime input.
    #[must_use]
    pub const fn pan_enabled(self) -> bool { self.input_gain.pan_input_gain().is_enabled() }

    /// Returns whether touch zoom participates in runtime input.
    #[must_use]
    pub const fn zoom_enabled(self) -> bool { self.input_gain.zoom_input_gain().is_enabled() }
}

impl From<OrbitCamTouchBinding> for OrbitCamTouchBindingConfig {
    fn from(binding: OrbitCamTouchBinding) -> Self { Self::new(binding) }
}

/// Whether scroll-based zoom (mouse wheel, pinch, smooth-scroll) is inverted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
pub enum ZoomInversion {
    /// Scroll-based zoom runs in its default direction.
    #[default]
    Normal,
    /// Scroll-based zoom is inverted: each gesture zooms the opposite way.
    Inverted,
}

/// Axis used for button-drag zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum OrbitCamButtonDragZoomAxis {
    /// Horizontal motion controls zoom.
    X,
    /// Vertical motion controls zoom.
    #[default]
    Y,
    /// Horizontal plus vertical motion controls zoom.
    XY,
}

/// Gamepad routing policy for camera input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[non_exhaustive]
pub enum CameraInputGamepadSelectionPolicy {
    /// Ignore gamepad input.
    #[default]
    Disabled,
    /// Route a single gamepad through active camera routing.
    Active,
}
