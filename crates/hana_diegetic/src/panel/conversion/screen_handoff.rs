//! Saved screen handoff state for reversible panel conversions.

use bevy::prelude::Component;
use bevy::prelude::Entity;

use super::PanelScreenConversion;

/// Camera and screen conversion data saved when a world panel enters screen space.
#[derive(Component, Clone, Debug)]
pub struct PanelScreenHandoff {
    /// Camera used for the screen handoff.
    pub camera:     Entity,
    /// Screen conversion applied at the handoff.
    pub conversion: PanelScreenConversion,
    /// Distance from the handoff camera to the panel anchor along the camera view axis.
    pub distance:   f32,
}

impl PanelScreenHandoff {
    pub(super) const fn new(
        camera: Entity,
        conversion: PanelScreenConversion,
        distance: f32,
    ) -> Self {
        Self {
            camera,
            conversion,
            distance,
        }
    }
}
