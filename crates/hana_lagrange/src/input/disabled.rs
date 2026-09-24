use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;

/// Disables app-level camera input for an entity while preserving its selected input mode.
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct CameraInputDisabled;

/// Withholds wheel and trackpad scroll from a camera while every other input still reaches it.
///
/// An app inserts this while the pointer rests over something that consumes scroll itself, such
/// as a scrollable panel body; drag, keyboard, pinch, and touch input keep driving the camera.
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct CameraScrollInputDisabled;
