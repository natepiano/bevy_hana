//! Which clock advances a camera's smoothing — shared across camera kinds.

use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;

/// Selects which clock advances a camera's smoothing each frame.
///
/// A required component on every camera kind. Swap it at runtime to change
/// behaviour: `commands.entity(camera).insert(TimeSource::Real)`.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[reflect(Component, Default)]
pub enum TimeSource {
    /// Bevy virtual time, which respects pause. The camera freezes when the
    /// world pauses.
    #[default]
    Virtual,
    /// Wall-clock time, which ignores pause. The camera keeps moving while the
    /// world is paused.
    Real,
}
