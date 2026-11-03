//! Surface depth shared by outlines and overlays, including transmissive meshes.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::extract::extract_component::ExtractComponent;
use bevy::pbr::MaterialExtension;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::render_resource::*;
use bevy::shader::ShaderRef;

use crate::OutlineCamera;
use crate::constants::OUTLINE_DEPTH_TEXTURE_LABEL;
use crate::constants::OUTLINE_OCCLUDED_SHADER_HANDLE;

/// Camera-sized surface depth, written before the main pass without changing
/// scene depth or the appearance of transparent materials.
#[derive(Component, Clone, ExtractComponent)]
#[extract_app(RenderApp)]
pub struct OutlineDepthTexture {
    /// Nearest outlined surface, before silhouette expansion.
    pub surface:     Handle<Image>,
    /// Full transmissive surface depth, independent of outline ownership.
    pub translucent: Handle<Image>,
    /// Expanded outline seeds: source UV, stroke width and source depth.
    pub strokes:     Handle<Image>,
}

/// An unlit overlay that is hidden by outlined surfaces, including transparent
/// ones. Bind the texture belonging to the camera that draws the overlay.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct OutlineDepthTest {
    /// The observing camera's [`OutlineDepthTexture`].
    #[texture(100, sample_type = "depth")]
    pub depth:       Handle<Image>,
    /// The same camera's expanded outline seeds.
    #[texture(101, sample_type = "float", filterable = false)]
    pub strokes:     Handle<Image>,
    /// Full transmissive surface depth for bodies without outlines.
    #[texture(102, sample_type = "depth")]
    pub translucent: Handle<Image>,
}

/// Registers the overlay shader independently of outline rendering.
pub struct OutlineDepthTestPlugin;

impl Plugin for OutlineDepthTestPlugin {
    fn build(&self, app: &mut App) {
        let _ = app.world_mut().resource_mut::<Assets<Shader>>().insert(
            OUTLINE_OCCLUDED_SHADER_HANDLE.id(),
            Shader::from_wesl(
                include_str!("shaders/outline_occluded.wesl"),
                "embedded://hana_liminal/outline_occluded.wesl",
            ),
        );
    }
}

impl MaterialExtension for OutlineDepthTest {
    fn fragment_shader() -> ShaderRef { OUTLINE_OCCLUDED_SHADER_HANDLE.into() }
    fn enable_prepass() -> bool { false }
    fn enable_shadows() -> bool { false }
}

pub(crate) fn prepare_camera_outline_depth(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<(Entity, &Camera, Option<&OutlineDepthTexture>), With<OutlineCamera>>,
) {
    for (entity, camera, depth) in &cameras {
        let Some(size) = camera.physical_target_size() else {
            continue;
        };
        if depth
            .and_then(|depth| images.get(&depth.surface))
            .is_some_and(|image| {
                image.texture_descriptor.size.width == size.x
                    && image.texture_descriptor.size.height == size.y
            })
        {
            continue;
        }
        let mut image = Image::new_uninit(
            Extent3d {
                width:                 size.x,
                height:                size.y,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            CORE_3D_DEPTH_FORMAT,
            RenderAssetUsages::default(),
        );
        image.texture_descriptor.usage =
            TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT;
        image.texture_descriptor.label = Some(OUTLINE_DEPTH_TEXTURE_LABEL);
        let mut strokes = Image::new_uninit(
            image.texture_descriptor.size,
            TextureDimension::D2,
            TextureFormat::Rgba32Float,
            RenderAssetUsages::default(),
        );
        strokes.texture_descriptor.usage = image.texture_descriptor.usage;
        let translucent = images.add(image.clone());
        commands.entity(entity).insert(OutlineDepthTexture {
            translucent,
            surface: images.add(image),
            strokes: images.add(strokes),
        });
    }
}
