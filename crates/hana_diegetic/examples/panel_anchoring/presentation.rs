//! Easing and world-panel materials shared by the anchor-demo and hinge-chain
//! scenes.

use bevy::prelude::Handle;
use bevy::prelude::Resource;
use bevy::prelude::StandardMaterial;
use hana_diegetic::default_panel_material;

/// Registered material handles shared by the anchor-panel examples.
#[derive(Clone, Resource)]
pub(crate) struct AnchorPanelMaterials {
    /// Source material for world-space panel fills.
    pub(crate) panel:  Handle<StandardMaterial>,
    /// Source material for world-space panel text.
    pub(crate) text:   Handle<StandardMaterial>,
    /// Source material for screen-space menu and info panels.
    pub(crate) screen: Handle<StandardMaterial>,
}

/// Cubic smoothstep easing on `t ∈ [0, 1]`.
pub(crate) fn smoothstep(t: f32) -> f32 { t * t * 2.0f32.mul_add(-t, 3.0) }

pub(crate) fn panel_material() -> StandardMaterial { default_panel_material() }

pub(crate) fn text_material() -> StandardMaterial {
    StandardMaterial {
        unlit: true,
        ..default_panel_material()
    }
}
