//! Shared analytic quadratic path renderer.
//!
//! Text still owns shaping, font lookup, and glyph outline extraction. This
//! module owns the renderer-facing records, packing, material, shader handles,
//! and batch store so future panel-authored vector marks can use the same
//! analytic coverage path.

mod after_oit_resolve;
mod atlas;
mod batching;
mod constants;
mod geometry;
mod material;
mod packing;

use std::collections::HashSet;

pub use after_oit_resolve::PostOitResolveText3d;
use after_oit_resolve::PostOitResolveTextPlugin;
pub(crate) use atlas::AtlasPublication;
pub(crate) use atlas::AtlasPublicationError;
pub(crate) use atlas::AtlasState;
pub(crate) use atlas::PathAtlas;
pub(crate) use batching::GeometryDirty;
pub(crate) use batching::MaterialDirty;
pub(crate) use batching::PathBatchKey;
pub(crate) use batching::PathBatchResources;
pub(crate) use batching::PlacementDirty;
pub(crate) use batching::TextRunBatchStore;
pub(crate) use batching::analytic_material_slot_candidate;
pub(crate) use batching::path_render_record_relative_to_first_draw_order_index;
use bevy::asset::load_internal_asset;
use bevy::pbr::MaterialPlugin;
use bevy::pbr::SpecializedMaterialPipelineCache;
use bevy::pbr::SpecializedPrepassMaterialPipelineCache;
use bevy::pbr::SpecializedShadowMaterialPipelineCache;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Plugin;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Shader;
use bevy::prelude::resource_equals;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSystems;
use bevy::render::render_resource::CachedPipelineState;
use bevy::render::render_resource::PipelineCache;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::render::storage::ShaderBuffer;
pub(crate) use geometry::Bounds;
pub(crate) use geometry::ContourRole;
pub(crate) use geometry::PathContour;
pub(crate) use geometry::PathOutline;
pub(crate) use geometry::QuadraticSegment;
pub(crate) use material::PathExtendedMaterial;
pub(crate) use material::PathExtension;
pub(crate) use material::PathMaterialBuffers;
pub(crate) use material::RenderMode;
#[cfg(test)]
pub(crate) use material::path_material_oit_paint_order;
pub(crate) use material::set_path_display_table;
pub(crate) use material::set_path_material_anti_alias;
pub(crate) use material::set_path_material_atlas;
pub(crate) use material::set_path_material_hairline;
pub(crate) use material::set_path_material_record_buffers;
pub(crate) use material::set_path_material_table_buffer;
pub(crate) use packing::BandRecord;
pub(crate) use packing::CurveRecord;
pub(crate) use packing::DEFAULT_BAND_COUNT;
pub(crate) use packing::PackedPath;
pub(crate) use packing::PackedPathRecord;
pub(crate) use packing::PathQuadRecord;
pub(crate) use packing::PathRenderRecord;
pub(crate) use packing::TextFitMode;
pub(crate) use packing::build_packed_path;

use self::constants::ANALYTIC_PATH_SHADER_HANDLE;
use self::constants::ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE;
use super::AntiAlias;

#[inline]
pub(super) fn vertex_pull(
    render_mode: RenderMode,
    oit_paint_order: f32,
    anti_alias: AntiAlias,
    buffers: PathMaterialBuffers,
) -> PathExtension {
    material::PathExtension::vertex_pull(
        render_mode,
        AlphaMode::Blend,
        oit_paint_order,
        anti_alias,
        buffers,
    )
}

#[inline]
pub(super) fn text_vertex_pull(
    alpha_mode: AlphaMode,
    oit_paint_order: f32,
    anti_alias: AntiAlias,
    buffers: PathMaterialBuffers,
) -> PathExtension {
    let mut extension = material::PathExtension::vertex_pull(
        RenderMode::Text,
        alpha_mode,
        oit_paint_order,
        anti_alias,
        buffers,
    );
    extension.is_panel_text = true;
    extension
}

/// Stable handles for the shared analytic-path atlas buffers every run's
/// material binds.
#[derive(Clone, Debug)]
pub(crate) struct PathAtlasHandles {
    /// Shared band-packed quadratic curve records.
    pub curves:       Handle<ShaderBuffer>,
    /// Shared along-Y/along-X band records.
    pub bands:        Handle<ShaderBuffer>,
    /// Shared packed-path records, indexed by each quad record's `packed_path_index`.
    pub path_records: Handle<ShaderBuffer>,
}

pub(super) struct AnalyticPathPlugin;

/// Whether the render world's first frame has waited for its analytic-path
/// pipelines. Only that frame waits, so its text and shapes draw; every later
/// frame runs without waiting, and a pipeline queued later compiles in the
/// background while Bevy skips its items.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FirstFrameWait {
    #[default]
    Pending,
    Done,
}

// Bevy queues render pipelines in this same Render set. Process that queue
// here so every visible analytic-path pipeline has a task, then wait only for
// those tasks before the first frame is submitted: text, shapes and their
// shadows draw on the first frame. Bevy's own queue system can safely process
// the now-empty queue again.
fn wait_for_first_frame_analytic_path_pipelines(
    mut pipeline_cache: ResMut<PipelineCache>,
    main_pass: Res<SpecializedMaterialPipelineCache>,
    prepass: Res<SpecializedPrepassMaterialPipelineCache>,
    shadow: Res<SpecializedShadowMaterialPipelineCache>,
    mut first_frame_wait: ResMut<FirstFrameWait>,
) {
    *first_frame_wait = FirstFrameWait::Done;
    pipeline_cache.process_queue();
    let main_pass_ids = main_pass.values().flat_map(|view| view.values().copied());
    let prepass_ids = prepass
        .values()
        .flat_map(|view| view.values().map(|&(_, id, _)| id));
    let shadow_ids = shadow
        .values()
        .flat_map(|view| view.values().map(|&(id, _, _)| id));
    let compiling: HashSet<_> = main_pass_ids
        .chain(prepass_ids)
        .chain(shadow_ids)
        .filter(|id| {
            matches!(
                pipeline_cache.get_render_pipeline_state(*id),
                CachedPipelineState::Creating(_)
            ) && draws_analytic_paths(pipeline_cache.get_render_pipeline_descriptor(*id))
        })
        .collect();
    for id in compiling {
        pipeline_cache.block_on_render_pipeline(id);
    }
}

/// Whether a pipeline draws analytic paths: text runs, and the panel shapes
/// that share their shaders.
///
/// Its fragment stage computes path coverage or its vertex stage pulls path
/// quads. A shadow pipeline may have no fragment stage; one with neither shader
/// keeps the standard vertex stage, which rasterizes nothing for a path batch.
#[must_use]
pub fn draws_analytic_paths(descriptor: &RenderPipelineDescriptor) -> bool {
    descriptor.vertex.shader.id() == ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE.id()
        || descriptor
            .fragment
            .as_ref()
            .is_some_and(|fragment| fragment.shader.id() == ANALYTIC_PATH_SHADER_HANDLE.id())
}

impl Plugin for AnalyticPathPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            ANALYTIC_PATH_SHADER_HANDLE,
            "analytic_path.wesl",
            |source, _| Shader::from_wesl(source, "embedded://hana_diegetic/analytic_path.wesl")
        );
        load_internal_asset!(
            app,
            ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE,
            "analytic_path_vertex_pull.wesl",
            |source, _| Shader::from_wesl(
                source,
                "embedded://hana_diegetic/analytic_path_vertex_pull.wesl"
            )
        );
        app.add_plugins(MaterialPlugin::<PathExtendedMaterial>::default());
        app.add_plugins(PostOitResolveTextPlugin);
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.init_resource::<FirstFrameWait>().add_systems(
                Render,
                wait_for_first_frame_analytic_path_pipelines
                    .run_if(resource_equals(FirstFrameWait::Pending))
                    .in_set(RenderSystems::Render)
                    .before(bevy::render::renderer::render_system),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::prelude::AssetPlugin;
    use bevy::prelude::Assets;
    use bevy::prelude::MinimalPlugins;
    use bevy::render::render_resource::FragmentState;
    use bevy::render::render_resource::VertexState;

    use super::*;

    #[test]
    fn plugin_build_holds_both_analytic_path_shaders_before_any_update() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_resource::<Assets<Shader>>();
        app.add_plugins(AnalyticPathPlugin);

        let shaders = app.world().resource::<Assets<Shader>>();
        assert!(shaders.contains(ANALYTIC_PATH_SHADER_HANDLE.id()));
        assert!(shaders.contains(ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE.id()));
    }

    #[test]
    fn analytic_path_pipelines_are_found_by_either_stage() {
        let with_stages =
            |vertex: Handle<Shader>, fragment: Option<Handle<Shader>>| RenderPipelineDescriptor {
                vertex: VertexState {
                    shader: vertex,
                    ..Default::default()
                },
                fragment: fragment.map(|shader| FragmentState {
                    shader,
                    ..Default::default()
                }),
                ..Default::default()
            };
        let other = Handle::<Shader>::default();

        assert!(draws_analytic_paths(&with_stages(
            other.clone(),
            Some(ANALYTIC_PATH_SHADER_HANDLE)
        )));
        // A shadow pipeline with no fragment stage still pulls path quads.
        assert!(draws_analytic_paths(&with_stages(
            ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE,
            None
        )));
        assert!(!draws_analytic_paths(&with_stages(
            other.clone(),
            Some(other.clone())
        )));
        assert!(!draws_analytic_paths(&with_stages(other, None)));
    }
}
