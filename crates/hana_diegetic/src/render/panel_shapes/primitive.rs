//! Stable source identity for batched panel-line primitives.

use bevy::prelude::Entity;

use crate::layout::PanelShapePrimitiveKey;

/// Stable cross-panel source identity for one line primitive record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PanelShapeRenderKey {
    /// Panel entity that owns the primitive source.
    pub(super) panel:  Entity,
    /// Stable primitive key inside the panel's resolved command stream.
    pub(super) source: PanelShapePrimitiveKey,
}
