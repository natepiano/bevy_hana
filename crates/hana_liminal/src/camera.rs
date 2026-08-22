use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::Add;
use bevy::prelude::Camera3d;
use bevy::prelude::Component;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::render::render_resource::TextureUsages;
use bevy_render::extract_component::ExtractComponent;
use bevy_render::view::NoIndirectDrawing;

/// Marker component for enabling a 3D camera to render mesh outlines.
///
/// Requires [`NoIndirectDrawing`]: the outline mask/hull phases build a parallel
/// `OutlineUniform` buffer indexed by the same per-instance index as `MeshUniform`
/// (see `render.rs`). In 0.19 the multidraw/indirect path assigns those indices on
/// the GPU and keeps its bins private, so the parallel buffer can only stay aligned
/// when the outline camera draws meshes directly with CPU-assigned indices.
#[derive(Debug, Component, Reflect, Clone, ExtractComponent)]
#[reflect(Component)]
#[require(DepthPrepass, NoIndirectDrawing)]
pub struct OutlineCamera;

/// Ensures the main pass depth texture has `TEXTURE_BINDING` so the compose shader
/// can sample it for correct occlusion of transmissive/transparent geometry.
///
/// Fires once when `OutlineCamera` is added, rather than polling every frame.
///
/// Needs to run in the main app because `Camera3d::depth_texture_usages` controls
/// how the GPU texture is allocated — by the time extraction runs, it's too late.
///
/// See `bevy_pbr::atmosphere::configure_camera_depth_usages` for the same pattern in Bevy.
pub(crate) fn configure_outline_camera_depth_texture(
    added: On<Add, OutlineCamera>,
    mut cameras: Query<&mut Camera3d>,
) {
    if let Ok(mut camera_3d) = cameras.get_mut(added.entity) {
        let mut usages = TextureUsages::from(camera_3d.depth_texture_usages);
        usages |= TextureUsages::TEXTURE_BINDING;
        camera_3d.depth_texture_usages = usages.into();
    }
}
