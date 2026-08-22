use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::SRes;
use bevy::pbr::DrawMesh;
use bevy::pbr::RenderMeshInstances;
use bevy::pbr::SetMeshBindGroup;
use bevy::pbr::SetMeshViewBindGroup;
use bevy::pbr::SetMeshViewBindingArrayBindGroup;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy_render::render_phase::PhaseItem;
use bevy_render::render_phase::RenderCommand;
use bevy_render::render_phase::RenderCommandResult;
use bevy_render::render_phase::SetItemPipeline;
use bevy_render::render_phase::TrackedRenderPass;
use bevy_render::render_phase::ViewBinnedRenderPhases;
use bevy_render::render_resource::BindGroup;
use bevy_render::render_resource::BindGroupEntries;
use bevy_render::render_resource::BindGroupEntry;
use bevy_render::render_resource::GpuArrayBuffer;
use bevy_render::render_resource::PipelineCache;
use bevy_render::render_resource::TextureViewDescriptor;
use bevy_render::renderer::RenderDevice;
use bevy_render::renderer::RenderQueue;
use bytemuck::Zeroable;

use super::constants::HULL_DEPTH_BIND_GROUP_LABEL;
use super::constants::HULL_DEPTH_BIND_GROUP_SLOT;
use super::constants::HULL_MESH_BIND_GROUP_SLOT;
use super::constants::HULL_MESH_VIEW_BIND_GROUP_SLOT;
use super::constants::HULL_MESH_VIEW_BINDING_ARRAY_BIND_GROUP_SLOT;
use super::constants::HULL_OUTLINE_BIND_GROUP_LABEL;
use super::constants::HULL_OUTLINE_BIND_GROUP_SLOT;
use super::constants::MESH_BIND_GROUP_SLOT;
use super::constants::MESH_VIEW_BIND_GROUP_SLOT;
use super::constants::MESH_VIEW_BINDING_ARRAY_BIND_GROUP_SLOT;
use super::constants::OUTLINE_BIND_GROUP_LABEL;
use super::constants::OUTLINE_BIND_GROUP_SLOT;
use super::constants::OUTLINE_UNIFORM_BIND_GROUP_ENTRY_BINDING;
use super::extract::ActiveOutlineModes;
use super::extract::ExtractedOutlineUniforms;
use super::hull_pipeline::HullPipeline;
use super::mask::HullOutlinePhase;
use super::mask::JumpFloodOutlinePhase;
use super::mask_pipeline::MeshMaskPipeline;
use super::texture::FloodTextures;
use super::uniforms::OutlineUniform;

pub(crate) type DrawOutline = (
    SetItemPipeline,
    SetMeshViewBindGroup<MESH_VIEW_BIND_GROUP_SLOT>,
    SetMeshViewBindingArrayBindGroup<MESH_VIEW_BINDING_ARRAY_BIND_GROUP_SLOT>,
    SetMeshBindGroup<MESH_BIND_GROUP_SLOT>,
    SetOutlineBindGroup<OUTLINE_BIND_GROUP_SLOT>,
    DrawMesh,
);

pub(crate) type DrawHull = (
    SetItemPipeline,
    SetMeshViewBindGroup<HULL_MESH_VIEW_BIND_GROUP_SLOT>,
    SetMeshViewBindingArrayBindGroup<HULL_MESH_VIEW_BINDING_ARRAY_BIND_GROUP_SLOT>,
    SetMeshBindGroup<HULL_MESH_BIND_GROUP_SLOT>,
    SetHullOutlineBindGroup<HULL_OUTLINE_BIND_GROUP_SLOT>,
    SetHullDepthBindGroup<HULL_DEPTH_BIND_GROUP_SLOT>,
    DrawMesh,
);

pub(crate) struct SetOutlineBindGroup<const I: usize>();

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetOutlineBindGroup<I> {
    type Param = SRes<OutlineBindGroup>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _: &P,
        (): ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<()>,
        outline_bind_group: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let outline_bind_group = outline_bind_group.into_inner();

        outline_bind_group
            .0
            .as_ref()
            .map_or(RenderCommandResult::Skip, |bind_group| {
                pass.set_bind_group(I, bind_group, &[]);
                RenderCommandResult::Success
            })
    }
}

#[derive(Resource)]
pub(crate) struct OutlineUniformBuffer(pub GpuArrayBuffer<OutlineUniform>);

#[derive(Resource, Default)]
pub(crate) struct OutlineBindGroup(Option<BindGroup>);

#[derive(Resource)]
pub(crate) struct HullOutlineUniformBuffer(pub(crate) GpuArrayBuffer<OutlineUniform>);

#[derive(Resource, Default)]
pub(crate) struct HullOutlineBindGroup(Option<BindGroup>);

pub(crate) struct SetHullOutlineBindGroup<const I: usize>();

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetHullOutlineBindGroup<I> {
    type Param = SRes<HullOutlineBindGroup>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _: &P,
        (): ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<()>,
        outline_bind_group: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let outline_bind_group = outline_bind_group.into_inner();

        outline_bind_group
            .0
            .as_ref()
            .map_or(RenderCommandResult::Skip, |bind_group| {
                pass.set_bind_group(I, bind_group, &[]);
                RenderCommandResult::Success
            })
    }
}

pub(crate) struct SetHullDepthBindGroup<const I: usize>();

#[derive(Component)]
pub(crate) struct HullDepthViewBindGroup(BindGroup);

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetHullDepthBindGroup<I> {
    type Param = ();
    type ViewQuery = &'static HullDepthViewBindGroup;
    type ItemQuery = ();

    fn render<'w>(
        _: &P,
        depth_bind_group: ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<()>,
        (): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        pass.set_bind_group(I, &depth_bind_group.0, &[]);
        RenderCommandResult::Success
    }
}

/// Pushes `OutlineUniform` data into the `GpuArrayBuffer` in the same order
/// that `batch_and_prepare_binned_render_phase` pushes `MeshUniform` data,
/// so that `instance_index` in the shader indexes both buffers identically.
///
/// In the GPU preprocessing path, `batch_and_prepare` processes every entity in
/// the bins unconditionally. In the CPU path it calls `get_binned_batch_data`
/// which skips entities whose mesh instance is missing. We must mirror that
/// skip logic exactly so our indices stay aligned.
pub(crate) fn prepare_outline_buffer(
    render_mesh_instances: Res<RenderMeshInstances>,
    extracted_outlines: Res<ExtractedOutlineUniforms>,
    outline_phases: Res<ViewBinnedRenderPhases<JumpFloodOutlinePhase>>,
    mut outline_buffer: ResMut<OutlineUniformBuffer>,
) {
    outline_buffer.0.clear();

    let cpu_building = matches!(*render_mesh_instances, RenderMeshInstances::CpuBuilding(_));

    for phase in outline_phases.values() {
        // 0.19's `batch_and_prepare_binned_render_phase` builds the per-instance buffer
        // in unbatchable-then-batchable order. Liminal forces non-multidrawable binning
        // (see `queue.rs`), so `phase.multidrawable_meshes` is always empty — and its bin
        // contents are private in 0.19 — so it is not visited here.
        for unbatchables in phase.unbatchable_meshes.values() {
            for &main_entity in unbatchables.entities.keys() {
                // In CPU mode, `get_binned_batch_data` skips entities with no mesh
                // instance — mirror that here so indices stay aligned.
                if cpu_building
                    && render_mesh_instances
                        .render_mesh_queue_data(main_entity)
                        .is_none()
                {
                    continue;
                }

                let outline_uniform = extracted_outlines
                    .by_main_entity
                    .get(&main_entity)
                    .map_or_else(OutlineUniform::zeroed, OutlineUniform::from);
                outline_buffer.0.push(outline_uniform);
            }
        }

        for bin in phase.batchable_meshes.values() {
            for &main_entity in bin.entities().keys() {
                if cpu_building
                    && render_mesh_instances
                        .render_mesh_queue_data(main_entity)
                        .is_none()
                {
                    continue;
                }

                let outline_uniform = extracted_outlines
                    .by_main_entity
                    .get(&main_entity)
                    .map_or_else(OutlineUniform::zeroed, OutlineUniform::from);
                outline_buffer.0.push(outline_uniform);
            }
        }
    }
}

/// Writes `OutlineUniformBuffer` to the GPU and stores its `OutlineBindGroup`.
pub(crate) fn prepare_outline_bind_group(
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
    mesh_mask_pipeline: Res<MeshMaskPipeline>,
    mut outline_buffer: ResMut<OutlineUniformBuffer>,
    mut outline_bind_group: ResMut<OutlineBindGroup>,
) {
    outline_buffer.0.write_buffer(&render_device, &render_queue);

    if let Some(binding) = outline_buffer.0.binding() {
        let bind_group = render_device.create_bind_group(
            Some(OUTLINE_BIND_GROUP_LABEL),
            &pipeline_cache.get_bind_group_layout(&mesh_mask_pipeline.outline_bind_group_layout),
            &[BindGroupEntry {
                binding:  OUTLINE_UNIFORM_BIND_GROUP_ENTRY_BINDING,
                resource: binding,
            }],
        );
        outline_bind_group.0 = Some(bind_group);
    } else {
        outline_bind_group.0 = None;
    }
}

pub(crate) fn prepare_hull_depth_view_bind_groups(
    active: Res<ActiveOutlineModes>,
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    hull_pipeline: Res<HullPipeline>,
    views: Query<(Entity, &FloodTextures)>,
) {
    if !active.methods.has_hull() {
        return;
    }

    for (entity, flood_textures) in &views {
        let outline_depth_view = flood_textures
            .outline_depth
            .create_view(&TextureViewDescriptor::default());

        let bind_group = render_device.create_bind_group(
            Some(HULL_DEPTH_BIND_GROUP_LABEL),
            &pipeline_cache.get_bind_group_layout(&hull_pipeline.depth_layout),
            &BindGroupEntries::sequential((
                &hull_pipeline.occlusion_sampler,
                &outline_depth_view,
                &flood_textures.owner.default_view,
            )),
        );
        commands
            .entity(entity)
            .insert(HullDepthViewBindGroup(bind_group));
    }
}

pub(crate) fn prepare_hull_outline_buffer(
    active: Res<ActiveOutlineModes>,
    render_mesh_instances: Res<RenderMeshInstances>,
    extracted_outlines: Res<ExtractedOutlineUniforms>,
    outline_phases: Res<ViewBinnedRenderPhases<HullOutlinePhase>>,
    mut outline_buffer: ResMut<HullOutlineUniformBuffer>,
) {
    outline_buffer.0.clear();

    if !active.methods.has_hull() {
        return;
    }

    let cpu_building = matches!(*render_mesh_instances, RenderMeshInstances::CpuBuilding(_));

    for phase in outline_phases.values() {
        // Unbatchable-then-batchable order, matching 0.19's
        // `batch_and_prepare_binned_render_phase`. Multidrawable bins are always empty
        // here (forced in `queue.rs`) and are private in 0.19, so they are not visited.
        for unbatchables in phase.unbatchable_meshes.values() {
            for &main_entity in unbatchables.entities.keys() {
                if cpu_building
                    && render_mesh_instances
                        .render_mesh_queue_data(main_entity)
                        .is_none()
                {
                    continue;
                }

                let outline_uniform = extracted_outlines
                    .by_main_entity
                    .get(&main_entity)
                    .map_or_else(OutlineUniform::zeroed, OutlineUniform::from);
                outline_buffer.0.push(outline_uniform);
            }
        }

        for bin in phase.batchable_meshes.values() {
            for &main_entity in bin.entities().keys() {
                if cpu_building
                    && render_mesh_instances
                        .render_mesh_queue_data(main_entity)
                        .is_none()
                {
                    continue;
                }

                let outline_uniform = extracted_outlines
                    .by_main_entity
                    .get(&main_entity)
                    .map_or_else(OutlineUniform::zeroed, OutlineUniform::from);
                outline_buffer.0.push(outline_uniform);
            }
        }
    }
}

pub(crate) fn prepare_hull_outline_bind_group(
    active: Res<ActiveOutlineModes>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
    hull_pipeline: Res<HullPipeline>,
    mut outline_buffer: ResMut<HullOutlineUniformBuffer>,
    mut outline_bind_group: ResMut<HullOutlineBindGroup>,
) {
    if !active.methods.has_hull() {
        outline_bind_group.0 = None;
        return;
    }

    outline_buffer.0.write_buffer(&render_device, &render_queue);

    if let Some(binding) = outline_buffer.0.binding() {
        let bind_group = render_device.create_bind_group(
            Some(HULL_OUTLINE_BIND_GROUP_LABEL),
            &pipeline_cache.get_bind_group_layout(&hull_pipeline.outline_layout),
            &[BindGroupEntry {
                binding:  OUTLINE_UNIFORM_BIND_GROUP_ENTRY_BINDING,
                resource: binding,
            }],
        );
        outline_bind_group.0 = Some(bind_group);
    } else {
        outline_bind_group.0 = None;
    }
}
