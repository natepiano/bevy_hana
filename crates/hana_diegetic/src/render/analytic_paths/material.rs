use bevy::asset::Asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::ExtendedMaterial;
use bevy::pbr::MATERIAL_BIND_GROUP_INDEX;
use bevy::pbr::MaterialExtension;
use bevy::pbr::MaterialExtensionKey;
use bevy::pbr::MaterialExtensionPipeline;
use bevy::pbr::MeshPipelineKey;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::prelude::Handle;
use bevy::prelude::Image;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::render_resource::FragmentState;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::render::render_resource::ShaderType;
use bevy::render::render_resource::SpecializedMeshPipelineError;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

use super::constants::ANALYTIC_PATH_SHADER_HANDLE;
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
    render_mode:     u32,
    /// Normalized local paint order used only at equal physical OIT depth.
    oit_paint_order: f32,
    /// Non-zero enables sub-pixel supersampling of path coverage (anti-aliases
    /// grazing-angle edges without MSAA).
    supersample:     u32,
    /// Non-zero switches the edge AA band from the scalar design-space
    /// `edge_width` to a screen-space band derived from the distance gradient,
    /// which fixes the convex-corner flare at grazing angles.
    aa_band:         u32,
    /// Minimum on-screen stroke width in device pixels for hairline-dilated
    /// paths (`PackedPathRecord::minimum_feature_size > 0`). Mirrored from
    /// [`HairlineWidth`](crate::HairlineWidth) by `sync_hairline_width`.
    hairline_min_px: f32,
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
    uniforms:                            PathUniform,
    /// Shared path record band-packed quadratic curve records.
    #[storage(101, read_only)]
    curves:                              Handle<ShaderBuffer>,
    /// Shared path along-Y/along-X band records.
    #[storage(102, read_only)]
    bands:                               Handle<ShaderBuffer>,
    /// Shared path records, indexed by each atlas record's `packed_path_index`.
    #[storage(103, read_only)]
    path_records:                        Handle<ShaderBuffer>,
    /// Per-path instance records read by the vertex-pulling and fragment stages.
    #[storage(104, read_only, visibility(vertex, fragment))]
    instances:                           Handle<ShaderBuffer>,
    /// Per-run records (world transform, material slot, render mode, depth
    /// nudge) read by the vertex-pulling stages.
    #[storage(105, read_only, visibility(vertex, fragment))]
    run_records:                         Handle<ShaderBuffer>,
    /// Shared `MaterialSlotValues` table read by migrated path fragments.
    #[storage(106, read_only, visibility(fragment))]
    material_table:                      Handle<ShaderBuffer>,
    /// Inverse display transform for unlit fragments in this batch's view.
    #[texture(109, dimension = "3d", visibility(fragment))]
    #[sampler(110, visibility(fragment))]
    pub(in crate::render) display_table: Handle<Image>,
    /// Routes this material's vertex stages (main, prepass, shadow) through
    /// `analytic_path_vertex_pull.wesl` instead of the standard mesh vertex stage.
    vertex_pull:                         bool,
    /// Identifies panel text among path batches that share the text shader mode.
    pub(super) is_panel_text:            bool,
    /// Distinguishes blend modes that share Bevy's mesh pipeline bits.
    alpha_mode:                          PathAlphaMode,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
enum PathAlphaMode {
    #[default]
    Other,
    Premultiplied,
    Add,
    Multiply,
}

impl From<AlphaMode> for PathAlphaMode {
    fn from(mode: AlphaMode) -> Self {
        match mode {
            AlphaMode::Premultiplied => Self::Premultiplied,
            AlphaMode::Add => Self::Add,
            AlphaMode::Multiply => Self::Multiply,
            _ => Self::Other,
        }
    }
}

impl PathExtension {
    /// Shared atlas bindings inspected by publication tests.
    #[cfg(test)]
    pub(crate) fn atlas_handles(&self) -> super::PathAtlasHandles {
        super::PathAtlasHandles {
            curves:       self.curves.clone(),
            bands:        self.bands.clone(),
            path_records: self.path_records.clone(),
        }
    }

    /// Builds the extension payload for one vertex-pulled analytic-path batch.
    #[must_use]
    pub(super) fn vertex_pull(
        render_mode: RenderMode,
        alpha_mode: AlphaMode,
        oit_paint_order: f32,
        anti_alias: AntiAlias,
        buffers: PathMaterialBuffers,
    ) -> Self {
        Self {
            uniforms:       PathUniform {
                render_mode: u32::from(render_mode),
                oit_paint_order,
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
            display_table:  Handle::default(),
            vertex_pull:    true,
            is_panel_text:  false,
            alpha_mode:     alpha_mode.into(),
        }
    }
}

/// Selects the world or identity display table for one path batch.
pub(crate) fn set_path_display_table(material: &mut PathExtendedMaterial, table: Handle<Image>) {
    material.extension.display_table = table;
}

#[cfg(test)]
pub(crate) const fn path_material_oit_paint_order(material: &PathExtendedMaterial) -> f32 {
    material.extension.uniforms.oit_paint_order
}

/// Pipeline-specialization key for [`PathExtension`]: which vertex stage a
/// material compiles against.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PathExtensionKey {
    /// Mirror of [`PathExtension::vertex_pull`].
    vertex_pull:              bool,
    pub(super) is_panel_text: bool,
    alpha_mode:               PathAlphaMode,
}

impl From<&PathExtension> for PathExtensionKey {
    fn from(extension: &PathExtension) -> Self {
        Self {
            vertex_pull:   extension.vertex_pull,
            is_panel_text: extension.is_panel_text,
            alpha_mode:    extension.alpha_mode,
        }
    }
}

impl MaterialExtension for PathExtension {
    fn fragment_shader() -> ShaderRef { ANALYTIC_PATH_SHADER_HANDLE.into() }

    fn prepass_fragment_shader() -> ShaderRef { ANALYTIC_PATH_SHADER_HANDLE.into() }

    // OIT resolves against prepass depth when the camera has DepthPrepass.
    // Opaque/masked paths must write it or a farther OIT panel hides their ink.
    // Our custom prepass fragment shader opts into PREPASS_READS_MATERIAL in
    // Bevy, retaining the bindings that the vertex-pull stage needs. Per frame
    // this adds depth-prepass draws and coverage evaluation for visible opaque
    // and masked batches; Bevy excludes blended batches from camera prepasses.
    fn enable_prepass() -> bool { true }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_path_extension_descriptor(
            descriptor,
            key.bind_group_data.vertex_pull,
            key.bind_group_data.alpha_mode,
            key.mesh_key,
            key.bind_group_data.is_panel_text,
        );
        Ok(())
    }
}

fn specialize_path_extension_descriptor(
    descriptor: &mut RenderPipelineDescriptor,
    vertex_pull: bool,
    alpha_mode: PathAlphaMode,
    mesh_key: MeshPipelineKey,
    is_panel_text: bool,
) {
    if vertex_pull && !material_group_is_stripped(descriptor) {
        specialize_path_camera_prepass(descriptor, mesh_key);
    }
    if !is_panel_text && let Some(fragment) = descriptor.fragment.as_mut() {
        fragment.shader_defs.push("NON_TEXT_PATH_COVERAGE".into());
    }
    let material_oit = descriptor.fragment.as_ref().is_some_and(|fragment| {
        fragment
            .shader_defs
            .contains(&"MATERIAL_OIT_ENABLED".into())
    });
    if material_oit {
        descriptor
            .vertex
            .shader_defs
            .push("MATERIAL_OIT_ENABLED".into());
    }
    if material_oit
        && alpha_mode == PathAlphaMode::Premultiplied
        && let Some(fragment) = descriptor.fragment.as_mut()
    {
        fragment.shader_defs.push("PATH_PREMULTIPLIED_OIT".into());
    } else if alpha_mode == PathAlphaMode::Premultiplied
        && let Some(fragment) = descriptor.fragment.as_mut()
    {
        fragment
            .shader_defs
            .push("PATH_PREMULTIPLIED_NON_OIT".into());
    }
    if vertex_pull && !material_group_is_stripped(descriptor) {
        if !descriptor
            .vertex
            .shader_defs
            .contains(&"PREPASS_PIPELINE".into())
            || descriptor
                .vertex
                .shader_defs
                .contains(&"PATH_CAMERA_PREPASS".into())
        {
            descriptor
                .vertex
                .shader_defs
                .push("PATH_EXPAND_QUADS".into());
        }
        // One WESL file serves the main, prepass, and shadow pipelines;
        // its `@if(PREPASS_PIPELINE)` gate picks the entry point. The
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
        // Share the SDF helper's stripped-material-group branch: the standard
        // vertex stage avoids vertex-pull bindings 104/105, and the helper
        // avoids material-table binding 106.
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment
                .shader_defs
                .push("SDF_STRIPPED_MATERIAL_GROUP".into());
        }
    }
}

/// Camera cutouts use the color pass's coverage, including its expanded quads.
/// Shadow views keep their existing binary silhouettes and island dithering.
fn specialize_path_camera_prepass(
    descriptor: &mut RenderPipelineDescriptor,
    mesh_key: MeshPipelineKey,
) {
    // Bevy's camera prepass key has no projection bits; shadow view keys set
    // perspective or orthographic bits in check_views_lights_need_specialization.
    if !descriptor
        .vertex
        .shader_defs
        .contains(&"PREPASS_PIPELINE".into())
        || mesh_key.intersects(MeshPipelineKey::VIEW_PROJECTION_RESERVED_BITS)
    {
        return;
    }
    // Bevy omits the fragment stage for a pure opaque depth-only pass, even
    // when a custom shader retains material bindings. Paths still need it to
    // discard the transparent area of their bounding quads.
    let fragment = descriptor.fragment.get_or_insert_with(|| FragmentState {
        shader: ANALYTIC_PATH_SHADER_HANDLE,
        shader_defs: descriptor.vertex.shader_defs.clone(),
        ..Default::default()
    });
    fragment.shader_defs.push("PATH_CAMERA_PREPASS".into());
    descriptor
        .vertex
        .shader_defs
        .push("PATH_CAMERA_PREPASS".into());
}

/// Whether bevy substituted the empty layout for the material bind group in
/// this pipeline. Depth-only pipelines do this — shadow views for alpha modes
/// without `MAY_DISCARD` (e.g. `Multiply`) — and the vertex-pull stage reads
/// bindings 104/105 from the material group, so swapping it in would fail
/// wgpu validation. Such pipelines keep the standard vertex stage instead:
/// the inert batch mesh's all-zero positions rasterize nothing, so the batch
/// casts no shadow there. The check reads the layout itself, so any other mode
/// or engine change that strips the material group takes the same branch.
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
    use bevy::math::Vec3;
    use bevy::render::render_resource::BlendState;
    use bevy::render::render_resource::ColorTargetState;
    use bevy::render::render_resource::ColorWrites;
    use bevy::render::render_resource::CompareFunction;
    use bevy::render::render_resource::DepthBiasState;
    use bevy::render::render_resource::DepthStencilState;
    use bevy::render::render_resource::FragmentState;
    use bevy::render::render_resource::StencilState;
    use bevy::render::render_resource::TextureFormat;
    use bevy::shader::ShaderDefVal;

    use super::*;
    use crate::render::material_table::MATERIAL_TABLE_BINDING;
    use crate::render::material_table::PATH_INSTANCES_BINDING;
    use crate::render::material_table::PATH_RUN_RECORDS_BINDING;

    #[test]
    fn path_camera_prepass_requires_a_cutout_fragment_even_for_opaque_materials() {
        let mut descriptor = RenderPipelineDescriptor::default();
        descriptor
            .vertex
            .shader_defs
            .push("PREPASS_PIPELINE".into());
        specialize_path_camera_prepass(&mut descriptor, MeshPipelineKey::DEPTH_PREPASS);
        assert!(
            descriptor.fragment.is_some(),
            "opaque analytic depth needs a cutout fragment"
        );
        let Some(fragment) = descriptor.fragment.as_ref() else {
            return;
        };
        assert_eq!(fragment.shader, ANALYTIC_PATH_SHADER_HANDLE);
        assert_eq!(fragment.targets, Vec::new());
        assert!(fragment.shader_defs.contains(&"PATH_CAMERA_PREPASS".into()));
        assert!(fragment.shader_defs.contains(&"PREPASS_PIPELINE".into()));
    }

    #[test]
    fn path_shadow_prepass_keeps_its_existing_silhouette_pipeline() {
        for projection in [
            MeshPipelineKey::VIEW_PROJECTION_PERSPECTIVE,
            MeshPipelineKey::VIEW_PROJECTION_ORTHOGRAPHIC | MeshPipelineKey::UNCLIPPED_DEPTH_ORTHO,
        ] {
            let mut descriptor = RenderPipelineDescriptor::default();
            descriptor
                .vertex
                .shader_defs
                .push("PREPASS_PIPELINE".into());
            specialize_path_camera_prepass(
                &mut descriptor,
                MeshPipelineKey::DEPTH_PREPASS | projection,
            );
            assert!(descriptor.fragment.is_none());
            assert!(
                !descriptor
                    .vertex
                    .shader_defs
                    .contains(&"PATH_CAMERA_PREPASS".into())
            );
        }
    }

    #[test]
    fn panel_text_skips_line_shader_code_while_shape_text_mode_keeps_it() {
        let mut text = RenderPipelineDescriptor {
            fragment: Some(FragmentState::default()),
            ..Default::default()
        };
        specialize_path_extension_descriptor(
            &mut text,
            true,
            PathAlphaMode::Other,
            MeshPipelineKey::empty(),
            true,
        );
        assert!(text.fragment.as_ref().is_some_and(|fragment| {
            !fragment
                .shader_defs
                .contains(&"NON_TEXT_PATH_COVERAGE".into())
        }));

        let mut shape = RenderPipelineDescriptor {
            fragment: Some(FragmentState::default()),
            ..Default::default()
        };
        specialize_path_extension_descriptor(
            &mut shape,
            true,
            PathAlphaMode::Other,
            MeshPipelineKey::empty(),
            false,
        );
        assert!(shape.fragment.as_ref().is_some_and(|fragment| {
            fragment
                .shader_defs
                .contains(&"NON_TEXT_PATH_COVERAGE".into())
        }));
    }

    #[test]
    fn stripped_analytic_pipeline_guards_vertex_pull_and_material_table_bindings() {
        let mut descriptor = RenderPipelineDescriptor {
            fragment: Some(FragmentState::default()),
            ..Default::default()
        };

        specialize_path_extension_descriptor(
            &mut descriptor,
            true,
            PathAlphaMode::Other,
            MeshPipelineKey::empty(),
            false,
        );

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
        let vertex_pull = include_str!("analytic_path_vertex_pull.wesl");
        let fragment_shader = include_str!("analytic_path.wesl");
        let helper = include_str!("../../shaders/sdf_material_table.wesl");
        assert!(vertex_pull.contains(&format!("@binding({PATH_INSTANCES_BINDING})")));
        assert!(vertex_pull.contains(&format!("@binding({PATH_RUN_RECORDS_BINDING})")));
        assert!(fragment_shader.contains(&format!("@binding({PATH_RUN_RECORDS_BINDING})")));
        assert!(helper.contains(&format!("@binding({MATERIAL_TABLE_BINDING})")));
        assert!(helper.contains("@if(!SDF_STRIPPED_MATERIAL_GROUP)"));
        assert!(helper.contains("@if(SDF_STRIPPED_MATERIAL_GROUP)"));
    }

    #[test]
    fn premultiplied_oit_specialization_writes_nodes_without_blending_or_depth_write() {
        let buffers = || PathMaterialBuffers {
            curves:       Handle::default(),
            bands:        Handle::default(),
            path_records: Handle::default(),
            instances:    Handle::default(),
            run_records:  Handle::default(),
        };
        let premultiplied = PathExtension::vertex_pull(
            RenderMode::Text,
            AlphaMode::Premultiplied,
            0.0,
            AntiAlias::default(),
            buffers(),
        );
        let add = PathExtension::vertex_pull(
            RenderMode::Text,
            AlphaMode::Add,
            0.0,
            AntiAlias::default(),
            buffers(),
        );
        assert_eq!(
            PathExtensionKey::from(&premultiplied).alpha_mode,
            PathAlphaMode::Premultiplied
        );
        assert_eq!(PathExtensionKey::from(&add).alpha_mode, PathAlphaMode::Add);
        let mut descriptor = RenderPipelineDescriptor {
            fragment: Some(FragmentState {
                shader_defs: vec!["MATERIAL_OIT_ENABLED".into()],
                targets: vec![Some(ColorTargetState {
                    format:     TextureFormat::Rgba8Unorm,
                    blend:      None,
                    write_mask: ColorWrites::ALL,
                })],
                ..Default::default()
            }),
            depth_stencil: Some(DepthStencilState {
                format:              bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare:       Some(CompareFunction::GreaterEqual),
                stencil:             StencilState::default(),
                bias:                DepthBiasState::default(),
            }),
            ..Default::default()
        };
        specialize_path_extension_descriptor(
            &mut descriptor,
            false,
            PathAlphaMode::Premultiplied,
            MeshPipelineKey::OIT_ENABLED | MeshPipelineKey::BLEND_PREMULTIPLIED_ALPHA,
            false,
        );
        assert!(
            descriptor
                .vertex
                .shader_defs
                .contains(&"MATERIAL_OIT_ENABLED".into())
        );
        assert!(descriptor.fragment.as_ref().is_some_and(|fragment| {
            fragment
                .shader_defs
                .contains(&"MATERIAL_OIT_ENABLED".into())
                && fragment
                    .shader_defs
                    .contains(&"PATH_PREMULTIPLIED_OIT".into())
        }));
        assert!(descriptor.fragment.as_ref().is_some_and(|fragment| {
            fragment.targets[0]
                .as_ref()
                .is_some_and(|target| target.blend.is_none())
        }));
        assert_eq!(
            descriptor
                .depth_stencil
                .as_ref()
                .and_then(|depth| depth.depth_write_enabled),
            Some(false)
        );
        assert!(
            include_str!("analytic_path.wesl")
                .contains("out.color = vec4<f32>(out.color.rgb * coverage, out.color.a);")
        );
    }

    #[test]
    fn premultiplied_oit_partial_glyph_coverage_blends_a_smooth_edge() {
        let shader = include_str!("analytic_path.wesl");
        assert!(shader.contains("let material_alpha = pbr_input.material.base_color.a;"));
        assert!(shader.contains("let final_alpha = coverage * material_alpha;"));
        assert!(shader.contains("out.color = vec4<f32>(out.color.rgb * coverage, out.color.a);"));
        assert!(!shader.contains("out.color.rgb / max(out.color.a, DISCARD_ALPHA)"));

        // Material RGB is already premultiplied by opacity. Coverage scales
        // both RGB and alpha before Bevy 0.20's sorted OIT resolve blends it.
        let ink = 0.8_f32;
        let fill = 0.1_f32;
        let material_alpha = 1.0_f32;
        let coverage = 0.25_f32;
        let stored_rgb = ink * material_alpha * coverage;
        let stored_alpha = coverage * material_alpha;
        let edge = fill.mul_add(1.0 - stored_alpha, stored_rgb);
        assert!(fill < edge && edge < ink);
        assert!((edge - 0.275).abs() < f32::EPSILON);
    }

    #[test]
    fn premultiplied_non_oit_partial_glyph_coverage_blends_between_ink_and_fill() {
        let mut descriptor = RenderPipelineDescriptor {
            fragment: Some(FragmentState {
                shader_defs: vec!["OIT_ENABLED".into()],
                targets: vec![Some(ColorTargetState {
                    format:     TextureFormat::Rgba8Unorm,
                    blend:      Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                ..Default::default()
            }),
            ..Default::default()
        };
        specialize_path_extension_descriptor(
            &mut descriptor,
            false,
            PathAlphaMode::Premultiplied,
            MeshPipelineKey::OIT_ENABLED | MeshPipelineKey::BLEND_PREMULTIPLIED_ALPHA,
            false,
        );
        assert!(
            !descriptor
                .vertex
                .shader_defs
                .contains(&"MATERIAL_OIT_ENABLED".into())
        );
        assert!(descriptor.fragment.as_ref().is_some_and(|fragment| {
            fragment
                .shader_defs
                .contains(&"PATH_PREMULTIPLIED_NON_OIT".into())
                && !fragment
                    .shader_defs
                    .contains(&"PATH_PREMULTIPLIED_OIT".into())
                && fragment.targets[0].as_ref().is_some_and(|target| {
                    target.blend == Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING)
                })
        }));
        assert!(
            include_str!("analytic_path.wesl")
                .contains("out.color = vec4<f32>(out.color.rgb * coverage, out.color.a);")
        );
        let coverage = 0.25;
        let ink = Vec3::new(226.0, 96.0, 55.0) / 255.0;
        let fill = Vec3::new(196.0, 176.0, 157.0) / 255.0;
        let composited = ink * coverage + fill * (1.0 - coverage);
        let old_composited = ink + fill * (1.0 - coverage);
        for channel in 0..3 {
            assert!(composited[channel] >= ink[channel].min(fill[channel]));
            assert!(composited[channel] <= ink[channel].max(fill[channel]));
        }
        assert!(old_composited.x > ink.x.max(fill.x));
    }

    /// The fragment band and vertex margin use the glyph's own em and one size ramp.
    #[test]
    fn text_coverage_uses_glyph_em_size_ramp_and_single_bias_transfer() {
        let fragment_shader = include_str!("analytic_path.wesl");
        let vertex_pull = include_str!("analytic_path_vertex_pull.wesl");
        assert!(fragment_shader.contains("text_em_size: f32"));
        assert!(vertex_pull.contains("text_em_size: f32"));
        assert!(fragment_shader.contains("let pixels_per_em = path.text_em_size /"));
        assert!(
            vertex_pull
                .contains("let design_span_x = record.uv_size.x * packed_path.bounds_min_size.z;")
        );
        assert!(vertex_pull.contains("let layout_em = packed_path.text_em_size * record.rect_size.x\n                / max(design_span_x, LINE_AA_EPSILON);"));
        assert!(
            vertex_pull
                .contains("let pixels_per_em = layout_em / max(units_per_pixel, LINE_AA_EPSILON);")
        );
        for shader in [fragment_shader, vertex_pull] {
            for policy in [
                "TEXT_LOW_FULL_PX_PER_EM: f32 = 4.9",
                "TEXT_LOW_END_PX_PER_EM: f32 = 2.9",
            ] {
                assert!(shader.contains(policy));
            }
            assert!(!shader.contains("text_stem_darkening"));
        }
        assert!(fragment_shader.contains("floor_strength = clamp("));
        assert!(fragment_shader.contains("center.counter_edges"));
        assert!(fragment_shader.contains("floor_strength > 0.0"));
        assert!(vertex_pull.contains("margin_px = TEXT_AA_MARGIN_PX * size_strength;"));
        assert!(vertex_pull.contains("const TEXT_AA_MARGIN_PX: f32 = TEXT_BAND_OUTER_HALF_PX;"));
        let transfers: Vec<&str> = fragment_shader
            .lines()
            .map(str::trim)
            .filter(|line| line.contains("= apply_text_coverage_bias("))
            .collect();
        assert_eq!(
            transfers,
            vec!["coverage = apply_text_coverage_bias(coverage, text_coverage_bias);"]
        );
        assert!(
            fragment_shader.contains("return run_records[render_index(in)].text_coverage_bias;"),
            "the transfer is the run record's authored bias"
        );
    }

    #[test]
    fn text_band_width_has_a_finite_zero_difference_guard() {
        let shader = include_str!("analytic_path.wesl");
        assert!(shader.contains("per_band = max(\n                            mix(forward_band, max(forward_band, analytic), floor_strength * (vec2<f32>(1.0) - center.counter_edges)),\n                            vec2<f32>(ROOT_EPSILON),\n                        );"));
        assert!(shader.contains("let band = max(mix("));
        assert!(shader.contains("), vec2<f32>(ROOT_EPSILON));"));
        let forward_band = 0.0_f32;
        let floor_strength = 0.0_f32;
        let band = (1.0 - forward_band)
            .mul_add(floor_strength, forward_band)
            .max(0.000_01);
        assert!((0.5 - 0.0 / band).is_finite());
    }

    #[test]
    fn off_and_supersample_scan_horizontal_edges_only_above_text_floor() {
        let shader = include_str!("analytic_path.wesl");
        assert!(shader.contains("    include_horizontal_x_edges: bool,\n) -> CoverageTerms {"));
        assert!(shader.contains("var include_horizontal_x_edges = floor_strength > 0.0;"));
        assert_eq!(
            shader
                .matches("band_walk_terms(point, edge_width, sample.scan_width_sq, 0.0, path, include_horizontal_x_edges);")
                .count(),
            1,
        );
        assert!(shader.contains("if path.text_em_size > 0.0 {"));
    }
}
