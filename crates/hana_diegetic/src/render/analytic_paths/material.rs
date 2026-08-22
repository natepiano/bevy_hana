use bevy::asset::Asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::ExtendedMaterial;
use bevy::pbr::MATERIAL_BIND_GROUP_INDEX;
use bevy::pbr::MaterialExtension;
use bevy::pbr::MaterialExtensionKey;
use bevy::pbr::MaterialExtensionPipeline;
use bevy::pbr::StandardMaterial;
use bevy::prelude::Handle;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::render::render_resource::ShaderType;
use bevy::render::render_resource::SpecializedMeshPipelineError;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

use super::constants::ANALYTIC_PATH_SHADER_PATH;
use super::constants::ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE;
use crate::layout::GlyphRenderMode;
use crate::render::AntiAlias;

/// Visible render mode for the path shader.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub(crate) enum RenderMode {
    /// Normal coverage fill.
    #[default]
    Text     = 1,
    /// Inverted coverage inside each path quad.
    PunchOut = 2,
}

impl From<RenderMode> for u32 {
    fn from(mode: RenderMode) -> Self { mode as Self }
}

impl From<GlyphRenderMode> for RenderMode {
    fn from(render_mode: GlyphRenderMode) -> Self {
        match render_mode {
            GlyphRenderMode::Text => Self::Text,
            GlyphRenderMode::PunchOut => Self::PunchOut,
        }
    }
}

/// Material used by the path renderer.
pub(crate) type PathExtendedMaterial = ExtendedMaterial<StandardMaterial, PathExtension>;

/// GPU buffer handles bound by `PathExtension` for one vertex-pulled path
/// batch.
pub(crate) struct PathMaterialBuffers {
    /// Shared path record band-packed quadratic curve records.
    pub curves:       Handle<ShaderBuffer>,
    /// Shared path along-Y/along-X band records.
    pub bands:        Handle<ShaderBuffer>,
    /// Shared path records, indexed by each atlas record's `packed_path_index`.
    pub path_records: Handle<ShaderBuffer>,
    /// Per-path instance records read by the vertex-pulling and fragment stages.
    pub instances:    Handle<ShaderBuffer>,
    /// Per-run records read by the vertex-pulling and fragment stages.
    pub run_records:  Handle<ShaderBuffer>,
}

/// Uniforms consumed by the path shader.
#[derive(Clone, Debug, ShaderType)]
struct PathUniform {
    /// Visible render mode for this pass.
    render_mode:      u32,
    /// Per-layer depth offset applied to the OIT fragment position for coplanar
    /// layer ordering.
    oit_depth_offset: f32,
    /// Non-zero enables sub-pixel supersampling of path coverage (anti-aliases
    /// grazing-angle edges without MSAA).
    supersample:      u32,
    /// Non-zero switches the edge AA band from the scalar design-space
    /// `edge_width` to a screen-space band derived from the distance gradient,
    /// which fixes the convex-corner flare at grazing angles.
    aa_band:          u32,
    /// Minimum on-screen stroke width in device pixels for hairline-dilated
    /// paths (`PackedPathRecord::min_feature > 0`). Mirrored from
    /// [`HairlineWidth`](crate::HairlineWidth) by `sync_hairline_width`.
    hairline_min_px:  f32,
}

/// Constructor default for `PathUniform::hairline_min_px`; `sync_hairline_width`
/// replaces it with the window-scale-derived value on the asset-added event.
const HAIRLINE_DEFAULT_DEVICE_PX: f32 = 2.0;

/// Text material extension over `StandardMaterial`.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
#[bind_group_data(PathExtensionKey)]
pub(crate) struct PathExtension {
    /// Shader uniforms.
    #[uniform(100)]
    uniforms:       PathUniform,
    /// Shared path record band-packed quadratic curve records.
    #[storage(101, read_only)]
    curves:         Handle<ShaderBuffer>,
    /// Shared path along-Y/along-X band records.
    #[storage(102, read_only)]
    bands:          Handle<ShaderBuffer>,
    /// Shared path records, indexed by each atlas record's `packed_path_index`.
    #[storage(103, read_only)]
    path_records:   Handle<ShaderBuffer>,
    /// Per-path instance records read by the vertex-pulling and fragment stages.
    #[storage(104, read_only, visibility(vertex, fragment))]
    instances:      Handle<ShaderBuffer>,
    /// Per-run records (world transform, material slot, render mode, depth
    /// nudge) read by the vertex-pulling stages.
    #[storage(105, read_only, visibility(vertex, fragment))]
    run_records:    Handle<ShaderBuffer>,
    /// Shared `MaterialSlotValues` table read by migrated path fragments.
    #[storage(106, read_only, visibility(fragment))]
    material_table: Handle<ShaderBuffer>,
    /// Routes this material's vertex stages (main, prepass, shadow) through
    /// `analytic_path_vertex_pull.wgsl` instead of the standard mesh vertex stage.
    vertex_pull:    bool,
}

impl PathExtension {
    /// Builds the extension payload for one vertex-pulled analytic-path batch.
    #[must_use]
    pub(super) fn vertex_pull(
        render_mode: RenderMode,
        oit_depth_offset: f32,
        anti_alias: AntiAlias,
        buffers: PathMaterialBuffers,
    ) -> Self {
        Self {
            uniforms:       PathUniform {
                render_mode: u32::from(render_mode),
                oit_depth_offset,
                supersample: u32::from(anti_alias.supersamples()),
                aa_band: u32::from(anti_alias.anisotropic()),
                hairline_min_px: HAIRLINE_DEFAULT_DEVICE_PX,
            },
            curves:         buffers.curves,
            bands:          buffers.bands,
            path_records:   buffers.path_records,
            instances:      buffers.instances,
            run_records:    buffers.run_records,
            material_table: Handle::default(),
            vertex_pull:    true,
        }
    }
}

#[cfg(test)]
pub(crate) const fn path_material_oit_depth_offset(material: &PathExtendedMaterial) -> f32 {
    material.extension.uniforms.oit_depth_offset
}

/// Pipeline-specialization key for [`PathExtension`]: which vertex stage a
/// material compiles against.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct PathExtensionKey {
    /// Mirror of [`PathExtension::vertex_pull`].
    vertex_pull: bool,
}

impl From<&PathExtension> for PathExtensionKey {
    fn from(extension: &PathExtension) -> Self {
        Self {
            vertex_pull: extension.vertex_pull,
        }
    }
}

impl MaterialExtension for PathExtension {
    fn fragment_shader() -> ShaderRef { ANALYTIC_PATH_SHADER_PATH.into() }

    fn prepass_fragment_shader() -> ShaderRef { ANALYTIC_PATH_SHADER_PATH.into() }

    // Standing contract of the path renderer: the camera depth prepass cannot
    // run vertex-pull batches. Bevy strips the material bind group from
    // depth-only opaque prepass pipelines (`is_depth_only_opaque_prepass`),
    // and the vertex-pull vertex stage reads bindings 104/105 from that
    // group — pipeline creation fails with a wgpu validation error. The main
    // opaque pass writes its own depth (`GreaterEqual`, write enabled), so
    // skipping the prepass is an early-z loss only. Shadow views still queue
    // (`enable_shadows`); the ones bevy routes depth-only fall back to the
    // standard vertex stage in `specialize` (see
    // `material_group_is_stripped`). Any future change that re-enables the
    // prepass must keep (or consciously extend) that guard.
    fn enable_prepass() -> bool { false }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_path_extension_descriptor(descriptor, key.bind_group_data.vertex_pull);
        Ok(())
    }
}

fn specialize_path_extension_descriptor(
    descriptor: &mut RenderPipelineDescriptor,
    vertex_pull: bool,
) {
    if vertex_pull && !material_group_is_stripped(descriptor) {
        // One WGSL file serves the main, prepass, and shadow pipelines;
        // its `#ifdef PREPASS_PIPELINE` gate picks the entry point. The
        // swap happens here rather than in `vertex_shader()` /
        // `prepass_vertex_shader()` because those are material-type-wide,
        // and stripped-group pipelines must keep the standard vertex
        // stage (see `material_group_is_stripped`).
        descriptor.vertex.shader = ANALYTIC_PATH_VERTEX_PULL_SHADER_HANDLE;
        // The fragment sources material slot / render mode from the run
        // table (binding 105) instead of the material uniform, so a batch
        // renders every run with its own material-table row and mode.
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment
                .shader_defs
                .push("FRAGMENT_DATA_FROM_BATCHED_PATHS".into());
        }
    } else if vertex_pull && material_group_is_stripped(descriptor) {
        // Deliberately share the SDF helper's stripped-material-group branch:
        // the standard vertex stage avoids vertex-pull bindings 104/105, and
        // the helper avoids material-table binding 106.
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment
                .shader_defs
                .push("SDF_STRIPPED_MATERIAL_GROUP".into());
        }
    }
}

/// Whether bevy substituted the empty layout for the material bind group in
/// this pipeline. Depth-only pipelines do this — shadow views for alpha modes
/// without `MAY_DISCARD` (e.g. `Multiply`) — and the vertex-pull stage reads
/// bindings 104/105 from the material group, so swapping it in would fail
/// wgpu validation. Such pipelines keep the standard vertex stage instead:
/// the inert batch mesh's all-zero positions rasterize nothing, so the batch
/// casts no shadow there. This also catches any future mode or engine change
/// that strips the material group.
fn material_group_is_stripped(descriptor: &RenderPipelineDescriptor) -> bool {
    descriptor
        .layout
        .get(MATERIAL_BIND_GROUP_INDEX)
        .is_none_or(|material_layout| material_layout.entries.is_empty())
}

/// Repoints a batch material at replacement record buffers after capacity
/// growth.
pub(crate) fn set_path_material_record_buffers(
    material: &mut PathExtendedMaterial,
    instances: Handle<ShaderBuffer>,
    run_records: Handle<ShaderBuffer>,
) {
    material.extension.instances = instances;
    material.extension.run_records = run_records;
}

/// Repoints a path batch material at the current frame material table buffer.
pub(crate) fn set_path_material_table_buffer(
    material: &mut PathExtendedMaterial,
    material_table: Handle<ShaderBuffer>,
) {
    material.extension.material_table = material_table;
}

/// Repoints a path material at replacement shared-atlas buffers after the
/// atlas grows. The batch record buffers (bindings 104/105) are per-batch,
/// not atlas-owned, so they are untouched here.
pub(crate) fn set_path_material_atlas(
    material: &mut PathExtendedMaterial,
    curves: Handle<ShaderBuffer>,
    bands: Handle<ShaderBuffer>,
    path_records: Handle<ShaderBuffer>,
) {
    material.extension.curves = curves;
    material.extension.bands = bands;
    material.extension.path_records = path_records;
}

/// Updates the hairline minimum stroke width (device pixels) on a path
/// material.
pub(crate) const fn set_path_material_hairline(
    material: &mut PathExtendedMaterial,
    device_px: f32,
) {
    material.extension.uniforms.hairline_min_px = device_px;
}

/// Updates the shader anti-aliasing switches on a path material.
pub(crate) fn set_path_material_anti_alias(
    material: &mut PathExtendedMaterial,
    anti_alias: AntiAlias,
) {
    material.extension.uniforms.supersample = u32::from(anti_alias.supersamples());
    material.extension.uniforms.aa_band = u32::from(anti_alias.anisotropic());
}

#[cfg(test)]
mod tests {
    use bevy::render::render_resource::FragmentState;
    use bevy::shader::ShaderDefVal;

    use super::*;
    use crate::render::material_table::MATERIAL_TABLE_BINDING;
    use crate::render::material_table::PATH_INSTANCES_BINDING;
    use crate::render::material_table::PATH_RUN_RECORDS_BINDING;

    #[test]
    fn stripped_analytic_pipeline_guards_vertex_pull_and_material_table_bindings() {
        let mut descriptor = RenderPipelineDescriptor {
            fragment: Some(FragmentState::default()),
            ..Default::default()
        };

        specialize_path_extension_descriptor(&mut descriptor, true);

        assert!(descriptor.fragment.is_some(), "fragment state should exist");
        let Some(fragment) = descriptor.fragment.as_ref() else {
            return;
        };
        assert!(
            fragment
                .shader_defs
                .contains(&ShaderDefVal::from("SDF_STRIPPED_MATERIAL_GROUP"))
        );
        assert!(
            !fragment
                .shader_defs
                .contains(&ShaderDefVal::from("FRAGMENT_DATA_FROM_BATCHED_PATHS"))
        );
        let vertex_pull = include_str!("analytic_path_vertex_pull.wgsl");
        let fragment_shader = include_str!("analytic_path.wgsl");
        let helper = include_str!("../../shaders/sdf_material_table.wgsl");
        assert!(vertex_pull.contains(&format!("@binding({PATH_INSTANCES_BINDING})")));
        assert!(vertex_pull.contains(&format!("@binding({PATH_RUN_RECORDS_BINDING})")));
        assert!(fragment_shader.contains(&format!("@binding({PATH_RUN_RECORDS_BINDING})")));
        assert!(helper.contains(&format!("@binding({MATERIAL_TABLE_BINDING})")));
        assert!(helper.contains("#ifndef SDF_STRIPPED_MATERIAL_GROUP"));
        assert!(helper.contains("#ifdef SDF_STRIPPED_MATERIAL_GROUP"));
    }
}
