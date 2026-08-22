//! Shared analytic quadratic path renderer.
//!
//! Text still owns shaping, font lookup, and glyph outline extraction. This
//! module owns the renderer-facing records, packing, material, shader handles,
//! and batch store so future panel-authored vector marks can use the same
//! analytic coverage path.

mod atlas;
mod batching;
mod constants;
mod geometry;
mod material;
mod packing;

pub(crate) use atlas::PathAtlas;
pub(crate) use batching::GeometryDirty;
pub(crate) use batching::MaterialDirty;
pub(crate) use batching::PathBatchKey;
pub(crate) use batching::PathBatchResources;
pub(crate) use batching::PlacementDirty;
pub(crate) use batching::TextRunBatchStore;
pub(crate) use batching::analytic_material_slot_candidate;
pub(crate) use batching::path_render_record_relative_to_first_draw_order_index;
use bevy::asset::embedded_asset;
use bevy::asset::load_internal_asset;
use bevy::pbr::MaterialPlugin;
use bevy::prelude::App;
use bevy::prelude::Handle;
use bevy::prelude::Plugin;
use bevy::prelude::Shader;
use bevy::render::storage::ShaderBuffer;
pub(crate) use geometry::Bounds;
pub(crate) use geometry::PathContour;
pub(crate) use geometry::PathOutline;
pub(crate) use geometry::QuadraticSegment;
pub(crate) use material::PathExtendedMaterial;
pub(crate) use material::PathExtension;
pub(crate) use material::PathMaterialBuffers;
pub(crate) use material::RenderMode;
#[cfg(test)]
pub(crate) use material::path_material_oit_depth_offset;
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
pub(crate) use packing::build_packed_path;

use super::AntiAlias;

#[inline]
pub(super) fn vertex_pull(
    render_mode: RenderMode,
    oit_depth_offset: f32,
    anti_alias: AntiAlias,
    buffers: PathMaterialBuffers,
) -> PathExtension {
    material::PathExtension::vertex_pull(render_mode, oit_depth_offset, anti_alias, buffers)
}

use self::constants::ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE;

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

impl Plugin for AnalyticPathPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "analytic_path.wgsl");
        load_internal_asset!(
            app,
            ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE,
            "analytic_path_vertex_pull.wgsl",
            Shader::from_wgsl
        );
        app.add_plugins(MaterialPlugin::<PathExtendedMaterial>::default());
    }
}
