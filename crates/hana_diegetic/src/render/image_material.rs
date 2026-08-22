//! Image batch material definition.

use bevy::asset::Asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::ExtendedMaterial;
use bevy::pbr::MaterialExtension;
use bevy::pbr::MaterialExtensionKey;
use bevy::pbr::MaterialExtensionPipeline;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::prelude::Handle;
use bevy::prelude::default;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::render::render_resource::SpecializedMeshPipelineError;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

use super::image_batch::ImageBatchKey;

/// Embedded shader path used by all image material passes.
const IMAGE_PANEL_SHADER_PATH: &str = "embedded://hana_diegetic/shaders/image_panel.wgsl";

/// Image material extension over `StandardMaterial`.
///
/// `ImageExtension::records` is always present in the material bind group, so
/// Bevy keeps `MATERIAL_BIND_GROUP_INDEX` for image pipelines.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
pub(super) struct ImageExtension {
    /// Batched `ImageRenderRecord` rows read by the vertex and fragment stages.
    #[storage(107, read_only, visibility(vertex, fragment))]
    records: Handle<ShaderBuffer>,
}

impl MaterialExtension for ImageExtension {
    fn vertex_shader() -> ShaderRef { IMAGE_PANEL_SHADER_PATH.into() }

    fn fragment_shader() -> ShaderRef { IMAGE_PANEL_SHADER_PATH.into() }

    fn prepass_vertex_shader() -> ShaderRef { IMAGE_PANEL_SHADER_PATH.into() }

    fn prepass_fragment_shader() -> ShaderRef { IMAGE_PANEL_SHADER_PATH.into() }

    fn deferred_vertex_shader() -> ShaderRef { IMAGE_PANEL_SHADER_PATH.into() }

    fn deferred_fragment_shader() -> ShaderRef { IMAGE_PANEL_SHADER_PATH.into() }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        _descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        Ok(())
    }
}

/// Image batch material type.
pub(super) type ImageExtendedMaterial = ExtendedMaterial<StandardMaterial, ImageExtension>;

/// Rebinds `ImageExtension::records` after `ImageBatchResources::records` grows.
pub(super) fn set_image_material_records(
    material: &mut ImageExtendedMaterial,
    records: Handle<ShaderBuffer>,
) {
    material.extension.records = records;
}

#[cfg(test)]
pub(super) const fn image_material_records(
    material: &ImageExtendedMaterial,
) -> &Handle<ShaderBuffer> {
    &material.extension.records
}

/// Builds the render material for one image batch.
#[must_use]
pub(super) fn image_batch_material(
    key: &ImageBatchKey,
    records: Handle<ShaderBuffer>,
) -> ImageExtendedMaterial {
    ExtendedMaterial {
        base:      StandardMaterial {
            base_color_texture: Some(key.texture.clone()),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Blend,
            depth_bias: key.depth_bias(),
            ..default()
        },
        extension: ImageExtension { records },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_image_shader_ref(shader_ref: ShaderRef) {
        assert!(matches!(
            shader_ref,
            ShaderRef::Path(path) if path.to_string() == IMAGE_PANEL_SHADER_PATH
        ));
    }

    #[test]
    fn image_extension_uses_embedded_shader_for_all_passes() {
        assert_image_shader_ref(ImageExtension::vertex_shader());
        assert_image_shader_ref(ImageExtension::fragment_shader());
        assert_image_shader_ref(ImageExtension::prepass_vertex_shader());
        assert_image_shader_ref(ImageExtension::prepass_fragment_shader());
        assert_image_shader_ref(ImageExtension::deferred_vertex_shader());
        assert_image_shader_ref(ImageExtension::deferred_fragment_shader());
    }

    #[test]
    fn image_shader_reads_record_buffer_and_uses_world_record_transform() {
        let shader = include_str!("../shaders/image_panel.wgsl");

        assert!(shader.contains("@binding(107) var<storage, read> image_records"));
        assert!(shader.contains("let local = signs * record.size * 0.5;"));
        assert!(shader.contains("let world = record.transform * vec4<f32>(local, 0.0, 1.0);"));
        assert!(!shader.contains("position_local_to_world"));
    }
}
