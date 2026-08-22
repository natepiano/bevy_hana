//! Batched rendering for panel-owned line primitives.

mod batching;
mod path;
mod primitive;
mod relationship;

use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::App;
use bevy::prelude::EntityCommands;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;

use self::batching::DiegeticPanelShapeBatch;
use self::batching::ShapeBatchStore;
use self::batching::commit_panel_line_batch_buffers;
use self::batching::reconcile_panel_line_batches;
use self::batching::update_panel_line_batch_bounds;
pub(crate) use self::primitive::PanelShapeRenderKey;
use self::relationship::PanelShapes;
use super::PanelChildSystems;
use super::material_table;
use super::material_table::BatchAssetWrites;
use super::material_table::BatchResourcesReady;
use crate::widgets::VisualOverrideIndex;

/// Plugin that adds batched panel-line rendering.
pub(super) struct PanelShapePlugin;

impl Plugin for PanelShapePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShapeBatchStore>()
            .init_resource::<VisualOverrideIndex>()
            .add_systems(
                PostUpdate,
                (
                    reconcile_panel_line_batches.in_set(BatchAssetWrites),
                    material_table::register_path_batch_materials::<DiegeticPanelShapeBatch>,
                    commit_panel_line_batch_buffers,
                    update_panel_line_batch_bounds,
                )
                    .chain()
                    .in_set(PanelChildSystems::Build)
                    .in_set(BatchResourcesReady)
                    .before(VisibilitySystems::CheckVisibility),
            );
    }
}

pub(crate) fn remove_panel_relationship(entity: &mut EntityCommands<'_>) {
    entity.remove::<PanelShapes>();
}
