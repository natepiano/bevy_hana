use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;

/// Enhanced-input context component installed on cameras controlled by `OrbitCam`.
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct OrbitCamInputContext;

/// Enhanced-input context component installed on cameras controlled by `FreeCam`.
#[derive(Component, Clone, Copy, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct FreeCamInputContext;
