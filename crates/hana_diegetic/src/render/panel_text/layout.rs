use bevy::prelude::Component;

use crate::PanelElementId;
use crate::layout::BoundingBox;
use crate::layout::DrawZIndex;
use crate::render::draw_order::DrawZIndexRank;

/// Layout payload for a panel-text run (a
/// [`TextContent`](crate::TextContent) entity).
///
/// Stores the layout-computed bounding box and panel scale factors needed to
/// build panel-local glyph meshes.
#[derive(Component, Clone, Debug)]
pub struct PanelTextLayout {
    /// Panel-local id of the source text run, plus the line ordinal within that
    /// run (`0` for an unwrapped run). Together they form the reification reuse
    /// key, replacing the former positional `(element_idx, command_index)` pair
    /// so a named run survives a sibling reorder.
    pub id:               PanelElementId,
    /// Line ordinal of this command within its run (`0`-based), so a wrapped
    /// multi-line run reuses each line stably.
    pub line_index:       usize,
    /// Index of the source element in the layout tree.
    pub element_idx:      usize,
    /// Dense panel-local `DrawOrderIndex`, stored as a `usize` for text
    /// batching.
    pub draw_ordinal:     usize,
    /// Legacy field name for the text run's non-OIT `ClipDepthNudge`.
    ///
    /// This is not the batch material's `StandardMaterial::depth_bias`; text
    /// batch materials derive that value from `PanelTextDrawZIndexRank`.
    pub depth_bias:       f32,
    /// OIT `position.z` offset projected from the run's `DrawOrderIndex`.
    pub oit_depth_offset: f32,
    /// Layout-computed position and size in layout coordinates.
    pub bounds:           BoundingBox,
    /// X scale: points to meters.
    pub scale_x:          f32,
    /// Y scale: points to meters.
    pub scale_y:          f32,
    /// `Anchor` X offset in world units.
    pub anchor_x:         f32,
    /// `Anchor` Y offset in world units.
    pub anchor_y:         f32,
    /// Active clip rect in layout coordinates, or `None` if unclipped.
    pub clip_rect:        Option<BoundingBox>,
}

/// Private batch-routing z-index for a panel-text run.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PanelTextDrawZIndex(pub(super) DrawZIndex);

/// Private batch-routing `DrawZIndexRank` for a panel-text run.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PanelTextDrawZIndexRank(pub(super) DrawZIndexRank);

impl PanelTextLayout {
    /// Bit-equality over the layout fields a panel-text glyph mesh depends on,
    /// used to gate per-run rebuilds.
    ///
    /// Compares `bounds`, ordering fields, `scale_x`, `scale_y`, `anchor_x`,
    /// `anchor_y`, and `clip_rect` via exact equality or `to_bits`. Excludes
    /// the reuse-identity fields (`id`/`line_index`/`element_idx`).
    pub(super) fn gating_eq(&self, other: &Self) -> bool {
        let Self {
            bounds,
            draw_ordinal,
            depth_bias,
            oit_depth_offset,
            scale_x,
            scale_y,
            anchor_x,
            anchor_y,
            clip_rect,
            id: _,
            line_index: _,
            element_idx: _,
        } = self;

        bbox_bits(bounds) == bbox_bits(&other.bounds)
            && *draw_ordinal == other.draw_ordinal
            && depth_bias.to_bits() == other.depth_bias.to_bits()
            && oit_depth_offset.to_bits() == other.oit_depth_offset.to_bits()
            && scale_x.to_bits() == other.scale_x.to_bits()
            && scale_y.to_bits() == other.scale_y.to_bits()
            && anchor_x.to_bits() == other.anchor_x.to_bits()
            && anchor_y.to_bits() == other.anchor_y.to_bits()
            && clip_rect.as_ref().map(bbox_bits) == other.clip_rect.as_ref().map(bbox_bits)
    }
}

/// Returns a [`BoundingBox`]'s four floats as raw bits for exact comparison.
const fn bbox_bits(bounds: &BoundingBox) -> [u32; 4] {
    [
        bounds.x.to_bits(),
        bounds.y.to_bits(),
        bounds.width.to_bits(),
        bounds.height.to_bits(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox(x: f32, y: f32, width: f32, height: f32) -> BoundingBox {
        BoundingBox {
            x,
            y,
            width,
            height,
        }
    }

    fn sample_layout() -> PanelTextLayout {
        PanelTextLayout {
            id:               PanelElementId::named("sample"),
            line_index:       0,
            element_idx:      0,
            draw_ordinal:     0,
            depth_bias:       0.0,
            oit_depth_offset: 0.0,
            bounds:           bbox(1.0, 2.0, 30.0, 12.0),
            scale_x:          0.5,
            scale_y:          0.5,
            anchor_x:         0.0,
            anchor_y:         0.0,
            clip_rect:        None,
        }
    }

    #[test]
    fn gating_eq_true_for_identical() {
        let layout = sample_layout();
        assert!(layout.gating_eq(&layout.clone()));
    }

    #[test]
    fn gating_eq_detects_bounds_change() {
        let base = sample_layout();
        let mut widened = sample_layout();
        widened.bounds.width = 40.0;
        assert!(!base.gating_eq(&widened));
    }

    #[test]
    fn gating_eq_ignores_reuse_key() {
        let base = sample_layout();
        let mut rekeyed = sample_layout();
        rekeyed.element_idx = 7;
        assert!(base.gating_eq(&rekeyed));
    }

    #[test]
    fn gating_eq_detects_ordering_change() {
        let base = sample_layout();
        let mut reordered = sample_layout();
        reordered.draw_ordinal = 3;
        reordered.depth_bias = 3.0;
        reordered.oit_depth_offset = 0.000_001;
        assert!(!base.gating_eq(&reordered));
    }

    #[test]
    fn gating_eq_detects_clip_rect_presence() {
        let base = sample_layout();
        let mut clipped = sample_layout();
        clipped.clip_rect = Some(bbox(0.0, 0.0, 100.0, 100.0));
        assert!(!base.gating_eq(&clipped));
    }

    #[test]
    fn gating_eq_distinguishes_signed_zero() {
        // to_bits, not ==: +0.0 and -0.0 are distinct bit patterns.
        let mut positive = sample_layout();
        let mut negative = sample_layout();
        positive.bounds.x = 0.0;
        negative.bounds.x = -0.0;
        assert!(!positive.gating_eq(&negative));
    }
}
