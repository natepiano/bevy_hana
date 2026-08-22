use std::ops::Range;

use bevy::asset::UntypedAssetId;
use bevy::prelude::Entity;
use bevy_render::mesh::allocator::MeshSlabs;
use bevy_render::render_phase::BinnedPhaseItem;
use bevy_render::render_phase::CachedRenderPipelinePhaseItem;
use bevy_render::render_phase::DrawFunctionId;
use bevy_render::render_phase::PhaseItem;
use bevy_render::render_phase::PhaseItemBatchSetKey;
use bevy_render::render_phase::PhaseItemExtraIndex;
use bevy_render::render_resource::CachedRenderPipelineId;
use bevy_render::sync_world::MainEntity;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OutlineBatchSetKey {
    pub(crate) cached_render_pipeline_id: CachedRenderPipelineId,
    pub(crate) draw_function_id:          DrawFunctionId,
    pub(crate) mesh_slabs:                MeshSlabs,
}

impl PhaseItemBatchSetKey for OutlineBatchSetKey {
    fn indexed(&self) -> bool { self.mesh_slabs.index_slab_id.is_some() }
}

/// Including `OutlineBinKey::main_entity` makes each entity its own unique bin.
/// Without it, GPU indirect drawing can reorder entities within a bin, causing
/// `instance_index` to map to the wrong `OutlineUniform` and shifting colors
/// between entities. `OutlineUniformBuffer` and `OutlineBindGroup` are still
/// shared across bins instead of allocating per-entity buffers.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OutlineBinKey {
    pub(crate) asset_id:    UntypedAssetId,
    pub(crate) main_entity: MainEntity,
}

pub(super) struct JumpFloodOutlinePhase {
    batch_set_key: OutlineBatchSetKey,
    entity:        Entity,
    main_entity:   MainEntity,
    batch_range:   Range<u32>,
    extra_index:   PhaseItemExtraIndex,
}

impl PhaseItem for JumpFloodOutlinePhase {
    #[inline]
    fn entity(&self) -> Entity { self.entity }

    fn main_entity(&self) -> MainEntity { self.main_entity }

    fn draw_function(&self) -> DrawFunctionId { self.batch_set_key.draw_function_id }

    fn batch_range(&self) -> &Range<u32> { &self.batch_range }

    fn batch_range_mut(&mut self) -> &mut Range<u32> { &mut self.batch_range }

    fn extra_index(&self) -> PhaseItemExtraIndex { self.extra_index.clone() }

    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.batch_range, &mut self.extra_index)
    }
}

impl BinnedPhaseItem for JumpFloodOutlinePhase {
    type BinKey = OutlineBinKey;
    type BatchSetKey = OutlineBatchSetKey;

    fn new(
        batch_set_key: Self::BatchSetKey,
        _: Self::BinKey,
        representative_entity: (Entity, MainEntity),
        batch_range: Range<u32>,
        extra_index: PhaseItemExtraIndex,
    ) -> Self {
        Self {
            batch_set_key,
            entity: representative_entity.0,
            main_entity: representative_entity.1,
            batch_range,
            extra_index,
        }
    }
}

impl CachedRenderPipelinePhaseItem for JumpFloodOutlinePhase {
    #[inline]
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.batch_set_key.cached_render_pipeline_id
    }
}

pub(crate) struct HullOutlinePhase {
    batch_set_key: OutlineBatchSetKey,
    entity:        Entity,
    main_entity:   MainEntity,
    batch_range:   Range<u32>,
    extra_index:   PhaseItemExtraIndex,
}

impl PhaseItem for HullOutlinePhase {
    #[inline]
    fn entity(&self) -> Entity { self.entity }

    fn main_entity(&self) -> MainEntity { self.main_entity }

    fn draw_function(&self) -> DrawFunctionId { self.batch_set_key.draw_function_id }

    fn batch_range(&self) -> &Range<u32> { &self.batch_range }

    fn batch_range_mut(&mut self) -> &mut Range<u32> { &mut self.batch_range }

    fn extra_index(&self) -> PhaseItemExtraIndex { self.extra_index.clone() }

    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.batch_range, &mut self.extra_index)
    }
}

impl BinnedPhaseItem for HullOutlinePhase {
    type BinKey = OutlineBinKey;
    type BatchSetKey = OutlineBatchSetKey;

    fn new(
        batch_set_key: Self::BatchSetKey,
        _: Self::BinKey,
        representative_entity: (Entity, MainEntity),
        batch_range: Range<u32>,
        extra_index: PhaseItemExtraIndex,
    ) -> Self {
        Self {
            batch_set_key,
            entity: representative_entity.0,
            main_entity: representative_entity.1,
            batch_range,
            extra_index,
        }
    }
}

impl CachedRenderPipelinePhaseItem for HullOutlinePhase {
    #[inline]
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.batch_set_key.cached_render_pipeline_id
    }
}
