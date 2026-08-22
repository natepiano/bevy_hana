//! Debug overlays for typography visualization.
//!
//! This module is only compiled when the `typography_overlay` feature is enabled.

mod constants;
mod typography_overlay;

use bevy::prelude::App;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::Update;
pub use typography_overlay::GlyphMetricVisibility;
pub use typography_overlay::OverlayBoundingBox;
pub use typography_overlay::TypographyOverlay;

use crate::render;

pub(crate) struct TypographyOverlayPlugin;

impl Plugin for TypographyOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(typography_overlay::on_overlay_added)
            .add_observer(typography_overlay::on_overlay_removed)
            .add_systems(
                Update,
                (
                    render::emit_computed_world_text,
                    typography_overlay::build_typography_overlay,
                )
                    .chain(),
            );
    }
}
