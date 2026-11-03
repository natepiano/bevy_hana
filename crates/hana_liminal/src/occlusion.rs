//! Full surface depth for transmissive bodies that do not carry an outline.

use std::ops::Range;

use bevy::extract::Extract;
use bevy::extract::sync_world::MainEntity;
use bevy::extract::sync_world::MainEntityHashSet;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::MeshPipeline;
use bevy::pbr::MeshPipelineKey;
use bevy::prelude::*;
use bevy::render::render_resource::CompareFunction;
use bevy::render::render_resource::DepthStencilState;
use bevy::render::render_resource::SpecializedMeshPipeline;
use bevy::render::render_resource::SpecializedMeshPipelineError;
use bevy_render::render_phase::BinnedPhaseItem;
use bevy_render::render_phase::CachedRenderPipelinePhaseItem;
use bevy_render::render_phase::DrawFunctionId;
use bevy_render::render_phase::PhaseItem;
use bevy_render::render_phase::PhaseItemExtraIndex;
use bevy_render::render_resource::CachedRenderPipelineId;
use bevy_render::render_resource::MultisampleState;
use bevy_render::render_resource::RenderPipelineDescriptor;

use crate::mask::OutlineBatchSetKey;
use crate::mask::OutlineBinKey;

pub(super) struct TranslucentDepthPhase {
    batch_set_key: OutlineBatchSetKey,
    entity:        Entity,
    main_entity:   MainEntity,
    batch_range:   Range<u32>,
    extra_index:   PhaseItemExtraIndex,
}

impl PhaseItem for TranslucentDepthPhase {
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

impl BinnedPhaseItem for TranslucentDepthPhase {
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

impl CachedRenderPipelinePhaseItem for TranslucentDepthPhase {
    #[inline]
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.batch_set_key.cached_render_pipeline_id
    }
}

#[derive(Resource, Default)]
pub(crate) struct TranslucentOccluders(pub(crate) MainEntityHashSet);

pub(crate) fn extract_translucent_occluders(
    mut occluders: ResMut<TranslucentOccluders>,
    meshes: Extract<Query<(Entity, &MeshMaterial3d<StandardMaterial>), With<Mesh3d>>>,
    materials: Extract<Res<Assets<StandardMaterial>>>,
) {
    occluders.0.clear();
    for (entity, handle) in &meshes {
        if materials
            .get(&handle.0)
            .is_some_and(|material| material.specular_transmission > 0.0)
        {
            occluders.0.insert(entity.into());
        }
    }
}

#[derive(Resource)]
pub(crate) struct TranslucentDepthPipeline(MeshPipeline);

impl FromWorld for TranslucentDepthPipeline {
    fn from_world(world: &mut World) -> Self { Self(world.resource::<MeshPipeline>().clone()) }
}

impl SpecializedMeshPipeline for TranslucentDepthPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.0.specialize(key, layout)?;
        descriptor.vertex.shader = crate::constants::MASK_SHADER_HANDLE;
        descriptor.fragment = None;
        descriptor.multisample = MultisampleState::default();
        descriptor.primitive.cull_mode = None;
        descriptor.depth_stencil = Some(DepthStencilState {
            format:              bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare:       Some(CompareFunction::GreaterEqual),
            stencil:             default(),
            bias:                default(),
        });
        Ok(descriptor)
    }
}
