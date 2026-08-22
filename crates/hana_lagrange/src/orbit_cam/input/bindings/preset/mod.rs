//! Built-in orbit-camera input presets that compile down to validated
//! [`super::OrbitCamBindings`].

mod blender_like;
mod blender_like_keyboard;
mod config;
mod enum_preset;
mod gamepad;
mod keyboard;
mod simple_mouse;
mod simple_mouse_keyboard;

pub use blender_like::OrbitCamBlenderLikePreset;
pub use blender_like_keyboard::OrbitCamBlenderLikeKeyboardPreset;
pub use enum_preset::OrbitCamPreset;
pub use enum_preset::OrbitCamPresetKind;
pub use gamepad::OrbitCamGamepadPreset;
pub use gamepad::OrbitCamGamepadPresetBuilder;
pub use keyboard::OrbitCamKeyboardPreset;
pub use simple_mouse::OrbitCamSimpleMousePreset;
pub use simple_mouse_keyboard::OrbitCamSimpleMouseKeyboardPreset;

pub use crate::input::GamepadInputGain;
pub use crate::input::MouseInputGain;
pub use crate::input::SmoothScrollInputGain;
