use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;

/// Disables app-level camera input for an entity while preserving its selected input mode.
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct CameraInputDisabled;
