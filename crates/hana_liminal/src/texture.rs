use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::With;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureDimension;
use bevy::render::render_resource::TextureFormat;
use bevy::render::render_resource::TextureUsages;
use bevy_render::camera::ExtractedCamera;
use bevy_render::render_resource::Texture;
use bevy_render::render_resource::TextureDescriptor;
use bevy_render::renderer::RenderDevice;
use bevy_render::texture::CachedTexture;
use bevy_render::texture::TextureCache;

use super::camera::OutlineCamera;
use super::constants::MSAA_DISABLED_SAMPLE_COUNT;
use super::constants::OUTLINE_DEPTH_TEXTURE_LABEL;
use super::constants::OUTLINE_TEXTURE_BASE_MIP_ONLY;
use super::constants::OUTLINE_TEXTURE_SINGLE_LAYER;

#[derive(Clone, Copy)]
enum PingPongState {
    PrimaryInput,
    SecondaryInput,
}

#[derive(Clone, Component)]
pub(crate) struct FloodTextures {
    ping_pong_state:          PingPongState,
    /// `input` and `output` are swapped by `PingPongState` between jump-flood
    /// passes.
    pub(crate) input:         CachedTexture,
    pub(crate) output:        CachedTexture,
    /// A dedicated depth texture for mesh outlines to later compare against
    /// global depth
    pub(crate) outline_depth: Texture,
    /// Stores mask `appearance_data`: color in rgb and priority in alpha.
    pub(crate) appearance:    CachedTexture,
    /// Stores mask `owner_data`: owner ID in x and overlap factor in y, read by
    /// the hull and jump-flood compose shaders for group-aware overlap resolution.
    pub(crate) owner:         CachedTexture,
}

impl FloodTextures {
    pub(crate) const fn input(&self) -> &CachedTexture {
        match self.ping_pong_state {
            PingPongState::PrimaryInput => &self.input,
            PingPongState::SecondaryInput => &self.output,
        }
    }

    pub(crate) const fn output(&self) -> &CachedTexture {
        match self.ping_pong_state {
            PingPongState::PrimaryInput => &self.output,
            PingPongState::SecondaryInput => &self.input,
        }
    }

    pub(crate) const fn swap_ping_pong(&mut self) {
        self.ping_pong_state = match self.ping_pong_state {
            PingPongState::PrimaryInput => PingPongState::SecondaryInput,
            PingPongState::SecondaryInput => PingPongState::PrimaryInput,
        };
    }
}

pub(crate) fn prepare_flood_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    cameras: Query<(Entity, &ExtractedCamera), With<OutlineCamera>>,
) {
    for (entity, camera) in cameras.iter() {
        let Some(target_size) = camera.physical_target_size else {
            continue;
        };

        let size = Extent3d {
            width:                 target_size.x,
            height:                target_size.y,
            depth_or_array_layers: OUTLINE_TEXTURE_SINGLE_LAYER,
        };

        let texture_descriptor = TextureDescriptor {
            label: None,
            size,
            mip_level_count: OUTLINE_TEXTURE_BASE_MIP_ONLY,
            sample_count: MSAA_DISABLED_SAMPLE_COUNT,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba32Float,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };

        // `FloodTextures::outline_depth` is the flood-init pass depth attachment in
        // `src/node.rs` and is sampled as `outline_depth_texture` by the compose and
        // hull shaders.
        let outline_depth = render_device.create_texture(&TextureDescriptor {
            label: Some(OUTLINE_DEPTH_TEXTURE_LABEL),
            size,
            mip_level_count: OUTLINE_TEXTURE_BASE_MIP_ONLY,
            sample_count: MSAA_DISABLED_SAMPLE_COUNT,
            dimension: TextureDimension::D2,
            format: CORE_3D_DEPTH_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        commands.entity(entity).insert(FloodTextures {
            ping_pong_state: PingPongState::PrimaryInput,
            input: texture_cache.get(&render_device, texture_descriptor.clone()),
            output: texture_cache.get(&render_device, texture_descriptor.clone()),
            outline_depth,
            appearance: texture_cache.get(&render_device, texture_descriptor.clone()),
            owner: texture_cache.get(&render_device, texture_descriptor),
        });
        texture_cache.update();
    }
}
