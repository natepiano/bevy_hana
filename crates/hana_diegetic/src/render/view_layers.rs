//! Private render layers shared by screen-space views and precompose rigs.

use std::collections::BTreeSet;

use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;

use super::constants::FIRST_PRIVATE_VIEW_RENDER_LAYER;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;

/// A render layer reserved for one screen-space view or precompose rig.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PrivateViewLayer(usize);

impl PrivateViewLayer {
    pub(crate) fn render_layers(self) -> RenderLayers { RenderLayers::from_layers(&[self.0]) }
}

/// All private layers are in use or cannot be represented by `usize`.
#[derive(Debug)]
pub(crate) struct PrivateViewLayersExhausted;

/// Private render layers currently held by screen-space views and rigs.
#[derive(Resource, Default)]
pub(crate) struct PrivateViewLayers {
    in_use: BTreeSet<usize>,
}

impl PrivateViewLayers {
    pub(crate) fn release(&mut self, layer: PrivateViewLayer) { self.in_use.remove(&layer.0); }
}

/// Reserves `PrivateViewLayers` entries, skipping every layer a screen-space
/// `DiegeticPanel` names in its `CoordinateSpace::Screen::render_layers` or an
/// entity already carries in its `RenderLayers`.
#[derive(SystemParam)]
pub(crate) struct PrivateViewLayerAllocator<'w, 's> {
    private_view_layers: ResMut<'w, PrivateViewLayers>,
    panels:              Query<'w, 's, &'static DiegeticPanel>,
    entity_layers:       Query<'w, 's, &'static RenderLayers>,
}

impl PrivateViewLayerAllocator<'_, '_> {
    pub(crate) fn allocate(&mut self) -> Result<PrivateViewLayer, PrivateViewLayersExhausted> {
        for candidate in FIRST_PRIVATE_VIEW_RENDER_LAYER..=usize::MAX {
            if self.private_view_layers.in_use.contains(&candidate) {
                continue;
            }
            let authored = self.panels.iter().any(|panel| {
                matches!(
                    panel.coordinate_space(),
                    CoordinateSpace::Screen { render_layers, .. }
                        if render_layers.iter().any(|layer| layer == candidate)
                )
            });
            let assigned = self
                .entity_layers
                .iter()
                .any(|layers| layers.iter().any(|layer| layer == candidate));
            if !authored && !assigned {
                self.private_view_layers.in_use.insert(candidate);
                return Ok(PrivateViewLayer(candidate));
            }
        }
        Err(PrivateViewLayersExhausted)
    }
}
