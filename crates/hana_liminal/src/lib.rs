//! Bevy plugin for rendering mesh outlines using jump-flood and hull-extrusion methods.

mod camera;
mod compose;
mod constants;
mod extract;
mod flood;
mod hull_pipeline;
mod indexing_mode;
mod mask;
mod mask_pipeline;
mod node;
mod outline;
mod outline_builder;
mod outline_normals;
mod propagation;
mod queue;
mod render;
mod shaders;
mod texture;
mod uniforms;
mod view;

use bevy::core_pipeline::Core3d;
use bevy::core_pipeline::Core3dSystems;
use bevy::pbr;
use bevy::pbr::MeshPipelineSystems;
use bevy::prelude::App;
use bevy::prelude::ExtractSchedule;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::World;
use bevy_render::Render;
use bevy_render::RenderApp;
use bevy_render::RenderDebugFlags;
use bevy_render::RenderStartup;
use bevy_render::RenderSystems;
use bevy_render::extract_component::ExtractComponentPlugin;
use bevy_render::render_phase::AddRenderCommand;
use bevy_render::render_phase::BinnedRenderPhasePlugin;
use bevy_render::render_phase::DrawFunctions;
use bevy_render::render_phase::ViewBinnedRenderPhases;
use bevy_render::render_resource::GpuArrayBuffer;
use bevy_render::render_resource::SpecializedMeshPipelines;
use bevy_render::renderer::RenderDevice;
pub use camera::OutlineCamera;
use compose::ComposeOutputPipeline;
pub use constants::ATTRIBUTE_OUTLINE_NORMAL;
use extract::ActiveOutlineModes;
use extract::ExtractedOutlineUniforms;
use flood::JumpFloodPipeline;
use hull_pipeline::HullPipeline;
use mask::HullOutlinePhase;
use mask::JumpFloodOutlinePhase;
use mask_pipeline::MeshMaskPipeline;
use node::outline_pass;
pub use outline::LineStyle;
pub use outline::NoOutline;
pub use outline::Outline;
pub use outline::OutlineActivity;
pub use outline::OutlineBarrier;
pub use outline::OutlineMethod;
pub use outline::OverlapMode;
pub use outline_builder::HullModeState;
pub use outline_builder::JumpFloodState;
pub use outline_builder::OutlineBuilder;
pub use outline_builder::OutlineModeState;
pub use outline_builder::ScreenHullState;
pub use outline_builder::WorldHullState;
pub use outline_normals::generate_outline_normals;
use render::DrawHull;
use render::DrawOutline;
use render::HullOutlineBindGroup;
use render::HullOutlineUniformBuffer;
use render::OutlineBindGroup;
use render::OutlineUniformBuffer;
use shaders::ShaderPlugin;

/// Bevy plugin that registers outline rendering systems and pipelines.
pub struct LiminalPlugin;

impl Plugin for LiminalPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ShaderPlugin,
            ExtractComponentPlugin::<OutlineCamera>::default(),
        ));

        // Propagation observers
        app.add_observer(propagation::propagate_outline_to_descendants);
        app.add_observer(propagation::propagate_outline_on_child_added);
        app.add_observer(propagation::propagate_outline_on_mesh_added);
        app.add_observer(propagation::propagate_outline_on_scene_ready);
        app.add_observer(propagation::remove_outline_from_descendants);

        // Outline normal generation observers
        app.add_observer(outline_normals::generate_normals_on_outline_added);
        app.add_observer(outline_normals::generate_normals_on_mesh_added);

        // Change detection for propagated outlines
        app.add_systems(PostUpdate, propagation::sync_propagated_outlines);

        // Ensure the main pass depth texture has TEXTURE_BINDING so the compose
        // shader can sample it for correct occlusion of transmissive/transparent geometry.
        app.add_observer(camera::configure_outline_camera_depth_texture);

        app.add_plugins((
            BinnedRenderPhasePlugin::<JumpFloodOutlinePhase, MeshMaskPipeline>::new(
                RenderDebugFlags::default(),
            ),
            BinnedRenderPhasePlugin::<HullOutlinePhase, HullPipeline>::new(
                RenderDebugFlags::default(),
            ),
        ));

        app.sub_app_mut(RenderApp)
            .init_resource::<DrawFunctions<JumpFloodOutlinePhase>>()
            .init_resource::<DrawFunctions<HullOutlinePhase>>()
            .init_resource::<SpecializedMeshPipelines<MeshMaskPipeline>>()
            .init_resource::<SpecializedMeshPipelines<HullPipeline>>()
            .init_resource::<ViewBinnedRenderPhases<JumpFloodOutlinePhase>>()
            .init_resource::<ViewBinnedRenderPhases<HullOutlinePhase>>()
            .init_resource::<OutlineBindGroup>()
            .init_resource::<HullOutlineBindGroup>()
            .init_resource::<ActiveOutlineModes>()
            .init_resource::<ExtractedOutlineUniforms>()
            // `MeshMaskPipeline` and `HullPipeline` clone the `MeshPipeline` resource,
            // which 0.19 creates in `RenderStartup` via `MeshPipelineSystems`, so their
            // initialization must run after it rather than in `Plugin::finish`.
            .add_systems(
                RenderStartup,
                init_outline_pipelines.after(MeshPipelineSystems),
            )
            .add_systems(
                ExtractSchedule,
                (
                    extract::extract_outline_uniforms,
                    view::update_views.after(pbr::extract_skins),
                ),
            )
            .add_systems(
                Render,
                (
                    extract::update_active_outline_modes
                        .in_set(RenderSystems::Queue)
                        .before(RenderSystems::QueueMeshes),
                    queue::queue_outline.in_set(RenderSystems::QueueMeshes),
                    queue::queue_hull_outline.in_set(RenderSystems::QueueMeshes),
                    render::prepare_outline_buffer.in_set(RenderSystems::PrepareResources),
                    render::prepare_hull_outline_buffer.in_set(RenderSystems::PrepareResources),
                    (
                        flood::prepare_flood_settings,
                        texture::prepare_flood_textures,
                        render::prepare_outline_bind_group.after(texture::prepare_flood_textures),
                        render::prepare_hull_outline_bind_group,
                        render::prepare_hull_depth_view_bind_groups
                            .after(texture::prepare_flood_textures),
                    )
                        .in_set(RenderSystems::PrepareBindGroups),
                ),
            )
            .add_render_command::<JumpFloodOutlinePhase, DrawOutline>()
            .add_render_command::<HullOutlinePhase, DrawHull>()
            // Run after the main pass and before bloom (`Core3dSystems::PostProcess`),
            // matching the former `EndMainPass -> OutlineNode -> Bloom` render-graph edges.
            .add_systems(Core3d, outline_pass.in_set(Core3dSystems::EarlyPostProcess));
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        let render_device = render_app.world().resource::<RenderDevice>();
        let outline_uniform_buffer =
            OutlineUniformBuffer(GpuArrayBuffer::new(&render_device.limits()));
        let hull_outline_uniform_buffer =
            HullOutlineUniformBuffer(GpuArrayBuffer::new(&render_device.limits()));

        render_app
            .insert_resource(outline_uniform_buffer)
            .insert_resource(hull_outline_uniform_buffer)
            .init_resource::<JumpFloodPipeline>()
            .init_resource::<ComposeOutputPipeline>();
    }
}

/// Initializes the outline pipelines that clone `MeshPipeline`. Runs in `RenderStartup`
/// after `MeshPipelineSystems` so the `MeshPipeline` resource already exists.
fn init_outline_pipelines(world: &mut World) {
    world.init_resource::<MeshMaskPipeline>();
    world.init_resource::<HullPipeline>();
}
