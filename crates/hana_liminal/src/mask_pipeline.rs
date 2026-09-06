use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::SRes;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::MeshInputUniform;
use bevy::pbr::MeshPipeline;
use bevy::pbr::MeshPipelineKey;
use bevy::pbr::MeshUniform;
use bevy::pbr::RenderMeshInstances;
use bevy::pbr::SkinUniforms;
use bevy::prelude::AssetId;
use bevy::prelude::Entity;
use bevy::prelude::FromWorld;
use bevy::prelude::Mesh;
use bevy::prelude::Resource;
use bevy::prelude::World;
use bevy::prelude::default;
use bevy::prelude::error;
use bevy::shader::ShaderDefVal;
use bevy_render::batching::GetBatchData;
use bevy_render::batching::GetFullBatchData;
use bevy_render::batching::gpu_preprocessing::IndirectParametersCpuMetadata;
use bevy_render::batching::gpu_preprocessing::UntypedPhaseIndirectParametersBuffers;
use bevy_render::mesh::RenderMesh;
use bevy_render::mesh::allocator::MeshAllocator;
use bevy_render::render_asset::RenderAssets;
use bevy_render::render_resource::BindGroupLayoutDescriptor;
use bevy_render::render_resource::BindGroupLayoutEntries;
use bevy_render::render_resource::ColorTargetState;
use bevy_render::render_resource::ColorWrites;
use bevy_render::render_resource::CompareFunction;
use bevy_render::render_resource::DepthStencilState;
use bevy_render::render_resource::Face;
use bevy_render::render_resource::FragmentState;
use bevy_render::render_resource::GpuArrayBuffer;
use bevy_render::render_resource::MultisampleState;
use bevy_render::render_resource::RenderPipelineDescriptor;
use bevy_render::render_resource::ShaderStages;
use bevy_render::render_resource::SpecializedMeshPipeline;
use bevy_render::render_resource::SpecializedMeshPipelineError;
use bevy_render::render_resource::TextureFormat;
use bevy_render::renderer::RenderDevice;
use bevy_render::sync_world::MainEntity;
use nonmax::NonMaxU32;

use super::constants::FRAGMENT_SHADER_ENTRY_POINT;
use super::constants::GET_BATCH_DATA_GPU_MODE_ERROR;
use super::constants::GET_BINNED_BATCH_DATA_GPU_MODE_ERROR;
use super::constants::GET_BINNED_INDEX_CPU_MODE_ERROR;
use super::constants::GET_INDEX_AND_COMPARE_DATA_CPU_MODE_ERROR;
use super::constants::MASK_SHADER_HANDLE;
use super::constants::MISSING_BATCH_SET_INDEX;
use super::constants::OUTLINE_INSTANCE_BIND_GROUP_LAYOUT_LABEL;
use super::constants::OUTLINE_PIPELINE_LABEL;
use super::constants::PER_OBJECT_BUFFER_BATCH_SIZE_SHADER_DEF;
use super::indexing_mode::IndexingMode;
use super::uniforms::OutlineUniform;

#[derive(Resource)]
pub(crate) struct MeshMaskPipeline {
    mesh_pipeline:                        MeshPipeline,
    pub(crate) outline_bind_group_layout: BindGroupLayoutDescriptor,
    /// `Some(N)` on WebGL2 where only fixed-size uniform arrays are available,
    /// `None` on native GPU where unbounded storage buffer arrays are supported.
    /// When `Some`, injects the `PER_OBJECT_BUFFER_BATCH_SIZE` shader def so the
    /// WGSL shader declares a fixed-size array instead of an unbounded one.
    per_object_buffer_batch_size:         Option<u32>,
}

impl FromWorld for MeshMaskPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();

        let outline_instance_bind_group_layout = BindGroupLayoutDescriptor::new(
            OUTLINE_INSTANCE_BIND_GROUP_LAYOUT_LABEL,
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (GpuArrayBuffer::<OutlineUniform>::binding_layout(
                    &render_device.limits(),
                ),),
            ),
        );

        let per_object_buffer_batch_size =
            GpuArrayBuffer::<OutlineUniform>::batch_size(&render_device.limits());

        let mesh_pipeline = world.resource::<MeshPipeline>().clone();

        Self {
            mesh_pipeline,
            outline_bind_group_layout: outline_instance_bind_group_layout,
            per_object_buffer_batch_size,
        }
    }
}

impl SpecializedMeshPipeline for MeshMaskPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;

        // Force single-sample rendering. The mask pass renders data (UV coords, depth,
        // width, color) into `Rgba32Float` textures, not visual output. Multisampling
        // this data would produce incorrect interpolated values at mesh edges.
        descriptor.multisample = MultisampleState::default();

        descriptor.vertex.shader = MASK_SHADER_HANDLE;

        let mut shader_defs = vec![];
        if let Some(per_object_buffer_batch_size) = self.per_object_buffer_batch_size {
            shader_defs.push(ShaderDefVal::UInt(
                PER_OBJECT_BUFFER_BATCH_SIZE_SHADER_DEF.into(),
                per_object_buffer_batch_size,
            ));
        }
        descriptor.vertex.shader_defs.extend(shader_defs.clone());

        let targets = vec![
            // RT0 maps to `FloodTextures::output` and mask `flood_data`:
            // seed uv, outline width, and surface depth.
            Some(ColorTargetState {
                format:     TextureFormat::Rgba32Float,
                blend:      None,
                write_mask: ColorWrites::ALL,
            }),
            // RT1 maps to `FloodTextures::appearance` and mask `appearance_data`:
            // outline color and priority.
            Some(ColorTargetState {
                format:     TextureFormat::Rgba32Float,
                blend:      None,
                write_mask: ColorWrites::ALL,
            }),
            // RT2 maps to `FloodTextures::owner` and mask `owner_data`: owner ID
            // in x and overlap factor in y, read by both the hull and jump-flood
            // compose shaders for group-aware overlap resolution.
            Some(ColorTargetState {
                format:     TextureFormat::Rgba32Float,
                blend:      None,
                write_mask: ColorWrites::ALL,
            }),
        ];

        descriptor.fragment = Some(FragmentState {
            shader: MASK_SHADER_HANDLE,
            shader_defs,
            entry_point: Some(FRAGMENT_SHADER_ENTRY_POINT.into()),
            targets,
        });

        descriptor.depth_stencil = Some(DepthStencilState {
            format:              CORE_3D_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare:       Some(CompareFunction::GreaterEqual),
            stencil:             default(),
            bias:                default(),
        });
        descriptor.label = Some(OUTLINE_PIPELINE_LABEL.into());
        descriptor
            .layout
            .push(self.outline_bind_group_layout.clone());
        descriptor.primitive.cull_mode = Some(Face::Back);

        Ok(descriptor)
    }
}

impl GetBatchData for MeshMaskPipeline {
    type Param = (
        SRes<RenderMeshInstances>,
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
        SRes<SkinUniforms>,
    );
    type BatchSetCompareData = ();
    type BatchCompareData = AssetId<Mesh>;

    type BufferData = MeshUniform;

    fn get_batch_data(
        (mesh_instances, _, mesh_allocator, skin_uniforms): &SystemParamItem<Self::Param>,
        (_, main_entity): (Entity, MainEntity),
    ) -> Option<(
        Self::BufferData,
        Option<(Self::BatchSetCompareData, Self::BatchCompareData)>,
    )> {
        let RenderMeshInstances::CpuBuilding(ref mesh_instances) = **mesh_instances else {
            error!("{GET_BATCH_DATA_GPU_MODE_ERROR}");
            return None;
        };
        let mesh_instance = mesh_instances.get(&main_entity)?;
        let first_vertex_index = mesh_allocator
            .mesh_vertex_slice(&mesh_instance.mesh_asset_id())
            .map_or(0, |slice| slice.range.start);

        let current_skin_index = skin_uniforms.skin_index(main_entity);
        let material_bind_group_index = mesh_instance.material_bindings_index();

        Some((
            MeshUniform::new(
                &mesh_instance.transforms,
                first_vertex_index,
                material_bind_group_index.slot,
                None,
                current_skin_index,
                None,
                Some(mesh_instance.tag()),
            ),
            Some(((), mesh_instance.mesh_asset_id())),
        ))
    }
}

impl GetFullBatchData for MeshMaskPipeline {
    type BufferInputData = MeshInputUniform;

    fn get_index_and_compare_data(
        (mesh_instances, _, _, _): &SystemParamItem<Self::Param>,
        main_entity: MainEntity,
    ) -> Option<(
        NonMaxU32,
        Option<(Self::BatchSetCompareData, Self::BatchCompareData)>,
    )> {
        // `MeshMaskPipeline::get_index_and_compare_data` requires
        // `RenderMeshInstances::GpuBuilding`.
        let RenderMeshInstances::GpuBuilding(ref mesh_instances) = **mesh_instances else {
            error!("{GET_INDEX_AND_COMPARE_DATA_CPU_MODE_ERROR}");
            return None;
        };

        let mesh_instance = mesh_instances.get(&main_entity)?;

        Some((
            NonMaxU32::new(mesh_instance.gpu_specific.current_uniform_index())?,
            Some(((), mesh_instance.mesh_asset_id())),
        ))
    }

    fn get_binned_batch_data(
        (mesh_instances, _, mesh_allocator, skin_uniforms): &SystemParamItem<Self::Param>,
        main_entity: MainEntity,
    ) -> Option<Self::BufferData> {
        let RenderMeshInstances::CpuBuilding(ref mesh_instances) = **mesh_instances else {
            error!("{GET_BINNED_BATCH_DATA_GPU_MODE_ERROR}");
            return None;
        };
        let mesh_instance = mesh_instances.get(&main_entity)?;
        let first_vertex_index = mesh_allocator
            .mesh_vertex_slice(&mesh_instance.mesh_asset_id())
            .map_or(0, |slice| slice.range.start);

        let current_skin_index = skin_uniforms.skin_index(main_entity);

        Some(MeshUniform::new(
            &mesh_instance.transforms,
            first_vertex_index,
            mesh_instance.material_bindings_index().slot,
            None,
            current_skin_index,
            None,
            Some(mesh_instance.tag()),
        ))
    }

    fn get_binned_index(
        (mesh_instances, _, _, _): &SystemParamItem<Self::Param>,
        main_entity: MainEntity,
    ) -> Option<NonMaxU32> {
        // `MeshMaskPipeline::get_binned_index` requires `RenderMeshInstances::GpuBuilding`.
        let RenderMeshInstances::GpuBuilding(ref mesh_instances) = **mesh_instances else {
            error!("{GET_BINNED_INDEX_CPU_MODE_ERROR}");
            return None;
        };

        mesh_instances
            .get(&main_entity)
            .and_then(|entity| NonMaxU32::new(entity.gpu_specific.current_uniform_index()))
    }

    fn write_batch_indirect_parameters_metadata(
        indexed: bool,
        base_output_index: u32,
        batch_set_index: Option<NonMaxU32>,
        phase_indirect_parameters_buffers: &mut UntypedPhaseIndirectParametersBuffers,
        indirect_parameters_offset: u32,
    ) {
        let indirect_parameters = IndirectParametersCpuMetadata {
            base_output_index,
            batch_set_index: batch_set_index.map_or(MISSING_BATCH_SET_INDEX, u32::from),
        };

        IndexingMode::from(indexed).write_metadata(
            phase_indirect_parameters_buffers,
            indirect_parameters_offset,
            indirect_parameters,
        );
    }
}
