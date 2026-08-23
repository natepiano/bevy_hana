use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::FromWorld;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use bevy::prelude::With;
use bevy::prelude::World;
use bevy::render::render_resource::BindGroupEntries;
use bevy::render::render_resource::BindGroupLayoutDescriptor;
use bevy::render::render_resource::BindGroupLayoutEntries;
use bevy::render::render_resource::CachedRenderPipelineId;
use bevy::render::render_resource::ColorTargetState;
use bevy::render::render_resource::ColorWrites;
use bevy::render::render_resource::DynamicUniformBuffer;
use bevy::render::render_resource::FilterMode;
use bevy::render::render_resource::FragmentState;
use bevy::render::render_resource::MultisampleState;
use bevy::render::render_resource::Operations;
use bevy::render::render_resource::PipelineCache;
use bevy::render::render_resource::PrimitiveState;
use bevy::render::render_resource::RenderPassColorAttachment;
use bevy::render::render_resource::RenderPassDescriptor;
use bevy::render::render_resource::RenderPipeline;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::render::render_resource::Sampler;
use bevy::render::render_resource::SamplerBindingType;
use bevy::render::render_resource::SamplerDescriptor;
use bevy::render::render_resource::ShaderStages;
use bevy::render::render_resource::ShaderType;
use bevy::render::render_resource::TextureFormat;
use bevy::render::render_resource::TextureSampleType;
use bevy::render::render_resource::binding_types;
use bevy::render::renderer::RenderContext;
use bevy::render::renderer::RenderDevice;
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::CachedTexture;
use bevy_render::render_resource::TextureView;
use hana_kana::ToU32;
use hana_kana::ToUsize;

use super::camera::OutlineCamera;
use super::constants::FLOOD_SHADER_HANDLE;
use super::constants::FRAGMENT_SHADER_ENTRY_POINT;
use super::constants::FULL_SCREEN_DRAW_INSTANCE_COUNT;
use super::constants::JUMP_FLOOD_BIND_GROUP_LABEL;
use super::constants::JUMP_FLOOD_BIND_GROUP_LAYOUT_LABEL;
use super::constants::JUMP_FLOOD_BIND_GROUP_SLOT;
use super::constants::JUMP_FLOOD_DIAMETER_SCALE;
use super::constants::JUMP_FLOOD_FINAL_COMPOSE_PASS_COUNT;
use super::constants::JUMP_FLOOD_RADIUS_DIVISOR;
use super::constants::NO_FLOOD_PASS_COUNT;
use super::constants::NO_FLOOD_WIDTH_THRESHOLD;
use super::constants::NO_RENDER_PIPELINE_IMMEDIATE_DATA_SIZE;
use super::constants::OUTLINE_JUMP_FLOOD_PASS_LABEL;
use super::constants::OUTLINE_JUMP_FLOOD_PIPELINE_LABEL;
use super::constants::TRIANGLE_VERTEX_COUNT;
use super::extract::ExtractedOutlineUniforms;

#[derive(ShaderType)]
struct JumpFloodUniform {
    step_length: u32,
}

#[derive(Component, Default, Clone)]
pub(crate) struct FloodSettings {
    pub(crate) width: f32,
}

#[derive(Resource)]
pub(crate) struct JumpFloodPipeline {
    layout:                    BindGroupLayoutDescriptor,
    pub(crate) sampler:        Sampler,
    cached_render_pipeline_id: CachedRenderPipelineId,
    lookup_buffer:             DynamicUniformBuffer<JumpFloodUniform>,
    lookup_offsets:            Vec<u32>,
}

impl FromWorld for JumpFloodPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>().clone();

        let layout = BindGroupLayoutDescriptor::new(
            JUMP_FLOOD_BIND_GROUP_LAYOUT_LABEL,
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    binding_types::texture_2d(TextureSampleType::Float { filterable: true }), /* flood_texture */
                    binding_types::sampler(SamplerBindingType::Filtering), // texture_sampler
                    binding_types::uniform_buffer::<JumpFloodUniform>(true), // instance
                    binding_types::texture_depth_2d(),                     // depth_texture
                    binding_types::texture_2d(TextureSampleType::Float { filterable: true }), /* appearance_texture */
                ),
            ),
        );
        let sampler = render_device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            ..Default::default()
        });

        let fullscreen_shader = world.resource::<FullscreenShader>().clone();

        let cached_render_pipeline_id = world
            .resource_mut::<PipelineCache>()
            .queue_render_pipeline(RenderPipelineDescriptor {
                label:                            Some(OUTLINE_JUMP_FLOOD_PIPELINE_LABEL.into()),
                layout:                           vec![layout.clone()],
                vertex:                           fullscreen_shader.to_vertex_state(),
                fragment:                         Some(FragmentState {
                    shader:      FLOOD_SHADER_HANDLE,
                    shader_defs: vec![],
                    entry_point: Some(FRAGMENT_SHADER_ENTRY_POINT.into()),
                    targets:     vec![Some(ColorTargetState {
                        format:     TextureFormat::Rgba32Float,
                        blend:      None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive:                        PrimitiveState::default(),
                depth_stencil:                    None,
                multisample:                      MultisampleState::default(),
                immediate_size:                   NO_RENDER_PIPELINE_IMMEDIATE_DATA_SIZE,
                zero_initialize_workgroup_memory: false,
            });

        let render_queue = world.resource::<RenderQueue>();
        let mut uniform_buffer = DynamicUniformBuffer::new_with_alignment(u64::from(
            render_device.limits().min_uniform_buffer_offset_alignment,
        ));
        let mut offsets = Vec::new();
        for bit in 0..u32::BITS {
            offsets.push(uniform_buffer.push(&JumpFloodUniform {
                step_length: 1 << bit,
            }));
        }
        uniform_buffer.write_buffer(&render_device, render_queue);

        Self {
            layout,
            sampler,
            cached_render_pipeline_id,
            lookup_buffer: uniform_buffer,
            lookup_offsets: offsets,
        }
    }
}

pub(crate) struct JumpFloodPass<'w> {
    pub(crate) pipeline: &'w JumpFloodPipeline,
    render_pipeline:     &'w RenderPipeline,
    pipeline_cache:      &'w PipelineCache,
}

pub(crate) struct JumpFloodStep<'a> {
    pub(crate) input:           &'a CachedTexture,
    pub(crate) output:          &'a CachedTexture,
    pub(crate) depth_view:      &'a TextureView,
    pub(crate) appearance_view: &'a TextureView,
    pub(crate) size:            u32,
}

impl<'w> JumpFloodPass<'w> {
    pub(crate) fn new(world: &'w World) -> Option<Self> {
        let pipeline = world.resource::<JumpFloodPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let render_pipeline =
            pipeline_cache.get_render_pipeline(pipeline.cached_render_pipeline_id)?;

        Some(Self {
            pipeline,
            render_pipeline,
            pipeline_cache,
        })
    }

    pub(crate) fn execute(
        &self,
        render_context: &mut RenderContext<'_, '_>,
        step: JumpFloodStep<'_>,
    ) {
        let JumpFloodStep {
            input,
            output,
            depth_view,
            appearance_view,
            size,
        } = step;
        let Some(lookup_binding) = self.pipeline.lookup_buffer.binding() else {
            return;
        };
        let bind_group = render_context.render_device().create_bind_group(
            JUMP_FLOOD_BIND_GROUP_LABEL,
            &self
                .pipeline_cache
                .get_bind_group_layout(&self.pipeline.layout),
            &BindGroupEntries::sequential((
                &input.default_view,
                &self.pipeline.sampler,
                lookup_binding,
                depth_view,
                appearance_view,
            )),
        );

        let mut render_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label:                    Some(OUTLINE_JUMP_FLOOD_PASS_LABEL),
            color_attachments:        &[Some(RenderPassColorAttachment {
                view:           &output.default_view,
                resolve_target: None,
                ops:            Operations::default(),
                depth_slice:    None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes:         None,
            occlusion_query_set:      None,
            multiview_mask:           None,
        });

        render_pass.set_render_pipeline(self.render_pipeline);
        render_pass.set_bind_group(
            JUMP_FLOOD_BIND_GROUP_SLOT,
            &bind_group,
            &[self.pipeline.lookup_offsets[size.to_usize()]],
        );
        render_pass.draw(0..TRIANGLE_VERTEX_COUNT, 0..FULL_SCREEN_DRAW_INSTANCE_COUNT);
    }
}

pub(crate) fn prepare_flood_settings(
    mut commands: Commands,
    extracted_outlines: Res<ExtractedOutlineUniforms>,
    cameras: Query<Entity, With<OutlineCamera>>,
) {
    let flood_settings = FloodSettings {
        width: extracted_outlines.max_jump_flood_width,
    };

    for entity in cameras.iter() {
        commands.entity(entity).insert(flood_settings.clone());
    }
}

/// Number of jump-flood passes required to cover an outline of the given
/// pixel width. Derived from the `JumpFlood` radius: convert width ->
/// diameter,
/// diameter → next-power-of-two radius, plus one final compose pass.
/// Returns 0 when no flood is needed.
pub(super) fn jump_flood_pass_count(width: f32) -> u32 {
    if width <= NO_FLOOD_WIDTH_THRESHOLD {
        return NO_FLOOD_PASS_COUNT;
    }

    ((width * JUMP_FLOOD_DIAMETER_SCALE).ceil().to_u32() / JUMP_FLOOD_RADIUS_DIVISOR
        + JUMP_FLOOD_FINAL_COMPOSE_PASS_COUNT)
        .next_power_of_two()
        .trailing_zeros()
        + JUMP_FLOOD_FINAL_COMPOSE_PASS_COUNT
}
