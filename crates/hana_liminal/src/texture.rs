use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::With;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureDimension;
use bevy::render::render_resource::TextureFormat;
use bevy::render::render_resource::TextureUsages;
use bevy_render::camera::ExtractedCamera;
use bevy_render::render_asset::RenderAssets;
use bevy_render::render_phase::ViewBinnedRenderPhases;
use bevy_render::render_resource::Texture;
use bevy_render::render_resource::TextureDescriptor;
use bevy_render::renderer::RenderDevice;
use bevy_render::texture::CachedTexture;
use bevy_render::texture::GpuImage;
use bevy_render::texture::TextureCache;
use bevy_render::view::ExtractedView;

use super::camera::OutlineCamera;
use super::constants::MSAA_DISABLED_SAMPLE_COUNT;
use super::constants::OUTLINE_TEXTURE_BASE_MIP_ONLY;
use super::constants::OUTLINE_TEXTURE_SINGLE_LAYER;
use super::extract::ActiveOutlineMethods;
use super::extract::ActiveOutlineModes;
use super::mask::HullOutlinePhase;
use super::mask::JumpFloodOutlinePhase;
use super::outline_depth::OutlineDepthTexture;

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
    /// Depth texture for the outline passes, sampled later to compare against
    /// the main pass depth.
    pub(crate) outline_depth: Texture,
    /// Stores mask `appearance_data`: color in rgb and priority in alpha.
    pub(crate) appearance:    CachedTexture,
    /// Stores mask `owner_data`: owner ID in x and overlap factor in y, read by
    /// the hull and jump-flood compose shaders for group-aware overlap resolution.
    /// Compose uses z for view distance and w for orthographic depth per metre.
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

pub(crate) fn flood_textures_are_needed(
    active: Res<ActiveOutlineModes>,
    textures: Query<(), With<FloodTextures>>,
) -> bool {
    active.methods != ActiveOutlineMethods::None || !textures.is_empty()
}

pub(crate) fn prepare_flood_textures(
    mut commands: Commands,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
    active: Res<ActiveOutlineModes>,
    outline_phases: Res<ViewBinnedRenderPhases<JumpFloodOutlinePhase>>,
    hull_phases: Res<ViewBinnedRenderPhases<HullOutlinePhase>>,
    images: Res<RenderAssets<GpuImage>>,
    cameras: Query<
        (
            Entity,
            &ExtractedCamera,
            &ExtractedView,
            Has<FloodTextures>,
            &OutlineDepthTexture,
        ),
        With<OutlineCamera>,
    >,
) {
    if active.methods == ActiveOutlineMethods::None {
        for (entity, _, _, _, _) in &cameras {
            commands.entity(entity).remove::<FloodTextures>();
        }
        return;
    }

    for (entity, camera, view, has_textures, depth) in &cameras {
        let has_outline = outline_phases
            .get(&view.retained_view_entity)
            .is_some_and(|phase| !phase.is_empty())
            || (active.methods.has_hull()
                && hull_phases
                    .get(&view.retained_view_entity)
                    .is_some_and(|phase| !phase.is_empty()));
        let Some(target_size) = camera.physical_target_size.filter(|_| has_outline) else {
            if has_textures {
                commands.entity(entity).remove::<FloodTextures>();
            }
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

        let (Some(outline_depth), Some(strokes)) =
            (images.get(&depth.surface), images.get(&depth.strokes))
        else {
            continue;
        };

        commands.entity(entity).insert(FloodTextures {
            ping_pong_state: PingPongState::PrimaryInput,
            input:           texture_cache.get(&render_device, texture_descriptor.clone()),
            output:          CachedTexture {
                texture:      strokes.texture.clone(),
                default_view: strokes.texture_view.clone(),
            },
            outline_depth:   outline_depth.texture.clone(),
            appearance:      texture_cache.get(&render_device, texture_descriptor.clone()),
            owner:           texture_cache.get(&render_device, texture_descriptor),
        });
        // Bevy releases and ages the shared cache once in RenderSystems::Cleanup.
        // Updating it here would let later cameras take this view's attachments.
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::RenderTarget;
    use bevy::camera::visibility::RenderLayers;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use bevy::render::RenderApp;
    use bevy::render::RenderPlugin;
    use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
    use bevy::window::ExitCondition;
    use bevy::winit::WinitPlugin;

    use super::*;
    use crate::LiminalPlugin;
    use crate::Outline;
    use crate::OutlineActivity;

    fn outline_app(
        activity: OutlineActivity,
        camera_layers: [usize; 2],
    ) -> (App, Entity, [Entity; 2]) {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<WinitPlugin>()
                .disable::<PipelinedRenderingPlugin>(),
        )
        .add_plugins(LiminalPlugin);
        let cameras = camera_layers.map(|layer| {
            let target =
                app.world_mut()
                    .resource_mut::<Assets<Image>>()
                    .add(Image::new_target_texture(
                        64,
                        64,
                        TextureFormat::Rgba8UnormSrgb,
                        None,
                    ));
            app.world_mut()
                .spawn((
                    Camera3d::default(),
                    OutlineCamera,
                    Msaa::Off,
                    RenderLayers::layer(layer),
                    RenderTarget::Image(target.into()),
                    Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
                ))
                .id()
        });
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(1.0, 1.0, 1.0));
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                unlit: true,
                ..default()
            });
        let mut outline = Outline::jump_flood(4.0).build();
        outline.activity = activity;
        let body = app
            .world_mut()
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                outline,
            ))
            .id();
        app.finish();
        app.cleanup();
        for _ in 0..8 {
            app.update();
        }
        (app, body, cameras)
    }

    fn view_textures(app: &mut App, camera: Entity) -> Result<FloodTextures, String> {
        let world = app.sub_app_mut(RenderApp).world_mut();
        world
            .query::<(&ExtractedView, &FloodTextures)>()
            .iter(world)
            .find(|(view, _)| view.retained_view_entity.main_entity == camera.into())
            .map(|(_, textures)| textures.clone())
            .ok_or_else(|| format!("camera {camera:?} has no flood textures"))
    }

    #[test]
    fn no_enabled_outline_prepares_no_flood_textures() -> Result<(), String> {
        let (mut app, body, cameras) = outline_app(OutlineActivity::Disabled, [0, 1]);
        let world = app.sub_app_mut(RenderApp).world_mut();
        assert!(world.query::<&FloodTextures>().iter(world).next().is_none());
        assert!(
            !world
                .run_system_once(flood_textures_are_needed)
                .map_err(|error| error.to_string())?
        );

        app.world_mut()
            .get_mut::<Outline>(body)
            .ok_or("outline missing")?
            .activity = OutlineActivity::Enabled;
        app.update();
        view_textures(&mut app, cameras[0])?;
        assert!(
            view_textures(&mut app, cameras[1]).is_err(),
            "a view on another layer has no outline to draw"
        );

        app.world_mut()
            .get_mut::<Outline>(body)
            .ok_or("outline missing")?
            .activity = OutlineActivity::Disabled;
        app.update();
        let world = app.sub_app_mut(RenderApp).world_mut();
        assert!(world.query::<&FloodTextures>().iter(world).next().is_none());
        assert!(
            !world
                .run_system_once(flood_textures_are_needed)
                .map_err(|error| error.to_string())?
        );
        Ok(())
    }

    #[test]
    fn enabled_outline_reuses_depth_on_consecutive_frames() -> Result<(), String> {
        let (mut app, _, cameras) = outline_app(OutlineActivity::Enabled, [0, 0]);
        let first = cameras
            .map(|camera| view_textures(&mut app, camera))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        assert_ne!(
            first[0].outline_depth.id(),
            first[1].outline_depth.id(),
            "views own separate depth attachments"
        );
        assert_ne!(
            first[0].input.texture.id(),
            first[1].input.texture.id(),
            "cache aging must not release attachments between cameras"
        );
        app.update();
        for (camera, previous) in cameras.into_iter().zip(first) {
            let next = view_textures(&mut app, camera)?;
            assert_eq!(previous.outline_depth.id(), next.outline_depth.id());
            assert_eq!(previous.input.texture.id(), next.input.texture.id());
            assert_eq!(previous.output.texture.id(), next.output.texture.id());
            assert_eq!(
                previous.appearance.texture.id(),
                next.appearance.texture.id()
            );
            assert_eq!(previous.owner.texture.id(), next.owner.texture.id());
        }
        Ok(())
    }
}
