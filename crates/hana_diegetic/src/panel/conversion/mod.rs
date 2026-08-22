//! Panel coordinate-space projection and conversion.

mod error;
mod projection;
mod saved_screen_state;
mod saved_world_state;
mod screen;
mod screen_handoff;
mod world;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::Entity;
use bevy::prelude::Transform;
pub use error::PanelProjectionError;
pub use projection::PanelProjectionParam;
pub use projection::PanelScreenProjection;
pub use projection::PanelWorldProjection;
pub use saved_screen_state::SavedPanelScreenState;
pub use saved_world_state::SavedPanelWorldState;
pub use screen::PanelScreenConversion;
pub use screen::PanelScreenTarget;
pub(crate) use screen::apply_screen_conversion;
pub(crate) use screen::apply_screen_root_sizing;
pub(crate) use screen::validate_screen_conversion;
pub use screen_handoff::PanelScreenHandoff;
pub use world::PanelWorldConversion;
pub use world::PanelWorldTarget;
pub use world::SavedWorldRestoreMode;
pub(crate) use world::apply_world_conversion;
pub(crate) use world::validate_world_conversion;

use super::DiegeticPanel;
use crate::layout::Lighting;
use crate::layout::Sidedness;
use crate::layout::Unit;

pub(super) fn saved_world_state_from_panel(
    panel: &DiegeticPanel,
    transform: &Transform,
    resolved_font_unit: Unit,
    resolved_lighting: Lighting,
    resolved_sidedness: Sidedness,
    render_layers: Option<&RenderLayers>,
) -> SavedPanelWorldState {
    SavedPanelWorldState::from_panel(
        panel,
        transform,
        resolved_font_unit,
        resolved_lighting,
        resolved_sidedness,
        render_layers,
    )
}

pub(super) const fn panel_screen_handoff(
    camera: Entity,
    conversion: PanelScreenConversion,
    distance: f32,
) -> PanelScreenHandoff {
    PanelScreenHandoff::new(camera, conversion, distance)
}
