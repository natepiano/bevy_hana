//! Window behavior that only the native window system provides: tab suppression and native
//! fullscreen notifications. `AppKit` is the only backend; elsewhere nothing is registered and
//! native fullscreen is never observed.

#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(not(target_os = "macos"), path = "unsupported.rs")]
mod selected;

pub(crate) use selected::NativeFullscreenObservations;
pub(crate) use selected::activate_fullscreen_window;
pub(crate) use selected::register;
